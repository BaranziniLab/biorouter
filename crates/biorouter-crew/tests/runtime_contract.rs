//! The broker's runtime directory and its lifecycle commands: `status` and `stop` never create
//! state, an empty descriptor left by an older release does not block a start, a runtime path
//! another account took after `/tmp` was cleaned moves to a fresh one instead of refusing every
//! start (and one only this account could ever have written into keeps its path), and decoy
//! directories cannot hide a running sibling from the workspace-name check. The bridge finding a
//! workspace that moved is tested beside it in `broker.rs`, since it needs the broker's
//! private helpers.
//!
//! Everything here runs on any Unix except what drives a real broker process (Linux only).
#![cfg(unix)]

mod support;

use serde_json::json;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use support::*;

const NODE: &str = "test-node";

/// A private file, as the broker writes its own.
fn private_write(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

/// The runtime descriptor `serve` writes after it binds `socket`.
fn write_descriptor(ws: &Workspace, socket: &Path) {
    let info = json!({
        "pid": std::process::id(),
        "socket": socket,
        "workspace_id": ws.broker.workspace().id,
        "host_uid": host_uid(),
        "protocol": 1,
        "node_id": NODE,
    });
    private_write(
        &ws.root.path().join("runtime.json"),
        &serde_json::to_vec(&info).unwrap(),
    );
}

// ---------------------------------------------------------------------------------------------
// BROKER-6: `status` and `stop` never create an empty runtime.json
// ---------------------------------------------------------------------------------------------

#[test]
fn status_without_a_descriptor_reports_not_running_and_creates_nothing() {
    let state = TempRoot::new("status-missing");
    let error = biorouter_crew::lifecycle("status", state.path(), "", None)
        .expect_err("nothing is running");
    assert!(error.to_string().starts_with("not_running:"), "{error}");
    assert!(
        !state.path().join("runtime.json").exists(),
        "status must not leave an empty descriptor behind"
    );

    let missing = state.path().join("never-created");
    let error = biorouter_crew::lifecycle("status", &missing, "", None).expect_err("no state");
    assert!(error.to_string().starts_with("not_running:"), "{error}");
    assert!(
        !missing.exists(),
        "status must not create the state directory"
    );
}

#[test]
fn status_reads_an_empty_descriptor_from_an_older_release_as_not_running() {
    let state = TempRoot::new("status-empty");
    private_write(&state.path().join("runtime.json"), b"");
    let error = biorouter_crew::lifecycle("status", state.path(), "", None)
        .expect_err("nothing is running");
    assert!(error.to_string().starts_with("not_running:"), "{error}");
}

#[cfg(target_os = "linux")]
#[test]
fn stop_without_a_descriptor_reports_not_running_and_creates_nothing() {
    let state = TempRoot::new("stop-missing");
    let error =
        biorouter_crew::lifecycle("stop", state.path(), "", None).expect_err("nothing is running");
    assert!(error.to_string().starts_with("not_running:"), "{error}");
    assert!(!state.path().join("runtime.json").exists());

    let missing = state.path().join("never-created");
    let error = biorouter_crew::lifecycle("stop", &missing, "", None).expect_err("no state");
    assert!(error.to_string().starts_with("not_running:"), "{error}");
    assert!(
        !missing.exists(),
        "stop must not create the state directory"
    );
}

/// A descriptor naming `socket` and `pid`, as a broker that has since stopped left it.
fn write_stale_descriptor(state: &Path, socket: &Path, pid: u32) {
    let info = json!({
        "pid": pid,
        "socket": socket,
        "workspace_id": "00000000-0000-4000-8000-000000000000",
        "host_uid": host_uid(),
        "protocol": 1,
        "node_id": NODE,
    });
    private_write(
        &state.join("runtime.json"),
        &serde_json::to_vec(&info).unwrap(),
    );
}

/// The pid of a process that has exited and been reaped.
fn exited_pid() -> u32 {
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    pid
}

/// `status`, and `stop` where it is supported, for `state`: each must say `not_running` and
/// leave the descriptor where it is, so the next `start` reclaims the same runtime path.
fn assert_not_running(state: &Path) {
    let before = fs::read(state.join("runtime.json")).unwrap();
    let mut commands = vec!["status"];
    if cfg!(target_os = "linux") {
        commands.push("stop");
    }
    for command in commands {
        let error =
            biorouter_crew::lifecycle(command, state, "", None).expect_err("nothing is running");
        assert_eq!(
            error.to_string(),
            "not_running: no broker is running from this state directory; start it with biorouter-crew start",
            "{command}"
        );
    }
    assert_eq!(fs::read(state.join("runtime.json")).unwrap(), before);
}

/// T3-BE-2: a broker that stopped cleanly, was killed, or whose server rebooted leaves its
/// runtime descriptor behind, and often its socket file. `status` and a second `stop` say
/// `not_running` for every way that looks: the process it names is gone, the socket file is
/// missing, or nothing listens on it. They used to print "Connection refused (os error 111)".
#[test]
fn status_and_stop_after_a_broker_stopped_report_not_running() {
    let state = TempRoot::new("status-stopped");
    let runtime = TempRoot::short();
    fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o711)).unwrap();
    let socket = runtime.path().join("broker.sock");

    // The process it names has exited.
    write_stale_descriptor(state.path(), &socket, exited_pid());
    assert_not_running(state.path());

    // The process is there (a pid the broker's could have been reused for), the socket is not.
    write_stale_descriptor(state.path(), &socket, std::process::id());
    assert_not_running(state.path());

    // The socket file is left, and nothing listens on it: connecting is refused.
    drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(socket.exists(), "a closed listener leaves its socket file");
    assert_not_running(state.path());
}

#[test]
fn an_empty_descriptor_does_not_block_the_next_start() {
    let mut ws = Workspace::new("empty-descriptor");
    let runtime = TempRoot::short();
    ws.broker.set_runtime_root(runtime.path());
    // What `status` or `stop` from an older release left behind when runtime.json was missing.
    private_write(&ws.root.path().join("runtime.json"), b"");
    let socket = ws
        .broker
        .prepare_runtime(NODE)
        .expect("an empty descriptor records nothing and must not read as corrupt");
    assert_eq!(socket.file_name().unwrap(), "broker.sock");
    assert_eq!(socket.parent().unwrap().parent().unwrap(), runtime.path());
}

/// The directory a runtime socket lives in, and its basename.
fn runtime_dir(socket: &Path) -> (PathBuf, String) {
    let directory = socket.parent().unwrap().to_path_buf();
    let basename = directory.file_name().unwrap().to_str().unwrap().to_owned();
    (directory, basename)
}

#[test]
fn the_runtime_directory_is_created_private_and_reused_across_restarts() {
    let mut ws = Workspace::new("runtime-reuse");
    let runtime = TempRoot::short();
    ws.broker.set_runtime_root(runtime.path());
    let socket = ws.broker.prepare_runtime(NODE).unwrap();
    let (directory, basename) = runtime_dir(&socket);
    assert!(basename.starts_with(&format!("crew-{}-", host_uid())));
    let metadata = fs::symlink_metadata(&directory).unwrap();
    assert!(metadata.is_dir());
    assert_eq!(metadata.uid(), host_uid());
    assert_eq!(metadata.mode() & 0o7777, 0o711);
    write_descriptor(&ws, &socket);

    // `/tmp` cleaned, nobody else took the name: the same path comes back.
    fs::remove_dir(&directory).unwrap();
    let mut ws = ws.reopen();
    ws.broker.set_runtime_root(runtime.path());
    assert_eq!(ws.broker.prepare_runtime(NODE).unwrap(), socket);
}

// ---------------------------------------------------------------------------------------------
// BROKER-3: a runtime path another account took after /tmp was cleaned moves, it never blocks
// ---------------------------------------------------------------------------------------------

/// The shapes an entry at the recorded runtime name can take when this account did not leave
/// it there as a usable runtime directory. Another account's directory is the case that
/// matters and cannot be made without root here; it takes the same path as these (and the
/// broker's own unit test drives it with a foreign owner).
fn squat(directory: &Path, shape: usize) {
    match shape {
        // A symbolic link, to a directory this account could otherwise use.
        0 => std::os::unix::fs::symlink("/", directory).unwrap(),
        // A plain file.
        1 => fs::write(directory, b"").unwrap(),
        // A directory anyone can write into.
        2 => {
            fs::create_dir(directory).unwrap();
            fs::set_permissions(directory, fs::Permissions::from_mode(0o777)).unwrap();
        }
        // The right directory, holding something that is not the broker's socket.
        _ => {
            fs::create_dir(directory).unwrap();
            fs::set_permissions(directory, fs::Permissions::from_mode(0o711)).unwrap();
            fs::write(directory.join("planted"), b"").unwrap();
        }
    }
}

#[test]
fn a_runtime_path_taken_after_tmp_was_cleaned_moves_to_a_fresh_private_one() {
    for shape in 0..4 {
        let mut ws = Workspace::new(&format!("runtime-squat-{shape}"));
        let runtime = TempRoot::short();
        ws.broker.set_runtime_root(runtime.path());
        let first = ws.broker.prepare_runtime(NODE).unwrap();
        write_descriptor(&ws, &first);
        let (directory, basename) = runtime_dir(&first);

        // /tmp is cleaned, and the recorded name is taken before the broker starts again.
        fs::remove_dir(&directory).unwrap();
        squat(&directory, shape);
        let mut ws = ws.reopen();
        ws.broker.set_runtime_root(runtime.path());
        let moved = ws
            .broker
            .prepare_runtime(NODE)
            .unwrap_or_else(|error| panic!("shape {shape}: the start must recover: {error}"));
        let (new_directory, new_basename) = runtime_dir(&moved);
        assert_ne!(new_basename, basename, "shape {shape}");
        assert!(new_basename.starts_with(&format!("crew-{}-", host_uid())));
        assert_eq!(new_directory.parent().unwrap(), runtime.path());
        let metadata = fs::symlink_metadata(&new_directory).unwrap();
        assert!(metadata.is_dir());
        assert_eq!(metadata.uid(), host_uid());
        assert_eq!(metadata.mode() & 0o7777, 0o711);
        assert!(
            fs::read_dir(&new_directory).unwrap().next().is_none(),
            "a fresh directory"
        );
        assert!(
            !ws.root.path().join("runtime.json").exists(),
            "the descriptor naming the old path is retired"
        );
        // The squatted entry is left alone.
        assert!(fs::symlink_metadata(&directory).is_ok());

        // The move is journaled: the next start comes back to the new path.
        let mut ws = ws.reopen();
        ws.broker.set_runtime_root(runtime.path());
        assert_eq!(
            ws.broker.prepare_runtime(NODE).unwrap(),
            moved,
            "shape {shape}"
        );
        let _ = fs::set_permissions(&directory, fs::Permissions::from_mode(0o700));
    }
}

#[test]
fn a_recorded_directory_only_this_account_could_write_into_keeps_its_path() {
    // Only this account and root can change the mode of this account's own directory, so with
    // any mode that never let another account write into it, nothing in it can be another
    // account's: the mode is put back and members keep the path they were given.
    for mode in [0o700, 0o755, 0o751] {
        let mut ws = Workspace::new(&format!("runtime-repair-{mode:o}"));
        let runtime = TempRoot::short();
        ws.broker.set_runtime_root(runtime.path());
        let socket = ws.broker.prepare_runtime(NODE).unwrap();
        write_descriptor(&ws, &socket);
        let (directory, _) = runtime_dir(&socket);
        fs::set_permissions(&directory, fs::Permissions::from_mode(mode)).unwrap();
        let mut ws = ws.reopen();
        ws.broker.set_runtime_root(runtime.path());
        assert_eq!(
            ws.broker.prepare_runtime(NODE).unwrap(),
            socket,
            "mode {mode:o}"
        );
        assert_eq!(
            fs::symlink_metadata(&directory).unwrap().mode() & 0o7777,
            0o711
        );
        assert!(
            ws.root.path().join("runtime.json").exists(),
            "nothing moved, so the descriptor stands"
        );
    }
    // A directory its group could write into is still moved, as one anyone can write into is.
    let mut ws = Workspace::new("runtime-repair-group");
    let runtime = TempRoot::short();
    ws.broker.set_runtime_root(runtime.path());
    let socket = ws.broker.prepare_runtime(NODE).unwrap();
    let (directory, _) = runtime_dir(&socket);
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o771)).unwrap();
    let mut ws = ws.reopen();
    ws.broker.set_runtime_root(runtime.path());
    assert_ne!(ws.broker.prepare_runtime(NODE).unwrap(), socket);
}

#[test]
fn a_live_listener_on_the_recorded_socket_is_still_refused() {
    let mut ws = Workspace::new("runtime-live");
    let runtime = TempRoot::short();
    ws.broker.set_runtime_root(runtime.path());
    let socket = ws.broker.prepare_runtime(NODE).unwrap();
    let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o666)).unwrap();
    let error = ws
        .broker
        .prepare_runtime(NODE)
        .expect_err("a live listener");
    assert!(error.to_string().starts_with("runtime_in_use:"), "{error}");
}

// ---------------------------------------------------------------------------------------------
// BROKER-5: decoy directories in /tmp cannot hide a running sibling from the name check
// ---------------------------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    /// A fake sibling broker at `runtime_root/basename`: answers every `hello` for
    /// `workspace_id` named `name`.
    fn fake_sibling(runtime_root: &Path, basename: &str, workspace_id: &str, name: &str) {
        let directory = runtime_root.join(basename);
        fs::create_dir(&directory).unwrap();
        let listener = UnixListener::bind(directory.join("broker.sock")).unwrap();
        let answer =
            json!({"id": "name-probe", "result": {"workspace_id": workspace_id, "name": name}});
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut line = String::new();
                if BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .is_ok()
                {
                    let _ = stream.write_all(format!("{answer}\n").as_bytes());
                }
            }
        });
    }

    #[test]
    fn decoys_that_sort_first_cannot_hide_a_running_sibling() {
        let mut ws = Workspace::new("sibling-decoys");
        let runtime = TempRoot::short();
        ws.broker.set_runtime_root(runtime.path());
        // The real sibling sorts last of everything named for this account.
        fake_sibling(
            runtime.path(),
            &format!("crew-{}-{}", host_uid(), "f".repeat(32)),
            &uuid(),
            "lab",
        );
        // Forty correctly named decoys that sort before it, of every shape another account
        // can leave in /tmp: an empty directory, a plain file, a symbolic link to the real
        // sibling's directory, and a directory whose broker.sock is not a socket.
        let real = runtime
            .path()
            .join(format!("crew-{}-{}", host_uid(), "f".repeat(32)));
        for index in 0..40u32 {
            let decoy = runtime
                .path()
                .join(format!("crew-{}-{index:032x}", host_uid()));
            match index % 4 {
                0 => fs::create_dir(&decoy).unwrap(),
                1 => fs::write(&decoy, b"").unwrap(),
                2 => std::os::unix::fs::symlink(&real, &decoy).unwrap(),
                _ => {
                    fs::create_dir(&decoy).unwrap();
                    fs::write(decoy.join("broker.sock"), b"").unwrap();
                }
            }
        }
        let (code, _) = refused(ws.host_call("workspace.rename", json!({"name": "lab"})));
        assert_eq!(
            code, "name_taken",
            "the running sibling must be probed whatever sorts before it"
        );
        assert_eq!(
            ws.host_ok("workspace.rename", json!({"name": "lab-two"}))["name"],
            "lab-two"
        );
    }
}
