use super::Connection;
use anyhow::{bail, ensure, Result};
use serde_json::{json, Value};
use std::{
    fmt,
    path::Path,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout},
    sync::watch,
    task::JoinHandle,
};

pub const MAX_FRAME: usize = 1_048_576;
/// Bytes of SSH stderr kept for classification. Everything after this is still
/// read, and discarded, so a chatty ssh can never fill the pipe and block.
const STDERR_CAPTURE_LIMIT: usize = 8 * 1024;
/// Upper bound, in bytes, on [`SshFailure::detail`].
pub const SSH_FAILURE_DETAIL_LIMIT: usize = 2 * 1024;
/// A closed pipe almost always means ssh is exiting. Waiting this long for it
/// lets the report carry ssh's own status (255, 127) instead of racing it.
const EXIT_GRACE: Duration = Duration::from_secs(2);
/// After ssh is gone, how long its stderr gets to reach end-of-file. A jump
/// host's helper can hold the pipe open, so this is bounded.
const STDERR_GRACE: Duration = Duration::from_millis(500);
/// At most this many fingerprints are summarised at the top of the detail.
const MAX_FINGERPRINTS: usize = 4;

struct WireFailure {
    code: String,
    description: &'static str,
    /// The peer closed a pipe, so ssh is most likely exiting on its own.
    peer_closed: bool,
}
impl WireFailure {
    fn new(code: &str, description: &'static str) -> Self {
        Self {
            code: code.into(),
            description,
            peer_closed: false,
        }
    }
    fn closed(code: &str, description: &'static str) -> Self {
        Self {
            peer_closed: true,
            ..Self::new(code, description)
        }
    }
    fn io(stage: &'static str, error: std::io::Error) -> Self {
        use std::io::ErrorKind;
        let kind = match error.kind() {
            ErrorKind::BrokenPipe => "broken_pipe",
            ErrorKind::ConnectionReset => "connection_reset",
            ErrorKind::ConnectionAborted => "connection_aborted",
            ErrorKind::NotConnected => "not_connected",
            ErrorKind::UnexpectedEof => "unexpected_eof",
            ErrorKind::TimedOut => "timed_out",
            ErrorKind::WouldBlock => "would_block",
            ErrorKind::Interrupted => "interrupted",
            ErrorKind::PermissionDenied => "permission_denied",
            ErrorKind::InvalidData => "invalid_data",
            _ => "other",
        };
        Self {
            code: format!("ssh_{stage}_io_{kind}"),
            description: "SSH pipe I/O failed",
            peer_closed: true,
        }
    }
}

/// Why the SSH transport failed, as far as OpenSSH's own words and the child's
/// exit status tell us. Classification never guesses: anything that matches no
/// rule is [`SshFailureKind::Other`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SshFailureKind {
    /// The server wants a password, MFA or a key the agent does not hold.
    AuthRequired,
    /// Strict host-key checking refused a host absent from known_hosts.
    HostKeyUnknown,
    /// The server offered a key that differs from the pinned one (or a revoked one).
    HostKeyChanged,
    /// DNS, routing or TCP failed before SSH could start.
    Unreachable,
    /// SSH worked but `~/.local/bin/biorouter-crew` could not be run.
    BridgeMissing,
    /// Anything else, including a failure while ssh was still running.
    Other,
}

impl SshFailureKind {
    /// The typed code the daemon's HTTP surface answers with for this kind.
    pub fn api_code(self) -> &'static str {
        match self {
            Self::AuthRequired => "crew_ssh_auth_required",
            Self::HostKeyUnknown => "crew_ssh_host_key_unknown",
            Self::HostKeyChanged => "crew_ssh_host_key_changed",
            Self::Unreachable => "crew_ssh_unreachable",
            Self::BridgeMissing => "crew_bridge_missing",
            Self::Other => "crew_ssh_failed",
        }
    }
}

/// A fatal SSH transport failure. `Display` is the stable, stderr-free message
/// (it is persisted as `last_error` and matched by older renderers); `detail`
/// carries OpenSSH's own bounded, control-stripped words for "Copy details"
/// only and is deliberately absent from both `Display` and `Debug`.
#[derive(Clone)]
pub struct SshFailure {
    pub kind: SshFailureKind,
    /// The wire-level failure code, e.g. `ssh_eof`.
    pub code: String,
    /// The child's state before our own cleanup, e.g. `exit_255` or `running`.
    pub status: String,
    pub description: String,
    /// At most [`SSH_FAILURE_DETAIL_LIMIT`] bytes of sanitized stderr, with any
    /// host-key fingerprints summarised first. `None` when ssh said nothing.
    pub detail: Option<String>,
}

impl SshFailure {
    pub fn api_code(&self) -> &'static str {
        self.kind.api_code()
    }
}

impl fmt::Display for SshFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.code == SIGN_IN_REFUSED {
            return f.write_str(&self.description);
        }
        write!(
            f,
            "Crew SSH failure [{}; child_before_cleanup={}]: {}; reconnect. Submitted operation outcome may be unknown; inspect history before retrying",
            self.code, self.status, self.description
        )
    }
}

impl fmt::Debug for SshFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `{:#?}` on an anyhow::Error prints this, so the detail stays out of logs.
        f.debug_struct("SshFailure")
            .field("kind", &self.kind)
            .field("code", &self.code)
            .field("status", &self.status)
            .field("description", &self.description)
            .field("detail_bytes", &self.detail.as_ref().map_or(0, String::len))
            .finish()
    }
}

impl std::error::Error for SshFailure {}

/// The child's state at the moment of failure, before any cleanup signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChildState {
    Exited(Option<i32>),
    Running,
    Unknown,
}

impl ChildState {
    fn label(self) -> String {
        match self {
            Self::Exited(Some(code)) => format!("exit_{code}"),
            Self::Exited(None) => "exited_without_code".into(),
            Self::Running => "running".into(),
            Self::Unknown => "unknown".into(),
        }
    }
}

/// Continuously drains a child's stderr, keeping only the first
/// [`STDERR_CAPTURE_LIMIT`] bytes.
struct StderrCapture {
    kept: Arc<Mutex<Vec<u8>>>,
    finished: watch::Receiver<bool>,
    task: JoinHandle<()>,
}

impl StderrCapture {
    fn spawn<R: AsyncRead + Unpin + Send + 'static>(reader: R) -> Self {
        let kept = Arc::new(Mutex::new(Vec::new()));
        let (done, finished) = watch::channel(false);
        let sink = kept.clone();
        let task = tokio::spawn(async move {
            drain_stderr(reader, &sink).await;
            let _ = done.send(true);
        });
        Self {
            kept,
            finished,
            task,
        }
    }

    /// Wait, at most `grace`, for the writer side to close.
    async fn settle(&mut self, grace: Duration) {
        let _ = tokio::time::timeout(grace, self.finished.wait_for(|done| *done)).await;
    }

    fn snapshot(&self) -> Vec<u8> {
        self.kept
            .lock()
            .map(|kept| kept.clone())
            .unwrap_or_default()
    }
}

impl Drop for StderrCapture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn drain_stderr<R: AsyncRead + Unpin>(mut reader: R, sink: &Mutex<Vec<u8>>) {
    let mut chunk = [0u8; 4096];
    loop {
        let read = match reader.read(&mut chunk).await {
            Ok(0) => return,
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            // Returning drops the reader, closing our end: ssh's later writes
            // fail with EPIPE instead of blocking on a full pipe.
            Err(_) => return,
        };
        if let Ok(mut kept) = sink.lock() {
            let room = STDERR_CAPTURE_LIMIT.saturating_sub(kept.len());
            kept.extend_from_slice(&chunk[..read.min(room)]);
        }
    }
}

pub struct Transport {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: StderrCapture,
    unusable: bool,
    /// Whether any complete answer has come back over this bridge. Until one has, the SSH
    /// session may never have been established, so nothing can have reached the broker.
    answered: bool,
    /// The server and login this bridge signs in to, for a refusal a person can read.
    sign_in: Option<SignInTarget>,
    /// When a request last started or finished on this bridge, by the wall clock. The
    /// broker drops a bridge that sends nothing for 300 s, and that clock keeps running while
    /// this computer sleeps, which a monotonic `Instant` would not see (D-KEEPALIVE).
    last_activity: std::time::SystemTime,
}

/// Who the bridge signs in as, and where, from the saved SSH login (`user@host` or an alias).
#[derive(Clone)]
struct SignInTarget {
    server: String,
    user: Option<String>,
}

impl SignInTarget {
    fn from_login(login: &str) -> Self {
        match login.rsplit_once('@') {
            Some((user, server)) if !user.is_empty() && !server.is_empty() => Self {
                server: server.to_owned(),
                user: Some(user.to_owned()),
            },
            _ => Self {
                server: login.to_owned(),
                user: None,
            },
        }
    }
    /// T-53: the sentence for a sign-in the server refused before any request reached the
    /// broker. Nothing was submitted, so it says nothing about an unknown outcome.
    fn refused(&self) -> String {
        match &self.user {
            Some(user) => format!(
                "Couldn't sign in to {} as {user}: the server refused this computer's SSH key.",
                self.server
            ),
            None => format!(
                "Couldn't sign in to {}: the server refused this computer's SSH key.",
                self.server
            ),
        }
    }
}

/// [`SshFailure::code`] for a sign-in the server refused before this bridge ever answered.
/// Its `Display` is the description alone: no request was submitted, so there is no unknown
/// outcome to warn about.
const SIGN_IN_REFUSED: &str = "ssh_sign_in_refused";

pub fn ssh_args(c: &Connection, control: &Path) -> Vec<String> {
    login_args(
        c.port,
        c.identity_file.as_deref(),
        c.proxy_jump.as_deref(),
        Some(control),
    )
}

/// The options every Crew `ssh` carries, for a login given by its parts: the hardening,
/// the development profile's SSH files, and the route. With no `control` socket, no
/// multiplexed master is used at all (`ControlPath=none`), not even one the person's own
/// configuration names.
pub(super) fn login_args(
    port: Option<u16>,
    identity_file: Option<&str>,
    proxy_jump: Option<&str>,
    control: Option<&Path>,
) -> Vec<String> {
    let mut args = vec!["-T".into()];
    match control {
        Some(control) => args.extend(["-S".into(), control.to_string_lossy().into_owned()]),
        None => args.extend([
            "-o".into(),
            "ControlPath=none".into(),
            "-o".into(),
            "ControlMaster=no".into(),
        ]),
    }
    if let Some(profile) = std::env::var_os("BIOROUTER_DEV_PROFILE_ROOT") {
        let ssh = std::path::PathBuf::from(profile).join("home/.ssh");
        args.extend([
            "-F".into(),
            ssh.join("config").to_string_lossy().into_owned(),
            "-o".into(),
            format!("UserKnownHostsFile={}", ssh.join("known_hosts").display()),
            "-o".into(),
            "IdentityAgent=none".into(),
            "-o".into(),
            "IdentitiesOnly=yes".into(),
        ]);
    }
    for option in [
        "StrictHostKeyChecking=yes",
        "ForwardAgent=no",
        "ForwardX11=no",
        "PermitLocalCommand=no",
        "ClearAllForwardings=yes",
        "ConnectTimeout=30",
        "ServerAliveInterval=30",
        "ServerAliveCountMax=3",
    ] {
        args.extend(["-o".into(), option.into()]);
    }
    if let Some(port) = port {
        args.extend(["-p".into(), port.to_string()]);
    }
    if let Some(identity) = identity_file {
        args.extend(["-i".into(), identity.to_owned()]);
    }
    if let Some(jump) = proxy_jump {
        args.extend(["-J".into(), jump.to_owned()]);
    }
    args
}

/// How a finished `ssh` failed, from its exit status and OpenSSH's own words, by the same
/// rules as a bridge's failure.
pub(super) fn classify_exit(code: Option<i32>, stderr: &str) -> SshFailureKind {
    classify(ChildState::Exited(code), stderr)
}

impl Transport {
    pub async fn connect(c: &Connection, control: &Path) -> Result<Self> {
        ensure!(
            super::safe_atom(&c.ssh_target)
                && super::safe_atom(&c.socket_path)
                && c.socket_path.starts_with('/'),
            "Saved SSH connection contains invalid command arguments"
        );
        uuid::Uuid::parse_str(&c.workspace_id)?;
        let mut args = ssh_args(c, control);
        args.extend(["-o".into(), "BatchMode=yes".into(), "-o".into(), "ControlMaster=no".into(), "-o".into(), "ControlPersist=no".into(), c.ssh_target.clone(),
            // Every remote argument has a restricted grammar; no content or credential enters this shell command.
            format!("~/.local/bin/biorouter-crew bridge --stdio --socket {} --owner-uid {} --workspace-id {}", c.socket_path, c.owner_uid, c.workspace_id)]);
        super::ssh_policy::preflight(&args, &c.ssh_target).await?;
        let mut command = tokio::process::Command::new("ssh");
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        crate::subprocess::prepare_agent_child_command(&mut command);
        let mut transport = Self::from_child(command.spawn()?)?;
        transport.sign_in = Some(SignInTarget::from_login(&c.ssh_target));
        Ok(transport)
    }
    /// Adopt a spawned child whose three standard streams are all piped. Must
    /// run inside a Tokio runtime: stderr is drained by a task from here on.
    fn from_child(mut child: Child) -> Result<Self> {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("SSH input unavailable"))?;
        let stdout = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| anyhow::anyhow!("SSH output unavailable"))?,
        );
        let stderr = StderrCapture::spawn(
            child
                .stderr
                .take()
                .ok_or_else(|| anyhow::anyhow!("SSH diagnostics unavailable"))?,
        );
        Ok(Self {
            child,
            stdin,
            stdout,
            stderr,
            unusable: false,
            answered: false,
            sign_in: None,
            last_activity: std::time::SystemTime::now(),
        })
    }
    pub async fn request(
        &mut self,
        method: &str,
        params: Value,
        auth: Option<Value>,
        credential: Option<&str>,
        id: Option<String>,
    ) -> Result<Value> {
        ensure!(!self.unusable, "Crew SSH transport is unusable; reconnect before issuing another request. Inspect any previously submitted operation before retrying");
        let id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let mut frame = json!({"version":1,"id":id,"method":method,"params":params});
        if let Some(auth) = auth {
            frame["auth"] = auth;
        }
        if let Some(credential) = credential {
            frame["credential"] = json!(credential);
        }
        let mut bytes = serde_json::to_vec(&frame)?;
        ensure!(bytes.len() < MAX_FRAME, "Crew request exceeds frame limit");
        bytes.push(b'\n');
        // Cancellation after a write must never allow the next caller to consume
        // this request's late reply. Only a complete valid envelope rearms it.
        self.unusable = true;
        self.last_activity = std::time::SystemTime::now();
        let result =
            tokio::time::timeout(Duration::from_secs(45), self.exchange(&bytes, &id)).await;
        self.last_activity = std::time::SystemTime::now();
        match result {
            Ok(Ok(v)) => {
                self.unusable = false;
                self.answered = true;
                if let Some(error) = v.get("error").filter(|v| !v.is_null()) {
                    bail!("Crew broker refused request: {}", error);
                }
                v.get("result")
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("Crew response has no result"))
            }
            Ok(Err(failure)) => Err(self.fatal_failure(failure).await),
            Err(_) => Err(self
                .fatal_failure(WireFailure::new("ssh_timeout", "Crew request timed out"))
                .await),
        }
    }
    async fn exchange(
        &mut self,
        bytes: &[u8],
        id: &str,
    ) -> std::result::Result<Value, WireFailure> {
        self.stdin
            .write_all(bytes)
            .await
            .map_err(|error| WireFailure::io("write", error))?;
        self.stdin
            .flush()
            .await
            .map_err(|error| WireFailure::io("write", error))?;
        let mut bytes = Vec::new();
        let length = (&mut self.stdout)
            .take((MAX_FRAME + 1) as u64)
            .read_until(b'\n', &mut bytes)
            .await
            .map_err(|error| WireFailure::io("read", error))?;
        if length == 0 {
            return Err(WireFailure::closed("ssh_eof", "SSH connection closed"));
        }
        if length > MAX_FRAME {
            return Err(WireFailure::new(
                "ssh_frame_too_large",
                "Crew response exceeds frame limit or is incomplete",
            ));
        }
        if bytes.last() != Some(&b'\n') {
            return Err(WireFailure::new(
                "ssh_frame_incomplete",
                "Crew response exceeds frame limit or is incomplete",
            ));
        }
        let response: Value = serde_json::from_slice(&bytes).map_err(|_| {
            WireFailure::new(
                "ssh_invalid_json",
                "Crew SSH frame validation failed: invalid JSON",
            )
        })?;
        if response.get("id").and_then(Value::as_str) != Some(id) {
            return Err(WireFailure::new(
                "ssh_response_id_mismatch",
                "Crew SSH frame validation failed: response ID mismatch",
            ));
        }
        if let Some(error) = response.get("error").filter(|value| !value.is_null()) {
            if !error.get("code").is_some_and(Value::is_string)
                || !error.get("message").is_some_and(Value::is_string)
            {
                return Err(WireFailure::new(
                    "ssh_invalid_envelope",
                    "Crew SSH frame validation failed: invalid broker error envelope",
                ));
            }
        } else if response.get("result").is_none() {
            return Err(WireFailure::new(
                "ssh_invalid_envelope",
                "Crew SSH frame validation failed: response has no result",
            ));
        }
        Ok(response)
    }
    async fn fatal_failure(&mut self, failure: WireFailure) -> anyhow::Error {
        // Capture exit status before our own cleanup: a cleanup signal must not
        // be misreported as the cause. A closed pipe means ssh is on its way
        // out, so give its own status a bounded moment to land first.
        if failure.peer_closed {
            let _ = tokio::time::timeout(EXIT_GRACE, self.child.wait()).await;
        }
        let state = match self.child.try_wait() {
            Ok(Some(status)) => ChildState::Exited(status.code()),
            Ok(None) => ChildState::Running,
            Err(_) => ChildState::Unknown,
        };
        let _ = self.child.kill().await;
        // Stderr feeds the typed kind and the "Copy details" text only. It never
        // reaches the message, which is persisted and shown verbatim.
        self.stderr.settle(STDERR_GRACE).await;
        let stderr = self.stderr.snapshot();
        let mut classified = classify_failure(failure.code, failure.description, state, &stderr);
        if classified.kind == SshFailureKind::AuthRequired && !self.answered {
            // SSH refuses a key before it runs the remote command, so this bridge never
            // carried a request: say who could not sign in where, not "outcome may be unknown".
            if let Some(target) = &self.sign_in {
                classified.code = SIGN_IN_REFUSED.into();
                classified.description = target.refused();
            }
        }
        anyhow::Error::new(classified)
    }
    pub fn is_usable(&self) -> bool {
        !self.unusable
    }
    /// How long, by the wall clock, since a request last started or finished here.
    pub fn idle_for(&self) -> Duration {
        std::time::SystemTime::now()
            .duration_since(self.last_activity)
            .unwrap_or(Duration::ZERO)
    }
    /// Whether this bridge can no longer carry a request: it was left unusable, or its `ssh`
    /// has exited. Asked **before** a request is written, so a dead bridge is noticed while
    /// nothing has been submitted over it.
    pub fn has_ended(&mut self) -> bool {
        self.unusable || !matches!(self.child.try_wait(), Ok(None))
    }
    pub async fn close(&mut self) {
        self.unusable = true;
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}

fn classify_failure(
    code: String,
    description: &str,
    state: ChildState,
    stderr: &[u8],
) -> SshFailure {
    let text = sanitize_stderr(&String::from_utf8_lossy(stderr));
    let kind = classify(state, &text);
    SshFailure {
        kind,
        code,
        status: state.label(),
        description: description.into(),
        detail: failure_detail(kind, &text),
    }
}

/// The status OpenSSH exits with for every failure of its own, a jump host's
/// included. Any other status is the remote command's.
const SSH_OWN_FAILURE_STATUS: i32 = 255;

/// Map ssh's exit status and its (sanitized) stderr onto a kind. Only a child
/// that has exited is classified: while ssh still runs, the session was
/// established, so its stderr (a banner, the bridge's own logs) cannot explain
/// the failure.
fn classify(state: ChildState, stderr: &str) -> SshFailureKind {
    let ChildState::Exited(code) = state else {
        return SshFailureKind::Other;
    };
    // The remote shell's "command not found" (127) and "cannot execute" (126).
    // ssh itself exits 255, so these statuses can only come from the remote side.
    if matches!(code, Some(126 | 127)) {
        return SshFailureKind::BridgeMissing;
    }
    let lines: Vec<String> = stderr
        .lines()
        .map(str::to_lowercase)
        // A stale ControlPath reports "Control socket connect(...): Connection
        // refused" before ssh falls back to a direct connection; it says nothing
        // about whether the server is reachable.
        .filter(|line| !line.contains("control socket"))
        .collect();

    // Once ssh has connected, its stderr also carries the remote bridge's, and
    // its status is the bridge's. The bridge reports its own startup errors as
    // `Error: <io error>` and exits 1: a stopped broker's stale socket reads
    // "Connection refused (os error 111)", a runtime-directory EACCES reads
    // "Permission denied (os error 13)". Read by the SSH rules those would say
    // the host is unreachable or ask the person to sign in again while SSH is
    // working. So only ssh's own 255 is read as an SSH-level failure. A signal
    // death is not ssh reporting a failure either, so it gets the same treatment.
    if code == Some(SSH_OWN_FAILURE_STATUS) {
        if let Some(kind) = ssh_level_kind(&lines) {
            return kind;
        }
    }
    if any_line(&lines, |line| {
        line.contains("biorouter-crew")
            && [
                "no such file or directory",
                "command not found",
                ": not found",
                "unknown command",
            ]
            .iter()
            .any(|needle| line.contains(needle))
    }) {
        return SshFailureKind::BridgeMissing;
    }
    SshFailureKind::Other
}

/// The kinds only OpenSSH itself can report, in the order they must be tested.
/// The caller has already established that ssh exited with its own status.
fn ssh_level_kind(lines: &[String]) -> Option<SshFailureKind> {
    // The changed banner also ends in "Host key verification failed", so it is
    // tested first; a revoked key or a spoofed-IP warning is the same danger.
    if any_line_contains(
        lines,
        &[
            "remote host identification has changed",
            "revoked host key",
            "possible dns spoofing detected",
            "differs from the key for the ip address",
        ],
    ) || any_line(lines, |line| {
        line.contains("host key for ") && line.contains(" has changed")
    }) {
        return Some(SshFailureKind::HostKeyChanged);
    }
    if any_line_contains(
        lines,
        &["host key verification failed", "host key is known for"],
    ) {
        return Some(SshFailureKind::HostKeyUnknown);
    }
    if any_line_contains(
        lines,
        &[
            "permission denied (",
            "authentication failed",
            "too many authentication failures",
            "no more authentication methods",
            "keyboard-interactive",
        ],
    ) {
        return Some(SshFailureKind::AuthRequired);
    }
    if any_line_contains(
        lines,
        &[
            "could not resolve hostname",
            "name or service not known",
            "nodename nor servname provided",
            "temporary failure in name resolution",
            "connection refused",
            "connection timed out",
            "operation timed out",
            "no route to host",
            "network is unreachable",
            "host is down",
            "open failed: connect failed",
        ],
    ) {
        return Some(SshFailureKind::Unreachable);
    }
    None
}

fn any_line(lines: &[String], test: impl Fn(&str) -> bool) -> bool {
    lines.iter().any(|line| test(line))
}

fn any_line_contains(lines: &[String], needles: &[&str]) -> bool {
    any_line(lines, |line| {
        needles.iter().any(|needle| line.contains(needle))
    })
}

/// Strip terminal escape sequences, control and bidirectional-override
/// characters, keeping one line per diagnostic. A lone carriage return (used
/// to overwrite a line on a terminal) becomes a line break so nothing hides.
fn sanitize_stderr(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => skip_escape_sequence(&mut chars),
            '\n' => out.push('\n'),
            '\r' => {
                if chars.peek() != Some(&'\n') {
                    out.push('\n');
                }
            }
            '\t' => out.push(' '),
            c if c.is_control() || crate::utils::is_invisible_formatting(c) => {}
            c => out.push(c),
        }
    }
    out.lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn skip_escape_sequence(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    match chars.next() {
        // CSI: parameters and intermediates, then one final byte in @..=~.
        Some('[') => {
            for c in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    break;
                }
            }
        }
        // OSC, DCS, SOS, PM, APC: a string ended by BEL or ESC \.
        Some(']' | 'P' | 'X' | '^' | '_') => {
            while let Some(c) = chars.next() {
                if c == '\u{7}' {
                    break;
                }
                if c == '\u{1b}' {
                    if chars.peek() == Some(&'\\') {
                        chars.next();
                    }
                    break;
                }
            }
        }
        // Any other two-character escape: the second character went with it.
        _ => {}
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FingerprintRole {
    Offered,
    Known,
    Unlabeled,
}

/// Every `SHA256:` host-key fingerprint in the text, labelled by the words
/// OpenSSH put around it (the line itself and the one before).
fn host_key_fingerprints(text: &str) -> Vec<(FingerprintRole, String)> {
    const MARKER: &str = "SHA256:";
    let lines: Vec<&str> = text.lines().collect();
    let mut found: Vec<(FingerprintRole, String)> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let previous = index.checked_sub(1).map_or("", |previous| lines[previous]);
        let context = format!("{previous} {line}").to_lowercase();
        let role = if [
            "sent by the remote host",
            "key fingerprint is",
            "server host key",
            "offered",
        ]
        .iter()
        .any(|needle| context.contains(needle))
        {
            FingerprintRole::Offered
        } else if ["known_hosts", "known host", "previously", "expected"]
            .iter()
            .any(|needle| context.contains(needle))
        {
            FingerprintRole::Known
        } else {
            FingerprintRole::Unlabeled
        };
        for (at, _) in line.match_indices(MARKER) {
            let digest: String = line
                .get(at + MARKER.len()..)
                .unwrap_or_default()
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
                .collect();
            // A SHA-256 digest is 43 base64 characters; shorter is not a fingerprint.
            if digest.len() >= 32 {
                let value = format!("{MARKER}{digest}");
                if !found.iter().any(|(_, seen)| *seen == value) {
                    found.push((role, value));
                }
            }
        }
    }
    found
}

/// The "Copy details" text: fingerprints first (so truncation can never cut
/// them), then OpenSSH's own words, head and tail kept, within the limit.
fn failure_detail(kind: SshFailureKind, text: &str) -> Option<String> {
    let mut summary = Vec::new();
    if matches!(
        kind,
        SshFailureKind::HostKeyUnknown | SshFailureKind::HostKeyChanged
    ) {
        for (role, value) in host_key_fingerprints(text)
            .into_iter()
            .take(MAX_FINGERPRINTS)
        {
            summary.push(match (role, kind) {
                (FingerprintRole::Offered, SshFailureKind::HostKeyChanged) => {
                    format!("New host key fingerprint (offered by the server): {value}")
                }
                (FingerprintRole::Offered, _) => {
                    format!("Offered host key fingerprint: {value}")
                }
                (FingerprintRole::Known, _) => {
                    format!("Previously known host key fingerprint: {value}")
                }
                (FingerprintRole::Unlabeled, _) => format!("Host key fingerprint: {value}"),
            });
        }
    }
    let summary = summary.join("\n");
    let detail = match (summary.is_empty(), text.is_empty()) {
        (true, true) => return None,
        (false, true) => summary,
        (true, false) => fit(text, SSH_FAILURE_DETAIL_LIMIT),
        (false, false) => {
            let budget = SSH_FAILURE_DETAIL_LIMIT.saturating_sub(summary.len() + 2);
            format!("{summary}\n\n{}", fit(text, budget))
        }
    };
    Some(fit(detail.trim_end(), SSH_FAILURE_DETAIL_LIMIT))
}

/// Shorten `text` to at most `max` bytes, keeping its opening (a banner, the
/// first complaint) and, mostly, its end, where OpenSSH states the final cause.
fn fit(text: &str, max: usize) -> String {
    const GAP: &str = "\n…\n";
    if text.len() <= max {
        return text.to_string();
    }
    if max <= GAP.len() {
        return String::new();
    }
    let room = max - GAP.len();
    let head_len = room / 4;
    let mut head_end = head_len;
    while !text.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = text.len() - (room - head_len);
    while !text.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    // Both indices sit on character boundaries, so `get` never comes back empty.
    let mut head = text.get(..head_end).unwrap_or_default();
    let mut tail = text.get(tail_start..).unwrap_or_default();
    // Prefer whole lines when that still leaves something on each side.
    if let Some(whole) = head
        .rfind('\n')
        .filter(|cut| *cut > 0)
        .and_then(|cut| head.get(..cut))
    {
        head = whole;
    }
    if let Some(whole) = tail
        .find('\n')
        .filter(|cut| cut + 1 < tail.len())
        .and_then(|cut| tail.get(cut + 1..))
    {
        tail = whole;
    }
    format!("{head}{GAP}{tail}")
}

#[cfg(all(test, unix))]
#[path = "transport_tests.rs"]
mod tests;
