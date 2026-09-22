#![cfg(target_os = "linux")]

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

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn key_hex(key: &SigningKey) -> String {
    hex::encode(key.verifying_key().to_bytes())
}

fn temp_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "biorouter-crew-journal-fault-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is after the Unix epoch")
            .as_nanos()
    ));
    fs::create_dir(&root).expect("create fixture root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("protect fixture root");
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
            &format!("{id}-challenge"),
            "auth.challenge",
            json!({"device_id":device_id}),
        ),
    );
    let nonce = challenge.result.expect("challenge succeeds")["nonce"]
        .as_str()
        .expect("challenge nonce")
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

fn bootstrap(root: &Path, key: &SigningKey) -> (Broker, Connection) {
    let public = key_hex(key);
    let mut broker = Broker::open(root, &public).expect("open broker");
    let mut connection = Connection::new();
    let device_id = digest(&key.verifying_key().to_bytes());
    let challenge = broker.handle(
        uid(),
        &mut connection,
        request(
            "bootstrap-challenge",
            "auth.challenge",
            json!({"device_id":device_id}),
        ),
    );
    let nonce = challenge.result.expect("bootstrap challenge")["nonce"]
        .as_str()
        .expect("bootstrap nonce")
        .to_owned();
    let params = json!({"public_key":public});
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
    (broker, connection)
}

fn blob_snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut entries = fs::read_dir(root.join("blobs"))
        .expect("read blob directory")
        .map(|entry| {
            let path = entry.expect("read blob entry").path();
            (
                path.file_name()
                    .expect("blob filename")
                    .to_string_lossy()
                    .into_owned(),
                fs::read(path).expect("read blob file"),
            )
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

#[test]
#[ignore = "requires the disposable Linux LD_PRELOAD interposer fixture"]
fn journal_fault_preserves_prior_ack_after_restart() {
    let marker = PathBuf::from(std::env::var("CREW_FAULT_MARKER").expect("fault marker path"));
    let hit_file = PathBuf::from(std::env::var("CREW_FAULT_HIT_FILE").expect("fault hit path"));
    let mode = std::env::var("CREW_FAULT_CALL").expect("fault call mode");
    assert!(matches!(mode.as_str(), "write" | "fsync"));
    let root = temp_root();
    let key = SigningKey::from_bytes(&[7; 32]);
    let (mut broker, mut connection) = bootstrap(&root, &key);
    let team = signed(
        &mut broker,
        &mut connection,
        &key,
        "healthy-team",
        "team.create",
        json!({"name":"healthy","idempotency_key":"healthy-team"}),
    );
    assert!(team.error.is_none(), "healthy operation acknowledged");
    let channel_id = team.result.expect("healthy team result")["channel"]["id"]
        .as_str()
        .expect("channel id")
        .to_owned();
    let healthy = signed(
        &mut broker,
        &mut connection,
        &key,
        "healthy-message",
        "message.post",
        json!({"channel_id":channel_id,"body":"prior ack","idempotency_key":"healthy-message"}),
    );
    assert!(healthy.error.is_none(), "prior message acknowledged");
    let healthy_message = healthy.result.expect("prior message result");
    let healthy_message_id = healthy_message["id"]
        .as_str()
        .expect("prior message id")
        .to_owned();
    let healthy_message_body = healthy_message["body"]
        .as_str()
        .expect("prior message body")
        .to_owned();
    let blobs_before_fault = blob_snapshot(&root);
    fs::write(&marker, b"armed").expect("arm interposer");
    let failed = signed(
        &mut broker,
        &mut connection,
        &key,
        "faulted-message",
        "message.post",
        json!({"channel_id":channel_id,"body":"uncertain ack","idempotency_key":"faulted-message"}),
    );
    assert!(
        failed.result.is_none(),
        "faulted operation was not acknowledged"
    );
    assert!(
        failed.error.is_some(),
        "faulted operation returned no error"
    );
    let replayed = signed(
        &mut broker,
        &mut connection,
        &key,
        "healthy-message-replay",
        "message.post",
        json!({"channel_id":channel_id,"body":healthy_message_body,"idempotency_key":"healthy-message"}),
    );
    assert!(
        replayed.error.is_none(),
        "cached prior ack remains replayable"
    );
    assert_eq!(
        replayed.result.expect("cached prior message result"),
        healthy_message
    );
    let blocked_blob = signed(
        &mut broker,
        &mut connection,
        &key,
        "blocked-blob",
        "blob.begin",
        json!({
            "channel_id":channel_id,
            "size":3,
            "sha256":digest(b"abc"),
            "name":"blocked.bin",
            "media_type":"application/octet-stream",
            "idempotency_key":"blocked-blob"
        }),
    );
    assert_eq!(
        blocked_blob
            .error
            .expect("poisoned broker rejects blob begin")
            .code,
        "storage_failed"
    );
    assert_eq!(
        blob_snapshot(&root),
        blobs_before_fault,
        "post-fault blob begin must not leave an orphan file"
    );
    let hits = fs::read_to_string(&hit_file).expect("read verified interposer hit file");
    assert_eq!(hits.lines().count(), 1, "fault injected exactly once");
    fs::remove_file(&marker).expect("disarm interposer");
    drop(connection);
    drop(broker);

    let public = key_hex(&key);
    let mut reopened = Broker::open(&root, &public).expect("restart after injected fault");
    let mut reopened_connection = Connection::new();
    let history = signed(
        &mut reopened,
        &mut reopened_connection,
        &key,
        "history",
        "messages.history",
        json!({"channel_id":channel_id}),
    );
    assert!(
        history.error.is_none(),
        "history remains readable after restart"
    );
    let history_result = history.result.expect("history result");
    let messages = history_result["messages"]
        .as_array()
        .expect("history messages");
    assert!(messages.iter().any(|message| {
        message.get("id").and_then(Value::as_str) == Some(healthy_message_id.as_str())
            && message.get("body").and_then(Value::as_str) == Some(healthy_message_body.as_str())
    }));
    let _ = fs::remove_dir_all(root);
}
