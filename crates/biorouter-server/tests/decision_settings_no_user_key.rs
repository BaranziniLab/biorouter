//! A chat's decision settings on a daemon that holds no user-action key
//! (`biorouter serve`, or a hand-run `biorouterd`).
//!
//! There nothing can produce the proof a keyed daemon asks a settings write for,
//! so `routes::decisions` leaves the reach gate as the whole of the gate, as
//! SD-11 does for Stop: a browser on a `serve` host can set a chat's monitors.
//! That includes a steer monitor (owner ruling: the route does not refuse
//! one; the steer note is framed as data where it is used). A subagent's chat
//! is the exception, refused with the sentence `/reply` and `/agent/stop`
//! answer there. A desktop daemon that was meant to receive a key and did not
//! keeps the refusal, because that is a fault a restart repairs, not a `serve`
//! host.
//!
//! ⚠ Its own test binary on purpose: the installed digest is a process global
//! (`OnceLock`), so a binary that installs a key can never observe a keyless
//! daemon. Nothing here installs one. The keyed half is in the lib's
//! `routes::decisions` tests.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can open the developer's real `sessions.db`.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use biorouter::model::ModelConfig;
use biorouter::privacy::SessionClassification;
use biorouter::session::session_manager::SessionType;
use biorouter_server::auth::{user_action_proof, UserActionProof};
use biorouter_server::routes::decisions::{
    DECISION_SETTINGS_KEY_MISSING, DECISION_SETTINGS_SUBAGENT_NO_KEY,
};
use biorouter_server::routes::session_reach::{CALLER_PROVIDER_HEADER, SESSION_REACH_NO_KEY};
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use tower::ServiceExt;

fn assert_the_daemon_is_keyless() {
    assert_eq!(
        user_action_proof(&HeaderMap::new()),
        UserActionProof::NoKeyInstalled,
        "something in this binary installed a user-action digest, so these tests would be \
         measuring a desktop daemon rather than a `biorouter serve` one"
    );
    assert!(
        biorouter::privacy::privacy_tiers_enabled(),
        "privacy tiers are off, so the reach gate below would admit everything either way"
    );
}

async fn seed(state: &Arc<AppState>, tier: SessionClassification) -> String {
    seed_typed(state, tier, SessionType::User).await
}

async fn seed_typed(
    state: &Arc<AppState>,
    tier: SessionClassification,
    session_type: SessionType,
) -> String {
    let manager = state.session_manager();
    let session = manager
        .create_session(
            std::env::temp_dir(),
            format!("Decision settings, keyless, {tier:?} {session_type} (test fixture)"),
            session_type,
        )
        .await
        .unwrap();
    if tier == SessionClassification::Private {
        manager
            .update(&session.id)
            .provider_name("versa_azure")
            .model_config(ModelConfig::new("gpt-4o").unwrap())
            .raise_privacy(SessionClassification::Private, "turn:versa_azure")
            .apply()
            .await
            .unwrap();
    }
    session.id
}

fn settings() -> Value {
    settings_with_action("notify")
}

/// A monitor that is switched off. A write is checked for what it turns on
/// (`routes::decisions`' save check, driven over a fake decision source in the
/// lib's own tests), and a monitor that is off turns nothing on, so these
/// writes ask no decision model: this binary runs where no Ollama answers, and
/// what it measures is the gate, not the check.
fn settings_with_action(action: &str) -> Value {
    json!({
        "monitors": [{
            "id": "form",
            "name": "Form reached",
            "question": {
                "instructions": "Has the agent reached the consent form page?",
                "kind": { "binary": {} }
            },
            "threshold": 0.85,
            "action": action,
            "enabled": false
        }]
    })
}

/// A decision chat's question. The chats here are bound to no decision model,
/// so the save check asks nothing of it either.
fn chat_question() -> Value {
    json!({ "question": { "instructions": "Is it done?", "kind": { "binary": {} } } })
}

/// One request through the served tree, with the headers a browser tab on a
/// `serve` daemon sends (its shim answers the user-action key with an empty
/// string), plus a stated capability when given.
async fn send(
    state: &Arc<AppState>,
    method: &str,
    uri: &str,
    body: Option<Value>,
    caller_provider: Option<&str>,
) -> (StatusCode, String) {
    let app = biorouter_server::routes::configure(state.clone(), "keyless-secret".to_string());
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-User-Action", "");
    if let Some(provider) = caller_provider {
        request = request.header(CALLER_PROVIDER_HEADER, provider);
    }
    let body = match body {
        Some(json) => {
            request = request.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&json).unwrap())
        }
        None => Body::empty(),
    };
    let res = app.oneshot(request.body(body).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_browser_on_a_keyless_daemon_sets_a_reachable_chats_monitors() {
    assert_the_daemon_is_keyless();
    biorouter_server::launch::record_launch_state(false);
    let state = AppState::new().await.unwrap();

    let public = seed(&state, SessionClassification::Public).await;
    let uri = format!("/sessions/{public}/decision-settings");
    let (status, body) = send(&state, "PUT", &uri, Some(settings()), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = send(&state, "GET", &uri, None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let stored: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(stored["monitors"][0]["id"], "form", "{body}");

    // A private chat is still the reach gate's: refused with its keyless
    // sentence to a caller that states nothing, and admitted to one that states
    // a private model, as every route that names a chat is on this daemon.
    let private = seed(&state, SessionClassification::Private).await;
    let uri = format!("/sessions/{private}/decision-settings");
    let (status, body) = send(&state, "PUT", &uri, Some(settings()), None).await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::FORBIDDEN, SESSION_REACH_NO_KEY)
    );
    let (status, body) = send(&state, "PUT", &uri, Some(settings()), Some("versa_azure")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let _ = state.session_manager().delete_session(&public).await;
    let _ = state.session_manager().delete_session(&private).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_desktop_daemon_that_lost_its_key_keeps_the_refusal() {
    assert_the_daemon_is_keyless();
    // The desktop launcher declared it would hand this daemon a key.
    biorouter_server::launch::record_launch_state(true);
    let state = AppState::new().await.unwrap();

    let chat = seed(&state, SessionClassification::Public).await;
    for uri in [
        format!("/sessions/{chat}/decision-settings"),
        format!("/sessions/{chat}/decision-chat"),
    ] {
        let body = if uri.ends_with("decision-chat") {
            chat_question()
        } else {
            settings()
        };
        let (status, got) = send(&state, "PUT", &uri, Some(body), None).await;
        assert_eq!(
            (status, got.as_str()),
            (StatusCode::FORBIDDEN, DECISION_SETTINGS_KEY_MISSING),
            "{uri}"
        );
    }
    // Reading is not a write: still the reach gate alone.
    let (status, body) = send(
        &state,
        "GET",
        &format!("/sessions/{chat}/decision-settings"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    biorouter_server::launch::record_launch_state(false);
    let _ = state.session_manager().delete_session(&chat).await;
}

// Owner ruling: on a keyless daemon the route does not refuse a steer
// monitor. A shell reaches the same store through `biorouter session monitors`
// with no daemon at all, so a route refusal would stop only the person in the
// browser; the steer note is framed as data where it is used instead. Pinned so
// the admission reads as a decision, not an accident.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_browser_on_a_keyless_daemon_may_add_a_steer_monitor() {
    assert_the_daemon_is_keyless();
    biorouter_server::launch::record_launch_state(false);
    let state = AppState::new().await.unwrap();

    let chat = seed(&state, SessionClassification::Public).await;
    let uri = format!("/sessions/{chat}/decision-settings");
    let (status, body) = send(
        &state,
        "PUT",
        &uri,
        Some(settings_with_action("steer")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = send(&state, "GET", &uri, None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let stored: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(stored["monitors"][0]["action"], "steer", "{body}");

    let _ = state.session_manager().delete_session(&chat).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_browser_on_a_keyless_daemon_sets_a_decision_chats_question() {
    assert_the_daemon_is_keyless();
    biorouter_server::launch::record_launch_state(false);
    let state = AppState::new().await.unwrap();

    let chat = seed(&state, SessionClassification::Public).await;
    let uri = format!("/sessions/{chat}/decision-chat");
    let (status, body) = send(&state, "PUT", &uri, Some(chat_question()), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = send(&state, "GET", &uri, None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let stored: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        stored["settings"]["question"]["instructions"], "Is it done?",
        "{body}"
    );

    let _ = state.session_manager().delete_session(&chat).await;
}

// SD-11 keeps a subagent's tab refused on a keyless daemon for Stop and
// steering, and a subagent's turns load its own chat's monitors, so neither
// write reaches a subagent's chat here. Reading it is not a write.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_keyless_write_never_reaches_a_subagents_chat() {
    assert_the_daemon_is_keyless();
    biorouter_server::launch::record_launch_state(false);
    let state = AppState::new().await.unwrap();

    let subagent = seed_typed(&state, SessionClassification::Public, SessionType::SubAgent).await;
    for (uri, body) in [
        (
            format!("/sessions/{subagent}/decision-settings"),
            settings(),
        ),
        (
            format!("/sessions/{subagent}/decision-chat"),
            chat_question(),
        ),
    ] {
        let (status, got) = send(&state, "PUT", &uri, Some(body), None).await;
        // The plain sentence, word for word (this module answers plain text,
        // not `/agent/stop`'s JSON), which `biorouter session monitors` prints
        // as the reason.
        assert_eq!(
            (status, got.as_str()),
            (StatusCode::FORBIDDEN, DECISION_SETTINGS_SUBAGENT_NO_KEY),
            "{uri}"
        );
    }
    let (status, body) = send(
        &state,
        "GET",
        &format!("/sessions/{subagent}/decision-settings"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let stored: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        stored["monitors"],
        json!([]),
        "a refused write stored nothing: {body}"
    );
    let (status, body) = send(
        &state,
        "GET",
        &format!("/sessions/{subagent}/decision-chat"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let stored: Value = serde_json::from_str(&body).unwrap();
    assert!(stored["settings"].is_null(), "{body}");

    let _ = state.session_manager().delete_session(&subagent).await;
}
