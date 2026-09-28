//! Shared workspace budgets: no single member can exhaust them, what ages out is removed, the
//! host keeps headroom for administrative operations, and responses other members can grow stay
//! under the frame limit.
//!
//! - BROKER-1: per-member shares of the state and journal, bounded free-text fields (escaped
//!   size included), an idempotency cache that ages out and is bounded per member, and headroom
//!   for the host's administrative operations.
//! - BROKER-2: snapshot sections other members can grow (references, invitations, runs) and
//!   the worker's context manifest and message pages stay within the frame limit; a repeat
//!   invitation renews rather than adds.
//! - BROKER-4: per-member shares of attachments, teams, channels and references, and
//!   unfinished uploads that expire.
#![cfg(unix)]

mod support;

use biorouter_crew::{Quotas, Request, MAX_FRAME};
use serde_json::{json, Value};
use support::*;

const MALLORY: u32 = 71_001;
const VICTOR: u32 = 71_002;

/// The broker's cache key for a human request by `principal` with `key`.
fn dedupe_key(principal: &str, key: &str) -> String {
    format!("{principal}:human:{key}")
}

/// The serialized size of a response as `serve` would frame it.
fn frame_len(response: &biorouter_crew::Response) -> usize {
    serde_json::to_vec(response).unwrap().len()
}

/// `run.create` parameters for a private run over `channel` in a workspace labelled `ucsf`.
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

fn worker(ws: &mut Workspace, uid: u32, credential: &str, method: &str, params: Value) -> Value {
    ok(worker_call(ws, uid, credential, method, params))
}

fn worker_call(
    ws: &mut Workspace,
    uid: u32,
    credential: &str,
    method: &str,
    params: Value,
) -> biorouter_crew::Response {
    let mut connection = biorouter_crew::Connection::new();
    ws.broker.handle(
        uid,
        &mut connection,
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

/// What `principal` holds against their share of the state, counted as the broker counts it
/// for the records these tests make: their messages, and their runs with a grant each.
fn share_used(ws: &Workspace, principal: &str) -> usize {
    let state = ws.broker.state_json();
    let size = |value: &Value| serde_json::to_vec(value).unwrap().len();
    let messages: usize = state["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["actor_id"] == principal)
        .map(size)
        .sum();
    let runs: usize = state["runs"]
        .as_object()
        .unwrap()
        .values()
        .filter(|run| run["owner_id"] == principal)
        .map(|run| size(run) + 110)
        .sum();
    messages + runs
}

// ---------------------------------------------------------------------------------------------
// BROKER-1: state and journal shares, free text, the idempotency cache, host headroom
// ---------------------------------------------------------------------------------------------

#[test]
fn one_member_cannot_fill_the_workspace_state() {
    let mut ws = Workspace::new("member-state-share");
    let mut mallory = ws.enroll(MALLORY, "mallory", 21);
    let mut victor = ws.enroll(VICTOR, "victor", 22);
    let (_, mallory_general) = ws.create_team(&mut mallory, "mallory-team");
    let (_, victor_general) = ws.create_team(&mut victor, "victor-team");
    let body = "x".repeat(65_000);
    let mut refusal = None;
    for index in 0..200 {
        let response = ws.call(
            &mut mallory,
            "message.post",
            json!({"channel_id": mallory_general, "body": body}),
        );
        if let Some(error) = response.error {
            refusal = Some((index, error));
            break;
        }
    }
    let (index, error) = refusal.expect("a member's share runs out");
    assert_eq!(error.code, "quota_exceeded");
    assert!(error.message.contains("your share"), "{}", error.message);
    // A quarter of the 16 MiB state, not all of it.
    assert!(
        (55..70).contains(&index),
        "refused after {index} posts of 65 KB"
    );
    // Everyone else still writes.
    ws.call_ok(
        &mut victor,
        "message.post",
        json!({"channel_id": victor_general, "body": "still here"}),
    );
    ws.host_ok("team.create", json!({"name": "host-team"}));
    // Mallory can still do what adds no content, and read.
    ws.call_ok(&mut mallory, "profile.update", json!({"nickname": null}));
    ws.call_ok(
        &mut mallory,
        "messages.history",
        json!({"channel_id": mallory_general, "latest": true, "limit": 1}),
    );
}

#[test]
fn a_message_body_is_bounded_after_escaping() {
    let mut ws = Workspace::new("escaped-body");
    let (_, general) = ws.host_team("escape");
    // 65,536 bytes that JSON escapes six-fold: 384 KiB stored, twice (history and the cache).
    let (code, _) = refused(ws.host_call(
        "message.post",
        json!({"channel_id": general, "body": "\u{1}".repeat(65_536)}),
    ));
    assert_eq!(code, "invalid_params");
    let (code, _) = refused(ws.host_call(
        "message.post",
        json!({"channel_id": general, "body": "\u{1b}[0m".repeat(16_000)}),
    ));
    assert_eq!(code, "invalid_params");
    // Quotes and backslashes double at most, so any ordinary body up to the limit still fits.
    ws.host_ok(
        "message.post",
        json!({"channel_id": general, "body": "\"".repeat(65_536)}),
    );
    // Colored terminal output of an ordinary density still posts.
    ws.host_ok(
        "message.post",
        json!({"channel_id": general, "body": "\u{1b}[31merror\u{1b}[0m: failed\n".repeat(2_000)}),
    );
}

#[test]
fn free_text_fields_are_bounded_and_printable() {
    let mut ws = Workspace::new("free-text");
    let (_, general) = ws.host_team("text");
    let sha = "0".repeat(64);
    let begin = |media_type: &str, name: &str| json!({"channel_id": general, "name": name, "media_type": media_type, "size": 1, "sha256": sha});
    for (media_type, name) in [
        ("a".repeat(1_040_000), "x".to_owned()),
        ("a".repeat(256), "x".to_owned()),
        ("text/plain\u{1}".to_owned(), "x".to_owned()),
        ("téxt/plain".to_owned(), "x".to_owned()),
        ("text/plain".to_owned(), "report\u{202e}fdp.exe".to_owned()),
        ("text/plain".to_owned(), "a\u{200b}b".to_owned()),
    ] {
        let (code, _) = refused(ws.host_call("blob.begin", begin(&media_type, &name)));
        assert_eq!(code, "invalid_params", "{name:?}");
    }
    ws.host_ok(
        "blob.begin",
        begin("text/csv; charset=utf-8", "results.csv"),
    );

    let mut run = run_params(&ws, &general);
    run["provider_policy_id"] = json!("p".repeat(2_000));
    assert_eq!(
        refused(ws.host_call("run.create", run.clone())).0,
        "invalid_params"
    );
    run["provider_policy_id"] = json!("private\u{1}");
    assert_eq!(
        refused(ws.host_call("run.create", run.clone())).0,
        "invalid_params"
    );
    run["provider_policy_id"] = json!("private");
    run["remote_root"] = json!(format!("/{}", "r".repeat(5_000)));
    assert_eq!(refused(ws.host_call("run.create", run)).0, "invalid_params");

    for (path, label) in [
        (format!("/{}", "\u{1}".repeat(4_095)), "data"),
        ("/data/\u{202e}vsc.exe".to_owned(), "data"),
        (format!("/{}", "p".repeat(4_096)), "data"),
        ("/data/set".to_owned(), "da\u{200b}ta"),
        ("/data/set".to_owned(), "data\u{2028}"),
    ] {
        let (code, _) = refused(ws.host_call(
            "reference.create",
            json!({"channel_id": general, "path": path, "label": label}),
        ));
        assert_eq!(code, "invalid_params", "{path:?} {label:?}");
    }
    ws.host_ok(
        "reference.create",
        json!({"channel_id": general, "path": "/data/set 1/\"quoted\" 📁", "label": "Set 1"}),
    );
}

#[test]
fn the_idempotency_cache_is_bounded_per_member() {
    let mut ws = Workspace::new("dedupe-per-member");
    let mut mallory = ws.enroll(MALLORY, "mallory", 21);
    ok(signed_raw(
        &mut ws.broker,
        &mut mallory,
        "profile.update",
        json!({"nickname": "First", "idempotency_key": "first"}),
    ));
    for index in 0..600 {
        ok(signed_raw(
            &mut ws.broker,
            &mut mallory,
            "profile.update",
            json!({"nickname": null, "idempotency_key": format!("fill-{index}")}),
        ));
    }
    let state = ws.broker.state_json();
    let prefix = format!("{}:", mallory.principal_id);
    let own = state["dedupe"]
        .as_object()
        .unwrap()
        .keys()
        .filter(|key| key.starts_with(&prefix))
        .count();
    assert!(
        own <= Quotas::STANDARD.dedupe_actor_entries,
        "{own} cached results for one member"
    );
    // The oldest result aged out of the member's cache: its key is free again rather than a
    // conflict.
    ok(signed_raw(
        &mut ws.broker,
        &mut mallory,
        "profile.update",
        json!({"nickname": "Changed", "idempotency_key": "first"}),
    ));
    // A retry of a recent request still replays.
    let replay = ok(signed_raw(
        &mut ws.broker,
        &mut mallory,
        "profile.update",
        json!({"nickname": null, "idempotency_key": "fill-599"}),
    ));
    assert_eq!(replay["nickname"], "mallory");
}

#[test]
fn an_agent_burst_never_evicts_the_persons_own_recent_result() {
    let mut ws = Workspace::new("dedupe-agent-burst");
    let mut mallory = ws.enroll(MALLORY, "mallory", 21);
    let (team, general) = ws.host_team("burst");
    ws.host_adds_to_team(&mut mallory, &team);
    let params = run_params(&ws, &general);
    let credential = ws.call_ok(&mut mallory, "run.create", params)["credential"]
        .as_str()
        .unwrap()
        .to_owned();
    let post = json!({"channel_id": general, "body": "only once", "idempotency_key": "keep"});
    ok(signed_raw(
        &mut ws.broker,
        &mut mallory,
        "message.post",
        post.clone(),
    ));
    for index in 0..600 {
        worker(
            &mut ws,
            MALLORY,
            &credential,
            "run.project",
            json!({"body": "working", "status": "progress", "idempotency_key": format!("p{index}")}),
        );
    }
    // The composer retries its unanswered post: it replays rather than posting again.
    ok(signed_raw(
        &mut ws.broker,
        &mut mallory,
        "message.post",
        post,
    ));
    let posted = ws.broker.state_json()["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["body"] == "only once")
        .count();
    assert_eq!(posted, 1);
}

#[test]
fn a_member_at_their_share_can_still_finish_an_agent_task() {
    let mut ws = Workspace::new("terminal-at-share");
    ws.broker.set_quotas(Quotas {
        member_state_bytes: 256 * 1024,
        ..Quotas::STANDARD
    });
    let mut mallory = ws.enroll(MALLORY, "mallory", 21);
    let (team, general) = ws.host_team("finish");
    ws.host_adds_to_team(&mut mallory, &team);
    let params = run_params(&ws, &general);
    let credential = ws.call_ok(&mut mallory, "run.create", params)["credential"]
        .as_str()
        .unwrap()
        .to_owned();
    // Fill the share to the last few bytes.
    for size in [60_000, 2_000, 100, 1] {
        let mut full = false;
        for _ in 0..100 {
            if ws
                .call(
                    &mut mallory,
                    "message.post",
                    json!({"channel_id": general, "body": "x".repeat(size)}),
                )
                .error
                .is_some()
            {
                full = true;
                break;
            }
        }
        assert!(full, "the share fills with {size}-byte posts");
    }
    worker(
        &mut ws,
        MALLORY,
        &credential,
        "run.project",
        json!({"body": "done", "status": "completed", "idempotency_key": "done"}),
    );
}

#[test]
fn terminal_projections_take_a_member_at_most_one_message_past_their_share() {
    let mut ws = Workspace::new("terminal-share-bound");
    // The standard limits scaled down, so the attack fits in a test: runs made while under the
    // share, then one maximum-size result per run.
    let share = 256 * 1024;
    ws.broker.set_quotas(Quotas {
        state_bytes: 2 * 1024 * 1024,
        state_admin_headroom: 128 * 1024,
        member_state_bytes: share,
        ..Quotas::STANDARD
    });
    let mut mallory = ws.enroll(MALLORY, "mallory", 21);
    let mut victor = ws.enroll(VICTOR, "victor", 22);
    let (team, general) = ws.host_team("results");
    ws.host_adds_to_team(&mut mallory, &team);
    ws.host_adds_to_team(&mut victor, &team);
    let params = run_params(&ws, &general);
    let runs: Vec<(String, String)> = (0..40)
        .map(|_| {
            let run = ws.call_ok(&mut mallory, "run.create", params.clone());
            (
                run["run"]["id"].as_str().unwrap().to_owned(),
                run["credential"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert!(share_used(&ws, &mallory.principal_id) < share);
    // Quotes escape to twice their size: the largest body a message may have.
    let body = "\"".repeat(65_536);
    let mut posted = 0;
    let mut stopped = Vec::new();
    for (index, (run, credential)) in runs.iter().enumerate() {
        let response = worker_call(
            &mut ws,
            MALLORY,
            credential,
            "run.project",
            json!({"body": body, "status": "completed", "idempotency_key": format!("done-{index}")}),
        );
        match response.error {
            None => posted += 1,
            Some(error) => {
                assert_eq!(error.code, "quota_exceeded", "{}", error.message);
                assert!(error.message.contains("your share"), "{}", error.message);
                stopped.push(run.clone());
            }
        }
    }
    assert!(
        posted > 0,
        "results post while the member is within the share"
    );
    assert!(!stopped.is_empty(), "and stop once they are past it");
    // Never more than one message past the share.
    let largest = ws.broker.state_json()["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["actor_id"] == json!(mallory.principal_id))
        .map(|message| serde_json::to_vec(message).unwrap().len())
        .max()
        .unwrap();
    let used = share_used(&ws, &mallory.principal_id);
    assert!(
        used <= share + largest,
        "{used} bytes held against a {share}-byte share"
    );
    // Everyone else keeps working.
    ws.call_ok(
        &mut victor,
        "message.post",
        json!({"channel_id": general, "body": "still here"}),
    );
    // A refused result leaves its run live, and its owner still ends it: removing access is
    // never refused for space.
    for run in &stopped {
        let revoked = ws.call_ok(&mut mallory, "run.revoke", json!({"run_id": run}));
        assert_eq!(revoked["revoked"], true);
    }
}

#[test]
fn the_idempotency_cache_ages_out() {
    let mut ws = Workspace::new("dedupe-ttl");
    let host = ws.host.principal_id.clone();
    for key in ["old", "legacy", "fresh"] {
        ok(signed_raw(
            &mut ws.broker,
            &mut ws.host,
            "profile.update",
            json!({"nickname": "Before", "idempotency_key": key}),
        ));
    }
    // One result cached two days ago, and one from a release that recorded no time.
    let ws = ws.edit_journal(|journal| {
        journal.append(
            "system",
            "test.age",
            vec![
                set(
                    &["dedupe", &dedupe_key(&host, "old"), "at"],
                    json!(now() - 2 * 24 * 60 * 60),
                ),
                remove(&["dedupe", &dedupe_key(&host, "legacy"), "at"]),
            ],
        );
    });
    let mut ws = ws;
    // Any mutation prunes what aged out.
    ws.host_ok("profile.update", json!({"nickname": null}));
    let state = ws.broker.state_json();
    let dedupe = state["dedupe"].as_object().unwrap();
    assert!(!dedupe.contains_key(&dedupe_key(&host, "old")));
    assert!(!dedupe.contains_key(&dedupe_key(&host, "legacy")));
    assert!(dedupe.contains_key(&dedupe_key(&host, "fresh")));
    for key in ["old", "legacy"] {
        ok(signed_raw(
            &mut ws.broker,
            &mut ws.host,
            "profile.update",
            json!({"nickname": "After", "idempotency_key": key}),
        ));
    }
    let (code, _) = refused(signed_raw(
        &mut ws.broker,
        &mut ws.host,
        "profile.update",
        json!({"nickname": "After", "idempotency_key": "fresh"}),
    ));
    assert_eq!(code, "conflict", "a fresh result still guards its key");
}

#[test]
fn a_workspace_filled_by_results_that_never_aged_recovers() {
    let mut ws = Workspace::new("legacy-cache-full");
    let mut mallory = ws.enroll(MALLORY, "mallory", 21);
    let (team, general) = ws.host_team("legacy");
    ws.host_adds_to_team(&mut mallory, &team);
    // 1,500 cached results from a release that kept them forever and recorded no time: more
    // than one mutation removes at once.
    let host = ws.host.principal_id.clone();
    let ws = ws.edit_journal(|journal| {
        let patches = (0..1_500)
            .map(|index| {
                set(
                    &["dedupe", &dedupe_key(&host, &format!("legacy-{index}"))],
                    json!({"digest": "0".repeat(64), "result": {"filler": "f".repeat(1_000)}}),
                )
            })
            .collect();
        journal.append("system", "test.legacy", patches);
    });
    let mut ws = ws;
    // Ordinary changes stop 1.3 MB below the state as it stands, so every change is over the
    // limit. One that removes more than it adds is still allowed: each mutation removes up to
    // 1,024 aged results, so the workspace sheds them and recovers.
    let size = serde_json::to_vec(&ws.broker.state_json()).unwrap().len();
    ws.broker.set_quotas(Quotas {
        state_bytes: size - 1_300_000 + 64 * 1024,
        state_admin_headroom: 64 * 1024,
        ..Quotas::STANDARD
    });
    let legacy = |ws: &Workspace| {
        ws.broker.state_json()["dedupe"]
            .as_object()
            .unwrap()
            .keys()
            .filter(|key| key.contains(":legacy-"))
            .count()
    };
    ws.call_ok(
        &mut mallory,
        "message.post",
        json!({"channel_id": general, "body": "hello"}),
    );
    assert_eq!(legacy(&ws), 1_500 - 1_024);
    ws.call_ok(
        &mut mallory,
        "message.post",
        json!({"channel_id": general, "body": "again"}),
    );
    assert_eq!(legacy(&ws), 0);
    // Now under the limit, ordinary changes carry on.
    ws.call_ok(
        &mut mallory,
        "message.post",
        json!({"channel_id": general, "body": "and again"}),
    );
}

#[test]
fn the_workspace_wide_cache_limit_evicts_instead_of_refusing() {
    let mut ws = Workspace::new("dedupe-total");
    ws.broker.set_quotas(Quotas {
        dedupe_entries: 40,
        ..Quotas::STANDARD
    });
    let mut mallory = ws.enroll(MALLORY, "mallory", 21);
    for _ in 0..30 {
        ws.host_ok("profile.update", json!({"nickname": null}));
        ws.call_ok(&mut mallory, "profile.update", json!({"nickname": null}));
    }
    assert!(ws.broker.state_json()["dedupe"].as_object().unwrap().len() <= 40);
}

#[test]
fn host_administrative_operations_keep_headroom() {
    let mut ws = Workspace::new("admin-headroom");
    // The standard limits scaled down eightfold, so the state fills in seconds; the rule is the
    // same at any size.
    ws.broker.set_quotas(Quotas {
        state_bytes: 2 * 1024 * 1024,
        state_admin_headroom: 128 * 1024,
        member_state_bytes: 512 * 1024,
        ..Quotas::STANDARD
    });
    let mut mallory = ws.enroll(MALLORY, "mallory", 21);
    let (team, general) = ws.host_team("full");
    ws.host_adds_to_team(&mut mallory, &team);
    let params = run_params(&ws, &general);
    let run = ws.call_ok(&mut mallory, "run.create", params)["run"]["id"].clone();
    // Fill the state to the last few hundred bytes the host's ordinary changes may use.
    for size in [65_000, 4_000, 200, 1] {
        let body = "x".repeat(size);
        let mut filled = false;
        for _ in 0..400 {
            if let Some(error) = ws
                .host_call("message.post", json!({"channel_id": general, "body": body}))
                .error
            {
                assert_eq!(error.code, "quota_exceeded", "{}", error.message);
                filled = true;
                break;
            }
        }
        assert!(filled, "the state fills with {size}-byte posts");
    }
    let (code, _) = refused(ws.call(
        &mut mallory,
        "message.post",
        json!({"channel_id": general, "body": "hello"}),
    ));
    assert_eq!(code, "quota_exceeded");
    let (code, _) = refused(ws.host_call(
        "message.post",
        json!({"channel_id": general, "body": "hello"}),
    ));
    assert_eq!(code, "quota_exceeded");
    // Taking access away never waits for storage: a member stops their agent, and the channel
    // owner removes someone.
    ws.call_ok(&mut mallory, "run.revoke", json!({"run_id": run}));
    ws.host_ok(
        "membership.revoke",
        json!({"channel_id": general, "principal_id": mallory.principal_id}),
    );
    // The host can still remove the member and change policy.
    ws.host_ok(
        "enrollment.revoke",
        json!({"principal_id": mallory.principal_id, "expected_username": "mallory"}),
    );
    ws.host_ok(
        "policy.set",
        json!({"mode": "private", "institution_id": "ucsf"}),
    );
    ws.host_ok("workspace.rename", json!({"name": "archive"}));
    // Reading still works.
    ws.host_snapshot();
}

#[test]
fn a_run_removed_after_it_expired_is_still_revoked_for_its_owner() {
    let mut ws = Workspace::new("removed-run-revoke");
    let mut mallory = ws.enroll(MALLORY, "mallory", 21);
    let mut victor = ws.enroll(VICTOR, "victor", 22);
    let (team, general) = ws.host_team("revoke");
    ws.host_adds_to_team(&mut mallory, &team);
    ws.host_adds_to_team(&mut victor, &team);
    let params = run_params(&ws, &general);
    let run_id = |run: Value| run["run"]["id"].as_str().unwrap().to_owned();
    let earlier = run_id(ws.call_ok(&mut mallory, "run.create", params.clone()));
    let later = run_id(ws.call_ok(&mut mallory, "run.create", params));
    let expired_two_days_ago = |ws: Workspace, run: &str| {
        ws.edit_journal(|journal| {
            journal.append(
                "system",
                "test.age",
                vec![set(
                    &["runs", run, "expires_at"],
                    json!(now() - 2 * 24 * 60 * 60),
                )],
            );
        })
    };
    let held = |ws: &Workspace, run: &str| ws.broker.state_json()["runs"].get(run).is_some();

    // Removed by an unrelated change, a day after it expired, then revoked by its owner: a
    // daemon that stopped the grant while the workspace was out of reach asks this whenever it
    // reconnects.
    let mut ws = expired_two_days_ago(ws, &earlier);
    ws.host_ok("profile.update", json!({"nickname": null}));
    assert!(!held(&ws, &earlier), "retention removed the run");
    let revoked = ws.call_ok(&mut mallory, "run.revoke", json!({"run_id": earlier}));
    assert_eq!(revoked["id"], json!(earlier));
    assert_eq!(revoked["revoked"], true);

    // Removed by the very revoke that asks about it.
    let mut ws = expired_two_days_ago(ws, &later);
    assert!(held(&ws, &later));
    let revoked = ws.call_ok(&mut mallory, "run.revoke", json!({"run_id": later}));
    assert_eq!(revoked["revoked"], true);
    assert!(!held(&ws, &later));

    // Anyone else is answered as for a run that never existed.
    let (code, message) = refused(ws.call(&mut victor, "run.revoke", json!({"run_id": earlier})));
    let (unknown_code, unknown_message) =
        refused(ws.call(&mut victor, "run.revoke", json!({"run_id": uuid()})));
    assert_eq!(
        (code.as_str(), message.as_str()),
        ("forbidden", unknown_message.as_str())
    );
    assert_eq!(unknown_code, "forbidden");
    let (code, _) = refused(ws.call(&mut mallory, "run.revoke", json!({"run_id": uuid()})));
    assert_eq!(code, "forbidden");

    // The removals are journaled, so a restarted broker still answers the owner.
    let mut ws = ws.reopen();
    for run in [&earlier, &later] {
        let revoked = ws.call_ok(&mut mallory, "run.revoke", json!({"run_id": run}));
        assert_eq!(revoked["revoked"], true);
    }
    let (code, _) = refused(ws.call(&mut victor, "run.revoke", json!({"run_id": later})));
    assert_eq!(code, "forbidden");
}

#[test]
fn repeated_removals_cannot_use_up_the_journal_headroom() {
    let mut ws = Workspace::new("journal-headroom");
    let mut mallory = ws.enroll(MALLORY, "mallory", 21);
    let (team, general) = ws.host_team("journal");
    ws.host_adds_to_team(&mut mallory, &team);
    let params = run_params(&ws, &general);
    let run = ws.call_ok(&mut mallory, "run.create", params)["run"]["id"].clone();
    // A journal 256 KiB from full, half of it kept for the host.
    let used = std::fs::metadata(ws.journal_path()).unwrap().len();
    ws.broker.set_quotas(Quotas {
        journal_bytes: used + 256 * 1024,
        journal_admin_headroom: 128 * 1024,
        ..Quotas::STANDARD
    });
    let mut refusal = None;
    for _ in 0..2_000 {
        if let Some(error) = ws
            .call(&mut mallory, "run.revoke", json!({"run_id": run}))
            .error
        {
            refusal = Some(error);
            break;
        }
    }
    let error = refusal.expect("repeated removals reach the ordinary journal limit");
    assert_eq!(error.code, "quota_exceeded");
    // What is left is the host's.
    ws.host_ok(
        "enrollment.revoke",
        json!({"principal_id": mallory.principal_id}),
    );
}

#[test]
fn one_member_cannot_fill_the_journal() {
    let mut ws = Workspace::new("member-journal-share");
    ws.broker.set_quotas(Quotas {
        member_journal_bytes: 256 * 1024,
        ..Quotas::STANDARD
    });
    let mut mallory = ws.enroll(MALLORY, "mallory", 21);
    let mut refusal = None;
    for index in 0..2_000 {
        if let Some(error) = ws
            .call(&mut mallory, "profile.update", json!({"nickname": null}))
            .error
        {
            refusal = Some((index, error));
            break;
        }
    }
    let (_, error) = refusal.expect("a member's journal share runs out");
    assert_eq!(error.code, "quota_exceeded");
    assert!(error.message.contains("audit journal"), "{}", error.message);
    // The share is counted from the journal itself, so a restart does not reset it.
    let quotas = Quotas {
        member_journal_bytes: 256 * 1024,
        ..Quotas::STANDARD
    };
    let mut ws = ws.reopen();
    ws.broker.set_quotas(quotas);
    let (code, _) = refused(ws.call(&mut mallory, "profile.update", json!({"nickname": null})));
    assert_eq!(code, "quota_exceeded");
    ws.host_ok("profile.update", json!({"nickname": null}));
}

// ---------------------------------------------------------------------------------------------
// BROKER-4: attachment, team, channel and reference shares; unfinished uploads expire
// ---------------------------------------------------------------------------------------------

#[test]
fn one_member_cannot_reserve_the_workspace_attachment_space() {
    let mut ws = Workspace::new("attachment-share");
    let mut mallory = ws.enroll(MALLORY, "mallory", 21);
    let mut victor = ws.enroll(VICTOR, "victor", 22);
    let (_, mallory_general) = ws.create_team(&mut mallory, "mallory-team");
    let (_, victor_general) = ws.create_team(&mut victor, "victor-team");
    let sha = "0".repeat(64);
    let gib = 1024u64 * 1024 * 1024;
    let begin = |channel: &str, size: u64| json!({"channel_id": channel, "name": "big.bin", "media_type": "application/octet-stream", "size": size, "sha256": sha});
    let mut begun = Vec::new();
    for _ in 0..10 {
        match ws.call(&mut mallory, "blob.begin", begin(&mallory_general, gib)) {
            response if response.error.is_none() => begun.push(ok(response)["id"].clone()),
            response => {
                let (code, message) = refused(response);
                assert_eq!(code, "quota_exceeded");
                assert!(message.contains("your share"), "{message}");
                break;
            }
        }
    }
    assert_eq!(
        begun.len(),
        2,
        "a quarter of 10 GiB holds two 1 GiB uploads"
    );
    // Another member can still attach files.
    ws.call_ok(&mut victor, "blob.begin", begin(&victor_general, 1_000));

    // Nothing touched Mallory's uploads for two days: they lapse, their files go, and her
    // space is back.
    let blob_ids: Vec<String> = begun
        .iter()
        .map(|id| id.as_str().unwrap().to_owned())
        .collect();
    let root = ws.root.path().to_path_buf();
    for id in &blob_ids {
        assert!(root.join("blobs").join(id).exists());
    }
    let ws = ws.edit_journal(|journal| {
        journal.append(
            "system",
            "test.age",
            blob_ids
                .iter()
                .map(|id| {
                    set(
                        &["blobs", id, "touched_at"],
                        json!(now() - 2 * 24 * 60 * 60),
                    )
                })
                .collect(),
        );
    });
    let mut ws = ws;
    ws.call_ok(&mut mallory, "blob.begin", begin(&mallory_general, gib));
    let state = ws.broker.state_json();
    for id in &blob_ids {
        assert!(
            state["blobs"].get(id).is_none(),
            "the lapsed upload is gone"
        );
        assert!(!root.join("blobs").join(id).exists(), "and so is its file");
    }
}

#[test]
fn progress_renews_an_upload_and_a_finished_attachment_never_lapses() {
    let mut ws = Workspace::new("upload-progress");
    let (_, general) = ws.host_team("upload");
    let data = b"hello";
    let sha = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(data));
    let blob = ws.host_ok(
        "blob.begin",
        json!({"channel_id": general, "name": "a.txt", "media_type": "text/plain", "size": 5, "sha256": sha}),
    );
    let id = blob["id"].as_str().unwrap().to_owned();
    let begun = blob["touched_at"]
        .as_u64()
        .expect("an upload records its activity");
    let chunk = ws.host_ok(
        "blob.chunk",
        json!({"blob_id": id, "offset": 0, "data_hex": hex::encode(data)}),
    );
    assert!(
        chunk["touched_at"].as_u64().unwrap() >= begun,
        "a chunk renews it"
    );
    ws.host_ok("blob.finish", json!({"blob_id": id}));
    // A finished attachment is history: it never lapses.
    let ws = ws.edit_journal(|journal| {
        journal.append(
            "system",
            "test.age",
            vec![set(&["blobs", &id, "touched_at"], json!(0))],
        );
    });
    let mut ws = ws;
    ws.host_ok("profile.update", json!({"nickname": null}));
    assert!(ws.broker.state_json()["blobs"].get(&id).is_some());
}

#[test]
fn one_member_cannot_take_every_team_channel_or_reference() {
    let mut ws = Workspace::new("create-shares");
    ws.broker.set_quotas(Quotas {
        member_references: 5,
        ..Quotas::STANDARD
    });
    let mut mallory = ws.enroll(MALLORY, "mallory", 21);
    let mut victor = ws.enroll(VICTOR, "victor", 22);
    let mut teams = Vec::new();
    for index in 0..11 {
        let response = ws.call(
            &mut mallory,
            "team.create",
            json!({"name": format!("m{index}")}),
        );
        if index < Quotas::STANDARD.member_teams {
            teams.push(ok(response)["team"]["id"].as_str().unwrap().to_owned());
        } else {
            let (code, message) = refused(response);
            assert_eq!(code, "quota_exceeded");
            assert!(message.contains("teams"), "{message}");
        }
    }
    // Ten teams brought ten #general channels; ninety more reach the channel share.
    let mut created = 10;
    let mut refused_channel = false;
    for index in 0..200 {
        let team = &teams[index % teams.len()];
        let response = ws.call(
            &mut mallory,
            "channel.create",
            json!({"team_id": team, "name": format!("c{index}")}),
        );
        if response.error.is_some() {
            let (code, message) = refused(response);
            assert_eq!(code, "quota_exceeded");
            assert!(message.contains("channels"), "{message}");
            refused_channel = true;
            break;
        }
        created += 1;
    }
    assert!(refused_channel);
    assert_eq!(created, Quotas::STANDARD.member_channels);
    // Others still create teams and channels; the host has no share.
    let (victor_team, victor_general) = ws.create_team(&mut victor, "victor-team");
    ws.call_ok(
        &mut victor,
        "channel.create",
        json!({"team_id": victor_team, "name": "notes"}),
    );
    for index in 0..12 {
        ws.host_ok("team.create", json!({"name": format!("h{index}")}));
    }

    let general = ws.snapshot(&mut mallory)["teams"][0]["general_channel_id"]
        .as_str()
        .unwrap()
        .to_owned();
    for index in 0..6 {
        let response = ws.call(
            &mut mallory,
            "reference.create",
            json!({"channel_id": general, "path": format!("/data/{index}"), "label": "d"}),
        );
        if index < 5 {
            ok(response);
        } else {
            assert_eq!(refused(response).0, "quota_exceeded");
        }
    }
    ws.call_ok(
        &mut victor,
        "reference.create",
        json!({"channel_id": victor_general, "path": "/data/v", "label": "v"}),
    );
}

// ---------------------------------------------------------------------------------------------
// BROKER-2: what other members can grow never pushes a response past the frame limit
// ---------------------------------------------------------------------------------------------

#[test]
fn a_snapshot_stays_under_the_frame_limit_whatever_another_member_adds() {
    let mut ws = Workspace::new("snapshot-bound");
    let mut mallory = ws.enroll(MALLORY, "mallory", 21);
    let mut victor = ws.enroll(VICTOR, "victor", 22);
    let (team, general) = ws.create_team(&mut mallory, "shared");
    ws.add_to_team(&mut mallory, &mut victor, &team);
    // References with the longest path a reference may have, in a channel Victor reads.
    let path = format!("/{}", "p".repeat(4_095));
    for _ in 0..300 {
        ws.call_ok(
            &mut mallory,
            "reference.create",
            json!({"channel_id": general, "path": path, "label": "l".repeat(255)}),
        );
    }
    // Invitations aimed at the host, again and again.
    for _ in 0..50 {
        ws.call_ok(
            &mut mallory,
            "invitation.create",
            json!({"kind": "team", "target_id": team, "principal_id": ws.host.principal_id}),
        );
    }
    let response = ws.call(&mut victor, "workspace.snapshot", json!({}));
    assert!(frame_len(&response) < MAX_FRAME, "{}", frame_len(&response));
    let snapshot = ok(response);
    assert_eq!(snapshot["totals"]["references"], 300);
    assert!(snapshot["references"].as_array().unwrap().len() < 300);

    let response = ws.host_call("workspace.snapshot", json!({}));
    assert!(frame_len(&response) < MAX_FRAME);
    let snapshot = ok(response);
    let from_mallory: Vec<&Value> = snapshot["invitations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["inviter_id"] == json!(mallory.principal_id))
        .collect();
    assert_eq!(
        from_mallory.len(),
        1,
        "a repeat invitation renews the first"
    );
    assert_eq!(snapshot["totals"]["invitations"], 1);
}

#[test]
fn one_inviter_cannot_crowd_the_others_out_of_a_snapshot() {
    let mut ws = Workspace::new("invitation-turns");
    ws.broker.set_quotas(Quotas {
        member_live_invitations: 1_000,
        member_channels: 1_000,
        ..Quotas::STANDARD
    });
    let mut mallory = ws.enroll(MALLORY, "mallory", 21);
    let mut victor = ws.enroll(VICTOR, "victor", 22);
    // The host's invitation is the oldest Victor has.
    let (host_team, _) = ws.host_team("host-team");
    let from_host = ws.host_ok(
        "invitation.create",
        json!({"kind": "team", "target_id": host_team, "principal_id": victor.principal_id}),
    )["id"]
        .clone();
    std::thread::sleep(std::time::Duration::from_millis(1_100));
    // Then Mallory sends him more than a snapshot section holds.
    let (team, _) = ws.create_team(&mut mallory, "crowd");
    ws.call_ok(
        &mut mallory,
        "team.add_member",
        json!({"team_id": team, "principal_id": victor.principal_id, "expected_username": "victor"}),
    );
    for index in 0..250 {
        let channel = ws.call_ok(
            &mut mallory,
            "channel.create",
            json!({"team_id": team, "name": format!("c{index}")}),
        )["id"]
            .clone();
        ws.call_ok(
            &mut mallory,
            "invitation.create",
            json!({"kind": "channel", "target_id": channel, "principal_id": victor.principal_id}),
        );
    }
    let snapshot = ws.snapshot(&mut victor);
    let listed = snapshot["invitations"].as_array().unwrap();
    assert!(listed.len() < 251, "the section is bounded");
    assert_eq!(snapshot["totals"]["invitations"], 251);
    assert!(
        listed
            .iter()
            .any(|invitation| invitation["id"] == from_host),
        "every inviter's newest invitation gets a turn"
    );
}

#[test]
fn a_member_has_a_bounded_number_of_invitations_outstanding() {
    let mut ws = Workspace::new("invitation-share");
    ws.broker.set_quotas(Quotas {
        member_live_invitations: 3,
        ..Quotas::STANDARD
    });
    let mut mallory = ws.enroll(MALLORY, "mallory", 21);
    let targets: Vec<String> = (0..4)
        .map(|index| ws.create_team(&mut mallory, &format!("t{index}")).0)
        .collect();
    for (index, team) in targets.iter().enumerate() {
        let response = ws.call(
            &mut mallory,
            "invitation.create",
            json!({"kind": "team", "target_id": team, "principal_id": ws.host.principal_id}),
        );
        if index < 3 {
            ok(response);
        } else {
            assert_eq!(refused(response).0, "quota_exceeded");
        }
    }
    // Renewing one already outstanding is not another.
    ws.call_ok(
        &mut mallory,
        "invitation.create",
        json!({"kind": "team", "target_id": targets[0], "principal_id": ws.host.principal_id}),
    );
}

#[test]
fn the_context_manifest_stays_under_the_frame_limit() {
    let mut ws = Workspace::new("manifest-bound");
    let mut mallory = ws.enroll(MALLORY, "mallory", 21);
    let (team, general) = ws.host_team("agents");
    ws.host_adds_to_team(&mut mallory, &team);
    for _ in 0..20 {
        ws.call_ok(
            &mut mallory,
            "message.post",
            json!({"channel_id": general, "body": "m".repeat(65_000)}),
        );
    }
    ws.host_ok(
        "message.post",
        json!({"channel_id": general, "body": "newest"}),
    );
    let params = run_params(&ws, &general);
    let run = ws.host_ok("run.create", params);
    let credential = run["credential"].as_str().unwrap().to_owned();
    let manifest = worker(
        &mut ws,
        host_uid(),
        &credential,
        "context.manifest",
        json!({}),
    );
    let size = serde_json::to_vec(&manifest).unwrap().len();
    assert!(size < MAX_FRAME, "{size}");
    let messages = manifest["messages"].as_array().unwrap();
    assert_eq!(
        messages[0]["body"], "newest",
        "the newest messages are kept"
    );
    assert!(messages.len() < 21);
    assert_eq!(manifest["restricted"], true);
}

#[test]
fn another_members_largest_posts_never_stop_an_agent_reading_its_channel() {
    let mut ws = Workspace::new("worker-history-bound");
    let mut mallory = ws.enroll(MALLORY, "mallory", 21);
    let (team, general) = ws.host_team("agents");
    ws.host_adds_to_team(&mut mallory, &team);
    // Twenty of the largest bodies a message may have (quotes escape to twice their size),
    // well within Mallory's share: 2.5 MiB, where one frame is 1 MiB.
    for _ in 0..20 {
        ws.call_ok(
            &mut mallory,
            "message.post",
            json!({"channel_id": general, "body": "\"".repeat(65_536)}),
        );
    }
    let newest = ws.host_ok(
        "message.post",
        json!({"channel_id": general, "body": "newest"}),
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let params = run_params(&ws, &general);
    let credential = ws.host_ok("run.create", params)["credential"]
        .as_str()
        .unwrap()
        .to_owned();
    let read = |ws: &mut Workspace, method: &str, params: Value| {
        let response = worker_call(ws, host_uid(), &credential, method, params);
        assert!(frame_len(&response) < MAX_FRAME, "{}", frame_len(&response));
        ok(response)
    };
    let ids = |page: &Value| -> Vec<String> {
        page["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["id"].as_str().unwrap().to_owned())
            .collect()
    };

    // What every agent task reads first: its destination's newest 50.
    let page = read(
        &mut ws,
        "messages.history",
        json!({"channel_id": general, "limit": 50, "latest": true}),
    );
    let mut seen = ids(&page);
    assert_eq!(seen.last(), Some(&newest), "the newest messages are kept");
    assert!(seen.len() < 21);
    assert_eq!(page["truncated"], true);
    assert_eq!(page["cursor"], json!(newest));
    // The rest is still there, before the first message of each page.
    loop {
        let page = read(
            &mut ws,
            "messages.history",
            json!({"channel_id": general, "limit": 50, "latest": true, "before": seen[0]}),
        );
        let older = ids(&page);
        if older.is_empty() {
            break;
        }
        seen.splice(0..0, older);
    }
    assert_eq!(seen.len(), 21);

    // Paging forward from the start reaches the same messages in the same order.
    let mut forward: Vec<String> = Vec::new();
    let mut cursor = Value::Null;
    loop {
        let mut params = json!({"channel_id": general, "limit": 50});
        if !cursor.is_null() {
            params["after"] = cursor.clone();
        }
        let page = read(&mut ws, "messages.history", params);
        let next = ids(&page);
        if next.is_empty() {
            break;
        }
        forward.extend(next);
        cursor = page["cursor"].clone();
    }
    assert_eq!(forward, seen);

    // Search pages are bounded the same way.
    let page = read(
        &mut ws,
        "messages.search",
        json!({"channel_id": general, "query": "\"", "limit": 50, "latest": true}),
    );
    assert_eq!(page["truncated"], true);

    // A person's page is not cut short: a client offers an older page when the one it has is
    // full, so it is refused as too large at the frame and asked again for fewer.
    let page = ws.host_ok(
        "messages.history",
        json!({"channel_id": general, "limit": 50, "latest": true}),
    );
    assert_eq!(page["messages"].as_array().unwrap().len(), 21);
    assert!(page.get("truncated").is_none());
}

#[test]
fn a_revoked_or_expired_run_leaves_the_snapshot() {
    let mut ws = Workspace::new("snapshot-runs");
    let (_, general) = ws.host_team("runs");
    let params = run_params(&ws, &general);
    let live = ws.host_ok("run.create", params.clone())["run"]["id"].clone();
    let revoked = ws.host_ok("run.create", params)["run"]["id"].clone();
    ws.host_ok("run.revoke", json!({"run_id": revoked}));
    let snapshot = ws.host_snapshot();
    let runs: Vec<&Value> = snapshot["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| &run["id"])
        .collect();
    assert_eq!(runs, vec![&live]);
    assert_eq!(snapshot["totals"]["runs"], 1);
}
