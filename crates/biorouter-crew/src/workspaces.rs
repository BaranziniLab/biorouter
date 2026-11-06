//! `biorouter-crew workspaces [--output-format json]` (account ownership, AO-6): what is on this
//! server for the account that runs it. A server command setup runs over SSH as the account. It
//! changes nothing, never fails because of one workspace, and finishes within 20 seconds.
//!
//! # The answer
//!
//! One line of JSON, at most 48 KiB:
//!
//! ```json
//! {"account": {"uid": 1001, "username": "wanjun"},
//!  "workspaces": [ENTRY, ...],
//!  "skipped": 0, "unsupported": 0, "truncated": false}
//! ```
//!
//! `username` is `null` when the account cannot be resolved. `skipped` and `unsupported` are
//! counts, `truncated` is `true` or `false`:
//!
//! - `skipped`: workspaces of other accounts that were not described: a candidate left untried
//!   by the per-account or total cap, one that did not answer within its deadline, and one whose
//!   answer did not check out. A running workspace of this account that did not answer counts
//!   too.
//! - `unsupported`: brokers of other accounts that answered but do not advertise
//!   `account_identity_v1`, so they cannot say whether this account belongs to them. They are
//!   never listed.
//! - `truncated`: whether workspaces were left out to keep the answer within its bounds: hosted
//!   folders past the 64th, runtime directories of this account past the 32nd, or entries past
//!   48 KiB.
//!
//! Each ENTRY is:
//!
//! ```json
//! {"workspace_id": "…", "name": "baranzini-lab", "role": "host",
//!  "host_uid": 1001, "host_username": "wanjun",
//!  "socket_path": "/tmp/crew-1001-…/broker.sock",
//!  "workspace_key_fingerprint": "…", "mode": "private", "institution_id": "ucsf",
//!  "state": "running", "version": "1.94.1", "binary_current": true,
//!  "state_dir": "/home/wanjun/.local/share/biorouter-crew/baranzini-lab",
//!  "found_by": "state_dir", "invitation": "brcrew1:…", "others_connected": 0}
//! ```
//!
//! - `role`: `host` for a workspace this account hosts; `member`, `invited` or `removed` for
//!   another account's workspace, as its broker's `account.status` answers.
//! - `state`: `running`, `stopped`, `deleted`, `storage_failed` or `unknown`. An entry that could
//!   not be read is `unknown` with an `error` (a coded sentence), and its other fields are
//!   `null` where they could not be learned.
//! - `version`: the version a running broker reports (`hello`'s, else its `runtime.json`'s);
//!   `null` for a stopped workspace or a broker too old to say.
//! - `binary_current`: for a running workspace this account hosts, whether the broker's process
//!   (the PID the kernel reports for the `hello` connection, never the PID `runtime.json`
//!   records) runs the same file (device and inode) as `~/.local/bin/biorouter-crew`; `null`
//!   otherwise.
//! - `state_dir`: the hosted workspace's state directory, for display only (setup addresses a
//!   workspace by its ID); `null` for a workspace started with a custom `--state-dir`, found
//!   only by its running broker, and for another account's.
//! - `found_by`: `state_dir` (a journal under `~/.local/share/biorouter-crew/`), `runtime` (a
//!   running broker of this account in `/tmp` with no state directory there) or `scan` (another
//!   account's broker).
//! - `invitation`: the workspace's `brcrew1:` line without `ssh_host`, the one `start` prints,
//!   so a computer with nothing saved opens it through the ordinary join path; `null` for a
//!   deleted or unknown workspace, or when a field it needs is missing.
//! - `others_connected`: for a running workspace this account hosts whose broker advertises
//!   `account_identity_v1`, how many live connections it holds from other accounts (AO-3; setup
//!   asks it before a restart, AO-8); `null` otherwise. It is not in AO-6's list: setup has no
//!   other way to learn it over SSH.
//!
//! # Where it looks
//!
//! **Hosted by this account:** every directory (not a symbolic link) under
//! `~/.local/share/biorouter-crew/` holding a journal or a deleted-workspace marker, at most 64,
//! and this account's own runtime directories in `/tmp`, at most 32, for workspaces started
//! with a custom `--state-dir`. A running workspace is described by `runtime.json` and a
//! `hello` checked as `status` checks it (socket and listener this account's, the recorded
//! workspace and key), without replaying the journal; a stopped one by its replayed journal.
//!
//! **Elsewhere:** runtime sockets `/tmp/crew-<uid>-<32 hex>/broker.sock` of other accounts.
//! Each candidate is checked before it is probed, as the bridge checks a workspace that moved:
//! the directory (not a symbolic link) and the socket belong to the UID its name carries and
//! nobody else can write into the directory, and the listener runs as that UID. At most 4 per
//! account, taken round-robin across accounts, and 64 in all, are probed concurrently, each
//! with a 1-second deadline and a 16 KiB cap on what it reads, within 5 seconds for the whole
//! scan. `hello` must be signed (v1 and v2) by the key it presents over a fresh nonce, and its
//! signed `host_uid` must be the socket's owner; `host_username` comes from resolving that UID,
//! never from the answer. Only `member`, `invited` and `removed` answers are listed.
//!
//! A workspace found by the scan is trusted on first use, bounded by what the kernel vouches
//! for: a socket in a directory only its host account controls, answered by a process running
//! as that account.
use super::by_id::{installed_program, peer_credentials, store_of};
use super::*;
use std::sync::mpsc;

/// Hosted state directories described, at most.
const HOSTED_LIMIT: usize = 64;
/// This account's own runtime directories probed, at most.
const OWN_RUNTIME_LIMIT: usize = 32;
/// Another account's runtime sockets probed, at most.
const SCAN_PER_UID: usize = 4;
/// Other accounts' runtime sockets probed, at most, in all.
const SCAN_LIMIT: usize = 64;
/// One probe of another account's broker, from connecting to the last byte read.
const SCAN_PROBE: Duration = Duration::from_secs(1);
/// The whole scan of other accounts.
const SCAN_DEADLINE: Duration = Duration::from_secs(5);
/// One probe of this account's own broker, as `status` gives one.
const OWN_PROBE: Duration = Duration::from_secs(3);
/// How long probes that have not answered are waited for past their deadline.
const COLLECT_GRACE: Duration = Duration::from_millis(250);
/// No journal replay starts later than this into the command.
const REPLAY_BUDGET: Duration = Duration::from_secs(15);
/// The most bytes read for one answer of a probed broker.
const FRAME_CAP: usize = 16 * 1024;
/// The whole answer stays within this.
const ANSWER_BYTES: usize = 48 * 1024;
/// What a broker that can say whether an account belongs to it advertises (AO-4).
const ACCOUNT_CAPABILITY: &str = "account_identity_v1";
/// The longest journal a stopped workspace is replayed from, as the broker opens one.
const JOURNAL_BYTES: u64 = 1024 * 1024 * 1024;

/// What one listing looks at and as whom.
struct Scope<'a> {
    home: &'a Path,
    /// `/tmp`, where runtime directories live.
    runtime_root: &'a Path,
    /// The account the listing is for: hosted runtime directories are this UID's, and the scan
    /// looks at every other UID's.
    account_uid: u32,
    directory: &'a dyn Directory,
}

impl Broker {
    /// `biorouter-crew workspaces` for the account whose home is `home`: the JSON answer the
    /// module documentation describes. Never fails; what cannot be read is said in it.
    pub fn list_workspaces(home: &Path) -> Value {
        report(&Scope {
            home,
            runtime_root: Path::new("/tmp"),
            account_uid: unsafe { libc::geteuid() },
            directory: &SystemDirectory,
        })
    }

    /// [`Broker::list_workspaces`] with another runtime root, another account UID (so the test
    /// account's own runtime directories can stand in for another account's) and an injected
    /// account directory. Test builds only.
    #[cfg(feature = "test-seams")]
    pub fn list_workspaces_with(
        home: &Path,
        runtime_root: &Path,
        account_uid: u32,
        directory: &dyn Directory,
    ) -> Value {
        report(&Scope {
            home,
            runtime_root,
            account_uid,
            directory,
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Probing a broker
// ---------------------------------------------------------------------------------------------

/// What one probe found.
enum Outcome {
    /// The broker answered `hello` (and `account.status` when asked and advertised).
    Answered {
        hello: Value,
        /// The listener's process, as the kernel reports it for this connection.
        pid: Option<i32>,
        /// `account.status`'s answer, or the refusal it gave.
        status: Option<std::result::Result<Value, String>>,
    },
    /// Nothing listens there: the socket is missing or refuses connections.
    Gone,
    /// Anything else: a refusal, a timeout, an answer that is not a broker's.
    Failed(String),
}

/// The time left before `deadline`, or `timed_out`.
fn left(deadline: Instant) -> Result<Duration> {
    let left = deadline.saturating_duration_since(Instant::now());
    ensure!(
        !left.is_zero(),
        "timed_out: the broker did not answer in time"
    );
    Ok(left)
}

/// One newline-terminated frame from `stream`, read before `deadline` and at most
/// [`FRAME_CAP`] bytes. A peer that sends a byte at a time cannot hold the read past the
/// deadline: every read is bounded by the time left.
fn read_line_within(stream: &mut UnixStream, deadline: Instant) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        stream.set_read_timeout(Some(left(deadline)?))?;
        let count = match stream.read(&mut chunk) {
            Ok(count) => count,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                bail!("timed_out: the broker did not answer in time")
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        ensure!(count > 0, "unavailable: the broker closed the connection");
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(end) = bytes.iter().position(|byte| *byte == b'\n') {
            ensure!(end < FRAME_CAP, "invalid_frame: the answer is too large");
            bytes.truncate(end + 1);
            return Ok(bytes);
        }
        ensure!(
            bytes.len() < FRAME_CAP,
            "invalid_frame: the answer is too large"
        );
    }
}

/// Send `request` and read its answer, both before `deadline`.
fn ask(stream: &mut UnixStream, deadline: Instant, request: &Value) -> Result<Response> {
    let mut bytes = serde_json::to_vec(request)?;
    bytes.push(b'\n');
    stream.set_write_timeout(Some(left(deadline)?))?;
    stream.write_all(&bytes)?;
    let line = read_line_within(stream, deadline)?;
    serde_json::from_slice(&line).context("invalid_response: the answer is not a broker's")
}

/// Whether `hello` advertises `capability`.
fn advertises(hello: &Value, capability: &str) -> bool {
    hello
        .get("capabilities")
        .and_then(Value::as_array)
        .is_some_and(|list| list.iter().any(|item| item.as_str() == Some(capability)))
}

/// `hello` (with the challenge `nonce`), then `account.status` when `ask_status` and the broker
/// advertises it, on one connection to `socket`, a socket of `owner`'s, all before `deadline`.
fn probe(socket: &Path, owner: u32, deadline: Instant, nonce: &str, ask_status: bool) -> Outcome {
    let gone = |error: &std::io::Error| {
        matches!(
            error.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
        )
    };
    if let Err(error) = check_runtime_socket(socket, owner) {
        return match error.downcast_ref::<std::io::Error>() {
            Some(io) if gone(io) => Outcome::Gone,
            _ => Outcome::Failed(format!("{error:#}")),
        };
    }
    let stream = match UnixStream::connect(socket) {
        Ok(stream) => stream,
        Err(error) if gone(&error) => return Outcome::Gone,
        Err(error) => return Outcome::Failed(error.to_string()),
    };
    match exchange(stream, owner, deadline, nonce, ask_status) {
        Ok(outcome) => outcome,
        Err(error) => Outcome::Failed(format!("{error:#}")),
    }
}

fn exchange(
    mut stream: UnixStream,
    owner: u32,
    deadline: Instant,
    nonce: &str,
    ask_status: bool,
) -> Result<Outcome> {
    let (uid, pid) = peer_credentials(&stream)?;
    ensure!(
        uid == owner,
        "identity_mismatch: the listener runs as another account"
    );
    let hello = ask(
        &mut stream,
        deadline,
        &json!({"version": 1, "id": "workspaces-hello", "method": "hello",
                "params": {"challenge_nonce": nonce}}),
    )?;
    if let Some(error) = hello.error {
        bail!("{}: {}", error.code, error.message);
    }
    let hello = hello
        .result
        .ok_or_else(|| anyhow!("invalid_response: hello answered nothing"))?;
    let status = if ask_status && advertises(&hello, ACCOUNT_CAPABILITY) {
        let answer = ask(
            &mut stream,
            deadline,
            &json!({"version": 1, "id": "workspaces-status", "method": "account.status",
                    "params": {}}),
        )?;
        Some(match (answer.result, answer.error) {
            (Some(result), None) => Ok(result),
            (_, Some(error)) => Err(format!("{}: {}", error.code, error.message)),
            (None, None) => Err("invalid_response: account.status answered nothing".to_owned()),
        })
    } else {
        None
    };
    Ok(Outcome::Answered {
        hello,
        pid: Some(pid),
        status,
    })
}

/// Probes running on their own threads, answered over one channel.
struct Probes {
    receiver: mpsc::Receiver<(usize, Outcome)>,
    count: usize,
    /// Every probe's own deadline: none answers later.
    deadline: Instant,
}

/// Start one probe per `(socket, owner)` target, all at once, each within `per_probe`, with the
/// matching challenge nonce. A probe whose thread cannot start fails at once.
fn spawn_probes(
    targets: &[(PathBuf, u32)],
    nonces: &[String],
    per_probe: Duration,
    ask_status: bool,
) -> Probes {
    let (sender, receiver) = mpsc::channel();
    let deadline = Instant::now() + per_probe;
    for (index, ((socket, owner), nonce)) in targets.iter().zip(nonces).enumerate() {
        let (socket, owner, nonce) = (socket.clone(), *owner, nonce.clone());
        let thread_sender = sender.clone();
        let spawned = std::thread::Builder::new()
            .name("crew-workspaces-probe".into())
            .spawn(move || {
                let outcome = probe(&socket, owner, deadline, &nonce, ask_status);
                let _ = thread_sender.send((index, outcome));
            });
        if let Err(error) = spawned {
            let _ = sender.send((index, Outcome::Failed(error.to_string())));
        }
    }
    Probes {
        receiver,
        count: targets.len(),
        deadline,
    }
}

/// The probes' outcomes, waiting at most until their deadline plus [`COLLECT_GRACE`] and never
/// past `until`. A probe that has not answered by then is `None`; its thread is left to end on
/// its own (every read and write it makes is bounded by its deadline).
fn collect(probes: Probes, until: Instant) -> Vec<Option<Outcome>> {
    let until = until.min(probes.deadline + COLLECT_GRACE);
    let mut outcomes: Vec<Option<Outcome>> = (0..probes.count).map(|_| None).collect();
    let mut answered = 0;
    while answered < probes.count {
        let wait = until.saturating_duration_since(Instant::now());
        match probes.receiver.recv_timeout(wait) {
            Ok((index, outcome)) => {
                if outcomes[index].is_none() {
                    answered += 1;
                }
                outcomes[index] = Some(outcome);
            }
            Err(_) => break,
        }
    }
    outcomes
}

/// Whether another account's `hello` checks out for the socket `owner` owns: it is signed, v1
/// and v2, over `nonce` by the workspace key it presents, and its signed `host_uid` is `owner`.
/// The v2 signature is what makes its capabilities, name, mode and institution trustworthy.
fn foreign_hello_checks_out(hello: &Value, owner: u32, nonce: &str) -> bool {
    let check = || -> Result<()> {
        let workspace_id = text(hello, "workspace_id")?;
        ensure!(broker_id(workspace_id), "invalid workspace ID");
        let host_uid = u32::try_from(number(hello, "host_uid")?)?;
        ensure!(host_uid == owner, "host_uid is not the socket's owner");
        let public_key = text(hello, "workspace_public_key")?;
        let key_bytes: [u8; 32] = hex::decode(public_key)?
            .try_into()
            .map_err(|_| anyhow!("invalid key"))?;
        let key = VerifyingKey::from_bytes(&key_bytes)?;
        let node_id = text(hello, "node_id")?;
        let signature = |field: &str| -> Result<Signature> {
            Ok(Signature::from_slice(&hex::decode(text(hello, field)?)?)?)
        };
        key.verify_strict(
            &hello_v1_payload(workspace_id, host_uid, nonce, public_key, node_id),
            &signature("signature")?,
        )?;
        let mode = mode(hello, "mode")?;
        let capabilities: Vec<&str> = hello
            .get("capabilities")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("capabilities missing"))?
            .iter()
            .map(|item| item.as_str().ok_or_else(|| anyhow!("invalid capability")))
            .collect::<Result<_>>()?;
        let payload = HelloV2 {
            workspace_id,
            host_uid,
            challenge_nonce: nonce,
            workspace_public_key: public_key,
            node_id,
            mode: &mode,
            institution_id: hello.get("institution_id").and_then(Value::as_str),
            policy_epoch: number(hello, "policy_epoch")?,
            name: hello.get("name").and_then(Value::as_str),
            capabilities: &capabilities,
        }
        .signing_payload();
        key.verify_strict(&payload, &signature("signature_v2")?)?;
        Ok(())
    };
    check().is_ok()
}

/// Whether the process `pid` runs the same file (device and inode) as `installed`; `None` when
/// the process's executable cannot be read (no `/proc`, or not this account's process).
fn runs_installed(pid: i32, installed: &Path) -> Option<bool> {
    let running = fs::metadata(format!("/proc/{pid}/exe")).ok()?;
    Some(
        fs::metadata(installed)
            .is_ok_and(|file| file.dev() == running.dev() && file.ino() == running.ino()),
    )
}

// ---------------------------------------------------------------------------------------------
// Entries
// ---------------------------------------------------------------------------------------------

/// One workspace as the answer lists it (see the module documentation).
#[derive(Default)]
struct Entry {
    workspace_id: Option<String>,
    name: Option<String>,
    role: &'static str,
    host_uid: Option<u32>,
    host_username: Option<String>,
    socket_path: Option<String>,
    /// The workspace public key, hex.
    public_key: Option<String>,
    mode: Option<Mode>,
    institution_id: Option<String>,
    state: &'static str,
    version: Option<String>,
    binary_current: Option<bool>,
    state_dir: Option<String>,
    found_by: &'static str,
    others_connected: Option<u64>,
    error: Option<String>,
}

impl Entry {
    /// The workspace's `brcrew1:` line without `ssh_host`, when it can be opened and every field
    /// the line needs is known.
    fn invitation(&self) -> Option<String> {
        if !matches!(self.state, "running" | "stopped" | "storage_failed") {
            return None;
        }
        crate::invitation::encode(&crate::invitation::WorkspaceInvitation {
            workspace_id: self.workspace_id.clone()?,
            workspace_public_key: self.public_key.clone()?,
            socket_path: self.socket_path.clone()?,
            owner_uid: self.host_uid?,
            workspace_name: self.name.clone(),
            host_username: Some(self.host_username.clone()?),
            host_display_name: None,
            mode: Some(self.mode.clone()?),
            institution_id: self.institution_id.clone(),
            ssh_host: None,
            ssh_port: None,
            proxy_jump: None,
            invitee_username: None,
        })
        .ok()
    }

    fn wire(&self) -> Value {
        let fingerprint = self
            .public_key
            .as_deref()
            .and_then(crate::invitation::workspace_key_fingerprint);
        let mut wire = json!({
            "workspace_id": self.workspace_id,
            "name": self.name,
            "role": self.role,
            "host_uid": self.host_uid,
            "host_username": self.host_username,
            "socket_path": self.socket_path,
            "workspace_key_fingerprint": fingerprint,
            "mode": self.mode,
            "institution_id": self.institution_id,
            "state": self.state,
            "version": self.version,
            "binary_current": self.binary_current,
            "state_dir": self.state_dir,
            "found_by": self.found_by,
            "invitation": self.invitation(),
            "others_connected": self.others_connected,
        });
        if let Some(error) = &self.error {
            wire["error"] = json!(error);
        }
        wire
    }

    /// The entry for a broker that answered `hello`: its state, name, mode, institution, key
    /// and version as it says them.
    fn from_hello(
        hello: &Value,
        socket: &Path,
        role: &'static str,
        found_by: &'static str,
    ) -> Self {
        let state = match hello.get("state").and_then(Value::as_str) {
            Some("deleted") => "deleted",
            Some("storage_failed") => "storage_failed",
            _ => "running",
        };
        let field = |key: &str| hello.get(key).and_then(Value::as_str).map(str::to_owned);
        Entry {
            workspace_id: field("workspace_id"),
            name: field("name"),
            role,
            host_uid: hello
                .get("host_uid")
                .and_then(Value::as_u64)
                .and_then(|uid| u32::try_from(uid).ok()),
            socket_path: socket.to_str().map(str::to_owned),
            public_key: field("workspace_public_key"),
            mode: hello
                .get("mode")
                .and_then(|mode| serde_json::from_value(mode.clone()).ok()),
            institution_id: field("institution_id"),
            state,
            version: field("version"),
            found_by,
            ..Entry::default()
        }
    }
}

/// `others_connected` from the host's own `account.status` answer.
fn others_connected(status: &Option<std::result::Result<Value, String>>) -> Option<u64> {
    match status {
        Some(Ok(status)) if status.get("role").and_then(Value::as_str) == Some("host") => {
            status.get("others_connected").and_then(Value::as_u64)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Hosted workspaces
// ---------------------------------------------------------------------------------------------

/// What a hosted state directory holds, before any broker is probed.
enum Hosted {
    /// `runtime.json` names a process that is still there: probe its broker.
    Running(Value),
    /// No broker runs from it: read its journal.
    Stopped,
    /// An erase the host started and a crash interrupted.
    Deleted(Option<String>),
    /// It could not be read.
    Unknown(Option<String>, String),
}

/// The state directories under `store` holding a journal or a deleted-workspace marker, in name
/// order, at most [`HOSTED_LIMIT`], and how many more there were. Symbolic links are passed
/// over: the broker never keeps a workspace behind one.
fn hosted_dirs(store: &Path) -> (Vec<PathBuf>, usize) {
    let Ok(entries) = fs::read_dir(store) else {
        return (Vec::new(), 0);
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .filter(|dir| {
            fs::symlink_metadata(dir.join("journal.jsonl")).is_ok()
                || fs::symlink_metadata(dir.join(DELETED_MARKER)).is_ok()
        })
        .collect();
    dirs.sort();
    let over = dirs.len().saturating_sub(HOSTED_LIMIT);
    dirs.truncate(HOSTED_LIMIT);
    (dirs, over)
}

fn classify(dir: &Path) -> Hosted {
    let private = fs::symlink_metadata(dir).is_ok_and(|metadata| {
        metadata.is_dir()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
    });
    if !private {
        return Hosted::Unknown(
            None,
            "unsafe_storage: state directory must be owner-only and not a symlink".into(),
        );
    }
    match deleted_marker(dir) {
        Ok(Some(marked)) => return Hosted::Deleted(marked_workspace(dir, marked)),
        Ok(None) => {}
        Err(error) => return Hosted::Unknown(None, format!("{error:#}")),
    }
    match existing_runtime_descriptor(dir) {
        Ok(Some(info)) if !recorded_process_gone(&info) => Hosted::Running(info),
        Ok(_) => Hosted::Stopped,
        Err(error) => Hosted::Unknown(None, format!("{error:#}")),
    }
}

/// The replayed state of the stopped workspace in `dir`.
fn stopped_state(dir: &Path) -> Result<Value> {
    let journal = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(dir.join("journal.jsonl"))
        .context("unsafe_storage: cannot read the journal")?;
    let metadata = journal.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "unsafe_storage: file ownership, mode or links invalid"
    );
    ensure!(
        metadata.len() <= JOURNAL_BYTES,
        "quota_exceeded: journal exceeds supported replay size of 1 GiB"
    );
    let replay = replay_journal(&journal)?;
    ensure!(
        replay.sequence > 0,
        "not_found: This state directory holds no workspace."
    );
    Ok(replay.state_value)
}

/// The entry for the stopped workspace whose replayed state is `state`.
fn stopped_entry(state: &Value, dir: &Path, runtime_root: &Path) -> Entry {
    let workspace = &state["workspace"];
    let field = |key: &str| {
        workspace
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let public_key = state
        .get("workspace_signing_key")
        .and_then(Value::as_str)
        .and_then(|secret| hex::decode(secret).ok())
        .and_then(|secret| <[u8; 32]>::try_from(secret).ok())
        .map(|secret| hex::encode(SigningKey::from_bytes(&secret).verifying_key().to_bytes()));
    let socket_path = state
        .get("runtime_basename")
        .and_then(Value::as_str)
        .and_then(|basename| {
            runtime_root
                .join(basename)
                .join("broker.sock")
                .to_str()
                .map(str::to_owned)
        });
    Entry {
        workspace_id: field("id"),
        name: field("name"),
        role: "host",
        host_uid: workspace
            .get("host_uid")
            .and_then(Value::as_u64)
            .and_then(|uid| u32::try_from(uid).ok()),
        socket_path,
        public_key,
        mode: workspace
            .get("mode")
            .and_then(|mode| serde_json::from_value(mode.clone()).ok()),
        institution_id: field("institution_id"),
        state: if workspace.get("deleted_at").is_some_and(|at| !at.is_null()) {
            "deleted"
        } else {
            "stopped"
        },
        state_dir: dir.to_str().map(str::to_owned),
        found_by: "state_dir",
        ..Entry::default()
    }
}

/// An entry that could not be read.
fn unknown_entry(
    workspace_id: Option<String>,
    dir: Option<&Path>,
    found_by: &'static str,
    error: String,
) -> Entry {
    Entry {
        workspace_id,
        role: "host",
        state: "unknown",
        state_dir: dir.and_then(Path::to_str).map(str::to_owned),
        found_by,
        error: Some(error),
        ..Entry::default()
    }
}

/// This account's runtime sockets in `runtime_root` (`crew-<uid>-<32 hex>/broker.sock`, the
/// directory and the socket its own), in name order, at most [`OWN_RUNTIME_LIMIT`], and how
/// many more there were.
fn own_runtime_sockets(runtime_root: &Path, uid: u32) -> (Vec<PathBuf>, usize) {
    let Ok(entries) = fs::read_dir(runtime_root) else {
        return (Vec::new(), 0);
    };
    let mut sockets: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| runtime_basename_of(name, uid))
        .map(|name| runtime_root.join(name).join("broker.sock"))
        .filter(|socket| check_runtime_socket(socket, uid).is_ok())
        .collect();
    sockets.sort();
    let over = sockets.len().saturating_sub(OWN_RUNTIME_LIMIT);
    sockets.truncate(OWN_RUNTIME_LIMIT);
    (sockets, over)
}

// ---------------------------------------------------------------------------------------------
// Other accounts' workspaces
// ---------------------------------------------------------------------------------------------

/// The UID a runtime directory's name carries: `crew-<uid>-<32 hex>`, with the UID written as
/// the broker writes it.
fn runtime_owner(name: &str) -> Option<u32> {
    let uid: u32 = name
        .strip_prefix("crew-")?
        .split('-')
        .next()?
        .parse()
        .ok()?;
    runtime_basename_of(name, uid).then_some(uid)
}

/// Other accounts' runtime sockets in `runtime_root`, by UID, each checked before anything is
/// opened: the directory (not a symbolic link) and the socket belong to the UID the name
/// carries, and no other account can write into the directory. Decoys anyone can create (a
/// name with another UID, a file, a link, a directory without a socket) are never candidates
/// and take no slot.
fn foreign_candidates(runtime_root: &Path, account_uid: u32) -> BTreeMap<u32, Vec<PathBuf>> {
    let mut candidates: BTreeMap<u32, Vec<PathBuf>> = BTreeMap::new();
    let Ok(entries) = fs::read_dir(runtime_root) else {
        return candidates;
    };
    for name in entries.filter_map(|entry| entry.ok()?.file_name().into_string().ok()) {
        let Some(uid) = runtime_owner(&name) else {
            continue;
        };
        if uid == account_uid {
            continue;
        }
        let socket = runtime_root.join(&name).join("broker.sock");
        if check_runtime_socket(&socket, uid).is_ok() {
            candidates.entry(uid).or_default().push(socket);
        }
    }
    candidates
}

/// The candidates to probe: at most [`SCAN_PER_UID`] per account, in name order, taken
/// round-robin across accounts, and [`SCAN_LIMIT`] in all, with how many were left untried.
/// One account's many directories never take another account's turn.
fn select_candidates(mut candidates: BTreeMap<u32, Vec<PathBuf>>) -> (Vec<(PathBuf, u32)>, usize) {
    for sockets in candidates.values_mut() {
        sockets.sort();
    }
    let total: usize = candidates.values().map(Vec::len).sum();
    let mut selected = Vec::new();
    'rounds: for round in 0..SCAN_PER_UID {
        for (uid, sockets) in &candidates {
            if selected.len() == SCAN_LIMIT {
                break 'rounds;
            }
            if let Some(socket) = sockets.get(round) {
                selected.push((socket.clone(), *uid));
            }
        }
    }
    let untried = total - selected.len();
    (selected, untried)
}

// ---------------------------------------------------------------------------------------------
// The listing
// ---------------------------------------------------------------------------------------------

/// One listing in progress: its scope, its counts and the account names resolved so far.
struct Listing<'a> {
    scope: &'a Scope<'a>,
    started: Instant,
    installed: PathBuf,
    usernames: BTreeMap<u32, Option<String>>,
    skipped: usize,
    unsupported: usize,
    truncated: usize,
}

/// The `(socket, owner)` targets the running hosted workspaces' runtime descriptors name, and
/// which directory each belongs to.
fn running_targets(hosted: &[Hosted]) -> (Vec<(PathBuf, u32)>, Vec<usize>) {
    let mut targets = Vec::new();
    let mut of = Vec::new();
    for (index, kind) in hosted.iter().enumerate() {
        if let Hosted::Running(info) = kind {
            let socket = info
                .get("socket")
                .and_then(Value::as_str)
                .map(PathBuf::from);
            let owner = info
                .get("host_uid")
                .and_then(Value::as_u64)
                .and_then(|uid| u32::try_from(uid).ok());
            if let (Some(socket), Some(owner)) = (socket, owner) {
                targets.push((socket, owner));
                of.push(index);
            }
        }
    }
    (targets, of)
}

fn nonces(count: usize) -> Vec<String> {
    (0..count).map(|_| token()).collect()
}

impl Listing<'_> {
    fn username(&mut self, uid: u32) -> Option<String> {
        let directory = self.scope.directory;
        self.usernames
            .entry(uid)
            .or_insert_with(|| directory.by_uid(uid).ok().map(|account| account.name))
            .clone()
    }

    /// The entry for the running hosted workspace in `dir` that `info` records, from its
    /// broker's answer; `None` when nothing listens there any more, so its journal is read.
    fn running_entry(
        &mut self,
        dir: &Path,
        info: &Value,
        outcome: Option<Outcome>,
    ) -> Option<Entry> {
        let recorded = info
            .get("workspace_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let (hello, pid, status) = match outcome {
            Some(Outcome::Answered { hello, pid, status }) => (hello, pid, status),
            Some(Outcome::Gone) => return None,
            Some(Outcome::Failed(error)) => {
                return Some(unknown_entry(recorded, Some(dir), "state_dir", error))
            }
            None => {
                let error = "timed_out: the workspace's broker did not answer in time";
                return Some(unknown_entry(
                    recorded,
                    Some(dir),
                    "state_dir",
                    error.into(),
                ));
            }
        };
        let matches = hello.get("workspace_id") == info.get("workspace_id")
            && hello.get("host_uid") == info.get("host_uid")
            && info
                .get("workspace_public_key")
                .is_none_or(|key| hello.get("workspace_public_key") == Some(key));
        if !matches {
            let error = "identity_mismatch: the broker at the recorded socket answers for another workspace";
            return Some(unknown_entry(
                recorded,
                Some(dir),
                "state_dir",
                error.into(),
            ));
        }
        let socket = PathBuf::from(info["socket"].as_str().unwrap_or_default());
        let mut entry = Entry::from_hello(&hello, &socket, "host", "state_dir");
        entry.version = entry.version.or_else(|| {
            info.get("version")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        entry.host_username = entry.host_uid.and_then(|uid| self.username(uid));
        entry.binary_current = pid.and_then(|pid| runs_installed(pid, &self.installed));
        entry.state_dir = dir.to_str().map(str::to_owned);
        entry.others_connected = others_connected(&status);
        Some(entry)
    }

    /// The entries for the hosted state directories `dirs`, which `hosted` classified; the
    /// running ones from their brokers' answers (`probed`, by directory), the stopped ones
    /// from their journals.
    fn hosted_entries(
        &mut self,
        dirs: &[PathBuf],
        hosted: &[Hosted],
        mut probed: BTreeMap<usize, Option<Outcome>>,
    ) -> Vec<Entry> {
        let mut entries = Vec::new();
        let mut to_replay = Vec::new();
        for (index, (dir, kind)) in dirs.iter().zip(hosted).enumerate() {
            match kind {
                Hosted::Deleted(id) => entries.push(Entry {
                    workspace_id: id.clone(),
                    role: "host",
                    state: "deleted",
                    state_dir: dir.to_str().map(str::to_owned),
                    found_by: "state_dir",
                    ..Entry::default()
                }),
                Hosted::Unknown(id, error) => entries.push(unknown_entry(
                    id.clone(),
                    Some(dir),
                    "state_dir",
                    error.clone(),
                )),
                Hosted::Stopped => to_replay.push(dir),
                Hosted::Running(info) => match probed.remove(&index) {
                    None => {
                        let id = info.get("workspace_id").and_then(Value::as_str);
                        let error = "unsafe_runtime: runtime.json names no socket or host";
                        let id = id.map(str::to_owned);
                        entries.push(unknown_entry(id, Some(dir), "state_dir", error.into()));
                    }
                    Some(outcome) => match self.running_entry(dir, info, outcome) {
                        Some(entry) => entries.push(entry),
                        None => to_replay.push(dir),
                    },
                },
            }
        }
        to_replay.sort();
        for dir in to_replay {
            entries.push(self.stopped(dir));
        }
        entries
    }

    /// The entry for the stopped workspace in `dir`, read from its journal while there is time.
    fn stopped(&mut self, dir: &Path) -> Entry {
        if self.started.elapsed() >= REPLAY_BUDGET {
            let error = "timed_out: there was no time left to read this workspace's journal";
            return unknown_entry(None, Some(dir), "state_dir", error.into());
        }
        match stopped_state(dir) {
            Ok(state) => {
                let mut entry = stopped_entry(&state, dir, self.scope.runtime_root);
                entry.host_username = entry.host_uid.and_then(|uid| self.username(uid));
                entry
            }
            Err(error) => unknown_entry(None, Some(dir), "state_dir", format!("{error:#}")),
        }
    }

    /// Running brokers of this account found by their runtime directories: workspaces started
    /// with a custom `--state-dir`, added to `entries`. One already listed is not listed twice;
    /// a stopped listing of a workspace whose broker turns out to run (its runtime.json went
    /// missing) says it runs.
    fn runtime_entries(
        &mut self,
        entries: &mut Vec<Entry>,
        sockets: &[PathBuf],
        outcomes: Vec<Option<Outcome>>,
    ) {
        let account_uid = u64::from(self.scope.account_uid);
        for (socket, outcome) in sockets.iter().zip(outcomes) {
            let (hello, pid, status) = match outcome {
                Some(Outcome::Answered { hello, pid, status }) => (hello, pid, status),
                Some(Outcome::Gone) => continue,
                Some(Outcome::Failed(_)) | None => {
                    self.skipped += 1;
                    continue;
                }
            };
            let host_uid = hello.get("host_uid").and_then(Value::as_u64);
            if host_uid != Some(account_uid) {
                self.skipped += 1;
                continue;
            }
            let mut entry = Entry::from_hello(&hello, socket, "host", "runtime");
            entry.host_username = entry.host_uid.and_then(|uid| self.username(uid));
            entry.binary_current = pid.and_then(|pid| runs_installed(pid, &self.installed));
            entry.others_connected = others_connected(&status);
            let listed = entries.iter_mut().find(|listed| {
                listed.workspace_id.is_some() && listed.workspace_id == entry.workspace_id
            });
            match listed {
                Some(listed) if listed.state == "stopped" => {
                    entry.state_dir = listed.state_dir.take();
                    entry.found_by = "state_dir";
                    *listed = entry;
                }
                Some(_) => {}
                None => entries.push(entry),
            }
        }
    }

    /// Other accounts' workspaces this account belongs to, was invited to or was removed from,
    /// from the probes of `targets` (challenged with `nonces`).
    fn scanned_entries(
        &mut self,
        targets: &[(PathBuf, u32)],
        nonces: &[String],
        outcomes: Vec<Option<Outcome>>,
    ) -> Vec<Entry> {
        let mut found = Vec::new();
        for (((socket, owner), nonce), outcome) in targets.iter().zip(nonces).zip(outcomes) {
            let (hello, status) = match outcome {
                Some(Outcome::Answered { hello, status, .. }) => (hello, status),
                Some(Outcome::Gone) => continue,
                Some(Outcome::Failed(_)) | None => {
                    self.skipped += 1;
                    continue;
                }
            };
            if !foreign_hello_checks_out(&hello, *owner, nonce) {
                self.skipped += 1;
                continue;
            }
            if !advertises(&hello, ACCOUNT_CAPABILITY) {
                self.unsupported += 1;
                continue;
            }
            if hello.get("state").and_then(Value::as_str) == Some("deleted") {
                continue;
            }
            // A refusal (a workspace counting down to its erase) is neither listed nor counted.
            let Some(Ok(status)) = status else {
                continue;
            };
            let role = match status.get("role").and_then(Value::as_str) {
                Some("member") => "member",
                Some("invited") => "invited",
                Some("removed") => "removed",
                _ => continue,
            };
            let mut entry = Entry::from_hello(&hello, socket, role, "scan");
            entry.host_uid = Some(*owner);
            entry.host_username = self.username(*owner);
            found.push(entry);
        }
        found.sort_by(|a, b| {
            (&a.host_username, &a.name, &a.workspace_id).cmp(&(
                &b.host_username,
                &b.name,
                &b.workspace_id,
            ))
        });
        found
    }

    /// The answer: the account, the entries in order while they fit in [`ANSWER_BYTES`], the
    /// counts, and whether anything was left out (`truncated` counts what was).
    fn answer(mut self, entries: Vec<Entry>) -> Value {
        let account_uid = self.scope.account_uid;
        let account = json!({"uid": account_uid, "username": self.username(account_uid)});
        // Room for the envelope and the counts, then entries in order while they fit.
        let mut used = json_len(&account) + 128;
        let mut listed = Vec::new();
        let mut entries = entries.into_iter();
        for entry in entries.by_ref() {
            let wire = entry.wire();
            let length = json_len(&wire) + 1;
            if used + length > ANSWER_BYTES {
                self.truncated += 1;
                break;
            }
            used += length;
            listed.push(wire);
        }
        self.truncated += entries.count();
        json!({
            "account": account,
            "workspaces": listed,
            "skipped": self.skipped,
            "unsupported": self.unsupported,
            "truncated": self.truncated > 0,
        })
    }
}

fn report(scope: &Scope) -> Value {
    let mut listing = Listing {
        scope,
        started: Instant::now(),
        installed: installed_program(scope.home),
        usernames: BTreeMap::new(),
        skipped: 0,
        unsupported: 0,
        truncated: 0,
    };
    let account_uid = scope.account_uid;

    // Other accounts' brokers are probed first, so their deadline runs while this account's
    // own workspaces are read.
    let (others, untried) = select_candidates(foreign_candidates(scope.runtime_root, account_uid));
    listing.skipped += untried;
    let other_nonces = nonces(others.len());
    let other_probes = spawn_probes(&others, &other_nonces, SCAN_PROBE, true);

    // This account's state directories, and its runtime directories for custom state dirs.
    let (dirs, over) = hosted_dirs(&store_of(scope.home));
    listing.truncated += over;
    let hosted: Vec<Hosted> = dirs.iter().map(|dir| classify(dir)).collect();
    let (running, running_of) = running_targets(&hosted);
    let (own_sockets, own_over) = own_runtime_sockets(scope.runtime_root, account_uid);
    listing.truncated += own_over;
    let own: Vec<(PathBuf, u32)> = own_sockets
        .iter()
        .map(|socket| (socket.clone(), account_uid))
        .collect();
    let running_probes = spawn_probes(&running, &nonces(running.len()), OWN_PROBE, true);
    let own_probes = spawn_probes(&own, &nonces(own.len()), OWN_PROBE, true);
    let own_until = listing.started + OWN_PROBE + COLLECT_GRACE;

    let probed = running_of
        .into_iter()
        .zip(collect(running_probes, own_until))
        .collect();
    let mut entries = listing.hosted_entries(&dirs, &hosted, probed);
    listing.runtime_entries(&mut entries, &own_sockets, collect(own_probes, own_until));
    entries.sort_by(|a, b| {
        (&a.name, &a.workspace_id, &a.state_dir).cmp(&(&b.name, &b.workspace_id, &b.state_dir))
    });
    let scanned = collect(other_probes, listing.started + SCAN_DEADLINE);
    entries.extend(listing.scanned_entries(&others, &other_nonces, scanned));
    listing.answer(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sockets(uid: u32, count: usize) -> Vec<PathBuf> {
        (0..count)
            .map(|index| PathBuf::from(format!("/tmp/crew-{uid}-{index:032x}/broker.sock")))
            .collect()
    }

    /// Four per account, round-robin, sixty-four in all: an account with many directories
    /// never takes another account's turn, and what is left untried is counted.
    #[test]
    fn candidates_are_taken_round_robin_within_both_caps() {
        let mut candidates = BTreeMap::new();
        candidates.insert(2000, sockets(2000, 100));
        candidates.insert(2001, sockets(2001, 1));
        candidates.insert(2002, sockets(2002, 2));
        let (selected, untried) = select_candidates(candidates);
        let per = |uid: u32| selected.iter().filter(|(_, owner)| *owner == uid).count();
        assert_eq!((per(2000), per(2001), per(2002)), (4, 1, 2));
        assert_eq!(untried, 96);
        // The first round reaches every account before anyone's second socket.
        let first: Vec<u32> = selected.iter().take(3).map(|(_, uid)| *uid).collect();
        assert_eq!(first, vec![2000, 2001, 2002]);

        // Twenty accounts with five sockets each: 80 within the per-account cap, 64 probed.
        let candidates: BTreeMap<u32, Vec<PathBuf>> = (0..20)
            .map(|uid| (3000 + uid, sockets(3000 + uid, 5)))
            .collect();
        let (selected, untried) = select_candidates(candidates);
        assert_eq!(selected.len(), SCAN_LIMIT);
        assert_eq!(untried, 100 - SCAN_LIMIT);
        // Every account got its first three turns before anyone's fourth.
        for uid in 3000..3020 {
            assert!(
                selected.iter().filter(|(_, owner)| *owner == uid).count() >= 3,
                "{uid}"
            );
        }
    }

    #[test]
    fn a_runtime_name_carries_its_owner_as_the_broker_writes_it() {
        let hex = "0".repeat(32);
        assert_eq!(runtime_owner(&format!("crew-1001-{hex}")), Some(1001));
        assert_eq!(runtime_owner(&format!("crew-01001-{hex}")), None);
        assert_eq!(
            runtime_owner(&format!("crew-1001-{}", "A".repeat(32))),
            None
        );
        assert_eq!(runtime_owner("crew-1001-short"), None);
        assert_eq!(runtime_owner(&format!("crt-1001-{hex}")), None);
    }

    /// An entry that cannot be opened carries no invitation, and one with every field does.
    #[test]
    fn an_invitation_needs_every_field_and_an_open_workspace() {
        let key = SigningKey::from_bytes(&[3; 32]);
        let mut entry = Entry {
            workspace_id: Some(Uuid::new_v4().to_string()),
            name: Some("lab".into()),
            role: "host",
            host_uid: Some(1001),
            host_username: Some("wanjun".into()),
            socket_path: Some(format!("/tmp/crew-1001-{}/broker.sock", "a".repeat(32))),
            public_key: Some(hex::encode(key.verifying_key().to_bytes())),
            mode: Some(Mode::Private),
            state: "stopped",
            found_by: "state_dir",
            ..Entry::default()
        };
        let line = entry
            .invitation()
            .expect("a stopped workspace can be opened");
        let parsed = crate::invitation::parse(&line).unwrap().invitation;
        assert_eq!(parsed.host_username.as_deref(), Some("wanjun"));
        assert_eq!(parsed.ssh_host, None);
        assert!(entry.wire()["workspace_key_fingerprint"].is_string());
        entry.state = "deleted";
        assert!(entry.invitation().is_none());
        entry.state = "running";
        entry.host_username = None;
        assert!(entry.invitation().is_none());
    }
}
