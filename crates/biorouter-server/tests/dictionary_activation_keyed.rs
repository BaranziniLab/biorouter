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

const SECRET: &str = "dictionary-activation-test-secret";
const USER_KEY: &str = "dictionary-activation-test-key";

async fn request(app: &Router, proof: bool, enabled: bool) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/dictionary/activation")
        .header("X-Secret-Key", SECRET)
        .header("content-type", "application/json");
    if proof {
        builder = builder.header("X-User-Action", USER_KEY);
    }
    let response = app
        .clone()
        .oneshot(
            builder
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
async fn activation_requires_the_persons_proof_on_a_keyed_daemon() {
    let digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(USER_KEY.as_bytes()).into();
    biorouter_server::auth::install_user_action_digest(Some(digest));
    let state = AppState::new().await.unwrap();
    let app = dictionary::routes(Arc::clone(&state)).layer(axum::middleware::from_fn_with_state(
        SECRET.to_string(),
        biorouter_server::auth::check_token,
    ));
    assert!(!biorouter::dictation::activation::suggest_from_chats());
    let (status, _) = request(&app, false, true).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!biorouter::dictation::activation::suggest_from_chats());
    let (status, answer) = request(&app, true, true).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["suggest_from_chats"], true);
    assert!(biorouter::dictation::activation::suggest_from_chats());
    let (status, answer) = request(&app, true, false).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["suggest_from_chats"], false);
}

async fn attach_dictionary(app: &Router, session_id: &str, proof: bool) -> (StatusCode, String) {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/agent/add_extension")
        .header("X-Secret-Key", SECRET)
        .header("content-type", "application/json");
    if proof {
        builder = builder.header("X-User-Action", USER_KEY);
    }
    let body = json!({
        "session_id": session_id,
        "config": {"type": "platform", "name": "dictionary", "description": "", "bundled": true}
    });
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

// DD-56: the Dictionary learns from the chat it is attached to, so `POST
// /agent/add_extension` asks for the person exactly as turning on suggestions does. A
// public chat needs no proof for any other extension, which is why the secret alone
// used to attach this one.
#[tokio::test]
async fn attaching_the_dictionary_to_a_chat_needs_the_persons_proof_on_a_keyed_daemon() {
    let digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(USER_KEY.as_bytes()).into();
    biorouter_server::auth::install_user_action_digest(Some(digest));
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
    let (status, body) = attach_dictionary(&app, &session.id, false).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains(dictionary::MSG_DICTIONARY_AGENT), "{body}");
    let (status, body) = attach_dictionary(&app, &session.id, true).await;
    assert_ne!(status, StatusCode::FORBIDDEN, "the person's attach was refused: {body}");
}
