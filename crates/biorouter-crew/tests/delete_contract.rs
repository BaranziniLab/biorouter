//! Broker contract for deletion (`delete_v1`, OW-4 and OW-5 of
//! `docs/crew/design/open-workspaces-and-automatic-setup.md`):
//!
//! - `channel.delete` and `team.delete` remove live state only with map removals (the channel
//!   or team, every attachment and reference posted in or derived from it, with the
//!   attachments' files, invitations and read positions), revoke the runs that name the channel,
//!   and leave the policy epoch alone. Messages stay in the journal and the state, readable by
//!   nobody, so the record never rewrites the `messages` array and an older broker replays it;
//! - a team's `#general` goes only with its team; a name changed since the person chose is
//!   refused; managed rules let the owner (or the creator), and the host for what someone who
//!   left owned, delete; an open workspace lets any member; an agent's grant never can;
//! - `workspace.delete` (the host, as a person) commits a tombstone, after which every request
//!   but `hello` is refused `workspace_deleted`, and the erase removes only what the broker
//!   created, starting and ending with the `DELETED` marker a crash would leave. The runtime
//!   directory then keeps one file, `deleted`, naming the workspace, so a member's bridge that
//!   finds no broker answers `workspace_deleted` too (L7);
//! - `biorouter-crew delete` erases a stopped broker's workspace, and finishes an interrupted
//!   erase.
//!
//! ```text
//! cargo test -p biorouter-crew --test delete_contract
//! ```
//!
//! The erase a running broker makes after the grace period needs a real broker (Linux, an
//! ordinary user and `/etc/machine-id`): `a_running_broker_erases_a_deleted_workspace_and_exits`
//! skips itself elsewhere.
#![cfg(unix)]

mod support;

use biorouter_crew::{Broker, Connection, Request, Response};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use support::*;

const BOB: u32 = 76_001;
const CAROL: u32 = 76_002;
const DAVE: u32 = 76_003;
const ERIN: u32 = 76_004;

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

fn id_of(value: &Value) -> String {
    value["id"].as_str().unwrap().to_owned()
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

/// A private run of `member` writing to `channel` and reading `sources` too; the credential.
fn grant(ws: &mut Workspace, member: &mut Member, channel: &str, sources: &[&str]) -> String {
    let params = json!({
        "channel_id": channel, "source_channels": sources,
        "provider_policy_id": "private", "personal_mode": "private",
        "public_provider": false, "expires_in": 600,
        "expected_workspace_policy_epoch": epoch(ws),
        "expected_protected_context": true,
        "workspace_institution_id": "ucsf", "connection_institution_id": "ucsf",
        "provider_affiliation": {"kind": "local"},
    });
    ws.call_ok(member, "run.create", params)["credential"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Upload `data` into `channel` as `member`; the attachment's ID.
fn upload(ws: &mut Workspace, member: &mut Member, channel: &str, data: &[u8]) -> String {
    let blob = ws.call_ok(
        member,
        "blob.begin",
        json!({
            "channel_id": channel, "size": data.len(), "sha256": digest(data),
            "name": "data.csv", "media_type": "text/csv",
        }),
    );
    let blob_id = id_of(&blob);
    ws.call_ok(
        member,
        "blob.chunk",
        json!({"blob_id": blob_id, "offset": 0, "data_hex": hex::encode(data)}),
    );
    ws.call_ok(member, "blob.finish", json!({"blob_id": blob_id}));
    blob_id
}

/// As [`upload`], by an agent's grant into its destination.
fn agent_upload(
    ws: &mut Workspace,
    uid: u32,
    credential: &str,
    channel: &str,
    data: &[u8],
) -> String {
    let blob = ok(worker(
        ws,
        uid,
        credential,
        "blob.begin",
        json!({
            "channel_id": channel, "size": data.len(), "sha256": digest(data),
            "name": "summary.txt", "media_type": "text/plain", "idempotency_key": "agent-begin",
        }),
    ));
    let blob_id = id_of(&blob);
    ok(worker(
        ws,
        uid,
        credential,
        "blob.chunk",
        json!({"blob_id": blob_id, "offset": 0, "data_hex": hex::encode(data), "idempotency_key": "agent-chunk"}),
    ));
    ok(worker(
        ws,
        uid,
        credential,
        "blob.finish",
        json!({"blob_id": blob_id, "idempotency_key": "agent-finish"}),
    ));
    blob_id
}

fn blob_file(ws: &Workspace, blob_id: &str) -> PathBuf {
    ws.root.path().join("blobs").join(blob_id)
}

fn history(ws: &mut Workspace, member: &mut Member, channel: &str) -> Response {
    ws.call(member, "messages.history", json!({"channel_id": channel}))
}

/// Bob's team `Lab` with `#data`, both shared with Carol; Dave and Erin in the workspace only.
struct Lab {
    bob: Member,
    carol: Member,
    dave: Member,
    erin: Member,
    team: String,
    general: String,
    data: String,
}

fn lab(ws: &mut Workspace) -> Lab {
    let mut bob = ws.enroll(BOB, "bob", 11);
    let carol = ws.enroll(CAROL, "carol", 12);
    let dave = ws.enroll(DAVE, "dave", 13);
    let erin = ws.enroll(ERIN, "erin", 14);
    let (team, general) = ws.create_team(&mut bob, "Lab");
    let data = id_of(&ws.call_ok(
        &mut bob,
        "channel.create",
        json!({"team_id": team, "name": "data"}),
    ));
    ws.call_ok(
        &mut bob,
        "team.add_member",
        json!({
            "team_id": team, "principal_id": carol.principal_id,
            "expected_username": "carol", "channel_ids": [data],
        }),
    );
    Lab {
        bob,
        carol,
        dave,
        erin,
        team,
        general,
        data,
    }
}

#[test]
fn deleting_a_channel_removes_what_hangs_off_it_and_keeps_its_messages_unreadable() {
    let mut ws = Workspace::new("delete-channel");
    let Lab {
        mut bob,
        mut carol,
        mut erin,
        team,
        general,
        data,
        ..
    } = lab(&mut ws);
    // Everything that hangs off #data: messages, an attachment, a reference, Carol's read
    // position, an invitation for Erin, and an agent that read #data and wrote into #general
    // (its message and its attachment are derived from #data).
    let posted = ws.call_ok(
        &mut carol,
        "message.post",
        json!({"channel_id": data, "body": "raw counts are in"}),
    );
    ws.call_ok(
        &mut carol,
        "channel.read",
        json!({"channel_id": data, "sequence": posted["sequence"]}),
    );
    let own = upload(&mut ws, &mut bob, &data, b"gene,count\nBRCA1,3\n");
    let reference = ws.call_ok(
        &mut bob,
        "reference.create",
        json!({"channel_id": data, "path": "/pool1/data/counts", "label": "counts"}),
    );
    ws.call_ok(
        &mut bob,
        "team.add_member",
        json!({"team_id": team, "principal_id": erin.principal_id, "expected_username": "erin"}),
    );
    ws.call_ok(
        &mut bob,
        "invitation.create",
        json!({"kind": "channel", "target_id": data, "principal_id": erin.principal_id}),
    );
    let credential = grant(&mut ws, &mut bob, &general, &[&general, &data]);
    ok(worker(
        &mut ws,
        BOB,
        &credential,
        "run.project",
        json!({"body": "summary of #data", "idempotency_key": "agent-message"}),
    ));
    let derived = agent_upload(&mut ws, BOB, &credential, &general, b"BRCA1 is up");
    let general_messages = |ws: &mut Workspace, member: &mut Member| {
        ok(history(ws, member, &general))["messages"]
            .as_array()
            .unwrap()
            .len()
    };
    assert_eq!(
        general_messages(&mut ws, &mut bob),
        1,
        "the agent's summary"
    );
    assert!(blob_file(&ws, &own).exists() && blob_file(&ws, &derived).exists());
    let messages = ws.broker.state_json()["messages"].as_array().unwrap().len();
    let before = epoch(&ws);

    let delete =
        json!({"channel_id": data, "expected_name": "#data", "idempotency_key": "delete-data"});
    let deleted = ws.call_ok(&mut bob, "channel.delete", delete.clone());
    assert_eq!(
        deleted,
        json!({
            "deleted": true, "channel_id": data, "team_id": team,
            "removed_attachments": 2, "removed_references": 1,
        })
    );
    assert_eq!(epoch(&ws), before, "a delete ends no unrelated agent grant");

    let state = ws.broker.state_json();
    assert!(state["channels"].get(&data).is_none());
    assert!(state["blobs"].get(&own).is_none() && state["blobs"].get(&derived).is_none());
    assert!(state["references"].get(id_of(&reference)).is_none());
    assert!(state["invitations"]
        .as_object()
        .unwrap()
        .values()
        .all(|invitation| invitation["target_id"] != data.as_str()));
    assert!(state["read_positions"]
        .as_object()
        .unwrap()
        .keys()
        .all(|key| !key.ends_with(&format!(":{data}"))));
    assert!(state["runs"]
        .as_object()
        .unwrap()
        .values()
        .all(|run| run["revoked"] == true));
    // The files go with the attachments, once the removal is journaled.
    assert!(!blob_file(&ws, &own).exists() && !blob_file(&ws, &derived).exists());
    // The messages stay, and nobody can read them: not in the deleted channel, and not the
    // agent's summary in #general that was derived from it.
    assert_eq!(state["messages"].as_array().unwrap().len(), messages);
    let (_, message) = refused(history(&mut ws, &mut carol, &data));
    assert_eq!(message, "forbidden: channel unavailable");
    assert_eq!(general_messages(&mut ws, &mut bob), 0);
    let (code, _) = refused(worker(
        &mut ws,
        BOB,
        &credential,
        "context.manifest",
        json!({}),
    ));
    assert_eq!(code, "grant_expired");
    assert!(!ws.snapshot(&mut erin)["invitations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|invitation| invitation["target_id"] == data.as_str()));

    // One record of map removals and sets: never the `messages` array, never an append.
    let record = last_record(&ws);
    assert_eq!(record["operation"], "channel.delete");
    for patch in record["patches"].as_array().unwrap() {
        assert_ne!(patch["path"][0], "messages", "{patch}");
        assert_ne!(patch["op"], "append", "{patch}");
    }

    // A retry after an unknown outcome gets the same answer, though the channel is gone.
    let lines = journal_lines(&ws);
    assert_eq!(ws.call_ok(&mut bob, "channel.delete", delete), deleted);
    assert_eq!(journal_lines(&ws), lines);

    // The journal replays.
    let mut ws = ws.reopen();
    assert!(ws.broker.state_json()["channels"].get(&data).is_none());
    let (_, message) = refused(history(&mut ws, &mut carol, &data));
    assert_eq!(message, "forbidden: channel unavailable");
    ws.call_ok(
        &mut bob,
        "message.post",
        json!({"channel_id": general, "body": "still working"}),
    );
}

#[test]
fn general_goes_only_with_its_team_and_a_renamed_channel_is_not_deleted() {
    let mut ws = Workspace::new("delete-general");
    let Lab {
        mut bob,
        general,
        data,
        ..
    } = lab(&mut ws);
    let lines = journal_lines(&ws);
    let (code, message) =
        refused(ws.call(&mut bob, "channel.delete", json!({"channel_id": general})));
    assert_eq!(
        (code.as_str(), message.as_str()),
        (
            "invalid_params",
            "invalid_params: #general belongs to its team. Delete the team instead."
        )
    );
    let (code, message) = refused(ws.call(
        &mut bob,
        "channel.delete",
        json!({"channel_id": data, "expected_name": "results"}),
    ));
    assert_eq!(code, "name_changed", "{message}");
    assert_eq!(journal_lines(&ws), lines);
    // No name: the ID alone decides.
    ws.call_ok(&mut bob, "channel.delete", json!({"channel_id": data}));
}

#[test]
fn who_may_delete_a_channel() {
    let mut ws = Workspace::new("delete-who");
    let Lab {
        mut bob,
        mut carol,
        mut erin,
        team,
        data,
        ..
    } = lab(&mut ws);
    let lines = journal_lines(&ws);
    // Managed: a member who doesn't own it, someone outside it, and the host while its owner
    // is still a member are all refused.
    let delete = json!({"channel_id": data});
    let (_, message) = refused(ws.call(&mut carol, "channel.delete", delete.clone()));
    assert_eq!(message, "forbidden: current owner required");
    let (_, message) = refused(ws.call(&mut erin, "channel.delete", delete.clone()));
    assert_eq!(message, "forbidden: channel unavailable");
    let (_, message) = refused(ws.host_call("channel.delete", delete.clone()));
    assert_eq!(message, "forbidden: channel unavailable");
    // Never an agent, whoever's it is.
    let credential = grant(&mut ws, &mut bob, &data, &[]);
    let mut params = delete.clone();
    params["idempotency_key"] = json!("agent-delete");
    for method in ["channel.delete", "team.delete"] {
        let mut params = params.clone();
        params["team_id"] = json!(team);
        let (_, message) = refused(worker(&mut ws, BOB, &credential, method, params));
        assert_eq!(
            message, "forbidden: worker operation unavailable",
            "{method}"
        );
    }
    assert_eq!(journal_lines(&ws), lines + 1, "only the grant was written");

    // Open: any member of the channel.
    ws.host_ok("workspace.set_governance", json!({"governance": "open"}));
    ws.call_ok(&mut carol, "channel.delete", delete);

    // The host stewards a channel whose owner left the workspace, from outside it.
    let notes = id_of(&ws.call_ok(
        &mut bob,
        "channel.create",
        json!({"team_id": team, "name": "notes"}),
    ));
    ws.host_ok("workspace.set_governance", json!({"governance": "managed"}));
    ws.offboard(&bob.principal_id);
    let host_is_in = ws.broker.state_json()["channels"][&notes]["members"]
        .as_array()
        .unwrap()
        .iter()
        .any(|member| member == ws.host.principal_id.as_str());
    assert!(!host_is_in);
    ws.host_ok(
        "channel.delete",
        json!({"channel_id": notes, "expected_name": "notes"}),
    );
}

#[test]
fn deleting_a_team_removes_every_channel_in_it() {
    let mut ws = Workspace::new("delete-team");
    let Lab {
        mut bob,
        mut carol,
        mut dave,
        mut erin,
        team,
        general,
        data,
    } = lab(&mut ws);
    // A channel Bob is not in, with an attachment, and a pending invitation to the team.
    let private = id_of(&ws.call_ok(
        &mut carol,
        "channel.create",
        json!({"team_id": team, "name": "private"}),
    ));
    let file = upload(&mut ws, &mut carol, &private, b"carol's notes");
    ws.call_ok(
        &mut bob,
        "invitation.create",
        json!({"kind": "team", "target_id": team, "principal_id": dave.principal_id}),
    );
    let lines = journal_lines(&ws);
    let (_, message) = refused(ws.call(&mut carol, "team.delete", json!({"team_id": team})));
    assert_eq!(message, "forbidden: team creator required");
    let (_, message) = refused(ws.call(&mut erin, "team.delete", json!({"team_id": team})));
    assert_eq!(message, "forbidden: team unavailable");
    let (_, message) = refused(ws.host_call("team.delete", json!({"team_id": team})));
    assert_eq!(message, "forbidden: team unavailable");
    let (code, _) = refused(ws.call(
        &mut bob,
        "team.delete",
        json!({"team_id": team, "expected_name": "Bench"}),
    ));
    assert_eq!(code, "name_changed");
    assert_eq!(journal_lines(&ws), lines);

    let before = epoch(&ws);
    assert_eq!(
        ws.call_ok(
            &mut bob,
            "team.delete",
            json!({"team_id": team, "expected_name": "Lab"})
        ),
        json!({"deleted": true, "team_id": team, "channels": 3})
    );
    assert_eq!(epoch(&ws), before);
    let state = ws.broker.state_json();
    assert!(state["teams"].get(&team).is_none());
    for channel in [&general, &data, &private] {
        assert!(state["channels"].get(channel).is_none());
    }
    assert!(state["blobs"].get(&file).is_none());
    assert!(!blob_file(&ws, &file).exists());
    assert_eq!(state["invitations"], json!({}));
    assert!(ws.snapshot(&mut dave)["invitations"]
        .as_array()
        .unwrap()
        .is_empty());
    for patch in last_record(&ws)["patches"].as_array().unwrap() {
        assert_ne!(patch["path"][0], "messages", "{patch}");
    }

    // The host deletes a team whose creator left the workspace, from outside it.
    let (solo, _) = ws.create_team(&mut erin, "Solo");
    ws.offboard(&erin.principal_id);
    ws.host_ok("team.delete", json!({"team_id": solo}));
    // An open workspace lets any member of the team delete it.
    ws.host_ok("workspace.set_governance", json!({"governance": "open"}));
    let (shared, _) = ws.create_team(&mut carol, "Shared");
    ws.call_ok(&mut dave, "team.delete", json!({"team_id": shared}));
    let mut ws = ws.reopen();
    assert!(ws.host_snapshot()["teams"].as_array().unwrap().is_empty());
}

/// A raw, unsigned request as `uid` on a fresh connection.
fn raw(ws: &mut Workspace, uid: u32, method: &str, params: Value) -> Response {
    ws.broker
        .handle(uid, &mut Connection::new(), request("raw", method, params))
}

fn workspace_delete(ws: &Workspace) -> Value {
    json!({"workspace_id": ws.broker.workspace().id, "expected_name": null})
}

#[test]
fn deleting_the_workspace_leaves_a_tombstone_that_refuses_everything_but_hello() {
    let mut ws = Workspace::new("delete-workspace");
    let Lab {
        mut bob, general, ..
    } = lab(&mut ws);
    let credential = grant(&mut ws, &mut bob, &general, &[]);
    let lines = journal_lines(&ws);
    // Only the host, as a person, naming this workspace by its ID and current name.
    let (_, message) = refused(ws.call(&mut bob, "workspace.delete", workspace_delete(&ws)));
    assert_eq!(message, "forbidden: workspace host device required");
    let mut params = workspace_delete(&ws);
    params["idempotency_key"] = json!("agent-delete");
    let (_, message) = refused(worker(
        &mut ws,
        BOB,
        &credential,
        "workspace.delete",
        params,
    ));
    assert_eq!(message, "forbidden: worker operation unavailable");
    let (code, _) = refused(ws.host_call(
        "workspace.delete",
        json!({"workspace_id": uuid(), "expected_name": null}),
    ));
    assert_eq!(code, "invalid_params");
    let (code, _) = refused(ws.host_call(
        "workspace.delete",
        json!({"workspace_id": ws.broker.workspace().id, "expected_name": "lab"}),
    ));
    assert_eq!(code, "name_changed");
    assert_eq!(journal_lines(&ws), lines);

    let before = epoch(&ws);
    let params = workspace_delete(&ws);
    assert_eq!(
        ws.host_ok("workspace.delete", params),
        json!({"deleted": true, "workspace_id": ws.broker.workspace().id, "erase_after_seconds": 30})
    );
    let state = ws.broker.state_json();
    assert!(state["workspace"]["deleted_at"].as_u64().is_some());
    assert_eq!(epoch(&ws), before + 1);
    for map in ["devices", "enrollments", "grants"] {
        assert_eq!(state[map], json!({}), "{map}");
    }
    assert!(state.get("pending_joins").is_none());
    assert!(state["runs"]
        .as_object()
        .unwrap()
        .values()
        .all(|run| run["revoked"] == true));

    let deleted =
        "workspace_deleted: The host deleted this workspace. Remove it from this computer.";
    let attempts = [
        (
            host_uid(),
            "auth.challenge",
            json!({"device_id": device_id(&ws.host.key)}),
        ),
        (
            host_uid(),
            "auth.bootstrap",
            json!({"public_key": key_hex(&ws.host.key)}),
        ),
        (
            BOB,
            "auth.challenge",
            json!({"device_id": device_id(&bob.key)}),
        ),
        (
            BOB,
            "auth.enroll",
            json!({"public_key": key_hex(&bob.key), "invitation": "x"}),
        ),
        (BOB, "enrollment.pending", json!({})),
        (
            BOB,
            "auth.join",
            json!({"public_key": key_hex(&bob.key), "join_id": "x"}),
        ),
        (BOB, "workspace.snapshot", json!({})),
    ];
    for (uid, method, params) in attempts {
        let (code, message) = refused(raw(&mut ws, uid, method, params));
        assert_eq!(
            (code.as_str(), message.as_str()),
            ("workspace_deleted", deleted),
            "{method}"
        );
    }
    let (_, message) = refused(worker(
        &mut ws,
        BOB,
        &credential,
        "context.manifest",
        json!({}),
    ));
    assert_eq!(message, deleted);
    #[cfg(target_os = "linux")]
    if std::path::Path::new("/etc/machine-id").exists() {
        assert_eq!(
            ok(raw(&mut ws, BOB, "hello", json!({})))["state"],
            "deleted"
        );
    }

    // The tombstone survives a restart.
    let mut ws = ws.reopen();
    let (_, message) = refused(raw(&mut ws, BOB, "workspace.snapshot", json!({})));
    assert_eq!(message, deleted);
}

#[test]
fn the_erase_removes_only_what_the_broker_created_and_ends_with_its_marker() {
    let mut ws = Workspace::new("delete-erase");
    let Lab { mut bob, data, .. } = lab(&mut ws);
    let file = upload(&mut ws, &mut bob, &data, b"payload");
    let root = ws.root.path().to_path_buf();
    // Files the host keeps in the same directory and in its `blobs/`, which the broker never
    // created: only blob files named by a blob ID are the broker's.
    std::fs::write(root.join("notes.txt"), b"mine").unwrap();
    std::fs::write(root.join("blobs").join("mine.txt"), b"mine").unwrap();
    let params = workspace_delete(&ws);
    ws.host_ok("workspace.delete", params);
    let deleted_at = ws.broker.state_json()["workspace"]["deleted_at"]
        .as_u64()
        .unwrap();
    assert!(
        ws.broker.erase_if_due(deleted_at + 29).is_none(),
        "still in its grace period"
    );
    assert!(root.join("journal.jsonl").exists());

    let leftovers = ws.broker.erase_if_due(deleted_at + 30).unwrap().unwrap();
    assert_eq!(
        leftovers,
        vec!["blobs/mine.txt".to_owned(), "notes.txt".to_owned()]
    );
    let mut left: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left, vec!["blobs".to_owned(), "notes.txt".to_owned()]);
    assert!(!root.join("blobs").join(&file).exists());
    assert_eq!(
        std::fs::read(root.join("blobs").join("mine.txt")).unwrap(),
        b"mine"
    );

    // Without anything of the host's in it, the directory itself goes.
    let mut ws = Workspace::new("delete-erase-all");
    let root = ws.root.path().to_path_buf();
    let params = workspace_delete(&ws);
    ws.host_ok("workspace.delete", params);
    assert_eq!(
        ws.broker.erase_if_due(u64::MAX).unwrap().unwrap(),
        Vec::<String>::new()
    );
    assert!(!root.exists());
}

/// Close the broker, keeping the state directory; the workspace ID and the directory.
fn stopped(ws: Workspace) -> (String, TempRoot, FakeDirectory) {
    let Workspace {
        broker,
        root,
        directory,
        ..
    } = ws;
    let id = broker.workspace().id.clone();
    drop(broker);
    (id, root, directory)
}

#[test]
fn a_directory_an_erase_left_half_done_never_reopens_and_delete_finishes_it() {
    let ws = Workspace::new("delete-marker");
    let (id, root, directory) = stopped(ws);
    // What a crash between the marker and the end of the erase leaves.
    let marker = root.path().join("DELETED");
    std::fs::write(&marker, &id).unwrap();
    std::fs::set_permissions(&marker, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    let error = Broker::open_with_directory(root.path(), &key_hex(&key(7)), directory.boxed())
        .err()
        .expect("a half-erased directory never reopens");
    let message = format!("{error:#}");
    assert!(message.starts_with("workspace_deleted:"), "{message}");
    assert!(
        message.contains(&format!("--workspace-id {id}")),
        "{message}"
    );

    let answer = biorouter_crew::delete_workspace(root.path(), &id).unwrap();
    assert_eq!(
        answer,
        json!({"deleted": true, "workspace_id": id, "leftovers": []})
    );
    assert!(!root.path().exists());
}

/// Write `contents` as the `DELETED` marker, owner-only, as an interrupted erase left it.
fn write_marker(root: &Path, contents: &[u8]) {
    let marker = root.join("DELETED");
    std::fs::write(&marker, contents).unwrap();
    std::fs::set_permissions(&marker, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
}

/// A marker a full disk or a crash cut short names no workspace. It still keeps the directory
/// from reopening, the refusal names the journal's ID, and `delete` finishes with that ID; with
/// the journal gone too, the ID the host gives finishes it.
#[test]
fn a_marker_that_names_no_workspace_is_finished_with_the_journals_id() {
    for contents in [&b""[..], b"\n", b"not-a-workspace-id"] {
        let ws = Workspace::new("delete-empty-marker");
        let (id, root, directory) = stopped(ws);
        write_marker(root.path(), contents);
        let error = Broker::open_with_directory(root.path(), &key_hex(&key(7)), directory.boxed())
            .err()
            .expect("a half-erased directory never reopens");
        let message = format!("{error:#}");
        assert!(message.starts_with("workspace_deleted:"), "{message}");
        assert!(
            message.ends_with(&format!("--workspace-id {id}")),
            "{message}"
        );
        let other = uuid();
        let error = biorouter_crew::delete_workspace(root.path(), &other).unwrap_err();
        assert!(
            format!("{error:#}").starts_with("identity_mismatch:"),
            "{error:#}"
        );
        let answer = biorouter_crew::delete_workspace(root.path(), &id).unwrap();
        assert_eq!(
            answer,
            json!({"deleted": true, "workspace_id": id, "leftovers": []})
        );
        assert!(!root.path().exists());
    }

    let ws = Workspace::new("delete-empty-marker-no-journal");
    let (id, root, _) = stopped(ws);
    std::fs::remove_file(root.path().join("journal.jsonl")).unwrap();
    write_marker(root.path(), b"");
    let answer = biorouter_crew::delete_workspace(root.path(), &id).unwrap();
    assert_eq!(answer["deleted"], true);
    assert!(!root.path().exists());
}

/// The marker is written beside the directory's files and renamed into place, so a crash
/// before the rename leaves the workspace whole and openable, and the erase removes the
/// temporary file with everything else.
#[test]
fn the_marker_is_written_whole_before_it_appears() {
    let ws = Workspace::new("delete-marker-temp");
    let (id, root, directory) = stopped(ws);
    let temporary = root.path().join(".DELETED.tmp");
    std::fs::write(&temporary, b"").unwrap();
    std::fs::set_permissions(
        &temporary,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .unwrap();
    let broker = Broker::open_with_directory(root.path(), &key_hex(&key(7)), directory.boxed())
        .expect("nothing was erased, so the workspace opens");
    drop(broker);
    let answer = biorouter_crew::delete_workspace(root.path(), &id).unwrap();
    assert_eq!(
        answer,
        json!({"deleted": true, "workspace_id": id, "leftovers": []})
    );
    assert!(!root.path().exists());
}

/// `serve` finishes an erase its marker shows was started, and `start` refuses to start one,
/// naming the command that finishes it. `serve` needs Linux.
#[cfg(target_os = "linux")]
#[test]
fn serve_finishes_an_interrupted_erase_and_start_refuses_it() {
    let crew = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_biorouter-crew"))
            .args(args)
            .output()
            .unwrap()
    };
    let ws = Workspace::new("delete-serve-marker");
    let (id, root, _) = stopped(ws);
    write_marker(root.path(), b"");
    let state_dir = root.path().to_str().unwrap().to_owned();
    let started = crew(&[
        "start",
        "--state-dir",
        &state_dir,
        "--bootstrap-key",
        &key_hex(&key(7)),
    ]);
    assert!(!started.status.success());
    let message = String::from_utf8_lossy(&started.stderr).into_owned();
    assert!(message.contains("workspace_deleted:"), "{message}");
    assert!(
        message.contains(&format!("--workspace-id {id}")),
        "{message}"
    );
    assert!(root.path().join("journal.jsonl").exists());

    let served = crew(&[
        "serve",
        "--state-dir",
        &state_dir,
        "--bootstrap-key",
        &key_hex(&key(7)),
    ]);
    assert!(
        served.status.success(),
        "{}",
        String::from_utf8_lossy(&served.stderr)
    );
    let answer: Value = serde_json::from_slice(&served.stdout).unwrap();
    assert_eq!(
        answer,
        json!({"deleted": true, "workspace_id": id, "leftovers": []})
    );
    assert!(!root.path().exists());
}

#[test]
fn delete_erases_a_stopped_workspace_and_refuses_a_running_or_another_one() {
    let mut ws = Workspace::new("delete-offline");
    let Lab { mut bob, data, .. } = lab(&mut ws);
    let file = upload(&mut ws, &mut bob, &data, b"payload");
    let root = ws.root.path().to_path_buf();
    let id = ws.broker.workspace().id.clone();

    // A broker holds the directory.
    let error = biorouter_crew::delete_workspace(&root, &id).unwrap_err();
    assert!(
        format!("{error:#}").starts_with("writer_active:"),
        "{error:#}"
    );
    let (id, temp, _) = stopped(ws);
    // The ID is the confirmation.
    let other = uuid();
    let error = biorouter_crew::delete_workspace(&root, &other).unwrap_err();
    assert!(
        format!("{error:#}").starts_with("identity_mismatch:"),
        "{error:#}"
    );
    assert!(root.join("journal.jsonl").exists());

    std::fs::write(root.join("notes.txt"), b"mine").unwrap();
    // Through the binary, as the host's daemon runs it over SSH.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_biorouter-crew"))
        .args([
            "delete",
            "--state-dir",
            root.to_str().unwrap(),
            "--workspace-id",
            &id,
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        answer,
        json!({"deleted": true, "workspace_id": id, "leftovers": ["notes.txt"]})
    );
    assert!(!root.join("journal.jsonl").exists());
    assert!(!root.join("blobs").join(&file).exists());
    assert!(root.join("notes.txt").exists());
    drop(temp);

    let missing = Path::new("/tmp").join(format!("crew-missing-{}", uuid()));
    let error = biorouter_crew::delete_workspace(&missing, &id).unwrap_err();
    assert!(format!("{error:#}").starts_with("not_found:"), "{error:#}");
}

/// A real broker counts down and then erases itself: needs Linux, an ordinary user (the broker
/// refuses root) and `/etc/machine-id`, and takes the 30-second grace period.
#[cfg(target_os = "linux")]
#[test]
fn a_running_broker_erases_a_deleted_workspace_and_exits() {
    use biorouter_crew::{signing_payload, DeviceAuth};
    use ed25519_dalek::Signer;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::time::{Duration, Instant};

    if host_uid() == 0 || !Path::new("/etc/machine-id").exists() {
        eprintln!("skipped: a real broker needs an ordinary user and /etc/machine-id");
        return;
    }
    let crew = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_biorouter-crew"))
            .args(args)
            .output()
            .unwrap()
    };
    let parent = TempRoot::new("delete-running");
    let state = parent.path().join("lab");
    let state_dir = state.to_str().unwrap();
    let host = key(7);
    let output = crew(&[
        "start",
        "--state-dir",
        state_dir,
        "--bootstrap-key",
        &key_hex(&host),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let started: Value = serde_json::from_slice(&output.stdout).unwrap();
    let workspace_id = started["workspace_id"].as_str().unwrap().to_owned();
    let runtime: Value =
        serde_json::from_slice(&std::fs::read(state.join("runtime.json")).unwrap()).unwrap();
    let socket = PathBuf::from(runtime["socket"].as_str().unwrap());

    let stream = UnixStream::connect(&socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut writer = stream;
    let mut send = |request: &Request| -> Response {
        let mut frame = serde_json::to_vec(request).unwrap();
        frame.push(b'\n');
        writer.write_all(&frame).unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let mut signed_send = |method: &str, params: Value| -> Response {
        let challenge = send(&request(
            "challenge",
            "auth.challenge",
            json!({"device_id": device_id(&host)}),
        ));
        let nonce = ok(challenge)["nonce"].as_str().unwrap().to_owned();
        let signature = host.sign(&signing_payload(
            &workspace_id,
            host_uid(),
            &nonce,
            method,
            &params,
        ));
        let mut frame = request(method, method, params);
        frame.auth = Some(DeviceAuth {
            device_id: device_id(&host),
            nonce,
            signature: hex::encode(signature.to_bytes()),
        });
        send(&frame)
    };
    ok(signed_send(
        "auth.bootstrap",
        json!({"public_key": key_hex(&host)}),
    ));
    // L7: a member's bridge, connected and idle: its daemon sends nothing through the grace
    // period, so it hears of the deletion only when the broker stops.
    let mut idle_bridge = std::process::Command::new(env!("CARGO_BIN_EXE_biorouter-crew"))
        .args(["bridge", "--stdio", "--socket"])
        .arg(&socket)
        .args(["--owner-uid", &host_uid().to_string()])
        .args(["--workspace-id", &workspace_id])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    ok(signed_send(
        "workspace.delete",
        json!({"workspace_id": workspace_id, "expected_name": null, "idempotency_key": "delete"}),
    ));
    // Counting down: `hello` says so, and everything else is refused.
    assert_eq!(
        ok(send(&request("hello", "hello", json!({}))))["state"],
        "deleted"
    );
    let (code, _) = refused(send(&request(
        "challenge",
        "auth.challenge",
        json!({"device_id": device_id(&host)}),
    )));
    assert_eq!(code, "workspace_deleted");

    let deadline = Instant::now() + Duration::from_secs(60);
    while (state.exists() || socket.exists()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(500));
    }
    assert!(!state.exists(), "the state directory is erased");
    assert!(!socket.exists(), "the socket is removed");
    // The idle bridge exits when the broker does, saying the host deleted the workspace. Its
    // input stays open until then: closed, it would end as a daemon that hung up.
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = idle_bridge.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "the idle bridge is still running"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(status.code(), Some(1));
    let mut said = String::new();
    std::io::Read::read_to_string(&mut idle_bridge.stderr.take().unwrap(), &mut said).unwrap();
    assert!(
        said.contains(
            "workspace_deleted: The host deleted this workspace. Remove it from this computer."
        ),
        "{said}"
    );
    the_tombstone_tells_members(&socket, &workspace_id);

    // The tombstone blocks nothing: a new workspace started in the same state directory gets a
    // runtime directory of its own, and a bridge for the old one still says it was deleted.
    let output = crew(&[
        "start",
        "--state-dir",
        state_dir,
        "--bootstrap-key",
        &key_hex(&host),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let restarted: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_ne!(restarted["workspace_id"], json!(workspace_id));
    let runtime: Value =
        serde_json::from_slice(&std::fs::read(state.join("runtime.json")).unwrap()).unwrap();
    let new_socket = PathBuf::from(runtime["socket"].as_str().unwrap());
    assert_ne!(new_socket.parent(), socket.parent());
    the_tombstone_tells_members(&socket, &workspace_id);
    let stopped = crew(&["stop", "--state-dir", state_dir]);
    assert!(
        stopped.status.success(),
        "{}",
        String::from_utf8_lossy(&stopped.stderr)
    );
    for socket in [&socket, &new_socket] {
        let directory = socket.parent().unwrap();
        for entry in std::fs::read_dir(directory).into_iter().flatten().flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
        let _ = std::fs::remove_dir(directory);
    }
}

/// L7: once a deleted workspace's broker has erased it and stopped, its runtime directory holds
/// only the tombstone naming the workspace, readable by every account, and a member's bridge
/// that finds no broker there fails with `workspace_deleted` (exit 1, as the member's daemon
/// reads it) for that workspace alone.
#[cfg(target_os = "linux")]
fn the_tombstone_tells_members(socket: &Path, workspace_id: &str) {
    use std::os::unix::fs::PermissionsExt;
    let directory = socket.parent().unwrap();
    let entries: Vec<String> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, ["deleted"], "only the tombstone stays");
    let tombstone = directory.join("deleted");
    assert_eq!(std::fs::read_to_string(&tombstone).unwrap(), workspace_id);
    assert_eq!(
        std::fs::metadata(&tombstone).unwrap().permissions().mode() & 0o7777,
        0o644
    );
    let bridge = |workspace: &str| {
        std::process::Command::new(env!("CARGO_BIN_EXE_biorouter-crew"))
            .args(["bridge", "--stdio", "--socket"])
            .arg(socket)
            .args(["--owner-uid", &host_uid().to_string()])
            .args(["--workspace-id", workspace])
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap()
    };
    let deleted = bridge(workspace_id);
    assert_eq!(deleted.status.code(), Some(1));
    let said = String::from_utf8_lossy(&deleted.stderr);
    assert!(
        said.contains(
            "workspace_deleted: The host deleted this workspace. Remove it from this computer."
        ),
        "{said}"
    );
    let other = bridge(&uuid());
    assert_eq!(other.status.code(), Some(1));
    let said = String::from_utf8_lossy(&other.stderr);
    assert!(!said.contains("workspace_deleted"), "{said}");
    assert!(said.contains("(os error 2)"), "{said}");
}
