//! Server commands addressed by workspace ID, and creating a workspace only on purpose
//! (account ownership, AO-7), plus the bridge's refusal to add or revoke a device from inside a
//! full-access job (the Security position's accident guard).
//!
//! What runs anywhere: an unknown ID is `not_found` and creates nothing, no journal is created
//! without `--bootstrap-key` or `--create`, an empty bootstrap key is stored as the empty
//! string, the restart lock, and the command line's refusals. What needs Linux: a broker that
//! will not stop (`stop_timeout`), a new program that will not start (`start_failed`), the
//! bridge's refusal, and (with `/etc/machine-id`, as a real broker needs) a whole restart.
#![cfg(unix)]

mod support;

use biorouter_crew::Broker;
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use support::*;

const BINARY: &str = env!("CARGO_BIN_EXE_biorouter-crew");

/// A home directory with the workspace store in it, as setup leaves one.
fn home(label: &str) -> (TempRoot, PathBuf) {
    let home = TempRoot::new(label);
    let store = home.path().join(".local/share/biorouter-crew");
    fs::create_dir_all(&store).unwrap();
    for dir in [
        home.path().join(".local"),
        home.path().join(".local/share"),
        store.clone(),
    ] {
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    }
    (home, store)
}

fn directory() -> FakeDirectory {
    let directory = FakeDirectory::default();
    directory.set(host_uid(), "alice", Some("Alice Chen"));
    directory
}

/// A stopped workspace in `store/folder`, created with a bootstrap key; its ID.
fn stored(store: &Path, folder: &str) -> String {
    let broker =
        Broker::open_with_directory(&store.join(folder), &key_hex(&key(7)), directory().boxed())
            .unwrap();
    broker.workspace().id.clone()
}

/// Every path under `root`, to show nothing was created.
fn paths(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        if fs::symlink_metadata(&path).unwrap().is_dir() {
            for child in fs::read_dir(&path).unwrap() {
                stack.push(child.unwrap().path());
            }
        }
        found.push(path);
    }
    found.sort();
    found
}

fn refusal(result: anyhow::Result<Value>) -> String {
    format!("{:#}", result.expect_err("refused"))
}

fn crew(args: &[&str], home: &Path) -> std::process::Output {
    std::process::Command::new(BINARY)
        .args(args)
        .env("HOME", home)
        .output()
        .unwrap()
}

#[test]
fn an_unknown_id_is_not_found_and_nothing_is_created() {
    let (home, store) = home("by-id-unknown");
    stored(&store, "lab");
    let before = paths(home.path());
    let unknown = uuid();
    let error = refusal(Broker::start_by_id(home.path(), &unknown));
    assert!(error.starts_with("not_found: "), "{error}");
    assert!(error.contains(&unknown), "{error}");
    let error = refusal(Broker::restart_by_id_with(
        home.path(),
        &unknown,
        Duration::from_secs(1),
    ));
    assert!(error.starts_with("not_found: "), "{error}");
    let error = refusal(Broker::start_by_id(home.path(), "lab"));
    assert!(error.starts_with("invalid_params: "), "{error}");
    let error = refusal(Broker::restart_by_id_with(
        home.path(),
        "../lab",
        Duration::from_secs(1),
    ));
    assert!(error.starts_with("invalid_params: "), "{error}");
    assert_eq!(paths(home.path()), before, "nothing was created");

    // An account with no workspace store at all: still not_found, and no store appears.
    let bare = TempRoot::new("by-id-bare");
    let error = refusal(Broker::start_by_id(bare.path(), &unknown));
    assert!(error.starts_with("not_found: "), "{error}");
    assert!(!bare.path().join(".local").exists());
}

/// A renamed workspace keeps its folder: the ID, never the folder's name, finds it.
#[test]
fn the_id_finds_the_folder_whatever_it_is_called() {
    let (home, store) = home("by-id-folder");
    let lab = stored(&store, "first-name");
    let other = stored(&store, "lab");
    // Folders that are not workspaces, or not this account's to use, are passed over.
    fs::create_dir(store.join("empty")).unwrap();
    std::os::unix::fs::symlink(store.join("first-name"), store.join("alias")).unwrap();
    // Another restart holds the lock, as the broker makes it: a private file.
    let held = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(store.join(".restart-lock"))
            .unwrap()
    };
    assert_eq!(
        unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&held), libc::LOCK_EX) },
        0
    );
    // The lock is taken after the folder is found, so it answers busy for a known ID only.
    let error = refusal(Broker::restart_by_id_with(
        home.path(),
        &lab,
        Duration::from_secs(1),
    ));
    assert!(error.starts_with("busy: "), "{error}");
    let error = refusal(Broker::restart_by_id_with(
        home.path(),
        &uuid(),
        Duration::from_secs(1),
    ));
    assert!(error.starts_with("not_found: "), "{error}");
    drop(held);
    assert_ne!(lab, other);
}

/// Nothing is stopped when there is no installed program to start in its place, and the code
/// says so: `start_failed` is kept for a broker that was stopped and did not start again.
#[test]
fn a_restart_without_an_installed_program_leaves_the_workspace_alone() {
    let (home, store) = home("restart-no-program");
    let id = stored(&store, "lab");
    let before = paths(&store.join("lab"));
    let error = refusal(Broker::restart_by_id_with(
        home.path(),
        &id,
        Duration::from_secs(1),
    ));
    assert!(error.starts_with("program_missing: "), "{error}");
    assert!(error.contains(".local/bin/biorouter-crew"), "{error}");
    assert!(
        error.ends_with("so the running broker was left as it is."),
        "{error}"
    );
    assert_eq!(paths(&store.join("lab")), before);
}

/// A copy of a workspace's folder kept in the store holds the same ID. Neither `start` nor
/// `restart` guesses which folder is the workspace: both refuse, naming the folders, and
/// nothing is started, stopped or written.
#[test]
fn an_id_two_folders_hold_is_refused_and_nothing_is_touched() {
    let (home, store) = home("by-id-copies");
    let id = stored(&store, "lab");
    copy_tree(&store.join("lab"), &store.join("backup-lab"));
    let before = paths(home.path());
    let start = refusal(Broker::start_by_id(home.path(), &id));
    let restart = refusal(Broker::restart_by_id_with(
        home.path(),
        &id,
        Duration::from_secs(1),
    ));
    for error in [&start, &restart] {
        assert!(error.starts_with("identity_conflict: "), "{error}");
        assert!(error.contains(&id), "{error}");
        assert!(error.contains("(backup-lab, lab)"), "{error}");
    }
    assert_eq!(paths(home.path()), before, "nothing was started or written");
    // With the copy moved out, the ID finds its folder again.
    fs::remove_dir_all(store.join("backup-lab")).unwrap();
    let error = refusal(Broker::restart_by_id_with(
        home.path(),
        &id,
        Duration::from_secs(1),
    ));
    assert!(error.starts_with("program_missing: "), "{error}");
}

/// The program setup installs, as a script that says why it cannot start and exits.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn install_failing_program(home: &Path) {
    let bin = home.join(".local/bin");
    fs::create_dir_all(&bin).unwrap();
    let program = bin.join("biorouter-crew");
    fs::write(
        &program,
        b"#!/bin/sh\necho 'boom: the new program could not start' >&2\nexit 3\n",
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn no_journal_is_created_without_a_key_or_create() {
    let parent = TempRoot::new("no-create");
    let empty = parent.path().join("empty");
    fs::create_dir(&empty).unwrap();
    fs::set_permissions(&empty, fs::Permissions::from_mode(0o700)).unwrap();
    let missing = parent.path().join("missing");

    for root in [&empty, &missing] {
        let error = refusal(Broker::start_state_dir(root, "", None, false));
        assert!(error.starts_with("not_found: "), "{error}");
        assert!(error.contains("--create"), "{error}");
        let error = Broker::open_with_directory(root, "", directory().boxed())
            .err()
            .expect("no key and no journal");
        assert!(format!("{error:#}").starts_with("not_found: "), "{error:#}");
    }
    assert!(!missing.exists(), "not even the directory is created");
    assert!(
        fs::read_dir(&empty).unwrap().next().is_none(),
        "nothing is written into an empty state directory"
    );
    let error = refusal(Broker::start_state_dir(
        &parent.path().join("named"),
        "",
        Some("lab"),
        false,
    ));
    assert!(error.starts_with("not_found: "), "{error}");
    assert!(!parent.path().join("named").exists());
}

/// `--create` without a key stores the key as the empty string, never null or absent, so an
/// older broker still replays the journal; the workspace then reopens without a key, and
/// `auth.bootstrap` admits nobody.
#[test]
fn a_workspace_created_without_a_key_stores_it_empty() {
    let parent = TempRoot::new("create-empty-key");
    let root = parent.path().join("lab");
    let broker = Broker::create_with_directory(&root, directory().boxed()).unwrap();
    let id = broker.workspace().id.clone();
    let state = broker.state_json();
    assert_eq!(state["bootstrap_key"], json!(""), "{state:#}");
    drop(broker);
    let journal = fs::read_to_string(root.join("journal.jsonl")).unwrap();
    assert!(journal.contains("\"bootstrap_key\":\"\""), "{journal}");

    let mut broker = Broker::open_with_directory(&root, "", directory().boxed()).unwrap();
    assert_eq!(
        broker.workspace().id,
        id,
        "an existing journal opens without a key"
    );
    let host_key = key(7);
    let public_key = key_hex(&host_key);
    let mut host = Member::new(host_uid(), host_key);
    let (code, _) = refused(signed_raw(
        &mut broker,
        &mut host,
        "auth.bootstrap",
        json!({"public_key": public_key}),
    ));
    assert_eq!(code, "forbidden");
    drop(broker);
    // A key given later never re-creates or re-keys an existing workspace.
    let broker =
        Broker::open_with_directory(&root, &key_hex(&key(8)), directory().boxed()).unwrap();
    assert_eq!(broker.workspace().id, id);
    assert_eq!(broker.state_json()["bootstrap_key"], json!(""));
}

#[test]
fn the_command_line_addresses_a_workspace_one_way_at_a_time() {
    let (home, store) = home("by-id-cli");
    let id = stored(&store, "lab");
    let mixed = crew(
        &["restart", "--workspace-id", &id, "--name", "lab"],
        home.path(),
    );
    assert!(!mixed.status.success());
    assert!(
        String::from_utf8_lossy(&mixed.stderr)
            .contains("--workspace-id names the workspace by itself"),
        "{}",
        String::from_utf8_lossy(&mixed.stderr)
    );
    let bare = crew(&["restart"], home.path());
    assert!(!bare.status.success());
    assert!(String::from_utf8_lossy(&bare.stderr).contains("--workspace-id required"));
    let unknown = crew(&["start", "--workspace-id", &uuid()], home.path());
    assert!(!unknown.status.success());
    assert!(
        String::from_utf8_lossy(&unknown.stderr).contains("not_found: "),
        "{}",
        String::from_utf8_lossy(&unknown.stderr)
    );
    let create = crew(
        &[
            "status",
            "--create",
            "--state-dir",
            store.join("lab").to_str().unwrap(),
        ],
        home.path(),
    );
    assert!(!create.status.success());
    assert!(String::from_utf8_lossy(&create.stderr).contains("--create applies to serve and start"));
    // `start` on a folder that holds no workspace, without --create: refused before anything
    // is spawned, and nothing is written.
    let empty = store.join("empty");
    let started = crew(
        &["start", "--state-dir", empty.to_str().unwrap()],
        home.path(),
    );
    assert!(!started.status.success());
    assert!(
        String::from_utf8_lossy(&started.stderr).contains("not_found: "),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    assert!(!empty.exists());
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    /// Not a test: the process a restart test stops. It listens at `FAKE_BROKER_SOCKET` and
    /// answers every `hello` for `FAKE_BROKER_WORKSPACE`, ignoring SIGTERM when
    /// `FAKE_BROKER_IGNORE_TERM` is set. Without the variables it does nothing.
    #[test]
    #[ignore = "a helper process for the restart tests"]
    fn fake_broker_process() {
        let Some(socket) = std::env::var_os("FAKE_BROKER_SOCKET") else {
            return;
        };
        let workspace = std::env::var("FAKE_BROKER_WORKSPACE").unwrap();
        if std::env::var_os("FAKE_BROKER_IGNORE_TERM").is_some() {
            unsafe {
                libc::signal(libc::SIGTERM, libc::SIG_IGN);
            }
        }
        let listener = UnixListener::bind(socket).unwrap();
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let mut writer = stream.try_clone().unwrap();
            let mut line = String::new();
            if BufReader::new(stream).read_line(&mut line).unwrap_or(0) > 0 {
                let request: Value = serde_json::from_str(&line).unwrap_or_default();
                let answer = json!({"id": request["id"], "result": {
                    "workspace_id": workspace, "state": "running", "version": "0.0.1"}});
                let _ = writer.write_all(format!("{answer}\n").as_bytes());
            }
        }
    }

    /// Start [`fake_broker_process`] as a process of its own at `socket`, wait until it
    /// listens, and record it in `state_dir`'s `runtime.json` as `workspace_id`'s broker.
    fn running_fake(
        state_dir: &Path,
        socket: &Path,
        workspace_id: &str,
        ignore_term: bool,
    ) -> Child {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "linux::fake_broker_process",
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads",
                "1",
            ])
            .env("FAKE_BROKER_SOCKET", socket)
            .env("FAKE_BROKER_WORKSPACE", workspace_id)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if ignore_term {
            command.env("FAKE_BROKER_IGNORE_TERM", "1");
        }
        let child = command.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while UnixStream::connect(socket).is_err() {
            assert!(Instant::now() < deadline, "the fake broker never listened");
            std::thread::sleep(Duration::from_millis(50));
        }
        let info = json!({
            "pid": child.id(),
            "socket": socket,
            "workspace_id": workspace_id,
            "host_uid": host_uid(),
            "protocol": 1,
        });
        let path = state_dir.join("runtime.json");
        fs::write(&path, serde_json::to_vec(&info).unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        child
    }

    fn alive(pid: u32) -> bool {
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    #[test]
    fn a_broker_that_does_not_stop_in_time_is_not_restarted() {
        let (home, store) = home("restart-timeout");
        let id = stored(&store, "lab");
        let runtime = TempRoot::short();
        let socket = runtime.path().join("broker.sock");
        let mut child = running_fake(&store.join("lab"), &socket, &id, true);
        install_failing_program(home.path());
        let started = Instant::now();
        let error = refusal(Broker::restart_by_id_with(
            home.path(),
            &id,
            Duration::from_secs(2),
        ));
        assert!(error.starts_with("stop_timeout: "), "{error}");
        assert!(started.elapsed() >= Duration::from_secs(2));
        assert!(alive(child.id()), "nothing was killed");
        // Nothing was started in its place.
        assert!(!store.join("lab").join("broker.log").exists());
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn a_new_program_that_does_not_start_is_start_failed_with_the_last_log_line() {
        let (home, store) = home("restart-start-failed");
        let id = stored(&store, "lab");
        let runtime = TempRoot::short();
        let socket = runtime.path().join("broker.sock");
        let mut child = running_fake(&store.join("lab"), &socket, &id, false);
        install_failing_program(home.path());
        let error = refusal(Broker::restart_by_id_with(
            home.path(),
            &id,
            Duration::from_secs(10),
        ));
        assert!(error.starts_with("start_failed: "), "{error}");
        assert!(
            error.contains("boom: the new program could not start"),
            "{error}"
        );
        // The old broker was stopped: its process has exited.
        let deadline = Instant::now() + Duration::from_secs(5);
        while child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "the old broker is still running");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// A fake broker in this process that answers `hello` for `workspace_id` and `{ok: true}`
    /// to anything else, recording every method it receives.
    fn recording_broker(
        directory: &Path,
        workspace_id: &str,
    ) -> (PathBuf, Arc<Mutex<Vec<String>>>) {
        let socket = directory.join("broker.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        let workspace_id = workspace_id.to_owned();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let (record, workspace_id) = (record.clone(), workspace_id.clone());
                std::thread::spawn(move || {
                    let mut writer = stream.try_clone().unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    while reader.read_line(&mut line).unwrap_or(0) > 0 {
                        let request: Value = serde_json::from_str(&line).unwrap();
                        let method = request["method"].as_str().unwrap_or_default().to_owned();
                        let result = if method == "hello" {
                            json!({"workspace_id": workspace_id})
                        } else {
                            json!({"ok": true})
                        };
                        record.lock().unwrap().push(method);
                        let answer = json!({"id": request["id"], "result": result});
                        if writer.write_all(format!("{answer}\n").as_bytes()).is_err() {
                            return;
                        }
                        line.clear();
                    }
                });
            }
        });
        (socket, seen)
    }

    /// Run `biorouter-crew bridge` against `socket`, inside a job when `job` names one, send
    /// it `methods` in turn and return its answers.
    fn bridge(
        socket: &Path,
        workspace_id: &str,
        job: Option<&str>,
        methods: &[&str],
    ) -> Vec<Value> {
        let mut command = Command::new(BINARY);
        command
            .args([
                "bridge",
                "--stdio",
                "--socket",
                socket.to_str().unwrap(),
                "--owner-uid",
                &host_uid().to_string(),
                "--workspace-id",
                workspace_id,
            ])
            .env_remove("BIOROUTER_CREW_JOB_ID")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(job) = job {
            command.env("BIOROUTER_CREW_JOB_ID", job);
        }
        let mut child = command.spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut answers = Vec::new();
        for (index, method) in methods.iter().enumerate() {
            let request = json!({"version": 1, "id": format!("r{index}"), "method": method,
                                 "params": {"public_key": "00"}});
            stdin.write_all(format!("{request}\n").as_bytes()).unwrap();
            let mut line = String::new();
            stdout.read_line(&mut line).unwrap();
            answers.push(serde_json::from_str(&line).unwrap_or_else(|_| {
                let mut stderr = String::new();
                let _ = child.stderr.take().unwrap().read_to_string(&mut stderr);
                panic!("no answer to {method}: {line:?} {stderr}")
            }));
        }
        drop(stdin);
        assert!(child.wait().unwrap().success());
        answers
    }

    /// A bridge started inside a full-access job (its environment carries the job's ID, which
    /// the job supervisor sets) answers `auth.account` and `device.revoke` itself with
    /// `agent_session_refused`, and they never reach the broker; anything else still does. A
    /// bridge outside a job forwards them.
    #[test]
    fn a_bridge_inside_an_agent_job_never_adds_or_revokes_a_device() {
        let runtime = TempRoot::short();
        let workspace_id = uuid();
        let (socket, seen) = recording_broker(runtime.path(), &workspace_id);
        let methods = ["auth.account", "device.revoke", "workspace.snapshot"];

        let answers = bridge(&socket, &workspace_id, Some("job-1"), &methods);
        for answer in &answers[..2] {
            assert_eq!(answer["error"]["code"], "agent_session_refused", "{answer}");
            assert_eq!(
                answer["error"]["message"],
                "agent_session_refused: An agent's job on the server cannot add or remove your computers. Do this from Biorouter on your computer."
            );
        }
        assert_eq!(answers[2]["result"]["ok"], true);
        assert_eq!(
            *seen.lock().unwrap(),
            vec!["hello".to_owned(), "workspace.snapshot".to_owned()],
            "the refused methods never reached the broker"
        );

        seen.lock().unwrap().clear();
        let answers = bridge(&socket, &workspace_id, None, &methods);
        assert!(
            answers.iter().all(|answer| answer["result"]["ok"] == true),
            "{answers:?}"
        );
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                "hello".to_owned(),
                "auth.account".to_owned(),
                "device.revoke".to_owned(),
                "workspace.snapshot".to_owned()
            ]
        );
    }

    /// Start, restart and start again by ID against real brokers: needs an ordinary user and
    /// `/etc/machine-id`, as a real broker does.
    #[test]
    fn restart_runs_the_installed_program_and_says_which_versions() {
        if host_uid() == 0 || !Path::new("/etc/machine-id").exists() {
            eprintln!("skipped: a real broker needs an ordinary user and /etc/machine-id");
            return;
        }
        let (home, store) = home("restart-real");
        let bin = home.path().join(".local/bin");
        fs::create_dir_all(&bin).unwrap();
        fs::copy(BINARY, bin.join("biorouter-crew")).unwrap();
        let json_of = |output: std::process::Output| -> Value {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            serde_json::from_slice(&output.stdout).unwrap()
        };
        let state_dir = store.join("lab");
        let state = state_dir.to_str().unwrap();
        let started = json_of(crew(
            &["start", "--state-dir", state, "--create"],
            home.path(),
        ));
        assert_eq!(started["state"], "running", "{started:#}");
        assert_eq!(started["created"], true);
        let id = started["workspace_id"].as_str().unwrap().to_owned();
        let runtime: Value =
            serde_json::from_slice(&fs::read(state_dir.join("runtime.json")).unwrap()).unwrap();
        assert_eq!(runtime["version"], env!("CARGO_PKG_VERSION"));

        let restarted = json_of(crew(&["restart", "--workspace-id", &id], home.path()));
        assert_eq!(
            restarted,
            json!({"restarted": true, "from_version": env!("CARGO_PKG_VERSION"),
                   "to_version": env!("CARGO_PKG_VERSION")})
        );
        let listed = Broker::list_workspaces(home.path());
        let lab = listed["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["workspace_id"] == id.as_str())
            .unwrap_or_else(|| panic!("{listed:#}"));
        assert_eq!(lab["state"], "running");
        assert_eq!(lab["binary_current"], true, "{lab:#}");

        json_of(crew(&["stop", "--state-dir", state], home.path()));
        let again = json_of(crew(&["start", "--workspace-id", &id], home.path()));
        assert_eq!(again["state"], "running");
        assert_eq!(again["created"], false);
        json_of(crew(&["stop", "--state-dir", state], home.path()));
    }
}
