//! Full remote access in the broker (design OW-6): `run.create` takes `remote_access: "full"`
//! only with a work folder, the person's execution grant and a private provider, echoes it in
//! the run, and `run.remote_scope` reports the level and the run's owner to the bridge.
#![cfg(unix)]

mod support;

use biorouter_crew::{Broker, Request};
use serde_json::{json, Value};
use support::{ok, refused, TempRoot, Workspace};

/// `run.create` params for a private run in `channel`, with the policy fields the broker checks.
fn run_params(ws: &Workspace, channel: &str, extra: Value) -> Value {
    let mut params = json!({
        "channel_id": channel,
        "source_channels": [channel],
        "provider_policy_id": "private-test",
        "personal_mode": "private",
        "public_provider": false,
        "expires_in": 600,
        "expected_workspace_policy_epoch": ws.broker.workspace().policy_epoch,
        "workspace_institution_id": ws.broker.workspace().institution_id,
        "connection_institution_id": ws.broker.workspace().institution_id,
        "provider_affiliation": {"kind": "local"},
        "expected_protected_context": true,
    });
    for (key, value) in extra.as_object().unwrap() {
        params[key] = value.clone();
    }
    params
}

fn worker(ws: &mut Workspace, credential: &str, method: &str) -> biorouter_crew::Response {
    let request = Request {
        version: 1,
        id: format!("worker-{method}"),
        method: method.into(),
        params: json!({}),
        auth: None,
        credential: Some(credential.into()),
    };
    ws.broker
        .handle(ws.host.uid, &mut ws.host.connection, request)
}

#[test]
fn a_full_access_run_is_echoed_and_its_scope_names_the_level_and_owner() {
    let mut ws = Workspace::new("full-access");
    let folder = TempRoot::new("full-access-work");
    let (_, channel) = ws.host_team("lab");
    let params = run_params(
        &ws,
        &channel,
        json!({"remote_root": folder.path(), "remote_execution": true, "remote_access": "full"}),
    );
    let created = ws.host_ok("run.create", params);
    assert_eq!(created["run"]["remote_access"], "full");
    let credential = created["credential"].as_str().unwrap().to_owned();
    let scope = ok(worker(&mut ws, &credential, "run.remote_scope"));
    assert_eq!(scope["access"], "full");
    assert_eq!(scope["owner_id"], ws.host.principal_id.as_str());
    assert_eq!(scope["channel_id"], channel.as_str());
    assert_eq!(scope["root"], folder.path().to_str().unwrap());
    assert_eq!(scope["allow_execute"], true);
}

#[test]
fn a_run_without_remote_access_stays_a_folder_run() {
    let mut ws = Workspace::new("folder-access");
    let folder = TempRoot::new("folder-access-work");
    let (_, channel) = ws.host_team("lab");
    for extra in [
        json!({"remote_root": folder.path(), "remote_execution": true}),
        json!({"remote_root": folder.path(), "remote_access": "folder"}),
        json!({"remote_root": folder.path(), "remote_access": null}),
    ] {
        let created = ws.host_ok("run.create", run_params(&ws, &channel, extra.clone()));
        // A folder run serializes as it always did: no `remote_access` field.
        assert!(
            created["run"].get("remote_access").is_none(),
            "{extra}: {created}"
        );
        let credential = created["credential"].as_str().unwrap().to_owned();
        let scope = ok(worker(&mut ws, &credential, "run.remote_scope"));
        assert_eq!(scope["access"], "folder", "{extra}");
    }
}

/// Full access needs everything a confined command needs, and an unknown level is refused.
#[test]
fn full_access_needs_a_folder_execution_and_a_private_provider() {
    let mut ws = Workspace::new("full-access-refusals");
    let folder = TempRoot::new("full-access-refusals-work");
    let (_, channel) = ws.host_team("lab");
    for (extra, code) in [
        (json!({"remote_access": "full"}), "invalid_params"),
        (
            json!({"remote_root": folder.path(), "remote_access": "full"}),
            "invalid_params",
        ),
        (
            json!({"remote_root": folder.path(), "remote_execution": false, "remote_access": "full"}),
            "invalid_params",
        ),
        (
            json!({"remote_root": folder.path(), "remote_execution": true, "remote_access": "root"}),
            "invalid_params",
        ),
        (
            json!({"remote_root": folder.path(), "remote_execution": true, "remote_access": true}),
            "invalid_params",
        ),
    ] {
        let params = run_params(&ws, &channel, extra.clone());
        let (got, message) = refused(ws.host_call("run.create", params));
        assert_eq!(got, code, "{extra}: {message}");
    }
    // A public provider never gets a remote root at all, so never full access either.
    let mut public = run_params(
        &ws,
        &channel,
        json!({"remote_root": folder.path(), "remote_execution": true, "remote_access": "full"}),
    );
    public["public_provider"] = json!(true);
    let (code, _) = refused(ws.host_call("run.create", public));
    assert_eq!(code, "privacy_denied");
}

/// A worker credential cannot mint a run, full or not: `run.create` stays a person's request.
#[test]
fn a_worker_cannot_create_a_full_access_run() {
    let mut ws = Workspace::new("full-access-worker");
    let folder = TempRoot::new("full-access-worker-work");
    let (_, channel) = ws.host_team("lab");
    let created = ws.host_ok(
        "run.create",
        run_params(
            &ws,
            &channel,
            json!({"remote_root": folder.path(), "remote_execution": true, "remote_access": "full"}),
        ),
    );
    let credential = created["credential"].as_str().unwrap().to_owned();
    let request = Request {
        version: 1,
        id: "worker-mint".into(),
        method: "run.create".into(),
        params: run_params(
            &ws,
            &channel,
            json!({"remote_root": folder.path(), "remote_execution": true, "remote_access": "full", "idempotency_key": "mint"}),
        ),
        auth: None,
        credential: Some(credential),
    };
    let response = ws
        .broker
        .handle(ws.host.uid, &mut ws.host.connection, request);
    assert_eq!(response.error.unwrap().code, "forbidden");
}

/// The level replays from the journal like every other run field.
#[test]
fn a_full_access_run_survives_a_restart() {
    let mut ws = Workspace::new("full-access-replay");
    let folder = TempRoot::new("full-access-replay-work");
    let (_, channel) = ws.host_team("lab");
    let created = ws.host_ok(
        "run.create",
        run_params(
            &ws,
            &channel,
            json!({"remote_root": folder.path(), "remote_execution": true, "remote_access": "full"}),
        ),
    );
    let credential = created["credential"].as_str().unwrap().to_owned();
    let mut ws = ws.reopen();
    let scope = ok(worker(&mut ws, &credential, "run.remote_scope"));
    assert_eq!(scope["access"], "full");
}

/// `remote_access_v1` is advertised after `presence_v1`, where the design appends capabilities.
#[test]
fn the_broker_advertises_remote_access() {
    let capabilities = Broker::capabilities();
    let presence = capabilities
        .iter()
        .position(|c| *c == "presence_v1")
        .unwrap();
    assert!(capabilities[presence + 1..].contains(&"remote_access_v1"));
}
