//! The broker's runtime directory and its lifecycle commands: `status` and `stop` never create
//! state, an empty descriptor left by an older release does not block a start, a runtime path
//! another account took after `/tmp` was cleaned moves to a fresh one instead of refusing every
//! start, and decoy directories cannot hide a running sibling from the workspace-name check.
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
