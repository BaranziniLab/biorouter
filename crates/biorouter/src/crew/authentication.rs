//! SSH authentication handoff and workspace admission.
//!
//! **SSH authentication handoff.** Human-only daemon PTYs: a person types their password or
//! verification code into an OpenSSH master this daemon owns, and [`handoff`] adopts that master
//! only after the exact owned process and a verified Crew broker behind it are confirmed. HTTP
//! adapters must prove human authority on every operation. A failed handoff is a
//! [`HandoffFailed`] (`crew_handoff_failed`), whose words are for a person; the older diagnostic
//! text stays reachable through [`HandoffFailed::log_message`].
//!
//! **Workspace admission (S3a).** Joining a workspace by a host's invitation and a device code,
//! as `docs/research/biorouter-crew/naming-design.md` ("Joining a workspace (S3a)", "The
//! invitation", "The device code") specifies. The `impl CrewManager` blocks below add:
//!
//! - [`CrewManager::connection_from_invitation`]: parse a pasted `brcrew1:` invitation (or the
//!   legacy `biorouter-crew status` JSON) and either preview it or save a connection pinned
//!   exactly as it says, through the ordinary save path.
//! - [`CrewManager::invitation_for`]: build the invitation a host sends, from the host's own
//!   verified connection, the workspace's own word about its name and privacy, and `ssh -G`
//!   (never a local alias, never the connection's local name).
//! - [`CrewManager::join_status`]: ask the workspace, unsigned and before authentication, as
//!   `hello` is asked, whether this account is invited, and show the device code **computed
//!   here** from the saved device key and the pinned workspace key.
//! - [`CrewManager::join`]: send `auth.join`, signed with the saved device key, only when the
//!   workspace says the host approved and this computer's own claim was not already refused
//!   under that approval, and only through `CrewManager::signed_join_request`.
//!
//! Nothing the broker returns can change the code this computer shows: a process in the bridge
//! path can relay, drop or fake every answer here, which can only mislead this screen. To have
//! its own key bound it would need a key whose 80-bit code equals the one shown here.
use super::{
    hex, institution, manager, safe_atom, transport, unhex, AuthenticationPlan, ClusterMode,
    Connection, CrewManager, SaveConnection,
};
use anyhow::{ensure, Context, Result};
use biorouter_crew::invitation::{self as crew_invitation, ParsedInvitation, WorkspaceInvitation};
use ed25519_dalek::SigningKey;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fmt,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, LazyLock, Mutex, PoisonError,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

pub const ATTACH_FAILURE_CODE: &str = "authentication_attach_failed";
/// The code older adapters send for a failed handoff. Kept so an older renderer or CLI still
/// maps it; new adapters answer [`HANDOFF_FAILED_CODE`].
pub const HANDOFF_FAILURE_CODE: &str = "authentication_handoff_failed";
pub const ATTACH_FAILURE_MESSAGE: &str = "SSH authentication terminal could not be attached. Close any existing authentication session, verify the daemon's SSH configuration, and try again.";
/// The diagnostic text of a failed handoff, for logs and for [`HANDOFF_FAILURE_CODE`].
pub const HANDOFF_FAILURE_MESSAGE: &str = "SSH authentication could not be handed off to a verified Crew broker. Verify ~/.local/bin/biorouter-crew is installed on the target host and check the saved broker socket/workspace identity, then reconnect.";
/// The typed code of a failed handoff ([`HandoffFailed`]).
pub const HANDOFF_FAILED_CODE: &str = "crew_handoff_failed";
/// What a person reads when sign-in worked but Crew did not start behind it.
pub const HANDOFF_FAILED_MESSAGE: &str =
    "Signed in, but Crew couldn't start on the server. Crew may not be set up for your account there.";

pub fn terminal_failure_message(code: &str) -> Option<&'static str> {
    match code {
        ATTACH_FAILURE_CODE => Some(ATTACH_FAILURE_MESSAGE),
        HANDOFF_FAILURE_CODE => Some(HANDOFF_FAILURE_MESSAGE),
        HANDOFF_FAILED_CODE => Some(HANDOFF_FAILED_MESSAGE),
        _ => None,
    }
}

/// Sign-in succeeded, but adopting the signed-in master or starting Crew behind it failed.
///
/// [`fmt::Display`] is [`HANDOFF_FAILED_MESSAGE`], written for a person; the cause and the older
/// diagnostic text stay out of it and are reachable through [`Self::log_message`] and
/// [`Self::cause`]. A route answers [`Self::api_code`].
#[derive(Debug)]
pub struct HandoffFailed {
    cause: anyhow::Error,
}

impl HandoffFailed {
    fn wrap(cause: anyhow::Error) -> anyhow::Error {
        anyhow::Error::new(Self { cause })
    }
    /// Always [`HANDOFF_FAILED_CODE`].
    pub fn api_code(&self) -> &'static str {
        HANDOFF_FAILED_CODE
    }
    /// Why the handoff failed, unchanged, e.g. an [`super::SshFailure`] or a
    /// [`super::WorkspaceIdentityError`] from the connect that followed sign-in.
    pub fn cause(&self) -> &anyhow::Error {
        &self.cause
    }
    /// Whether the broker behind the signed-in master is not the workspace this connection
    /// pinned. A route may prefer `crew_workspace_identity_mismatch` for this cause: it is a
    /// trust problem, not a missing installation.
    pub fn workspace_identity_mismatch(&self) -> bool {
        self.cause
            .downcast_ref::<super::WorkspaceIdentityError>()
            .is_some()
    }
    /// The diagnostic text for logs: [`HANDOFF_FAILURE_MESSAGE`] and the cause.
    pub fn log_message(&self) -> String {
        format!("{HANDOFF_FAILURE_MESSAGE} Cause: {:#}", self.cause)
    }
}

impl fmt::Display for HandoffFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(HANDOFF_FAILED_MESSAGE)
    }
}

impl std::error::Error for HandoffFailed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.cause.as_ref())
    }
}

static SESSIONS: LazyLock<Mutex<HashMap<String, Arc<AuthSession>>>> =
    LazyLock::new(Default::default);
static INSTANCE: LazyLock<String> = LazyLock::new(|| uuid::Uuid::new_v4().to_string());

#[derive(Clone, Serialize)]
pub struct AuthenticationSession {
    pub authentication_id: String,
    pub connection_id: String,
    pub controller_id: String,
    pub instance_id: String,
}
pub enum TerminalEvent {
    Data(Vec<u8>),
    Exit(Option<u32>),
}
struct Runtime {
    child: Arc<Mutex<OwnedChild>>,
    master: Box<dyn MasterPty + Send>,
    input: std::sync::mpsc::SyncSender<Vec<u8>>,
    pid: u32,
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.child.lock().unwrap().kill();
    }
}
// portable-pty may reap inside kill; all signals and status checks share this
// mutex, and a terminal or uncertain outcome permanently disarms the PID.
struct OwnedChild {
    child: Box<dyn Child + Send>,
    terminal_status: Option<Option<u32>>,
}
impl OwnedChild {
    fn kill(&mut self) {
        if self.poll_reap().is_some() {
            return;
        }
        let failed = self.child.kill().is_err();
        if self.poll_reap().is_none() && failed {
            self.terminal_status = Some(None);
        }
    }
    fn poll_reap(&mut self) -> Option<Option<u32>> {
        if self.terminal_status.is_some() {
            return self.terminal_status;
        }
        self.terminal_status = match self.child.try_wait() {
            Ok(None) => None,
            Ok(Some(status)) => Some(Some(status.exit_code())),
            Err(_) => Some(None),
        };
        self.terminal_status
    }
}
struct AuthSession {
    info: AuthenticationSession,
    request_id: String,
    binding: Mutex<Vec<u8>>,
    adopted: Arc<AtomicBool>,
    plan: AuthenticationPlan,
    created: Instant,
    size: PtySize,
    started: Mutex<bool>,
    runtime: Mutex<Option<Runtime>>,
}
fn foreground_plan(mut plan: AuthenticationPlan) -> AuthenticationPlan {
    for argument in &mut plan.args {
        if argument.starts_with("ControlPersist=") {
            *argument = "ControlPersist=no".into();
        }
    }
    plan
}

fn dimensions(cols: u16, rows: u16) -> Result<PtySize> {
    ensure!(
        (20..=500).contains(&cols) && (5..=200).contains(&rows),
        "Invalid authentication terminal size"
    );
    Ok(PtySize {
        cols,
        rows,
        pixel_width: 0,
        pixel_height: 0,
    })
}
async fn binding(connection: &str) -> Result<Vec<u8>> {
    let connection = manager()?.connection(connection).await?;
    Ok(
        Sha256::digest(serde_json::to_vec(&super::connection_binding(
            &connection,
        )?)?)
        .to_vec(),
    )
}

pub async fn prepare(
    connection: &str,
    request: &str,
    controller: &str,
    cols: u16,
    rows: u16,
) -> Result<AuthenticationSession> {
    uuid::Uuid::parse_str(request)?;
    uuid::Uuid::parse_str(controller)?;
    let size = dimensions(cols, rows)?;
    let manager = manager()?;
    let _lifecycle = manager.connection_guard(connection).await?;
    let plan = foreground_plan(manager.authentication_plan(connection).await?);
    let binding = binding(connection).await?;
    let mut sessions = SESSIONS
        .lock()
        .map_err(|_| anyhow::anyhow!("Authentication state unavailable"))?;
    sessions.retain(|_, session| {
        session.created.elapsed() < Duration::from_secs(60) || *session.started.lock().unwrap()
    });
    for session in sessions.values() {
        if session.request_id == request && session.info.controller_id == controller {
            ensure!(
                session.info.connection_id == connection
                    && *session.binding.lock().unwrap() == binding,
                "Authentication request changed; use a fresh request ID"
            );
            return Ok(session.info.clone());
        }
        ensure!(session.info.connection_id != connection, "This connection already has an authentication controller; close it before opening another");
    }
    ensure!(
        sessions.len() < 32,
        "Close an existing SSH authentication session first"
    );
    let info = AuthenticationSession {
        authentication_id: uuid::Uuid::new_v4().to_string(),
        connection_id: connection.into(),
        controller_id: controller.into(),
        instance_id: INSTANCE.clone(),
    };
    sessions.insert(
        info.authentication_id.clone(),
        Arc::new(AuthSession {
            info: info.clone(),
            request_id: request.into(),
            binding: Mutex::new(binding),
            adopted: Arc::new(AtomicBool::new(false)),
            plan,
            created: Instant::now(),
            size,
            started: Mutex::new(false),
            runtime: Mutex::new(None),
        }),
    );
    Ok(info)
}
/// Called while holding the connection lifecycle guard; only handoff may connect
/// a pending native authentication session and change its verified node/epoch.
pub(super) fn ensure_connect_available(connection: &str) -> Result<()> {
    let mut sessions = SESSIONS
        .lock()
        .map_err(|_| anyhow::anyhow!("Authentication state unavailable"))?;
    sessions.retain(|_, session| {
        session.created.elapsed() < Duration::from_secs(60) || *session.started.lock().unwrap()
    });
    ensure!(!sessions.values().any(|session| session.info.connection_id == connection && !session.adopted.load(Ordering::Acquire)),
        "Native SSH authentication is still pending. Wait for its verified daemon completion, or close authentication before connecting again");
    Ok(())
}

fn session(id: &str, controller: &str) -> Result<Arc<AuthSession>> {
    let session = SESSIONS
        .lock()
        .map_err(|_| anyhow::anyhow!("Authentication state unavailable"))?
        .get(id)
        .cloned()
        .context("Authentication session unavailable")?;
    ensure!(
        session.info.controller_id == controller,
        "Authentication controller mismatch"
    );
    Ok(session)
}
pub async fn validate(id: &str, controller: &str) -> Result<String> {
    let session = session(id, controller)?;
    let current = binding(&session.info.connection_id).await?;
    ensure!(
        *session.binding.lock().unwrap() == current,
        "Saved connection changed; close authentication and start again"
    );
    Ok(session.info.connection_id.clone())
}
pub async fn attach(id: &str, controller: &str) -> Result<mpsc::Receiver<TerminalEvent>> {
    let connection = session(id, controller)?.info.connection_id.clone();
    let manager = manager()?;
    let _lifecycle = manager.connection_guard(&connection).await?;
    validate(id, controller).await?;
    let session = session(id, controller)?;
    let fresh = foreground_plan(
        manager
            .authentication_plan(&session.info.connection_id)
            .await?,
    );
    ensure!(
        fresh.args == session.plan.args,
        "SSH authentication plan changed; start again"
    );
    validate(id, controller).await?;
    let mut started = session
        .started
        .lock()
        .map_err(|_| anyhow::anyhow!("Authentication state unavailable"))?;
    ensure!(
        !*started && session.created.elapsed() < Duration::from_secs(60),
        "Authentication session already attached or expired"
    );
    let (runtime, output) = spawn(&session.plan, session.size, session.adopted.clone())?;
    *session.runtime.lock().unwrap() = Some(runtime);
    *started = true;
    Ok(output)
}
fn command(plan: &AuthenticationPlan) -> CommandBuilder {
    let mut command = CommandBuilder::new("ssh");
    command.args(&plan.args);
    command.env_clear();
    let profile = std::env::var_os("BIOROUTER_DEV_PROFILE_ROOT");
    for (key, value) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if biorouter_mcp::developer::shell::is_daemon_private_env_key(&name) {
            continue;
        }
        if profile.is_some()
            && !matches!(
                name.as_ref(),
                "PATH" | "LANG" | "LC_ALL" | "SystemRoot" | "WINDIR" | "ComSpec" | "PATHEXT"
            )
        {
            continue;
        }
        command.env(key, value);
    }
    if let Some(profile) = profile {
        let home = std::path::PathBuf::from(profile).join("home");
        command.env("HOME", &home);
        command.env("USERPROFILE", &home);
        command.cwd(home);
    }
    command.env("TERM", "xterm-256color");
    command
}
fn spawn(
    plan: &AuthenticationPlan,
    size: PtySize,
    adopted: Arc<AtomicBool>,
) -> Result<(Runtime, mpsc::Receiver<TerminalEvent>)> {
    let pair = native_pty_system().openpty(size)?;
    let mut reader = pair.master.try_clone_reader()?;
    let mut writer = pair.master.take_writer()?;
    let child = pair.slave.spawn_command(command(plan))?;
    let pid = child
        .process_id()
        .context("SSH child process identity unavailable")?;
    let child = Arc::new(Mutex::new(OwnedChild {
        child,
        terminal_status: None,
    }));
    let reader_child = child.clone();
    let writer_child = child.clone();
    drop(pair.slave);
    let (input, receive_input) = std::sync::mpsc::sync_channel::<Vec<u8>>(16);
    let (output, receive_output) = mpsc::channel(16);
    std::thread::spawn(move || {
        while let Ok(mut bytes) = receive_input.recv() {
            let result = writer.write_all(&bytes).and_then(|_| writer.flush());
            bytes.fill(0);
            if result.is_err() {
                writer_child.lock().unwrap().kill();
                break;
            }
        }
    });
    std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    if adopted.load(Ordering::Acquire) {
                        buffer.fill(0);
                        continue;
                    }
                    if output
                        .blocking_send(TerminalEvent::Data(buffer[..count].to_vec()))
                        .is_err()
                        && !adopted.load(Ordering::Acquire)
                    {
                        break;
                    }
                    buffer.fill(0);
                }
            }
        }
        buffer.fill(0);
        reader_child.lock().unwrap().kill();
        let status = loop {
            if let Some(status) = reader_child.lock().unwrap().poll_reap() {
                break status;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let _ = output.blocking_send(TerminalEvent::Exit(status));
    });
    Ok((
        Runtime {
            child,
            master: pair.master,
            input,
            pid,
        },
        receive_output,
    ))
}
pub fn input(id: &str, controller: &str, data: &str) -> Result<()> {
    ensure!(
        data.len() <= 4096,
        "Authentication input exceeds frame limit"
    );
    let session = session(id, controller)?;
    let runtime = session.runtime.lock().unwrap();
    runtime
        .as_ref()
        .context("Authentication terminal is not running")?
        .input
        .try_send(data.as_bytes().to_vec())
        .map_err(|_| anyhow::anyhow!("Authentication input queue is full or closed"))?;
    Ok(())
}
pub fn resize(id: &str, controller: &str, cols: u16, rows: u16) -> Result<()> {
    let size = dimensions(cols, rows)?;
    let session = session(id, controller)?;
    let runtime = session.runtime.lock().unwrap();
    runtime
        .as_ref()
        .context("Authentication terminal is not running")?
        .master
        .resize(size)
}
fn cancel(id: &str, controller: &str) -> Result<String> {
    let session = session(id, controller)?;
    SESSIONS.lock().unwrap().remove(id);
    *session.started.lock().unwrap() = true;
    session.runtime.lock().unwrap().take();
    Ok(session.info.connection_id.clone())
}
pub fn cancel_connection(connection: &str) {
    let removed: Vec<_> = {
        let mut sessions = SESSIONS.lock().unwrap();
        let ids: Vec<_> = sessions
            .iter()
            .filter(|(_, s)| s.info.connection_id == connection)
            .map(|(id, _)| id.clone())
            .collect();
        ids.iter().filter_map(|id| sessions.remove(id)).collect()
    };
    for session in removed {
        *session.started.lock().unwrap() = true;
        session.runtime.lock().unwrap().take();
    }
}
pub fn shutdown() {
    let sessions: Vec<_> = SESSIONS.lock().unwrap().drain().map(|(_, s)| s).collect();
    for session in sessions {
        *session.started.lock().unwrap() = true;
        session.runtime.lock().unwrap().take();
    }
}

/// Complete native authentication only after the exact owned master and broker are verified.
///
/// A failure after the session is found is a [`HandoffFailed`]: the connection is disconnected
/// and its `last_error` carries [`HANDOFF_FAILED_MESSAGE`], while the diagnostic text and cause
/// go to the log.
pub async fn handoff(id: &str, controller: &str) -> Result<bool> {
    let connection = session(id, controller)?.info.connection_id.clone();
    let manager = manager()?;
    let _lifecycle = manager.connection_guard(&connection).await?;
    // A replaced or cancelled session must not clean up a later connection.
    session(id, controller)?;
    match handoff_locked(id, controller).await {
        Ok(adopted) => Ok(adopted),
        Err(cause) => {
            let _ = manager.disconnect_locked(&connection).await;
            let mut registry = manager.registry.lock().await;
            if let Some(entry) = registry.connections.iter_mut().find(|c| c.id == connection) {
                entry.last_error = Some(HANDOFF_FAILED_MESSAGE.into());
            }
            drop(registry);
            let failure = HandoffFailed::wrap(cause);
            if let Some(typed) = failure.downcast_ref::<HandoffFailed>() {
                tracing::warn!(connection = %connection, "{}", typed.log_message());
            }
            Err(failure)
        }
    }
}
async fn handoff_locked(id: &str, controller: &str) -> Result<bool> {
    validate(id, controller).await?;
    let session = session(id, controller)?;
    if session.adopted.load(Ordering::Acquire) {
        return Ok(true);
    }
    let pid = session
        .runtime
        .lock()
        .unwrap()
        .as_ref()
        .context("Authentication terminal unavailable")?
        .pid;
    let control = session
        .plan
        .args
        .windows(2)
        .find(|args| args[0] == "-S")
        .context("Owned SSH control path missing")?[1]
        .clone();
    let target = manager()?
        .connection(&session.info.connection_id)
        .await?
        .ssh_target;
    if !master_ready(&control, &target, pid).await? {
        return Ok(false);
    }
    validate(id, controller).await?;
    let connected = manager()?
        .connect_locked(&session.info.connection_id)
        .await?;
    let expected =
        Sha256::digest(serde_json::to_vec(&super::connection_binding(&connected)?)?).to_vec();
    ensure!(
        binding(&session.info.connection_id).await? == expected,
        "Connection changed during authentication handoff"
    );
    let sessions = SESSIONS.lock().unwrap();
    ensure!(
        sessions
            .get(id)
            .is_some_and(|current| Arc::ptr_eq(current, &session)),
        "Authentication was cancelled during handoff"
    );
    ensure!(
        session.runtime.lock().unwrap().is_some(),
        "Authentication process ended during handoff"
    );
    *session.binding.lock().unwrap() = expected;
    session.adopted.store(true, Ordering::Release);
    Ok(true)
}
async fn master_ready(control: &str, target: &str, pid: u32) -> Result<bool> {
    use tokio::io::AsyncReadExt;
    let mut command = tokio::process::Command::new("ssh");
    command
        .args(["-F", "none", "-S", control, "-O", "check", target])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    crate::subprocess::prepare_agent_child_command(&mut command);
    let mut child = command.spawn()?;
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        let mut bytes = Vec::new();
        child
            .stderr
            .take()
            .context("SSH control response unavailable")?
            .take(4097)
            .read_to_end(&mut bytes)
            .await?;
        ensure!(bytes.len() <= 4096, "SSH control response exceeded limit");
        let status = child.wait().await?;
        let text = std::str::from_utf8(&bytes).unwrap_or("").trim();
        Ok::<_, anyhow::Error>(status.success() && text == format!("Master running (pid={pid})"))
    })
    .await;
    match result {
        Ok(result) => result,
        Err(_) => Ok(false),
    }
}
/// Complete teardown under the same connection generation used for adoption.
pub async fn detach(id: &str, controller: &str) -> Result<()> {
    let connection = session(id, controller)?.info.connection_id.clone();
    let manager = manager()?;
    let _lifecycle = manager.connection_guard(&connection).await?;
    let current = session(id, controller)?;
    if !current.adopted.load(Ordering::Acquire) {
        cancel(id, controller)?;
    }
    Ok(())
}
pub async fn cancel_and_disconnect(id: &str, controller: &str) -> Result<()> {
    let connection = session(id, controller)?.info.connection_id.clone();
    let manager = manager()?;
    let _lifecycle = manager.connection_guard(&connection).await?;
    session(id, controller)?;
    cancel(id, controller)?;
    manager.disconnect_locked(&connection).await
}

// ---------------------------------------------------------------------------------------------
// Workspace admission (S3a)
// ---------------------------------------------------------------------------------------------

/// The `hello` capability of a broker that answers `enrollment.pending` and `auth.join`.
pub const JOIN_BY_NAME_CAPABILITY: &str = "join_by_name_v1";
/// How the transport reports a broker's refusal: this prefix, then the error envelope as JSON.
const BROKER_REFUSAL_PREFIX: &str = "Crew broker refused request: ";
/// The longest join ID accepted from a broker (it is 128 random bits; this is generous).
const MAX_JOIN_ID_BYTES: usize = 128;
/// The longest connection name the save path accepts.
const MAX_CONNECTION_NAME_BYTES: usize = 120;
/// How long `ssh -G` may take to describe the host's SSH settings.
const SSH_RESOLVE_TIMEOUT: Duration = Duration::from_secs(10);
/// The most output `ssh -G` may print before it is refused.
const SSH_RESOLVE_LIMIT: u64 = 1_048_576;
/// At most this many jump hosts are described in an invitation.
const MAX_JUMP_HOPS: usize = 16;

/// Invitation saves run one at a time, so a double-submitted paste finds the connection the
/// first one saved instead of minting a second device key (and so a second device code).
static INVITATION_SAVES: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(Default::default);

/// What the person chose on the Join screen, beside the pasted invitation. Everything is
/// optional: an absent value takes the invitation's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InvitationOverrides {
    /// The joiner's account name on the server. Default: the invitation's invited username.
    #[serde(default)]
    pub username: Option<String>,
    /// How this computer treats the workspace. Default: the workspace's own mode, else Private.
    #[serde(default)]
    pub mode: Option<ClusterMode>,
    /// Default: the invitation's institution.
    #[serde(default)]
    pub institution_id: Option<String>,
    #[serde(default)]
    pub advanced: InvitationAdvanced,
}

/// The Join screen's Advanced settings. None of them can change the pinned workspace.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InvitationAdvanced {
    /// A server login from the person's own SSH settings (`hpc`, `bob@hpc.ucsf.edu`), used
    /// instead of `{username}@{server}`. With one, the invitation's port and jump host are not
    /// applied: the person's SSH settings for that login decide them.
    #[serde(default)]
    pub ssh_target: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub identity_file: Option<String>,
    /// A jump route. An empty string means none, even when the invitation suggests one.
    #[serde(default)]
    pub proxy_jump: Option<String>,
    /// This computer's name for the connection. Default: the workspace's name.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub remote_root: Option<String>,
    #[serde(default)]
    pub remote_execution: bool,
    /// A prepared hosting identity (`POST /crew/devices/prepare`), when a host saves their own
    /// workspace from what `biorouter-crew start` printed.
    #[serde(default)]
    pub preparation_id: Option<String>,
}

/// Where a previewed invitation came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InvitationSourceKind {
    /// A `brcrew1:` line, alone or inside the host's message.
    Invitation,
    /// The JSON `biorouter-crew status` prints.
    LegacyStatus,
}

/// Something saving still needs, which the invitation did not say and the person has not given.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InvitationMissing {
    /// The joiner's username on the server.
    Username,
    /// The server's address (a legacy status JSON, or `start` output, names none).
    Server,
    /// An institution, which a Private connection requires.
    Institution,
}

/// A parsed invitation and what saving it would do. Labels are not authority: nothing here is
/// trusted until `hello` verifies against the pinned workspace key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct InvitationPreview {
    pub source: InvitationSourceKind,
    /// The pinned workspace: shown under Advanced only.
    pub workspace_id: String,
    pub socket_path: String,
    pub owner_uid: u32,
    /// SHA-256 of the workspace key, lowercase hex.
    pub workspace_key_fingerprint: String,
    /// The short form a person compares by eye (`3F2A 9C1E 77B0 D4E1`).
    pub fingerprint: String,
    pub workspace_name: Option<String>,
    /// How to name the workspace: its name, else "{host}'s workspace", else "a workspace".
    pub workspace_label: String,
    pub host_username: Option<String>,
    pub host_display_name: Option<String>,
    /// The workspace's own privacy mode, as the invitation states it.
    pub workspace_mode: Option<ClusterMode>,
    pub workspace_institution_id: Option<String>,
    /// The server named by the invitation.
    pub server: Option<String>,
    pub invitee_username: Option<String>,
    /// What saving would use: the username, the SSH login and route, and the privacy.
    pub username: Option<String>,
    pub ssh_target: Option<String>,
    pub port: Option<u16>,
    pub proxy_jump: Option<String>,
    pub mode: ClusterMode,
    pub institution_id: Option<String>,
    /// The chosen mode differs from the workspace's own.
    pub mode_differs: bool,
    /// The connection name saving would use.
    pub name: String,
    /// A connection on this computer that already pins this workspace.
    pub existing_connection_id: Option<String>,
    /// What saving still needs; empty when it can save.
    pub missing: Vec<InvitationMissing>,
    /// Another saved connection reaches the same server under a different institution, in
    /// people's words. Saving is still allowed; connecting would be refused, because one
    /// computer can't mix institutions on one server (T-52).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub institution_conflict: Option<String>,
    /// What to call the server on screen (D-ALIAS): the person's own SSH alias for the address
    /// saving would use, when one maps to it, else that address's host. Display only; `server`
    /// and `ssh_target` stay the invitation's resolved address. See [`super::server_label`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_label: Option<String>,
}

/// The result of [`CrewManager::connection_from_invitation`].
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum InvitationOutcome {
    /// `preview: true`: nothing was saved.
    Preview(Box<InvitationPreview>),
    /// The saved connection (or the one already on this computer for the same workspace and
    /// settings).
    Saved(Box<Connection>),
}

/// Why an invitation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvitationRefusal {
    /// The pasted text is not a usable invitation; the invitation codec's own code.
    Unreadable(&'static str),
    /// A value the person typed or chose can't be used.
    InvalidChoice,
    /// Saving needs something neither the invitation nor the person gave.
    Missing(InvitationMissing),
    /// This computer already pins a different identity for the same workspace ID. Never
    /// re-pinned from a paste.
    IdentityConflict,
    /// This computer already has the workspace, with different settings.
    AlreadySaved,
}

/// A refused invitation. [`fmt::Display`] is a plain sentence that never echoes the pasted text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvitationRefused {
    reason: InvitationRefusal,
    message: String,
    connection_id: Option<String>,
}

impl InvitationRefused {
    fn new(reason: InvitationRefusal, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
            connection_id: None,
        }
    }
    fn choice(message: impl Into<String>) -> Self {
        Self::new(InvitationRefusal::InvalidChoice, message)
    }
    /// `crew_invitation_invalid` for anything wrong with the paste or the choices,
    /// `crew_invitation_conflict` for a different pinned identity, `crew_connection_exists` for
    /// the same workspace saved with other settings.
    pub fn api_code(&self) -> &'static str {
        match self.reason {
            InvitationRefusal::Unreadable(_)
            | InvitationRefusal::InvalidChoice
            | InvitationRefusal::Missing(_) => "crew_invitation_invalid",
            InvitationRefusal::IdentityConflict => "crew_invitation_conflict",
            InvitationRefusal::AlreadySaved => "crew_connection_exists",
        }
    }
    pub fn reason(&self) -> InvitationRefusal {
        self.reason
    }
    /// The saved connection a conflict or duplicate concerns.
    pub fn connection_id(&self) -> Option<&str> {
        self.connection_id.as_deref()
    }
}

impl fmt::Display for InvitationRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for InvitationRefused {}

/// The invitation a host sends: the whole message, and its `brcrew1:` line alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct InvitationText {
    pub message: String,
    pub line: String,
}

/// Where this computer stands in joining a workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JoinState {
    /// Invited; the host has not approved this computer's code yet.
    Invited,
    /// The host approved a code; [`CrewManager::join`] can claim.
    Approved,
    /// The host approved a code, and this computer's claim was refused under it.
    CodeMismatch,
    /// This account has no invitation and is not a member.
    NotInvited,
    Expired,
    /// This computer's key is a member's device.
    Joined,
    /// The workspace does not support joining by invitation (`join_by_name_v1` absent).
    Unsupported,
}

/// A person named in a join status. Labels only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct JoinPerson {
    pub username: String,
    /// The username when they set no display name of their own.
    pub display_name: String,
}

/// The join status the Join screen shows.
///
/// `code` is computed on this computer from its saved device key and the pinned workspace key.
/// It is never read from the workspace's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct JoinStatus {
    pub status: JoinState,
    /// This computer's device code (`7QK2-M9XA-3JTP-WZ4D`), while invited, approved or refused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inviter: Option<JoinPerson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_name: Option<String>,
    /// When the invitation expires (seconds since the Unix epoch).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    /// The invitation adds this computer to an existing member.
    pub add_device: bool,
}

impl JoinStatus {
    fn bare(status: JoinState, workspace_name: Option<String>) -> Self {
        Self {
            status,
            code: None,
            inviter: None,
            workspace_name,
            expires_at: None,
            add_device: false,
        }
    }
}

/// Why [`CrewManager::join`] did not join.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JoinRefusal {
    Unsupported,
    /// The host has not approved this computer's code yet.
    NotApproved,
    CodeMismatch,
    NotInvited,
    Expired,
    /// The host replaced the invitation.
    Replaced,
    /// The server account changed since the host invited it.
    AccountChanged,
    /// This computer's key is already a device of the workspace.
    DeviceConflict,
    /// Another active member has this username.
    IdentityConflict,
    /// Any other refusal from the workspace.
    Refused,
}

impl JoinRefusal {
    fn from_broker_code(code: &str) -> Self {
        match code {
            "code_mismatch" => Self::CodeMismatch,
            "not_invited" => Self::NotInvited,
            "join_expired" => Self::Expired,
            "join_changed" => Self::Replaced,
            "account_changed" => Self::AccountChanged,
            "device_conflict" => Self::DeviceConflict,
            "identity_conflict" => Self::IdentityConflict,
            "unsupported" => Self::Unsupported,
            _ => Self::Refused,
        }
    }
    fn from_state(state: JoinState) -> Option<Self> {
        match state {
            JoinState::Invited => Some(Self::NotApproved),
            JoinState::CodeMismatch => Some(Self::CodeMismatch),
            JoinState::NotInvited => Some(Self::NotInvited),
            JoinState::Expired => Some(Self::Expired),
            JoinState::Unsupported => Some(Self::Unsupported),
            JoinState::Approved | JoinState::Joined => None,
        }
    }
    pub fn api_code(self) -> &'static str {
        match self {
            Self::Unsupported => "crew_join_unsupported",
            Self::NotApproved => "crew_join_not_approved",
            Self::CodeMismatch => "crew_join_code_mismatch",
            Self::NotInvited => "crew_join_not_invited",
            Self::Expired => "crew_join_expired",
            Self::Replaced => "crew_join_replaced",
            Self::AccountChanged => "crew_join_account_changed",
            Self::DeviceConflict => "crew_join_device_conflict",
            Self::IdentityConflict => "crew_join_identity_conflict",
            Self::Refused => "crew_join_refused",
        }
    }
    fn message(self) -> &'static str {
        match self {
            Self::Unsupported => "This workspace's server doesn't support joining by invitation yet. Ask your host for an invitation token instead.",
            Self::NotApproved => "Your host hasn't let this computer in yet. Send them the code shown on your screen.",
            Self::CodeMismatch => "The code your host entered doesn't match this computer. Send them the code shown on your screen again.",
            Self::NotInvited => "You're not invited to this workspace yet. Ask your host to invite you.",
            Self::Expired => "This invitation expired. Ask your host to invite you again.",
            Self::Replaced => "Your host sent a new invitation. Check your join status and try again.",
            Self::AccountChanged => "Your account on the server changed since your host invited it. Ask your host to invite you again.",
            Self::DeviceConflict => "This computer's key is already in this workspace.",
            Self::IdentityConflict => "Another member of this workspace already has your username. Ask your host to remove the old account first.",
            Self::Refused => "The workspace didn't let this computer join.",
        }
    }
}

/// A join the workspace (or this computer, before asking it) refused. [`fmt::Display`] is a
/// plain sentence chosen here; the workspace's own words, which are unauthenticated, are kept
/// apart in [`Self::broker_message`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinRefused {
    refusal: JoinRefusal,
    broker_message: Option<String>,
}

impl JoinRefused {
    fn error(refusal: JoinRefusal) -> anyhow::Error {
        anyhow::Error::new(Self {
            refusal,
            broker_message: None,
        })
    }
    pub fn refusal(&self) -> JoinRefusal {
        self.refusal
    }
    pub fn api_code(&self) -> &'static str {
        self.refusal.api_code()
    }
    /// The workspace's refusal text, for logs only.
    pub fn broker_message(&self) -> Option<&str> {
        self.broker_message.as_deref()
    }
}

impl fmt::Display for JoinRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.refusal.message())
    }
}

impl std::error::Error for JoinRefused {}

/// The workspace's refusal as the transport reports it: the envelope's `code` and `message`.
/// `None` for anything else (an SSH failure, a local check).
fn broker_refusal(error: &anyhow::Error) -> Option<(String, String)> {
    let text = error.to_string();
    let envelope: Value = serde_json::from_str(text.strip_prefix(BROKER_REFUSAL_PREFIX)?).ok()?;
    Some((
        envelope.get("code")?.as_str()?.to_owned(),
        envelope.get("message")?.as_str()?.to_owned(),
    ))
}

fn cluster_mode(mode: &biorouter_crew::Mode) -> ClusterMode {
    match mode {
        biorouter_crew::Mode::Private => ClusterMode::Private,
        biorouter_crew::Mode::Public => ClusterMode::Public,
    }
}

fn crew_mode(mode: ClusterMode) -> biorouter_crew::Mode {
    match mode {
        ClusterMode::Private => biorouter_crew::Mode::Private,
        ClusterMode::Public => biorouter_crew::Mode::Public,
    }
}

/// `value` cut to at most `limit` bytes on a character boundary.
fn truncate_bytes(value: &str, limit: usize) -> String {
    let mut out = String::new();
    for c in value.chars() {
        if out.len() + c.len_utf8() > limit {
            break;
        }
        out.push(c);
    }
    out
}

/// A typed person's username: one leading `@` stripped, then the account-name rules.
fn typed_username(typed: &str, server: Option<&str>) -> Result<String, InvitationRefused> {
    let name = typed.strip_prefix('@').unwrap_or(typed);
    if biorouter_crew::valid_username(name) {
        Ok(name.to_owned())
    } else {
        Err(InvitationRefused::choice(format!(
            "Type your username on {}, with no spaces, slashes or colons.",
            server.unwrap_or("the server")
        )))
    }
}

fn same_workspace_id(saved: &str, invited: &str) -> bool {
    match (uuid::Uuid::parse_str(saved), uuid::Uuid::parse_str(invited)) {
        (Ok(saved), Ok(invited)) => saved == invited,
        _ => false,
    }
}

/// The saved connection that already pins this invitation's workspace, if any. A saved
/// connection with the same workspace ID but another key, socket or host is a conflict: a paste
/// never re-pins a workspace.
fn saved_match(
    connections: &[Connection],
    invitation: &WorkspaceInvitation,
) -> Result<Option<Connection>, InvitationRefused> {
    let mut found = None;
    for saved in connections
        .iter()
        .filter(|saved| same_workspace_id(&saved.workspace_id, &invitation.workspace_id))
    {
        if !saved
            .workspace_public_key
            .eq_ignore_ascii_case(&invitation.workspace_public_key)
            || saved.socket_path != invitation.socket_path
            || saved.owner_uid != invitation.owner_uid
        {
            let mut refused = InvitationRefused::new(
                InvitationRefusal::IdentityConflict,
                format!(
                    "This invitation doesn't match \u{201c}{}\u{201d}, which this computer already has for the same workspace. Ask your host to send it again, and compare the fingerprint.",
                    super::plain_label(&saved.name)
                ),
            );
            refused.connection_id = Some(saved.id.clone());
            return Err(refused);
        }
        if found.is_none() {
            found = Some(saved.clone());
        }
    }
    Ok(found)
}

/// Whether an existing connection already has what saving `input` would give it. The name, the
/// remote folder and agent execution are this computer's own choices and are not compared.
fn same_settings(existing: &Connection, input: &SaveConnection) -> bool {
    existing.ssh_target == input.ssh_target
        && existing.port == input.port
        && existing.identity_file == input.identity_file
        && existing.proxy_jump == input.proxy_jump
        && existing.mode == input.mode
        && existing.institution_id == input.institution_id
}

/// A parsed invitation, resolved against the person's choices and this computer's connections.
struct InvitationPlan {
    preview: InvitationPreview,
    save: SaveConnection,
    existing: Option<Connection>,
}

/// The SSH login and route saving would use.
struct PlannedRoute {
    username: Option<String>,
    ssh_target: Option<String>,
    port: Option<u16>,
    proxy_jump: Option<String>,
}

fn planned_route(
    invitation: &WorkspaceInvitation,
    overrides: &InvitationOverrides,
) -> Result<PlannedRoute, InvitationRefused> {
    let server = invitation.ssh_host.as_deref();
    let advanced = &overrides.advanced;
    let username = match overrides.username.as_deref().map(str::trim) {
        Some(typed) if !typed.is_empty() => Some(typed_username(typed, server)?),
        _ => invitation.invitee_username.clone(),
    };
    let alias = advanced
        .ssh_target
        .as_deref()
        .map(str::trim)
        .filter(|alias| !alias.is_empty());
    if alias.is_some_and(|alias| !safe_atom(alias)) {
        return Err(InvitationRefused::choice(
            "Type a server login from your SSH settings, like hpc or bob@hpc.ucsf.edu.",
        ));
    }
    let ssh_target = match (alias, &username, server) {
        (Some(alias), _, _) => Some(alias.to_owned()),
        (None, Some(user), Some(host)) => {
            let target = format!("{user}@{host}");
            if !safe_atom(&target) {
                return Err(InvitationRefused::choice(format!(
                    "@{user} can't be used as an SSH login. Set a server login under Advanced instead."
                )));
            }
            Some(target)
        }
        _ => None,
    };
    if advanced.port == Some(0) {
        return Err(InvitationRefused::choice(
            "Choose a port between 1 and 65535.",
        ));
    }
    let jump = match advanced.proxy_jump.as_deref().map(str::trim) {
        Some("") => None,
        Some(route) => {
            if !route.split(',').all(safe_atom) {
                return Err(InvitationRefused::choice(
                    "Type jump hosts as host names separated by commas, like gateway.ucsf.edu.",
                ));
            }
            Some(route.to_owned())
        }
        // The invitation's hints describe its own server; a login from the person's SSH
        // settings brings its own port and route.
        None if alias.is_none() => invitation.proxy_jump.clone(),
        None => None,
    };
    let port = match (advanced.port, alias) {
        (Some(port), _) => Some(port),
        (None, None) => invitation.ssh_port,
        (None, Some(_)) => None,
    };
    Ok(PlannedRoute {
        username,
        ssh_target,
        port,
        proxy_jump: jump,
    })
}

/// The connection name saving would use: the person's, else the workspace's, qualified by the
/// server when another saved connection already has that name.
fn planned_name(
    invitation: &WorkspaceInvitation,
    advanced: &InvitationAdvanced,
    route: &PlannedRoute,
    connections: &[Connection],
    existing: Option<&Connection>,
) -> Result<String, InvitationRefused> {
    if let Some(name) = advanced.name.as_deref().map(str::trim) {
        if !name.is_empty() {
            if name.len() > MAX_CONNECTION_NAME_BYTES {
                return Err(InvitationRefused::choice(
                    "Connection names can be at most 120 characters.",
                ));
            }
            return Ok(name.to_owned());
        }
    }
    let base = invitation
        .workspace_name
        .clone()
        .unwrap_or_else(|| crew_invitation::workspace_label(invitation));
    let taken = connections
        .iter()
        .any(|saved| existing.is_none_or(|existing| existing.id != saved.id) && saved.name == base);
    let server = invitation
        .ssh_host
        .as_deref()
        .or(route.ssh_target.as_deref())
        .filter(|_| taken);
    Ok(truncate_bytes(
        &match server {
            Some(server) => format!("{base} \u{2014} {server}"),
            None => base,
        },
        MAX_CONNECTION_NAME_BYTES,
    ))
}

/// The ordinary save path's input: the four pinned fields exactly as the invitation states them,
/// everything else from the plan.
fn save_input(
    invitation: &WorkspaceInvitation,
    advanced: &InvitationAdvanced,
    route: &PlannedRoute,
    name: String,
    mode: ClusterMode,
    institution_id: Option<String>,
) -> SaveConnection {
    SaveConnection {
        preparation_id: advanced.preparation_id.clone(),
        name,
        ssh_target: route.ssh_target.clone().unwrap_or_default(),
        port: route.port,
        identity_file: advanced.identity_file.clone(),
        proxy_jump: route.proxy_jump.clone(),
        socket_path: invitation.socket_path.clone(),
        owner_uid: invitation.owner_uid,
        workspace_id: invitation.workspace_id.clone(),
        workspace_public_key: invitation.workspace_public_key.clone(),
        remote_root: advanced.remote_root.clone(),
        remote_execution: advanced.remote_execution,
        cluster_connection_id: None,
        mode,
        institution_id,
    }
}

fn plan_invitation(
    parsed: &ParsedInvitation,
    overrides: &InvitationOverrides,
    connections: &[Connection],
) -> Result<InvitationPlan, InvitationRefused> {
    let invitation = &parsed.invitation;
    let advanced = &overrides.advanced;
    let route = planned_route(invitation, overrides)?;
    if let Some(identity) = &advanced.identity_file {
        if !std::path::Path::new(identity).is_absolute() || identity.contains('\n') {
            return Err(InvitationRefused::choice(
                "Choose the identity file by its full path.",
            ));
        }
    }
    let workspace_mode = invitation.mode.as_ref().map(cluster_mode);
    let mode = overrides.mode.or(workspace_mode).unwrap_or_default();
    let institution_id = match overrides.institution_id.as_deref().map(str::trim) {
        Some(typed) if !typed.is_empty() => Some(institution::normalize(typed).map_err(|_| {
            InvitationRefused::choice(
                "Type an institution as lowercase letters, numbers, - or _, like ucsf.",
            )
        })?),
        _ => invitation.institution_id.clone(),
    };
    let mut missing = Vec::new();
    if route.ssh_target.is_none() {
        if route.username.is_none() {
            missing.push(InvitationMissing::Username);
        }
        if invitation.ssh_host.is_none() {
            missing.push(InvitationMissing::Server);
        }
    }
    if mode == ClusterMode::Private && institution_id.is_none() {
        missing.push(InvitationMissing::Institution);
    }
    let existing = saved_match(connections, invitation)?;
    let name = planned_name(invitation, advanced, &route, connections, existing.as_ref())?;
    let fingerprint = crew_invitation::workspace_key_fingerprint(&invitation.workspace_public_key)
        .ok_or_else(|| {
            InvitationRefused::new(
                InvitationRefusal::Unreadable("invitation_invalid_field"),
                "This invitation has an invalid workspace key. Ask your host to copy it again.",
            )
        })?;
    let save = save_input(
        invitation,
        advanced,
        &route,
        name.clone(),
        mode,
        institution_id.clone(),
    );
    let preview = InvitationPreview {
        source: match parsed.source {
            crew_invitation::InvitationSource::Invitation => InvitationSourceKind::Invitation,
            crew_invitation::InvitationSource::LegacyStatus => InvitationSourceKind::LegacyStatus,
        },
        workspace_id: invitation.workspace_id.clone(),
        socket_path: invitation.socket_path.clone(),
        owner_uid: invitation.owner_uid,
        fingerprint: crew_invitation::grouped_fingerprint(&fingerprint),
        workspace_key_fingerprint: fingerprint,
        workspace_name: invitation.workspace_name.clone(),
        workspace_label: crew_invitation::workspace_label(invitation),
        host_username: invitation.host_username.clone(),
        host_display_name: invitation.host_display_name.clone(),
        workspace_mode,
        workspace_institution_id: invitation.institution_id.clone(),
        server: invitation.ssh_host.clone(),
        invitee_username: invitation.invitee_username.clone(),
        username: route.username,
        ssh_target: route.ssh_target,
        port: route.port,
        proxy_jump: route.proxy_jump,
        mode,
        institution_id,
        mode_differs: workspace_mode.is_some_and(|workspace| workspace != mode),
        name,
        existing_connection_id: existing.as_ref().map(|saved| saved.id.clone()),
        missing,
        institution_conflict: None,
        server_label: None,
    };
    Ok(InvitationPlan {
        preview,
        save,
        existing,
    })
}

fn missing_refusal(missing: InvitationMissing, preview: &InvitationPreview) -> InvitationRefused {
    let server = preview.server.as_deref().unwrap_or("the server");
    InvitationRefused::new(
        InvitationRefusal::Missing(missing),
        match missing {
            InvitationMissing::Username => format!("Type your username on {server}."),
            InvitationMissing::Server => {
                "This invitation doesn't name its server. Add a server login under Advanced."
                    .to_owned()
            }
            InvitationMissing::Institution => {
                "Choose the institution for this private workspace, like ucsf.".to_owned()
            }
        },
    )
}

/// What `enrollment.pending` said about an invited account. Unauthenticated: it chooses which
/// card to show, never the code.
struct PendingInvitation {
    join_id: String,
    workspace_name: Option<String>,
    inviter: Option<JoinPerson>,
    add_device: bool,
    approved: bool,
    expires_at: Option<u64>,
    expired: bool,
    /// Some claim for this account was refused `code_mismatch` under the host's current
    /// approval. Not necessarily this computer's: the broker keeps one per join, and a join
    /// belongs to an account, not to a device. See [`RefusedClaim`].
    last_refusal: bool,
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

impl PendingInvitation {
    /// `Ok(None)` for `{"invited": false}`. Anything malformed is refused rather than guessed.
    fn read(answer: &Value) -> Result<Option<Self>> {
        let invalid = || {
            anyhow::anyhow!(
                "The workspace sent a join status Biorouter can't read. Try again, or ask your host to update Crew on the server."
            )
        };
        let flag = |field: &str| match answer.get(field) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(value)) => Ok(Some(*value)),
            Some(_) => Err(invalid()),
        };
        if !flag("invited")?.ok_or_else(invalid)? {
            return Ok(None);
        }
        let join_id = answer["join_id"]
            .as_str()
            .filter(|id| {
                (1..=MAX_JOIN_ID_BYTES).contains(&id.len())
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            })
            .ok_or_else(invalid)?
            .to_owned();
        let expires_at = match answer.get("expires_at") {
            None | Some(Value::Null) => None,
            Some(value) => Some(value.as_u64().ok_or_else(invalid)?),
        };
        let expired = match flag("expired")? {
            Some(expired) => expired,
            None => expires_at.is_some_and(|at| at <= now_seconds()),
        };
        Ok(Some(Self {
            join_id,
            workspace_name: answer["workspace_name"]
                .as_str()
                .filter(|name| biorouter_crew::workspace_name_valid(name))
                .map(str::to_owned),
            inviter: join_person(&answer["inviter"]),
            add_device: flag("add_device")?.unwrap_or(false),
            approved: flag("approved")?.unwrap_or(false),
            expires_at,
            expired,
            last_refusal: answer["last_refusal"].as_str() == Some("code_mismatch"),
        }))
    }
    /// The state, given whether this computer's own claim was refused under the current
    /// approval ([`CrewManager::refused_here`]). The join-wide `last_refusal` alone never makes
    /// a `code_mismatch`: it may be another device's.
    fn state(&self, refused_here: bool) -> JoinState {
        if self.expired {
            JoinState::Expired
        } else if refused_here {
            JoinState::CodeMismatch
        } else if self.approved {
            JoinState::Approved
        } else {
            JoinState::Invited
        }
    }
}

/// This computer's own `auth.join` that the workspace refused `code_mismatch`, per connection.
///
/// The broker's `last_refusal` cannot say whose claim it refused. It is kept per join, and a
/// join belongs to an account (a UID), not to a device: the broker sets it whenever *any* claim
/// for the account is refused, and reports it for as long as the host's approval is unchanged.
/// Another computer of the same account, or a same-account process in the bridge path, can set
/// it under an approval of *this* computer's code; and approving that code again changes
/// nothing on the broker. Only this record tells this computer's refusal from another's, so only
/// it withholds a claim.
///
/// Memory only, like the broker's own record: a restart forgets it, which costs at most one more
/// claim (and one more warning for the host).
#[derive(Clone, Debug, PartialEq, Eq)]
struct RefusedClaim {
    device_id: String,
    join_id: String,
    /// When it was recorded, on [`CLAIM_CLOCK`].
    at: u64,
}

/// What a join status answer means for a [`RefusedClaim`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RefusalVerdict {
    /// This computer's claim was refused under the join's current approval.
    refused_here: bool,
    /// The record still stands; `false` forgets it.
    keep: bool,
}

impl RefusedClaim {
    /// Read the record against an answer (`None`: not invited) to a question asked at
    /// `asked_at` on [`CLAIM_CLOCK`], from the computer whose device is `device_id`.
    ///
    /// It stands for the same device and the same join while the answer still reports a
    /// refusal, which the broker does only while the approval is unchanged. An answer to a
    /// question asked *before* the refusal was recorded may predate the refusal, so it can
    /// neither clear the record nor contradict it; the next question settles it.
    fn judge(
        &self,
        device_id: &str,
        pending: Option<&PendingInvitation>,
        asked_at: u64,
    ) -> RefusalVerdict {
        let recorded_after_asking = self.at > asked_at;
        let same_join = self.device_id == device_id
            && pending.is_some_and(|pending| pending.join_id == self.join_id);
        let still_refused =
            pending.is_some_and(|pending| pending.last_refusal) || recorded_after_asking;
        let refused_here = same_join && still_refused;
        RefusalVerdict {
            refused_here,
            keep: refused_here || recorded_after_asking,
        }
    }
}

/// [`RefusedClaim`]s, by manager root and connection ID.
static REFUSED_CLAIMS: LazyLock<Mutex<HashMap<(PathBuf, String), RefusedClaim>>> =
    LazyLock::new(Default::default);
/// Orders a join status question against a recorded refusal.
static CLAIM_CLOCK: AtomicU64 = AtomicU64::new(0);

fn claim_clock_tick() -> u64 {
    CLAIM_CLOCK.fetch_add(1, Ordering::SeqCst) + 1
}

/// A person from an unauthenticated answer, made safe to show: a valid username, and a display
/// name that passes the display-name rules, else the username.
fn join_person(value: &Value) -> Option<JoinPerson> {
    let username = value["username"].as_str()?;
    if !biorouter_crew::valid_username(username) || username.chars().any(char::is_control) {
        return None;
    }
    let display_name = value["display_name"]
        .as_str()
        .map(|name| {
            biorouter_crew::sanitize_display_name(name, username, std::iter::empty::<&str>())
        })
        .unwrap_or_else(|| username.to_owned());
    Some(JoinPerson {
        username: username.to_owned(),
        display_name,
    })
}

/// A join status as read, with the join ID a claim sends back.
struct ObservedJoin {
    status: JoinStatus,
    join_id: Option<String>,
}

/// The address a joiner reaches the host's server by, as the host's own `ssh -G` resolves it.
#[derive(Debug, PartialEq, Eq)]
struct ServerAddress {
    host: String,
    port: Option<u16>,
    proxy_jump: Option<String>,
}

/// `ssh -G <args>`: the effective settings, first value per key.
pub(super) async fn resolve_ssh(args: &[String]) -> Result<HashMap<String, String>> {
    use tokio::io::AsyncReadExt;
    let mut command = tokio::process::Command::new("ssh");
    command
        .arg("-G")
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    crate::subprocess::prepare_agent_child_command(&mut command);
    let mut child = command
        .spawn()
        .context("Couldn't run ssh to read this server's address")?;
    let mut stdout = child
        .stdout
        .take()
        .context("SSH configuration output unavailable")?;
    let read = async {
        let mut bytes = Vec::new();
        (&mut stdout)
            .take(SSH_RESOLVE_LIMIT + 1)
            .read_to_end(&mut bytes)
            .await?;
        ensure!(
            bytes.len() as u64 <= SSH_RESOLVE_LIMIT,
            "SSH configuration output exceeds one MiB"
        );
        ensure!(
            child.wait().await?.success(),
            "ssh couldn't read your settings for this server"
        );
        Ok::<_, anyhow::Error>(bytes)
    };
    let bytes = tokio::time::timeout(SSH_RESOLVE_TIMEOUT, read)
        .await
        .context("ssh took too long to read your settings for this server")??;
    let text = String::from_utf8(bytes).context("SSH configuration output is not UTF-8")?;
    let mut settings = HashMap::new();
    for line in text.lines() {
        if let Some((key, value)) = line.split_once(' ') {
            settings
                .entry(key.to_ascii_lowercase())
                .or_insert_with(|| value.trim().to_owned());
        }
    }
    Ok(settings)
}

/// A host name an invitation may carry: DNS characters only, so never a path, option or
/// bracketed address.
fn plain_host(host: &str) -> bool {
    (1..=253).contains(&host.len())
        && !host.starts_with(['-', '.'])
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// Whether a verified `hello` v2 no longer describes the workspace a fresh snapshot shows:
/// another policy epoch, privacy mode, institution or name. The snapshot is unsigned, so it
/// only decides whether to ask again; it never supplies the value.
fn hello_is_stale(hello: &super::BrokerHello, workspace: &Value) -> bool {
    let name = workspace["name"]
        .as_str()
        .filter(|name| biorouter_crew::workspace_name_valid(name));
    let mode = serde_json::from_value::<biorouter_crew::Mode>(workspace["mode"].clone())
        .ok()
        .as_ref()
        .map(cluster_mode);
    hello.policy_epoch != workspace["policy_epoch"].as_u64()
        || hello.mode != mode
        || hello.institution_id.as_deref() != workspace["institution_id"].as_str()
        || hello.workspace_name.as_deref() != name
}

/// Where an SSH login really goes: the lowercase hostname and port `ssh -G` resolves under
/// the same configuration the bridge reads, else the login's own host part and port. `None`
/// for a login that can't safely be passed to `ssh`.
pub(super) async fn ssh_endpoint(target: &str, port: Option<u16>) -> Option<(String, u16)> {
    if !safe_atom(target) {
        return None;
    }
    let mut args = Vec::new();
    if let Some(profile) = std::env::var_os("BIOROUTER_DEV_PROFILE_ROOT") {
        args.extend([
            "-F".to_owned(),
            PathBuf::from(profile)
                .join("home/.ssh/config")
                .to_string_lossy()
                .into_owned(),
        ]);
    }
    if let Some(port) = port {
        args.extend(["-p".to_owned(), port.to_string()]);
    }
    args.push(target.to_owned());
    if let Some((host, port)) = resolve_ssh(&args)
        .await
        .ok()
        .and_then(|settings| resolved_endpoint(&settings).ok())
    {
        return Some((host.to_ascii_lowercase(), port));
    }
    let host = target.rsplit_once('@').map_or(target, |(_, host)| host);
    Some((host.to_ascii_lowercase(), port.unwrap_or(22)))
}

/// `hostname` and `port` from `ssh -G` settings.
pub(super) fn resolved_endpoint(settings: &HashMap<String, String>) -> Result<(String, u16)> {
    let host = settings
        .get("hostname")
        .filter(|host| safe_atom(host) && !host.contains(['@', '/']) && host.len() <= 253)
        .context("Couldn't read this server's address from your SSH settings")?;
    let port: u16 = settings
        .get("port")
        .and_then(|port| port.parse().ok())
        .filter(|port| *port > 0)
        .context("Couldn't read this server's SSH port from your SSH settings")?;
    Ok((host.clone(), port))
}

/// One hop of a ProxyJump route: `[ssh://][user@]host[:port]`, the user dropped (the joiner's
/// account on a jump host is their own), the host and port kept.
fn jump_hop(hop: &str) -> Option<(String, Option<u16>)> {
    let authority = hop.strip_prefix("ssh://").unwrap_or(hop);
    let address = authority
        .rsplit_once('@')
        .map_or(authority, |(_, address)| address);
    let (host, port) = match address.split_once(':') {
        Some((host, port)) => (host, Some(port.parse::<u16>().ok().filter(|p| *p > 0)?)),
        None => (address, None),
    };
    plain_host(host).then(|| (host.to_owned(), port))
}

// ---------------------------------------------------------------------------------------------

impl CrewManager {
    /// Parse a pasted invitation (the host's whole message, the bare `brcrew1:` line, or the
    /// legacy `biorouter-crew status` JSON) and preview it, or save a connection pinned exactly
    /// as it says: the workspace ID, workspace key, socket and host UID are never taken from
    /// anywhere else. Saving goes through the ordinary save path, so a Private save needs an
    /// institution, which the invitation of a Private workspace supplies.
    ///
    /// A preview saves nothing and touches no credential. A save of a workspace this computer
    /// already has returns that connection when the settings agree (so a repeated paste never
    /// mints a second device key, which would change the device code), and is refused when
    /// they differ or when the saved connection pins another identity.
    pub async fn connection_from_invitation(
        &self,
        text: &str,
        preview: bool,
        overrides: InvitationOverrides,
    ) -> Result<InvitationOutcome> {
        let parsed = crew_invitation::parse(text).map_err(|error| {
            InvitationRefused::new(
                InvitationRefusal::Unreadable(error.code()),
                error.to_string(),
            )
        })?;
        if preview {
            let plan = plan_invitation(&parsed, &overrides, &self.list().await)?;
            let mut preview = plan.preview;
            preview.institution_conflict = self.institution_conflict(&preview).await;
            if let Some(target) = preview.ssh_target.as_deref() {
                preview.server_label = Some(super::server_label(target, preview.port).await);
            }
            return Ok(InvitationOutcome::Preview(Box::new(preview)));
        }
        let _serial = INVITATION_SAVES.lock().await;
        let plan = plan_invitation(&parsed, &overrides, &self.list().await)?;
        if let Some(missing) = plan.preview.missing.first() {
            return Err(missing_refusal(*missing, &plan.preview).into());
        }
        if let Some(existing) = plan.existing {
            if same_settings(&existing, &plan.save) {
                return Ok(InvitationOutcome::Saved(Box::new(existing)));
            }
            let mut refused = InvitationRefused::new(
                InvitationRefusal::AlreadySaved,
                format!(
                    "This computer already has \u{201c}{}\u{201d} for this workspace. Change it in its connection settings instead.",
                    super::plain_label(&existing.name)
                ),
            );
            refused.connection_id = Some(existing.id.clone());
            return Err(refused.into());
        }
        Ok(InvitationOutcome::Saved(Box::new(
            self.save(plan.save).await?,
        )))
    }

    /// T-52: whether a saved connection other than the one this invitation would reuse
    /// reaches the same server (as `ssh -G` resolves each login, else by its host part) with a
    /// different institution than the one saving would record. Best effort and advisory: the
    /// connect-time refusal is what enforces the rule, and it stays.
    async fn institution_conflict(&self, preview: &InvitationPreview) -> Option<String> {
        let institution = preview.institution_id.as_deref()?;
        let target = preview.ssh_target.as_deref()?;
        let others: Vec<Connection> = self
            .list()
            .await
            .into_iter()
            .filter(|saved| Some(saved.id.as_str()) != preview.existing_connection_id.as_deref())
            .filter(|saved| {
                saved
                    .institution_id
                    .as_deref()
                    .is_some_and(|other| !other.eq_ignore_ascii_case(institution))
            })
            .collect();
        if others.is_empty() {
            return None;
        }
        let here = ssh_endpoint(target, preview.port).await?;
        for other in others {
            if ssh_endpoint(&other.ssh_target, other.port).await.as_ref() == Some(&here) {
                return Some(format!(
                    "You already use this server for {} ({}). {} uses {}; one computer can't mix institutions on the same server.",
                    super::plain_label(&other.name),
                    super::plain_label(other.institution_id.as_deref().unwrap_or_default()),
                    super::plain_label(&preview.workspace_label),
                    super::plain_label(institution),
                ));
            }
        }
        None
    }

    /// T-10: the cached `hello` is the answer at connect time. When the workspace has changed
    /// since (a host set the institution, or renamed it), an invitation is built from a fresh,
    /// verified `hello`, never from the unsigned snapshot and never from the stale answer; if
    /// that can't be had, nothing is built.
    async fn current_signed_hello(
        &self,
        connection_id: &str,
        c: &Connection,
        cached: super::BrokerHello,
        workspace: &Value,
    ) -> Result<super::BrokerHello> {
        if cached.signature_version < 2 || !hello_is_stale(&cached, workspace) {
            return Ok(cached);
        }
        let reconnect = format!(
            "Reconnect to {}, then invite again.",
            cached
                .workspace_name
                .as_deref()
                .map(super::plain_label)
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| super::plain_label(&c.name))
        );
        match self.refresh_broker_hello(connection_id).await {
            Ok(fresh) if fresh.signature_version >= 2 => Ok(fresh),
            Ok(_) => anyhow::bail!(reconnect),
            Err(error) => Err(error.context(reconnect)),
        }
    }

    /// The invitation a host sends, for `invitee` when given (`@bob` or `bob`).
    ///
    /// Built from this computer's verified connection (its four pinned fields, never its local
    /// name or work folder), the workspace's name, privacy and institution (from the signed
    /// `hello` v2 when the broker sent one, else from a fresh snapshot), the host's username
    /// from that snapshot, and `ssh -G` for the connection's SSH login: the server's real
    /// hostname, port and jump hosts, never a local alias.
    pub async fn invitation_for(
        &self,
        connection_id: &str,
        invitee: Option<&str>,
    ) -> Result<InvitationText> {
        let invitee = match invitee.map(str::trim).filter(|typed| !typed.is_empty()) {
            Some(typed) => {
                let name = typed.strip_prefix('@').unwrap_or(typed);
                ensure!(
                    biorouter_crew::valid_username(name),
                    "Type the person's username on the server, like @bob."
                );
                Some(name.to_owned())
            }
            None => None,
        };
        let c = self.connection(connection_id).await?;
        let cached = self
            .broker_hello(connection_id)
            .filter(|_| c.node_id.is_some())
            .context("Connect to this workspace before inviting people.")?;
        let snapshot = self
            .human_request(connection_id, "workspace.snapshot", json!({}), None)
            .await?;
        let workspace = &snapshot["workspace"];
        ensure!(
            workspace["id"]
                .as_str()
                .is_some_and(|id| same_workspace_id(id, &c.workspace_id)),
            "The workspace's answer doesn't match this connection. Reconnect and try again."
        );
        let hello = self
            .current_signed_hello(connection_id, &c, cached, workspace)
            .await?;
        let (workspace_name, mode, institution_id) = if hello.signature_version >= 2 {
            (
                hello.workspace_name.clone(),
                hello.mode.map(crew_mode),
                hello.institution_id.clone(),
            )
        } else {
            (
                workspace["name"]
                    .as_str()
                    .filter(|name| biorouter_crew::workspace_name_valid(name))
                    .map(str::to_owned),
                serde_json::from_value::<biorouter_crew::Mode>(workspace["mode"].clone()).ok(),
                workspace["institution_id"].as_str().map(str::to_owned),
            )
        };
        let mode = mode.context("The workspace didn't say whether it is private or public.")?;
        let host = snapshot["principals"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|principal| {
                principal["uid"].as_u64() == Some(u64::from(c.owner_uid))
                    && principal["active"].as_bool() != Some(false)
            })
            .context("The workspace's host hasn't joined it yet. Create the workspace first.")?;
        let host_username = host["username"]
            .as_str()
            .filter(|name| biorouter_crew::valid_username(name))
            .context("The workspace's host has a username Biorouter can't put in an invitation.")?
            .to_owned();
        let host_display_name = host["display_name"]
            .as_str()
            .and_then(|name| biorouter_crew::validate_display_name(name).ok())
            .filter(|name| {
                biorouter_crew::name_key(name) != biorouter_crew::name_key(&host_username)
            });
        let server = self.server_address(&c).await?;
        let invitation = WorkspaceInvitation {
            workspace_id: uuid::Uuid::parse_str(&c.workspace_id)?
                .hyphenated()
                .to_string(),
            workspace_public_key: c.workspace_public_key.to_ascii_lowercase(),
            socket_path: c.socket_path.clone(),
            owner_uid: c.owner_uid,
            workspace_name,
            host_username: Some(host_username),
            host_display_name,
            mode: Some(mode),
            institution_id,
            ssh_host: Some(server.host),
            ssh_port: server.port,
            proxy_jump: server.proxy_jump,
            invitee_username: invitee,
        };
        let unusable = |error: crew_invitation::InvitationError| {
            anyhow::anyhow!(
                "Biorouter couldn't build an invitation from this connection's saved details ({}).",
                error.code()
            )
        };
        Ok(InvitationText {
            line: crew_invitation::encode(&invitation).map_err(unusable)?,
            message: crew_invitation::message(&invitation).map_err(unusable)?,
        })
    }

    /// The server's hostname, port (when not 22) and jump hosts, as `ssh -G` resolves this
    /// connection's login. Each jump host is resolved too, so no hop is a local alias; a route
    /// that can't be described that way is left out, and the joiner sets it under Advanced.
    async fn server_address(&self, c: &Connection) -> Result<ServerAddress> {
        ensure!(
            safe_atom(&c.ssh_target),
            "SSH target must be a host alias or user@host"
        );
        let base = transport::ssh_args(c, &self.control_path(&c.id)?);
        let config: Vec<String> = base
            .windows(2)
            .find(|pair| pair[0] == "-F")
            .map(|pair| pair.to_vec())
            .unwrap_or_default();
        let mut args = base;
        args.push(c.ssh_target.clone());
        let settings = resolve_ssh(&args).await?;
        let (host, port) = resolved_endpoint(&settings)?;
        ensure!(
            plain_host(&host),
            "This server's address from your SSH settings can't be put in an invitation."
        );
        let mut hops = Vec::new();
        let route = settings
            .get("proxyjump")
            .map(String::as_str)
            .filter(|route| !route.is_empty() && *route != "none");
        if let Some(route) = route {
            for hop in route.split(',').take(MAX_JUMP_HOPS + 1) {
                let Some((alias, hop_port)) = jump_hop(hop) else {
                    hops.clear();
                    break;
                };
                let mut hop_args = config.clone();
                if let Some(hop_port) = hop_port {
                    hop_args.extend(["-p".into(), hop_port.to_string()]);
                }
                hop_args.push(alias);
                let (hop_host, hop_port) = resolved_endpoint(&resolve_ssh(&hop_args).await?)?;
                if !plain_host(&hop_host) {
                    hops.clear();
                    break;
                }
                hops.push(if hop_port == 22 {
                    hop_host
                } else {
                    format!("{hop_host}:{hop_port}")
                });
            }
            if hops.len() > MAX_JUMP_HOPS {
                hops.clear();
            }
        }
        Ok(ServerAddress {
            host,
            port: (port != 22).then_some(port),
            proxy_jump: (!hops.is_empty()).then(|| hops.join(",")),
        })
    }

    /// Where this computer stands in joining the connection's workspace.
    ///
    /// Sends the **unsigned**, pre-authentication `enrollment.pending` over the bridge, as
    /// `hello` is sent, only when the broker announced [`JOIN_BY_NAME_CAPABILITY`]
    /// (`unsupported` otherwise, with nothing sent). When the account is not invited, one
    /// signed read tells a member (`joined`) from a stranger (`not_invited`). The device code
    /// is computed here from the saved device key and the pinned workspace key; the answer's
    /// fields only choose which card to show.
    pub async fn join_status(&self, id: &str) -> Result<JoinStatus> {
        Ok(self.observe_join(id).await?.status)
    }

    /// Join the workspace: when (and only when) the workspace says the host approved this
    /// computer, send `auth.challenge` and `auth.join {public_key, join_id}` signed with the
    /// saved device key, through the one door that may send it. Idempotent: an account that
    /// is already a member answers `joined` without sending anything.
    ///
    /// Any other state is a [`JoinRefused`] and sends nothing, because a claim under an
    /// unapproved code, or under an approval this computer's own claim was already refused
    /// under, would only show the host a warning about a device with a different code. A
    /// refusal the workspace reports for the join but this computer never received (another
    /// device's) does not stop the claim; see [`RefusedClaim`].
    pub async fn join(&self, id: &str) -> Result<JoinStatus> {
        let observed = self.observe_join(id).await?;
        let workspace_name = observed.status.workspace_name.clone();
        if observed.status.status == JoinState::Joined {
            return Ok(observed.status);
        }
        if let Some(refusal) = JoinRefusal::from_state(observed.status.status) {
            return Err(JoinRefused::error(refusal));
        }
        let join_id = observed
            .join_id
            .context("The workspace approved a join without naming it")?;
        let c = self.connection(id).await?;
        let params = json!({"public_key": c.public_key, "join_id": join_id});
        let answer = match self.signed_join_request(id, params, None).await {
            Ok(answer) => answer,
            Err(error) => {
                let Some((code, message)) = broker_refusal(&error) else {
                    return Err(error);
                };
                let refusal = JoinRefusal::from_broker_code(&code);
                if refusal == JoinRefusal::CodeMismatch {
                    self.remember_refused_claim(&c, &join_id);
                }
                // A second claim racing the first finds the join already used.
                if refusal == JoinRefusal::NotInvited && self.is_member(id).await? {
                    return Ok(JoinStatus::bare(JoinState::Joined, workspace_name));
                }
                return Err(anyhow::Error::new(JoinRefused {
                    refusal,
                    broker_message: Some(message),
                }));
            }
        };
        ensure!(
            answer["device_id"]
                .as_str()
                .is_none_or(|device| device == c.device_id),
            "The workspace's answer names a different device. Check your join status."
        );
        Ok(JoinStatus {
            add_device: observed.status.add_device,
            inviter: observed.status.inviter,
            ..JoinStatus::bare(JoinState::Joined, workspace_name)
        })
    }

    async fn observe_join(&self, id: &str) -> Result<ObservedJoin> {
        let c = self.connection(id).await?;
        // Disconnected is an error, not a status: nothing can be asked.
        self.transport(id).await?;
        let hello = self.broker_hello(id);
        let signed_name = hello
            .as_ref()
            .filter(|hello| hello.signature_version >= 2)
            .and_then(|hello| hello.workspace_name.clone());
        let unsupported = || ObservedJoin {
            status: JoinStatus::bare(JoinState::Unsupported, signed_name.clone()),
            join_id: None,
        };
        if !hello.as_ref().is_some_and(|hello| {
            hello
                .capabilities
                .iter()
                .any(|capability| capability == JOIN_BY_NAME_CAPABILITY)
        }) {
            return Ok(unsupported());
        }
        let asked_at = claim_clock_tick();
        let answer = match self
            .pre_authentication_request(id, "enrollment.pending")
            .await
        {
            Ok(answer) => answer,
            // An unsigned capability can be spoofed; a broker without the method refuses it.
            Err(error) => match broker_refusal(&error) {
                Some((code, _)) if matches!(code.as_str(), "unauthorized" | "unsupported") => {
                    return Ok(unsupported());
                }
                _ => return Err(error),
            },
        };
        let pending = PendingInvitation::read(&answer)?;
        let refused_here = self.refused_here(&c, pending.as_ref(), asked_at);
        let Some(pending) = pending else {
            let state = if self.is_member(id).await? {
                JoinState::Joined
            } else {
                JoinState::NotInvited
            };
            return Ok(ObservedJoin {
                status: JoinStatus::bare(state, signed_name),
                join_id: None,
            });
        };
        let state = pending.state(refused_here);
        let code = match state {
            JoinState::Invited | JoinState::Approved | JoinState::CodeMismatch => {
                Some(self.device_code_of(&c)?)
            }
            _ => None,
        };
        Ok(ObservedJoin {
            status: JoinStatus {
                status: state,
                code,
                inviter: pending.inviter,
                workspace_name: signed_name.or(pending.workspace_name),
                expires_at: pending.expires_at,
                add_device: pending.add_device,
            },
            join_id: Some(pending.join_id),
        })
    }

    /// Remember that the workspace refused this computer's claim under `join_id` with
    /// `code_mismatch` ([`RefusedClaim`]).
    fn remember_refused_claim(&self, c: &Connection, join_id: &str) {
        REFUSED_CLAIMS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                (self.root.clone(), c.id.clone()),
                RefusedClaim {
                    device_id: c.device_id.clone(),
                    join_id: join_id.to_owned(),
                    at: claim_clock_tick(),
                },
            );
    }

    /// Whether this computer's own claim was refused under the join's current approval, read
    /// against `pending`, the answer to a question asked at `asked_at`. Forgets a record the
    /// answer outdates: the host approved another code, replaced or cancelled the invitation,
    /// or the account joined.
    fn refused_here(
        &self,
        c: &Connection,
        pending: Option<&PendingInvitation>,
        asked_at: u64,
    ) -> bool {
        let mut claims = REFUSED_CLAIMS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let key = (self.root.clone(), c.id.clone());
        let Some(verdict) = claims
            .get(&key)
            .map(|claim| claim.judge(&c.device_id, pending, asked_at))
        else {
            return false;
        };
        if !verdict.keep {
            claims.remove(&key);
        }
        verdict.refused_here
    }

    /// This computer's device code for the connection's workspace (`7QK2-M9XA-3JTP-WZ4D`),
    /// computed here and never asked of the workspace. Needs no connection.
    pub async fn device_code(&self, id: &str) -> Result<String> {
        let c = self.connection(id).await?;
        self.device_code_of(&c)
    }

    /// `device_code(workspace_id, W, K)` over the pinned workspace key `W` and the public key
    /// `K` of the saved signing key, which must be the key the connection saved, formatted for
    /// display.
    fn device_code_of(&self, c: &Connection) -> Result<String> {
        let secret: [u8; 32] = unhex(&self.read_credential(&format!("device:{}", c.id))?)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid device key"))?;
        let public = SigningKey::from_bytes(&secret).verifying_key().to_bytes();
        ensure!(
            hex(&public) == c.public_key && hex(&Sha256::digest(public)) == c.device_id,
            "Saved Crew device identity does not match its signing credential; reconnect using a verified device identity"
        );
        let workspace_key: [u8; 32] = unhex(&c.workspace_public_key)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid workspace key"))?;
        Ok(biorouter_crew::format_device_code(
            &biorouter_crew::device_code(&c.workspace_id, &workspace_key, &public),
        ))
    }

    /// Whether this computer's key is a member's device: one signed read, refused as
    /// `unauthorized` for a device the workspace does not know (or whose account left).
    async fn is_member(&self, id: &str) -> Result<bool> {
        match self
            .signed_request(id, "profile.suggest", json!({}), None)
            .await
        {
            Ok(_) => Ok(true),
            Err(error) => match broker_refusal(&error) {
                Some((code, _)) if code == "unauthorized" => Ok(false),
                // Refused after authenticating: a member, on a broker without the method.
                Some((code, _)) if code == "unsupported" => Ok(true),
                _ => Err(error),
            },
        }
    }

    /// An unsigned request before authentication, like `hello`. Only `enrollment.pending`.
    async fn pre_authentication_request(&self, id: &str, method: &str) -> Result<Value> {
        ensure!(
            method == "enrollment.pending",
            "Only the join status is read before authentication"
        );
        let transport = self.transport(id).await?;
        let mut locked = transport.lock().await;
        let result = locked.request(method, json!({}), None, None, None).await;
        let usable = locked.is_usable();
        drop(locked);
        if !usable {
            self.retire_failed_transport(id, &transport).await?;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_pty::{Child as PtyChild, ChildKiller, ExitStatus};
    use std::{
        io,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc, Barrier,
        },
        thread,
    };

    #[derive(Debug, Default)]
    struct FakeChildState {
        reaped: AtomicBool,
        try_wait_error: AtomicBool,
        kill_calls: AtomicUsize,
        try_wait_calls: AtomicUsize,
        active_calls: AtomicUsize,
        overlapped_calls: AtomicBool,
    }

    #[derive(Debug, Clone)]
    struct FakeChild {
        state: Arc<FakeChildState>,
        reap_on_kill: bool,
    }

    struct CallGuard<'a> {
        active_calls: &'a AtomicUsize,
    }

    impl Drop for CallGuard<'_> {
        fn drop(&mut self) {
            self.active_calls.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl FakeChild {
        fn new(reap_on_kill: bool) -> (Self, Arc<FakeChildState>) {
            let state = Arc::new(FakeChildState::default());
            (
                Self {
                    state: state.clone(),
                    reap_on_kill,
                },
                state,
            )
        }

        fn enter_call(&self) -> CallGuard<'_> {
            if self.state.active_calls.fetch_add(1, Ordering::SeqCst) > 0 {
                self.state.overlapped_calls.store(true, Ordering::SeqCst);
            }
            CallGuard {
                active_calls: &self.state.active_calls,
            }
        }
    }

    impl ChildKiller for FakeChild {
        fn kill(&mut self) -> io::Result<()> {
            let _guard = self.enter_call();
            self.state.kill_calls.fetch_add(1, Ordering::SeqCst);
            if self.reap_on_kill {
                self.state.reaped.store(true, Ordering::SeqCst);
            }
            Ok(())
        }

        fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
            Box::new(self.clone())
        }
    }

    impl PtyChild for FakeChild {
        fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            let _guard = self.enter_call();
            self.state.try_wait_calls.fetch_add(1, Ordering::SeqCst);
            if self.state.try_wait_error.load(Ordering::SeqCst) {
                return Err(io::Error::other("synthetic try_wait failure"));
            }
            Ok(self
                .state
                .reaped
                .load(Ordering::SeqCst)
                .then(|| ExitStatus::with_exit_code(17)))
        }

        fn wait(&mut self) -> io::Result<ExitStatus> {
            self.state.reaped.store(true, Ordering::SeqCst);
            Ok(ExitStatus::with_exit_code(17))
        }

        fn process_id(&self) -> Option<u32> {
            Some(17)
        }

        #[cfg(windows)]
        fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
            None
        }
    }

    fn owned_child(fake: FakeChild) -> OwnedChild {
        OwnedChild {
            child: Box::new(fake),
            terminal_status: None,
        }
    }

    #[test]
    fn terminal_dimensions_are_bounded() {
        assert!(dimensions(80, 24).is_ok());
        assert!(dimensions(19, 24).is_err());
        assert!(dimensions(80, 4).is_err());
        assert!(dimensions(501, 24).is_err());
        assert!(dimensions(80, 201).is_err());
    }

    #[test]
    fn terminal_failure_guidance_is_allowlisted_and_never_echoes_unknown_codes() {
        assert_eq!(
            terminal_failure_message(ATTACH_FAILURE_CODE),
            Some(ATTACH_FAILURE_MESSAGE)
        );
        assert_eq!(
            terminal_failure_message(HANDOFF_FAILURE_CODE),
            Some(HANDOFF_FAILURE_MESSAGE)
        );
        assert_eq!(
            terminal_failure_message(HANDOFF_FAILED_CODE),
            Some(HANDOFF_FAILED_MESSAGE)
        );
        let malicious = "authentication_handoff_failed: secret=synthetic\ntrace";
        assert_eq!(terminal_failure_message(malicious), None);
    }

    #[test]
    fn kill_does_not_signal_again_when_kill_reaps_internally() {
        let (fake, state) = FakeChild::new(true);
        let mut owned = owned_child(fake);

        owned.kill();
        owned.kill();

        assert_eq!(state.kill_calls.load(Ordering::SeqCst), 1);
        assert_eq!(owned.terminal_status, Some(Some(17)));
    }

    #[test]
    fn kill_after_terminal_status_never_resignals() {
        let (fake, state) = FakeChild::new(false);
        state.reaped.store(true, Ordering::SeqCst);
        let mut owned = owned_child(fake);

        owned.kill();
        owned.kill();

        assert_eq!(state.kill_calls.load(Ordering::SeqCst), 0);
        assert_eq!(state.try_wait_calls.load(Ordering::SeqCst), 1);
        assert_eq!(owned.terminal_status, Some(Some(17)));
    }

    #[test]
    fn try_wait_error_permanently_disarms_future_kill() {
        let (fake, state) = FakeChild::new(true);
        state.try_wait_error.store(true, Ordering::SeqCst);
        let mut owned = owned_child(fake);

        owned.kill();
        state.try_wait_error.store(false, Ordering::SeqCst);
        owned.kill();

        assert_eq!(state.kill_calls.load(Ordering::SeqCst), 0);
        assert_eq!(state.try_wait_calls.load(Ordering::SeqCst), 1);
        assert_eq!(owned.terminal_status, Some(None));
    }

    #[test]
    fn concurrent_kill_and_reap_calls_are_serialized_by_runtime_mutex() {
        let (fake, state) = FakeChild::new(true);
        let owned = Arc::new(Mutex::new(owned_child(fake)));
        let barrier = Arc::new(Barrier::new(8));
        let mut threads = Vec::new();

        for _ in 0..8 {
            let owned = owned.clone();
            let barrier = barrier.clone();
            threads.push(thread::spawn(move || {
                barrier.wait();
                for _ in 0..8 {
                    owned.lock().unwrap().kill();
                }
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }

        assert_eq!(state.kill_calls.load(Ordering::SeqCst), 1);
        assert!(!state.overlapped_calls.load(Ordering::SeqCst));
        assert_eq!(state.active_calls.load(Ordering::SeqCst), 0);
        assert_eq!(owned.lock().unwrap().terminal_status, Some(Some(17)));
    }
}

#[cfg(test)]
mod admission_tests {
    //! Workspace admission (S3a). Tests that write credentials, spawn a fake `ssh` or move the
    //! environment run in a process of their own, as the core's other Crew tests do.
    use super::*;
    use crate::crew::{BrokerHello, Registry, WorkspaceIdentityError};
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    const WORKSPACE_ID: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000000a1";
    const SOCKET: &str = "/tmp/crew-1000-0123456789abcdef0123456789abcdef/broker.sock";

    fn workspace_key() -> [u8; 32] {
        SigningKey::from_bytes(&[9; 32]).verifying_key().to_bytes()
    }

    fn lab_invitation() -> WorkspaceInvitation {
        WorkspaceInvitation {
            workspace_id: WORKSPACE_ID.into(),
            workspace_public_key: hex(&workspace_key()),
            socket_path: SOCKET.into(),
            owner_uid: 1000,
            workspace_name: Some("lab".into()),
            host_username: Some("alice".into()),
            host_display_name: Some("Alice Chen".into()),
            mode: Some(biorouter_crew::Mode::Private),
            institution_id: Some("ucsf".into()),
            ssh_host: Some("hpc.example.org".into()),
            ssh_port: None,
            proxy_jump: None,
            invitee_username: Some("bob".into()),
        }
    }

    fn fixture_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "biorouter-crew-admission-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn preview_of(outcome: InvitationOutcome) -> InvitationPreview {
        match outcome {
            InvitationOutcome::Preview(preview) => *preview,
            InvitationOutcome::Saved(_) => panic!("a preview saved a connection"),
        }
    }

    fn saved_of(outcome: InvitationOutcome) -> Connection {
        match outcome {
            InvitationOutcome::Saved(connection) => *connection,
            InvitationOutcome::Preview(_) => panic!("a save only previewed"),
        }
    }

    fn refused(error: &anyhow::Error) -> InvitationRefused {
        error
            .downcast_ref::<InvitationRefused>()
            .cloned()
            .unwrap_or_else(|| panic!("an invitation refusal is typed: {error:#}"))
    }

    /// A saved connection to `workspace_id`, pinned to `key`.
    fn saved_connection(id: &str, name: &str, workspace_id: &str, key: &str) -> Connection {
        let device = SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes();
        Connection {
            id: id.into(),
            node_id: Some("ab".repeat(32)),
            name: name.into(),
            ssh_target: "bob@hpc.example.org".into(),
            port: None,
            identity_file: None,
            proxy_jump: None,
            socket_path: SOCKET.into(),
            owner_uid: 1000,
            workspace_id: workspace_id.into(),
            workspace_public_key: key.into(),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: uuid::Uuid::new_v4().to_string(),
            mode: ClusterMode::Private,
            institution_id: Some("ucsf".into()),
            policy_epoch: 1,
            status: "connected".into(),
            last_error: None,
            device_id: hex(&Sha256::digest(device)),
            public_key: hex(&device),
        }
    }

    #[tokio::test]
    async fn invitation_parser_accepts_the_message_the_bare_line_and_legacy_status() {
        let root = fixture_root("forms");
        let manager = CrewManager::new(root.clone()).unwrap();
        let message = crew_invitation::message(&lab_invitation()).unwrap();
        let line = crew_invitation::encode(&lab_invitation()).unwrap();
        let fingerprint =
            crew_invitation::workspace_key_fingerprint(&hex(&workspace_key())).unwrap();
        for text in [
            message.clone(),
            line.clone(),
            format!("Hi Bob!\n\n{message}\n\nSee you Monday."),
        ] {
            let preview = preview_of(
                manager
                    .connection_from_invitation(&text, true, InvitationOverrides::default())
                    .await
                    .unwrap(),
            );
            assert_eq!(preview.source, InvitationSourceKind::Invitation);
            assert_eq!(preview.workspace_id, WORKSPACE_ID);
            assert_eq!(preview.workspace_name.as_deref(), Some("lab"));
            assert_eq!(preview.host_display_name.as_deref(), Some("Alice Chen"));
            assert_eq!(preview.username.as_deref(), Some("bob"));
            assert_eq!(preview.ssh_target.as_deref(), Some("bob@hpc.example.org"));
            assert_eq!(preview.workspace_mode, Some(ClusterMode::Private));
            assert_eq!(preview.mode, ClusterMode::Private);
            assert_eq!(preview.institution_id.as_deref(), Some("ucsf"));
            assert!(!preview.mode_differs);
            assert!(preview.missing.is_empty());
            assert_eq!(preview.name, "lab");
            assert_eq!(
                preview.fingerprint,
                crew_invitation::grouped_fingerprint(&fingerprint)
            );
        }
        let status = json!({"protocol": 1, "workspace_id": WORKSPACE_ID, "host_uid": 1000,
            "workspace_public_key": hex(&workspace_key()), "socket": SOCKET,
            "workspace_key_fingerprint": fingerprint})
        .to_string();
        let legacy = preview_of(
            manager
                .connection_from_invitation(
                    &format!("$ biorouter-crew status\n{status}\n$"),
                    true,
                    InvitationOverrides::default(),
                )
                .await
                .unwrap(),
        );
        assert_eq!(legacy.source, InvitationSourceKind::LegacyStatus);
        assert_eq!(legacy.workspace_mode, None);
        assert_eq!(
            legacy.missing,
            vec![
                InvitationMissing::Username,
                InvitationMissing::Server,
                InvitationMissing::Institution
            ]
        );
        // A host saving their own workspace names their login and institution.
        let host = InvitationOverrides {
            institution_id: Some("ucsf".into()),
            advanced: InvitationAdvanced {
                ssh_target: Some("alice@hpc.example.org".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let host = preview_of(
            manager
                .connection_from_invitation(&status, true, host)
                .await
                .unwrap(),
        );
        assert!(host.missing.is_empty(), "{:?}", host.missing);
        assert_eq!(host.ssh_target.as_deref(), Some("alice@hpc.example.org"));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn invitation_parser_refuses_garbage_oversize_and_unknown_versions() {
        let root = fixture_root("refusals");
        let manager = CrewManager::new(root.clone()).unwrap();
        let cases = [
            (
                "Hi Bob, here is the thing we talked about".to_owned(),
                "invitation_not_found",
            ),
            ("brcrew1:!!!!".to_owned(), "invitation_malformed"),
            (
                "brcrew1:".to_owned() + &"A".repeat(crew_invitation::MAX_PASTED_BYTES),
                "invitation_too_long",
            ),
            (
                "brcrew1:eyJ2IjoyfQ".to_owned(),
                "invitation_unsupported_version",
            ),
        ];
        for (text, code) in cases {
            for preview in [true, false] {
                let error = manager
                    .connection_from_invitation(&text, preview, InvitationOverrides::default())
                    .await
                    .unwrap_err();
                let refusal = refused(&error);
                assert_eq!(refusal.reason(), InvitationRefusal::Unreadable(code));
                assert_eq!(refusal.api_code(), "crew_invitation_invalid");
                assert!(!error.to_string().contains("talked about"), "{error}");
            }
        }
        assert!(manager.list().await.is_empty());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_preview_saves_nothing_and_touches_no_credential() {
        let root = fixture_root("preview");
        let manager = CrewManager::new(root.clone()).unwrap();
        let message = crew_invitation::message(&lab_invitation()).unwrap();
        let preview = preview_of(
            manager
                .connection_from_invitation(&message, true, InvitationOverrides::default())
                .await
                .unwrap(),
        );
        assert!(preview.existing_connection_id.is_none());
        assert!(manager.list().await.is_empty());
        assert!(manager.registry.lock().await.pending_device.is_none());
        assert_eq!(
            fs::read_dir(&root).unwrap().count(),
            0,
            "a preview writes nothing: no registry, no credential"
        );
        let _ = fs::remove_dir_all(root);
    }

    async fn preview_with(
        manager: &CrewManager,
        text: &str,
        overrides: InvitationOverrides,
    ) -> Result<InvitationPreview> {
        Ok(preview_of(
            manager
                .connection_from_invitation(text, true, overrides)
                .await?,
        ))
    }

    #[tokio::test]
    async fn a_preview_applies_the_persons_choices_and_only_the_invitations_hints() {
        let root = fixture_root("choices");
        let manager = CrewManager::new(root.clone()).unwrap();
        let text = crew_invitation::message(&WorkspaceInvitation {
            ssh_port: Some(2222),
            proxy_jump: Some("gateway.example.org".into()),
            ..lab_invitation()
        })
        .unwrap();
        let hinted = preview_with(&manager, &text, InvitationOverrides::default())
            .await
            .unwrap();
        assert_eq!(
            (hinted.port, hinted.proxy_jump.as_deref()),
            (Some(2222), Some("gateway.example.org"))
        );
        // A login from the person's own SSH settings brings its own port and route.
        let alias = InvitationOverrides {
            advanced: InvitationAdvanced {
                ssh_target: Some("hpc".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let alias = preview_with(&manager, &text, alias).await.unwrap();
        assert_eq!(alias.ssh_target.as_deref(), Some("hpc"));
        assert_eq!((alias.port, alias.proxy_jump), (None, None));
        let no_jump = InvitationOverrides {
            advanced: InvitationAdvanced {
                proxy_jump: Some(String::new()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            preview_with(&manager, &text, no_jump)
                .await
                .unwrap()
                .proxy_jump,
            None
        );
        let public = InvitationOverrides {
            username: Some("@robert".into()),
            mode: Some(ClusterMode::Public),
            ..Default::default()
        };
        let public = preview_with(&manager, &text, public).await.unwrap();
        assert!(public.mode_differs && public.missing.is_empty());
        assert_eq!(public.ssh_target.as_deref(), Some("robert@hpc.example.org"));
        assert_eq!(public.institution_id.as_deref(), Some("ucsf"));
        for bad in [
            InvitationOverrides {
                username: Some("b ob".into()),
                ..Default::default()
            },
            InvitationOverrides {
                institution_id: Some("UCSF Health!".into()),
                ..Default::default()
            },
            InvitationOverrides {
                advanced: InvitationAdvanced {
                    ssh_target: Some("-oProxyCommand=evil".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
        ] {
            let error = preview_with(&manager, &text, bad).await.unwrap_err();
            assert_eq!(refused(&error).reason(), InvitationRefusal::InvalidChoice);
        }
        assert!(manager.list().await.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_preview_names_the_saved_connection_and_never_re_pins_a_workspace() {
        let root = fixture_root("saved");
        let manager = CrewManager::new(root.clone()).unwrap();
        let text = crew_invitation::message(&lab_invitation()).unwrap();
        let key = hex(&workspace_key());
        let other_workspace = "3f2a9c1e-77b0-4d4e-8a11-0000000000b2";
        manager.registry.lock().await.connections = vec![
            saved_connection("same", "lab", WORKSPACE_ID, &key),
            saved_connection("elsewhere", "lab", other_workspace, &"12".repeat(32)),
        ];
        let preview = preview_with(&manager, &text, InvitationOverrides::default())
            .await
            .unwrap();
        assert_eq!(preview.existing_connection_id.as_deref(), Some("same"));
        manager.registry.lock().await.connections.remove(0);
        let preview = preview_with(&manager, &text, InvitationOverrides::default())
            .await
            .unwrap();
        assert_eq!(preview.existing_connection_id, None);
        assert_eq!(preview.name, "lab \u{2014} hpc.example.org");
        // The same workspace ID pinned to another key is refused, preview or save.
        manager
            .registry
            .lock()
            .await
            .connections
            .push(saved_connection(
                "tampered",
                "lab",
                WORKSPACE_ID,
                &hex(&SigningKey::from_bytes(&[3; 32]).verifying_key().to_bytes()),
            ));
        for preview in [true, false] {
            let error = manager
                .connection_from_invitation(&text, preview, InvitationOverrides::default())
                .await
                .unwrap_err();
            let refusal = refused(&error);
            assert_eq!(refusal.reason(), InvitationRefusal::IdentityConflict);
            assert_eq!(refusal.api_code(), "crew_invitation_conflict");
            assert_eq!(refusal.connection_id(), Some("tampered"));
        }
        assert_eq!(manager.list().await.len(), 2);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_pending_answer_maps_to_each_join_state() {
        let far = now_seconds() + 3600;
        let invited = json!({"invited": true, "join_id": "join-1", "workspace_name": "lab",
            "inviter": {"username": "alice", "display_name": "Alice Chen"},
            "add_device": false, "approved": false, "expires_at": far, "expired": false});
        let with = |patch: Value| {
            let mut answer = invited.clone();
            for (key, value) in patch.as_object().unwrap() {
                answer[key] = value.clone();
            }
            answer
        };
        let state_if = |answer: &Value, refused_here: bool| {
            PendingInvitation::read(answer)
                .unwrap()
                .map(|pending| pending.state(refused_here))
        };
        let state = |answer: &Value| state_if(answer, false);
        assert_eq!(state(&invited), Some(JoinState::Invited));
        assert_eq!(
            state(&with(json!({"approved": true}))),
            Some(JoinState::Approved)
        );
        // The join-wide refusal alone is not this computer's: it may be another device's.
        let refused = with(json!({"approved": true, "last_refusal": "code_mismatch"}));
        assert!(
            PendingInvitation::read(&refused)
                .unwrap()
                .unwrap()
                .last_refusal
        );
        assert_eq!(state(&refused), Some(JoinState::Approved));
        assert_eq!(state_if(&refused, true), Some(JoinState::CodeMismatch));
        assert_eq!(
            state_if(&with(json!({"expired": true, "approved": true})), true),
            Some(JoinState::Expired)
        );
        assert_eq!(
            state(&with(json!({"expired": null, "expires_at": 1}))),
            Some(JoinState::Expired)
        );
        assert_eq!(state(&json!({"invited": false})), None);
        let pending = PendingInvitation::read(&invited).unwrap().unwrap();
        assert_eq!(pending.join_id, "join-1");
        assert_eq!(
            pending.inviter,
            Some(JoinPerson {
                username: "alice".into(),
                display_name: "Alice Chen".into()
            })
        );
        // A display name with nothing visible left is shown as the username; a username that
        // is not an account name drops the inviter rather than showing it.
        let blank =
            with(json!({"inviter": {"username": "alice", "display_name": "\u{200b}\u{202e}"}}));
        let pending = PendingInvitation::read(&blank).unwrap().unwrap();
        assert_eq!(pending.inviter.unwrap().display_name, "alice");
        let control = with(json!({"inviter": {"username": "al\u{7}ice"}}));
        assert!(PendingInvitation::read(&control)
            .unwrap()
            .unwrap()
            .inviter
            .is_none());
        for malformed in [
            json!({}),
            json!({"invited": "yes"}),
            json!({"invited": true}),
            with(json!({"join_id": "join 1"})),
            with(json!({"join_id": "x".repeat(129)})),
            with(json!({"approved": "true"})),
            with(json!({"expires_at": "soon"})),
        ] {
            assert!(PendingInvitation::read(&malformed).is_err(), "{malformed}");
        }
    }

    #[test]
    fn only_this_computers_own_refusal_withholds_a_claim() {
        let pending = |join_id: &str, last_refusal: bool| {
            let mut answer = json!({"invited": true, "join_id": join_id, "approved": true});
            if last_refusal {
                answer["last_refusal"] = json!("code_mismatch");
            }
            PendingInvitation::read(&answer).unwrap()
        };
        let claim = RefusedClaim {
            device_id: "d-desk".into(),
            join_id: "join-1".into(),
            at: 10,
        };
        let verdict = |device: &str, answer: Option<PendingInvitation>, asked_at: u64| {
            let verdict = claim.judge(device, answer.as_ref(), asked_at);
            (verdict.refused_here, verdict.keep)
        };
        // Asked after the refusal was recorded: it stands while the workspace still reports a
        // refusal for this join, and an answer without one (a new approval) forgets it.
        assert_eq!(verdict("d-desk", pending("join-1", true), 11), (true, true));
        assert_eq!(
            verdict("d-desk", pending("join-1", false), 11),
            (false, false)
        );
        // A new invitation, or none, outdates it; so does another device's key.
        assert_eq!(
            verdict("d-desk", pending("join-2", true), 11),
            (false, false)
        );
        assert_eq!(verdict("d-desk", None, 11), (false, false));
        assert_eq!(
            verdict("d-lap", pending("join-1", true), 11),
            (false, false)
        );
        // An answer to a question asked before the refusal was recorded may predate it: it can
        // neither clear the record nor contradict it.
        assert_eq!(verdict("d-desk", pending("join-1", false), 9), (true, true));
        assert_eq!(verdict("d-desk", None, 9), (false, true));
        assert_eq!(
            verdict("d-desk", pending("join-2", false), 9),
            (false, true)
        );
        assert!(claim_clock_tick() < claim_clock_tick());
    }

    #[test]
    fn a_broker_refusal_is_read_only_from_the_transport_envelope() {
        let refusal = anyhow::anyhow!(
            "{BROKER_REFUSAL_PREFIX}{}",
            json!({"code": "code_mismatch", "message": "code_mismatch: no"})
        );
        assert_eq!(
            broker_refusal(&refusal),
            Some(("code_mismatch".into(), "code_mismatch: no".into()))
        );
        assert_eq!(broker_refusal(&anyhow::anyhow!("code_mismatch: no")), None);
        assert_eq!(
            broker_refusal(&anyhow::anyhow!("{BROKER_REFUSAL_PREFIX}not json")),
            None
        );
        assert_eq!(
            JoinRefusal::from_broker_code("code_mismatch").api_code(),
            "crew_join_code_mismatch"
        );
        assert_eq!(
            JoinRefusal::from_broker_code("something_new"),
            JoinRefusal::Refused
        );
    }

    #[test]
    fn jump_hops_keep_the_host_and_port_and_drop_the_user() {
        assert_eq!(
            jump_hop("alice@gw.example.org:2222"),
            Some(("gw.example.org".into(), Some(2222)))
        );
        assert_eq!(jump_hop("ssh://gw"), Some(("gw".into(), None)));
        for refused in [
            "-oProxyCommand=evil",
            "[::1]:22",
            "gw:0",
            "gw:port",
            "",
            "a/b",
        ] {
            assert_eq!(jump_hop(refused), None, "{refused}");
        }
    }

    #[test]
    fn the_device_code_is_the_one_the_broker_checks() {
        let device = SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes();
        let code = biorouter_crew::format_device_code(&biorouter_crew::device_code(
            WORKSPACE_ID,
            &workspace_key(),
            &device,
        ));
        assert_eq!(code.len(), 19);
        assert!(biorouter_crew::device_code_matches(
            &code,
            WORKSPACE_ID,
            &workspace_key(),
            &device
        ));
        let other = SigningKey::from_bytes(&[8; 32]).verifying_key().to_bytes();
        assert!(!biorouter_crew::device_code_matches(
            &code,
            WORKSPACE_ID,
            &workspace_key(),
            &other
        ));
    }

    #[tokio::test]
    async fn a_failed_handoff_is_typed_for_people_and_keeps_its_diagnostic_for_logs() {
        let manager = manager().unwrap();
        let connection_id = format!("handoff-failure-{}", uuid::Uuid::new_v4());
        manager.registry.lock().await.connections.push(Connection {
            id: connection_id.clone(),
            ..saved_connection("", "lab", WORKSPACE_ID, &hex(&workspace_key()))
        });
        let auth_id = uuid::Uuid::new_v4().to_string();
        let controller = uuid::Uuid::new_v4().to_string();
        let session = AuthSession {
            info: AuthenticationSession {
                authentication_id: auth_id.clone(),
                connection_id: connection_id.clone(),
                controller_id: controller.clone(),
                instance_id: INSTANCE.clone(),
            },
            request_id: uuid::Uuid::new_v4().to_string(),
            binding: Mutex::new(binding(&connection_id).await.unwrap()),
            adopted: Arc::new(AtomicBool::new(false)),
            plan: AuthenticationPlan {
                program: "ssh".into(),
                args: Vec::new(),
                connection_id: connection_id.clone(),
                authentication_id: auth_id.clone(),
            },
            created: Instant::now(),
            size: dimensions(80, 24).unwrap(),
            started: Mutex::new(true),
            runtime: Mutex::new(None),
        };
        SESSIONS
            .lock()
            .unwrap()
            .insert(auth_id.clone(), Arc::new(session));

        let error = handoff(&auth_id, &controller).await.unwrap_err();
        let typed = error
            .downcast_ref::<HandoffFailed>()
            .expect("a failed handoff is typed");
        assert_eq!(typed.api_code(), "crew_handoff_failed");
        assert_eq!(error.to_string(), HANDOFF_FAILED_MESSAGE);
        assert!(typed.log_message().starts_with(HANDOFF_FAILURE_MESSAGE));
        assert!(typed
            .log_message()
            .contains("Authentication terminal unavailable"));
        assert!(!typed.workspace_identity_mismatch());
        let saved = manager.connection(&connection_id).await.unwrap();
        assert_eq!(saved.status, "disconnected");
        assert_eq!(saved.last_error.as_deref(), Some(HANDOFF_FAILED_MESSAGE));

        let identity = HandoffFailed::wrap(WorkspaceIdentityError::wrap(anyhow::anyhow!(
            "Workspace identity mismatch"
        )));
        let identity = identity.downcast_ref::<HandoffFailed>().unwrap();
        assert!(identity.workspace_identity_mismatch());
        assert_eq!(identity.api_code(), HANDOFF_FAILED_CODE);

        SESSIONS.lock().unwrap().remove(&auth_id);
        manager
            .registry
            .lock()
            .await
            .connections
            .retain(|c| c.id != connection_id);
    }

    /// Point the profile, credentials and `ssh` of this process at `root`: file credentials
    /// under a development profile, never the OS keychain. Only inside a process of its own.
    #[cfg(unix)]
    fn isolated_env(root: &Path) -> env_lock::EnvGuard<'static> {
        let profile_root = root.join("profile");
        fs::create_dir_all(&profile_root).unwrap();
        let original_path = std::env::var("PATH").unwrap_or_default();
        let path = format!("{}:{original_path}", root.join("bin").display());
        let profile = profile_root.to_string_lossy().into_owned();
        crate::test_sandbox::relocate_path_root_and(
            profile.as_str(),
            [
                ("BIOROUTER_DEV_PROFILE_ROOT", Some(profile.as_str())),
                ("BIOROUTER_DISABLE_KEYRING", Some("true")),
                ("PATH", Some(path.as_str())),
            ],
        )
    }

    /// A fake `ssh`. `-G` prints `hosts`' lines for its last argument first, then settings the
    /// SSH preflight accepts. As a bridge, it logs every request line and answers
    /// `auth.challenge`, then each of `answers` (a method and its whole envelope, `{"result":
    /// …}` or `{"error": …}`), then anything else with `{"accepted_method":"fixture"}`.
    #[cfg(unix)]
    fn write_fake_ssh(root: &Path, hosts: &[(&str, &[&str])], answers: &[(&str, Value)]) {
        use std::os::unix::fs::PermissionsExt;
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let quoted = |text: &str| {
            assert!(
                !text.contains('\'') && !text.contains('%'),
                "fixture text must survive sh quoting and printf: {text}"
            );
            format!("'{text}'")
        };
        let mut cases = String::new();
        for (host, lines) in hosts {
            let lines: Vec<String> = lines.iter().map(|line| quoted(line)).collect();
            cases.push_str(&format!(
                "    {}) printf '%s\\n' {} ;;\n",
                quoted(host),
                lines.join(" ")
            ));
        }
        let challenge = json!({"workspace_id": WORKSPACE_ID, "nonce": "nonce", "uid": 10001});
        let mut branches = format!(
            "  if printf '%s\\n' \"$line\" | grep -q '\"method\":\"auth.challenge\"'; then\n    printf '{{\"id\":\"%s\",\"result\":%s}}\\n' \"$id\" {}\n",
            quoted(&challenge.to_string())
        );
        for (method, envelope) in answers {
            let envelope = envelope.to_string();
            let rest = envelope
                .strip_prefix('{')
                .expect("an envelope is an object");
            branches.push_str(&format!(
                "  elif printf '%s\\n' \"$line\" | grep -q '\"method\":\"{method}\"'; then\n    printf '{{\"id\":\"%s\",%s\\n' \"$id\" {}\n",
                quoted(rest)
            ));
        }
        let script = format!(
            r#"#!/bin/sh
log='{log}'
if [ "$1" = "-G" ]; then
  for last; do :; done
  case "$last" in
{cases}  esac
  printf '%s\n' 'hostname 127.0.0.1' 'port 22' 'stricthostkeychecking yes' \
    'forwardagent no' 'forwardx11 no' 'permitlocalcommand no' \
    'clearallforwardings yes' 'nohostauthenticationforlocalhost no' \
    'tunnel no' 'forkafterauthentication no' \
    'gssapidelegatecredentials no' 'proxycommand none' \
    'controlmaster no' 'controlpersist no' 'controlpath none'
  exit 0
fi
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$log"
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
{branches}  else
    printf '{{"id":"%s","result":{{"accepted_method":"fixture"}}}}\n' "$id"
  fi
done
"#,
            log = root.join("requests.log").display()
        );
        let ssh = bin.join("ssh");
        fs::write(&ssh, script).unwrap();
        fs::set_permissions(&ssh, fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[cfg(unix)]
    fn logged(root: &Path, method: &str) -> Vec<String> {
        let needle = format!("\"method\":\"{method}\"");
        fs::read_to_string(root.join("requests.log"))
            .unwrap_or_default()
            .lines()
            .filter(|line| line.contains(&needle))
            .map(str::to_owned)
            .collect()
    }

    /// A fresh bridge for `connection`, replacing (and closing) any earlier one.
    #[cfg(unix)]
    async fn attach_transport(manager: &CrewManager, connection: &Connection) {
        let control = manager.control_path(&connection.id).unwrap();
        let fresh = transport::Transport::connect(connection, &control)
            .await
            .unwrap();
        let old = manager.transports.lock().await.insert(
            connection.id.clone(),
            Arc::new(tokio::sync::Mutex::new(fresh)),
        );
        if let Some(old) = old {
            old.lock().await.close().await;
        }
    }

    /// What the broker's `hello` announced, as a verified connect would remember it.
    fn remember_hello(
        manager: &CrewManager,
        id: &str,
        signature_version: u8,
        capabilities: &[&str],
    ) {
        let signed = signature_version >= 2;
        manager.brokers.lock().unwrap().insert(
            id.into(),
            BrokerHello {
                signature_version,
                capabilities: capabilities.iter().map(|c| (*c).to_owned()).collect(),
                workspace_name: signed.then(|| "lab".into()),
                mode: signed.then_some(ClusterMode::Private),
                institution_id: signed.then(|| "ucsf".into()),
                policy_epoch: signed.then_some(1),
            },
        );
    }

    /// A connected manager holding `connection`, whose device key is `device`.
    #[cfg(unix)]
    async fn connected_manager(
        root: &Path,
        connection: &Connection,
        device: &SigningKey,
        capabilities: &[&str],
    ) -> Arc<CrewManager> {
        let registry = Registry {
            connections: vec![connection.clone()],
            ..Default::default()
        };
        fs::write(
            root.join("connections.json"),
            serde_json::to_vec(&registry).unwrap(),
        )
        .unwrap();
        let manager = Arc::new(CrewManager::new(root.to_owned()).unwrap());
        manager
            .write_credential(
                &format!("device:{}", connection.id),
                &hex(&device.to_bytes()),
            )
            .unwrap();
        remember_hello(&manager, &connection.id, 2, capabilities);
        attach_transport(&manager, connection).await;
        manager
    }

    /// Bob's connection to `lab`, with the device key it saved.
    fn joiner(id: &str) -> (Connection, SigningKey) {
        let device = SigningKey::from_bytes(&[7; 32]);
        (
            saved_connection(id, "lab", WORKSPACE_ID, &hex(&workspace_key())),
            device,
        )
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_private_invitation_saves_pinned_with_its_institution() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("save");
        let _env = isolated_env(&root);
        let profile = root.join("crew");
        let manager = CrewManager::new(profile.clone()).unwrap();
        let message = crew_invitation::message(&lab_invitation()).unwrap();
        preview_with(&manager, &message, InvitationOverrides::default())
            .await
            .unwrap();
        assert!(!profile.exists(), "a preview writes nothing");

        let saved = saved_of(
            manager
                .connection_from_invitation(&message, false, InvitationOverrides::default())
                .await
                .unwrap(),
        );
        assert_eq!(
            (saved.mode, saved.institution_id.as_deref()),
            (ClusterMode::Private, Some("ucsf"))
        );
        assert_eq!(
            (
                saved.workspace_id.as_str(),
                saved.workspace_public_key.as_str(),
                saved.socket_path.as_str(),
                saved.owner_uid
            ),
            (WORKSPACE_ID, hex(&workspace_key()).as_str(), SOCKET, 1000)
        );
        assert_eq!(saved.ssh_target, "bob@hpc.example.org");
        assert_eq!((saved.name.as_str(), saved.port), ("lab", None));
        let secret: [u8; 32] = unhex(
            &manager
                .read_credential(&format!("device:{}", saved.id))
                .unwrap(),
        )
        .unwrap()
        .try_into()
        .unwrap();
        assert_eq!(
            hex(&SigningKey::from_bytes(&secret).verifying_key().to_bytes()),
            saved.public_key
        );

        // Pasting the same invitation again keeps the one device key (and so the one code).
        let again = saved_of(
            manager
                .connection_from_invitation(&message, false, InvitationOverrides::default())
                .await
                .unwrap(),
        );
        assert_eq!(
            (again.id.as_str(), again.public_key.as_str()),
            (saved.id.as_str(), saved.public_key.as_str())
        );
        let robert = InvitationOverrides {
            username: Some("robert".into()),
            ..Default::default()
        };
        let error = manager
            .connection_from_invitation(&message, false, robert)
            .await
            .unwrap_err();
        assert_eq!(refused(&error).api_code(), "crew_connection_exists");
        assert_eq!(refused(&error).connection_id(), Some(saved.id.as_str()));
        let restarted = CrewManager::new(profile).unwrap();
        assert_eq!(restarted.list().await.len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_private_choice_without_an_institution_saves_nothing() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("save-public");
        let _env = isolated_env(&root);
        let manager = CrewManager::new(root.join("crew")).unwrap();
        let public = crew_invitation::message(&WorkspaceInvitation {
            mode: Some(biorouter_crew::Mode::Public),
            institution_id: None,
            ..lab_invitation()
        })
        .unwrap();
        let private = InvitationOverrides {
            mode: Some(ClusterMode::Private),
            ..Default::default()
        };
        let error = manager
            .connection_from_invitation(&public, false, private)
            .await
            .unwrap_err();
        assert_eq!(
            refused(&error).reason(),
            InvitationRefusal::Missing(InvitationMissing::Institution)
        );
        assert!(manager.list().await.is_empty());
        assert!(!root.join("crew").exists(), "a refused save writes nothing");
        let joined_public = saved_of(
            manager
                .connection_from_invitation(&public, false, InvitationOverrides::default())
                .await
                .unwrap(),
        );
        assert_eq!(
            (joined_public.mode, joined_public.institution_id),
            (ClusterMode::Public, None)
        );
        let _ = fs::remove_dir_all(root);
    }

    /// Bob's computer, connected to a broker that announced `capabilities`, in this process's
    /// own environment. The fake `ssh` is rewritten per case.
    #[cfg(unix)]
    async fn joiner_fixture(
        label: &str,
        id: &str,
        capabilities: &[&str],
    ) -> (
        PathBuf,
        env_lock::EnvGuard<'static>,
        Connection,
        Arc<CrewManager>,
    ) {
        let root = fixture_root(label);
        let (connection, device) = joiner(id);
        write_fake_ssh(&root, &[], &[]);
        let env = isolated_env(&root);
        let manager = connected_manager(&root, &connection, &device, capabilities).await;
        (root, env, connection, manager)
    }

    /// Each `enrollment.pending` answer (with the membership probe's), and the state it means.
    fn status_cases() -> Vec<(Value, Value, JoinState)> {
        let invited = json!({"invited": true, "join_id": "join-1", "workspace_name": "lab",
            "inviter": {"username": "alice", "display_name": "Alice Chen"},
            "approved": false, "expires_at": now_seconds() + 3600, "expired": false,
            "code": "ZZZZ-ZZZZ-ZZZZ-ZZZZ"});
        let answer = |patch: Value| {
            let mut answer = invited.clone();
            for (key, value) in patch.as_object().unwrap() {
                answer[key] = value.clone();
            }
            json!({"result": answer})
        };
        let stranger =
            json!({"error": {"code": "unauthorized", "message": "unauthorized: unknown device"}});
        let not_invited = json!({"result": {"invited": false}});
        vec![
            (answer(json!({})), stranger.clone(), JoinState::Invited),
            (
                answer(json!({"approved": true})),
                stranger.clone(),
                JoinState::Approved,
            ),
            // Another device's refusal: this computer never claimed, so its code may be the
            // approved one.
            (
                answer(json!({"approved": true, "last_refusal": "code_mismatch"})),
                stranger.clone(),
                JoinState::Approved,
            ),
            (
                answer(json!({"expired": true})),
                stranger.clone(),
                JoinState::Expired,
            ),
            (not_invited.clone(), stranger.clone(), JoinState::NotInvited),
            (
                not_invited,
                json!({"result": {"full_name": null}}),
                JoinState::Joined,
            ),
            (
                json!({"error": {"code": "unauthorized", "message": "unauthorized: signed device required"}}),
                stranger,
                JoinState::Unsupported,
            ),
        ]
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn join_status_maps_each_answer_and_computes_the_code_here() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let (root, _env, connection, manager) = joiner_fixture(
            "join-status",
            "4b4b4b4b-4b4b-44b4-84b4-4b4b4b4b4b4b",
            &[JOIN_BY_NAME_CAPABILITY],
        )
        .await;
        let public = SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes();
        let local = biorouter_crew::format_device_code(&biorouter_crew::device_code(
            WORKSPACE_ID,
            &workspace_key(),
            &public,
        ));
        assert_eq!(manager.device_code(&connection.id).await.unwrap(), local);
        let cases = status_cases();
        let count = cases.len();
        for (pending, probe, expected) in cases {
            write_fake_ssh(
                &root,
                &[],
                &[("enrollment.pending", pending), ("profile.suggest", probe)],
            );
            attach_transport(&manager, &connection).await;
            let status = manager.join_status(&connection.id).await.unwrap();
            assert_eq!(status.status, expected);
            let shows_code = matches!(
                expected,
                JoinState::Invited | JoinState::Approved | JoinState::CodeMismatch
            );
            // The code is this computer's, never the answer's, and it is the one the broker
            // checks at `auth.join`.
            assert_eq!(status.code.as_deref(), shows_code.then_some(local.as_str()));
            if let Some(code) = &status.code {
                assert!(biorouter_crew::device_code_matches(
                    code,
                    WORKSPACE_ID,
                    &workspace_key(),
                    &public
                ));
                assert!(!serde_json::to_string(&status).unwrap().contains("ZZZZ"));
                assert_eq!(status.inviter.as_ref().unwrap().display_name, "Alice Chen");
            }
            assert_eq!(status.workspace_name.as_deref(), Some("lab"));
        }
        let pending = logged(&root, "enrollment.pending");
        assert_eq!(pending.len(), count);
        assert!(
            pending
                .iter()
                .all(|line| !line.contains("\"auth\"") && !line.contains("signature")),
            "the join status is read unsigned"
        );
        assert!(
            logged(&root, "auth.join").is_empty(),
            "a status read never claims"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn join_status_asks_nothing_without_the_capability_or_a_connection() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let (root, _env, connection, manager) = joiner_fixture(
            "join-status-unsupported",
            "4c4c4c4c-4c4c-44c4-84c4-4c4c4c4c4c4c",
            &["human_chat", "human_names_v1"],
        )
        .await;
        let status = manager.join_status(&connection.id).await.unwrap();
        assert_eq!((status.status, status.code), (JoinState::Unsupported, None));
        assert!(
            !root.join("requests.log").exists(),
            "nothing is sent to a broker without the capability"
        );
        let error = manager.join(&connection.id).await.unwrap_err();
        assert_eq!(
            error.downcast_ref::<JoinRefused>().unwrap().api_code(),
            "crew_join_unsupported"
        );
        assert!(!root.join("requests.log").exists());
        manager.disconnect(&connection.id).await.unwrap();
        let error = manager.join_status(&connection.id).await.unwrap_err();
        assert!(error.to_string().contains("disconnected"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    /// One join: the status answer, the membership probe's, `auth.join`'s, the expected refusal
    /// (`None`: joined) and how many claims reach the workspace.
    type ClaimCase = (Value, Value, Value, Option<JoinRefusal>, usize);

    fn claim_cases(device_id: &str) -> Vec<ClaimCase> {
        let pending = |approved: bool, refused: bool| {
            let mut answer = json!({"invited": true, "join_id": "join-1", "approved": approved,
                "expires_at": now_seconds() + 3600});
            if refused {
                answer["last_refusal"] = json!("code_mismatch");
            }
            json!({"result": answer})
        };
        let stranger =
            json!({"error": {"code": "unauthorized", "message": "unauthorized: unknown device"}});
        let joined = json!({"result": {"principal": {"username": "bob", "display_name": "bob"},
            "device_id": device_id, "workspace": {"id": WORKSPACE_ID}}});
        let mismatch = json!({"error": {"code": "code_mismatch", "message": "code_mismatch: not this device"}});
        let not_invited = json!({"result": {"invited": false}});
        // In order: the manager remembers its own refused claim from one case to the next.
        vec![
            (
                pending(false, false),
                stranger.clone(),
                joined.clone(),
                Some(JoinRefusal::NotApproved),
                0,
            ),
            // Another device's refusal under the approval: this computer never claimed, so it
            // claims exactly once.
            (
                pending(true, true),
                stranger.clone(),
                joined.clone(),
                None,
                1,
            ),
            (
                not_invited.clone(),
                stranger.clone(),
                joined.clone(),
                Some(JoinRefusal::NotInvited),
                0,
            ),
            (not_invited, json!({"result": {}}), joined.clone(), None, 0),
            // This computer's own claim refused: remembered.
            (
                pending(true, false),
                stranger.clone(),
                mismatch,
                Some(JoinRefusal::CodeMismatch),
                1,
            ),
            // The workspace still reports it: no second claim under the same approval.
            (
                pending(true, true),
                stranger.clone(),
                joined.clone(),
                Some(JoinRefusal::CodeMismatch),
                0,
            ),
            // It no longer does (the host approved another code): claim once more.
            (pending(true, false), stranger, joined, None, 1),
        ]
    }

    /// Every `auth.join` sent carries this computer's key and the join's ID, signed.
    #[cfg(unix)]
    fn assert_claims(root: &Path, connection: &Connection, claims: usize, case: usize) {
        let sent = logged(root, "auth.join");
        assert_eq!(sent.len(), claims, "case {case}: claims sent");
        for line in sent {
            let frame: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(frame["params"]["join_id"], "join-1");
            assert_eq!(frame["params"]["public_key"], json!(connection.public_key));
            assert_eq!(frame["auth"]["device_id"], json!(connection.device_id));
            assert!(frame["auth"]["signature"]
                .as_str()
                .is_some_and(|s| s.len() == 128));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn join_claims_only_when_the_host_approved() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let (root, _env, connection, manager) = joiner_fixture(
            "join",
            "5c5c5c5c-5c5c-45c5-85c5-5c5c5c5c5c5c",
            &[JOIN_BY_NAME_CAPABILITY],
        )
        .await;
        let cases = claim_cases(&connection.device_id);
        for (case, (status, probe, claim, refusal, claims)) in cases.into_iter().enumerate() {
            let _ = fs::remove_file(root.join("requests.log"));
            write_fake_ssh(
                &root,
                &[],
                &[
                    ("enrollment.pending", status),
                    ("profile.suggest", probe),
                    ("auth.join", claim),
                ],
            );
            attach_transport(&manager, &connection).await;
            let result = manager.join(&connection.id).await;
            match refusal {
                None => assert_eq!(result.unwrap().status, JoinState::Joined, "case {case}"),
                Some(refusal) => {
                    let error = result.unwrap_err();
                    let typed = error
                        .downcast_ref::<JoinRefused>()
                        .unwrap_or_else(|| panic!("case {case}: {error:#}"));
                    assert_eq!(typed.refusal(), refusal, "case {case}");
                    assert_eq!(error.to_string(), refusal.message());
                }
            }
            assert_claims(&root, &connection, claims, case);
        }
        // An answer naming another device is not taken as joined.
        let approved = json!({"result": {"invited": true, "join_id": "join-1", "approved": true}});
        write_fake_ssh(
            &root,
            &[],
            &[
                ("enrollment.pending", approved),
                (
                    "auth.join",
                    json!({"result": {"device_id": "cd".repeat(32)}}),
                ),
            ],
        );
        attach_transport(&manager, &connection).await;
        let error = manager.join(&connection.id).await.unwrap_err();
        assert!(error.to_string().contains("different device"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    /// Answer the next `enrollment.pending` with `pending` and `auth.join` with `claim`.
    #[cfg(unix)]
    async fn answer_join(
        root: &Path,
        manager: &CrewManager,
        connection: &Connection,
        pending: &Value,
        claim: &Value,
    ) {
        write_fake_ssh(
            root,
            &[],
            &[
                ("enrollment.pending", pending.clone()),
                ("auth.join", claim.clone()),
            ],
        );
        attach_transport(manager, connection).await;
    }

    /// Bob pastes the invitation on his desktop and his laptop, and Alice approves the
    /// desktop's code. The laptop claims first and is refused, which puts a join-wide
    /// `last_refusal` on the account. The desktop must still show `approved` and claim; only a
    /// refusal of its own claim makes it `code_mismatch`, and only until the approval changes.
    #[cfg(unix)]
    #[tokio::test]
    async fn another_devices_refusal_never_strands_the_approved_computer() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let (root, _env, connection, manager) = joiner_fixture(
            "join-refusal",
            "5d5d5d5d-5d5d-45d5-85d5-5d5d5d5d5d5d",
            &[JOIN_BY_NAME_CAPABILITY],
        )
        .await;
        let id = connection.id.as_str();
        let refused = json!({"result": {"invited": true, "join_id": "join-1", "approved": true,
            "last_refusal": "code_mismatch", "expires_at": now_seconds() + 3600}});
        let mut approved = refused.clone();
        approved["result"]
            .as_object_mut()
            .unwrap()
            .remove("last_refusal");
        let mismatch = json!({"error": {"code": "code_mismatch", "message": "code_mismatch: not this device"}});
        let joined = json!({"result": {"device_id": connection.device_id}});
        let refusal = |error: anyhow::Error| error.downcast_ref::<JoinRefused>().unwrap().refusal();

        // The laptop's refusal: the desktop shows its code as approved and claims once.
        answer_join(&root, &manager, &connection, &refused, &joined).await;
        let status = manager.join_status(id).await.unwrap();
        assert_eq!(status.status, JoinState::Approved);
        assert_eq!(status.code, Some(manager.device_code(id).await.unwrap()));
        assert_eq!(manager.join(id).await.unwrap().status, JoinState::Joined);
        assert_claims(&root, &connection, 1, 0);

        // Its own claim refused: `code_mismatch` while the workspace still reports a refusal,
        // and no second claim under that approval.
        answer_join(&root, &manager, &connection, &approved, &mismatch).await;
        let error = manager.join(id).await.unwrap_err();
        assert_eq!(refusal(error), JoinRefusal::CodeMismatch);
        assert_claims(&root, &connection, 2, 1);
        answer_join(&root, &manager, &connection, &refused, &joined).await;
        let status = manager.join_status(id).await.unwrap();
        assert_eq!(status.status, JoinState::CodeMismatch);
        assert!(status.code.is_some());
        let error = manager.join(id).await.unwrap_err();
        assert_eq!(refusal(error), JoinRefusal::CodeMismatch);
        assert_claims(&root, &connection, 2, 2);

        // The host approved another code: forgotten, and a later refusal (another device's,
        // under the new approval) no longer holds this computer back.
        answer_join(&root, &manager, &connection, &approved, &joined).await;
        assert_eq!(
            manager.join_status(id).await.unwrap().status,
            JoinState::Approved
        );
        answer_join(&root, &manager, &connection, &refused, &joined).await;
        assert_eq!(
            manager.join_status(id).await.unwrap().status,
            JoinState::Approved
        );
        assert_eq!(manager.join(id).await.unwrap().status, JoinState::Joined);
        assert_claims(&root, &connection, 3, 3);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_invitation_never_carries_an_alias_or_a_local_name() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("invitation-for");
        let host_key = SigningKey::from_bytes(&[5; 32]);
        let host_public = host_key.verifying_key().to_bytes();
        let connection = Connection {
            name: "Alice private nickname".into(),
            ssh_target: "alice@hpc-alias".into(),
            remote_root: Some("/home/alice/private-folder".into()),
            device_id: hex(&Sha256::digest(host_public)),
            public_key: hex(&host_public),
            ..saved_connection(
                "6d6d6d6d-6d6d-46d6-86d6-6d6d6d6d6d6d",
                "",
                WORKSPACE_ID,
                &hex(&workspace_key()),
            )
        };
        let snapshot = json!({"result": {
        "workspace": {"id": WORKSPACE_ID, "host_uid": 1000, "name": "lab", "mode": "private",
            "institution_id": "ucsf", "policy_epoch": 1, "host_principal_id": "p-alice"},
        "principals": [
            {"id": "p-carol", "uid": 1002, "username": "carol", "display_name": "carol", "active": true},
            {"id": "p-alice", "uid": 1000, "username": "alice", "display_name": "Alice Chen", "active": true}
        ]}});
        write_fake_ssh(
            &root,
            &[
                (
                    "alice@hpc-alias",
                    &[
                        "hostname hpc.example.org",
                        "port 2222",
                        "proxyjump gw-alias",
                    ],
                ),
                ("gw-alias", &["hostname gateway.example.org", "port 22"]),
            ],
            &[("workspace.snapshot", snapshot)],
        );
        let _env = isolated_env(&root);
        let manager =
            connected_manager(&root, &connection, &host_key, &[JOIN_BY_NAME_CAPABILITY]).await;
        let expected = WorkspaceInvitation {
            ssh_port: Some(2222),
            proxy_jump: Some("gateway.example.org".into()),
            ..lab_invitation()
        };
        for (signature_version, invitee) in [(2, Some("@bob")), (1, None)] {
            remember_hello(
                &manager,
                &connection.id,
                signature_version,
                &[JOIN_BY_NAME_CAPABILITY],
            );
            let text = manager
                .invitation_for(&connection.id, invitee)
                .await
                .unwrap();
            assert!(text.message.contains(&text.line));
            assert!(text.message.starts_with("Join lab on Crew."));
            let parsed = crew_invitation::parse(&text.message).unwrap();
            assert_eq!(
                parsed.invitation,
                WorkspaceInvitation {
                    invitee_username: invitee.map(|_| "bob".into()),
                    ..expected.clone()
                }
            );
            let decoded = serde_json::to_string(&parsed.invitation).unwrap();
            for local in [
                "hpc-alias",
                "gw-alias",
                "private nickname",
                "private-folder",
            ] {
                assert!(
                    !decoded.contains(local) && !text.message.contains(local),
                    "{local}"
                );
            }
        }
        let error = manager
            .invitation_for(&connection.id, Some("b ob"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("username"), "{error}");
        manager.disconnect(&connection.id).await.unwrap();
        assert!(manager.invitation_for(&connection.id, None).await.is_err());
        let _ = fs::remove_dir_all(root);
    }

    /// A `hello` as a broker signs it over `nonce` (v1 and v2), for `connection`'s pinned
    /// workspace key (`workspace_key()`'s secret) and node.
    fn signed_hello(
        connection: &Connection,
        nonce: &str,
        institution_id: Option<&str>,
        policy_epoch: u64,
    ) -> Value {
        use ed25519_dalek::Signer;
        let key = SigningKey::from_bytes(&[9; 32]);
        let node = connection.node_id.clone().unwrap();
        let capabilities = [JOIN_BY_NAME_CAPABILITY, "direct_add_v1"];
        let v1 = hex(&key
            .sign(&biorouter_crew::hello_v1_payload(
                &connection.workspace_id,
                connection.owner_uid,
                nonce,
                &connection.workspace_public_key,
                &node,
            ))
            .to_bytes());
        let v2 = hex(&key
            .sign(
                &biorouter_crew::HelloV2 {
                    workspace_id: &connection.workspace_id,
                    host_uid: connection.owner_uid,
                    challenge_nonce: nonce,
                    workspace_public_key: &connection.workspace_public_key,
                    node_id: &node,
                    mode: &biorouter_crew::Mode::Private,
                    institution_id,
                    policy_epoch,
                    name: Some("lab"),
                    capabilities: &capabilities,
                }
                .signing_payload(),
            )
            .to_bytes());
        json!({
            "protocol": 1,
            "workspace_id": connection.workspace_id,
            "host_uid": connection.owner_uid,
            "workspace_public_key": connection.workspace_public_key,
            "challenge_nonce": nonce,
            "node_id": node,
            "mode": "private",
            "institution_id": institution_id,
            "policy_epoch": policy_epoch,
            "name": "lab",
            "capabilities": capabilities,
            "signature": v1,
            "signature_v2": v2,
        })
    }

    /// The host's own connection to the `lab` workspace, which a snapshot says is now Private
    /// `ucsf` at policy epoch 2, and a manager whose cached `hello` is the one from connect
    /// time, before the host set the institution.
    #[cfg(unix)]
    async fn stale_hello_fixture(
        label: &str,
        hello: Option<Value>,
    ) -> (
        PathBuf,
        Connection,
        Arc<CrewManager>,
        env_lock::EnvGuard<'static>,
    ) {
        let root = fixture_root(label);
        let host_key = SigningKey::from_bytes(&[5; 32]);
        let host_public = host_key.verifying_key().to_bytes();
        let connection = Connection {
            name: "Alice lab".into(),
            ssh_target: "alice@hpc.example.org".into(),
            device_id: hex(&Sha256::digest(host_public)),
            public_key: hex(&host_public),
            ..saved_connection(
                "6e6e6e6e-6e6e-46e6-86e6-6e6e6e6e6e6e",
                "",
                WORKSPACE_ID,
                &hex(&workspace_key()),
            )
        };
        let snapshot = json!({"result": {
        "workspace": {"id": WORKSPACE_ID, "host_uid": 1000, "name": "lab", "mode": "private",
            "institution_id": "ucsf", "policy_epoch": 2, "host_principal_id": "p-alice"},
        "principals": [
            {"id": "p-alice", "uid": 1000, "username": "alice", "display_name": "Alice Chen", "active": true}
        ]}});
        let mut answers = vec![("workspace.snapshot", snapshot)];
        if let Some(hello) = hello {
            answers.push(("hello", json!({ "result": hello })));
        }
        write_fake_ssh(
            &root,
            &[(
                "alice@hpc.example.org",
                &["hostname hpc.example.org", "port 22"],
            )],
            &answers,
        );
        let env = isolated_env(&root);
        let manager =
            connected_manager(&root, &connection, &host_key, &[JOIN_BY_NAME_CAPABILITY]).await;
        manager.brokers.lock().unwrap().insert(
            connection.id.clone(),
            BrokerHello {
                signature_version: 2,
                capabilities: vec![JOIN_BY_NAME_CAPABILITY.into()],
                workspace_name: Some("lab".into()),
                mode: Some(ClusterMode::Private),
                institution_id: None,
                policy_epoch: Some(1),
            },
        );
        (root, connection, manager, env)
    }

    /// T-10: an invitation minted after the host set the institution carries it. The stale
    /// connect-time `hello` (institution null) disagrees with the snapshot (`ucsf`), so the
    /// daemon asks the broker again over a fresh nonce; the invitation is built from that
    /// signed answer, the cache is replaced, and the pinned node is left alone.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stale_cached_hello_is_refreshed_before_an_invitation_is_built() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let nonce = "t10-refresh-nonce";
        *crate::crew::TEST_HELLO_NONCE.lock().unwrap() = Some(nonce.into());
        let connection_for_hello = saved_connection(
            "6e6e6e6e-6e6e-46e6-86e6-6e6e6e6e6e6e",
            "",
            WORKSPACE_ID,
            &hex(&workspace_key()),
        );
        let hello = signed_hello(&connection_for_hello, nonce, Some("ucsf"), 2);
        let (root, connection, manager, _env) =
            stale_hello_fixture("invitation-stale-hello", Some(hello)).await;

        let text = manager
            .invitation_for(&connection.id, Some("@bob"))
            .await
            .unwrap();
        let parsed = crew_invitation::parse(&text.message).unwrap();
        assert_eq!(parsed.invitation.institution_id.as_deref(), Some("ucsf"));
        assert_eq!(parsed.invitation, lab_invitation());
        assert_eq!(logged(&root, "hello").len(), 1, "asked the broker once");
        let cached = manager.broker_hello(&connection.id).unwrap();
        assert_eq!(cached.institution_id.as_deref(), Some("ucsf"));
        assert_eq!(cached.policy_epoch, Some(2));
        assert!(cached
            .capabilities
            .iter()
            .any(|capability| capability == "direct_add_v1"));
        assert_eq!(
            manager.connection(&connection.id).await.unwrap().node_id,
            connection.node_id
        );

        // The cache now agrees with the workspace, so the next invitation asks nothing more.
        manager.invitation_for(&connection.id, None).await.unwrap();
        assert_eq!(logged(&root, "hello").len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    /// T-10: when the fresh `hello` can't be had or doesn't verify, no invitation is built,
    /// neither from the stale answer nor from the unsigned snapshot.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_invitation_is_refused_when_a_stale_hello_cannot_be_refreshed() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let nonce = "t10-refused-nonce";
        *crate::crew::TEST_HELLO_NONCE.lock().unwrap() = Some(nonce.into());
        // The broker's answer is signed over another nonce: a replay, which never verifies.
        let connection_for_hello = saved_connection(
            "6e6e6e6e-6e6e-46e6-86e6-6e6e6e6e6e6e",
            "",
            WORKSPACE_ID,
            &hex(&workspace_key()),
        );
        let replayed = signed_hello(&connection_for_hello, "an-earlier-nonce", Some("ucsf"), 2);
        let (root, connection, manager, _env) =
            stale_hello_fixture("invitation-stale-refused", Some(replayed)).await;

        let error = manager
            .invitation_for(&connection.id, Some("@bob"))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "Reconnect to lab, then invite again.");
        assert_eq!(logged(&root, "hello").len(), 1);
        let cached = manager.broker_hello(&connection.id).unwrap();
        assert_eq!(
            cached.institution_id, None,
            "a refused answer is never cached"
        );
        assert_eq!(cached.policy_epoch, Some(1));
        let _ = fs::remove_dir_all(root);
    }

    /// D-ALIAS: the preview names the server by the joiner's own SSH alias for the address the
    /// invitation carries, while the login it would save keeps that address.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_preview_names_the_server_by_the_joiners_own_alias() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("preview-server-label");
        write_fake_ssh(
            &root,
            &[
                (
                    "bob@hpc.example.org",
                    &["hostname hpc.example.org", "port 22"],
                ),
                ("hpc-alias", &["hostname hpc.example.org", "port 22"]),
                ("elsewhere", &["hostname other.example.org", "port 22"]),
            ],
            &[],
        );
        let _env = isolated_env(&root);
        let ssh = root.join("profile/home/.ssh");
        fs::create_dir_all(&ssh).unwrap();
        fs::write(
            ssh.join("config"),
            "Host elsewhere\n  HostName other.example.org\nHost hpc-alias\n  HostName hpc.example.org\n",
        )
        .unwrap();
        let manager = CrewManager::new(root.clone()).unwrap();
        let text = crew_invitation::message(&lab_invitation()).unwrap();
        let preview = preview_of(
            manager
                .connection_from_invitation(&text, true, InvitationOverrides::default())
                .await
                .unwrap(),
        );
        assert_eq!(preview.ssh_target.as_deref(), Some("bob@hpc.example.org"));
        assert_eq!(preview.server.as_deref(), Some("hpc.example.org"));
        assert_eq!(preview.server_label.as_deref(), Some("hpc-alias"));
        let json = serde_json::to_value(&preview).unwrap();
        assert_eq!(json["server_label"], "hpc-alias");
        let _ = fs::remove_dir_all(root);
    }

    /// T-52: a preview warns, in people's words, when this computer already uses the same
    /// server (as SSH resolves it, aliases included) under another institution. Saving is not
    /// refused here; connecting still is.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_preview_warns_when_this_server_already_holds_another_institution() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("institution-conflict");
        write_fake_ssh(
            &root,
            &[
                (
                    "bob@hpc.example.org",
                    &["hostname hpc.example.org", "port 22"],
                ),
                ("bob@hpc-alias", &["hostname hpc.example.org", "port 22"]),
                ("bob@elsewhere", &["hostname other.example.org", "port 22"]),
            ],
            &[],
        );
        let _env = isolated_env(&root);
        let foreign = Connection {
            name: "Foreign lab".into(),
            ssh_target: "bob@hpc-alias".into(),
            institution_id: Some("foreign-synthetic".into()),
            ..saved_connection(
                "7a7a7a7a-7a7a-47a7-87a7-7a7a7a7a7a7a",
                "",
                "5b5b5b5b-5b5b-45b5-85b5-5b5b5b5b5b5b",
                &"66".repeat(32),
            )
        };
        let elsewhere = Connection {
            id: "7b7b7b7b-7b7b-47b7-87b7-7b7b7b7b7b7b".into(),
            name: "Other server".into(),
            ssh_target: "bob@elsewhere".into(),
            workspace_id: "5c5c5c5c-5c5c-45c5-85c5-5c5c5c5c5c5c".into(),
            institution_id: Some("stanford".into()),
            ..foreign.clone()
        };
        let preview_with = |connections: Vec<Connection>, overrides: InvitationOverrides| {
            let root = root.clone();
            async move {
                let registry = Registry {
                    connections,
                    ..Default::default()
                };
                fs::write(
                    root.join("connections.json"),
                    serde_json::to_vec(&registry).unwrap(),
                )
                .unwrap();
                let manager = CrewManager::new(root.clone()).unwrap();
                let text = crew_invitation::message(&lab_invitation()).unwrap();
                preview_of(
                    manager
                        .connection_from_invitation(&text, true, overrides)
                        .await
                        .unwrap(),
                )
            }
        };

        let preview = preview_with(
            vec![foreign.clone(), elsewhere.clone()],
            InvitationOverrides::default(),
        )
        .await;
        assert_eq!(
            preview.institution_conflict.as_deref(),
            Some("You already use this server for Foreign lab (foreign-synthetic). lab uses ucsf; one computer can't mix institutions on the same server.")
        );
        let json = serde_json::to_value(&preview).unwrap();
        assert_eq!(
            json["institution_conflict"],
            json!(preview.institution_conflict)
        );

        // Another server, or the same institution, is no conflict; the field is then absent.
        let preview = preview_with(vec![elsewhere], InvitationOverrides::default()).await;
        assert_eq!(preview.institution_conflict, None);
        assert!(serde_json::to_value(&preview)
            .unwrap()
            .get("institution_conflict")
            .is_none());
        let same = InvitationOverrides {
            institution_id: Some("foreign-synthetic".into()),
            ..Default::default()
        };
        let preview = preview_with(vec![foreign], same).await;
        assert_eq!(preview.institution_conflict, None);
        let _ = fs::remove_dir_all(root);
    }
}
