#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use axum::body::{to_bytes, Body};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::Router;
use biorouter::config::Config;
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use tower::ServiceExt;

const USER_ACTION_KEY: &str = "synthetic-desktop-user-action";
const SYNTHETIC_KEY: &str = "synthetic-google-api-key";

async fn request(
    app: &Router,
    path: &str,
    body: Value,
    proof: Option<&str>,
) -> (StatusCode, HeaderMap, Value) {
    let mut builder = Request::post(path).header("content-type", "application/json");
    if let Some(proof) = proof {
        builder = builder.header("X-User-Action", proof);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, headers, body)
}

#[tokio::test]
async fn a_saved_provider_secret_is_previewed_only_for_a_proven_person_and_declared_key() {
    for (key, value) in [
        ("GOOGLE_API_KEY", SYNTHETIC_KEY),
        ("ANTHROPIC_API_KEY", "synthetic-anthropic-key"),
        ("GOOGLE_HOST", "synthetic-not-a-secret-field"),
        ("GITHUB_COPILOT_TOKEN", "synthetic-oauth-token"),
    ] {
        std::env::remove_var(key);
        Config::global().set_secret(key, &value).unwrap();
    }
    let digest: [u8; 32] =
        <sha2::Sha256 as sha2::Digest>::digest(USER_ACTION_KEY.as_bytes()).into();
    biorouter_server::auth::install_user_action_digest(Some(digest));
    let state = AppState::new().await.unwrap();
    let app = biorouter_server::routes::config_management::routes(state);

    let (status, _, masked) = request(
        &app,
        "/config/read",
        json!({ "key": "GOOGLE_API_KEY", "is_secret": true }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(masked, json!({ "maskedValue": "••••••••" }));

    let query = json!({ "provider": "google", "key": "GOOGLE_API_KEY" });
    for proof in [None, Some("wrong-proof")] {
        let (status, _, body) = request(
            &app,
            "/config/provider-secret/preview",
            query.clone(),
            proof,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_ne!(body, json!(SYNTHETIC_KEY));
    }

    let (status, headers, body) = request(
        &app,
        "/config/provider-secret/preview",
        query,
        Some(USER_ACTION_KEY),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!("synt••••••••-key"));
    assert_ne!(body, json!(SYNTHETIC_KEY));
    assert_eq!(headers.get("cache-control").unwrap(), "no-store");

    let (status, _, body) = request(
        &app,
        "/config/provider-secret/reveal",
        json!({ "provider": "google", "key": "GOOGLE_API_KEY" }),
        Some(USER_ACTION_KEY),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_ne!(body, json!(SYNTHETIC_KEY));

    for (secret, expected) in [
        ("abcd", "••••••••"),
        ("abcdefgh", "••••••••"),
        ("abcdefghi", "abcd••••••••fghi"),
        ("甲乙丙丁中戊己庚辛", "甲乙丙丁••••••••戊己庚辛"),
    ] {
        Config::global()
            .set_secret("GOOGLE_API_KEY", &secret)
            .unwrap();
        let (status, _, body) = request(
            &app,
            "/config/provider-secret/preview",
            json!({ "provider": "google", "key": "GOOGLE_API_KEY" }),
            Some(USER_ACTION_KEY),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!(expected));
        assert_ne!(body, json!(secret));
    }

    for query in [
        json!({ "provider": "google", "key": "ANTHROPIC_API_KEY" }),
        json!({ "provider": "google", "key": "GOOGLE_HOST" }),
        json!({ "provider": "not-a-provider", "key": "GOOGLE_API_KEY" }),
        json!({ "provider": "github_copilot", "key": "GITHUB_COPILOT_TOKEN" }),
    ] {
        let (status, _, body) = request(
            &app,
            "/config/provider-secret/preview",
            query,
            Some(USER_ACTION_KEY),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_ne!(body, json!(SYNTHETIC_KEY));
    }

    let custom = biorouter::config::declarative_providers::create_custom_provider(
        "openai_compatible",
        "Secret fixture".to_string(),
        "https://example.invalid/v1".to_string(),
        "synthetic-custom-key".to_string(),
        vec!["fixture-model".to_string()],
        Some(true),
        None,
    )
    .unwrap();
    std::env::remove_var(&custom.api_key_env);
    biorouter::providers::refresh_custom_providers()
        .await
        .unwrap();
    let (status, _, body) = request(
        &app,
        "/config/provider-secret/preview",
        json!({ "provider": custom.name, "key": custom.api_key_env }),
        Some(USER_ACTION_KEY),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!("synt••••••••-key"));

    Config::global().delete_secret("GOOGLE_API_KEY").unwrap();
    let (status, _, _) = request(
        &app,
        "/config/provider-secret/preview",
        json!({ "provider": "google", "key": "GOOGLE_API_KEY" }),
        Some(USER_ACTION_KEY),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
