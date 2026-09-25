//! `GET /crew/connections/{id}/grants` and `POST /crew/connections/{id}/sessions/{session}/revoke`
//! on a daemon that holds a user-action key: the person's list of chats and tasks holding Crew
//! access, and the control that takes it away (ui-redesign-spec "Revoke", RV-D1 to RV-D3).
//!
//! What is pinned here, against the real routes behind the real `check_token`:
//!
//! - both routes answer only a request that proves a person sent it, and a refused revoke
//!   changes nothing;
//! - a revoke with no grant is a 404 and one through the wrong connection a 409, each with its
//!   own code, so the interface never has to read an error sentence;
//! - a revoke the workspace cannot confirm (here: the connection is not connected, as after a
//!   dropped SSH bridge) still stops the grant on this device and saves that, then answers 503
//!   `crew_revocation_unconfirmed`. The grant used to stay fully active in that case (RV-D1);
//! - a task's session is stopped through the task cancel path, so the ledger records the
//!   outcome rather than a stale `running` (RV-D3);
//! - each grant row says whether it is a chat or a task, and names the chat (RV-D2).
//!
//! ⚠ **Its own binary**, because the installed digest is a process-global `OnceLock`. Offline:
//! nothing here opens a socket, so CI runs it in the loopback-only integration step. The
//! Crew device credential is read from a file, never the OS keychain: the constructor below
//! selects Crew's development credential backend before `main`, and the seeded connection has
//! no credential at all, which is one of the ways a workspace fails to confirm.

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use biorouter::config::paths::Paths;
use biorouter::session::session_manager::SessionType;
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use tokio::sync::OnceCell;
use tower::ServiceExt;

/// The server secret this binary's "daemon" was launched with.
const TEST_SECRET: &str = "crew-revoke-gate-secret";
/// The user-action key the desktop app would hold; the daemon keeps only its digest.
const USER_KEY: &str = "crew-revoke-gate-user-action-key";

/// The saved connection every grant here belongs to. It is never connected.
const CONNECTION: &str = "conn-methods-lab";
/// A chat grant only the refusal tests touch, so it must still be active after each of them.
/// No conversation exists under this ID, so its row has no name.
const GUARDED_SESSION: &str = "crew-revoke-guarded-session";
const GUARDED_RUN: &str = "run-guarded";
/// A task started from Crew: the run ledger holds this session and run.
const TASK_SESSION: &str = "crew-revoke-task-session";
const TASK_RUN: &str = "run-task";
/// The chat grant's run; the chat itself is a real conversation, created by the fixture.
const CHAT_RUN: &str = "run-chat";
const CHAT_TITLE: &str = "Plot review";
const EXPIRES_AT: u64 = 1_790_000_000;

/// Crew's development credential backend: device keys are files under the Crew root, so a
/// revoke reads a file rather than asking the OS keychain. Set before `main`, while nothing
/// else is running, because Crew reads both variables at the moment it needs a key.
#[ctor::ctor]
fn select_the_file_credential_backend() {
    std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
    if std::env::var_os("BIOROUTER_DEV_PROFILE_ROOT").is_none() {
        std::env::set_var(
            "BIOROUTER_DEV_PROFILE_ROOT",
            std::env::temp_dir().join(format!(
                "biorouter-crew-revoke-profile-{}",
                std::process::id()
            )),
        );
    }
}

fn install_the_desktop_key() {
    let digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(USER_KEY.as_bytes()).into();
    biorouter_server::auth::install_user_action_digest(Some(digest));
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("X-User-Action", USER_KEY.parse().unwrap());
    assert!(
        biorouter_server::auth::is_user_action(&headers),
        "the digest this binary installs did not take, so every positive arm below would be \
         measuring a refusal"
    );
}

/// What a request carries besides the secret.
#[derive(Clone, Copy, Debug)]
enum Credential {
    /// Nothing: indistinguishable from a model holding the daemon secret.
    SecretOnly,
    /// A key that is not the one this daemon holds the digest of.
    WrongKey,
    /// The desktop app's proof.
    Proof,
}

/// The chat grant's session ID, once the fixture has seeded everything.
static FIXTURE: OnceCell<String> = OnceCell::const_new();

/// Seed, once per process and before anything reads them, a saved connection holding three
/// grants and a run ledger holding the task. Returns the chat's session ID.
async fn fixture(state: &Arc<AppState>) -> String {
    FIXTURE
        .get_or_init(|| async {
            assert_eq!(
                std::env::var("BIOROUTER_DISABLE_KEYRING").as_deref(),
                Ok("true"),
                "Crew would read device keys from the OS keychain in this binary"
            );
            let chat = state
                .session_manager()
                .create_session(
                    std::env::temp_dir().join("crew_revoke_routes"),
                    CHAT_TITLE.to_string(),
                    SessionType::User,
                )
                .await
                .unwrap()
                .id;
            let crew_root = Paths::config_dir().join("crew");
            std::fs::create_dir_all(&crew_root).unwrap();
            assert!(
                !crew_root.join("connections.json").exists(),
                "something saved Crew connections before the fixture"
            );
            let scope = |run: &str, expires_at: Option<u64>| {
                let mut scope = json!({
                    "connection_id": CONNECTION,
                    "run_id": run,
                    "channel_id": "chan-methods",
                    "source_channels": ["chan-raw-data"],
                    "epoch": 1,
                    "provider_binding": "versa_azure",
                    "public_provider": false,
                    "expired": false,
                });
                if let Some(at) = expires_at {
                    scope["expires_at"] = json!(at);
                }
                scope
            };
            let registry = json!({
                "connections": [{
                    "id": CONNECTION,
                    "name": "Methods lab",
                    "ssh_target": "crew@crew.invalid",
                    "port": null,
                    "identity_file": null,
                    "proxy_jump": null,
                    "socket_path": "/tmp/crew-revoke-routes.sock",
                    "owner_uid": 501,
                    "workspace_id": "workspace-methods",
                    "workspace_public_key": "00".repeat(32),
                    "cluster_connection_id": "cluster-methods",
                    "mode": "private",
                    "policy_epoch": 1,
                    "status": "disconnected",
                    "last_error": null,
                    "device_id": "11".repeat(32),
                    "public_key": "22".repeat(32),
                }],
                "scopes": {
                    GUARDED_SESSION: scope(GUARDED_RUN, None),
                    TASK_SESSION: scope(TASK_RUN, Some(EXPIRES_AT)),
                    chat.as_str(): scope(CHAT_RUN, Some(EXPIRES_AT)),
                },
            });
            std::fs::write(
                crew_root.join("connections.json"),
                serde_json::to_vec(&registry).unwrap(),
            )
            .unwrap();
            let ledger = Paths::state_dir().join("crew").join("runs.json");
            std::fs::create_dir_all(ledger.parent().unwrap()).unwrap();
            assert!(
                !ledger.exists(),
                "something wrote the Crew run ledger before the fixture"
            );
            std::fs::write(
                &ledger,
                serde_json::to_vec(&json!({
                    "runs": [{
                        "run_id": TASK_RUN,
                        "connection_id": CONNECTION,
                        "channel_id": "chan-methods",
                        "session_id": TASK_SESSION,
                        "status": "running",
                        "error": null,
                    }],
                    "requests": {},
                }))
                .unwrap(),
            )
            .unwrap();
            chat
        })
        .await
        .clone()
}

/// The Crew routes behind the SAME `check_token` middleware the daemon installs.
fn app(state: Arc<AppState>) -> axum::Router {
    biorouter_server::routes::crew_profile::routes(state.clone())
        .merge(biorouter_server::routes::crew::routes(state))
        .layer(axum::middleware::from_fn_with_state(
            TEST_SECRET.to_string(),
            biorouter_server::auth::check_token,
        ))
}

async fn send(
    state: &Arc<AppState>,
    method: &str,
    uri: &str,
    credential: Credential,
) -> (StatusCode, Value) {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Secret-Key", TEST_SECRET);
    let builder = match credential {
        Credential::SecretOnly => builder,
        Credential::WrongKey => builder.header("X-User-Action", "not-the-desktop-key"),
        Credential::Proof => builder.header("X-User-Action", USER_KEY),
    };
    let response = app(state.clone())
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
    (status, json)
}

async fn revoke(
    state: &Arc<AppState>,
    connection: &str,
    session: &str,
    credential: Credential,
) -> (StatusCode, Value) {
    send(
        state,
        "POST",
        &format!("/crew/connections/{connection}/sessions/{session}/revoke"),
        credential,
    )
    .await
}

/// The person's grant list, keyed by session.
async fn grants(state: &Arc<AppState>) -> serde_json::Map<String, Value> {
    let (status, body) = send(
        state,
        "GET",
        &format!("/crew/connections/{CONNECTION}/grants"),
        Credential::Proof,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the person's grant list: {body}");
    body["grants"]
        .as_array()
        .unwrap_or_else(|| panic!("no grants array: {body}"))
        .iter()
        .map(|row| (row["session_id"].as_str().unwrap().to_owned(), row.clone()))
        .collect()
}

/// The grant as this device saved it, read from disk rather than through the manager, so an
/// expiry that only landed in memory does not count.
fn saved_expired(session: &str) -> bool {
    let saved: Value = serde_json::from_slice(
        &std::fs::read(Paths::config_dir().join("crew").join("connections.json")).unwrap(),
    )
    .unwrap();
    saved["scopes"][session]["expired"]
        .as_bool()
        .unwrap_or_else(|| panic!("no saved grant for {session}: {saved}"))
}

async fn setup() -> (Arc<AppState>, String) {
    install_the_desktop_key();
    let state = AppState::new().await.unwrap();
    let chat = fixture(&state).await;
    (state, chat)
}

/// Neither the list nor the revoke answers a caller that did not prove a person sent it,
/// and the refused revoke leaves the grant exactly as it was, in memory and on disk.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn grants_and_revoke_refuse_a_caller_without_the_persons_proof() {
    let (state, _) = setup().await;
    for credential in [Credential::SecretOnly, Credential::WrongKey] {
        let (status, body) = send(
            &state,
            "GET",
            &format!("/crew/connections/{CONNECTION}/grants"),
            credential,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{credential:?} listed: {body}"
        );
        assert_eq!(body["code"], "crew_user_action_required", "{body}");
        assert!(body.get("grants").is_none(), "a refusal carried grants");

        let (status, body) = revoke(&state, CONNECTION, GUARDED_SESSION, credential).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{credential:?} revoked: {body}"
        );
        assert_eq!(body["code"], "crew_user_action_required", "{body}");
    }
    let listed = grants(&state).await;
    assert_eq!(
        listed[GUARDED_SESSION]["expired"],
        json!(false),
        "a refused revoke stopped the grant"
    );
    assert!(
        !saved_expired(GUARDED_SESSION),
        "a refused revoke was saved"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn revoking_a_session_without_a_grant_is_not_found() {
    let (state, _) = setup().await;
    let (status, body) = revoke(
        &state,
        CONNECTION,
        "crew-revoke-never-granted",
        Credential::Proof,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "crew_grant_not_found", "{body}");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn revoking_through_another_connection_is_a_conflict_that_changes_nothing() {
    let (state, _) = setup().await;
    let (status, body) = revoke(
        &state,
        "conn-some-other-lab",
        GUARDED_SESSION,
        Credential::Proof,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "crew_grant_other_connection", "{body}");
    assert_eq!(
        grants(&state).await[GUARDED_SESSION]["expired"],
        json!(false)
    );
    assert!(!saved_expired(GUARDED_SESSION));
}

/// RV-D1. The workspace cannot be reached, so it cannot confirm; the grant is stopped and
/// saved on this device anyway, and the answer says exactly that rather than success. A
/// retry asks again and still stops nothing twice.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_revoke_the_workspace_cannot_confirm_stops_the_grant_here_and_says_so() {
    let (state, chat) = setup().await;
    for attempt in ["first", "retry"] {
        let (status, body) = revoke(&state, CONNECTION, &chat, Credential::Proof).await;
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{attempt} revoke of a disconnected connection's grant: {body}"
        );
        assert_eq!(body["code"], "crew_revocation_unconfirmed", "{body}");
        assert_eq!(
            body["error"],
            "Stopped on this device. The workspace hasn't confirmed yet; Biorouter confirms it by itself when the connection is back.",
        );
        assert_eq!(body["session_id"], json!(chat));
        assert_eq!(body["run_id"], CHAT_RUN);
        assert_eq!(body["stopped_on_this_device"], json!(true));
        assert!(
            body["detail"]
                .as_str()
                .is_some_and(|detail| !detail.is_empty()),
            "the workspace's reason is missing: {body}"
        );
        assert_ne!(
            body["revoked"],
            json!(true),
            "an unconfirmed revoke claimed success"
        );
    }
    let listed = grants(&state).await;
    assert_eq!(
        listed[chat.as_str()]["expired"],
        json!(true),
        "the grant is still active on this device after an unconfirmed revoke"
    );
    // F3: the list says the workspace has not confirmed, rather than reading as revoked.
    assert_eq!(listed[chat.as_str()]["revocation"], "unconfirmed");
    assert_eq!(
        listed[chat.as_str()]["remote_revocation_confirmed"],
        json!(false)
    );
    assert!(listed[GUARDED_SESSION]["revocation"].is_null());
    assert!(listed[GUARDED_SESSION]["remote_revocation_confirmed"].is_null());
    assert!(saved_expired(&chat), "the local stop was not saved");
    assert_eq!(
        listed[GUARDED_SESSION]["expired"],
        json!(false),
        "revoking one chat stopped another"
    );
}

/// RV-D2: every row says what kind of grant it is and which chat holds it, and when the
/// workspace ends it, so the list reads without a network call.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn each_grant_names_its_chat_and_says_whether_it_is_a_task() {
    let (state, chat) = setup().await;
    let listed = grants(&state).await;
    assert_eq!(listed.len(), 3, "{listed:?}");

    let chat_row = &listed[chat.as_str()];
    assert_eq!(chat_row["kind"], "chat", "{chat_row}");
    assert_eq!(chat_row["session_name"], CHAT_TITLE, "{chat_row}");
    assert_eq!(chat_row["expires_at"], json!(EXPIRES_AT), "{chat_row}");
    assert_eq!(chat_row["run_id"], CHAT_RUN);
    assert_eq!(chat_row["channel_id"], "chan-methods");
    assert_eq!(chat_row["source_channels"], json!(["chan-raw-data"]));

    let task_row = &listed[TASK_SESSION];
    assert_eq!(task_row["kind"], "task", "{task_row}");

    let guarded_row = &listed[GUARDED_SESSION];
    assert_eq!(guarded_row["kind"], "chat", "{guarded_row}");
    assert_eq!(
        guarded_row["session_name"],
        Value::Null,
        "a grant whose conversation is gone was given a name: {guarded_row}"
    );
    assert_eq!(guarded_row["expires_at"], Value::Null, "{guarded_row}");
}

/// RV-D3: revoking a task's session goes through the task cancel path, so its ledger entry
/// records the outcome instead of staying `running` (here `interrupted`, as a restart leaves
/// it) while the grant is gone.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn revoking_a_task_session_records_the_outcome_in_the_task_ledger() {
    let (state, _) = setup().await;
    let (status, body) = revoke(&state, CONNECTION, TASK_SESSION, Credential::Proof).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["code"], "crew_revocation_unconfirmed", "{body}");
    assert_eq!(body["task_status"], "cancellation_unconfirmed", "{body}");
    assert_eq!(body["run_id"], TASK_RUN);

    let (status, runs) = send(
        &state,
        "GET",
        &format!("/crew/connections/{CONNECTION}/runs"),
        Credential::Proof,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{runs}");
    let task = runs["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|run| run["run_id"] == TASK_RUN)
        .unwrap_or_else(|| panic!("the task left the ledger: {runs}"));
    assert_eq!(task["status"], "cancellation_unconfirmed", "{task}");

    let listed = grants(&state).await;
    assert_eq!(listed[TASK_SESSION]["expired"], json!(true));
    assert_eq!(listed[TASK_SESSION]["kind"], "task");
    assert!(saved_expired(TASK_SESSION));
}
