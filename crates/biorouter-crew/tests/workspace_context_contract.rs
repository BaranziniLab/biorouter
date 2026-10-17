//! Broker contract for the workspace context document (`workspace_context_v1`, OW-8 of
//! `docs/crew/design/open-workspaces-and-automatic-setup.md`):
//!
//! - `workspace.context.set {text, expected_revision}` is a person's: the host's in a managed
//!   workspace, any member's in an open one; a stale revision is `revision_conflict`; at most
//!   64 KiB of text, with line breaks and tabs and no other control character; it counts toward
//!   its editor's share of the state and never moves the policy epoch;
//! - `workspace.context.get {}` and the snapshot's `context` serve it to every member, and to
//!   every agent's grant (the one new method in the worker allowlist), which can never set it;
//! - it survives a restart, and a journal without one replays as before.
//!
//! ```text
//! cargo test -p biorouter-crew --test workspace_context_contract
//! ```
#![cfg(unix)]

mod support;

use biorouter_crew::{Connection, Quotas, Request, Response};
use serde_json::{json, Value};
use support::*;

const BOB: u32 = 77_001;
const CAROL: u32 = 77_002;

const DRAFT: &str = "# leo.ucsf.edu\n\nUbuntu 22.04, 16 CPUs, 187 GiB.\n\n- Data: `/pool1/data`\n- Jobs:\trun `nohup` in `~/crew-work`\r\n";

fn epoch(ws: &Workspace) -> u64 {
    ws.broker.workspace().policy_epoch
}

fn journal_lines(ws: &Workspace) -> usize {
    String::from_utf8(ws.journal_bytes())
        .unwrap()
        .lines()
        .count()
}

fn set(text: &str, expected_revision: u64) -> Value {
    json!({"text": text, "expected_revision": expected_revision})
}

fn worker(ws: &mut Workspace, uid: u32, credential: &str, method: &str, params: Value) -> Response {
    ws.broker.handle(
        uid,
        &mut Connection::new(),
        Request {
            version: 1,
            id: format!("worker-{method}"),
            method: method.into(),
            params,
            auth: None,
            credential: Some(credential.into()),
        },
    )
}

/// A grant for `member` over `channel`, private or public.
fn grant(ws: &mut Workspace, member: &mut Member, channel: &str, public: bool) -> String {
    let params = if public {
        json!({
            "channel_id": channel, "source_channels": [channel],
            "provider_policy_id": "public", "personal_mode": "public",
            "public_provider": true, "expires_in": 600,
            "expected_workspace_policy_epoch": epoch(ws),
            "expected_protected_context": false,
            "workspace_institution_id": null, "connection_institution_id": null,
            "provider_affiliation": {"kind": "unstated"},
        })
    } else {
        json!({
            "channel_id": channel, "source_channels": [channel],
            "provider_policy_id": "private", "personal_mode": "private",
            "public_provider": false, "expires_in": 600,
            "expected_workspace_policy_epoch": epoch(ws),
            "expected_protected_context": true,
            "workspace_institution_id": "ucsf", "connection_institution_id": "ucsf",
            "provider_affiliation": {"kind": "local"},
        })
    };
    ws.call_ok(member, "run.create", params)["credential"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Bob, in the host's team `Lab`: `(bob, general)`.
fn with_bob(ws: &mut Workspace) -> (Member, String) {
    let bob = ws.enroll(BOB, "bob", 11);
    let (team, general) = ws.host_team("Lab");
    ws.host_ok(
        "team.add_member",
        json!({"team_id": team, "principal_id": bob.principal_id, "expected_username": "bob"}),
    );
    (bob, general)
}

#[test]
fn the_host_writes_it_in_a_managed_workspace_and_any_member_in_an_open_one() {
    let mut ws = Workspace::new("context-set");
    let (mut bob, _) = with_bob(&mut ws);
    assert_eq!(ws.snapshot(&mut bob)["context"], Value::Null);
    assert_eq!(
        ws.call_ok(&mut bob, "workspace.context.get", json!({})),
        json!({"context": null})
    );
    let before = epoch(&ws);

    let (code, message) = refused(ws.call(&mut bob, "workspace.context.set", set("mine", 0)));
    assert_eq!(
        (code.as_str(), message.as_str()),
        ("forbidden", "forbidden: workspace host device required")
    );
    let written = ws.host_ok("workspace.context.set", set(DRAFT, 0));
    assert_eq!(written["revision"], 1);
    let updated_at = written["updated_at"].as_u64().unwrap();
    assert!(updated_at.abs_diff(now()) < 60);
    let context = json!({
        "text": DRAFT, "revision": 1,
        "updated_by": ws.host.principal_id, "updated_at": updated_at,
    });
    assert_eq!(
        ws.call_ok(&mut bob, "workspace.context.get", json!({})),
        json!({"context": context})
    );
    assert_eq!(ws.snapshot(&mut bob)["context"], context);
    assert_eq!(epoch(&ws), before, "a document authorizes nothing");

    ws.host_ok("workspace.set_governance", json!({"governance": "open"}));
    let written = ws.call_ok(&mut bob, "workspace.context.set", set("# Lab server\n", 1));
    assert_eq!(written["revision"], 2);
    let context = ws.call_ok(&mut bob, "workspace.context.get", json!({}))["context"].clone();
    assert_eq!(context["updated_by"], bob.principal_id.as_str());
    assert_eq!(context["text"], "# Lab server\n");

    // Kept across a restart, every revision in the journal.
    let mut ws = ws.reopen();
    assert_eq!(ws.host_snapshot()["context"], context);
    let journal = String::from_utf8(ws.journal_bytes()).unwrap();
    let revisions = journal
        .lines()
        .filter(|line| line.contains("\"operation\":\"workspace.context.set\""))
        .count();
    assert_eq!(revisions, 2);
}

#[test]
fn a_change_made_from_an_old_revision_is_a_conflict() {
    let mut ws = Workspace::new("context-conflict");
    ws.host_ok("workspace.context.set", set("first", 0));
    ws.host_ok("workspace.context.set", set("second", 1));
    let lines = journal_lines(&ws);
    for stale in [0, 1, 3] {
        let (code, message) = refused(ws.host_call("workspace.context.set", set("lost", stale)));
        assert_eq!(code, "revision_conflict", "{stale}: {message}");
    }
    let (code, _) = refused(ws.host_call("workspace.context.set", json!({"text": "no base"})));
    assert_eq!(code, "invalid_params");
    assert_eq!(journal_lines(&ws), lines);
    assert_eq!(
        ws.host_ok("workspace.context.get", json!({}))["context"]["text"],
        "second"
    );
}

#[test]
fn it_is_at_most_64_kib_of_text_with_line_breaks_and_tabs_only() {
    let mut ws = Workspace::new("context-size");
    let lines = journal_lines(&ws);
    let (code, message) =
        refused(ws.host_call("workspace.context.set", set(&"x".repeat(64 * 1024 + 1), 0)));
    assert_eq!(code, "invalid_params", "{message}");
    for control in ["\u{7}", "\u{0}", "\u{1b}[31m", "\u{7f}"] {
        let (code, _) = refused(ws.host_call(
            "workspace.context.set",
            set(&format!("before{control}after"), 0),
        ));
        assert_eq!(code, "invalid_params", "{control:?}");
    }
    let (code, _) = refused(ws.host_call(
        "workspace.context.set",
        json!({"text": 42, "expected_revision": 0}),
    ));
    assert_eq!(code, "invalid_params");
    assert_eq!(journal_lines(&ws), lines);

    // Exactly 64 KiB, multi-byte characters included, is accepted, and so is clearing it.
    let full = format!("{}é", "x".repeat(64 * 1024 - 2));
    assert_eq!(full.len(), 64 * 1024);
    ws.host_ok("workspace.context.set", set(&full, 0));
    ws.host_ok("workspace.context.set", set("", 1));
    assert_eq!(
        ws.host_ok("workspace.context.get", json!({}))["context"]["text"],
        ""
    );
}

#[test]
fn it_counts_toward_its_editors_share() {
    let mut ws = Workspace::new("context-share");
    let (mut bob, _) = with_bob(&mut ws);
    ws.host_ok("workspace.set_governance", json!({"governance": "open"}));
    ws.broker.set_quotas(Quotas {
        member_state_bytes: 32 * 1024,
        ..Quotas::STANDARD
    });
    let text = "x".repeat(48 * 1024);
    let (code, _) = refused(ws.call(&mut bob, "workspace.context.set", set(&text, 0)));
    assert_eq!(code, "quota_exceeded");
    // The host holds no share.
    ws.host_ok("workspace.context.set", set(&text, 0));
    // Once the revision is the host's, Bob's own small edit fits again.
    ws.call_ok(&mut bob, "workspace.context.set", set("short", 1));
}

#[test]
fn every_agent_reads_it_and_none_writes_it() {
    let mut ws = Workspace::new("context-worker");
    let (mut bob, general) = with_bob(&mut ws);
    ws.host_ok("workspace.context.set", set(DRAFT, 0));
    let credential = grant(&mut ws, &mut bob, &general, false);
    let read = ok(worker(
        &mut ws,
        BOB,
        &credential,
        "workspace.context.get",
        json!({}),
    ));
    assert_eq!(read["context"]["text"], DRAFT);
    let lines = journal_lines(&ws);
    let mut params = set("injected", 1);
    params["idempotency_key"] = json!("agent-set");
    let (code, message) = refused(worker(
        &mut ws,
        BOB,
        &credential,
        "workspace.context.set",
        params,
    ));
    assert_eq!(
        (code.as_str(), message.as_str()),
        ("forbidden", "forbidden: worker operation unavailable")
    );
    assert_eq!(journal_lines(&ws), lines);

    // An agent on a public model reads it in a Public workspace.
    let mut ws = Workspace::unlabelled("context-public");
    ws.host_ok("policy.set", json!({"mode": "public"}));
    let mut carol = ws.enroll(CAROL, "carol", 12);
    let (team, general) = ws.host_team("Open lab");
    ws.host_ok(
        "team.add_member",
        json!({"team_id": team, "principal_id": carol.principal_id, "expected_username": "carol"}),
    );
    ws.host_ok("workspace.context.set", set("Public notes", 0));
    let credential = grant(&mut ws, &mut carol, &general, true);
    let read = ok(worker(
        &mut ws,
        CAROL,
        &credential,
        "workspace.context.get",
        json!({}),
    ));
    assert_eq!(read["context"]["text"], "Public notes");
}
