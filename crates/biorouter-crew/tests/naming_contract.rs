//! Broker contract for human-readable names, slices S1a and S2a of
//! `docs/research/biorouter-crew/naming-design.md` ("Tests", "Broker").
//!
//! S1a: former principals, invitation enrichment, the `people` and `channel_names` maps, one
//! active principal per username (D3), display-name rules, `profile.suggest`,
//! `expected_username`, `channel.transfer`'s active successor, devices, `host_principal_id`,
//! and `hello` v1 and v2.
//!
//! S2a: team and channel uniqueness, canonical channel slugs, the byte-identical collision
//! refusal and its rate limit, visible-only `name_conflict`, rename authority, workspace names
//! and the sibling probe, `start --name`, and a generated legacy journal that replays with its
//! flags set and heals by rename.
//!
//! Tests that sign `hello` or run a real broker need Linux (a machine identity and
//! `SO_PEERCRED`); the rest run on every Unix.
#![cfg(unix)]

mod support;

use biorouter_crew::Connection;
use serde_json::{json, Value};
use support::*;

const BOB: u32 = 70_001;
const CAROL: u32 = 70_002;
const DAVE: u32 = 70_003;
const EVE: u32 = 70_004;

const TEAM_TAKEN: &str = "name_taken: A team with this name, or one that looks like it, already exists in this workspace. Choose a different name.";
const CHANNEL_TAKEN: &str = "name_taken: A channel with this name, or one that looks like it, already exists in this team. Choose a different name.";
const RATE_LIMITED: &str = "rate_limited: Too many name attempts. Try again later.";

fn ids(list: &Value) -> Vec<String> {
    list.as_array()
        .expect("a list")
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect()
}

fn find<'a>(list: &'a Value, id: &str) -> &'a Value {
    list.as_array()
        .expect("a list")
        .iter()
        .find(|item| item["id"] == id)
        .unwrap_or_else(|| panic!("{id} missing from {list}"))
}

fn channel_create(ws: &mut Workspace, member: &mut Member, team: &str, name: &str) -> Value {
    ws.call_ok(
        member,
        "channel.create",
        json!({"team_id": team, "name": name}),
    )
}

fn host_channel(ws: &mut Workspace, team: &str, name: &str) -> String {
    ws.host_ok("channel.create", json!({"team_id": team, "name": name}))["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// The host invites `member` to `channel` and the member accepts.
fn host_adds_to_channel(ws: &mut Workspace, member: &mut Member, channel: &str) {
    let invitation = ws.host_ok(
        "invitation.create",
        json!({"kind": "channel", "target_id": channel, "principal_id": member.principal_id}),
    );
    ws.call_ok(
        member,
        "invitation.accept",
        json!({"invitation_id": invitation["id"]}),
    );
}

// ---------------------------------------------------------------------------------------------
// S1a
// ---------------------------------------------------------------------------------------------

#[test]
fn former_principals_list_only_inactive_people_the_viewer_can_reach() {
    let mut ws = Workspace::new("former-principals");
    let mut bob = ws.enroll(BOB, "bob", 11);
    let mut carol = ws.enroll(CAROL, "carol", 12);
    let mut dave = ws.enroll(DAVE, "dave", 13);
    let eve = ws.enroll(EVE, "eve", 14);
    let (team, _) = ws.host_team("Analysis Lab");
    ws.host_adds_to_team(&mut bob, &team);
    ws.host_adds_to_team(&mut dave, &team);
    ws.create_team(&mut carol, "Carol Team");
    ws.offboard(&dave.principal_id);
    ws.offboard(&eve.principal_id);

    let host = ws.host_snapshot();
    assert_eq!(
        ids(&host["former_principals"]),
        vec![dave.principal_id.clone()]
    );
    let former = &host["former_principals"][0];
    assert_eq!(former["username"], "dave");
    assert_eq!(former["display_name"], "dave");
    assert_eq!(former["active"], false);
    assert!(
        former.get("uid").is_none(),
        "a former principal projects no UID: {former}"
    );
    assert!(!ids(&host["principals"]).contains(&dave.principal_id));
    assert!(!ids(&host["principals"]).contains(&eve.principal_id));

    let bob_view = ws.snapshot(&mut bob);
    assert_eq!(
        ids(&bob_view["former_principals"]),
        vec![dave.principal_id.clone()],
        "a teammate sees the former member of their team"
    );
    let carol_view = ws.snapshot(&mut carol);
    assert_eq!(
        carol_view["former_principals"],
        json!([]),
        "nothing carol can see references an inactive principal"
    );
}

#[test]
fn invitations_name_their_target_and_inviter_and_hide_expired_ones_from_the_invitee() {
    let mut ws = Workspace::new("invitation-enrichment");
    ws.host_ok("profile.update", json!({"nickname": "Alice Chen"}));
    let mut bob = ws.enroll(BOB, "bob", 11);
    let mut carol = ws.enroll(CAROL, "carol", 12);
    let (team, _) = ws.host_team("Analysis Lab");
    let pending = ws.host_ok(
        "invitation.create",
        json!({"kind": "team", "target_id": team, "principal_id": bob.principal_id}),
    );

    let bob_view = ws.snapshot(&mut bob);
    let invitation = find(&bob_view["invitations"], pending["id"].as_str().unwrap());
    assert_eq!(invitation["target_name"], "Analysis Lab");
    assert_eq!(
        invitation["inviter"],
        json!({"username": "alice", "display_name": "Alice Chen"})
    );
    assert_eq!(invitation["expired"], false);
    assert!(invitation.get("team_name").is_none());
    assert_eq!(ws.snapshot(&mut carol)["invitations"], json!([]));

    ws.host_adds_to_team(&mut carol, &team);
    let methods = host_channel(&mut ws, &team, "methods");
    let channel_invite = ws.host_ok(
        "invitation.create",
        json!({"kind": "channel", "target_id": methods, "principal_id": carol.principal_id}),
    );
    let carol_view = ws.snapshot(&mut carol);
    let invitation = find(
        &carol_view["invitations"],
        channel_invite["id"].as_str().unwrap(),
    );
    assert_eq!(invitation["target_name"], "methods");
    assert_eq!(invitation["team_name"], "Analysis Lab");

    // An expired invitation, written as a legacy record: hidden from its invitee, marked for
    // its inviter, and never acceptable.
    let expired_id = uuid();
    let host_id = ws.host.principal_id.clone();
    let bob_id = bob.principal_id.clone();
    let team_id = team.clone();
    let mut ws = ws.edit_journal(|journal| {
        journal.append(
            &host_id,
            "invitation.create",
            vec![set(
                &["invitations", &expired_id],
                json!({"id": expired_id, "kind": "team", "target_id": team_id,
                       "principal_id": bob_id, "inviter_id": host_id, "expires_at": now() - 10}),
            )],
        );
    });
    let bob_view = ws.snapshot(&mut bob);
    assert!(!ids(&bob_view["invitations"]).contains(&expired_id));
    assert_eq!(ids(&bob_view["invitations"]).len(), 1);
    let host = ws.host_snapshot();
    assert_eq!(find(&host["invitations"], &expired_id)["expired"], true);
    assert_eq!(
        find(&host["invitations"], pending["id"].as_str().unwrap())["expired"],
        false
    );
    let (code, _) = refused(ws.call(
        &mut bob,
        "invitation.accept",
        json!({"invitation_id": expired_id}),
    ));
    assert_eq!(code, "forbidden");
}

/// A run over `sources` owned by the host, returning its worker credential.
fn host_run(ws: &mut Workspace, destination: &str, sources: &[&str]) -> String {
    let epoch = ws.broker.workspace().policy_epoch;
    let run = ws.host_ok(
        "run.create",
        json!({
            "channel_id": destination, "source_channels": sources,
            "provider_policy_id": "private", "personal_mode": "private",
            "public_provider": false, "expires_in": 60,
            "expected_workspace_policy_epoch": epoch,
            "workspace_institution_id": "ucsf", "connection_institution_id": "ucsf",
            "provider_affiliation": {"kind": "local"}, "expected_protected_context": true,
        }),
    );
    run["credential"].as_str().unwrap().to_owned()
}

fn worker(ws: &mut Workspace, credential: &str, method: &str, params: Value) -> Value {
    let mut req = request("worker", method, params);
    req.credential = Some(credential.into());
    let mut connection = Connection::new();
    let response = ws.broker.handle(host_uid(), &mut connection, req);
    ok(response)
}

#[test]
fn message_results_name_their_authors_and_only_readable_channels() {
    let mut ws = Workspace::new("people-map");
    let mut bob = ws.enroll(BOB, "bob", 11);
    let mut dave = ws.enroll(DAVE, "dave", 13);
    let (team, general) = ws.host_team("Analysis Lab");
    ws.host_adds_to_team(&mut bob, &team);
    ws.host_adds_to_team(&mut dave, &team);
    let hidden = host_channel(&mut ws, &team, "hidden");
    ws.call_ok(&mut dave, "profile.update", json!({"nickname": "Dave Old"}));

    let posted = ws.call_ok(
        &mut bob,
        "message.post",
        json!({"channel_id": general, "body": "from bob"}),
    );
    assert_eq!(
        posted["people"][&bob.principal_id],
        json!({"username": "bob", "display_name": "bob", "active": true})
    );
    assert_eq!(
        posted["channel_names"],
        json!({ general.clone(): "general" })
    );
    ws.call_ok(
        &mut dave,
        "message.post",
        json!({"channel_id": general, "body": "from dave"}),
    );
    ws.offboard(&dave.principal_id);

    // A worker post drawing on a channel bob cannot read.
    let credential = host_run(&mut ws, &general, &[&general, &hidden]);
    let projected = worker(
        &mut ws,
        &credential,
        "run.project",
        json!({"body": "summary", "status": "progress", "idempotency_key": "project-1"}),
    );
    assert_eq!(projected["channel_names"][&hidden], "hidden");

    let history = ws.call_ok(&mut bob, "messages.history", json!({"channel_id": general}));
    assert_eq!(history["messages"].as_array().unwrap().len(), 2);
    assert_eq!(
        history["people"][&dave.principal_id],
        json!({"username": "dave", "display_name": "Dave Old", "active": false})
    );
    assert_eq!(history["people"][&bob.principal_id]["active"], true);
    assert_eq!(
        history["channel_names"],
        json!({ general.clone(): "general" })
    );

    let host_history = ws.host_ok("messages.history", json!({"channel_id": general}));
    assert_eq!(host_history["messages"].as_array().unwrap().len(), 3);
    assert_eq!(host_history["channel_names"][&hidden], "hidden");
    assert!(host_history["people"][&ws.host.principal_id].is_object());

    let search = ws.call_ok(
        &mut bob,
        "messages.search",
        json!({"channel_id": general, "query": "from"}),
    );
    assert!(search["people"][&bob.principal_id].is_object());
    assert!(search["channel_names"].get(&hidden).is_none());

    let manifest = worker(&mut ws, &credential, "context.manifest", json!({}));
    assert_eq!(manifest["people"][&dave.principal_id]["active"], false);
    assert_eq!(manifest["channel_names"][&hidden], "hidden");
}

#[test]
fn a_second_active_username_is_refused_at_both_legacy_binds() {
    let mut ws = Workspace::new("one-active-username");
    let bob = ws.enroll(BOB, "bob", 11);

    ws.directory.set(70_010, "bob", None);
    let (code, message) = refused(ws.legacy_invite(70_010, &key(21)));
    assert_eq!(code, "identity_conflict");
    assert_eq!(
        message,
        "identity_conflict: another active member is @bob; remove the old @bob first"
    );
    ws.directory.set(70_011, "Bob", None);
    assert_eq!(
        refused(ws.legacy_invite(70_011, &key(22))).0,
        "identity_conflict",
        "a case-only difference is the same username"
    );

    // Invited while named robert, renamed to bob before redeeming: refused at the bind, and
    // nothing is written.
    ws.directory.set(70_012, "robert", None);
    let token = ok(ws.legacy_invite(70_012, &key(23)))["invitation"]
        .as_str()
        .unwrap()
        .to_owned();
    ws.directory.set(70_012, "bob", None);
    let before = ws.journal_bytes();
    let mut renamed = Member::new(70_012, key(23));
    assert_eq!(
        refused(ws.legacy_enroll(&mut renamed, &token)).0,
        "identity_conflict"
    );
    assert_eq!(ws.journal_bytes(), before);

    // Once the old @bob is offboarded, another account named bob may join.
    ws.offboard(&bob.principal_id);
    ws.enroll(70_013, "bob", 24);
}

#[test]
fn display_names_follow_the_rules_and_nickname_null_resets() {
    let mut ws = Workspace::new("display-names");
    let mut bob = ws.enroll(BOB, "bob", 11);
    let dave = ws.enroll(DAVE, "dave", 13);
    ws.offboard(&dave.principal_id);

    let too_long = "x".repeat(65);
    for refused_name in [
        "\u{202E}Bob",
        "\u{2066}Bob",
        "Bo\u{200B}b",
        "Bo\u{200D}b",
        "\u{FEFF}Bob",
        "\u{FE0F}",
        "\u{3164}",
        "Bob\u{0007}",
        "Bob @ Lab",
        "Bob \u{FF20} Lab",
        "",
        "   ",
        too_long.as_str(),
        "alice",
        "ALICE",
        "dave",
    ] {
        let (code, message) = refused(ws.call(
            &mut bob,
            "profile.update",
            json!({"nickname": refused_name}),
        ));
        assert_eq!(code, "name_invalid", "{refused_name:?} was not refused");
        assert!(
            !message.contains(refused_name) || refused_name.trim().is_empty(),
            "the refusal echoes the input: {message}"
        );
    }
    for accepted in ["李明 Li Ming", "bob", "Bob", "  Bob   Lee  "] {
        ws.call_ok(&mut bob, "profile.update", json!({"nickname": accepted}));
    }
    let actor = &ws.snapshot(&mut bob)["actor"];
    assert_eq!(actor["nickname"], "Bob Lee", "the stored name is cleaned");
    assert_eq!(actor["display_name"], "Bob Lee");

    let reset = ws.call_ok(&mut bob, "profile.update", json!({"nickname": null}));
    assert_eq!(reset["nickname"], "bob");
    assert_eq!(
        refused(ws.call(&mut bob, "profile.update", json!({"nickname": 7}))).0,
        "invalid_params"
    );
    assert_eq!(
        refused(ws.call(&mut bob, "profile.update", json!({"avatar": "B"}))).0,
        "invalid_params",
        "nickname stays required"
    );
}

#[test]
fn profile_suggest_offers_only_the_callers_own_valid_full_name() {
    let mut ws = Workspace::new("profile-suggest");
    let mut bob = ws.enroll(BOB, "bob", 11);
    let mut carol = ws.enroll(CAROL, "carol", 12);
    let mut dave = ws.enroll(DAVE, "dave", 13);
    let mut eve = ws.enroll(EVE, "eve", 14);
    ws.directory.set(BOB, "bob", Some("Bob Lee"));
    ws.directory.set(CAROL, "carol", Some("\u{202E}Carol"));
    ws.directory.set(DAVE, "dave", Some("alice"));

    let before = ws.directory.calls();
    let suggestion = ws.call_ok(&mut bob, "profile.suggest", json!({}));
    assert_eq!(suggestion, json!({"full_name": "Bob Lee"}));
    assert_eq!(
        ws.directory.calls() - before,
        2,
        "the request's own account check plus exactly one lookup for the suggestion"
    );
    assert_eq!(
        ws.call_ok(&mut carol, "profile.suggest", json!({})),
        json!({"full_name": null}),
        "an invalid full name is not suggested"
    );
    assert_eq!(
        ws.call_ok(&mut dave, "profile.suggest", json!({})),
        json!({"full_name": null}),
        "a full name that is another person's username is not suggested"
    );
    assert_eq!(
        ws.call_ok(&mut eve, "profile.suggest", json!({})),
        json!({"full_name": null})
    );
    let snapshot_before = ws.directory.calls();
    ws.snapshot(&mut bob);
    assert_eq!(
        ws.directory.calls() - snapshot_before,
        1,
        "the snapshot path makes no lookup of its own"
    );
    let journal = String::from_utf8(ws.journal_bytes()).unwrap();
    assert!(!journal.contains("Bob Lee"), "a suggestion is never stored");
}

#[test]
fn a_re_enrolled_person_cannot_accept_an_invitation_issued_to_the_old_one() {
    let mut ws = Workspace::new("generation-binding");
    let old_bob = ws.enroll(BOB, "bob", 11);
    let (team, _) = ws.host_team("Analysis Lab");
    let invitation = ws.host_ok(
        "invitation.create",
        json!({"kind": "team", "target_id": team, "principal_id": old_bob.principal_id}),
    );
    ws.offboard(&old_bob.principal_id);
    let mut new_bob = ws.enroll(BOB, "bob", 21);
    assert_ne!(new_bob.principal_id, old_bob.principal_id);
    let (code, _) = refused(ws.call(
        &mut new_bob,
        "invitation.accept",
        json!({"invitation_id": invitation["id"]}),
    ));
    assert_eq!(code, "forbidden");
    assert_eq!(ws.snapshot(&mut new_bob)["invitations"], json!([]));
}

#[test]
fn person_targeted_mutations_check_the_confirmed_username() {
    let mut ws = Workspace::new("expected-username");
    let mut bob = ws.enroll(BOB, "bob", 11);
    let mut carol = ws.enroll(CAROL, "carol", 12);
    let (team, _) = ws.host_team("Analysis Lab");
    ws.host_adds_to_team(&mut bob, &team);
    ws.host_adds_to_team(&mut carol, &team);
    let methods = host_channel(&mut ws, &team, "methods");

    let mismatch = |response: biorouter_crew::Response| {
        let (code, message) = refused(response);
        assert_eq!(code, "target_mismatch");
        assert!(!message.contains("carol") && !message.contains("bob"));
    };
    mismatch(ws.host_call(
        "invitation.create",
        json!({"kind": "channel", "target_id": methods, "principal_id": bob.principal_id,
               "expected_username": "carol"}),
    ));
    let invitation = ws.host_ok(
        "invitation.create",
        json!({"kind": "channel", "target_id": methods, "principal_id": bob.principal_id,
               "expected_username": "@bob"}),
    );
    ws.call_ok(
        &mut bob,
        "invitation.accept",
        json!({"invitation_id": invitation["id"]}),
    );

    mismatch(ws.host_call(
        "membership.revoke",
        json!({"channel_id": methods, "principal_id": bob.principal_id,
               "expected_username": "carol"}),
    ));
    assert!(ids(&ws.snapshot(&mut bob)["channels"]).contains(&methods));
    mismatch(ws.host_call(
        "channel.transfer",
        json!({"channel_id": methods, "successor_id": bob.principal_id,
               "expected_username": "Bob"}),
    ));
    ws.host_ok(
        "channel.transfer",
        json!({"channel_id": methods, "successor_id": bob.principal_id,
               "expected_username": "bob"}),
    );
    mismatch(ws.host_call(
        "enrollment.revoke",
        json!({"principal_id": carol.principal_id, "expected_username": "bob"}),
    ));
    assert!(ids(&ws.host_snapshot()["principals"]).contains(&carol.principal_id));
    assert_eq!(
        refused(ws.host_call(
            "enrollment.revoke",
            json!({"principal_id": carol.principal_id, "expected_username": 42}),
        ))
        .0,
        "invalid_params"
    );
    ws.host_ok(
        "enrollment.revoke",
        json!({"principal_id": carol.principal_id, "expected_username": "carol"}),
    );
    ws.host_ok(
        "membership.revoke",
        json!({"channel_id": methods, "principal_id": bob.principal_id,
               "expected_username": "bob"}),
    );
}

#[test]
fn channel_transfer_refuses_an_inactive_successor() {
    let mut ws = Workspace::new("inactive-successor");
    let mut bob = ws.enroll(BOB, "bob", 11);
    let (team, _) = ws.host_team("Analysis Lab");
    ws.host_adds_to_team(&mut bob, &team);
    let methods = host_channel(&mut ws, &team, "methods");
    host_adds_to_channel(&mut ws, &mut bob, &methods);
    ws.offboard(&bob.principal_id);
    let (code, message) = refused(ws.host_call(
        "channel.transfer",
        json!({"channel_id": methods, "successor_id": bob.principal_id}),
    ));
    assert_eq!(code, "forbidden");
    assert_eq!(message, "forbidden: eligible successor required");
}

#[test]
fn a_person_sees_their_own_devices_and_nobody_elses() {
    let mut ws = Workspace::new("devices");
    // Taken before bob's first device is stamped: both clocks read whole
    // seconds, so sampling after the enroll races a second boundary.
    let started = now();
    let mut bob = ws.enroll(BOB, "bob", 11);

    let host = ws.host_snapshot();
    let devices = host["actor"]["devices"].as_array().unwrap();
    assert_eq!(devices.len(), 1, "only the host's own device");
    let fingerprint = devices[0]["fingerprint"].as_str().unwrap();
    let expected: String = device_id(&ws.host.key)
        .to_uppercase()
        .chars()
        .take(16)
        .collect::<Vec<_>>()
        .chunks(4)
        .map(|group| group.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(fingerprint, expected);
    assert_eq!(devices[0]["added_via"], "bootstrap");

    // A second device for bob, by the legacy add-device path.
    let second = key(31);
    let token = ok(ws.host_call(
        "enrollment.invite",
        json!({"uid": BOB, "public_key": key_hex(&second),
               "existing_principal_id": bob.principal_id}),
    ))["invitation"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut bob_laptop = Member::new(BOB, second);
    ok(ws.legacy_enroll(&mut bob_laptop, &token));
    let bob_view = ws.snapshot(&mut bob);
    let devices = bob_view["actor"]["devices"].as_array().unwrap();
    assert_eq!(devices.len(), 2);
    for device in devices {
        assert_eq!(device["added_via"], "token");
        assert!(device["added_at"].as_u64().unwrap() >= started);
        assert!(!contains_machine_id(&device.to_string()));
    }
    assert!(
        ws.host_snapshot()["principals"]
            .as_array()
            .unwrap()
            .iter()
            .all(|principal| principal.get("devices").is_none()),
        "other people's devices are never projected"
    );
}

#[test]
fn a_key_that_is_already_a_device_is_never_bound_again() {
    let mut ws = Workspace::new("device-reuse");
    let bob = ws.enroll(BOB, "bob", 11);
    ws.directory.set(CAROL, "carol", None);
    for reused in [bob.key.clone(), ws.host.key.clone()] {
        let (code, _) = refused(ws.legacy_invite(CAROL, &reused));
        assert_eq!(code, "device_conflict");
    }

    // Two invitations for one key, redeemed one after the other: the second bind is refused
    // and writes nothing.
    ws.directory.set(DAVE, "dave", None);
    ws.directory.set(EVE, "eve", None);
    let shared = key(41);
    let first = ok(ws.legacy_invite(DAVE, &shared))["invitation"]
        .as_str()
        .unwrap()
        .to_owned();
    let second = ok(ws.legacy_invite(EVE, &shared))["invitation"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(ws.legacy_enroll(&mut Member::new(DAVE, shared.clone()), &first));
    let before = ws.journal_bytes();
    let (code, _) = refused(ws.legacy_enroll(&mut Member::new(EVE, shared), &second));
    assert_eq!(code, "device_conflict");
    assert_eq!(ws.journal_bytes(), before);
}

#[test]
fn every_snapshot_names_the_host_without_storing_it() {
    let mut ws = Workspace::new("host-principal");
    let mut bob = ws.enroll(BOB, "bob", 11);
    let host_id = ws.host.principal_id.clone();
    assert_eq!(
        ws.host_snapshot()["workspace"]["host_principal_id"],
        host_id
    );
    assert_eq!(
        ws.snapshot(&mut bob)["workspace"]["host_principal_id"],
        host_id
    );
    let journal = String::from_utf8(ws.journal_bytes()).unwrap();
    assert!(!journal.contains("host_principal_id"));
    assert!(!journal.contains("display_name"));
    assert!(!journal.contains("former_principals"));
}

// ---------------------------------------------------------------------------------------------
// S2a
// ---------------------------------------------------------------------------------------------

#[test]
fn team_names_are_unique_whatever_the_spelling() {
    let mut ws = Workspace::new("team-uniqueness");
    let (lab, _) = ws.host_team("Lab");
    for taken in ["lab", "LAB", "Analysis Lab"] {
        if taken == "Analysis Lab" {
            ws.host_team(taken);
            continue;
        }
        assert_eq!(
            refused(ws.host_call("team.create", json!({"name": taken}))),
            ("name_taken".into(), TEAM_TAKEN.into()),
            "{taken}"
        );
    }
    for taken in [
        "analysis-lab",
        "ANALYSIS_LAB",
        "Analysis.Lab",
        "AnaIysis Lab",
    ] {
        assert_eq!(
            refused(ws.host_call("team.create", json!({"name": taken}))),
            ("name_taken".into(), TEAM_TAKEN.into()),
            "{taken}"
        );
    }
    // Width and invisible variants never reach the uniqueness check: they are not valid team
    // names at all.
    for invalid in ["\u{FF2C}\u{FF41}\u{FF42}", "Lab\u{FE0F}"] {
        assert_eq!(
            refused(ws.host_call("team.create", json!({"name": invalid}))).0,
            "name_invalid",
            "{invalid:?}"
        );
    }
    let cleaned = ws.host_ok("team.create", json!({"name": "  Data   Team "}));
    assert_eq!(cleaned["team"]["name"], "Data Team");
    let renamed = ws.host_ok("team.rename", json!({"team_id": lab, "name": "LAB"}));
    assert_eq!(
        renamed["name"], "LAB",
        "a team may change only its own case"
    );
}

#[test]
fn channel_names_are_canonical_slugs_unique_within_their_team() {
    let mut ws = Workspace::new("channel-uniqueness");
    let (lab, lab_general) = ws.host_team("Lab");
    let (other, _) = ws.host_team("Other");
    let data = ws.host_ok(
        "channel.create",
        json!({"team_id": lab, "name": "Data Analysis"}),
    );
    assert_eq!(data["name"], "data-analysis");
    for taken in ["data_analysis", "Data.Analysis", "#data-analysis"] {
        assert_eq!(
            refused(ws.host_call("channel.create", json!({"team_id": lab, "name": taken}),)),
            ("name_taken".into(), CHANNEL_TAKEN.into()),
            "{taken}"
        );
    }
    ws.host_ok(
        "channel.create",
        json!({"team_id": other, "name": "data-analysis"}),
    );
    assert_eq!(
        ws.host_ok(
            "channel.create",
            json!({"team_id": lab, "name": "#methods"})
        )["name"],
        "methods"
    );

    // Archived channels keep their names.
    ws.host_ok("channel.archive", json!({"channel_id": data["id"]}));
    assert_eq!(
        refused(ws.host_call(
            "channel.create",
            json!({"team_id": lab, "name": "data-analysis"}),
        ))
        .1,
        CHANNEL_TAKEN
    );

    // `general` belongs to the team's first channel, even after that channel is renamed.
    let reserved =
        refused(ws.host_call("channel.create", json!({"team_id": lab, "name": "General"})));
    assert_eq!(reserved.0, "name_invalid");
    ws.host_ok(
        "channel.rename",
        json!({"channel_id": lab_general, "name": "announcements"}),
    );
    assert_eq!(
        refused(ws.host_call("channel.create", json!({"team_id": lab, "name": "general"}),)),
        reserved
    );
    ws.host_ok(
        "channel.rename",
        json!({"channel_id": lab_general, "name": "general"}),
    );
}

#[test]
fn names_shaped_like_ids_or_selectors_are_invalid() {
    let mut ws = Workspace::new("id-shaped");
    let (team, _) = ws.host_team("Lab");
    let uuid_name = uuid();
    let hex_name = "a".repeat(64);
    for invalid in [
        uuid_name.as_str(),
        hex_name.as_str(),
        "Lab/Ops",
        "Lab: Ops",
        "@lab",
    ] {
        assert_eq!(
            refused(ws.host_call("team.create", json!({"name": invalid}))).0,
            "name_invalid",
            "{invalid}"
        );
    }
    for invalid in [
        uuid_name.as_str(),
        hex_name.as_str(),
        "a/b",
        "@methods",
        "_methods",
    ] {
        assert_eq!(
            refused(ws.host_call("channel.create", json!({"team_id": team, "name": invalid}),)).0,
            "name_invalid",
            "{invalid}"
        );
    }
    assert!(
        ws.host_snapshot().get("name_collision_refusals") == Some(&json!({})),
        "an invalid name is not a collision"
    );
}

#[test]
fn a_collision_refusal_is_identical_for_visible_and_hidden_objects() {
    let mut ws = Workspace::new("identical-refusal");
    let mut bob = ws.enroll(BOB, "bob", 11);
    let (secret, _) = ws.create_team(&mut bob, "Secret Plans");
    let (lab, _) = ws.host_team("Lab");

    let hidden = refused(ws.host_call("team.create", json!({"name": "secret plans"})));
    let visible = refused(ws.host_call("team.create", json!({"name": "lab"})));
    assert_eq!(hidden, visible);
    assert_eq!(hidden.1, TEAM_TAKEN);
    for leak in ["Secret", "secret", "bob", &secret, &bob.principal_id] {
        assert!(!hidden.1.contains(leak), "the refusal names {leak}");
    }

    ws.host_adds_to_team(&mut bob, &lab);
    let restricted = channel_create(&mut ws, &mut bob, &lab, "hidden-plans");
    assert!(!ids(&ws.host_snapshot()["channels"])
        .contains(&restricted["id"].as_str().unwrap().to_owned()));
    host_channel(&mut ws, &lab, "methods");
    let hidden = refused(ws.host_call(
        "channel.create",
        json!({"team_id": lab, "name": "hidden_plans"}),
    ));
    let visible =
        refused(ws.host_call("channel.create", json!({"team_id": lab, "name": "Methods"})));
    assert_eq!(hidden, visible);
    assert_eq!(hidden.1, CHANNEL_TAKEN);
}

#[test]
fn the_eleventh_name_refusal_in_ten_minutes_gets_the_generic_answer() {
    let mut ws = Workspace::new("name-rate-limit");
    let mut bob = ws.enroll(BOB, "bob", 11);
    let mut carol = ws.enroll(CAROL, "carol", 12);
    ws.host_team("Lab");
    for attempt in 1..=10 {
        assert_eq!(
            refused(ws.call(&mut bob, "team.create", json!({"name": "lab"}))).1,
            TEAM_TAKEN,
            "attempt {attempt}"
        );
    }
    for name in ["lab", "A Fresh Team"] {
        assert_eq!(
            refused(ws.call(&mut bob, "team.create", json!({"name": name}))),
            ("rate_limited".into(), RATE_LIMITED.into()),
            "{name}: a limited answer must not say whether the name is taken"
        );
    }
    let (carol_team, _) = ws.create_team(&mut carol, "Carol Team");
    assert_eq!(
        refused(ws.call(
            &mut bob,
            "team.rename",
            json!({"team_id": carol_team, "name": "x"})
        ))
        .0,
        "rate_limited",
        "renames are limited too"
    );

    // Invalid names never count.
    for _ in 0..12 {
        assert_eq!(
            refused(ws.call(&mut carol, "team.create", json!({"name": "a/b"}))).0,
            "name_invalid"
        );
    }
    ws.create_team(&mut carol, "Another Carol Team");

    let host = ws.host_snapshot();
    assert_eq!(
        host["name_collision_refusals"],
        json!({ bob.principal_id.clone(): 10 })
    );
    assert!(ws
        .snapshot(&mut bob)
        .get("name_collision_refusals")
        .is_none());
}

#[test]
fn name_conflict_is_computed_only_among_objects_the_viewer_can_see() {
    let mut ws = Workspace::new("visible-conflict");
    let mut bob = ws.enroll(BOB, "bob", 11);
    let host_id = ws.host.principal_id.clone();
    let bob_id = bob.principal_id.clone();
    let (host_lab, host_general) = (uuid(), uuid());
    let (bob_lab, bob_general) = (uuid(), uuid());
    let (shared, shared_general) = (uuid(), uuid());
    let written = (host_lab.clone(), bob_lab.clone(), shared.clone());
    let mut ws = ws.edit_journal(|journal| {
        journal.append(
            &host_id,
            "team.create",
            vec![
                set(
                    &["teams", &host_lab],
                    legacy_team(&host_lab, "Lab", &host_id, &[&host_id], &host_general),
                ),
                set(
                    &["channels", &host_general],
                    legacy_channel(&host_general, &host_lab, "general", &host_id, &[&host_id]),
                ),
                set(
                    &["teams", &bob_lab],
                    legacy_team(&bob_lab, "lab", &bob_id, &[&bob_id], &bob_general),
                ),
                set(
                    &["channels", &bob_general],
                    legacy_channel(&bob_general, &bob_lab, "general", &bob_id, &[&bob_id]),
                ),
            ],
        );
    });
    let conflict =
        |snapshot: &Value, id: &str| find(&snapshot["teams"], id)["name_conflict"].clone();
    let host = ws.host_snapshot();
    assert_eq!(
        conflict(&host, &written.0),
        false,
        "the duplicate is hidden from the host"
    );
    let bob_view = ws.snapshot(&mut bob);
    assert_eq!(
        conflict(&bob_view, &written.1),
        false,
        "and the host's from bob"
    );

    let host_id = ws.host.principal_id.clone();
    let bob_id = bob.principal_id.clone();
    let mut ws = ws.edit_journal(|journal| {
        journal.append(
            &host_id,
            "team.create",
            vec![
                set(
                    &["teams", &shared],
                    legacy_team(
                        &shared,
                        "LAB",
                        &host_id,
                        &[&host_id, &bob_id],
                        &shared_general,
                    ),
                ),
                set(
                    &["channels", &shared_general],
                    legacy_channel(
                        &shared_general,
                        &shared,
                        "general",
                        &host_id,
                        &[&host_id, &bob_id],
                    ),
                ),
            ],
        );
    });
    let host = ws.host_snapshot();
    assert_eq!(conflict(&host, &written.0), true);
    assert_eq!(conflict(&host, &written.2), true);
    let bob_view = ws.snapshot(&mut bob);
    assert_eq!(
        conflict(&bob_view, &written.1),
        true,
        "the owner of a hidden duplicate sees their own flag"
    );
    assert_eq!(conflict(&bob_view, &written.2), true);
}

#[test]
fn only_the_creator_owner_or_host_renames() {
    let mut ws = Workspace::new("rename-authority");
    let mut bob = ws.enroll(BOB, "bob", 11);
    let mut carol = ws.enroll(CAROL, "carol", 12);
    let (lab, _) = ws.host_team("Lab");
    ws.host_adds_to_team(&mut bob, &lab);

    for member in [&mut bob, &mut carol] {
        let (code, message) = refused(ws.call(
            member,
            "team.rename",
            json!({"team_id": lab, "name": "Bob Lab"}),
        ));
        assert_eq!(
            (code.as_str(), message.as_str()),
            ("forbidden", "forbidden: team creator required")
        );
    }
    let renamed = ws.host_ok(
        "team.rename",
        json!({"team_id": lab, "name": "Analysis Lab"}),
    );
    assert_eq!(renamed["id"], lab);
    assert_eq!(
        find(&ws.snapshot(&mut bob)["teams"], &lab)["name"],
        "Analysis Lab"
    );
    ws.create_team(&mut carol, "Lab");

    let methods = host_channel(&mut ws, &lab, "methods");
    host_adds_to_channel(&mut ws, &mut bob, &methods);
    assert_eq!(
        refused(ws.call(
            &mut bob,
            "channel.rename",
            json!({"channel_id": methods, "name": "mine"})
        ))
        .1,
        "forbidden: current owner required"
    );
    assert_eq!(
        ws.host_ok(
            "channel.rename",
            json!({"channel_id": methods, "name": "Methods Two"})
        )["name"],
        "methods-two"
    );

    assert_eq!(
        refused(ws.call(&mut bob, "workspace.rename", json!({"name": "lab"}))).0,
        "forbidden"
    );
    let too_long = "a".repeat(41);
    let uuid_name = uuid();
    for invalid in [
        "Lab",
        "-lab",
        "lab-",
        "lab_1",
        "lab.two",
        too_long.as_str(),
        uuid_name.as_str(),
    ] {
        assert_eq!(
            refused(ws.host_call("workspace.rename", json!({"name": invalid}))).0,
            "name_invalid",
            "{invalid}"
        );
    }
    assert_eq!(
        ws.host_ok("workspace.rename", json!({"name": "lab"}))["name"],
        "lab"
    );
    assert_eq!(ws.snapshot(&mut bob)["workspace"]["name"], "lab");
}

#[test]
fn a_generated_legacy_journal_replays_with_flags_and_heals_by_rename() {
    let mut ws = Workspace::new("legacy-journal");
    let host_id = ws.host.principal_id.clone();
    let mut bob = ws.enroll(BOB, "bob", 11);
    let bob_id = bob.principal_id.clone();
    let (old_bob, mallory, ghost) = (uuid(), uuid(), uuid());
    let (lab, lab_general, lab_second_general) = (uuid(), uuid(), uuid());
    let (lab_dup, lab_dup_general) = (uuid(), uuid());
    let (blank, blank_general) = (uuid(), uuid());
    ws.directory.set(70_020, "bob-old", None);
    ws.directory.set(70_021, "mallory", None);
    ws.directory.set(70_022, "ghost", None);
    let written = ws.journal_bytes();
    let mut ws = ws.edit_journal(|journal| {
        journal.append(
            "system",
            "legacy.people",
            vec![
                set(
                    &["principals", &old_bob],
                    legacy_principal(&old_bob, 70_020, "bob", "bob", true),
                ),
                set(
                    &["devices", &device_id(&key(51))],
                    legacy_device(&old_bob, &key(51)),
                ),
                set(
                    &["principals", &mallory],
                    legacy_principal(&mallory, 70_021, "mallory", "\u{202E}yrollaM", true),
                ),
                set(
                    &["principals", &ghost],
                    legacy_principal(&ghost, 70_022, "ghost", "alice", true),
                ),
            ],
        );
        journal.append(
            &host_id,
            "legacy.teams",
            vec![
                set(
                    &["teams", &lab],
                    legacy_team(&lab, "Lab", &host_id, &[&host_id, &bob_id], &lab_general),
                ),
                set(
                    &["channels", &lab_general],
                    legacy_channel(
                        &lab_general,
                        &lab,
                        "general",
                        &host_id,
                        &[&host_id, &bob_id],
                    ),
                ),
                set(
                    &["channels", &lab_second_general],
                    legacy_channel(
                        &lab_second_general,
                        &lab,
                        "general",
                        &host_id,
                        &[&host_id, &bob_id],
                    ),
                ),
                set(
                    &["teams", &lab_dup],
                    legacy_team(
                        &lab_dup,
                        "lab",
                        &host_id,
                        &[&host_id, &bob_id],
                        &lab_dup_general,
                    ),
                ),
                set(
                    &["channels", &lab_dup_general],
                    legacy_channel(
                        &lab_dup_general,
                        &lab_dup,
                        "general",
                        &host_id,
                        &[&host_id, &bob_id],
                    ),
                ),
                set(
                    &["teams", &blank],
                    legacy_team(&blank, "   ", &host_id, &[&host_id], &blank_general),
                ),
                set(
                    &["channels", &blank_general],
                    legacy_channel(
                        &blank_general,
                        &blank,
                        "Data Analysis",
                        &host_id,
                        &[&host_id],
                    ),
                ),
            ],
        );
    });
    assert!(
        ws.journal_bytes().starts_with(&written),
        "replay rewrote nothing"
    );

    let host = ws.host_snapshot();
    let team = |snapshot: &Value, id: &str| find(&snapshot["teams"], id).clone();
    let channel = |snapshot: &Value, id: &str| find(&snapshot["channels"], id).clone();
    assert_eq!(team(&host, &lab)["name_conflict"], true);
    assert_eq!(team(&host, &lab_dup)["name_conflict"], true);
    assert_eq!(team(&host, &lab)["handle"], "lab");
    let untitled = team(&host, &blank);
    assert_eq!(untitled["display_name"], "Untitled team");
    assert_eq!(untitled["name_invalid"], true);
    assert_eq!(untitled["name"], "   ", "the raw name stays for audit");
    assert_eq!(channel(&host, &lab_general)["name_conflict"], true);
    assert_eq!(channel(&host, &lab_second_general)["name_conflict"], true);
    let legacy_slug = channel(&host, &blank_general);
    assert_eq!(legacy_slug["name_invalid"], true);
    assert_eq!(legacy_slug["handle"], "data-analysis");
    assert_eq!(legacy_slug["display_name"], "Data Analysis");

    let principal = |snapshot: &Value, id: &str| find(&snapshot["principals"], id).clone();
    assert_eq!(principal(&host, &old_bob)["account_stale"], true);
    assert!(principal(&host, &bob_id).get("account_stale").is_none());
    assert_eq!(principal(&host, &mallory)["display_name"], "yrollaM");
    assert_eq!(principal(&host, &mallory)["nickname"], "\u{202E}yrollaM");
    assert_eq!(principal(&host, &ghost)["display_name"], "ghost");

    let bob_view = ws.snapshot(&mut bob);
    assert!(
        !bob_view.to_string().contains("account_stale"),
        "account_stale is for the host alone"
    );
    assert_eq!(team(&bob_view, &lab)["name_conflict"], true);

    // New names follow the rules; renaming one duplicate clears both flags.
    assert_eq!(
        refused(ws.host_call("team.create", json!({"name": "LAB"}))).1,
        TEAM_TAKEN
    );
    ws.host_ok(
        "team.rename",
        json!({"team_id": lab_dup, "name": "Lab Two"}),
    );
    ws.host_ok(
        "channel.rename",
        json!({"channel_id": lab_second_general, "name": "announcements"}),
    );
    let check = |host: &Value| {
        assert_eq!(find(&host["teams"], &lab)["name_conflict"], false);
        assert_eq!(find(&host["teams"], &lab_dup)["name_conflict"], false);
        assert_eq!(find(&host["teams"], &lab_dup)["name"], "Lab Two");
        assert_eq!(
            find(&host["channels"], &lab_general)["name_conflict"],
            false
        );
    };
    check(&ws.host_snapshot());

    // The journal written after replay replays again.
    let mut ws = ws.reopen();
    check(&ws.host_snapshot());
    ws.call_ok(
        &mut bob,
        "message.post",
        json!({"channel_id": lab_general, "body": "still here"}),
    );
}

#[test]
fn a_journal_without_pending_joins_is_untouched_on_open() {
    let ws = Workspace::new("no-upgrade-record");
    let before = ws.journal_bytes();
    let mut ws = ws.reopen();
    ws.host_snapshot();
    assert_eq!(
        ws.journal_bytes(),
        before,
        "opening and reading write nothing"
    );
    ws.host_team("Lab");
    let journal = String::from_utf8(ws.journal_bytes()).unwrap();
    assert!(journal.starts_with(std::str::from_utf8(&before).unwrap()));
    assert!(!journal.contains("pending_joins"));
    let ws = ws.reopen();
    drop(ws);
}

// ---------------------------------------------------------------------------------------------
// Linux: `hello`, the sibling probe and `start --name`
// ---------------------------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use biorouter_crew::{hello_v1_payload, HelloV2, Mode};
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::path::Path;

    fn hello(ws: &mut Workspace, nonce: &str) -> Value {
        let mut connection = Connection::new();
        ok(ws.broker.handle(
            BOB,
            &mut connection,
            request("hello", "hello", json!({"challenge_nonce": nonce})),
        ))
    }

    #[test]
    fn hello_returns_the_name_and_a_v2_signature_beside_v1() {
        let mut ws = Workspace::new("hello-v2");
        ws.host_ok("workspace.rename", json!({"name": "lab"}));
        let hello = hello(&mut ws, "nonce-1");
        assert_eq!(hello["protocol"], 1);
        assert_eq!(hello["name"], "lab");
        let capabilities: Vec<&str> = hello["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap())
            .collect();
        for capability in ["human_names_v1", "unique_names_v1"] {
            assert!(capabilities.contains(&capability));
        }
        assert_eq!(
            capabilities.contains(&"join_by_name_v1"),
            cfg!(feature = "join-by-name")
        );
        let key_hex = hello["workspace_public_key"].as_str().unwrap();
        let key =
            VerifyingKey::from_bytes(&hex::decode(key_hex).unwrap().try_into().unwrap()).unwrap();
        let signature = |field: &str| {
            Signature::from_slice(&hex::decode(hello[field].as_str().unwrap()).unwrap()).unwrap()
        };
        let workspace_id = hello["workspace_id"].as_str().unwrap();
        let node_id = hello["node_id"].as_str().unwrap();
        let host_uid = hello["host_uid"].as_u64().unwrap() as u32;
        key.verify(
            &hello_v1_payload(workspace_id, host_uid, "nonce-1", key_hex, node_id),
            &signature("signature"),
        )
        .expect("v1 still verifies");
        let v2 = HelloV2 {
            workspace_id,
            host_uid,
            challenge_nonce: "nonce-1",
            workspace_public_key: key_hex,
            node_id,
            mode: &Mode::Private,
            institution_id: Some("ucsf"),
            policy_epoch: hello["policy_epoch"].as_u64().unwrap(),
            name: Some("lab"),
            capabilities: &capabilities,
        };
        key.verify(&v2.signing_payload(), &signature("signature_v2"))
            .expect("v2 verifies over every listed field");
        let reordered: Vec<&str> = capabilities.iter().rev().copied().collect();
        for tampered in [
            HelloV2 {
                mode: &Mode::Public,
                ..v2
            },
            HelloV2 {
                institution_id: None,
                ..v2
            },
            HelloV2 {
                policy_epoch: v2.policy_epoch + 1,
                ..v2
            },
            HelloV2 {
                name: Some("lab2"),
                ..v2
            },
            HelloV2 { name: None, ..v2 },
            HelloV2 {
                capabilities: &reordered,
                ..v2
            },
            HelloV2 {
                challenge_nonce: "nonce-2",
                ..v2
            },
        ] {
            assert!(key
                .verify(&tampered.signing_payload(), &signature("signature_v2"))
                .is_err());
        }
    }

    /// A fake sibling broker in `runtime_root`: answers every `hello` for `workspace_id`
    /// named `name`.
    fn fake_sibling(runtime_root: &Path, workspace_id: &str, name: &str) {
        let directory = runtime_root.join(format!(
            "crew-{}-{}",
            host_uid(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&directory).unwrap();
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
    fn workspace_rename_refuses_a_name_a_running_sibling_uses() {
        let mut ws = Workspace::new("sibling-probe");
        let runtime = TempRoot::short();
        ws.broker.set_runtime_root(runtime.path());
        fake_sibling(runtime.path(), &uuid(), "lab");
        let own_id = ws.broker.workspace().id.clone();
        fake_sibling(runtime.path(), &own_id, "mine");
        // A stale directory with no listener is skipped.
        std::fs::create_dir(
            runtime
                .path()
                .join(format!("crew-{}-{}", host_uid(), "0".repeat(32))),
        )
        .unwrap();

        let (code, message) = refused(ws.host_call("workspace.rename", json!({"name": "lab"})));
        assert_eq!(code, "name_taken");
        assert!(!message.contains("lab"), "{message}");
        assert_eq!(
            ws.host_ok("workspace.rename", json!({"name": "mine"}))["name"],
            "mine"
        );
        assert_eq!(
            ws.host_ok("workspace.rename", json!({"name": "lab-two"}))["name"],
            "lab-two"
        );
    }

    fn crew(args: &[&str]) -> std::process::Output {
        std::process::Command::new(env!("CARGO_BIN_EXE_biorouter-crew"))
            .args(args)
            .output()
            .unwrap()
    }

    /// Stops a broker the test started, whatever happens to the test.
    struct Running(std::path::PathBuf);
    impl Drop for Running {
        fn drop(&mut self) {
            let state = self.0.to_str().unwrap();
            let runtime: Option<Value> = std::fs::read(self.0.join("runtime.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok());
            let _ = crew(&["stop", "--state-dir", state]);
            if let Some(socket) = runtime.as_ref().and_then(|info| info["socket"].as_str()) {
                let socket = Path::new(socket);
                let _ = std::fs::remove_file(socket);
                if let Some(directory) = socket.parent() {
                    let _ = std::fs::remove_dir(directory);
                }
            }
        }
    }

    #[test]
    fn start_with_a_name_prints_an_invitation_and_refuses_a_running_duplicate() {
        if host_uid() == 0 || !Path::new("/etc/machine-id").exists() {
            eprintln!("skipped: a real broker needs an ordinary user and /etc/machine-id");
            return;
        }
        let parent = TempRoot::new("start-name");
        let name = format!("t{}-{}", std::process::id(), now() % 1_000_000);
        let first = parent.path().join("first");
        let first_state = first.to_str().unwrap();
        let output = crew(&[
            "start",
            "--state-dir",
            first_state,
            "--name",
            &name,
            "--bootstrap-key",
            &key_hex(&key(7)),
        ]);
        let _running = Running(first.clone());
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let started: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(started["state"], "running");
        assert_eq!(started["name"], name.as_str());
        let line = started["invitation"].as_str().expect("an invitation line");
        assert!(line.starts_with("brcrew1:"));
        let parsed = biorouter_crew::invitation::parse(&String::from_utf8_lossy(&output.stdout))
            .expect("the printed output parses as an invitation");
        let invitation = parsed.invitation;
        assert_eq!(invitation.workspace_name.as_deref(), Some(name.as_str()));
        assert_eq!(invitation.owner_uid, host_uid());
        assert_eq!(invitation.mode, Some(Mode::Private));
        assert!(invitation.host_username.is_some());
        assert_eq!(
            Some(invitation.workspace_id.as_str()),
            started["workspace_id"].as_str()
        );

        let second = parent.path().join("second");
        let output = crew(&[
            "start",
            "--state-dir",
            second.to_str().unwrap(),
            "--name",
            &name,
            "--bootstrap-key",
            &key_hex(&key(8)),
        ]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("name_taken"));
        assert!(
            !second.join("journal.jsonl").exists(),
            "nothing was spawned"
        );

        let third = parent.path().join("third");
        let output = crew(&[
            "start",
            "--state-dir",
            third.to_str().unwrap(),
            "--name",
            "Bad_Name",
            "--bootstrap-key",
            &key_hex(&key(9)),
        ]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("name_invalid"));
        assert!(
            !third.exists(),
            "an invalid name is refused before anything is created"
        );

        // Starting the same workspace again under another name is refused before the spawn.
        let output = crew(&["start", "--state-dir", first_state, "--name", "other-name"]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("name_mismatch"));
    }
}
