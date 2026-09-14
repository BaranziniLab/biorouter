//! SD-8 at `POST /reset` and `GET /reset/preview`: on a daemon that holds no
//! proof-of-user key — the one `biorouter serve` starts (SD-7) — a reset is
//! refused, and the refusal says something its reader can act on.
//!
//! On `origin/main` (`1038a113`) neither route took a header, so a keyless
//! daemon reset whatever any caller holding its secret asked it to: measured on
//! a sandboxed daemon, `POST /reset {"categories":["history"]}` with only
//! `X-Secret-Key` emptied the session store, a private chat included. On a
//! `serve` host that secret sits in the served page and in the daemon's
//! environment, so the caller could be the browser tab or a model with a shell.
//!
//! A reset is the person's decision and this daemon cannot tell a person from a
//! model, so it cannot be performed here — the same ruling
//! `declassify_no_user_key.rs` pins for lowering a chat's tier. What this binary
//! pins beyond the refusal is its WORDS: the sentence for a daemon that holds a
//! key ends "stop and ask the user to reset it from Settings", which is a loop
//! for a person who is standing in Settings. This daemon names itself as the
//! reason and points at the machine it runs on.
//!
//! ⚠ **Its own test binary on purpose**: the installed digest is a process-global
//! `OnceLock`, and `reset_requires_user.rs` installs one. Nothing here installs a
//! digest — which is exactly how `biorouter serve` starts its daemon.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can open the developer's real `sessions.db`.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use biorouter::conversation::message::Message;
use biorouter::model::ModelConfig;
use biorouter::privacy::SessionClassification;
use biorouter::session::session_manager::SessionType;
use biorouter_server::auth::{user_action_proof, UserActionProof};
use biorouter_server::routes::reset::{RESET_NEEDS_USER, RESET_NO_USER_KEY};
use biorouter_server::state::AppState;
use serial_test::serial;
use tower::ServiceExt;

/// The server secret this binary's "daemon" was launched with.
const TEST_SECRET: &str = "sd8-reset-no-user-key";

/// Every test here stands on this: the daemon under test holds no key.
fn assert_the_daemon_is_keyless() {
    assert_eq!(
        user_action_proof(&HeaderMap::new()),
        UserActionProof::NoKeyInstalled,
        "something in this binary installed a user-action digest, so these tests would be \
         measuring a desktop daemon rather than a `biorouter serve` one"
    );
}

async fn seed_private_chat(state: &Arc<AppState>) -> String {
    let manager = state.session_manager();
    let session = manager
        .create_session(
            std::env::temp_dir().join("sd8_reset"),
            "SD-8 reset fixture".to_string(),
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

/// The request a browser page on `biorouter serve` sends: the secret it was
/// handed, the host's provider named the way `userActionHeaders()` names it
/// there, and — because there is no key to offer — a guess at `X-User-Action`
/// standing in for anything else a caller might try. Layered through the SAME
/// `check_token` middleware the real daemon installs.
async fn send(
    state: Arc<AppState>,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let app =
        biorouter_server::routes::reset::routes(state).layer(axum::middleware::from_fn_with_state(
            TEST_SECRET.to_string(),
            biorouter_server::auth::check_token,
        ));
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Secret-Key", TEST_SECRET)
        .header("X-Caller-Provider", "versa_azure")
        .header("X-User-Action", "biorouter-dev-user-action");
    let request = match body {
        Some(json) => builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&json).unwrap())),
        None => builder.body(Body::empty()),
    }
    .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(&bytes).into())),
    )
}

fn message_of(body: &serde_json::Value) -> &str {
    body.get("message")
        .and_then(|message| message.as_str())
        .unwrap_or_else(|| panic!("the refusal is not in the route's error envelope: {body}"))
}

/// ⚠ **Fails with the gate removed** (the handlers as on `origin/main`): 200, and the private chat
/// is gone.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_keyless_daemon_refuses_a_reset_in_words_a_person_can_act_on() {
    assert_the_daemon_is_keyless();
    let state = AppState::new().await.unwrap();
    let id = seed_private_chat(&state).await;

    let (status, body) = send(
        state.clone(),
        "POST",
        "/reset",
        Some(serde_json::json!({ "categories": ["history"] })),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a keyless daemon reset History for a caller it cannot tell from a model: {body}"
    );
    assert_eq!(
        state
            .session_manager()
            .get_session(&id, false)
            .await
            .expect("the refused reset deleted the private chat anyway")
            .privacy_tier,
        SessionClassification::Private
    );
    assert_eq!(message_of(&body), RESET_NO_USER_KEY);
    assert_ne!(
        message_of(&body),
        RESET_NEEDS_USER,
        "the person at the keyboard was handed the sentence written for an AI agent"
    );

    state.session_manager().delete_session(&id).await.unwrap();
}

/// ⚠ **Fails with the gate removed** (the handlers as on `origin/main`): 200 with every count on
/// the machine.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_keyless_daemon_counts_nothing_for_the_preview() {
    assert_the_daemon_is_keyless();
    let state = AppState::new().await.unwrap();

    let (status, body) = send(state, "GET", "/reset/preview", None).await;

    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(message_of(&body), RESET_NO_USER_KEY);
    assert!(body.get("counts").is_none(), "{body}");
}
