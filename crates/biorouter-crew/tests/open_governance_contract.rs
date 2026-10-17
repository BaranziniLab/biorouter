//! Broker contract for open governance (`open_governance_v1`, OW-1 to OW-3 of
//! `docs/crew/design/open-workspaces-and-automatic-setup.md`):
//!
//! - a workspace is `managed` (today's rules, and every journal written before this change) or
//!   `open`, and only the host, as a person, changes it, without moving the policy epoch;
//! - under open governance every active member renames, archives and adds people to the teams
//!   and channels they are in, removes anyone but a channel's owner, invites, joins on their own
//!   (`team.join`, `channel.join`), and admits joiners; the same requests under managed rules
//!   are refused exactly as before;
//! - removing someone from the workspace, privacy, governance, renaming and deleting the
//!   workspace stay the host's;
//! - an open workspace admits a new member into every team and unarchived channel, gives a new
//!   team everyone and a new channel its whole team, and no addition ends an agent grant, while
//!   a removal still does;
//! - an agent's grant can do none of it.
//!
//! ```text
//! cargo test -p biorouter-crew --test open_governance_contract
//! ```
#![cfg(unix)]

mod support;

use biorouter_crew::{Connection, Request, Response};
use serde_json::{json, Value};
use support::*;

const BOB: u32 = 75_001;
const CAROL: u32 = 75_002;
const DAVE: u32 = 75_003;
const ERIN: u32 = 75_004;

fn epoch(ws: &Workspace) -> u64 {
    ws.broker.workspace().policy_epoch
}

fn journal_lines(ws: &Workspace) -> usize {
    String::from_utf8(ws.journal_bytes())
        .unwrap()
        .lines()
        .count()
}

fn open(ws: &mut Workspace) {
    assert_eq!(
        ws.host_ok("workspace.set_governance", json!({"governance": "open"})),
        json!({"governance": "open"})
    );
}

/// The IDs `member` sees in its snapshot's `field`.
fn ids(snapshot: &Value, field: &str) -> Vec<String> {
    let mut ids: Vec<String> = snapshot[field]
        .as_array()
        .unwrap_or_else(|| panic!("snapshot has no {field}: {snapshot}"))
        .iter()
        .map(|entry| entry["id"].as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    ids
}

fn sorted(ids: &[&str]) -> Vec<String> {
    let mut ids: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
    ids.sort();
    ids
}

/// The host adds `member` (`username`) to `team_id` and the listed channels.
fn host_adds(
    ws: &mut Workspace,
    member: &Member,
    username: &str,
    team_id: &str,
    channels: &[&str],
) {
    ws.host_ok(
        "team.add_member",
        json!({
            "team_id": team_id, "principal_id": member.principal_id,
            "expected_username": username, "channel_ids": channels,
        }),
    );
}

/// `run.create` parameters for a private run over `channel` in a workspace labelled `ucsf`.
fn run_params(ws: &Workspace, channel: &str) -> Value {
    json!({
        "channel_id": channel, "source_channels": [channel],
        "provider_policy_id": "private", "personal_mode": "private",
        "public_provider": false, "expires_in": 600,
        "expected_workspace_policy_epoch": epoch(ws),
        "expected_protected_context": true,
        "workspace_institution_id": "ucsf", "connection_institution_id": "ucsf",
        "provider_affiliation": {"kind": "local"},
    })
}

/// A private run of `member` over `channel`; returns the credential.
fn grant(ws: &mut Workspace, member: &mut Member, channel: &str) -> String {
    let params = run_params(ws, channel);
    ws.call_ok(member, "run.create", params)["credential"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// As [`grant`], for the host.
fn host_grant(ws: &mut Workspace, channel: &str) -> String {
    let params = run_params(ws, channel);
    ws.host_ok("run.create", params)["credential"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn worker(ws: &mut Workspace, uid: u32, credential: &str, method: &str, params: Value) -> Response {
    let mut params = params;
    params["idempotency_key"] = json!(format!("worker-{method}"));
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

/// The host's team `Lab` with `#methods`, Bob in both, and Carol and Dave in the workspace and
/// the team only. `(team, general, methods)`.
struct Lab {
    bob: Member,
    carol: Member,
    dave: Member,
    team: String,
    general: String,
    methods: String,
}

fn lab(ws: &mut Workspace) -> Lab {
    let bob = ws.enroll(BOB, "bob", 11);
    let carol = ws.enroll(CAROL, "carol", 12);
    let dave = ws.enroll(DAVE, "dave", 13);
    let (team, general) = ws.host_team("Lab");
    let methods = ws.host_ok(
        "channel.create",
        json!({"team_id": team, "name": "methods"}),
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    host_adds(ws, &bob, "bob", &team, &[&methods]);
    host_adds(ws, &carol, "carol", &team, &[]);
    host_adds(ws, &dave, "dave", &team, &[]);
    Lab {
        bob,
        carol,
        dave,
        team,
        general,
        methods,
    }
}

#[test]
fn only_the_host_as_a_person_sets_governance_and_it_never_moves_the_epoch() {
    let mut ws = Workspace::new("governance-set");
    let mut bob = ws.enroll(BOB, "bob", 11);
    let (_, general) = ws.host_team("Lab");
    // A journal written before governance existed reads as managed, and the field is stored only
    // when open, so such a journal re-serializes byte for byte.
    assert!(ws.broker.state_json()["workspace"]
        .get("governance")
        .is_none());
    assert_eq!(ws.host_snapshot()["workspace"]["governance"], "managed");
    let before = epoch(&ws);

    let (code, message) = refused(ws.call(
        &mut bob,
        "workspace.set_governance",
        json!({"governance": "open"}),
    ));
    assert_eq!(code, "forbidden", "{message}");
    let (code, _) =
        refused(ws.host_call("workspace.set_governance", json!({"governance": "anarchy"})));
    assert_eq!(code, "invalid_params");
    let credential = host_grant(&mut ws, &general);
    let lines = journal_lines(&ws);
    let (code, message) = refused(worker(
        &mut ws,
        host_uid(),
        &credential,
        "workspace.set_governance",
        json!({"governance": "open"}),
    ));
    assert_eq!(
        (code.as_str(), message.as_str()),
        ("forbidden", "forbidden: worker operation unavailable")
    );
    assert_eq!(journal_lines(&ws), lines);

    open(&mut ws);
    assert_eq!(epoch(&ws), before, "governance never ends an agent grant");
    assert_eq!(ws.broker.state_json()["workspace"]["governance"], "open");
    assert_eq!(ws.snapshot(&mut bob)["workspace"]["governance"], "open");
    // The host's own agent still runs: the epoch did not move.
    ok(worker(
        &mut ws,
        host_uid(),
        &credential,
        "context.manifest",
        json!({}),
    ));

    assert_eq!(
        ws.host_ok("workspace.set_governance", json!({"governance": "managed"})),
        json!({"governance": "managed"})
    );
    assert!(ws.broker.state_json()["workspace"]
        .get("governance")
        .is_none());
    // It survives a restart.
    open(&mut ws);
    let mut ws = ws.reopen();
    assert_eq!(ws.host_snapshot()["workspace"]["governance"], "open");
}

#[test]
fn renames_and_archives_are_any_members_only_when_open() {
    let mut ws = Workspace::new("governance-rename");
    let Lab {
        mut bob,
        team,
        methods,
        ..
    } = lab(&mut ws);
    let lines = journal_lines(&ws);
    for (method, params, refusal) in [
        (
            "team.rename",
            json!({"team_id": team, "name": "Bench"}),
            "forbidden: team creator required",
        ),
        (
            "channel.rename",
            json!({"channel_id": methods, "name": "protocols"}),
            "forbidden: current owner required",
        ),
        (
            "channel.archive",
            json!({"channel_id": methods}),
            "forbidden: current owner required",
        ),
    ] {
        let (_, message) = refused(ws.call(&mut bob, method, params));
        assert_eq!(message, refusal, "{method}");
    }
    assert_eq!(journal_lines(&ws), lines, "managed: nothing written");
    // Made before the workspace opens, so it has only the host in it.
    let (other, _) = ws.host_team("Other");

    open(&mut ws);
    assert_eq!(
        ws.call_ok(
            &mut bob,
            "team.rename",
            json!({"team_id": team, "name": "Bench"})
        )["name"],
        "Bench"
    );
    assert_eq!(
        ws.call_ok(
            &mut bob,
            "channel.rename",
            json!({"channel_id": methods, "name": "protocols"})
        )["name"],
        "protocols"
    );
    let before = epoch(&ws);
    assert_eq!(
        ws.call_ok(&mut bob, "channel.archive", json!({"channel_id": methods}))["archived"],
        true
    );
    assert_eq!(epoch(&ws), before + 1, "an archive still ends agent grants");
    // Membership is still what decides: a team Bob is not in is not his to rename.
    let (_, message) = refused(ws.call(
        &mut bob,
        "team.rename",
        json!({"team_id": other, "name": "Mine"}),
    ));
    assert_eq!(message, "forbidden: team creator required");
}

#[test]
fn invitations_are_any_members_when_open_and_accepting_rechecks_membership() {
    let mut ws = Workspace::new("governance-invite");
    let Lab {
        mut bob,
        mut carol,
        team,
        methods,
        ..
    } = lab(&mut ws);
    let invite_team = |principal: &Member| json!({"kind": "team", "target_id": team, "principal_id": principal.principal_id});
    let invite_channel = |principal: &Member| json!({"kind": "channel", "target_id": methods, "principal_id": principal.principal_id});
    let mut erin = ws.enroll(ERIN, "erin", 14);
    let (_, message) = refused(ws.call(&mut bob, "invitation.create", invite_team(&erin)));
    assert_eq!(message, "forbidden: team owner required");
    let (_, message) = refused(ws.call(&mut bob, "invitation.create", invite_channel(&carol)));
    assert_eq!(message, "forbidden: current owner required");

    open(&mut ws);
    let to_team = ws.call_ok(&mut bob, "invitation.create", invite_team(&erin));
    let to_channel = ws.call_ok(&mut bob, "invitation.create", invite_channel(&carol));
    let before = epoch(&ws);
    ws.call_ok(
        &mut erin,
        "invitation.accept",
        json!({"invitation_id": to_team["id"]}),
    );
    assert_eq!(
        epoch(&ws),
        before,
        "an accepted invitation in an open workspace ends no grant"
    );
    assert!(ids(&ws.snapshot(&mut erin), "teams").contains(&team));

    // Bob leaves #methods before Carol accepts: his authority to invite to it went with him.
    let bob_id = bob.principal_id.clone();
    ws.call_ok(
        &mut bob,
        "membership.revoke",
        json!({"channel_id": methods, "principal_id": bob_id}),
    );
    let (_, message) = refused(ws.call(
        &mut carol,
        "invitation.accept",
        json!({"invitation_id": to_channel["id"]}),
    ));
    assert_eq!(message, "forbidden: invitation authority changed");
}

#[test]
fn direct_adds_and_removals_are_any_members_when_open() {
    let mut ws = Workspace::new("governance-add");
    let Lab {
        mut bob,
        carol,
        dave,
        team,
        methods,
        ..
    } = lab(&mut ws);
    let mut erin = ws.enroll(ERIN, "erin", 14);
    let add_erin = |channels: &[&str]| {
        json!({
            "team_id": team, "principal_id": erin.principal_id,
            "expected_username": "erin", "channel_ids": channels,
        })
    };
    let add_carol = json!({
        "channel_id": methods, "principal_id": carol.principal_id, "expected_username": "carol",
    });
    let remove_dave = json!({"channel_id": methods, "principal_id": dave.principal_id, "expected_username": "dave"});
    let lines = journal_lines(&ws);
    let (_, message) = refused(ws.call(&mut bob, "team.add_member", add_erin(&[])));
    assert_eq!(
        message,
        "forbidden: Only the team's owner or the workspace host can add people to it."
    );
    let (_, message) = refused(ws.call(&mut bob, "channel.add_member", add_carol.clone()));
    assert_eq!(
        message,
        "forbidden: Only the channel's owner or the workspace host can add people to it."
    );
    assert_eq!(journal_lines(&ws), lines);
    // Made before the workspace opens, so only the host is in it.
    let private = ws.host_ok(
        "channel.create",
        json!({"team_id": team, "name": "private"}),
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();

    open(&mut ws);
    let credential = grant(&mut ws, &mut bob, &methods);
    let before = epoch(&ws);
    let added = ws.call_ok(&mut bob, "team.add_member", add_erin(&[&methods]));
    assert_eq!(added["already_member"], false);
    assert_eq!(added["added_channels"].as_array().unwrap().len(), 2);
    ws.call_ok(&mut bob, "channel.add_member", add_carol);
    assert_eq!(
        epoch(&ws),
        before,
        "additions in an open workspace end no grant"
    );
    ok(worker(
        &mut ws,
        BOB,
        &credential,
        "context.manifest",
        json!({}),
    ));
    assert!(ids(&ws.snapshot(&mut erin), "channels").contains(&methods));

    // Only into channels the adder is in themselves: the host's channel Bob is not in reads as
    // one outside the team.
    let (_, message) = refused(ws.call(
        &mut bob,
        "team.add_member",
        json!({
            "team_id": team, "principal_id": dave.principal_id,
            "expected_username": "dave", "channel_ids": [private],
        }),
    ));
    assert_eq!(
        message,
        "invalid_params: One of the chosen channels isn't in this team. Refresh and choose again."
    );

    // Removing anyone but the owner, and removal still ends grants.
    ws.host_ok(
        "channel.add_member",
        json!({"channel_id": methods, "principal_id": dave.principal_id, "expected_username": "dave"}),
    );
    let before = epoch(&ws);
    ws.call_ok(&mut bob, "membership.revoke", remove_dave);
    assert_eq!(epoch(&ws), before + 1, "a removal still ends agent grants");
    let (code, _) = refused(worker(
        &mut ws,
        BOB,
        &credential,
        "context.manifest",
        json!({}),
    ));
    assert_eq!(code, "grant_expired");
    let host = ws.host.principal_id.clone();
    let (_, message) = refused(ws.call(
        &mut bob,
        "membership.revoke",
        json!({"channel_id": methods, "principal_id": host}),
    ));
    assert_eq!(message, "forbidden: transfer before owner removal");
    // Only the owner transfers.
    let (_, message) = refused(ws.call(
        &mut bob,
        "channel.transfer",
        json!({"channel_id": methods, "successor_id": erin.principal_id}),
    ));
    assert_eq!(message, "forbidden: current owner required");
}

#[test]
fn members_join_teams_and_channels_on_their_own_only_when_open() {
    let mut ws = Workspace::new("governance-join");
    let Lab {
        mut bob,
        mut carol,
        team,
        general,
        methods,
        ..
    } = lab(&mut ws);
    let (other, other_general) = ws.host_team("Other");
    let archived = ws.host_ok("channel.create", json!({"team_id": team, "name": "old"}))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    ws.host_ok("channel.archive", json!({"channel_id": archived}));
    for (method, params) in [
        ("team.join", json!({"team_id": other})),
        ("channel.join", json!({"channel_id": methods})),
    ] {
        let (_, message) = refused(ws.call(&mut carol, method, params));
        assert_eq!(
            message,
            "forbidden: In this workspace, the owner or the host adds people. Ask them to add you."
        );
    }
    let snapshot = ws.snapshot(&mut carol);
    assert!(snapshot.get("joinable_teams").is_none());
    assert!(snapshot.get("joinable_channels").is_none());
    // A team none of them is in.
    let (third, third_general) = ws.host_team("Third");

    open(&mut ws);
    let snapshot = ws.snapshot(&mut carol);
    assert_eq!(ids(&snapshot, "joinable_teams"), sorted(&[&other, &third]));
    // Unarchived channels of Carol's own teams that she is not in; never another team's.
    assert_eq!(ids(&snapshot, "joinable_channels"), vec![methods.clone()]);
    let listed = &snapshot["joinable_channels"][0];
    assert_eq!(listed["team_id"], team.as_str());
    assert_eq!(listed["name"], "methods");
    assert_eq!(listed["handle"], "methods");
    assert_eq!(listed["classification"], "restricted");
    assert_eq!(listed["member_count"], 2, "the host and Bob");
    assert_eq!(snapshot["joinable_teams"][0]["member_count"], 1);
    assert_eq!(snapshot["totals"]["joinable_teams"], 2);
    assert_eq!(snapshot["totals"]["joinable_channels"], 1);

    let credential = grant(&mut ws, &mut bob, &general);
    let before = epoch(&ws);
    assert_eq!(
        ws.call_ok(&mut carol, "team.join", json!({"team_id": other})),
        json!({"team_id": other, "joined": true})
    );
    assert_eq!(
        ws.call_ok(&mut carol, "channel.join", json!({"channel_id": methods})),
        json!({"channel_id": methods, "joined": true})
    );
    assert_eq!(epoch(&ws), before, "joining ends nobody's grant");
    ok(worker(
        &mut ws,
        BOB,
        &credential,
        "context.manifest",
        json!({}),
    ));
    let snapshot = ws.snapshot(&mut carol);
    let teams = ids(&snapshot, "teams");
    assert!(teams.contains(&other));
    let channels = ids(&snapshot, "channels");
    assert!(channels.contains(&other_general) && channels.contains(&methods));
    assert_eq!(ids(&snapshot, "joinable_teams"), vec![third]);
    assert_eq!(snapshot["joinable_channels"], json!([]));

    let (code, _) = refused(ws.call(&mut carol, "channel.join", json!({"channel_id": archived})));
    assert_eq!(code, "channel_archived");
    // A channel of a team the joiner is not in reads as one that doesn't exist.
    let (_, message) = refused(ws.call(
        &mut bob,
        "channel.join",
        json!({"channel_id": third_general}),
    ));
    assert_eq!(message, "forbidden: channel unavailable");
}

#[test]
fn an_open_workspace_puts_everyone_everywhere() {
    let mut ws = Workspace::new("governance-everywhere");
    let mut bob = ws.enroll(BOB, "bob", 11);
    let carol = ws.enroll(CAROL, "carol", 12);
    let (team, general) = ws.host_team("Lab");
    let methods = ws.host_ok(
        "channel.create",
        json!({"team_id": team, "name": "methods"}),
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let old = ws.host_ok("channel.create", json!({"team_id": team, "name": "old"}))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    ws.host_ok("channel.archive", json!({"channel_id": old}));
    // Under managed rules a new member is added nowhere.
    let mut dave = ws.enroll(DAVE, "dave", 13);
    assert_eq!(ids(&ws.snapshot(&mut dave), "teams"), Vec::<String>::new());

    open(&mut ws);
    let before = epoch(&ws);
    let mut erin = ws.enroll(ERIN, "erin", 14);
    let snapshot = ws.snapshot(&mut erin);
    assert_eq!(ids(&snapshot, "teams"), vec![team.clone()]);
    assert_eq!(
        ids(&snapshot, "channels"),
        sorted(&[&general, &methods]),
        "every unarchived channel, never an archived one"
    );
    assert_eq!(epoch(&ws), before);

    // A new team has every active member, and so does its #general; the epoch stays.
    let created = ws.call_ok(&mut bob, "team.create", json!({"name": "Bench"}));
    let everyone = sorted(&[
        &ws.host.principal_id,
        &bob.principal_id,
        &carol.principal_id,
        &dave.principal_id,
        &erin.principal_id,
    ]);
    let members = |value: &Value| -> Vec<String> {
        let mut members: Vec<String> = serde_json::from_value(value["members"].clone()).unwrap();
        members.sort();
        members
    };
    assert_eq!(members(&created["team"]), everyone);
    assert_eq!(members(&created["channel"]), everyone);
    // An offboarded member is not put back anywhere.
    ws.offboard(&dave.principal_id);
    let bench = created["team"]["id"].as_str().unwrap().to_owned();
    let before = epoch(&ws);
    let channel = ws.call_ok(
        &mut bob,
        "channel.create",
        json!({"team_id": bench, "name": "notes"}),
    );
    let mut expected = everyone.clone();
    expected.retain(|id| *id != dave.principal_id);
    assert_eq!(members(&channel), expected, "the team's active members");
    assert_eq!(epoch(&ws), before);
}

#[test]
fn removal_from_the_workspace_privacy_governance_and_the_workspace_itself_stay_the_hosts() {
    let mut ws = Workspace::new("governance-host-only");
    let Lab { mut bob, carol, .. } = lab(&mut ws);
    open(&mut ws);
    let workspace_id = ws.broker.workspace().id.clone();
    let lines = journal_lines(&ws);
    for (method, params) in [
        (
            "enrollment.revoke",
            json!({"principal_id": carol.principal_id}),
        ),
        ("policy.set", json!({"mode": "public"})),
        ("workspace.rename", json!({"name": "mine"})),
        ("workspace.set_governance", json!({"governance": "managed"})),
        (
            "workspace.delete",
            json!({"workspace_id": workspace_id, "expected_name": null}),
        ),
        (
            "enrollment.invite",
            json!({"uid": ERIN, "public_key": key_hex(&key(14))}),
        ),
    ] {
        let (code, message) = refused(ws.call(&mut bob, method, params));
        assert_eq!(
            (code.as_str(), message.as_str()),
            ("forbidden", "forbidden: workspace host device required"),
            "{method}"
        );
    }
    assert_eq!(journal_lines(&ws), lines);
}

#[test]
fn an_agents_grant_can_do_none_of_it_even_when_open() {
    let mut ws = Workspace::new("governance-worker");
    let Lab {
        mut bob,
        carol,
        team,
        general,
        methods,
        ..
    } = lab(&mut ws);
    open(&mut ws);
    let credential = grant(&mut ws, &mut bob, &methods);
    let lines = journal_lines(&ws);
    for (method, params) in [
        ("team.rename", json!({"team_id": team, "name": "Agents"})),
        (
            "channel.rename",
            json!({"channel_id": methods, "name": "agents"}),
        ),
        ("channel.archive", json!({"channel_id": methods})),
        (
            "invitation.create",
            json!({"kind": "team", "target_id": team, "principal_id": carol.principal_id}),
        ),
        (
            "team.add_member",
            json!({"team_id": team, "principal_id": carol.principal_id, "expected_username": "carol"}),
        ),
        (
            "channel.add_member",
            json!({"channel_id": methods, "principal_id": carol.principal_id, "expected_username": "carol"}),
        ),
        (
            "membership.revoke",
            json!({"channel_id": general, "principal_id": carol.principal_id}),
        ),
        ("team.join", json!({"team_id": team})),
        ("channel.join", json!({"channel_id": methods})),
        ("enrollment.invite", json!({"username": "erin"})),
        (
            "enrollment.approve",
            json!({"username": "erin", "code": "0000000000000000"}),
        ),
        ("enrollment.cancel", json!({"username": "erin"})),
        ("workspace.set_governance", json!({"governance": "managed"})),
    ] {
        let (code, message) = refused(worker(&mut ws, BOB, &credential, method, params));
        assert_eq!(
            (code.as_str(), message.as_str()),
            ("forbidden", "forbidden: worker operation unavailable"),
            "{method}"
        );
    }
    assert_eq!(journal_lines(&ws), lines);
}

/// `wide_runs_v1`: a run may read up to 100 channels, so a member's agent can read every
/// channel they are in.
#[test]
fn a_run_reads_up_to_a_hundred_channels() {
    let mut ws = Workspace::new("governance-wide-runs");
    let (team, general) = ws.host_team("Lab");
    let channels: Vec<String> = (1..=100)
        .map(|n| {
            ws.host_ok(
                "channel.create",
                json!({"team_id": team, "name": format!("c{n}")}),
            )["id"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    let mut params = run_params(&ws, &general);
    params["source_channels"] = json!(channels[..99]);
    ws.host_ok("run.create", params.clone());
    params["source_channels"] = json!(channels);
    let (code, message) = refused(ws.host_call("run.create", params));
    assert_eq!(
        (code.as_str(), message.as_str()),
        (
            "quota_exceeded",
            "quota_exceeded: too many context channels"
        )
    );
}

/// A task created without a remote work folder is told where the folder is set: in the member's
/// own connection settings, not by anyone's approval.
#[test]
fn a_task_without_a_work_folder_is_told_where_to_set_one() {
    let mut ws = Workspace::new("governance-no-folder");
    let Lab {
        mut bob, general, ..
    } = lab(&mut ws);
    let credential = grant(&mut ws, &mut bob, &general);
    let sentence = "forbidden: This task has no remote work folder. Set one in Connection settings, then start a new task.";
    let (_, message) = refused(worker(
        &mut ws,
        BOB,
        &credential,
        "run.remote_scope",
        json!({}),
    ));
    assert_eq!(message, sentence);
    let (_, message) = refused(worker(
        &mut ws,
        BOB,
        &credential,
        "reference.create",
        json!({"channel_id": general, "path": "/pool1/data/x", "label": "x"}),
    ));
    assert_eq!(message, sentence);
}

/// `hello` signs its capabilities in order, so the new ones are appended after `presence_v1`
/// and `join_by_name_v1` stays last. Needs a machine identity (Linux).
#[cfg(target_os = "linux")]
#[test]
fn hello_appends_the_new_capabilities_in_order() {
    if !std::path::Path::new("/etc/machine-id").exists() {
        eprintln!("skipped: hello needs /etc/machine-id");
        return;
    }
    let mut ws = Workspace::new("governance-hello");
    let hello = ok(ws.broker.handle(
        host_uid(),
        &mut Connection::new(),
        request("hello", "hello", json!({"challenge_nonce": "n"})),
    ));
    let capabilities: Vec<&str> = hello["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|capability| capability.as_str().unwrap())
        .collect();
    let at = |name: &str| {
        capabilities
            .iter()
            .position(|capability| *capability == name)
            .unwrap_or_else(|| panic!("{name} missing from {capabilities:?}"))
    };
    let order = [
        "presence_v1",
        "open_governance_v1",
        "delete_v1",
        "workspace_context_v1",
        "wide_runs_v1",
    ];
    for pair in order.windows(2) {
        assert!(at(pair[0]) < at(pair[1]), "{capabilities:?}");
    }
    if cfg!(feature = "join-by-name") {
        assert_eq!(capabilities.last(), Some(&"join_by_name_v1"));
    }
}

/// Joining by invitation and device code when the workspace is open: any member invites and
/// admits, and the joiner lands in every team and unarchived channel.
#[cfg(feature = "join-by-name")]
mod join {
    use super::*;
    use biorouter_crew::device_code;
    use ed25519_dalek::SigningKey;

    /// The workspace public key, from the signing key the first journal record holds.
    fn workspace_key(ws: &Workspace) -> [u8; 32] {
        let journal = String::from_utf8(ws.journal_bytes()).unwrap();
        let first: Value = serde_json::from_str(journal.lines().next().unwrap()).unwrap();
        let secret = first["patches"][0]["value"]["workspace_signing_key"]
            .as_str()
            .unwrap();
        let secret: [u8; 32] = hex::decode(secret).unwrap().try_into().unwrap();
        SigningKey::from_bytes(&secret).verifying_key().to_bytes()
    }

    #[test]
    fn any_member_of_an_open_workspace_invites_and_admits_and_the_joiner_is_everywhere() {
        let mut ws = Workspace::new("governance-join-by-name");
        let Lab {
            mut bob,
            team,
            general,
            methods,
            ..
        } = lab(&mut ws);
        ws.directory.set(ERIN, "erin", Some("Erin Diaz"));
        let (_, message) =
            refused(ws.call(&mut bob, "enrollment.invite", json!({"username": "erin"})));
        assert_eq!(message, "forbidden: workspace host device required");

        open(&mut ws);
        ws.call_ok(&mut bob, "enrollment.invite", json!({"username": "erin"}));
        // Bob sees who is waiting, as the host does; never a code, key or UID.
        let joins = ws.snapshot(&mut bob)["pending_joins"].clone();
        assert_eq!(joins[0]["username"], "erin");
        assert!(!contains_machine_id(&joins.to_string()), "{joins}");

        let mut erin = Member::new(ERIN, key(14));
        let code = device_code(
            &ws.broker.workspace().id,
            &workspace_key(&ws),
            &erin.key.verifying_key().to_bytes(),
        );
        ws.call_ok(
            &mut bob,
            "enrollment.approve",
            json!({"username": "erin", "code": code}),
        );
        let join_id = ok(ws.broker.handle(
            ERIN,
            &mut Connection::new(),
            request("pending", "enrollment.pending", json!({})),
        ))["join_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let before = epoch(&ws);
        let claim = json!({"public_key": key_hex(&erin.key), "join_id": join_id});
        ok(signed_raw(&mut ws.broker, &mut erin, "auth.join", claim));
        assert_eq!(epoch(&ws), before);
        let snapshot = ws.snapshot(&mut erin);
        assert_eq!(ids(&snapshot, "teams"), vec![team]);
        assert_eq!(ids(&snapshot, "channels"), sorted(&[&general, &methods]));
    }
}
