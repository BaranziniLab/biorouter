#![cfg(unix)]

mod support;

use biorouter_crew::{signing_payload, Broker, Connection, DeviceAuth, Request};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn uid() -> u32 {
    unsafe { libc::geteuid() }
}
/// An ordinary person's UID for the guest these tests enroll by token. It used to be UID 0 (or
/// 1 under root), which the token path now refuses like every other enrollment path (Q2-14,
/// `legacy_invite_contract.rs`), so the guest lives in a fake account directory instead.
fn guest_uid() -> u32 {
    74_999
}
/// The broker at `root`, reading accounts from a fake directory that holds this process's own
/// UID (the host) and the guest, both with a login shell. `UID_MIN` is lowered to the host's
/// UID where a test machine's own account sits below 1000 (macOS starts people at 501), so the
/// host may still add devices of its own.
fn open_broker(root: &Path, bootstrap_key: &str) -> anyhow::Result<Broker> {
    let directory = support::FakeDirectory::default();
    directory.set_with_shell(uid(), "host", "/bin/bash");
    directory.set_with_shell(guest_uid(), "guest", "/bin/bash");
    directory.set_uid_min(uid().clamp(1, 1000));
    Broker::open_with_directory(root, bootstrap_key, directory.boxed())
}
fn key_hex(key: &SigningKey) -> String {
    hex::encode(key.verifying_key().to_bytes())
}
fn digest(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}
fn temp_root(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "biorouter-crew-{label}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}
fn request(id: &str, method: &str, params: Value) -> Request {
    Request {
        version: 1,
        id: id.into(),
        method: method.into(),
        params,
        auth: None,
        credential: None,
    }
}
fn run_contract_params(broker: &Broker, mut params: Value) -> Value {
    if let Some(fields) = params.as_object_mut() {
        fields
            .entry("expected_workspace_policy_epoch")
            .or_insert_with(|| json!(broker.workspace().policy_epoch));
        fields
            .entry("workspace_institution_id")
            .or_insert_with(|| json!(broker.workspace().institution_id));
        let private = fields.get("personal_mode").and_then(Value::as_str) == Some("private");
        fields
            .entry("connection_institution_id")
            .or_insert_with(|| {
                if private {
                    json!(broker.workspace().institution_id)
                } else {
                    Value::Null
                }
            });
        fields.entry("provider_affiliation").or_insert_with(|| {
            if private {
                json!({"kind":"local"})
            } else {
                json!({"kind":"unstated"})
            }
        });
        fields
            .entry("expected_protected_context")
            .or_insert_with(|| json!(private));
    }
    params
}

fn signed_as(
    broker: &mut Broker,
    connection: &mut Connection,
    caller_uid: u32,
    key: &SigningKey,
    id: &str,
    method: &str,
    params: Value,
) -> biorouter_crew::Response {
    let params = if method == "run.create" {
        run_contract_params(broker, params)
    } else {
        params
    };
    signed_as_raw(broker, connection, caller_uid, key, id, method, params)
}

fn signed_as_raw(
    broker: &mut Broker,
    connection: &mut Connection,
    caller_uid: u32,
    key: &SigningKey,
    id: &str,
    method: &str,
    params: Value,
) -> biorouter_crew::Response {
    let device_id = digest(&key.verifying_key().to_bytes());
    let challenge = broker.handle(
        caller_uid,
        connection,
        request(
            "challenge",
            "auth.challenge",
            json!({"device_id": device_id}),
        ),
    );
    let nonce = challenge.result.unwrap()["nonce"]
        .as_str()
        .unwrap()
        .to_owned();
    let signature = key.sign(&signing_payload(
        &broker.workspace().id,
        caller_uid,
        &nonce,
        method,
        &params,
    ));
    let mut req = request(id, method, params);
    req.auth = Some(DeviceAuth {
        device_id,
        nonce,
        signature: hex::encode(signature.to_bytes()),
    });
    broker.handle(caller_uid, connection, req)
}
fn bootstrap_unlabelled(root: &Path) -> (Broker, Connection, SigningKey) {
    let key = SigningKey::from_bytes(&[7; 32]);
    let public = key_hex(&key);
    let mut broker = open_broker(root, &public).unwrap();
    let mut connection = Connection::new();
    let device_id = digest(&key.verifying_key().to_bytes());
    let challenge = broker.handle(
        uid(),
        &mut connection,
        request(
            "challenge",
            "auth.challenge",
            json!({"device_id": device_id}),
        ),
    );
    let nonce = challenge.result.unwrap()["nonce"]
        .as_str()
        .unwrap()
        .to_owned();
    let params = json!({"public_key": public});
    let signature = key.sign(&signing_payload(
        &broker.workspace().id,
        uid(),
        &nonce,
        "auth.bootstrap",
        &params,
    ));
    let auth = DeviceAuth {
        device_id,
        nonce,
        signature: hex::encode(signature.to_bytes()),
    };
    let mut req = request("bootstrap", "auth.bootstrap", params);
    req.auth = Some(auth);
    assert!(broker.handle(uid(), &mut connection, req).error.is_none());
    (broker, connection, key)
}

fn bootstrap(root: &Path) -> (Broker, Connection, SigningKey) {
    let (mut broker, mut connection, key) = bootstrap_unlabelled(root);
    let labelled = signed(
        &mut broker,
        &mut connection,
        &key,
        "label-workspace",
        "policy.set",
        json!({
            "mode": "private",
            "institution_id": "ucsf",
            "idempotency_key": "label-workspace"
        }),
    );
    assert!(
        labelled.error.is_none(),
        "workspace institution fixture failed: {:?}",
        labelled.error
    );
    (broker, connection, key)
}
fn enroll_user(
    broker: &mut Broker,
    host_connection: &mut Connection,
    host_key: &SigningKey,
    guest_uid: u32,
    guest_key: &SigningKey,
) -> (Connection, String) {
    let guest_public = key_hex(guest_key);
    let invitation = signed(
        broker,
        host_connection,
        host_key,
        "enrollment-invite",
        "enrollment.invite",
        json!({
            "uid": guest_uid,
            "public_key": guest_public,
            "idempotency_key": format!("enrollment-{guest_uid}")
        }),
    );
    assert!(
        invitation.error.is_none(),
        "invite failed: {:?}",
        invitation.error
    );
    let invitation = invitation.result.unwrap()["invitation"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut guest_connection = Connection::new();
    let device_id = digest(&guest_key.verifying_key().to_bytes());
    let challenge = broker.handle(
        guest_uid,
        &mut guest_connection,
        request(
            "guest-challenge",
            "auth.challenge",
            json!({"device_id": device_id}),
        ),
    );
    let nonce = challenge.result.unwrap()["nonce"]
        .as_str()
        .unwrap()
        .to_owned();
    let params = json!({"public_key": guest_public, "invitation": invitation});
    let signature = guest_key.sign(&signing_payload(
        &broker.workspace().id,
        guest_uid,
        &nonce,
        "auth.enroll",
        &params,
    ));
    let mut enroll = request("guest-enroll", "auth.enroll", params);
    enroll.auth = Some(DeviceAuth {
        device_id,
        nonce,
        signature: hex::encode(signature.to_bytes()),
    });
    let response = broker.handle(guest_uid, &mut guest_connection, enroll);
    assert!(
        response.error.is_none(),
        "enroll failed: {:?}",
        response.error
    );
    let principal = response.result.unwrap()["principal"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    (guest_connection, principal)
}
fn signed(
    broker: &mut Broker,
    connection: &mut Connection,
    key: &SigningKey,
    id: &str,
    method: &str,
    params: Value,
) -> biorouter_crew::Response {
    let params = if method == "run.create" {
        run_contract_params(broker, params)
    } else {
        params
    };
    signed_raw(broker, connection, key, id, method, params)
}

fn signed_raw(
    broker: &mut Broker,
    connection: &mut Connection,
    key: &SigningKey,
    id: &str,
    method: &str,
    params: Value,
) -> biorouter_crew::Response {
    let device_id = digest(&key.verifying_key().to_bytes());
    let challenge = broker.handle(
        uid(),
        connection,
        request(
            "challenge",
            "auth.challenge",
            json!({"device_id": device_id}),
        ),
    );
    let nonce = challenge.result.unwrap()["nonce"]
        .as_str()
        .unwrap()
        .to_owned();
    let signature = key.sign(&signing_payload(
        &broker.workspace().id,
        uid(),
        &nonce,
        method,
        &params,
    ));
    let mut req = request(id, method, params);
    req.auth = Some(DeviceAuth {
        device_id,
        nonce,
        signature: hex::encode(signature.to_bytes()),
    });
    broker.handle(uid(), connection, req)
}
fn with_cleanup<T>(root: &PathBuf, f: impl FnOnce() -> T) -> T {
    let result = f();
    let _ = fs::remove_dir_all(root);
    result
}

fn run_with_affiliation(
    broker: &mut Broker,
    connection: &mut Connection,
    key: &SigningKey,
    id: &str,
    channel: &str,
    affiliation: Value,
    personal_mode: &str,
) -> biorouter_crew::Response {
    signed(
        broker,
        connection,
        key,
        id,
        "run.create",
        json!({
            "channel_id": channel,
            "source_channels": [channel],
            "provider_policy_id": "resolved-provider-fixture",
            "provider_affiliation": affiliation,
            "personal_mode": personal_mode,
            "public_provider": personal_mode == "public",
            "expires_in": 60,
            "idempotency_key": id,
        }),
    )
}

#[test]
fn private_workspace_rejects_public_run_before_dispatch() {
    let root = temp_root("private-provider");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap(&root);
        let team_response = signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"synthetic", "idempotency_key":"team"}),
        );
        assert!(
            team_response.error.is_none(),
            "team.create failed: {:?}",
            team_response.error
        );
        let team = team_response.result.unwrap();
        let channel = team["channel"]["id"].as_str().unwrap();
        let response = signed(
            &mut broker,
            &mut connection,
            &key,
            "run",
            "run.create",
            json!({
            "channel_id": channel, "source_channels": [channel], "provider_policy_id": "public-test-sink",
            "personal_mode": "public", "public_provider": true, "expires_in": 60, "idempotency_key": "run"
            }),
        );
        assert_eq!(response.error.unwrap().code, "privacy_denied");
    });
}

#[test]
fn public_workspace_allows_public_safe_run() {
    let root = temp_root("public-provider");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap(&root);
        let policy = signed(
            &mut broker,
            &mut connection,
            &key,
            "policy",
            "policy.set",
            json!({"mode":"public", "idempotency_key":"policy"}),
        );
        assert!(
            policy.error.is_none(),
            "policy.set failed: {:?}",
            policy.error
        );
        let team = signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"public-safe", "idempotency_key":"team"}),
        )
        .result
        .unwrap();
        let channel = team["channel"]["id"].as_str().unwrap();
        let response = signed(
            &mut broker,
            &mut connection,
            &key,
            "run",
            "run.create",
            json!({
                "channel_id": channel, "source_channels": [channel], "provider_policy_id": "public-test-sink",
                "personal_mode": "public", "public_provider": true, "expires_in": 60, "idempotency_key": "run"
            }),
        );
        assert!(
            response.error.is_none(),
            "public-safe run failed: {:?}",
            response.error
        );
        assert_eq!(response.result.unwrap()["run"]["public_provider"], true);
    });
}

#[test]
fn attachment_begin_uses_attachment_handler() {
    let root = temp_root("attachment-begin");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap(&root);
        let team = signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"files", "idempotency_key":"team"}),
        )
        .result
        .unwrap();
        let channel = team["channel"]["id"].as_str().unwrap();
        let response = signed(
            &mut broker,
            &mut connection,
            &key,
            "blob",
            "blob.begin",
            json!({
                "channel_id": channel, "size": 0, "sha256": hex::encode([0u8; 32]),
                "name": "empty.bin", "media_type": "application/octet-stream", "idempotency_key": "blob"
            }),
        );
        assert!(
            response.error.is_none(),
            "blob.begin failed: {:?}",
            response.error
        );
        assert_eq!(response.result.unwrap()["complete"], false);
    });
}

#[test]
fn revoked_worker_credential_cannot_read_context() {
    let root = temp_root("revoke-worker");
    let remote_root = temp_root("revoke-worker-remote");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap(&root);
        let team = signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"worker", "idempotency_key":"team"}),
        )
        .result
        .unwrap();
        let channel = team["channel"]["id"].as_str().unwrap();
        let run = signed(&mut broker, &mut connection, &key, "run", "run.create", json!({
            "channel_id": channel, "source_channels": [channel], "provider_policy_id": "private-test",
            "personal_mode": "private", "public_provider": false, "remote_root": remote_root.to_string_lossy(),
            "expires_in": 60, "idempotency_key": "run"
        })).result.unwrap();
        let run_id = run["run"]["id"].as_str().unwrap().to_owned();
        let credential = run["credential"].as_str().unwrap().to_owned();
        let worker = Request {
            version: 1,
            id: "worker-read".into(),
            method: "context.manifest".into(),
            params: json!({}),
            auth: None,
            credential: Some(credential.clone()),
        };
        assert!(broker
            .handle(uid(), &mut connection, worker.clone())
            .error
            .is_none());
        let revoked = signed(
            &mut broker,
            &mut connection,
            &key,
            "revoke",
            "run.revoke",
            json!({"run_id": run_id, "idempotency_key":"revoke"}),
        );
        assert!(revoked.error.is_none());
        assert_eq!(
            broker
                .handle(uid(), &mut connection, worker)
                .error
                .unwrap()
                .code,
            "grant_expired"
        );
    });
    let _ = fs::remove_dir_all(&remote_root);
}

#[test]
fn policy_epoch_invalidates_existing_worker_grant() {
    let root = temp_root("policy-epoch");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap(&root);
        let team = signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"epoch", "idempotency_key":"team"}),
        )
        .result
        .unwrap();
        let channel = team["channel"]["id"].as_str().unwrap();
        let run = signed(&mut broker, &mut connection, &key, "run", "run.create", json!({
            "channel_id": channel, "source_channels": [channel], "provider_policy_id": "private-test",
            "personal_mode": "private", "public_provider": false, "expires_in": 60, "idempotency_key": "run"
        })).result.unwrap();
        let credential = run["credential"].as_str().unwrap().to_owned();
        let changed = signed(
            &mut broker,
            &mut connection,
            &key,
            "policy",
            "policy.set",
            json!({"mode":"public", "idempotency_key":"policy"}),
        );
        assert!(changed.error.is_none());
        let worker = Request {
            version: 1,
            id: "worker-read".into(),
            method: "context.manifest".into(),
            params: json!({}),
            auth: None,
            credential: Some(credential),
        };
        assert_eq!(
            broker
                .handle(uid(), &mut connection, worker)
                .error
                .unwrap()
                .code,
            "grant_expired"
        );
    });
}

#[test]
fn idempotency_replays_exact_result_and_rejects_changed_payload() {
    let root = temp_root("idempotency");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap(&root);
        let params = json!({"name":"same-key","idempotency_key":"fixed"});
        let first = signed(
            &mut broker,
            &mut connection,
            &key,
            "first",
            "team.create",
            params.clone(),
        );
        let second = signed(
            &mut broker,
            &mut connection,
            &key,
            "second",
            "team.create",
            params,
        );
        assert_eq!(first.result, second.result);
        let conflict = signed(
            &mut broker,
            &mut connection,
            &key,
            "third",
            "team.create",
            json!({"name":"changed","idempotency_key":"fixed"}),
        );
        assert_eq!(conflict.error.unwrap().code, "conflict");
    });
}

#[test]
fn signed_nonce_is_single_use() {
    let root = temp_root("nonce");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap(&root);
        let device_id = digest(&key.verifying_key().to_bytes());
        let challenge = broker.handle(
            uid(),
            &mut connection,
            request(
                "challenge",
                "auth.challenge",
                json!({"device_id": device_id}),
            ),
        );
        let nonce = challenge.result.unwrap()["nonce"]
            .as_str()
            .unwrap()
            .to_owned();
        let params = json!({"nickname":"nonce-test", "idempotency_key":"profile"});
        let signature = key.sign(&signing_payload(
            &broker.workspace().id,
            uid(),
            &nonce,
            "profile.update",
            &params,
        ));
        let auth = DeviceAuth {
            device_id,
            nonce,
            signature: hex::encode(signature.to_bytes()),
        };
        let mut req = request("profile", "profile.update", params);
        req.auth = Some(auth);
        assert!(broker
            .handle(uid(), &mut connection, req.clone())
            .error
            .is_none());
        assert_eq!(
            broker
                .handle(uid(), &mut connection, req)
                .error
                .unwrap()
                .code,
            "unauthorized"
        );
    });
}

#[test]
fn journal_replays_after_a_torn_final_record() {
    let root = temp_root("replay");
    with_cleanup(&root, || {
        let key = SigningKey::from_bytes(&[7; 32]);
        let public = key_hex(&key);
        {
            let (mut broker, mut connection, _) = bootstrap(&root);
            let result = signed(
                &mut broker,
                &mut connection,
                &key,
                "team",
                "team.create",
                json!({"name":"replay", "idempotency_key":"team"}),
            );
            assert!(
                result.error.is_none(),
                "team.create failed: {:?}",
                result.error
            );
        }
        fs::OpenOptions::new()
            .append(true)
            .open(root.join("journal.jsonl"))
            .unwrap()
            .write_all(b"{\"version\":1")
            .unwrap();
        let reopened = open_broker(&root, &public).unwrap();
        assert_eq!(reopened.workspace().mode, biorouter_crew::Mode::Private);
        assert!(fs::read_dir(&root).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("torn-tail-")));
    });
}

#[test]
fn signed_auth_binds_uid_and_device_key() {
    let root = temp_root("uid-device-binding");
    with_cleanup(&root, || {
        let (mut broker, mut host_connection, host_key) = bootstrap(&root);
        let guest_key = SigningKey::from_bytes(&[8; 32]);
        let guest_uid = guest_uid();
        let (_guest_connection, _guest_principal) = enroll_user(
            &mut broker,
            &mut host_connection,
            &host_key,
            guest_uid,
            &guest_key,
        );

        let forged_uid = signed_as(
            &mut broker,
            &mut host_connection,
            uid(),
            &guest_key,
            "forged-uid",
            "profile.update",
            json!({"nickname":"forged", "idempotency_key":"forged-uid"}),
        );
        assert_eq!(forged_uid.error.unwrap().code, "unauthorized");

        let forged_device = signed_as(
            &mut broker,
            &mut host_connection,
            guest_uid,
            &host_key,
            "forged-device",
            "profile.update",
            json!({"nickname":"forged", "idempotency_key":"forged-device"}),
        );
        assert_eq!(forged_device.error.unwrap().code, "unauthorized");
    });
}

#[test]
fn enrollment_invitation_is_bound_to_the_intended_device_key() {
    let root = temp_root("enrollment-key-binding");
    with_cleanup(&root, || {
        let (mut broker, mut host_connection, host_key) = bootstrap(&root);
        let guest_uid = guest_uid();
        let invited_key = SigningKey::from_bytes(&[8; 32]);
        let wrong_key = SigningKey::from_bytes(&[9; 32]);
        let invited_public = key_hex(&invited_key);
        let invitation = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "enrollment-invite",
            "enrollment.invite",
            json!({"uid":guest_uid,"public_key":invited_public,"idempotency_key":"invite"}),
        )
        .result
        .unwrap()["invitation"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut wrong_connection = Connection::new();
        let wrong_device_id = digest(&wrong_key.verifying_key().to_bytes());
        let challenge = broker.handle(
            guest_uid,
            &mut wrong_connection,
            request(
                "wrong-challenge",
                "auth.challenge",
                json!({"device_id": wrong_device_id}),
            ),
        );
        let nonce = challenge.result.unwrap()["nonce"]
            .as_str()
            .unwrap()
            .to_owned();
        let params = json!({"public_key":wrong_device_id,"invitation":invitation});
        let signature = wrong_key.sign(&signing_payload(
            &broker.workspace().id,
            guest_uid,
            &nonce,
            "auth.enroll",
            &params,
        ));
        let mut request = request("wrong-enroll", "auth.enroll", params);
        request.auth = Some(DeviceAuth {
            device_id: wrong_device_id,
            nonce,
            signature: hex::encode(signature.to_bytes()),
        });
        let response = broker.handle(0, &mut wrong_connection, request);
        assert_eq!(response.error.unwrap().code, "forbidden");
    });
}

#[test]
fn channel_membership_revocation_invalidates_reads_and_worker_grants() {
    let root = temp_root("membership-revocation");
    with_cleanup(&root, || {
        let (mut broker, mut host_connection, host_key) = bootstrap(&root);
        let guest_key = SigningKey::from_bytes(&[8; 32]);
        let guest_uid = guest_uid();
        let (mut guest_connection, guest_principal) = enroll_user(
            &mut broker,
            &mut host_connection,
            &host_key,
            guest_uid,
            &guest_key,
        );
        let team = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "team",
            "team.create",
            json!({"name":"revocation", "idempotency_key":"team"}),
        )
        .result
        .unwrap();
        let team_id = team["team"]["id"].as_str().unwrap();
        let general = team["channel"]["id"].as_str().unwrap();
        let invitation = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "team-invite",
            "invitation.create",
            json!({"kind":"team","target_id":team_id,"principal_id":guest_principal,"idempotency_key":"team-invite"}),
        )
        .result
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
        let accepted = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "team-accept",
            "invitation.accept",
            json!({"invitation_id":invitation,"idempotency_key":"team-accept"}),
        );
        assert!(
            accepted.error.is_none(),
            "team accept failed: {:?}",
            accepted.error
        );
        let run = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "guest-run",
            "run.create",
            json!({"channel_id":general,"source_channels":[general],"provider_policy_id":"private","personal_mode":"private","public_provider":false,"expires_in":60,"idempotency_key":"guest-run"}),
        )
        .result
        .unwrap();
        let credential = run["credential"].as_str().unwrap().to_owned();
        let before = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "before-read",
            "messages.history",
            json!({"channel_id":general}),
        );
        assert!(
            before.error.is_none(),
            "member could not read: {:?}",
            before.error
        );
        let revoked = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "revoke",
            "membership.revoke",
            json!({"channel_id":general,"principal_id":guest_principal,"idempotency_key":"revoke"}),
        );
        assert!(
            revoked.error.is_none(),
            "revoke failed: {:?}",
            revoked.error
        );
        let after = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "after-read",
            "messages.history",
            json!({"channel_id":general}),
        );
        assert_eq!(after.error.unwrap().code, "forbidden");
        let worker = broker.handle(
            guest_uid,
            &mut guest_connection,
            Request {
                version: 1,
                id: "after-revoke-worker".into(),
                method: "context.manifest".into(),
                params: json!({}),
                auth: None,
                credential: Some(credential),
            },
        );
        assert_eq!(worker.error.unwrap().code, "grant_expired");
    });
}

#[test]
fn ownership_transfer_removes_old_owner_and_invalidates_stale_invites() {
    let root = temp_root("ownership-transfer");
    with_cleanup(&root, || {
        let (mut broker, mut host_connection, host_key) = bootstrap(&root);
        let guest_key = SigningKey::from_bytes(&[8; 32]);
        let guest_uid = guest_uid();
        let (mut guest_connection, guest_principal) = enroll_user(
            &mut broker,
            &mut host_connection,
            &host_key,
            guest_uid,
            &guest_key,
        );
        let team = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "team",
            "team.create",
            json!({"name":"transfer", "idempotency_key":"team"}),
        )
        .result
        .unwrap();
        let team_id = team["team"]["id"].as_str().unwrap();
        let general = team["channel"]["id"].as_str().unwrap();
        let team_invite = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "team-invite",
            "invitation.create",
            json!({"kind":"team","target_id":team_id,"principal_id":guest_principal,"idempotency_key":"team-invite"}),
        )
        .result
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
        assert!(signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "team-accept",
            "invitation.accept",
            json!({"invitation_id":team_invite,"idempotency_key":"team-accept"})
        )
        .error
        .is_none());
        let channel = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "channel",
            "channel.create",
            json!({"team_id":team_id,"name":"transfer-me","classification":"restricted","idempotency_key":"channel"}),
        )
        .result
        .unwrap();
        let channel_id = channel["id"].as_str().unwrap().to_owned();
        let channel_invite = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "channel-invite",
            "invitation.create",
            json!({"kind":"channel","target_id":channel_id,"principal_id":guest_principal,"idempotency_key":"channel-invite"}),
        )
        .result
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
        assert!(signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "channel-accept",
            "invitation.accept",
            json!({"invitation_id":channel_invite,"idempotency_key":"channel-accept"})
        )
        .error
        .is_none());
        let stale = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "stale-invite",
            "invitation.create",
            json!({"kind":"channel","target_id":channel_id,"principal_id":guest_principal,"idempotency_key":"stale-invite"}),
        )
        .result
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
        let transfer = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "transfer",
            "channel.transfer",
            json!({"channel_id":channel_id,"successor_id":guest_principal,"idempotency_key":"transfer"}),
        );
        assert_eq!(
            transfer.result.unwrap()["created_by"],
            channel["created_by"]
        );
        assert!(signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "transfer-accept",
            "transfer.accept",
            json!({"channel_id":channel_id,"idempotency_key":"transfer-accept"})
        )
        .error
        .is_none());
        let stale_accept = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "stale-accept",
            "invitation.accept",
            json!({"invitation_id":stale,"idempotency_key":"stale-accept"}),
        );
        assert_eq!(stale_accept.error.unwrap().code, "forbidden");
        let old_owner = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "old-owner-read",
            "messages.history",
            json!({"channel_id":channel_id}),
        );
        assert_eq!(old_owner.error.unwrap().code, "forbidden");
        let new_invite = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "new-invite",
            "invitation.create",
            json!({"kind":"channel","target_id":channel_id,"principal_id":team["team"]["created_by"],"idempotency_key":"new-invite"}),
        );
        assert!(
            new_invite.error.is_none(),
            "successor lost invite authority: {:?}",
            new_invite.error
        );
        let _ = general;
    });
}

#[test]
fn cross_channel_attachment_and_reference_provenance_cannot_be_dropped() {
    let root = temp_root("provenance");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap(&root);
        let policy = signed(
            &mut broker,
            &mut connection,
            &key,
            "policy",
            "policy.set",
            json!({"mode":"public","idempotency_key":"policy"}),
        );
        assert!(policy.error.is_none());
        let team = signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"provenance", "idempotency_key":"team"}),
        )
        .result
        .unwrap();
        let team_id = team["team"]["id"].as_str().unwrap();
        let channel_a = team["channel"]["id"].as_str().unwrap().to_owned();
        let channel_b = signed(
            &mut broker,
            &mut connection,
            &key,
            "channel-b",
            "channel.create",
            json!({"team_id":team_id,"name":"source","classification":"public_safe","idempotency_key":"channel-b"}),
        )
        .result
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
        let data = [42u8];
        let blob = signed(
            &mut broker,
            &mut connection,
            &key,
            "blob-begin",
            "blob.begin",
            json!({"channel_id":channel_b,"size":1,"sha256":digest(&data),"name":"restricted.bin","media_type":"application/octet-stream","personal_mode":"private","idempotency_key":"blob-begin"}),
        )
        .result
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
        assert!(signed(
            &mut broker,
            &mut connection,
            &key,
            "blob-chunk",
            "blob.chunk",
            json!({"blob_id":blob,"offset":0,"data_hex":hex::encode(data),"idempotency_key":"blob-chunk"}),
        )
        .error
        .is_none());
        assert!(signed(
            &mut broker,
            &mut connection,
            &key,
            "blob-finish",
            "blob.finish",
            json!({"blob_id":blob,"idempotency_key":"blob-finish"}),
        )
        .error
        .is_none());
        let dropped_attachment = signed(
            &mut broker,
            &mut connection,
            &key,
            "drop-attachment-source",
            "message.post",
            json!({"channel_id":channel_a,"body":"cross-channel attachment","attachments":[blob],"personal_mode":"public","idempotency_key":"drop-attachment-source"}),
        );
        assert_eq!(dropped_attachment.error.unwrap().code, "forbidden");

        let reference = signed(
            &mut broker,
            &mut connection,
            &key,
            "reference",
            "reference.create",
            json!({"channel_id":channel_b,"path":"/tmp/synthetic-source","label":"restricted source","idempotency_key":"reference"}),
        )
        .result
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
        let dropped_reference = signed(
            &mut broker,
            &mut connection,
            &key,
            "drop-reference-source",
            "message.post",
            json!({"channel_id":channel_a,"body":"cross-channel reference","references":[reference],"personal_mode":"public","idempotency_key":"drop-reference-source"}),
        );
        assert_eq!(dropped_reference.error.unwrap().code, "forbidden");

        let run = signed(
            &mut broker,
            &mut connection,
            &key,
            "public-run",
            "run.create",
            json!({"channel_id":channel_a,"source_channels":[channel_a],"provider_policy_id":"public","personal_mode":"public","public_provider":true,"expires_in":60,"idempotency_key":"public-run"}),
        )
        .result
        .unwrap();
        let credential = run["credential"].as_str().unwrap().to_owned();
        let worker_blob = broker.handle(
            uid(),
            &mut connection,
            Request {
                version: 1,
                id: "worker-blob".into(),
                method: "blob.status".into(),
                params: json!({"blob_id":blob}),
                auth: None,
                credential: Some(credential.clone()),
            },
        );
        assert_eq!(worker_blob.error.unwrap().code, "privacy_denied");
        let worker_reference = broker.handle(
            uid(),
            &mut connection,
            Request {
                version: 1,
                id: "worker-reference".into(),
                method: "reference.get".into(),
                params: json!({"reference_id":reference}),
                auth: None,
                credential: Some(credential),
            },
        );
        assert_eq!(worker_reference.error.unwrap().code, "privacy_denied");
    });
}

#[test]
fn restricted_message_invalidates_existing_public_run() {
    let root = temp_root("public-invalidation");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap(&root);
        assert!(signed(
            &mut broker,
            &mut connection,
            &key,
            "policy",
            "policy.set",
            json!({"mode":"public","idempotency_key":"policy"}),
        )
        .error
        .is_none());
        let team = signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"public-transition", "idempotency_key":"team"}),
        )
        .result
        .unwrap();
        let channel = team["channel"]["id"].as_str().unwrap().to_owned();
        let run = signed(
            &mut broker,
            &mut connection,
            &key,
            "run",
            "run.create",
            json!({"channel_id":channel,"source_channels":[channel],"provider_policy_id":"public","personal_mode":"public","public_provider":true,"expires_in":60,"idempotency_key":"run"}),
        )
        .result
        .unwrap();
        let credential = run["credential"].as_str().unwrap().to_owned();
        let message = signed(
            &mut broker,
            &mut connection,
            &key,
            "restricted-message",
            "message.post",
            json!({"channel_id":channel,"body":"restricted-canary","idempotency_key":"restricted-message"}),
        );
        assert!(
            message.error.is_none(),
            "restricted post failed: {:?}",
            message.error
        );
        let worker = broker.handle(
            uid(),
            &mut connection,
            Request {
                version: 1,
                id: "worker-after-restriction".into(),
                method: "context.manifest".into(),
                params: json!({}),
                auth: None,
                credential: Some(credential),
            },
        );
        let worker_error = worker.error.unwrap();
        assert_eq!(worker_error.code, "grant_expired");
        assert!(worker_error
            .message
            .contains("selected context became protected"));
        assert!(worker.result.is_none());
        let fresh = signed(
            &mut broker,
            &mut connection,
            &key,
            "fresh-public-run",
            "run.create",
            json!({"channel_id":channel,"source_channels":[channel],"provider_policy_id":"public","personal_mode":"public","public_provider":true,"expires_in":60,"idempotency_key":"fresh-public-run"}),
        );
        assert_eq!(fresh.error.unwrap().code, "privacy_denied");
    });
}

#[test]
fn delayed_public_worker_response_rechecks_newly_restricted_channel() {
    let root = temp_root("public-delayed-response");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap(&root);
        assert!(signed(
            &mut broker,
            &mut connection,
            &key,
            "policy",
            "policy.set",
            json!({"mode":"public","idempotency_key":"policy"}),
        )
        .error
        .is_none());
        let team = signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"delayed-public","idempotency_key":"team"}),
        )
        .result
        .unwrap();
        let channel = team["channel"]["id"].as_str().unwrap().to_owned();
        let run = signed(
            &mut broker,
            &mut connection,
            &key,
            "run",
            "run.create",
            json!({"channel_id":channel,"source_channels":[channel],"provider_policy_id":"public","personal_mode":"public","public_provider":true,"expires_in":60,"idempotency_key":"run"}),
        )
        .result
        .unwrap();
        let credential = run["credential"].as_str().unwrap().to_owned();
        let progress = broker.handle(
            uid(),
            &mut connection,
            Request {
                version: 1,
                id: "progress".into(),
                method: "run.project".into(),
                params: json!({"body":"safe progress","status":"progress","idempotency_key":"progress"}),
                auth: None,
                credential: Some(credential.clone()),
            },
        );
        assert!(
            progress.error.is_none(),
            "initial progress failed: {:?}",
            progress.error
        );

        // This is the policy transition a queued public response must not cross.
        let restricted = signed(
            &mut broker,
            &mut connection,
            &key,
            "restricted",
            "message.post",
            json!({"channel_id":channel,"body":"private human update","personal_mode":"private","idempotency_key":"restricted"}),
        );
        assert!(
            restricted.error.is_none(),
            "restricted post failed: {:?}",
            restricted.error
        );
        let delayed = broker.handle(
            uid(),
            &mut connection,
            Request {
                version: 1,
                id: "delayed".into(),
                method: "run.project".into(),
                params: json!({"body":"late public response","status":"completed","idempotency_key":"delayed"}),
                auth: None,
                credential: Some(credential),
            },
        );
        let error = delayed
            .error
            .expect("protected transition must refuse the stale grant");
        assert_eq!(error.code, "grant_expired");
        assert!(delayed.result.is_none());
    });
}

#[test]
fn local_public_preflight_rejects_protection_transition_before_grant() {
    let root = temp_root("local-public-preflight-transition");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap(&root);
        assert!(signed(
            &mut broker,
            &mut connection,
            &key,
            "policy",
            "policy.set",
            json!({"mode":"public","idempotency_key":"policy"}),
        )
        .error
        .is_none());
        let team = signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"local-public-preflight","idempotency_key":"team"}),
        )
        .result
        .unwrap();
        let channel = team["channel"]["id"].as_str().unwrap().to_owned();
        let snapshot = signed(
            &mut broker,
            &mut connection,
            &key,
            "preflight",
            "workspace.snapshot",
            json!({}),
        );
        assert!(
            snapshot.error.is_none(),
            "preflight failed: {:?}",
            snapshot.error
        );
        let policy_epoch = broker.workspace().policy_epoch;
        let run = signed(
            &mut broker,
            &mut connection,
            &key,
            "initial-local-run",
            "run.create",
            json!({
                "channel_id":channel,
                "source_channels":[channel],
                "provider_policy_id":"local-provider",
                "provider_affiliation":{"kind":"local"},
                "workspace_institution_id":"ucsf",
                "connection_institution_id":"ucsf",
                "expected_workspace_policy_epoch":policy_epoch,
                "expected_protected_context":false,
                "personal_mode":"public",
                "public_provider":false,
                "expires_in":60,
                "idempotency_key":"initial-local-run"
            }),
        );
        assert!(
            run.error.is_none(),
            "initial local run failed: {:?}",
            run.error
        );
        let restricted = signed(
            &mut broker,
            &mut connection,
            &key,
            "restricted-human",
            "message.post",
            json!({"channel_id":channel,"body":"restricted human update","personal_mode":"private","idempotency_key":"restricted-human"}),
        );
        assert!(
            restricted.error.is_none(),
            "restricted post failed: {:?}",
            restricted.error
        );
        let policy_epoch = broker.workspace().policy_epoch;
        let stale = signed(
            &mut broker,
            &mut connection,
            &key,
            "stale-local-run",
            "run.create",
            json!({
                "channel_id":channel,
                "source_channels":[channel],
                "provider_policy_id":"local-provider",
                "provider_affiliation":{"kind":"local"},
                "workspace_institution_id":"ucsf",
                "connection_institution_id":"ucsf",
                "expected_workspace_policy_epoch":policy_epoch,
                "expected_protected_context":false,
                "personal_mode":"public",
                "public_provider":false,
                "expires_in":60,
                "idempotency_key":"stale-local-run"
            }),
        );
        assert_eq!(stale.error.unwrap().code, "stale_policy");
        assert!(stale.result.is_none());
    });
}

#[test]
fn local_public_grant_is_rechecked_after_source_taint() {
    let root = temp_root("local-public-grant-taint");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap(&root);
        assert!(signed(
            &mut broker,
            &mut connection,
            &key,
            "policy",
            "policy.set",
            json!({"mode":"public","idempotency_key":"policy"}),
        )
        .error
        .is_none());
        let team = signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"local-public-taint","idempotency_key":"team"}),
        )
        .result
        .unwrap();
        let channel = team["channel"]["id"].as_str().unwrap().to_owned();
        let policy_epoch = broker.workspace().policy_epoch;
        let run = signed(
            &mut broker,
            &mut connection,
            &key,
            "local-run",
            "run.create",
            json!({
                "channel_id":channel,
                "source_channels":[channel],
                "provider_policy_id":"local-provider",
                "provider_affiliation":{"kind":"local"},
                "workspace_institution_id":"ucsf",
                "connection_institution_id":"ucsf",
                "expected_workspace_policy_epoch":policy_epoch,
                "expected_protected_context":false,
                "personal_mode":"public",
                "public_provider":false,
                "expires_in":60,
                "idempotency_key":"local-run"
            }),
        )
        .result
        .unwrap();
        let credential = run["credential"].as_str().unwrap().to_owned();
        let restricted = signed(
            &mut broker,
            &mut connection,
            &key,
            "restricted-source",
            "message.post",
            json!({"channel_id":channel,"body":"source taint","personal_mode":"private","idempotency_key":"restricted-source"}),
        );
        assert!(
            restricted.error.is_none(),
            "restricted post failed: {:?}",
            restricted.error
        );
        let worker = broker.handle(
            uid(),
            &mut connection,
            Request {
                version: 1,
                id: "tainted-worker".into(),
                method: "context.manifest".into(),
                params: json!({}),
                auth: None,
                credential: Some(credential),
            },
        );
        assert_eq!(worker.error.unwrap().code, "grant_expired");
        assert!(worker.result.is_none());
    });
}

#[test]
fn worker_grant_epoch_change_requires_regrant_before_projection() {
    let root = temp_root("worker-regrant");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap(&root);
        let team = signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"regrant","idempotency_key":"team"}),
        )
        .result
        .unwrap();
        let channel = team["channel"]["id"].as_str().unwrap().to_owned();
        let first = signed(
            &mut broker,
            &mut connection,
            &key,
            "first-run",
            "run.create",
            json!({"channel_id":channel,"source_channels":[channel],"provider_policy_id":"private","personal_mode":"private","public_provider":false,"expires_in":60,"idempotency_key":"first-run"}),
        )
        .result
        .unwrap();
        let stale_credential = first["credential"].as_str().unwrap().to_owned();
        let before = broker.handle(
            uid(),
            &mut connection,
            Request {
                version: 1,
                id: "before".into(),
                method: "context.manifest".into(),
                params: json!({}),
                auth: None,
                credential: Some(stale_credential.clone()),
            },
        );
        assert!(
            before.error.is_none(),
            "fresh grant rejected: {:?}",
            before.error
        );
        let policy = signed(
            &mut broker,
            &mut connection,
            &key,
            "policy",
            "policy.set",
            json!({"mode":"public","idempotency_key":"policy"}),
        );
        assert!(
            policy.error.is_none(),
            "policy change failed: {:?}",
            policy.error
        );
        let stale = broker.handle(
            uid(),
            &mut connection,
            Request {
                version: 1,
                id: "stale".into(),
                method: "run.project".into(),
                params: json!({"body":"stale projection","status":"progress","idempotency_key":"stale"}),
                auth: None,
                credential: Some(stale_credential),
            },
        );
        assert_eq!(stale.error.unwrap().code, "grant_expired");

        let fresh = signed(
            &mut broker,
            &mut connection,
            &key,
            "fresh-run",
            "run.create",
            json!({"channel_id":channel,"source_channels":[channel],"provider_policy_id":"private","personal_mode":"private","public_provider":false,"expires_in":60,"idempotency_key":"fresh-run"}),
        )
        .result
        .unwrap();
        let fresh_credential = fresh["credential"].as_str().unwrap().to_owned();
        let accepted = broker.handle(
            uid(),
            &mut connection,
            Request {
                version: 1,
                id: "accepted".into(),
                method: "run.project".into(),
                params: json!({"body":"fresh projection","status":"progress","idempotency_key":"accepted"}),
                auth: None,
                credential: Some(fresh_credential),
            },
        );
        assert!(
            accepted.error.is_none(),
            "regrant was rejected: {:?}",
            accepted.error
        );
    });
}

#[test]
fn journal_v2_delta_replays_authoritative_state_after_restart() {
    let root = temp_root("v2-restart");
    let key = SigningKey::from_bytes(&[7; 32]);
    let public = key_hex(&key);
    {
        let (mut broker, mut connection, _) = bootstrap(&root);
        let team = signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"restart","idempotency_key":"team"}),
        );
        assert!(team.error.is_none());
    }
    let journal = fs::read_to_string(root.join("journal.jsonl")).unwrap();
    assert!(journal.lines().all(|line| line.contains("\"version\":2")));
    let (mut reopened, mut connection) = (open_broker(&root, &public).unwrap(), Connection::new());
    let snapshot = signed(
        &mut reopened,
        &mut connection,
        &key,
        "snapshot",
        "workspace.snapshot",
        json!({}),
    );
    assert_eq!(
        snapshot.result.unwrap()["teams"].as_array().unwrap().len(),
        1
    );
    drop(reopened);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn journal_replay_rejects_complete_record_checksum_corruption() {
    let root = temp_root("journal-corruption");
    let key = SigningKey::from_bytes(&[7; 32]);
    let public = key_hex(&key);
    {
        let (mut broker, mut connection, _) = bootstrap(&root);
        assert!(signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"corrupt","idempotency_key":"team"}),
        )
        .error
        .is_none());
    }
    let mut bytes = fs::read(root.join("journal.jsonl")).unwrap();
    let marker = b"\"checksum\":\"";
    let offset = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap()
        + marker.len();
    bytes[offset] = if bytes[offset] == b'0' { b'1' } else { b'0' };
    fs::write(root.join("journal.jsonl"), bytes).unwrap();
    match open_broker(&root, &public) {
        Ok(_) => panic!("corrupted complete journal record was accepted"),
        Err(error) => assert!(error.to_string().starts_with("journal_corrupt:"), "{error}"),
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn host_remote_scope_cannot_overlap_broker_authority_state() {
    let root = temp_root("remote-authority");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap(&root);
        let team = signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"authority","idempotency_key":"team"}),
        )
        .result
        .unwrap();
        let channel = team["channel"]["id"].as_str().unwrap();
        let inside = root.join("inside-authority");
        fs::create_dir(&inside).unwrap();
        let response = signed(
            &mut broker,
            &mut connection,
            &key,
            "run",
            "run.create",
            json!({"channel_id":channel,"source_channels":[channel],"provider_policy_id":"private","personal_mode":"private","public_provider":false,"remote_root":inside,"expires_in":60,"idempotency_key":"run"}),
        );
        assert_eq!(response.error.unwrap().code, "forbidden");
    });
}

#[test]
fn logical_state_quota_failure_does_not_commit_the_oversized_mutation() {
    let root = temp_root("state-quota");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap(&root);
        let team = signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"quota","idempotency_key":"team"}),
        )
        .result
        .unwrap();
        let channel = team["channel"]["id"].as_str().unwrap().to_owned();
        let mut quota_error = None;
        for index in 0..320 {
            let body = format!("quota-marker-{index}-{}", "x".repeat(65_500));
            let response = signed(
                &mut broker,
                &mut connection,
                &key,
                &format!("quota-{index}"),
                "message.post",
                json!({"channel_id":channel,"body":body,"idempotency_key":format!("quota-{index}")}),
            );
            if let Some(error) = response.error {
                quota_error = Some((index, error.code));
                break;
            }
        }
        let (index, code) = quota_error.expect("the bounded state quota should be reached");
        assert_eq!(
            code, "quota_exceeded",
            "quota failed for an unexpected reason at {index}"
        );
        let search = signed(
            &mut broker,
            &mut connection,
            &key,
            "quota-search",
            "messages.search",
            json!({"channel_id":channel,"query":"quota-marker-319-","limit":20}),
        );
        assert!(
            search.error.is_none(),
            "read after quota denial failed: {:?}",
            search.error
        );
        assert!(search.result.unwrap()["messages"]
            .as_array()
            .unwrap()
            .is_empty());
    });
}

#[test]
fn additional_device_invitation_requires_the_active_principal_binding() {
    let root = temp_root("additional-device");
    with_cleanup(&root, || {
        let (mut broker, mut connection, host_key) = bootstrap(&root);
        let principal = signed(
            &mut broker,
            &mut connection,
            &host_key,
            "snapshot",
            "workspace.snapshot",
            json!({}),
        )
        .result
        .unwrap()["actor"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let second_key = SigningKey::from_bytes(&[10; 32]);
        let public_key = key_hex(&second_key);
        let missing_binding = signed(
            &mut broker,
            &mut connection,
            &host_key,
            "missing-binding",
            "enrollment.invite",
            json!({"uid":uid(),"public_key":public_key,"idempotency_key":"missing-binding"}),
        );
        assert_eq!(missing_binding.error.unwrap().code, "invalid_params");
        let invitation = signed(
            &mut broker,
            &mut connection,
            &host_key,
            "bound-invite",
            "enrollment.invite",
            json!({"uid":uid(),"public_key":public_key,"existing_principal_id":principal,"idempotency_key":"bound-invite"}),
        )
        .result
        .unwrap()["invitation"]
        .as_str()
        .unwrap()
        .to_owned();
        let mut second_connection = Connection::new();
        let device_id = digest(&second_key.verifying_key().to_bytes());
        let challenge = broker.handle(
            uid(),
            &mut second_connection,
            request(
                "second-challenge",
                "auth.challenge",
                json!({"device_id":device_id}),
            ),
        );
        let nonce = challenge.result.unwrap()["nonce"]
            .as_str()
            .unwrap()
            .to_owned();
        let params = json!({"public_key":public_key,"invitation":invitation});
        let signature = second_key.sign(&signing_payload(
            &broker.workspace().id,
            uid(),
            &nonce,
            "auth.enroll",
            &params,
        ));
        let mut enroll = request("second-enroll", "auth.enroll", params);
        enroll.auth = Some(DeviceAuth {
            device_id,
            nonce,
            signature: hex::encode(signature.to_bytes()),
        });
        let enrolled = broker.handle(uid(), &mut second_connection, enroll);
        assert!(
            enrolled.error.is_none(),
            "additional device rejected: {:?}",
            enrolled.error
        );
        assert_eq!(enrolled.result.unwrap()["principal"]["id"], principal);
    });
}

#[test]
fn empty_human_message_requires_and_preserves_a_validated_attachment() {
    let root = temp_root("empty-message");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap(&root);
        let team = signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"empty-body","idempotency_key":"team"}),
        )
        .result
        .unwrap();
        let channel = team["channel"]["id"].as_str().unwrap().to_owned();
        let blob = signed(
            &mut broker,
            &mut connection,
            &key,
            "blob",
            "blob.begin",
            json!({"channel_id":channel,"size":0,"sha256":digest(&[]),"name":"empty.bin","media_type":"application/octet-stream","idempotency_key":"blob"}),
        )
        .result
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
        assert!(signed(
            &mut broker,
            &mut connection,
            &key,
            "finish",
            "blob.finish",
            json!({"blob_id":blob,"idempotency_key":"finish"}),
        )
        .error
        .is_none());
        let empty_with_attachment = signed(
            &mut broker,
            &mut connection,
            &key,
            "attachment-only",
            "message.post",
            json!({"channel_id":channel,"body":"","attachments":[blob],"idempotency_key":"attachment-only"}),
        );
        assert!(
            empty_with_attachment.error.is_none(),
            "validated attachment should permit an empty body: {:?}",
            empty_with_attachment.error
        );
        assert_eq!(empty_with_attachment.result.unwrap()["body"], "");
        let empty_without_context = signed(
            &mut broker,
            &mut connection,
            &key,
            "empty",
            "message.post",
            json!({"channel_id":channel,"body":"","idempotency_key":"empty"}),
        );
        assert_eq!(empty_without_context.error.unwrap().code, "invalid_params");
    });
}

#[cfg(target_os = "linux")]
#[test]
fn wrong_writer_node_refuses_before_torn_tail_repair() {
    let root = temp_root("wrong-node-tail");
    let key = SigningKey::from_bytes(&[7; 32]);
    let public = key_hex(&key);
    {
        let (mut broker, mut connection, _) = bootstrap(&root);
        assert!(signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"wrong-node","idempotency_key":"team"}),
        )
        .error
        .is_none());
    }
    let journal_path = root.join("journal.jsonl");
    let mut lines = fs::read_to_string(&journal_path)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let previous: Value = serde_json::from_str(lines.last().unwrap()).unwrap();
    let sequence = previous["sequence"].as_u64().unwrap() + 1;
    let previous_checksum = previous["checksum"].as_str().unwrap().to_owned();
    let wrong_node = "0".repeat(64);
    let patches = vec![
        json!({"op":"set","path":["sequence"],"value":sequence}),
        json!({"op":"set","path":["writer_node_id"],"value":wrong_node}),
    ];
    let timestamp = 1u64;
    let checksum = digest(
        &serde_json::to_vec(&(
            2u32,
            sequence,
            &previous_checksum,
            "test",
            "test.writer_node",
            timestamp,
            &patches,
        ))
        .unwrap(),
    );
    let record = json!({
        "version":2,
        "sequence":sequence,
        "previous":previous_checksum,
        "checksum":checksum,
        "actor":"test",
        "operation":"test.writer_node",
        "timestamp":timestamp,
        "patches":patches,
    });
    lines.push(serde_json::to_string(&record).unwrap());
    fs::write(
        &journal_path,
        format!("{}\n{{\"version\":2", lines.join("\n")),
    )
    .unwrap();
    let before_journal = fs::read(&journal_path).unwrap();
    let before_entries = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<std::collections::BTreeSet<_>>();
    match open_broker(&root, &public) {
        Ok(_) => panic!("mismatched writer node was accepted"),
        Err(error) => assert!(
            error.to_string().starts_with("node_identity_changed:"),
            "{error}"
        ),
    }
    assert_eq!(fs::read(&journal_path).unwrap(), before_journal);
    let after_entries = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(after_entries, before_entries);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn resolved_provider_affiliation_controls_private_run_admission() {
    let root = temp_root("provider-affiliation");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap(&root);
        let team = signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"affiliation", "idempotency_key":"team"}),
        )
        .result
        .unwrap();
        let channel = team["channel"]["id"].as_str().unwrap();

        let local = run_with_affiliation(
            &mut broker,
            &mut connection,
            &key,
            "local",
            channel,
            json!({"kind":"local"}),
            "private",
        );
        assert!(
            local.error.is_none(),
            "local private run: {:?}",
            local.error
        );

        let ucsf = run_with_affiliation(
            &mut broker,
            &mut connection,
            &key,
            "ucsf",
            channel,
            json!({"kind":"institutions","institution_ids":["ucsf"]}),
            "private",
        );
        assert!(ucsf.error.is_none(), "UCSF private run: {:?}", ucsf.error);

        for (id, affiliation) in [
            (
                "foreign",
                json!({"kind":"institutions","institution_ids":["stanford"]}),
            ),
            ("unstated", json!({"kind":"unstated"})),
        ] {
            let response = run_with_affiliation(
                &mut broker,
                &mut connection,
                &key,
                id,
                channel,
                affiliation,
                "private",
            );
            assert_eq!(response.error.unwrap().code, "privacy_denied", "{id}");
            assert!(
                response.result.is_none(),
                "denied {id} run produced a result"
            );
        }

        let public = run_with_affiliation(
            &mut broker,
            &mut connection,
            &key,
            "public",
            channel,
            json!({"kind":"institutions","institution_ids":["ucsf"]}),
            "public",
        );
        assert_eq!(public.error.unwrap().code, "privacy_denied");
        assert!(public.result.is_none());
    });
}

#[test]
fn protected_unlabelled_workspace_denies_even_local_provider_affiliation() {
    let root = temp_root("unlabelled-private");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap_unlabelled(&root);
        let team = signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"unlabelled", "idempotency_key":"team"}),
        )
        .result
        .unwrap();
        let channel = team["channel"]["id"].as_str().unwrap();
        for (id, affiliation) in [
            ("local", json!({"kind":"local"})),
            ("unstated", json!({"kind":"unstated"})),
        ] {
            let response = run_with_affiliation(
                &mut broker,
                &mut connection,
                &key,
                id,
                channel,
                affiliation,
                "private",
            );
            assert_eq!(response.error.unwrap().code, "privacy_denied", "{id}");
            assert!(
                response.result.is_none(),
                "{id} bypassed unlabelled protection"
            );
        }
    });
}

#[test]
fn run_create_requires_all_institution_and_epoch_fields() {
    let root = temp_root("run-contract-fields");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap(&root);
        let team = signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"required-fields", "idempotency_key":"team"}),
        )
        .result
        .unwrap();
        let channel = team["channel"]["id"].as_str().unwrap();
        let base = json!({
            "channel_id": channel,
            "source_channels": [channel],
            "provider_policy_id": "resolved-provider-fixture",
            "provider_affiliation": {"kind":"local"},
            "workspace_institution_id": "ucsf",
            "connection_institution_id": "ucsf",
            "expected_workspace_policy_epoch": broker.workspace().policy_epoch,
            "expected_protected_context": true,
            "personal_mode": "private",
            "public_provider": false,
            "expires_in": 60,
        });
        for (index, missing) in [
            "provider_affiliation",
            "workspace_institution_id",
            "connection_institution_id",
            "expected_workspace_policy_epoch",
            "expected_protected_context",
        ]
        .into_iter()
        .enumerate()
        {
            let mut params = base.clone();
            params.as_object_mut().unwrap().remove(missing);
            params["idempotency_key"] = json!(format!("missing-{index}"));
            let response = signed_raw(
                &mut broker,
                &mut connection,
                &key,
                &format!("missing-{index}"),
                "run.create",
                params,
            );
            assert_eq!(response.error.unwrap().code, "invalid_params", "{missing}");
            assert!(response.result.is_none());
        }
    });
}

#[test]
fn forged_institution_metadata_is_refused_before_run_creation() {
    let root = temp_root("forged-institution");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap(&root);
        let team = signed(
            &mut broker,
            &mut connection,
            &key,
            "team",
            "team.create",
            json!({"name":"forged", "idempotency_key":"team"}),
        )
        .result
        .unwrap();
        let channel = team["channel"]["id"].as_str().unwrap();
        let epoch = broker.workspace().policy_epoch;
        let mismatched_connection = signed(
            &mut broker,
            &mut connection,
            &key,
            "foreign-connection",
            "run.create",
            json!({
                "channel_id": channel,
                "source_channels": [channel],
                "provider_policy_id": "resolved-provider-fixture",
                "provider_affiliation": {"kind":"institutions","institution_ids":["stanford"]},
                "workspace_institution_id": "ucsf",
                "connection_institution_id": "stanford",
                "expected_workspace_policy_epoch": epoch,
                "personal_mode": "private",
                "public_provider": false,
                "expires_in": 60,
                "idempotency_key": "foreign-connection",
            }),
        );
        assert_eq!(mismatched_connection.error.unwrap().code, "privacy_denied");
        assert!(mismatched_connection.result.is_none());

        let epoch = broker.workspace().policy_epoch;
        let forged_workspace = signed(
            &mut broker,
            &mut connection,
            &key,
            "forged-workspace",
            "run.create",
            json!({
                "channel_id": channel,
                "source_channels": [channel],
                "provider_policy_id": "resolved-provider-fixture",
                "provider_affiliation": {"kind":"institutions","institution_ids":["ucsf"]},
                "workspace_institution_id": "stanford",
                "connection_institution_id": "stanford",
                "expected_workspace_policy_epoch": epoch,
                "personal_mode": "private",
                "public_provider": false,
                "expires_in": 60,
                "idempotency_key": "forged-workspace",
            }),
        );
        assert_eq!(forged_workspace.error.unwrap().code, "stale_policy");
        assert!(forged_workspace.result.is_none());
    });
}
