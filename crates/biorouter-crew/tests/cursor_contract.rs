#![cfg(unix)]

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
use uuid::Uuid;

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

fn create_team(
    broker: &mut Broker,
    connection: &mut Connection,
    key: &SigningKey,
    request_id: &str,
    name: &str,
) -> (String, String) {
    let result = signed(
        broker,
        connection,
        key,
        request_id,
        "team.create",
        json!({"name":name,"idempotency_key":format!("{request_id}-key")}),
    )
    .result
    .unwrap();
    (
        result["team"]["id"].as_str().unwrap().to_owned(),
        result["channel"]["id"].as_str().unwrap().to_owned(),
    )
}

fn invite_team(
    broker: &mut Broker,
    connection: &mut Connection,
    key: &SigningKey,
    team_id: &str,
    principal: &str,
    request_id: &str,
) -> String {
    signed(
        broker,
        connection,
        key,
        request_id,
        "invitation.create",
        json!({
            "kind":"team",
            "target_id":team_id,
            "principal_id":principal,
            "idempotency_key":format!("{request_id}-key")
        }),
    )
    .result
    .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn accept_invitation(
    broker: &mut Broker,
    connection: &mut Connection,
    uid: u32,
    key: &SigningKey,
    invitation: &str,
    request_id: &str,
) {
    let response = signed_as(
        broker,
        connection,
        uid,
        key,
        request_id,
        "invitation.accept",
        json!({"invitation_id":invitation,"idempotency_key":format!("{request_id}-key")}),
    );
    assert!(
        response.error.is_none(),
        "invitation failed: {:?}",
        response.error
    );
}

fn post_message(
    broker: &mut Broker,
    connection: &mut Connection,
    key: &SigningKey,
    channel: &str,
    body: &str,
    request_id: &str,
) -> Value {
    signed(
        broker,
        connection,
        key,
        request_id,
        "message.post",
        json!({"channel_id":channel,"body":body,"idempotency_key":format!("{request_id}-key")}),
    )
    .result
    .unwrap()
}

fn worker_request(
    broker: &mut Broker,
    connection: &mut Connection,
    credential: &str,
    request_id: &str,
    method: &str,
    params: Value,
) -> biorouter_crew::Response {
    broker.handle(
        uid(),
        connection,
        Request {
            version: 1,
            id: request_id.into(),
            method: method.into(),
            params,
            auth: None,
            credential: Some(credential.into()),
        },
    )
}

fn opaque_sequence(value: &Value) -> &str {
    value["sequence"]
        .as_str()
        .expect("wire message sequence is an opaque string")
}

fn assert_wire_messages(result: &Value) {
    for message in result["messages"].as_array().expect("messages") {
        let sequence = message["sequence"]
            .as_str()
            .expect("message sequence must be a string");
        assert!(
            Uuid::parse_str(sequence).is_ok(),
            "sequence was not a UUID: {sequence}"
        );
    }
    if !result["cursor"].is_null() {
        assert!(
            Uuid::parse_str(result["cursor"].as_str().expect("cursor string")).is_ok(),
            "cursor was not an opaque UUID"
        );
    }
}

fn same_error(left: &biorouter_crew::Response, right: &biorouter_crew::Response) {
    let left = left.error.as_ref().expect("left request should be refused");
    let right = right
        .error
        .as_ref()
        .expect("right request should be refused");
    assert_eq!(left.code, right.code);
    assert_eq!(left.message, right.message);
}

#[test]
fn opaque_cursor_hides_global_order_and_survives_restart() {
    let root = temp_root("opaque-cursor-restart");
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
        let (visible_team, visible_channel) = create_team(
            &mut broker,
            &mut host_connection,
            &host_key,
            "visible-team",
            "visible",
        );
        let invitation = invite_team(
            &mut broker,
            &mut host_connection,
            &host_key,
            &visible_team,
            &guest_principal,
            "visible-invite",
        );
        accept_invitation(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            &invitation,
            "visible-accept",
        );
        let (_, hidden_channel) = create_team(
            &mut broker,
            &mut host_connection,
            &host_key,
            "hidden-team",
            "hidden",
        );
        let first = post_message(
            &mut broker,
            &mut host_connection,
            &host_key,
            &visible_channel,
            "visible-first",
            "visible-first",
        );
        let hidden = post_message(
            &mut broker,
            &mut host_connection,
            &host_key,
            &hidden_channel,
            "hidden-between",
            "hidden-between",
        );
        let second = post_message(
            &mut broker,
            &mut host_connection,
            &host_key,
            &visible_channel,
            "visible-second",
            "visible-second",
        );
        let first_sequence = opaque_sequence(&first).to_owned();
        let hidden_sequence = opaque_sequence(&hidden).to_owned();
        let second_sequence = opaque_sequence(&second).to_owned();
        assert_ne!(first_sequence, hidden_sequence);
        assert_ne!(hidden_sequence, second_sequence);

        let page = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "page-one",
            "messages.history",
            json!({"channel_id":visible_channel,"limit":1}),
        );
        assert!(page.error.is_none(), "history failed: {:?}", page.error);
        let page = page.result.unwrap();
        assert_wire_messages(&page);
        assert_eq!(page["messages"].as_array().unwrap().len(), 1);
        assert_eq!(page["messages"][0]["body"], "visible-first");
        assert_eq!(page["cursor"], first_sequence);
        assert_ne!(page["cursor"], 1);
        assert!(!serde_json::to_string(&page)
            .unwrap()
            .contains(&hidden_sequence));

        let next = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "page-two",
            "messages.history",
            json!({"channel_id":visible_channel,"after":first_sequence,"limit":1}),
        );
        assert!(next.error.is_none(), "next page failed: {:?}", next.error);
        let next = next.result.unwrap();
        assert_wire_messages(&next);
        assert_eq!(next["messages"][0]["body"], "visible-second");
        assert_eq!(next["cursor"], second_sequence);

        let empty = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "empty-page",
            "messages.history",
            json!({"channel_id":visible_channel,"after":second_sequence,"limit":1}),
        );
        assert!(
            empty.error.is_none(),
            "empty page failed: {:?}",
            empty.error
        );
        let empty = empty.result.unwrap();
        assert!(empty["messages"].as_array().unwrap().is_empty());
        assert_eq!(empty["cursor"], second_sequence);

        drop(guest_connection);
        drop(host_connection);
        drop(broker);
        let public = key_hex(&host_key);
        let mut reopened = Broker::open(&root, &public).unwrap();
        let mut reopened_guest = Connection::new();
        let resumed = signed_as(
            &mut reopened,
            &mut reopened_guest,
            guest_uid,
            &guest_key,
            "restart-page",
            "messages.history",
            json!({"channel_id":visible_channel,"after":first_sequence,"limit":1}),
        );
        assert!(
            resumed.error.is_none(),
            "restart page failed: {:?}",
            resumed.error
        );
        let resumed = resumed.result.unwrap();
        assert_wire_messages(&resumed);
        assert_eq!(resumed["messages"][0]["body"], "visible-second");
        assert_eq!(resumed["cursor"], second_sequence);
    });
}

#[test]
fn opaque_cursor_rejects_numeric_and_unavailable_anchors_uniformly() {
    let root = temp_root("opaque-cursor-refusal");
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
        let (team_a, channel_a) = create_team(
            &mut broker,
            &mut host_connection,
            &host_key,
            "anchor-a",
            "anchor-a",
        );
        let (team_b, channel_b) = create_team(
            &mut broker,
            &mut host_connection,
            &host_key,
            "anchor-b",
            "anchor-b",
        );
        for (team, suffix) in [(&team_a, "a"), (&team_b, "b")] {
            let invitation = invite_team(
                &mut broker,
                &mut host_connection,
                &host_key,
                team,
                &guest_principal,
                &format!("invite-{suffix}"),
            );
            accept_invitation(
                &mut broker,
                &mut guest_connection,
                guest_uid,
                &guest_key,
                &invitation,
                &format!("accept-{suffix}"),
            );
        }
        let visible = post_message(
            &mut broker,
            &mut host_connection,
            &host_key,
            &channel_a,
            "anchor-visible",
            "anchor-visible",
        );
        let hidden_room = post_message(
            &mut broker,
            &mut host_connection,
            &host_key,
            &channel_b,
            "anchor-other-room",
            "anchor-other-room",
        );
        let run = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "source-hidden-run",
            "run.create",
            json!({"channel_id":channel_a,"source_channels":[channel_a,channel_b],"provider_policy_id":"private","personal_mode":"private","public_provider":false,"expires_in":60,"idempotency_key":"source-hidden-run-key"}),
        )
        .result
        .unwrap();
        let credential = run["credential"].as_str().unwrap().to_owned();
        let source_hidden = worker_request(
            &mut broker,
            &mut host_connection,
            &credential,
            "source-hidden-post",
            "run.project",
            json!({"body":"source-hidden","status":"progress","idempotency_key":"source-hidden-post-key"}),
        );
        assert!(
            source_hidden.error.is_none(),
            "projection failed: {:?}",
            source_hidden.error
        );
        let source_hidden_sequence =
            opaque_sequence(source_hidden.result.as_ref().unwrap()).to_owned();
        let hidden_room_sequence = opaque_sequence(&hidden_room).to_owned();
        let visible_sequence = opaque_sequence(&visible).to_owned();

        let unknown = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "unknown-anchor",
            "messages.history",
            json!({"channel_id":channel_a,"after":Uuid::new_v4().to_string()}),
        );
        let cross_room = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "cross-room-anchor",
            "messages.history",
            json!({"channel_id":channel_a,"after":hidden_room_sequence}),
        );
        same_error(&unknown, &cross_room);

        let numeric = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "numeric-anchor",
            "messages.history",
            json!({"channel_id":channel_a,"after":1}),
        );
        assert_eq!(numeric.error.unwrap().code, "invalid_params");
        let numeric_before = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "numeric-before",
            "messages.history",
            json!({"channel_id":channel_a,"before":1}),
        );
        assert_eq!(numeric_before.error.unwrap().code, "invalid_params");
        let numeric_read = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "numeric-read",
            "channel.read",
            json!({"channel_id":channel_a,"sequence":1}),
        );
        assert_eq!(numeric_read.error.unwrap().code, "invalid_params");

        let revoke = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "revoke-source-membership",
            "membership.revoke",
            json!({"channel_id":channel_b,"principal_id":guest_principal,"idempotency_key":"revoke-source-membership-key"}),
        );
        assert!(
            revoke.error.is_none(),
            "source revoke failed: {:?}",
            revoke.error
        );
        let revoked_anchor = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "revoked-anchor",
            "messages.history",
            json!({"channel_id":channel_a,"after":source_hidden_sequence}),
        );
        same_error(&unknown, &revoked_anchor);
        assert_ne!(visible_sequence, hidden_room_sequence);
    });
}

#[test]
fn opaque_sequence_projects_all_message_surfaces_and_cached_results() {
    let root = temp_root("opaque-cursor-projections");
    with_cleanup(&root, || {
        let (mut broker, mut connection, key) = bootstrap(&root);
        let (_, channel) = create_team(
            &mut broker,
            &mut connection,
            &key,
            "projections",
            "projections",
        );
        let posted = post_message(
            &mut broker,
            &mut connection,
            &key,
            &channel,
            "cached-human",
            "cached-human",
        );
        let posted_sequence = opaque_sequence(&posted).to_owned();
        assert!(Uuid::parse_str(&posted_sequence).is_ok());
        let replay = signed(
            &mut broker,
            &mut connection,
            &key,
            "cached-human-replay",
            "message.post",
            json!({"channel_id":channel,"body":"cached-human","idempotency_key":"cached-human-key"}),
        );
        assert!(
            replay.error.is_none(),
            "cached post failed: {:?}",
            replay.error
        );
        assert_eq!(replay.result.unwrap(), posted);

        let search = signed(
            &mut broker,
            &mut connection,
            &key,
            "search",
            "messages.search",
            json!({"channel_id":channel,"query":"cached-human"}),
        );
        assert!(search.error.is_none(), "search failed: {:?}", search.error);
        let search = search.result.unwrap();
        assert_wire_messages(&search);
        assert_eq!(search["cursor"], posted_sequence);

        let run = signed(
            &mut broker,
            &mut connection,
            &key,
            "project-run",
            "run.create",
            json!({"channel_id":channel,"source_channels":[channel],"provider_policy_id":"private","personal_mode":"private","public_provider":false,"expires_in":60,"idempotency_key":"project-run-key"}),
        )
        .result
        .unwrap();
        let credential = run["credential"].as_str().unwrap().to_owned();
        let manifest = worker_request(
            &mut broker,
            &mut connection,
            &credential,
            "manifest",
            "context.manifest",
            json!({}),
        );
        assert!(
            manifest.error.is_none(),
            "manifest failed: {:?}",
            manifest.error
        );
        let manifest = manifest.result.unwrap();
        assert_wire_messages(&json!({"messages":manifest["messages"],"cursor":Value::Null}));

        let project = worker_request(
            &mut broker,
            &mut connection,
            &credential,
            "project",
            "run.project",
            json!({"body":"cached-project","status":"completed","idempotency_key":"cached-project-key"}),
        );
        assert!(
            project.error.is_none(),
            "project failed: {:?}",
            project.error
        );
        let project_result = project.result.unwrap();
        assert!(Uuid::parse_str(opaque_sequence(&project_result)).is_ok());
        let project_replay = worker_request(
            &mut broker,
            &mut connection,
            &credential,
            "project-replay",
            "run.project",
            json!({"body":"cached-project","status":"completed","idempotency_key":"cached-project-key"}),
        );
        assert!(
            project_replay.error.is_none(),
            "project replay failed: {:?}",
            project_replay.error
        );
        assert_eq!(project_replay.result.unwrap(), project_result);
    });
}

#[test]
fn read_watermarks_are_opaque_monotonic_and_acl_safe_after_restart() {
    let root = temp_root("opaque-read-watermark");
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
        let (team_a, channel_a) = create_team(
            &mut broker,
            &mut host_connection,
            &host_key,
            "watermark-a",
            "watermark-a",
        );
        let (team_b, channel_b) = create_team(
            &mut broker,
            &mut host_connection,
            &host_key,
            "watermark-b",
            "watermark-b",
        );
        for (team, suffix) in [(&team_a, "a"), (&team_b, "b")] {
            let invitation = invite_team(
                &mut broker,
                &mut host_connection,
                &host_key,
                team,
                &guest_principal,
                &format!("watermark-invite-{suffix}"),
            );
            accept_invitation(
                &mut broker,
                &mut guest_connection,
                guest_uid,
                &guest_key,
                &invitation,
                &format!("watermark-accept-{suffix}"),
            );
        }
        let first = post_message(
            &mut broker,
            &mut host_connection,
            &host_key,
            &channel_a,
            "watermark-first",
            "watermark-first",
        );
        let second = post_message(
            &mut broker,
            &mut host_connection,
            &host_key,
            &channel_a,
            "watermark-second",
            "watermark-second",
        );
        let first_sequence = opaque_sequence(&first).to_owned();
        let second_sequence = opaque_sequence(&second).to_owned();
        let initial = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "initial-snapshot",
            "workspace.snapshot",
            json!({}),
        );
        assert!(
            initial.error.is_none(),
            "initial snapshot failed: {:?}",
            initial.error
        );
        assert!(initial.result.unwrap()["read_positions"][&channel_a].is_null());

        let read_second = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "read-second",
            "channel.read",
            json!({"channel_id":channel_a,"sequence":second_sequence,"idempotency_key":"read-second-key"}),
        );
        assert!(
            read_second.error.is_none(),
            "read failed: {:?}",
            read_second.error
        );
        assert_eq!(read_second.result.unwrap()["sequence"], second_sequence);
        let read_first = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "read-first-after-second",
            "channel.read",
            json!({"channel_id":channel_a,"sequence":first_sequence,"idempotency_key":"read-first-key"}),
        );
        assert!(
            read_first.error.is_none(),
            "monotonic read failed: {:?}",
            read_first.error
        );
        assert_eq!(read_first.result.unwrap()["sequence"], second_sequence);

        let run = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "watermark-hidden-run",
            "run.create",
            json!({"channel_id":channel_a,"source_channels":[channel_a,channel_b],"provider_policy_id":"private","personal_mode":"private","public_provider":false,"expires_in":60,"idempotency_key":"watermark-hidden-run-key"}),
        )
        .result
        .unwrap();
        let credential = run["credential"].as_str().unwrap().to_owned();
        let hidden = worker_request(
            &mut broker,
            &mut host_connection,
            &credential,
            "watermark-hidden",
            "run.project",
            json!({"body":"watermark-source-hidden","status":"progress","idempotency_key":"watermark-hidden-key"}),
        );
        assert!(
            hidden.error.is_none(),
            "hidden projection failed: {:?}",
            hidden.error
        );
        let hidden_sequence = opaque_sequence(hidden.result.as_ref().unwrap()).to_owned();
        let read_hidden = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "read-hidden-before-revoke",
            "channel.read",
            json!({"channel_id":channel_a,"sequence":hidden_sequence,"idempotency_key":"read-hidden-key"}),
        );
        assert!(
            read_hidden.error.is_none(),
            "hidden read failed: {:?}",
            read_hidden.error
        );
        let revoke = signed(
            &mut broker,
            &mut host_connection,
            &host_key,
            "watermark-revoke-source",
            "membership.revoke",
            json!({"channel_id":channel_b,"principal_id":guest_principal,"idempotency_key":"watermark-revoke-source-key"}),
        );
        assert!(
            revoke.error.is_none(),
            "source revoke failed: {:?}",
            revoke.error
        );
        let after_acl_loss = signed_as(
            &mut broker,
            &mut guest_connection,
            guest_uid,
            &guest_key,
            "snapshot-after-acl-loss",
            "workspace.snapshot",
            json!({}),
        );
        assert!(
            after_acl_loss.error.is_none(),
            "snapshot after ACL loss failed: {:?}",
            after_acl_loss.error
        );
        let after_acl_loss = after_acl_loss.result.unwrap();
        let position = &after_acl_loss["read_positions"][&channel_a];
        assert_eq!(position, &second_sequence);
        let protected = after_acl_loss["protected_channel_ids"]
            .as_array()
            .expect("snapshot protected channel ids");
        assert!(
            protected
                .iter()
                .any(|id| id.as_str() == Some(channel_a.as_str())),
            "retained restricted projection must keep its channel protected"
        );
        assert!(
            protected
                .iter()
                .all(|id| id.as_str() != Some(channel_b.as_str())),
            "snapshot must not disclose the revoked source channel"
        );

        drop(guest_connection);
        drop(host_connection);
        drop(broker);
        let public = key_hex(&host_key);
        let mut reopened = Broker::open(&root, &public).unwrap();
        let mut reopened_guest = Connection::new();
        let snapshot = signed_as(
            &mut reopened,
            &mut reopened_guest,
            guest_uid,
            &guest_key,
            "restart-snapshot",
            "workspace.snapshot",
            json!({}),
        );
        assert!(
            snapshot.error.is_none(),
            "restart snapshot failed: {:?}",
            snapshot.error
        );
        let snapshot = snapshot.result.unwrap();
        let position = &snapshot["read_positions"][&channel_a];
        assert_eq!(position, &second_sequence);
        let protected = snapshot["protected_channel_ids"]
            .as_array()
            .expect("restart snapshot protected channel ids");
        assert!(protected
            .iter()
            .any(|id| id.as_str() == Some(channel_a.as_str())));
        assert!(protected
            .iter()
            .all(|id| id.as_str() != Some(channel_b.as_str())));
        assert!(!serde_json::to_string(&snapshot)
            .unwrap()
            .contains(&hidden_sequence));
    });
}
