#![cfg(unix)]

mod support;

use biorouter_crew::{signing_payload, Broker, Connection, DeviceAuth, Request};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
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

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn temp_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "biorouter-crew-adversarial-{label}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is after the Unix epoch")
            .as_nanos()
    ));
    fs::create_dir(&root).expect("create test root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("protect test root");
    root
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
            &format!("{id}-challenge"),
            "auth.challenge",
            json!({"device_id": device_id}),
        ),
    );
    let nonce = challenge
        .result
        .expect("challenge succeeds")
        .get("nonce")
        .and_then(Value::as_str)
        .expect("challenge returns nonce")
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

fn signed(
    broker: &mut Broker,
    connection: &mut Connection,
    key: &SigningKey,
    id: &str,
    method: &str,
    params: Value,
) -> biorouter_crew::Response {
    signed_as(broker, connection, uid(), key, id, method, params)
}

fn bootstrap(root: &Path) -> (Broker, Connection, SigningKey) {
    let key = SigningKey::from_bytes(&[7; 32]);
    let public = key_hex(&key);
    let mut broker = open_broker(root, &public).expect("open broker");
    let mut connection = Connection::new();
    let device_id = digest(&key.verifying_key().to_bytes());
    let challenge = broker.handle(
        uid(),
        &mut connection,
        request(
            "bootstrap-challenge",
            "auth.challenge",
            json!({"device_id": device_id}),
        ),
    );
    let nonce = challenge.result.expect("bootstrap challenge")["nonce"]
        .as_str()
        .expect("bootstrap nonce")
        .to_owned();
    let params = json!({"public_key": public});
    let signature = key.sign(&signing_payload(
        &broker.workspace().id,
        uid(),
        &nonce,
        "auth.bootstrap",
        &params,
    ));
    let mut req = request("bootstrap", "auth.bootstrap", params);
    req.auth = Some(DeviceAuth {
        device_id,
        nonce,
        signature: hex::encode(signature.to_bytes()),
    });
    assert!(broker.handle(uid(), &mut connection, req).error.is_none());
    (broker, connection, key)
}

fn enroll_user(
    broker: &mut Broker,
    host_connection: &mut Connection,
    host_key: &SigningKey,
    guest_key: &SigningKey,
) -> (Connection, String, u32) {
    let guest_uid = guest_uid();
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
            "idempotency_key": "adversarial-enrollment"
        }),
    )
    .result
    .expect("enrollment invitation")["invitation"]
        .as_str()
        .expect("invitation token")
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
    let nonce = challenge.result.expect("guest challenge")["nonce"]
        .as_str()
        .expect("guest nonce")
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
    let principal = response.result.expect("guest enrollment")["principal"]["id"]
        .as_str()
        .expect("guest principal")
        .to_owned();
    (guest_connection, principal, guest_uid)
}

fn cleanup(root: &Path) {
    let _ = fs::remove_dir_all(root);
}

#[test]
fn membership_revocation_between_blob_chunks_denies_completion_and_reads() {
    let root = temp_root("blob-revoke");
    {
        let (mut broker, mut host_connection, host_key) = bootstrap(&root);
        let guest_key = SigningKey::from_bytes(&[8; 32]);
        let (mut guest_connection, guest_principal, guest_uid) =
            enroll_user(&mut broker, &mut host_connection, &host_key, &guest_key);
        let team = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "team",
            "team.create",
            json!({"name":"blob-revoke", "idempotency_key":"team"}),
        )
        .result
        .expect("team creation");
        let team_id = team["team"]["id"].as_str().expect("team id");
        let channel_id = team["channel"]["id"].as_str().expect("channel id");
        let invite = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "team-invite",
            "invitation.create",
            json!({
                "kind":"team",
                "target_id":team_id,
                "principal_id":guest_principal,
                "idempotency_key":"team-invite"
            }),
        )
        .result
        .expect("team invitation")["id"]
            .as_str()
            .expect("invitation id")
            .to_owned();
        assert!(signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "team-accept",
            "invitation.accept",
            json!({"invitation_id":invite, "idempotency_key":"team-accept"}),
        )
        .error
        .is_none());

        let bytes = b"abcdef";
        let blob = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "blob-begin",
            "blob.begin",
            json!({
                "channel_id":channel_id,
                "size":bytes.len(),
                "sha256":digest(bytes),
                "name":"payload.bin",
                "media_type":"application/octet-stream",
                "idempotency_key":"blob"
            }),
        )
        .result
        .expect("blob begin");
        let blob_id = blob["id"].as_str().expect("blob id").to_owned();
        let first = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "blob-first",
            "blob.chunk",
            json!({"blob_id":blob_id,"offset":0,"data_hex":hex::encode(b"abc"),"idempotency_key":"chunk-1"}),
        );
        assert!(first.error.is_none(), "first chunk is the positive control");
        assert_eq!(first.result.expect("first chunk result")["offset"], 3);
        let status = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "blob-status-before-revoke",
            "blob.status",
            json!({"blob_id":blob_id}),
        );
        assert!(
            status.error.is_none(),
            "owner can inspect an incomplete blob"
        );

        let revoked = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "revoke",
            "membership.revoke",
            json!({"channel_id":channel_id,"principal_id":guest_principal,"idempotency_key":"revoke"}),
        );
        assert!(revoked.error.is_none(), "owner revocation succeeds");
        let journal_after_revoke = fs::read(root.join("journal.jsonl")).expect("read journal");
        let blob_after_revoke =
            fs::read(root.join("blobs").join(&blob_id)).expect("read partial blob");
        assert_eq!(blob_after_revoke, b"abc");

        for (id, method, params) in [
            (
                "blob-second",
                "blob.chunk",
                json!({"blob_id":blob_id,"offset":3,"data_hex":hex::encode(b"def"),"idempotency_key":"chunk-2"}),
            ),
            (
                "blob-finish",
                "blob.finish",
                json!({"blob_id":blob_id,"idempotency_key":"finish"}),
            ),
        ] {
            let response = signed_as(
                &mut broker,
                &mut guest_connection,
                guest_uid,
                &guest_key,
                id,
                method,
                params,
            );
            assert_eq!(
                response.error.expect("revoked operation denied").code,
                "forbidden"
            );
            assert_eq!(
                fs::read(root.join("journal.jsonl")).expect("read journal after denial"),
                journal_after_revoke,
                "denied blob operation must not journal a mutation"
            );
            assert_eq!(
                fs::read(root.join("blobs").join(&blob_id)).expect("read blob after denial"),
                blob_after_revoke,
                "denied blob operation must not write another chunk"
            );
        }
        for (id, method, params) in [
            (
                "blob-status-after-revoke",
                "blob.status",
                json!({"blob_id":blob_id}),
            ),
            (
                "blob-read-after-revoke",
                "blob.read",
                json!({"blob_id":blob_id,"offset":0}),
            ),
        ] {
            let response = signed_as(
                &mut broker,
                &mut guest_connection,
                guest_uid,
                &guest_key,
                id,
                method,
                params,
            );
            assert_eq!(
                response.error.expect("revoked read denied").code,
                "forbidden"
            );
            assert_eq!(
                fs::read(root.join("journal.jsonl")).expect("read journal after denied read"),
                journal_after_revoke,
                "denied blob read must not journal a mutation"
            );
            assert_eq!(
                fs::read(root.join("blobs").join(&blob_id)).expect("read blob after denied read"),
                blob_after_revoke,
                "denied blob read must not alter the partial blob"
            );
        }
    }
    cleanup(&root);
}

#[test]
fn non_owner_cannot_archive_or_transfer_channel() {
    let root = temp_root("owner-controls");
    {
        let (mut broker, mut host_connection, host_key) = bootstrap(&root);
        let guest_key = SigningKey::from_bytes(&[8; 32]);
        let (mut guest_connection, guest_principal, guest_uid) =
            enroll_user(&mut broker, &mut host_connection, &host_key, &guest_key);
        let team = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "team",
            "team.create",
            json!({"name":"owner-controls", "idempotency_key":"team"}),
        )
        .result
        .expect("team creation");
        let team_id = team["team"]["id"].as_str().expect("team id");
        let team_invite = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "team-invite",
            "invitation.create",
            json!({"kind":"team","target_id":team_id,"principal_id":guest_principal,"idempotency_key":"team-invite"}),
        )
        .result
        .expect("team invitation")["id"]
            .as_str()
            .expect("team invitation id")
            .to_owned();
        assert!(signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "team-accept",
            "invitation.accept",
            json!({"invitation_id":team_invite,"idempotency_key":"team-accept"}),
        )
        .error
        .is_none());
        let channel = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "channel",
            "channel.create",
            json!({"team_id":team_id,"name":"owned","classification":"restricted","idempotency_key":"channel"}),
        )
        .result
        .expect("channel creation");
        let channel_id = channel["id"].as_str().expect("channel id").to_owned();
        let invite = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "channel-invite",
            "invitation.create",
            json!({"kind":"channel","target_id":channel_id,"principal_id":guest_principal,"idempotency_key":"invite"}),
        )
        .result
        .expect("channel invitation")["id"]
            .as_str()
            .expect("invitation id")
            .to_owned();
        assert!(signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "accept",
            "invitation.accept",
            json!({"invitation_id":invite,"idempotency_key":"accept"}),
        )
        .error
        .is_none());
        assert!(signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "member-read",
            "messages.history",
            json!({"channel_id":channel_id}),
        )
        .error
        .is_none());

        for (id, method, params) in [
            (
                "guest-archive",
                "channel.archive",
                json!({"channel_id":channel_id,"idempotency_key":"guest-archive"}),
            ),
            (
                "guest-transfer",
                "channel.transfer",
                json!({"channel_id":channel_id,"successor_id":team["team"]["created_by"],"idempotency_key":"guest-transfer"}),
            ),
        ] {
            let response = signed_as(
                &mut broker,
                &mut guest_connection,
                guest_uid,
                &guest_key,
                id,
                method,
                params,
            );
            assert_eq!(
                response.error.expect("non-owner operation denied").code,
                "forbidden"
            );
        }
        let transfer = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "owner-transfer",
            "channel.transfer",
            json!({"channel_id":channel_id,"successor_id":guest_principal,"idempotency_key":"owner-transfer"}),
        );
        assert!(transfer.error.is_none(), "current owner can offer transfer");
        assert!(signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "accept-transfer",
            "transfer.accept",
            json!({"channel_id":channel_id,"idempotency_key":"accept-transfer"}),
        )
        .error
        .is_none());
        let old_owner_archive = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "old-owner-archive",
            "channel.archive",
            json!({"channel_id":channel_id,"idempotency_key":"old-owner-archive"}),
        );
        assert_eq!(
            old_owner_archive.error.expect("old owner removed").code,
            "forbidden"
        );
        assert!(signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "new-owner-archive",
            "channel.archive",
            json!({"channel_id":channel_id,"idempotency_key":"new-owner-archive"}),
        )
        .error
        .is_none());
    }
    cleanup(&root);
}

#[test]
fn broker_open_rejects_concurrent_writer_and_allows_reopen_after_release() {
    let root = temp_root("writer-lock");
    let key = SigningKey::from_bytes(&[7; 32]);
    let public = key_hex(&key);
    let first = open_broker(&root, &public).expect("first writer opens");
    let second = match open_broker(&root, &public) {
        Ok(_) => panic!("second writer must be fenced"),
        Err(error) => error,
    };
    assert!(second.to_string().contains("writer_active"), "{second:#}");
    drop(first);
    let reopened = open_broker(&root, &public).expect("writer lock releases with broker");
    assert_eq!(reopened.workspace().host_uid, uid());
    drop(reopened);
    cleanup(&root);
}

#[test]
fn request_version_id_and_parameter_bounds_reject_malformed_inputs() {
    let root = temp_root("protocol-bounds");
    {
        let (mut broker, mut connection, key) = bootstrap(&root);
        let device_id = digest(&key.verifying_key().to_bytes());
        let valid = broker.handle(
            uid(),
            &mut connection,
            request(
                "challenge",
                "auth.challenge",
                json!({"device_id":device_id}),
            ),
        );
        assert!(
            valid.error.is_none(),
            "valid challenge is the positive control"
        );
        assert!(valid.result.expect("challenge result")["nonce"].is_string());

        let mut wrong_version = request(
            "wrong-version",
            "auth.challenge",
            json!({"device_id":device_id}),
        );
        wrong_version.version = 2;
        assert_eq!(
            broker
                .handle(uid(), &mut connection, wrong_version)
                .error
                .expect("version rejected")
                .code,
            "invalid_request"
        );

        let oversized_id = request(
            &"i".repeat(129),
            "auth.challenge",
            json!({"device_id":device_id}),
        );
        assert_eq!(
            broker
                .handle(uid(), &mut connection, oversized_id)
                .error
                .expect("request id rejected")
                .code,
            "invalid_request"
        );

        let oversized_device = request(
            "oversized-device",
            "auth.challenge",
            json!({"device_id":"a".repeat(biorouter_crew::MAX_FRAME + 1)}),
        );
        assert_eq!(
            broker
                .handle(uid(), &mut connection, oversized_device)
                .error
                .expect("oversized field rejected")
                .code,
            "invalid_params"
        );
    }
    cleanup(&root);
}
