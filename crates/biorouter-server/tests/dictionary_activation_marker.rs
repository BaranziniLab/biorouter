#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use biorouter::config::Config;
use biorouter::dictation::activation::ACTIVATION_VERSION_KEY;
use biorouter_server::{routes::dictionary, state::AppState};
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt;

const SECRET: &str = "dictionary-activation-marker-secret";
const USER_KEY: &str = "dictionary-activation-marker-key";

async fn activate(app: &Router, enabled: bool, first_install: bool) -> StatusCode {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/dictionary/activation")
                .header("X-Secret-Key", SECRET)
                .header("X-User-Action", USER_KEY)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"suggest_from_chats": enabled, "first_install": first_install})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    response.status()
}

#[tokio::test]
async fn first_verified_activation_writes_version_one_and_later_off_keeps_it() {
    let digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(USER_KEY.as_bytes()).into();
    biorouter_server::auth::install_user_action_digest(Some(digest));
    let state = AppState::new().await.unwrap();
    let app = dictionary::routes(Arc::clone(&state)).layer(axum::middleware::from_fn_with_state(
        SECRET.to_string(),
        biorouter_server::auth::check_token,
    ));
    assert!(Config::global()
        .get_param::<u8>(ACTIVATION_VERSION_KEY)
        .is_err());
    assert_eq!(activate(&app, true, true).await, StatusCode::OK);
    assert_eq!(
        Config::global()
            .get_param::<u8>(ACTIVATION_VERSION_KEY)
            .unwrap(),
        1
    );
    assert_eq!(activate(&app, false, false).await, StatusCode::OK);
    assert_eq!(
        Config::global()
            .get_param::<u8>(ACTIVATION_VERSION_KEY)
            .unwrap(),
        1
    );
    assert!(!biorouter::dictation::activation::suggest_from_chats());
    assert_eq!(activate(&app, true, true).await, StatusCode::OK);
    assert!(!biorouter::dictation::activation::suggest_from_chats());
}
