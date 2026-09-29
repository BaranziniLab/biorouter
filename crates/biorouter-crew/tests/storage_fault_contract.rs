//! A disk that fills up, or fails, under a running broker (R-2).
//!
//! The broker stops saving changes the moment a journal write or sync fails (fail-stop: it can
//! no longer know what reached the disk), and a restart repairs the journal. What this pins is
//! that nobody is left guessing: the request that hit the fault and every change after it are
//! refused with one code, `storage_full` for a full disk or quota and `storage_failed` for any
//! other storage error, in a sentence rather than the operating system's text; `hello` reports
//! the stop; the host's `broker.log` says what happened and what to run; and an attachment whose
//! file cannot be written is refused the same way, without stopping the broker.
//!
//! The failure is injected at the journal call (`Broker::inject_journal_fault`, `test-seams`).
//! `journal_fault_contract.rs` drives the same path with a real failing syscall under an
//! `LD_PRELOAD` interposer. Linux only, as `hello` needs the node identity.
#![cfg(target_os = "linux")]

mod support;

use biorouter_crew::{Connection, JournalCall, Request};
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use support::*;

const MALLORY: u32 = 72_001;

const FULL_NOT_SAVED: &str = "The workspace server is out of disk space, so this change was not saved. Reading still works. Ask the host to free space on the server and restart Crew.";
const FULL_STOPPED: &str = "The workspace server ran out of disk space and has stopped saving changes. Reading still works. Ask the host to free space on the server and restart Crew.";
const FAILED_UNCERTAIN: &str = "The workspace server could not confirm this change was saved to disk, so it may not have been saved. Reading still works. Ask the host to check the server's storage and restart Crew.";
const FAILED_STOPPED: &str = "The workspace server could not save a change to disk and has stopped saving changes. Reading still works. Ask the host to check the server's storage and restart Crew.";

fn hello(ws: &mut Workspace) -> Value {
    let mut connection = Connection::new();
    let response = ws.broker.handle(
        host_uid(),
        &mut connection,
        Request {
            version: 1,
            id: "hello".into(),
            method: "hello".into(),
            params: json!({}),
            auth: None,
            credential: None,
        },
    );
    ok(response)
}

fn history(ws: &mut Workspace, channel: &str) -> Vec<String> {
    ws.host_ok("messages.history", json!({"channel_id": channel}))["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["body"].as_str().unwrap().to_owned())
        .collect()
}

fn log_lines(ws: &Workspace) -> Vec<String> {
    fs::read_to_string(ws.root.path().join("broker.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn begin(channel: &str) -> Value {
    json!({
        "channel_id": channel,
        "size": 3,
        "sha256": digest(b"abc"),
        "name": "notes.txt",
        "media_type": "text/plain",
    })
}

fn blob_files(ws: &Workspace) -> usize {
    fs::read_dir(ws.root.path().join("blobs")).unwrap().count()
}

#[test]
fn a_full_disk_stops_saving_and_every_refusal_says_so_in_one_code() {
    let mut ws = Workspace::new("storage-full");
    let mut mallory = ws.enroll(MALLORY, "mallory", 31);
    let (team, general) = ws.host_team("lab");
    ws.host_adds_to_team(&mut mallory, &team);
    ws.host_ok(
        "message.post",
        json!({"channel_id": general, "body": "saved", "idempotency_key": "saved"}),
    );
    assert_eq!(hello(&mut ws)["state"], "running");

    ws.broker
        .inject_journal_fault(JournalCall::Write, libc::ENOSPC);
    // The request that hit the fault: its record was never written, and it says so.
    let (code, message) = refused(ws.host_call(
        "message.post",
        json!({"channel_id": general, "body": "lost"}),
    ));
    assert_eq!(code, "storage_full");
    assert_eq!(message, format!("storage_full: {FULL_NOT_SAVED}"));
    assert!(!message.contains("os error"), "{message}");

    // Every change after it, from anyone, is refused with the same code, before anything is
    // touched: an attachment leaves no file behind.
    let files = blob_files(&ws);
    for response in [
        ws.call(
            &mut mallory,
            "message.post",
            json!({"channel_id": general, "body": "later"}),
        ),
        ws.call(&mut mallory, "blob.begin", begin(&general)),
        ws.host_call("policy.set", json!({"mode": "public"})),
    ] {
        assert_eq!(
            refused(response),
            (
                "storage_full".into(),
                format!("storage_full: {FULL_STOPPED}")
            )
        );
    }
    assert_eq!(blob_files(&ws), files, "a refused upload left a file");

    // Reading still works, and a change acknowledged before the fault still replays.
    assert_eq!(history(&mut ws, &general), vec!["saved"]);
    ws.snapshot(&mut mallory);
    ws.host_ok(
        "message.post",
        json!({"channel_id": general, "body": "saved", "idempotency_key": "saved"}),
    );

    // Anyone who asks the broker learns it has stopped, in words a member can act on.
    let answer = hello(&mut ws);
    assert_eq!(answer["state"], "storage_failed");
    assert_eq!(answer["storage"]["code"], "storage_full");
    assert_eq!(answer["storage"]["message"], FULL_STOPPED);
    assert!(answer["storage"]["since"].as_u64().unwrap() > 0);

    // The host's log has one line: when, the OS's error, how much of the journal is kept,
    // and the two commands that bring the workspace back.
    let lines = log_lines(&ws);
    assert_eq!(lines.len(), 1, "{lines:#?}");
    let line = &lines[0];
    let root = fs::canonicalize(ws.root.path()).unwrap();
    for expected in [
        "storage_full: Crew stopped saving changes: ".to_owned(),
        "(os error 28)".to_owned(),
        "journal.jsonl held ".to_owned(),
        format!("biorouter-crew stop --state-dir {}", root.display()),
        format!("biorouter-crew start --state-dir {}", root.display()),
        "Free space on this server".to_owned(),
    ] {
        assert!(line.contains(&expected), "{expected:?} missing from {line}");
    }
    assert!(
        line.starts_with("20"),
        "the line starts with its UTC time: {line}"
    );

    // A restart repairs the journal and saves again; what was refused was never saved.
    let mut ws = ws.reopen();
    assert_eq!(hello(&mut ws)["state"], "running");
    ws.host_ok(
        "message.post",
        json!({"channel_id": general, "body": "after restart"}),
    );
    assert_eq!(history(&mut ws, &general), vec!["saved", "after restart"]);
}

#[test]
fn a_failed_sync_says_the_change_may_not_have_been_saved_and_a_retry_never_duplicates_it() {
    let mut ws = Workspace::new("storage-sync");
    let (_, general) = ws.host_team("lab");
    ws.broker.inject_journal_fault(JournalCall::Sync, libc::EIO);
    let params = json!({"channel_id": general, "body": "uncertain", "idempotency_key": "once"});
    assert_eq!(
        refused(ws.host_call("message.post", params.clone())),
        (
            "storage_failed".into(),
            format!("storage_failed: {FAILED_UNCERTAIN}")
        )
    );
    assert_eq!(
        refused(ws.host_call("team.create", json!({"name": "other"}))),
        (
            "storage_failed".into(),
            format!("storage_failed: {FAILED_STOPPED}")
        )
    );
    let answer = hello(&mut ws);
    assert_eq!(answer["state"], "storage_failed");
    assert_eq!(answer["storage"]["code"], "storage_failed");
    let lines = log_lines(&ws);
    assert_eq!(lines.len(), 1, "{lines:#?}");
    assert!(lines[0].contains("storage_failed: Crew stopped saving changes: "));
    assert!(lines[0].contains("Check this server's storage"));

    // The record did reach the file here, so the restart keeps it; sending the same request
    // again is answered from it, never posted twice.
    let mut ws = ws.reopen();
    ws.host_ok("message.post", params);
    assert_eq!(history(&mut ws, &general), vec!["uncertain"]);
}

#[test]
fn a_log_line_the_full_disk_refused_is_written_once_there_is_space() {
    let mut ws = Workspace::new("storage-log-retry");
    let (_, general) = ws.host_team("lab");
    // broker.log cannot be created while the state directory refuses new files, as a full
    // disk refuses them.
    let root = ws.root.path().to_path_buf();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o500)).unwrap();
    ws.broker
        .inject_journal_fault(JournalCall::Write, libc::EDQUOT);
    let (code, _) = refused(ws.host_call(
        "message.post",
        json!({"channel_id": general, "body": "lost"}),
    ));
    assert_eq!(code, "storage_full", "a full quota is a full disk");
    let writable = fs::File::create(root.join("probe")).is_ok();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    if writable {
        // Running as root: permissions refuse nothing, so there was nothing to retry.
        return;
    }
    assert!(log_lines(&ws).is_empty());
    // The next request, of any kind, tries again, and the line lands once.
    hello(&mut ws);
    hello(&mut ws);
    let lines = log_lines(&ws);
    assert_eq!(lines.len(), 1, "{lines:#?}");
    assert!(lines[0].contains("storage_full: Crew stopped saving changes: "));
}

#[test]
fn an_attachment_that_cannot_be_written_is_refused_in_words_and_the_broker_keeps_saving() {
    let mut ws = Workspace::new("storage-blob");
    let mut mallory = ws.enroll(MALLORY, "mallory", 31);
    let (team, general) = ws.host_team("lab");
    ws.host_adds_to_team(&mut mallory, &team);
    let blob = ws.call_ok(&mut mallory, "blob.begin", begin(&general));
    let blob_id = blob["id"].as_str().unwrap().to_owned();
    let file = ws.root.path().join("blobs").join(&blob_id);
    // The attachment's file refuses the write, as a full or failing disk would.
    fs::set_permissions(&file, fs::Permissions::from_mode(0o400)).unwrap();
    let chunk = json!({"blob_id": blob_id, "offset": 0, "data_hex": hex::encode(b"abc")});
    let response = ws.call(&mut mallory, "blob.chunk", chunk.clone());
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    if response.error.is_none() {
        // Running as root: permissions refuse nothing.
        return;
    }
    let (code, message) = refused(response);
    assert_eq!(code, "storage_failed");
    assert_eq!(
        message,
        "storage_failed: The workspace server could not read or write its storage. Ask the host to check the server's storage, then try again."
    );
    // Nothing was recorded, so the broker is still saving and the same chunk goes through.
    assert_eq!(hello(&mut ws)["state"], "running");
    ws.call_ok(&mut mallory, "blob.chunk", chunk);
    ws.call_ok(&mut mallory, "blob.finish", json!({"blob_id": blob_id}));
    ws.host_ok(
        "message.post",
        json!({"channel_id": general, "body": "still saving"}),
    );
}
