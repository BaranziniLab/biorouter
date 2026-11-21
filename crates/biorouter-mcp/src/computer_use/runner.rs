//! Run one engine command and read its one JSON envelope (AD-07 and "Spawn hardening" in
//! docs/design/copilot-agent-desktop.md).
//!
//! Every command line is built by `plan.rs` from typed arguments; this module only runs what
//! it is given:
//!
//! - the environment is cleared and refilled from an allowlist, so model credentials, daemon
//!   secrets and any engine variable of the daemon never reach the child;
//! - the engine's state root and temporary folder are the task's own (AD-09);
//! - typed text goes on standard input, never on the command line;
//! - standard output is read up to [`MAX_OUTPUT_BYTES`]; standard error is discarded, because
//!   the engine logs there and its logs can quote app text;
//! - the child is its own process group (Unix) or Job Object (Windows), ends within its deadline
//!   plus [`OUTER_GRACE`], and on Stop gets `SIGTERM` and [`RELEASE_GRACE`] to let its release
//!   guard post the button and key releases of an interrupted gesture before `SIGKILL` (AD-15);
//! - on macOS it is started with responsibility disclaimed (AD-04), so Accessibility and Screen
//!   Recording belong to the Biorouter Copilot bundle and never to Biorouter itself. A spawn that
//!   cannot disclaim fails; it never falls back to an ordinary spawn, which would silently
//!   borrow the launcher's permissions.
use serde_json::Value;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// The largest envelope read. A larger one is an error, never a truncation.
pub const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
/// Added to every command's own deadline before the wrapper ends it.
pub const OUTER_GRACE: Duration = Duration::from_secs(5);
/// Between `SIGTERM` and `SIGKILL` on Stop.
// Windows has no graceful signal to give a grace to: there the wrapper lifts held buttons itself.
#[cfg_attr(windows, allow(dead_code))]
pub const RELEASE_GRACE: Duration = Duration::from_secs(1);

#[cfg(all(test, unix))]
thread_local! {
    /// A longer grace for a test, on its own thread, that must see a stand-in engine's release
    /// guard run: on a loaded machine a shell can take longer than [`RELEASE_GRACE`] to run its
    /// trap, and is then killed before it does.
    pub(super) static TEST_RELEASE_GRACE: std::cell::Cell<Option<Duration>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(unix)]
fn release_grace() -> Duration {
    #[cfg(test)]
    if let Some(grace) = TEST_RELEASE_GRACE.with(std::cell::Cell::get) {
        return grace;
    }
    RELEASE_GRACE
}

/// One engine command, fully decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    /// Every argument after the executable.
    pub args: Vec<String>,
    /// Text for standard input (`type --stdin`, `set-value --stdin`).
    pub stdin: Option<String>,
    /// The command's own deadline. The wrapper ends it at this plus [`OUTER_GRACE`].
    pub deadline: Duration,
    /// Physical input: an interrupted gesture may leave the pointer pressed.
    pub physical: bool,
    /// `open_app` only: the app the engine starts must outlive the command (Windows job).
    pub releases_launched_apps: bool,
}

/// The engine's one-line JSON answer (contract 2.6).
#[derive(Clone, Debug)]
pub struct Envelope {
    pub ok: bool,
    pub data: Value,
    pub error: Value,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RunError {
    #[error("computer_use_cancelled")]
    Cancelled {
        /// The process had to be killed before it exited on its own.
        killed: bool,
        physical: bool,
    },
    #[error("computer_use_timeout")]
    TimedOut { physical: bool },
    #[error("{0}")]
    Spawn(String),
    #[error("{0}")]
    Protocol(String),
}

/// Where and how to run the engine for one task.
#[derive(Clone, Debug)]
pub struct Engine {
    pub executable: PathBuf,
    /// `AGENT_DESKTOP_HOME`: the task's state folder.
    pub home: PathBuf,
    /// `TMPDIR`, `TMP`, `TEMP`.
    pub temp: PathBuf,
}

/// The variables the engine needs from the person's session, and nothing else.
pub fn session_environment(engine: &Engine) -> Vec<(OsString, OsString)> {
    const SESSION: &[&str] = &[
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "SystemRoot",
        "WINDIR",
        "USERPROFILE",
        "LOCALAPPDATA",
        "APPDATA",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XAUTHORITY",
        "XDG_RUNTIME_DIR",
        "XDG_SESSION_TYPE",
        "XDG_SESSION_ID",
        "XDG_SESSION_DESKTOP",
        "XDG_CURRENT_DESKTOP",
        "XDG_DATA_DIRS",
        "XDG_DATA_HOME",
        "XDG_CONFIG_DIRS",
        "XDG_CONFIG_HOME",
        "DESKTOP_SESSION",
        "DBUS_SESSION_BUS_ADDRESS",
        "AT_SPI_BUS_ADDRESS",
    ];
    // Windows process startup reads these; the OS writes them at logon, so none can carry a
    // model credential or a daemon secret.
    #[cfg(windows)]
    const PLATFORM: &[&str] = &[
        "ComSpec",
        "PATHEXT",
        "SystemDrive",
        "ProgramData",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "CommonProgramFiles",
        "CommonProgramFiles(x86)",
        "CommonProgramW6432",
        "ALLUSERSPROFILE",
        "PUBLIC",
        "HOMEDRIVE",
        "HOMEPATH",
        "USERNAME",
        "USERDOMAIN",
        "PROCESSOR_ARCHITECTURE",
        "PROCESSOR_ARCHITEW6432",
        "NUMBER_OF_PROCESSORS",
        "OS",
    ];
    #[cfg(not(windows))]
    const PLATFORM: &[&str] = &[];
    let mut environment: Vec<(OsString, OsString)> = SESSION
        .iter()
        .chain(PLATFORM.iter())
        .filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(name), value)))
        .collect();
    environment.push(("AGENT_DESKTOP_HOME".into(), engine.home.clone().into()));
    for name in ["TMPDIR", "TMP", "TEMP"] {
        environment.push((name.into(), engine.temp.clone().into()));
    }
    environment
}

/// Parse and check one envelope. Anything but one JSON object on contract 2.6 is a protocol
/// error, and an `ok` envelope must carry `data`, an error one `error`.
pub fn parse_envelope(bytes: &[u8]) -> Result<Envelope, RunError> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        RunError::Protocol("computer_use_protocol_error: output is not UTF-8".into())
    })?;
    let line = text.trim();
    if line.is_empty() {
        return Err(RunError::Protocol(
            "computer_use_protocol_error: the native runtime printed nothing".into(),
        ));
    }
    let value: Value = serde_json::from_str(line).map_err(|_| {
        RunError::Protocol("computer_use_protocol_error: malformed native response".into())
    })?;
    if value.get("version").and_then(Value::as_str) != Some(super::manifest::CONTRACT_VERSION) {
        return Err(RunError::Protocol(
            "computer_use_incompatible_runtime: unexpected response contract version".into(),
        ));
    }
    let ok = value
        .get("ok")
        .and_then(Value::as_bool)
        .ok_or_else(|| RunError::Protocol("computer_use_protocol_error: missing ok".into()))?;
    let data = value.get("data").cloned().unwrap_or(Value::Null);
    let error = value.get("error").cloned().unwrap_or(Value::Null);
    if (ok && data.is_null()) || (!ok && !error.is_object()) {
        return Err(RunError::Protocol(
            "computer_use_protocol_error: incomplete native response".into(),
        ));
    }
    Ok(Envelope { ok, data, error })
}

/// Run one command to completion, Stop, or its deadline.
pub async fn run(
    engine: &Engine,
    invocation: &Invocation,
    cancel: &CancellationToken,
) -> Result<Envelope, RunError> {
    if cancel.is_cancelled() {
        return Err(RunError::Cancelled {
            killed: false,
            physical: invocation.physical,
        });
    }
    let environment = session_environment(engine);
    let bytes = platform::run(
        &engine.executable,
        invocation,
        &environment,
        cancel,
        invocation.deadline + OUTER_GRACE,
    )
    .await?;
    if bytes.len() > MAX_OUTPUT_BYTES {
        return Err(RunError::Protocol(
            "computer_use_payload_limit: the native response exceeds 16 MiB".into(),
        ));
    }
    parse_envelope(&bytes)
}

/// Read up to the cap plus one byte, so an oversized answer is detected rather than cut.
#[cfg_attr(windows, allow(dead_code))]
fn read_capped(reader: impl std::io::Read) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    reader
        .take(MAX_OUTPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    struct Spawned {
        pid: libc::pid_t,
        stdout: std::io::PipeReader,
        stdin: Option<std::io::PipeWriter>,
    }

    fn c_string(bytes: &[u8]) -> Result<CString, RunError> {
        CString::new(bytes).map_err(|_| {
            RunError::Spawn("computer_use_invalid_argument: an argument contains a NUL byte".into())
        })
    }

    /// `responsibility_spawnattrs_setdisclaim`, a private libSystem function terminals and
    /// debuggers use for the same purpose. Looked up at run time, never linked.
    #[cfg(target_os = "macos")]
    fn disclaim(attributes: &mut libc::posix_spawnattr_t) -> Result<(), RunError> {
        type SetDisclaim =
            unsafe extern "C" fn(*mut libc::posix_spawnattr_t, libc::c_int) -> libc::c_int;
        let name = c"responsibility_spawnattrs_setdisclaim";
        // SAFETY: dlsym with RTLD_DEFAULT and a NUL-terminated name only reads the loaded images.
        let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
        if symbol.is_null() {
            return Err(RunError::Spawn(
                "computer_use_unsupported_environment: this macOS cannot start Biorouter Copilot under its own permissions".into(),
            ));
        }
        // SAFETY: the symbol has this signature on every macOS that exports it.
        let set: SetDisclaim = unsafe { std::mem::transmute(symbol) };
        // SAFETY: `attributes` was initialised by posix_spawnattr_init.
        if unsafe { set(attributes, 1) } != 0 {
            return Err(RunError::Spawn(
                "computer_use_unsupported_environment: Biorouter Copilot could not be started under its own permissions".into(),
            ));
        }
        Ok(())
    }

    fn spawn_error(what: &str, code: libc::c_int) -> RunError {
        RunError::Spawn(format!(
            "computer_use_missing_runtime: the native runtime could not start ({what}: {})",
            std::io::Error::from_raw_os_error(code)
        ))
    }

    fn check(what: &str, code: libc::c_int) -> Result<(), RunError> {
        if code == 0 {
            Ok(())
        } else {
            Err(spawn_error(what, code))
        }
    }

    /// `argv` and `envp` for posix_spawn: the owned strings and the NULL-terminated pointer
    /// arrays into them.
    fn c_vectors(
        executable: &Path,
        invocation: &Invocation,
        environment: &[(OsString, OsString)],
    ) -> Result<(Vec<CString>, Vec<CString>), RunError> {
        let mut argv = vec![c_string(executable.as_os_str().as_bytes())?];
        for argument in &invocation.args {
            argv.push(c_string(argument.as_bytes())?);
        }
        let mut envp = Vec::with_capacity(environment.len());
        for (name, value) in environment {
            let mut entry = name.as_bytes().to_vec();
            entry.push(b'=');
            entry.extend_from_slice(value.as_bytes());
            envp.push(c_string(&entry)?);
        }
        Ok((argv, envp))
    }

    fn pointers(strings: &[CString]) -> Vec<*mut libc::c_char> {
        let mut pointers: Vec<*mut libc::c_char> =
            strings.iter().map(|s| s.as_ptr() as *mut _).collect();
        pointers.push(std::ptr::null_mut());
        pointers
    }

    /// Standard input from a pipe or `/dev/null`, standard output to the pipe, standard error to
    /// `/dev/null`.
    ///
    /// # Safety
    /// `actions` must be initialised.
    unsafe fn route_streams(
        actions: &mut libc::posix_spawn_file_actions_t,
        stdin: Option<libc::c_int>,
        stdout: libc::c_int,
    ) -> Result<(), RunError> {
        let dev_null = c"/dev/null";
        // SAFETY: `actions` is initialised (the caller's contract) and the path outlives the call.
        unsafe {
            match stdin {
                Some(fd) => check(
                    "stdin",
                    libc::posix_spawn_file_actions_adddup2(actions, fd, 0),
                )?,
                None => check(
                    "stdin",
                    libc::posix_spawn_file_actions_addopen(
                        actions,
                        0,
                        dev_null.as_ptr(),
                        libc::O_RDONLY,
                        0,
                    ),
                )?,
            }
            check(
                "stdout",
                libc::posix_spawn_file_actions_adddup2(actions, stdout, 1),
            )?;
            check(
                "stderr",
                libc::posix_spawn_file_actions_addopen(
                    actions,
                    2,
                    dev_null.as_ptr(),
                    libc::O_WRONLY,
                    0,
                ),
            )
        }
    }

    /// Default signal dispositions and an empty mask (the daemon ignores SIGPIPE, as Rust does;
    /// std::process::Command resets both for its children too), a process group of its own,
    /// close-on-exec for everything else on macOS, and responsibility disclaimed there (AD-04).
    ///
    /// # Safety
    /// `attributes` must be initialised.
    unsafe fn configure(attributes: &mut libc::posix_spawnattr_t) -> Result<(), RunError> {
        // SAFETY: `attributes` is initialised (the caller's contract); the sets are local.
        unsafe {
            let mut defaults: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut defaults);
            libc::sigaddset(&mut defaults, libc::SIGPIPE);
            libc::sigaddset(&mut defaults, libc::SIGTERM);
            libc::sigaddset(&mut defaults, libc::SIGINT);
            check(
                "signals",
                libc::posix_spawnattr_setsigdefault(attributes, &defaults),
            )?;
            let mut mask: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut mask);
            check(
                "signal mask",
                libc::posix_spawnattr_setsigmask(attributes, &mask),
            )?;
            #[allow(unused_mut)]
            let mut flags = libc::POSIX_SPAWN_SETPGROUP
                | libc::POSIX_SPAWN_SETSIGDEF
                | libc::POSIX_SPAWN_SETSIGMASK;
            #[cfg(target_os = "macos")]
            {
                flags |= libc::POSIX_SPAWN_CLOEXEC_DEFAULT;
            }
            check(
                "flags",
                libc::posix_spawnattr_setflags(attributes, flags as _),
            )?;
            check(
                "process group",
                libc::posix_spawnattr_setpgroup(attributes, 0),
            )?;
        }
        #[cfg(target_os = "macos")]
        disclaim(attributes)?;
        Ok(())
    }

    fn spawn(
        executable: &Path,
        invocation: &Invocation,
        environment: &[(OsString, OsString)],
    ) -> Result<Spawned, RunError> {
        let (argv_owned, env_owned) = c_vectors(executable, invocation, environment)?;
        let argv = pointers(&argv_owned);
        let envp = pointers(&env_owned);
        // Pipes from std are close-on-exec; dup2 in the file actions clears it on the copy.
        let pipe = || {
            std::io::pipe()
                .map_err(|e| RunError::Spawn(format!("computer_use_missing_runtime: {e}")))
        };
        let (stdout_read, stdout_write) = pipe()?;
        let stdin_pipe = if invocation.stdin.is_some() {
            Some(pipe()?)
        } else {
            None
        };
        // SAFETY: the action and attribute objects are initialised before use and destroyed on
        // every path after initialisation; every pointer passed outlives the call.
        let pid = unsafe {
            let mut actions: libc::posix_spawn_file_actions_t = std::mem::zeroed();
            check(
                "file actions",
                libc::posix_spawn_file_actions_init(&mut actions),
            )?;
            let mut attributes: libc::posix_spawnattr_t = std::mem::zeroed();
            let code = libc::posix_spawnattr_init(&mut attributes);
            if code != 0 {
                libc::posix_spawn_file_actions_destroy(&mut actions);
                return Err(spawn_error("attributes", code));
            }
            let stdin_fd = stdin_pipe.as_ref().map(|(read, _)| read.as_raw_fd());
            let result = route_streams(&mut actions, stdin_fd, stdout_write.as_raw_fd())
                .and_then(|()| configure(&mut attributes))
                .and_then(|()| {
                    let mut pid: libc::pid_t = 0;
                    check(
                        "spawn",
                        libc::posix_spawn(
                            &mut pid,
                            argv_owned[0].as_ptr(),
                            &actions,
                            &attributes,
                            argv.as_ptr(),
                            envp.as_ptr(),
                        ),
                    )
                    .map(|()| pid)
                });
            libc::posix_spawnattr_destroy(&mut attributes);
            libc::posix_spawn_file_actions_destroy(&mut actions);
            result?
        };
        drop(stdout_write);
        let stdin = stdin_pipe.map(|(read, write)| {
            drop(read);
            write
        });
        Ok(Spawned {
            pid,
            stdout: stdout_read,
            stdin,
        })
    }

    fn signal_group(pid: libc::pid_t, reaped: &AtomicBool, signal: libc::c_int) {
        if pid > 1 && !reaped.load(Ordering::Acquire) {
            // SAFETY: a negative pid names the process group this wrapper created for the
            // command; it is signalled only while its leader has not been reaped.
            unsafe {
                libc::kill(-pid, signal);
            }
        }
    }

    pub(super) async fn run(
        executable: &Path,
        invocation: &Invocation,
        environment: &[(OsString, OsString)],
        cancel: &CancellationToken,
        bound: Duration,
    ) -> Result<Vec<u8>, RunError> {
        let Spawned { pid, stdout, stdin } = spawn(executable, invocation, environment)?;
        let reaped = Arc::new(AtomicBool::new(false));
        if let (Some(mut writer), Some(text)) = (stdin, invocation.stdin.clone()) {
            tokio::task::spawn_blocking(move || {
                use std::io::Write;
                let _ = writer.write_all(text.as_bytes());
            });
        }
        let reader = tokio::task::spawn_blocking(move || read_capped(stdout));
        let waiter = {
            let reaped = reaped.clone();
            tokio::task::spawn_blocking(move || {
                // Observe the exit without reaping, so the leader stays a zombie and its group
                // id cannot be reused while the group is swept; then sweep, then reap.
                loop {
                    // SAFETY: a zeroed siginfo_t is a valid out parameter for waitid.
                    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
                    // SAFETY: waits on exactly the child this wrapper spawned.
                    let result = unsafe {
                        libc::waitid(
                            libc::P_PID,
                            pid as libc::id_t,
                            &mut info,
                            libc::WEXITED | libc::WNOWAIT,
                        )
                    };
                    if result == 0
                        || std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR)
                    {
                        break;
                    }
                }
                // SAFETY: the leader is an unreaped zombie, so `-pid` is still this command's
                // own group: nothing it started outlives it.
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
                let mut status = 0;
                loop {
                    // SAFETY: reaps exactly the child this wrapper spawned.
                    let result = unsafe { libc::waitpid(pid, &mut status, 0) };
                    if result == pid
                        || (result == -1
                            && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR))
                    {
                        break;
                    }
                }
                reaped.store(true, Ordering::Release);
                status
            })
        };
        tokio::pin!(waiter);
        let stopped = tokio::select! {
            biased;
            _ = cancel.cancelled() => Some(true),
            _ = tokio::time::sleep(bound) => Some(false),
            _ = &mut waiter => None,
        };
        if let Some(cancelled) = stopped {
            signal_group(pid, &reaped, libc::SIGTERM);
            let exited = tokio::time::timeout(super::release_grace(), &mut waiter)
                .await
                .is_ok();
            if !exited {
                signal_group(pid, &reaped, libc::SIGKILL);
                let _ = tokio::time::timeout(Duration::from_secs(5), &mut waiter).await;
            }
            reader.abort();
            return Err(if cancelled {
                RunError::Cancelled {
                    killed: !exited,
                    physical: invocation.physical,
                }
            } else {
                RunError::TimedOut {
                    physical: invocation.physical,
                }
            });
        }
        // A descendant that kept standard output open must not hold the call.
        match tokio::time::timeout(Duration::from_secs(2), reader).await {
            Ok(Ok(Ok(bytes))) => Ok(bytes),
            _ => Err(RunError::Protocol(
                "computer_use_protocol_error: the native response could not be read".into(),
            )),
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::process::Stdio;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    pub(super) async fn run(
        executable: &Path,
        invocation: &Invocation,
        environment: &[(OsString, OsString)],
        cancel: &CancellationToken,
        bound: Duration,
    ) -> Result<Vec<u8>, RunError> {
        let mut command = tokio::process::Command::new(executable);
        command
            .args(&invocation.args)
            .env_clear()
            .envs(environment.iter().map(|(k, v)| (k, v)))
            .stdin(if invocation.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .creation_flags(
                windows_sys::Win32::System::Threading::CREATE_NO_WINDOW
                    | windows_sys::Win32::System::Threading::CREATE_SUSPENDED,
            );
        let mut child = command.spawn().map_err(|e| {
            RunError::Spawn(format!(
                "computer_use_missing_runtime: the native runtime could not start ({e})"
            ))
        })?;
        let job = super::super::windows_job::ProcessJob::attach_with(
            &child,
            invocation.releases_launched_apps,
        )
        .map_err(|e| RunError::Spawn(e.to_string()))?;
        if let (Some(mut writer), Some(text)) = (child.stdin.take(), invocation.stdin.clone()) {
            tokio::spawn(async move {
                let _ = writer.write_all(text.as_bytes()).await;
            });
        }
        let mut stdout = child.stdout.take().ok_or_else(|| {
            RunError::Spawn("computer_use_missing_runtime: no output pipe".into())
        })?;
        let reader = tokio::spawn(async move {
            let mut bytes = Vec::new();
            (&mut stdout)
                .take(MAX_OUTPUT_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .await
                .map(|_| bytes)
        });
        let stopped = tokio::select! {
            biased;
            _ = cancel.cancelled() => Some(true),
            _ = tokio::time::sleep(bound) => Some(false),
            _ = child.wait() => None,
        };
        if let Some(cancelled) = stopped {
            // Windows has no graceful signal for a windowless console process: the job ends
            // the whole tree at once, so the engine's release guard never runs.
            job.terminate();
            job.wait_empty().await;
            if invocation.physical {
                release_held_buttons();
            }
            let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
            reader.abort();
            return Err(if cancelled {
                RunError::Cancelled {
                    killed: true,
                    physical: invocation.physical,
                }
            } else {
                RunError::TimedOut {
                    physical: invocation.physical,
                }
            });
        }
        let bytes = match tokio::time::timeout(Duration::from_secs(2), reader).await {
            Ok(Ok(Ok(bytes))) => Ok(bytes),
            _ => Err(RunError::Protocol(
                "computer_use_protocol_error: the native response could not be read".into(),
            )),
        };
        if !invocation.releases_launched_apps {
            job.terminate();
        }
        bytes
    }

    /// AD-15 on Windows: a physical command ended by the job never ran its release guard, so a
    /// mouse button it pressed (a drag's) would stay down for every app on the desktop. Lift each
    /// button the system still reports pressed. Only after a physical command was stopped or timed
    /// out, so a button the person holds at any other moment is never touched.
    fn release_held_buttons() {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTUP,
            MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTUP, MOUSEINPUT, VK_LBUTTON, VK_MBUTTON,
            VK_RBUTTON,
        };
        for (key, flag) in [
            (VK_LBUTTON, MOUSEEVENTF_LEFTUP),
            (VK_RBUTTON, MOUSEEVENTF_RIGHTUP),
            (VK_MBUTTON, MOUSEEVENTF_MIDDLEUP),
        ] {
            // SAFETY: GetAsyncKeyState reads one key's state; the high bit means pressed.
            if unsafe { GetAsyncKeyState(i32::from(key)) } >= 0 {
                continue;
            }
            let input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: 0,
                        dy: 0,
                        mouseData: 0,
                        dwFlags: flag,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            // SAFETY: one fully initialised INPUT, with its size as the API requires.
            unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) };
        }
    }
}

#[cfg(all(test, unix))]
pub(crate) mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in engine: a shell script that records its argv, environment and standard input
    /// and prints what the test scripted.
    pub fn fake_engine(root: &Path, body: &str) -> Engine {
        let executable = root.join("agent-desktop");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$AGENT_DESKTOP_HOME/argv\"\nenv > \"$AGENT_DESKTOP_HOME/env\"\n{body}\n"
        );
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let home = root.join("home");
        let temp = root.join("tmp");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&temp).unwrap();
        Engine {
            executable,
            home,
            temp,
        }
    }

    fn invocation(args: &[&str]) -> Invocation {
        Invocation {
            args: args.iter().map(|a| a.to_string()).collect(),
            stdin: None,
            deadline: Duration::from_secs(10),
            physical: false,
            releases_launched_apps: false,
        }
    }

    const OK: &str =
        r#"echo '{"version":"2.6","ok":true,"command":"version","data":{"version":"1.3.1"}}'"#;

    #[test]
    fn the_environment_is_an_allowlist_plus_the_task_folders() {
        let root = tempfile::tempdir().unwrap();
        let engine = fake_engine(root.path(), OK);
        let environment = temp_env::with_vars(
            [
                ("BIOROUTER_SERVER__SECRET_KEY", Some("daemon-secret")),
                ("ANTHROPIC_API_KEY", Some("sk-ant-model-credential")),
                ("AGENT_DESKTOP_SESSION", Some("leaked-session")),
                ("AGENT_DESKTOP_HOME", Some("/somewhere/else")),
            ],
            || session_environment(&engine),
        );
        let names: Vec<String> = environment
            .iter()
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect();
        for secret in [
            "BIOROUTER_SERVER__SECRET_KEY",
            "ANTHROPIC_API_KEY",
            "AGENT_DESKTOP_SESSION",
        ] {
            assert!(
                !names.iter().any(|name| name == secret),
                "{secret} reached the engine"
            );
        }
        let home: Vec<_> = environment
            .iter()
            .filter(|(name, _)| name == "AGENT_DESKTOP_HOME")
            .collect();
        assert_eq!(home.len(), 1);
        assert_eq!(home[0].1, engine.home.as_os_str());
    }

    #[tokio::test]
    async fn a_command_gets_its_arguments_its_stdin_and_its_own_state_root() {
        let root = tempfile::tempdir().unwrap();
        let engine = fake_engine(
            root.path(),
            &format!("cat > \"$AGENT_DESKTOP_HOME/stdin\"\n{OK}"),
        );
        let mut call = invocation(&["type", "@s1:e1", "--stdin"]);
        call.stdin = Some("hello\nworld".into());
        let envelope = run(&engine, &call, &CancellationToken::new())
            .await
            .unwrap();
        assert!(envelope.ok);
        assert_eq!(envelope.data["version"], "1.3.1");
        let argv = std::fs::read_to_string(engine.home.join("argv")).unwrap();
        assert_eq!(argv, "type\n@s1:e1\n--stdin\n");
        let stdin = std::fs::read_to_string(engine.home.join("stdin")).unwrap();
        assert_eq!(stdin, "hello\nworld");
        let env = std::fs::read_to_string(engine.home.join("env")).unwrap();
        assert!(env.contains(&format!("AGENT_DESKTOP_HOME={}", engine.home.display())));
        assert!(env.contains(&format!("TMPDIR={}", engine.temp.display())));
    }

    #[tokio::test]
    async fn a_command_that_exits_leaves_nothing_running_in_its_group() {
        let root = tempfile::tempdir().unwrap();
        let engine = fake_engine(
            root.path(),
            &format!("sh -c 'echo $$ > \"$AGENT_DESKTOP_HOME/child\"; exec sleep 60' &\nwhile [ ! -s \"$AGENT_DESKTOP_HOME/child\" ]; do sleep 0.05; done\n{OK}"),
        );
        run(
            &engine,
            &invocation(&["version"]),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let child = wait_for_file(&engine.home.join("child")).await;
        assert!(gone(child).await, "a descendant outlived its command");
    }

    #[tokio::test]
    async fn malformed_oversized_and_wrong_contract_output_is_refused() {
        let root = tempfile::tempdir().unwrap();
        for (body, expected) in [
            ("echo 'not json'", "malformed"),
            ("true", "printed nothing"),
            (
                r#"echo '{"version":"2.5","ok":true,"command":"x","data":{}}'"#,
                "contract version",
            ),
            (
                r#"echo '{"version":"2.6","ok":false,"command":"x"}'"#,
                "incomplete",
            ),
            ("head -c 16777300 /dev/zero | tr '\\0' 'a'", "16 MiB"),
        ] {
            let engine = fake_engine(root.path(), body);
            let error = run(
                &engine,
                &invocation(&["version"]),
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();
            assert!(error.to_string().contains(expected), "{body}: {error}");
        }
    }

    #[tokio::test]
    async fn an_error_envelope_is_returned_with_its_disposition() {
        let root = tempfile::tempdir().unwrap();
        let engine = fake_engine(
            root.path(),
            r#"echo '{"version":"2.6","ok":false,"command":"click","error":{"code":"STALE_REF","message":"gone","disposition":{"delivery":"not_delivered","retry":"safe"}}}'; exit 1"#,
        );
        let envelope = run(
            &engine,
            &invocation(&["click", "@s1:e1"]),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(!envelope.ok);
        assert_eq!(envelope.error["code"], "STALE_REF");
    }

    fn alive(pid: i32) -> bool {
        // SAFETY: probes a pid this test's fixture wrote.
        let exists = unsafe { libc::kill(pid, 0) } == 0;
        #[cfg(target_os = "linux")]
        let exists = exists
            && !std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .is_ok_and(|stat| stat.split_whitespace().nth(2) == Some("Z"));
        exists
    }

    async fn gone(pid: i32) -> bool {
        for _ in 0..200 {
            if !alive(pid) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    async fn wait_for_file(path: &Path) -> i32 {
        for _ in 0..500 {
            if let Some(pid) = std::fs::read_to_string(path)
                .ok()
                .and_then(|text| text.trim().parse().ok())
            {
                return pid;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("fixture never started");
    }

    #[tokio::test]
    async fn stop_gives_the_release_guard_a_grace_then_ends_the_whole_group() {
        let root = tempfile::tempdir().unwrap();
        // The engine traps SIGTERM like a release guard would, writes that it released, and
        // exits; its child ignores SIGTERM and must still die with the group.
        let engine = fake_engine(
            root.path(),
            "sh -c 'trap \"\" TERM; echo $$ > \"$AGENT_DESKTOP_HOME/child\"; while :; do sleep 1; done' &\n\
             trap 'echo released > \"$AGENT_DESKTOP_HOME/released\"; exit 0' TERM\n\
             echo $$ > \"$AGENT_DESKTOP_HOME/engine\"\nwhile :; do sleep 1; done",
        );
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        let home = engine.home.clone();
        let watcher = tokio::spawn(async move {
            let child = wait_for_file(&home.join("child")).await;
            wait_for_file(&home.join("engine")).await;
            stop.cancel();
            child
        });
        let mut call = invocation(&["drag"]);
        call.physical = true;
        let error = run(&engine, &call, &cancel).await.unwrap_err();
        let child = watcher.await.unwrap();
        assert!(
            matches!(error, RunError::Cancelled { physical: true, .. }),
            "{error:?}"
        );
        assert_eq!(
            std::fs::read_to_string(engine.home.join("released"))
                .unwrap()
                .trim(),
            "released",
            "SIGTERM came first, so the release guard could run"
        );
        // The group got SIGKILL after the grace because the trapping child outlived SIGTERM.
        assert!(gone(child).await, "a descendant outlived Stop");
    }

    #[tokio::test]
    async fn the_outer_deadline_ends_a_command_that_hangs() {
        let root = tempfile::tempdir().unwrap();
        let engine = fake_engine(
            root.path(),
            "echo $$ > \"$AGENT_DESKTOP_HOME/engine\"\nsleep 60",
        );
        let mut call = invocation(&["snapshot"]);
        call.deadline = Duration::from_millis(1);
        let started = std::time::Instant::now();
        let error = tokio::time::timeout(
            Duration::from_secs(20),
            run(&engine, &call, &CancellationToken::new()),
        )
        .await
        .expect("the outer bound must end the command")
        .unwrap_err();
        assert_eq!(error, RunError::TimedOut { physical: false });
        assert!(started.elapsed() < Duration::from_secs(10));
        let pid = wait_for_file(&engine.home.join("engine")).await;
        assert!(gone(pid).await);
    }

    #[tokio::test]
    async fn a_missing_executable_is_a_spawn_error_not_a_panic() {
        let root = tempfile::tempdir().unwrap();
        let engine = Engine {
            executable: root.path().join("missing"),
            home: root.path().to_owned(),
            temp: root.path().to_owned(),
        };
        let error = run(
            &engine,
            &invocation(&["version"]),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, RunError::Spawn(_)), "{error:?}");
    }

    /// The pinned engine through the real spawn path (disclaimed on macOS): its version, and a
    /// bare reference refused before anything reaches the desktop.
    #[tokio::test]
    #[ignore = "needs a built payload in BIOROUTER_COMPUTER_USE_DIR; touches no desktop"]
    async fn the_pinned_engine_answers_through_the_wrapper_spawn() {
        let root = std::env::var_os("BIOROUTER_COMPUTER_USE_DIR").expect("select the payload");
        let payload = super::super::manifest::validate(Path::new(&root), true).expect("payload");
        let state = tempfile::tempdir().unwrap();
        let engine = Engine {
            executable: payload.executable,
            home: state.path().to_owned(),
            temp: state.path().to_owned(),
        };
        let version = run(
            &engine,
            &invocation(&["version"]),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(version.ok);
        assert_eq!(
            version.data["version"],
            super::super::manifest::UPSTREAM_VERSION
        );
        let bare = run(
            &engine,
            &invocation(&["click", "--session=contract", "--", "@e1"]),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(!bare.ok);
        assert_eq!(bare.error["code"], "INVALID_ARGS");
        assert_eq!(bare.error["disposition"]["delivery"], "not_delivered");
    }
}
