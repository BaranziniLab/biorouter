#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use biorouter_server::{routes::dictionary, state::AppState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

const SECRET: &str = "dictionary-activation-keyless-secret";

/// The tests that read or write the machine-wide `dictionary` capability hold this, because
/// every test in the binary shares one `Config::global()` and libtest runs them at once.
static MACHINE_WIDE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// `POST /agent/start` naming the Dictionary in its overrides, as the desktop's Home names
/// every enabled extension, with no proof: the status, the body and the chats it added to
/// the listing. The listing leaves out a chat with no messages, so an admitted start is
/// checked by the chat its answer names.
async fn start_naming_the_dictionary(state: &Arc<AppState>) -> (StatusCode, String, usize) {
    let app = biorouter_server::routes::agent::routes(Arc::clone(state)).layer(
        axum::middleware::from_fn_with_state(
            SECRET.to_string(),
            biorouter_server::auth::check_token,
        ),
    );
    let before = state.session_manager().list_sessions().await.unwrap().len();
    let body = json!({
        "working_dir": std::env::temp_dir().to_string_lossy(),
        "extension_overrides": [
            {"type": "platform", "name": "dictionary", "description": "", "bundled": true}
        ]
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/agent/start")
                .header("X-Secret-Key", SECRET)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let after = state.session_manager().list_sessions().await.unwrap().len();
    (
        status,
        String::from_utf8_lossy(&bytes).into_owned(),
        after - before,
    )
}

async fn request(app: &Router, enabled: bool) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/dictionary/activation")
                .header("X-Secret-Key", SECRET)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"suggest_from_chats":enabled}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[tokio::test]
async fn keyless_daemon_can_turn_off_but_cannot_turn_on() {
    let _machine_wide = MACHINE_WIDE.lock().await;
    let state = AppState::new().await.unwrap();
    let app = dictionary::routes(Arc::clone(&state)).layer(axum::middleware::from_fn_with_state(
        SECRET.to_string(),
        biorouter_server::auth::check_token,
    ));
    // On, as a fresh configuration has it (owner, 2026-11-19); set here because another test
    // in this binary may have left it off. Naming what is already on is no raise, turning it
    // off is admitted, and turning it back on is refused.
    biorouter::dictation::activation::activate(true).unwrap();
    let (status, body) = request(&app, true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["suggest_from_chats"], true);
    let (status, body) = request(&app, false).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["suggest_from_chats"], false);
    let (status, body) = request(&app, true).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        body["message"],
        dictionary::msg_dictionary_host_settings("--suggest-from-chats on")
    );
    assert!(!biorouter::dictation::activation::suggest_from_chats());
}

// DD-56 on a daemon that holds no key (`biorouter serve`): nothing can prove the person
// asked, so attaching the Dictionary to a chat is sent to the server, as turning on
// suggestions from chats is.
#[tokio::test]
async fn keyless_daemon_does_not_attach_the_dictionary_to_a_chat() {
    let state = AppState::new().await.unwrap();
    let session = state
        .session_manager()
        .create_session(
            std::env::temp_dir(),
            "Public chat".to_string(),
            biorouter::session::session_manager::SessionType::User,
        )
        .await
        .unwrap();
    let app = biorouter_server::routes::agent::routes(Arc::clone(&state)).layer(
        axum::middleware::from_fn_with_state(
            SECRET.to_string(),
            biorouter_server::auth::check_token,
        ),
    );
    let body = json!({
        "session_id": session.id,
        "config": {"type": "platform", "name": "dictionary", "description": "", "bundled": true}
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/agent/add_extension")
                .header("X-Secret-Key", SECRET)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
    assert!(
        text.contains(&dictionary::msg_dictionary_host_settings(
            "--suggest-from-chats on"
        )),
        "{text}"
    );
}

// DD-56 at the new-chat door on a daemon that holds no key: while the person has the
// Dictionary off, a start that names it is sent to the server, like the activation choice,
// and leaves no chat.
#[tokio::test]
async fn keyless_daemon_does_not_start_a_chat_with_the_dictionary() {
    let _machine_wide = MACHINE_WIDE.lock().await;
    // The person's saved off, as `biorouter dictionary settings --suggest-from-chats off`
    // writes it; a fresh configuration has it on (owner, 2026-11-19).
    biorouter::dictation::activation::activate(false).unwrap();
    let state = AppState::new().await.unwrap();
    let (status, text, added) = start_naming_the_dictionary(&state).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
    assert!(
        text.contains(&dictionary::msg_dictionary_host_settings(
            "--suggest-from-chats on"
        )),
        "{text}"
    );
    assert_eq!(added, 0, "a refused start leaves no chat behind");
}

// While the Dictionary is on for new chats (a fresh configuration, owner 2026-11-19), a start
// that names it gets what a start naming nothing gets, so it is not asked again. The desktop's
// Home names every enabled extension in its overrides, so asking here refused every new chat
// in a `biorouter serve` browser, which can never show the proof.
#[tokio::test]
async fn keyless_daemon_starts_a_chat_that_names_the_dictionary_while_it_is_on() {
    let _machine_wide = MACHINE_WIDE.lock().await;
    biorouter::dictation::activation::activate(true).unwrap();
    let state = AppState::new().await.unwrap();
    let (status, text, _) = start_naming_the_dictionary(&state).await;
    assert_ne!(status, StatusCode::FORBIDDEN, "{text}");
    assert!(
        !text.contains(&dictionary::msg_dictionary_host_settings(
            "--suggest-from-chats on"
        )),
        "{text}"
    );
    assert_eq!(status, StatusCode::OK, "{text}");
    let id = serde_json::from_str::<Value>(&text).unwrap()["id"]
        .as_str()
        .expect("the started chat carries an id")
        .to_string();
    assert!(
        state
            .session_manager()
            .get_session(&id, false)
            .await
            .is_ok(),
        "the start made its chat"
    );
}
