//! Human-only daemon PTYs. HTTP adapters must prove human authority on every operation.
use super::{manager, AuthenticationPlan};
use anyhow::{ensure, Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

pub const ATTACH_FAILURE_CODE: &str = "authentication_attach_failed";
pub const HANDOFF_FAILURE_CODE: &str = "authentication_handoff_failed";
pub const ATTACH_FAILURE_MESSAGE: &str = "SSH authentication terminal could not be attached. Close any existing authentication session, verify the daemon's SSH configuration, and try again.";
pub const HANDOFF_FAILURE_MESSAGE: &str = "SSH authentication could not be handed off to a verified Crew broker. Verify ~/.local/bin/biorouter-crew is installed on the target host and check the saved broker socket/workspace identity, then reconnect.";

pub fn terminal_failure_message(code: &str) -> Option<&'static str> {
    match code {
        ATTACH_FAILURE_CODE => Some(ATTACH_FAILURE_MESSAGE),
        HANDOFF_FAILURE_CODE => Some(HANDOFF_FAILURE_MESSAGE),
        _ => None,
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
pub async fn handoff(id: &str, controller: &str) -> Result<bool> {
    let connection = session(id, controller)?.info.connection_id.clone();
    let manager = manager()?;
    let _lifecycle = manager.connection_guard(&connection).await?;
    // A replaced or cancelled session must not clean up a later connection.
    session(id, controller)?;
    let result = handoff_locked(id, controller).await;
    if result.is_err() {
        let _ = manager.disconnect_locked(&connection).await;
        let mut registry = manager.registry.lock().await;
        if let Some(entry) = registry.connections.iter_mut().find(|c| c.id == connection) {
            entry.last_error = Some(HANDOFF_FAILURE_MESSAGE.into());
        }
    }
    result
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
