//! `POST /reset` and `GET /reset/preview` on a daemon that HOLDS a user-action
//! key — the desktop application's — answer only a request that proves the
//! person at the keyboard sent it.
//!
//! **Measured before any of this was written (2026-09-14, a sandboxed
//! `biorouterd agent` built from `origin/main` at `1038a113`, digest piped on
//! stdin).** A private chat was imported; `GET /sessions/{id}` holding only
//! `X-Secret-Key` answered **403**, as the reach gate says it must. Then, holding
//! the same secret and nothing else:
//!
//! ```text
//! GET  /reset/preview                          → 200 {"counts":{…,"conversations":1}}
//! POST /reset {"categories":["history"]}       → 200 {"reset":["history"],"removed":{…,"conversations":1}}
//! sqlite: select count(*) from sessions        → 0
//! ```
//!
//! A private knowledge base went the same way through `{"categories":
//! ["knowledge"]}` while its own `GET /knowledge/bases/{id}` refused the caller.
//! The secret is recoverable from inside a public chat's shell (AR-11), so that
//! was a public model deleting every private chat on the machine — the
//! machine-wide twin of QA's F0 (`DELETE /sessions/{id}`), which was gated and
//! this was not.
//!
//! ⚠ **Its own binary**, because the installed digest is a process-global
//! `OnceLock`: this one installs a key, `reset_no_user_key.rs` must not. And
//! because a successful History reset here really does empty this binary's
//! (sandboxed) session store, every test is `#[serial]`.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can open the developer's real `sessions.db` —
// which matters more than usual in a file that resets History.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use biorouter::conversation::message::Message;
use biorouter::model::ModelConfig;
use biorouter::privacy::SessionClassification;
use biorouter::session::session_manager::SessionType;
use biorouter_server::routes::reset::{RESET_NEEDS_USER, RESET_NO_USER_KEY};
use biorouter_server::state::AppState;
use serial_test::serial;
use tower::ServiceExt;

/// The server secret this binary's "daemon" was launched with.
const TEST_SECRET: &str = "reset-gate-secret";

/// The user-action key the desktop app would hold. The daemon keeps only its
/// digest, exactly as `commands::agent` installs one read off stdin.
const USER_KEY: &str = "reset-gate-user-action-key";

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
#[derive(Clone, Copy)]
enum Credential {
    /// Nothing — the caller AR-11 establishes is indistinguishable from a model.
    SecretOnly,
    /// A key that is not the one this daemon holds the digest of.
    WrongKey,
    /// A stated PRIVATE capability, which the reach gate honours for reading a
    /// chat and this gate must not honour for destroying every chat.
    PrivateCapability,
    /// The desktop app's proof.
    Proof,
}

/// The reset routes behind the SAME `check_token` middleware
/// `commands::agent::run` installs, so this is the request the real daemon
/// answers and not a bare router's.
fn app(state: Arc<AppState>) -> axum::Router {
    biorouter_server::routes::reset::routes(state.clone())
        .merge(biorouter_server::routes::session::routes(state))
        .layer(axum::middleware::from_fn_with_state(
            TEST_SECRET.to_string(),
            biorouter_server::auth::check_token,
        ))
}

async fn send(
    state: &Arc<AppState>,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
    credential: Credential,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Secret-Key", TEST_SECRET);
    builder = match credential {
        Credential::SecretOnly => builder,
        Credential::WrongKey => builder.header("X-User-Action", "not-the-desktop-key"),
        Credential::PrivateCapability => builder.header("X-Caller-Provider", "versa_azure"),
        Credential::Proof => builder.header("X-User-Action", USER_KEY),
    };
    let request = match body {
        Some(json) => builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&json).unwrap())),
        None => builder.body(Body::empty()),
    }
    .unwrap();
    let response = app(state.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(&bytes).into()));
    (status, json)
}

async fn reset(
    state: &Arc<AppState>,
    categories: &[&str],
    credential: Credential,
) -> (StatusCode, serde_json::Value) {
    send(
        state,
        "POST",
        "/reset",
        Some(serde_json::json!({ "categories": categories })),
        credential,
    )
    .await
}

/// A private chat with one message, raised the way a real chat gets there
/// rather than by writing the column.
async fn seed_private_chat(state: &Arc<AppState>) -> String {
    let manager = state.session_manager();
    let session = manager
        .create_session(
            std::env::temp_dir().join("reset_gate"),
            "Reset gate fixture".to_string(),
            SessionType::User,
        )
        .await
        .unwrap();
    manager
        .add_message(&session.id, &Message::user().with_text("patient MRN 12345"))
        .await
        .unwrap();
    manager
        .update(&session.id)
        .provider_name("versa_azure")
        .model_config(ModelConfig::new("gpt-4o").unwrap())
        .raise_privacy(SessionClassification::Private, "turn:versa_azure")
        .apply()
        .await
        .unwrap();
    session.id
}

async fn still_private(state: &Arc<AppState>, id: &str) -> bool {
    state
        .session_manager()
        .get_session(id, false)
        .await
        .map(|session| session.privacy_tier == SessionClassification::Private)
        .unwrap_or(false)
}

fn message_of(body: &serde_json::Value) -> &str {
    body.get("message")
        .and_then(|message| message.as_str())
        .unwrap_or_else(|| panic!("the refusal is not in the route's error envelope: {body}"))
}

/// The measured defect, as its own regression test.
///
/// ⚠ **Fails with the gate removed** — the handlers as they are on `origin/main`: the History reset
/// answers 200 with `"conversations"` ≥ 1 and the private chat is gone, so the first assertion
/// fails on the status.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_caller_holding_only_the_secret_cannot_reset_history() {
    install_the_desktop_key();
    let state = AppState::new().await.unwrap();
    let id = seed_private_chat(&state).await;

    for credential in [
        Credential::SecretOnly,
        Credential::WrongKey,
        Credential::PrivateCapability,
    ] {
        let (status, body) = reset(&state, &["history"], credential).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a caller that did not prove it is the user reset History: {body}"
        );
        assert_eq!(message_of(&body), RESET_NEEDS_USER);
        assert!(
            still_private(&state, &id).await,
            "the refused reset deleted the private chat anyway"
        );
    }

    // The contrast that makes the rule visible: the stated private capability
    // that was just refused a reset DOES read the chat, because reaching a chat
    // and destroying every chat are different questions.
    let (read, _) = send(
        &state,
        "GET",
        &format!("/sessions/{id}"),
        None,
        Credential::PrivateCapability,
    )
    .await;
    assert_eq!(
        read,
        StatusCode::OK,
        "the reach gate refused a stated private capability, so the refusal above says nothing \
         about the difference between the two rules"
    );

    state.session_manager().delete_session(&id).await.unwrap();
}

/// The same for every other category the reset deletes — knowledge bases
/// among them, which it deletes whatever their tier.
///
/// ⚠ **Fails with the gate removed** (the handlers as on `origin/main`): the base is deleted and
/// Soul recreated, 200.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_caller_holding_only_the_secret_cannot_reset_anything_else() {
    install_the_desktop_key();
    let state = AppState::new().await.unwrap();
    state
        .knowledge_service
        .create_base("reset-gate-fixture", "Reset gate fixture", None)
        .unwrap();

    let every_category = [
        "applications",
        "knowledge",
        "skills",
        "extensions",
        "schedules",
        "workflows",
        "history",
    ];
    // Bounded, because a reset that is NOT refused does not always come back in
    // this harness: in one of the runs with the gate removed, this all-category
    // reset parked and hung the whole binary instead of answering. A refusal is
    // immediate, so a regression fails here with a sentence, not a hang.
    let (status, body) = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        reset(&state, &every_category, Credential::SecretOnly),
    )
    .await
    .expect("the reset RAN for a caller holding only the secret, instead of being refused");
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(message_of(&body), RESET_NEEDS_USER);
    assert!(
        state
            .knowledge_service
            .get_base("reset-gate-fixture")
            .is_ok(),
        "the refused reset deleted a knowledge base anyway"
    );

    state
        .knowledge_service
        .delete_base("reset-gate-fixture")
        .unwrap();
}

/// The preview counts every private chat and every private base on the
/// machine — rows the listings omit for this caller — so it is refused too.
///
/// ⚠ **Fails with the gate removed** (the handlers as on `origin/main`): 200 with the counts.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_preview_counts_nothing_for_a_caller_that_did_not_prove_it_is_the_user() {
    install_the_desktop_key();
    let state = AppState::new().await.unwrap();
    let id = seed_private_chat(&state).await;

    for credential in [
        Credential::SecretOnly,
        Credential::WrongKey,
        Credential::PrivateCapability,
    ] {
        let (status, body) = send(&state, "GET", "/reset/preview", None, credential).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "the preview answered an unproven caller: {body}"
        );
        assert_eq!(message_of(&body), RESET_NEEDS_USER);
        assert!(body.get("counts").is_none(), "{body}");
    }

    state.session_manager().delete_session(&id).await.unwrap();
}

/// The refusal is the FIRST answer, ahead of the 400 for an empty selection —
/// and, by the same position, ahead of the 409 that would tell a refused caller
/// a chat or a scheduled run is in flight right now.
///
/// ⚠ **Fails with the gate removed** (the handlers as on `origin/main`): 400 "Select at least one
/// reset category".
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_refusal_comes_before_every_other_answer() {
    install_the_desktop_key();
    let state = AppState::new().await.unwrap();

    let (status, body) = reset(&state, &[], Credential::SecretOnly).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(message_of(&body), RESET_NEEDS_USER);

    // …while the person gets the route's own answer to the same request.
    let (status, body) = reset(&state, &[], Credential::Proof).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

/// The positive control, without which every refusal above could be a route
/// that simply stopped working: the desktop's proof previews and resets, and the
/// private chat is gone.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_person_at_the_keyboard_can_still_reset() {
    install_the_desktop_key();
    let state = AppState::new().await.unwrap();
    let id = seed_private_chat(&state).await;

    let (status, body) = send(&state, "GET", "/reset/preview", None, Credential::Proof).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["counts"]["conversations"].as_u64().unwrap_or(0) >= 1,
        "the preview did not count the seeded chat: {body}"
    );

    let (status, body) = reset(&state, &["history"], Credential::Proof).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["reset"], serde_json::json!(["history"]));
    assert!(
        state
            .session_manager()
            .get_session(&id, false)
            .await
            .is_err(),
        "a proven History reset left the chat in place"
    );
}

/// This binary installs a key, so the keyless sentence must never reach it; if
/// it did, every assertion above would be measuring a different daemon.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_daemon_with_a_key_never_says_it_has_none() {
    install_the_desktop_key();
    let state = AppState::new().await.unwrap();
    let (status, body) = reset(&state, &["workflows"], Credential::SecretOnly).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_ne!(message_of(&body), RESET_NO_USER_KEY);
}
