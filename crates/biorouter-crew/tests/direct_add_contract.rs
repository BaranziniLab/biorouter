//! Broker contract for direct add (`direct_add_v1`): a team's owner or the workspace host adds an
//! already-admitted member straight into a team, its `#general` and chosen channels
//! (`team.add_member`), or into one channel of a team they are already in
//! (`channel.add_member`). There is no acceptance step, so every refusal below is the only thing
//! standing between a caller and someone else's membership:
//!
//! - only a person's own signed device, never an agent's worker grant;
//! - only the team's owner (or channel's owner) or the host, never another member;
//! - only an active principal whose username is exactly the one the caller confirmed, and whose
//!   server account still has that name (a renamed or recycled UID is refused);
//! - a listed channel outside the team, or one the caller can't see, refuses the whole add;
//! - a retry with the same idempotency key replays the first answer and writes nothing.
//!
//! ```text
//! cargo test -p biorouter-crew --test direct_add_contract
//! ```
#![cfg(unix)]

mod support;

use biorouter_crew::Connection;
use serde_json::{json, Value};
use support::*;

const BOB: u32 = 73_001;
const CAROL: u32 = 73_002;
const DAVE: u32 = 73_003;

/// A channel the host creates in `team_id`.
fn host_channel(ws: &mut Workspace, team_id: &str, name: &str) -> String {
    let channel = ws.host_ok("channel.create", json!({"team_id": team_id, "name": name}));
    channel["id"].as_str().unwrap().to_owned()
}

/// A channel `member` creates in `team_id`.
fn member_channel(ws: &mut Workspace, member: &mut Member, team_id: &str, name: &str) -> String {
    let channel = ws.call_ok(
        member,
        "channel.create",
        json!({"team_id": team_id, "name": name}),
    );
    channel["id"].as_str().unwrap().to_owned()
}

fn epoch(ws: &Workspace) -> u64 {
    ws.broker.workspace().policy_epoch
}

fn journal_lines(ws: &Workspace) -> usize {
    String::from_utf8(ws.journal_bytes())
        .unwrap()
        .lines()
        .count()
}

fn last_record(ws: &Workspace) -> Value {
    let journal = String::from_utf8(ws.journal_bytes()).unwrap();
    serde_json::from_str(journal.lines().last().unwrap()).unwrap()
}

/// The IDs `member` sees in its snapshot's `field` (`teams` or `channels`).
fn visible(ws: &mut Workspace, member: &mut Member, field: &str) -> Vec<String> {
    ws.snapshot(member)[field]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["id"].as_str().unwrap().to_owned())
        .collect()
}

/// Whether `member` sees `team_id` in its own snapshot, which lists exactly its teams.
fn in_team(ws: &mut Workspace, member: &mut Member, team_id: &str) -> bool {
    visible(ws, member, "teams").iter().any(|id| id == team_id)
}

fn add(principal: &Member, username: &str, team_id: &str, channels: &[&str]) -> Value {
    json!({
        "team_id": team_id,
        "principal_id": principal.principal_id,
        "expected_username": username,
        "channel_ids": channels,
    })
}

#[test]
fn the_host_adds_a_member_to_the_team_its_general_and_the_chosen_channels() {
    let mut ws = Workspace::new("direct-add-host");
    let mut bob = ws.enroll(BOB, "bob", 11);
    let (team, general) = ws.host_team("Lab");
    let methods = host_channel(&mut ws, &team, "methods");
    let _quiet = host_channel(&mut ws, &team, "quiet");
    let before = epoch(&ws);

    let added = ws.host_ok("team.add_member", add(&bob, "@bob", &team, &[&methods]));
    assert_eq!(
        added,
        json!({
            "team_id": team,
            "principal_id": bob.principal_id,
            "added_channels": [general, methods],
            "already_member": false,
        })
    );
    // Membership changed, so the epoch moves, as it does when an invitation is accepted.
    assert_eq!(epoch(&ws), before + 1);
    // Journaled as the host's own act, under the method's name.
    let record = last_record(&ws);
    assert_eq!(record["operation"], "team.add_member");
    assert_eq!(record["actor"], ws.host.principal_id.as_str());

    // No acceptance step: Bob sees the team and exactly the channels he was added to.
    assert_eq!(visible(&mut ws, &mut bob, "teams"), vec![team.clone()]);
    let mut channels = visible(&mut ws, &mut bob, "channels");
    channels.sort();
    let mut expected = vec![general.clone(), methods.clone()];
    expected.sort();
    assert_eq!(channels, expected);
    ws.call_ok(
        &mut bob,
        "message.post",
        json!({"channel_id": methods, "body": "hello"}),
    );
}

#[test]
fn a_team_owner_who_is_not_the_host_adds_to_their_own_team_and_channels() {
    let mut ws = Workspace::new("direct-add-owner");
    let mut carol = ws.enroll(CAROL, "carol", 12);
    let mut bob = ws.enroll(BOB, "bob", 11);
    let (team, general) = ws.create_team(&mut carol, "Carol Lab");
    let analysis = member_channel(&mut ws, &mut carol, &team, "analysis");

    let added = ws.call_ok(
        &mut carol,
        "team.add_member",
        add(&bob, "bob", &team, &[&general, &analysis]),
    );
    assert_eq!(added["added_channels"], json!([general, analysis]));
    assert_eq!(added["already_member"], false);
    assert!(visible(&mut ws, &mut bob, "channels").contains(&analysis));
}

#[test]
fn a_retry_replays_the_first_answer_and_a_repeat_add_is_a_no_op() {
    let mut ws = Workspace::new("direct-add-retry");
    let bob = ws.enroll(BOB, "bob", 11);
    let (team, general) = ws.host_team("Lab");
    let mut params = add(&bob, "bob", &team, &[]);
    params["idempotency_key"] = json!("add-bob-once");

    let first = ws.host_ok("team.add_member", params.clone());
    assert_eq!(first["added_channels"], json!([general]));
    let (lines, after_first) = (journal_lines(&ws), epoch(&ws));

    // The same request again (a retry after an uncertain answer): the first answer, no write.
    let replay = ws.host_ok("team.add_member", params.clone());
    assert_eq!(replay, first);
    assert_eq!(journal_lines(&ws), lines);
    assert_eq!(epoch(&ws), after_first);

    // The same key with a different request is a conflict, not a second add.
    let mut different = params.clone();
    different["channel_ids"] = json!([general]);
    let (code, _) = refused(ws.host_call("team.add_member", different));
    assert_eq!(code, "conflict");

    // A new request for someone already in: success, nothing added, the epoch left alone.
    let again = ws.host_ok("team.add_member", add(&bob, "bob", &team, &[&general]));
    assert_eq!(again["already_member"], true);
    assert_eq!(again["added_channels"], json!([]));
    assert_eq!(epoch(&ws), after_first);
}

#[test]
fn a_member_who_does_not_own_the_team_or_channel_is_refused() {
    let mut ws = Workspace::new("direct-add-non-owner");
    let mut carol = ws.enroll(CAROL, "carol", 12);
    let dave = ws.enroll(DAVE, "dave", 13);
    let (team, _) = ws.host_team("Lab");
    let methods = host_channel(&mut ws, &team, "methods");
    ws.host_adds_to_team(&mut carol, &team);
    ws.host_ok(
        "channel.add_member",
        json!({"channel_id": methods, "principal_id": carol.principal_id, "expected_username": "carol"}),
    );
    let before = (epoch(&ws), journal_lines(&ws));

    // Carol is in the team and the channel, but owns neither.
    let (code, message) = refused(ws.call(
        &mut carol,
        "team.add_member",
        add(&dave, "dave", &team, &[]),
    ));
    assert_eq!(code, "forbidden");
    assert!(message.contains("team's owner"), "{message}");
    let (code, message) = refused(ws.call(
        &mut carol,
        "channel.add_member",
        json!({"channel_id": methods, "principal_id": dave.principal_id, "expected_username": "dave"}),
    ));
    assert_eq!(code, "forbidden");
    assert!(message.contains("channel's owner"), "{message}");

    // Dave, in no team at all, can't add himself or anyone to a team he can't see.
    let mut dave = dave;
    let himself = add(&dave, "dave", &team, &[]);
    let (code, message) = refused(ws.call(&mut dave, "team.add_member", himself));
    assert_eq!(code, "forbidden");
    assert_eq!(message, "forbidden: team unavailable");

    assert_eq!((epoch(&ws), journal_lines(&ws)), before);
    assert!(!in_team(&mut ws, &mut dave, &team));
}

#[test]
fn a_worker_grant_can_never_add_people() {
    let mut ws = Workspace::new("direct-add-worker");
    let mut bob = ws.enroll(BOB, "bob", 11);
    let (team, general) = ws.host_team("Lab");
    let epoch_now = epoch(&ws);
    let run = ws.host_ok(
        "run.create",
        json!({
            "channel_id": general, "source_channels": [general],
            "provider_policy_id": "private", "personal_mode": "private",
            "public_provider": false, "expires_in": 60,
            "expected_workspace_policy_epoch": epoch_now,
            "workspace_institution_id": "ucsf", "connection_institution_id": "ucsf",
            "provider_affiliation": {"kind": "local"}, "expected_protected_context": true,
        }),
    );
    let credential = run["credential"].as_str().unwrap().to_owned();
    let before = journal_lines(&ws);
    for (method, params) in [
        ("team.add_member", add(&bob, "bob", &team, &[])),
        (
            "channel.add_member",
            json!({"channel_id": general, "principal_id": bob.principal_id, "expected_username": "bob"}),
        ),
    ] {
        let mut params = params;
        params["idempotency_key"] = json!(format!("worker-{method}"));
        let mut req = request("worker", method, params);
        req.credential = Some(credential.clone());
        let (code, message) = refused(ws.broker.handle(host_uid(), &mut Connection::new(), req));
        assert_eq!(code, "forbidden", "{method}: {message}");
    }
    assert_eq!(journal_lines(&ws), before);
    assert!(!in_team(&mut ws, &mut bob, &team));
}

#[test]
fn a_changed_username_a_recycled_uid_or_a_former_member_is_refused() {
    let mut ws = Workspace::new("direct-add-identity");
    let bob = ws.enroll(BOB, "bob", 11);
    let carol = ws.enroll(CAROL, "carol", 12);
    let (team, _) = ws.host_team("Lab");
    let before = journal_lines(&ws);

    // The name the host confirmed is not this principal's.
    let (code, message) = refused(ws.host_call("team.add_member", add(&bob, "robert", &team, &[])));
    assert_eq!(code, "target_mismatch");
    assert!(message.contains("no longer has that username"), "{message}");

    // The confirmed name is required, not optional, for a direct add.
    let mut unconfirmed = add(&bob, "bob", &team, &[]);
    unconfirmed
        .as_object_mut()
        .unwrap()
        .remove("expected_username");
    let (code, _) = refused(ws.host_call("team.add_member", unconfirmed));
    assert_eq!(code, "invalid_params");

    // The server account behind Bob's UID was renamed, then recycled for someone else.
    for renamed in ["robert", "mallory"] {
        ws.directory.set(BOB, renamed, None);
        let (code, message) =
            refused(ws.host_call("team.add_member", add(&bob, "bob", &team, &[])));
        assert_eq!(code, "target_mismatch", "{renamed}");
        assert!(
            message.contains("account on this server changed"),
            "{message}"
        );
    }
    // The account is gone from the server.
    ws.directory.remove(BOB);
    let (code, _) = refused(ws.host_call("team.add_member", add(&bob, "bob", &team, &[])));
    assert_eq!(code, "target_mismatch");

    // A former member is not a member.
    ws.offboard(&carol.principal_id);
    let before = journal_lines(&ws).max(before);
    let (code, message) =
        refused(ws.host_call("team.add_member", add(&carol, "carol", &team, &[])));
    assert_eq!(code, "forbidden");
    assert!(
        message.contains("isn't a member of this workspace"),
        "{message}"
    );
    // Someone who never joined (an unknown principal ID) reads the same.
    let stranger = json!({"team_id": team, "principal_id": uuid(), "expected_username": "erin", "channel_ids": []});
    let (code, _) = refused(ws.host_call("team.add_member", stranger));
    assert_eq!(code, "forbidden");

    assert_eq!(journal_lines(&ws), before);
    let snapshot = ws.host_snapshot();
    let members = &snapshot["teams"][0]["members"];
    assert_eq!(members, &json!([ws.host.principal_id]));
}

#[test]
fn a_foreign_unknown_or_unowned_channel_refuses_the_whole_add() {
    let mut ws = Workspace::new("direct-add-atomic");
    let mut carol = ws.enroll(CAROL, "carol", 12);
    let mut bob = ws.enroll(BOB, "bob", 11);
    let mut dave = ws.enroll(DAVE, "dave", 13);
    let (host_team, _) = ws.host_team("Host Lab");
    let foreign = host_channel(&mut ws, &host_team, "foreign");
    let (team, _) = ws.create_team(&mut carol, "Carol Lab");
    let owned = member_channel(&mut ws, &mut carol, &team, "owned");
    ws.call_ok(&mut carol, "team.add_member", add(&bob, "bob", &team, &[]));
    // Bob's own channel in Carol's team: Carol can't see it, then can see it but doesn't own it.
    let bobs = member_channel(&mut ws, &mut bob, &team, "bobs");
    let before = (epoch(&ws), journal_lines(&ws));

    for (channels, code) in [
        (vec![owned.as_str(), foreign.as_str()], "invalid_params"),
        (vec![owned.as_str(), "not-a-channel"], "invalid_params"),
        (vec![owned.as_str(), bobs.as_str()], "invalid_params"),
    ] {
        let (refusal, message) = refused(ws.call(
            &mut carol,
            "team.add_member",
            add(&dave, "dave", &team, &channels),
        ));
        assert_eq!(refusal, code, "{channels:?}: {message}");
        // Invisible and foreign channels are indistinguishable from missing ones.
        assert!(
            !message.contains("bobs") && !message.contains("foreign"),
            "{message}"
        );
    }
    let (code, _) = refused(ws.call(
        &mut carol,
        "team.add_member",
        json!({"team_id": team, "principal_id": dave.principal_id, "expected_username": "dave", "channel_ids": "owned"}),
    ));
    assert_eq!(code, "invalid_params");

    // Now visible to Carol, but Bob owns it.
    ws.call_ok(
        &mut bob,
        "channel.add_member",
        json!({"channel_id": bobs, "principal_id": carol.principal_id, "expected_username": "carol"}),
    );
    let before = (before.0 + 1, journal_lines(&ws));
    let (code, message) = refused(ws.call(
        &mut carol,
        "team.add_member",
        add(&dave, "dave", &team, &[&owned, &bobs]),
    ));
    assert_eq!(code, "forbidden");
    assert!(message.contains("channels you own"), "{message}");

    // Nothing was added anywhere by any refused request.
    assert_eq!((epoch(&ws), journal_lines(&ws)), before);
    assert!(!in_team(&mut ws, &mut dave, &team));
    assert!(visible(&mut ws, &mut dave, "channels").is_empty());
}

#[test]
fn channel_add_member_adds_a_team_member_and_refuses_anyone_outside_the_team() {
    let mut ws = Workspace::new("direct-add-channel");
    let mut bob = ws.enroll(BOB, "bob", 11);
    let dave = ws.enroll(DAVE, "dave", 13);
    let (team, _) = ws.host_team("Lab");
    let methods = host_channel(&mut ws, &team, "methods");
    ws.host_ok("team.add_member", add(&bob, "bob", &team, &[]));
    assert!(!visible(&mut ws, &mut bob, "channels").contains(&methods));
    let before = epoch(&ws);

    let added = ws.host_ok(
        "channel.add_member",
        json!({"channel_id": methods, "principal_id": bob.principal_id, "expected_username": "@bob"}),
    );
    assert_eq!(
        added,
        json!({"channel_id": methods, "principal_id": bob.principal_id, "already_member": false})
    );
    assert_eq!(epoch(&ws), before + 1);
    assert!(visible(&mut ws, &mut bob, "channels").contains(&methods));
    let again = ws.host_ok(
        "channel.add_member",
        json!({"channel_id": methods, "principal_id": bob.principal_id, "expected_username": "bob"}),
    );
    assert_eq!(again["already_member"], true);
    assert_eq!(epoch(&ws), before + 1);

    let (code, message) = refused(ws.host_call(
        "channel.add_member",
        json!({"channel_id": methods, "principal_id": dave.principal_id, "expected_username": "dave"}),
    ));
    assert_eq!(code, "forbidden");
    assert!(
        message.contains("@dave isn't in this channel's team yet"),
        "{message}"
    );
}

#[test]
fn an_archived_channel_takes_no_one() {
    let mut ws = Workspace::new("direct-add-archived");
    let mut bob = ws.enroll(BOB, "bob", 11);
    let (team, _) = ws.host_team("Lab");
    let old = host_channel(&mut ws, &team, "old");
    ws.host_ok("channel.archive", json!({"channel_id": old}));
    let (code, _) = refused(ws.host_call("team.add_member", add(&bob, "bob", &team, &[&old])));
    assert_eq!(code, "channel_archived");
    assert!(!in_team(&mut ws, &mut bob, &team));
}

/// `hello` signs its capabilities, so this needs a machine identity (Linux).
#[cfg(target_os = "linux")]
#[test]
fn hello_advertises_direct_add() {
    let mut ws = Workspace::new("direct-add-hello");
    let hello = ok(ws.broker.handle(
        host_uid(),
        &mut Connection::new(),
        request("hello", "hello", json!({"challenge_nonce": "n"})),
    ));
    assert!(
        hello["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("direct_add_v1")),
        "{hello}"
    );
}
