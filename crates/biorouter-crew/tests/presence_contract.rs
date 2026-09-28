//! Presence (M18): who is online, as a snapshot's `online_principal_ids`.
//!
//! A person is online while a connection their signed device spoke on is open (a member's
//! bridge holds one for as long as their computer is connected), and for a window after their
//! last signed request. Presence lives in the broker's memory only: it is never journaled,
//! costs the state nothing, and no authorization reads it. An agent's grant is not a person
//! being there.
#![cfg(unix)]

mod support;

use biorouter_crew::Connection;
use serde_json::{json, Value};
use std::time::Duration;
use support::*;

const BOB: u32 = 73_001;
const CAROL: u32 = 73_002;

/// A run's parameters, as `quota_contract.rs` builds them.
fn run_params(ws: &Workspace, channel: &str) -> Value {
    json!({
        "channel_id": channel,
        "source_channels": [channel],
        "provider_policy_id": "private",
        "personal_mode": "private",
        "public_provider": false,
        "expires_in": 600,
        "expected_workspace_policy_epoch": ws.broker.workspace().policy_epoch,
        "expected_protected_context": true,
        "workspace_institution_id": "ucsf",
        "connection_institution_id": "ucsf",
        "provider_affiliation": {"kind": "local"},
    })
}

fn online(snapshot: &Value) -> Vec<String> {
    let mut ids: Vec<String> = snapshot["online_principal_ids"]
        .as_array()
        .expect("the snapshot lists who is online")
        .iter()
        .map(|id| id.as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    ids
}

fn sorted(ids: &[&str]) -> Vec<String> {
    let mut ids: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
    ids.sort();
    ids
}

#[test]
fn a_person_is_online_while_connected_and_for_a_window_after_their_last_request() {
    let mut ws = Workspace::new("presence");
    ws.broker.set_presence_window(Duration::from_millis(300));
    let mut bob = ws.enroll(BOB, "bob", 41);
    let mut carol = ws.enroll(CAROL, "carol", 42);
    let host = ws.host.principal_id.clone();

    // Bob and Carol have both spoken on their connections: both are online, and so is the
    // host reading the snapshot.
    ws.snapshot(&mut bob);
    let snapshot = ws.snapshot(&mut carol);
    assert_eq!(
        online(&snapshot),
        sorted(&[&host, &bob.principal_id, &carol.principal_id])
    );

    // Carol's connection closes (her computer slept, her bridge ended) and she stays quiet
    // past the window: she is offline. Bob's connection is still open, so he stays online
    // however long he is idle, as a member whose app sits connected in the background.
    ws.broker.connection_closed(&mut carol.connection);
    std::thread::sleep(Duration::from_millis(450));
    let snapshot = ws.snapshot(&mut bob);
    assert_eq!(online(&snapshot), sorted(&[&host, &bob.principal_id]));

    // Within the window after a request, a person is online even with no connection open.
    ws.snapshot(&mut carol);
    ws.broker.connection_closed(&mut carol.connection);
    let snapshot = ws.host_snapshot();
    assert!(online(&snapshot).contains(&carol.principal_id));
    std::thread::sleep(Duration::from_millis(450));
    let snapshot = ws.host_snapshot();
    assert!(!online(&snapshot).contains(&carol.principal_id));

    // A connection closing twice, or one that never signed, changes nothing.
    ws.broker.connection_closed(&mut carol.connection);
    ws.broker.connection_closed(&mut Connection::new());
    let snapshot = ws.snapshot(&mut bob);
    assert_eq!(online(&snapshot), sorted(&[&host, &bob.principal_id]));
}

#[test]
fn presence_is_never_journaled_and_a_removed_member_is_never_online() {
    let mut ws = Workspace::new("presence-journal");
    let mut bob = ws.enroll(BOB, "bob", 41);
    let journal = ws.journal_bytes();
    let state = ws.broker.state_json();
    for _ in 0..3 {
        ws.snapshot(&mut bob);
    }
    assert_eq!(ws.journal_bytes(), journal, "a read wrote the journal");
    assert_eq!(ws.broker.state_json(), state, "presence reached the state");
    assert!(!serde_json::to_string(&state).unwrap().contains("online"));

    // Bob's connection is still open, but once he is removed he is not listed as online.
    ws.offboard(&bob.principal_id);
    let snapshot = ws.host_snapshot();
    assert!(!online(&snapshot).contains(&bob.principal_id));

    // A restart forgets presence: nobody is online until they speak again.
    let mut ws = ws.reopen();
    let host = ws.host.principal_id.clone();
    let snapshot = ws.host_snapshot();
    assert_eq!(online(&snapshot), vec![host]);
}

#[test]
fn an_agents_grant_does_not_make_its_owner_online() {
    let mut ws = Workspace::new("presence-agent");
    ws.broker.set_presence_window(Duration::from_millis(200));
    let mut bob = ws.enroll(BOB, "bob", 41);
    let (team, general) = ws.host_team("lab");
    ws.host_adds_to_team(&mut bob, &team);
    let params = run_params(&ws, &general);
    let credential = ws.call_ok(&mut bob, "run.create", params)["credential"]
        .as_str()
        .unwrap()
        .to_owned();
    ws.broker.connection_closed(&mut bob.connection);
    std::thread::sleep(Duration::from_millis(300));
    // Bob's agent keeps reading on its own connection while Bob is away.
    let mut worker = Connection::new();
    let read = ws.broker.handle(
        BOB,
        &mut worker,
        biorouter_crew::Request {
            version: 1,
            id: "worker".into(),
            method: "context.manifest".into(),
            params: json!({}),
            auth: None,
            credential: Some(credential),
        },
    );
    assert!(read.error.is_none(), "{:?}", read.error);
    let snapshot = ws.host_snapshot();
    assert!(!online(&snapshot).contains(&bob.principal_id));
}

/// A client learns from `hello` that this broker reports presence; an older broker sends no
/// `online_principal_ids`, which a client must read as unknown rather than as nobody online.
/// `hello` needs the node identity, so this runs on Linux.
#[cfg(target_os = "linux")]
#[test]
fn hello_advertises_presence() {
    let mut ws = Workspace::new("presence-hello");
    let hello = ok(ws.broker.handle(
        host_uid(),
        &mut Connection::new(),
        biorouter_crew::Request {
            version: 1,
            id: "hello".into(),
            method: "hello".into(),
            params: json!({}),
            auth: None,
            credential: None,
        },
    ));
    assert!(hello["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("presence_v1")));
}
