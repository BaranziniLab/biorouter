//! Starting and restarting a workspace by its ID (account ownership, AO-7).
//!
//! `biorouter-crew start --workspace-id UUID` and `biorouter-crew restart --workspace-id UUID`
//! find the workspace's state directory under `~/.local/share/biorouter-crew/` by the ID its
//! `journal.jsonl` holds, so setup only ever substitutes an ID it checked, never a path the
//! server printed. Neither ever creates a journal: an ID no directory holds is refused
//! `not_found`, and so is a directory whose journal went away before the broker opened it. An
//! ID more than one directory holds (a copy of a workspace's folder kept beside it) is refused
//! `identity_conflict`, naming the folders, before anything is started or stopped.
//!
//! `start --workspace-id` answers exactly as `start --state-dir` does, with `created: false`.
//!
//! `restart --workspace-id` is one command, so a dropped SSH connection can never leave a
//! workspace stopped halfway through:
//!
//! 1. It takes the account's restart lock, an exclusive `flock` on
//!    `~/.local/share/biorouter-crew/.restart-lock`, refused `busy` while another restart holds
//!    it (as setup's install lock refuses a second install). Without an installed program to
//!    start in the old one's place it stops nothing (`program_missing`).
//! 2. It leaves the terminal's session (`setsid`, `SIGHUP` ignored) before it stops anything.
//! 3. It stops the running broker, checked as `stop` checks it (the listener's kernel PID is the
//!    one `runtime.json` records, and it answers `hello` for this workspace), and waits on the
//!    process's pidfd until it has exited: at most 15 seconds, else `stop_timeout`, and then
//!    nothing is started.
//! 4. It starts the installed program, `~/.local/bin/biorouter-crew`, and checks that `hello`
//!    answers for the same workspace, else `start_failed` with the last line of `broker.log`.
//! 5. It answers `{"restarted": true, "from_version": "1.93.0", "to_version": "1.94.1"}`, where
//!    `from_version` is the version the stopped broker reported (`null` for a broker too old to
//!    say, or when the workspace was not running, in which case it is only started) and
//!    `to_version` the version the new one records in `runtime.json`.
//!
//! A deleted workspace counting down to its erase is never restarted (`workspace_deleted`).
use super::*;

/// The account's workspace store: `<home>/.local/share/biorouter-crew`.
pub(super) fn store_of(home: &Path) -> PathBuf {
    home.join(".local").join("share").join("biorouter-crew")
}

/// The program setup installs: `<home>/.local/bin/biorouter-crew`.
pub(super) fn installed_program(home: &Path) -> PathBuf {
    home.join(".local").join("bin").join("biorouter-crew")
}

/// The file whose `flock` serializes restarts in one account.
const RESTART_LOCK: &str = ".restart-lock";
/// How long `restart` waits for the old broker to exit.
const STOP_TIMEOUT: Duration = Duration::from_secs(15);
/// The most entries of the store a lookup by ID reads.
const STORE_SCAN_LIMIT: usize = 4096;
/// The first journal record is read up to this bound, as replay bounds every record.
const FIRST_RECORD_BYTES: u64 = 16 * 1024 * 1024;

const RESTART_BUSY: &str = "busy: Another restart is running for a workspace in this account. Try again when it has finished.";
#[cfg(target_os = "linux")]
const RESTART_DELETED: &str =
    "workspace_deleted: This workspace was deleted and is erasing itself, so it is not restarted.";

fn not_found(workspace_id: &str) -> anyhow::Error {
    anyhow!("not_found: No workspace with ID {workspace_id} is kept in this account.")
}

/// The kernel's account and process for the other end of `stream` (`SO_PEERCRED`).
#[cfg(target_os = "linux")]
pub(super) fn peer_credentials(stream: &UnixStream) -> Result<(u32, i32)> {
    let mut cred = std::mem::MaybeUninit::<libc::ucred>::uninit();
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    ensure!(
        unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                cred.as_mut_ptr().cast(),
                &mut length,
            )
        } == 0
            && length as usize == std::mem::size_of::<libc::ucred>(),
        "identity_unavailable: kernel peer credentials unavailable"
    );
    let cred = unsafe { cred.assume_init() };
    Ok((cred.uid, cred.pid))
}
#[cfg(not(target_os = "linux"))]
pub(super) fn peer_credentials(_stream: &UnixStream) -> Result<(u32, i32)> {
    bail!("unsupported: Crew brokers require Linux SO_PEERCRED")
}

/// The workspace ID `journal` names, read from its first record alone: the record that
/// initialized the workspace carries the whole first state, and a workspace's ID never changes.
/// `None` when the journal is missing, not this account's private regular file, empty, or
/// shaped otherwise.
pub(super) fn journal_workspace_id(journal: &Path) -> Option<String> {
    let file = OpenOptions::new()
        .read(true)
        // A FIFO in its place must not stall the lookup.
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(journal)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !(metadata.is_file()
        && metadata.uid() == unsafe { libc::geteuid() }
        && metadata.mode() & 0o077 == 0)
    {
        return None;
    }
    let mut line = Vec::new();
    BufReader::new(file.take(FIRST_RECORD_BYTES))
        .read_until(b'\n', &mut line)
        .ok()?;
    if !line.ends_with(b"\n") {
        return None;
    }
    let record: Value = serde_json::from_slice(&line).ok()?;
    let state = match record.get("version").and_then(Value::as_u64) {
        Some(1) => record.get("state")?,
        Some(2) => {
            let first = record.get("patches")?.as_array()?.first()?;
            let root = first.get("path")?.as_array()?.is_empty();
            if first.get("op").and_then(Value::as_str) != Some("set") || !root {
                return None;
            }
            first.get("value")?
        }
        _ => return None,
    };
    let id = state.get("workspace")?.get("id")?.as_str()?;
    broker_id(id).then(|| id.to_owned())
}

/// The refusal for a workspace ID that more than one folder of the store holds: a copy kept
/// beside the workspace's own folder (`cp -a lab lab-backup`). Which one is the workspace
/// cannot be told from the folders, so nothing is started or stopped, and the listing reports
/// each of them with this sentence instead of guessing.
pub(super) fn held_by_several_folders(workspace_id: &str, folders: &[PathBuf]) -> String {
    let names: Vec<String> = folders
        .iter()
        .map(|folder| {
            folder.file_name().map_or_else(
                || folder.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            )
        })
        .collect();
    format!(
        "identity_conflict: Workspace {workspace_id} is kept in more than one folder of ~/.local/share/biorouter-crew ({}). Move every copy out of that folder, keeping the workspace's own, then try again.",
        names.join(", ")
    )
}

/// The state directory under `store` whose journal holds `workspace_id`: a directory (not a
/// symbolic link) this account owns. Directory names are never trusted to say which workspace
/// is inside; a renamed workspace keeps its folder. When more than one folder holds the ID
/// (a copy of the folder kept in the store), none is chosen: [`held_by_several_folders`].
pub(super) fn find_state_dir(store: &Path, workspace_id: &str) -> Result<PathBuf> {
    ensure!(
        broker_id(workspace_id),
        "invalid_params: --workspace-id must be a workspace ID (a lowercase UUID)"
    );
    let entries = match fs::read_dir(store) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(not_found(workspace_id))
        }
        Err(error) => {
            return Err(error).context("unsafe_storage: cannot read the workspace folder")
        }
    };
    let euid = unsafe { libc::geteuid() };
    // Sorted before the cap, so which folders are read never depends on directory order.
    let mut names: Vec<std::ffi::OsString> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name())
        .filter(|name| !name.to_string_lossy().starts_with('.'))
        .collect();
    names.sort();
    names.truncate(STORE_SCAN_LIMIT);
    let mut holding = Vec::new();
    for name in names {
        let dir = store.join(&name);
        let owned = fs::symlink_metadata(&dir)
            .is_ok_and(|metadata| metadata.is_dir() && metadata.uid() == euid);
        if !owned {
            continue;
        }
        let held = journal_workspace_id(&dir.join("journal.jsonl")).or_else(|| {
            // A journal whose first record is shaped otherwise is read whole, as `start` reads
            // one.
            stored_workspace(&dir)
                .ok()
                .flatten()
                .map(|stored| stored.id)
        });
        if held.as_deref() == Some(workspace_id) {
            holding.push(dir);
        }
    }
    match holding.len() {
        0 => Err(not_found(workspace_id)),
        1 => Ok(holding.remove(0)),
        _ => bail!(held_by_several_folders(workspace_id, &holding)),
    }
}

/// The account's restart lock, held for as long as this value lives (and by a child this
/// process forks, which shares the open file).
struct RestartLock {
    _file: File,
}

fn restart_lock(store: &Path) -> Result<RestartLock> {
    let file = private_file(&store.join(RESTART_LOCK), false)?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
            bail!(RESTART_BUSY);
        }
        return Err(error).context("restart_lock: cannot lock the restart lock file");
    }
    Ok(RestartLock { _file: file })
}

/// Leave the terminal's session before anything is stopped, so a closed SSH connection or a
/// hung-up terminal cannot end the restart between stopping the old broker and starting the
/// new one. `SIGHUP` is ignored, and the process starts a session of its own. A process that
/// already leads a process group (a shell's job, or a program a login shell replaced itself
/// with) cannot, so it carries on in a child that can, while the parent waits for it and exits
/// with its status. Called while the process has only its main thread.
fn leave_session() -> Result<()> {
    unsafe {
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }
    if unsafe { libc::setsid() } >= 0 {
        return Ok(());
    }
    match unsafe { libc::fork() } {
        -1 => Err(std::io::Error::last_os_error()).context("restart: cannot leave the terminal"),
        0 => {
            ensure!(
                unsafe { libc::setsid() } >= 0,
                "restart: cannot leave the terminal's session"
            );
            Ok(())
        }
        child => {
            let mut status = 0;
            while unsafe { libc::waitpid(child, &mut status, 0) } < 0 {
                if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                    std::process::exit(1);
                }
            }
            let code = if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status)
            } else {
                1
            };
            std::process::exit(code)
        }
    }
}

/// The version a broker reports: `hello`'s, else its `runtime.json`'s; `None` for a broker too
/// old to say.
fn reported_version(hello: Option<&Value>, info: Option<&Value>) -> Option<String> {
    hello
        .and_then(|hello| hello.get("version"))
        .or_else(|| info.and_then(|info| info.get("version")))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Stop `workspace_id`'s broker running from `root` and wait until its process has exited, at
/// most `timeout`. `Ok(None)` when no broker is running there; `Ok(Some(version))` once the one
/// that was has exited, with the version it reported. The process is verified as `stop`
/// verifies it, and signalled through its pidfd, so a reused PID is never signalled.
#[cfg(target_os = "linux")]
fn stop_and_wait(
    root: &Path,
    workspace_id: &str,
    timeout: Duration,
) -> Result<Option<Option<String>>> {
    use std::os::fd::FromRawFd;
    let Some(info) = existing_runtime_descriptor(root)? else {
        return Ok(None);
    };
    ensure!(
        info.get("workspace_id").and_then(Value::as_str) == Some(workspace_id),
        "identity_mismatch: runtime.json names another workspace"
    );
    let owner = number(&info, "host_uid")? as u32;
    ensure!(
        owner == unsafe { libc::geteuid() },
        "forbidden: only host account can stop broker"
    );
    let socket = PathBuf::from(text(&info, "socket")?);
    let mut stream = match connect_recorded(&info, &socket, owner) {
        Ok(stream) => stream,
        Err(error) if error.to_string() == NOT_RUNNING => return Ok(None),
        Err(error) => return Err(error),
    };
    let (uid, pid) = peer_credentials(&stream)?;
    ensure!(
        uid == owner && pid as u64 == number(&info, "pid")?,
        "identity_mismatch"
    );
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    ensure!(fd >= 0, "unsupported: safe stop requires Linux pidfd_open");
    let process = unsafe { File::from_raw_fd(fd as i32) };
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(
        b"{\"version\":1,\"id\":\"restart-verify\",\"method\":\"hello\",\"params\":{}}\n",
    )?;
    let response: Response = serde_json::from_slice(
        &read_frame(&mut BufReader::new(stream))?.ok_or_else(|| anyhow!("unavailable"))?,
    )?;
    let hello = response.result.unwrap_or_default();
    ensure!(
        hello.get("workspace_id").and_then(Value::as_str) == Some(workspace_id),
        "identity_mismatch"
    );
    ensure!(
        hello.get("state").and_then(Value::as_str) != Some("deleted"),
        RESTART_DELETED
    );
    let from_version = reported_version(Some(&hello), Some(&info));
    ensure!(
        unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                process.as_raw_fd(),
                libc::SIGTERM,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        } == 0,
        "stop_failed: the broker could not be signalled"
    );
    // The pidfd becomes readable when the process has exited.
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let mut poll = libc::pollfd {
            fd: process.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let millis = left.as_millis().min(i32::MAX as u128) as i32;
        let ready = unsafe { libc::poll(&mut poll, 1, millis) };
        if ready > 0 {
            ensure!(
                poll.revents & libc::POLLIN != 0,
                "stop_failed: cannot wait for the broker to exit"
            );
            return Ok(Some(from_version));
        }
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error).context("stop_failed: cannot wait for the broker to exit");
        }
        bail!(
            "stop_timeout: The workspace's broker (process {pid}) was asked to stop and was still running after {} seconds, so the new program was not started. Try the restart again once it has stopped; biorouter-crew status --state-dir {} says whether it still runs.",
            timeout.as_secs(),
            root.display()
        );
    }
}
#[cfg(not(target_os = "linux"))]
fn stop_and_wait(
    _root: &Path,
    _workspace_id: &str,
    _timeout: Duration,
) -> Result<Option<Option<String>>> {
    bail!("unsupported: restart requires Linux pidfd")
}

/// `restart --workspace-id` for the account whose home is `home` (see the module's
/// documentation). `leave` detaches from the terminal first; only the command does, never a
/// test.
fn restart(home: &Path, workspace_id: &str, stop_timeout: Duration, leave: bool) -> Result<Value> {
    let store = store_of(home);
    let root = find_state_dir(&store, workspace_id)?;
    let _lock = restart_lock(&store)?;
    // Nothing is stopped unless there is a program to start in its place. This is its own
    // code: `start_failed` means the broker was stopped and the new one did not start.
    let program = installed_program(home);
    ensure!(
        fs::metadata(&program).is_ok_and(|file| file.is_file() && file.mode() & 0o111 != 0),
        "program_missing: {} is missing or cannot run, so the running broker was left as it is.",
        program.display()
    );
    if leave {
        leave_session()?;
    }
    let from_version = stop_and_wait(&root, workspace_id, stop_timeout)?.flatten();
    let started = start(&root, "", None, false, &program)?;
    if started.get("state").and_then(Value::as_str) != Some("running") {
        bail!(
            "start_failed: the broker did not answer within {} seconds; last log line: {}",
            START_READY_TIMEOUT.as_secs(),
            last_log_line(&root)
        );
    }
    ensure!(
        started.get("workspace_id").and_then(Value::as_str) == Some(workspace_id),
        "start_failed: the broker that started answers for another workspace; last log line: {}",
        last_log_line(&root)
    );
    let runtime = read_runtime_descriptor(&root).ok().flatten();
    Ok(json!({
        "restarted": true,
        "from_version": from_version,
        "to_version": reported_version(None, runtime.as_ref()),
    }))
}

impl Broker {
    /// `biorouter-crew start (--state-dir PATH | --name NAME) [--bootstrap-key HEX]
    /// [--create]`: launch this program as the workspace's broker. A workspace is created only
    /// when `root` holds none and a key or `create` is given (AO-7), and then the answer says
    /// `created: true`.
    pub fn start_state_dir(
        root: &Path,
        bootstrap_key: &str,
        name: Option<&str>,
        create: bool,
    ) -> Result<Value> {
        start(root, bootstrap_key, name, create, &std::env::current_exe()?)
    }

    /// `biorouter-crew start --workspace-id UUID` for the account whose home is `home`: start
    /// the workspace its store holds under that ID. Never creates one (`not_found`).
    pub fn start_by_id(home: &Path, workspace_id: &str) -> Result<Value> {
        let root = find_state_dir(&store_of(home), workspace_id)?;
        start(&root, "", None, false, &std::env::current_exe()?)
    }

    /// `biorouter-crew restart --workspace-id UUID` for the account whose home is `home`: stop
    /// the workspace's running broker and start the installed program in its place, as one
    /// command that leaves the terminal first. See the module's documentation for the answer
    /// and the refusals.
    pub fn restart_by_id(home: &Path, workspace_id: &str) -> Result<Value> {
        restart(home, workspace_id, STOP_TIMEOUT, true)
    }

    /// [`Broker::restart_by_id`] waiting at most `stop_timeout` for the old broker, without
    /// leaving the test's session. Test builds only.
    #[cfg(feature = "test-seams")]
    pub fn restart_by_id_with(
        home: &Path,
        workspace_id: &str,
        stop_timeout: Duration,
    ) -> Result<Value> {
        restart(home, workspace_id, stop_timeout, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn private_temp(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("crew-by-id-{label}-{}", Uuid::new_v4()));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        path
    }

    fn write_private(path: &Path, bytes: &[u8]) {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap();
        file.write_all(bytes).unwrap();
    }

    /// The first record of a journal names its workspace in either record version; anything
    /// else names none.
    #[test]
    fn the_first_record_names_the_workspace() {
        let dir = private_temp("first-record");
        let id = Uuid::new_v4().to_string();
        let v2 = dir.join("v2.jsonl");
        let record = json!({"version": 2, "sequence": 1, "patches": [
            {"op": "set", "path": [], "value": {"workspace": {"id": id}}}
        ]});
        write_private(&v2, format!("{record}\n{{\"later\":true}}\n").as_bytes());
        assert_eq!(journal_workspace_id(&v2).as_deref(), Some(id.as_str()));

        let v1 = dir.join("v1.jsonl");
        let record = json!({"version": 1, "state": {"workspace": {"id": id}}});
        write_private(&v1, format!("{record}\n").as_bytes());
        assert_eq!(journal_workspace_id(&v1).as_deref(), Some(id.as_str()));

        // A torn first record, a patch that is not the root, and an ID not shaped like one.
        let torn = dir.join("torn.jsonl");
        write_private(&torn, b"{\"version\":2");
        assert_eq!(journal_workspace_id(&torn), None);
        let nested = dir.join("nested.jsonl");
        let record = json!({"version": 2, "patches": [
            {"op": "set", "path": ["workspace"], "value": {"id": id}}
        ]});
        write_private(&nested, format!("{record}\n").as_bytes());
        assert_eq!(journal_workspace_id(&nested), None);
        let shaped = dir.join("shaped.jsonl");
        let record = json!({"version": 1, "state": {"workspace": {"id": "../lab"}}});
        write_private(&shaped, format!("{record}\n").as_bytes());
        assert_eq!(journal_workspace_id(&shaped), None);
        // Another account could read a journal others can read; it is not taken as this one's.
        let open = dir.join("open.jsonl");
        let record = json!({"version": 1, "state": {"workspace": {"id": id}}});
        write_private(&open, format!("{record}\n").as_bytes());
        fs::set_permissions(&open, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(journal_workspace_id(&open), None);
        let _ = fs::remove_dir_all(&dir);
    }

    /// The relay of a bridge inside a full-access job answers `auth.account` and
    /// `device.revoke` itself with `agent_session_refused` and never forwards them; anything
    /// else still reaches the broker. Outside a job nothing is refused.
    #[test]
    fn a_relay_inside_a_job_never_forwards_device_changes() {
        use std::os::fd::FromRawFd;
        for inside_job in [true, false] {
            let mut fds = [0; 2];
            assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
            let (input, mut daemon) =
                unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) };
            let (bridge_side, broker_side) = UnixStream::pair().unwrap();
            let seen = Arc::new(Mutex::new(Vec::<String>::new()));
            let record = seen.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(broker_side.try_clone().unwrap());
                let mut writer = broker_side;
                while let Ok(Some(frame)) = read_frame(&mut reader) {
                    let request: Value = serde_json::from_slice(&frame).unwrap();
                    record
                        .lock()
                        .unwrap()
                        .push(request["method"].as_str().unwrap_or_default().to_owned());
                    let answer = json!({"id": request["id"], "result": {"ok": true}});
                    writer.write_all(format!("{answer}\n").as_bytes()).unwrap();
                }
            });
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let mut input = BufReader::new(input);
                let reader = BufReader::new(bridge_side.try_clone().unwrap());
                let mut output = Vec::new();
                let reconnect: crate::remote_full::Connect =
                    Arc::new(|| bail!("no second broker connection in this test"));
                let result = relay_from(
                    &mut input,
                    &mut output,
                    bridge_side,
                    reader,
                    &reconnect,
                    inside_job,
                );
                let _ = sender.send((result.is_ok(), output));
            });
            for (id, method) in [
                ("a", "auth.account"),
                ("b", "device.revoke"),
                ("c", "workspace.snapshot"),
            ] {
                let request = json!({"version": 1, "id": id, "method": method, "params": {}});
                daemon.write_all(format!("{request}\n").as_bytes()).unwrap();
            }
            drop(daemon);
            let (ended, output) = receiver
                .recv_timeout(Duration::from_secs(10))
                .expect("the relay ends when the daemon closes its end");
            assert!(ended);
            let answers: Vec<Value> = output
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
                .map(|line| serde_json::from_slice(line).unwrap())
                .collect();
            assert_eq!(answers.len(), 3);
            let forwarded = seen.lock().unwrap().clone();
            if inside_job {
                for answer in &answers[..2] {
                    assert_eq!(answer["error"]["code"], "agent_session_refused", "{answer}");
                    assert_eq!(answer["error"]["message"], AGENT_SESSION_REFUSED);
                }
                assert_eq!(answers[2]["result"]["ok"], true);
                assert_eq!(forwarded, vec!["workspace.snapshot".to_owned()]);
            } else {
                assert!(answers.iter().all(|answer| answer["result"]["ok"] == true));
                assert_eq!(
                    forwarded,
                    vec![
                        "auth.account".to_owned(),
                        "device.revoke".to_owned(),
                        "workspace.snapshot".to_owned()
                    ]
                );
            }
        }
    }

    #[test]
    fn the_restart_lock_admits_one_restart_at_a_time() {
        let store = private_temp("lock");
        let held = restart_lock(&store).unwrap();
        let error = restart_lock(&store)
            .err()
            .expect("a second restart waits its turn");
        assert!(error.to_string().starts_with("busy: "), "{error}");
        drop(held);
        restart_lock(&store).expect("free again once the first one ends");
        let _ = fs::remove_dir_all(&store);
    }
}
