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
