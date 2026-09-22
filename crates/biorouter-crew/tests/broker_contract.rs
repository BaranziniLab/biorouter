use biorouter_crew::{signing_payload, Broker, Connection, DeviceAuth, Request};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    io::Write,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

fn uid() -> u32 {
    unsafe { libc::geteuid() }
}
fn guest_uid() -> u32 {
    if uid() == 0 {
        1
    } else {
        0
    }
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
fn signed_as(
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
fn bootstrap(root: &PathBuf) -> (Broker, Connection, SigningKey) {
    let key = SigningKey::from_bytes(&[7; 32]);
    let public = key_hex(&key);
    let mut broker = Broker::open(root, &public).unwrap();
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
    let result = with_cleanup(&root, || {
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
    result
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
        let reopened = Broker::open(&root, &public).unwrap();
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
        assert_eq!(worker.error.unwrap().code, "privacy_denied");
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
    let (mut reopened, mut connection) = (Broker::open(&root, &public).unwrap(), Connection::new());
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
    match Broker::open(&root, &public) {
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
    match Broker::open(&root, &public) {
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
