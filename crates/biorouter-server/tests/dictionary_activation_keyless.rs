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
    let state = AppState::new().await.unwrap();
    let app = dictionary::routes(Arc::clone(&state)).layer(axum::middleware::from_fn_with_state(
        SECRET.to_string(),
        biorouter_server::auth::check_token,
    ));
    let (status, body) = request(&app, true).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        body["message"],
        dictionary::msg_dictionary_host_settings("--suggest-from-chats on")
    );
    let (status, body) = request(&app, false).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["suggest_from_chats"], false);
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
        text.contains(&dictionary::msg_dictionary_host_settings("--suggest-from-chats on")),
        "{text}"
    );
}
