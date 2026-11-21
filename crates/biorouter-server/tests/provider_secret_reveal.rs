#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use axum::body::{to_bytes, Body};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::Router;
use biorouter::config::Config;
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use tower::ServiceExt;

const USER_ACTION_KEY: &str = "synthetic-desktop-user-action";
const SECRET_MASK: &str = "••••••••";

async fn app() -> Router {
    assert!(biorouter::config::test_binary_credentials::os_store_is_kept_off());
    let digest: [u8; 32] =
        <sha2::Sha256 as sha2::Digest>::digest(USER_ACTION_KEY.as_bytes()).into();
    biorouter_server::auth::install_user_action_digest(Some(digest));
    biorouter_server::routes::config_management::routes(AppState::new().await.unwrap())
}

async fn request(
    app: &Router,
    path: &str,
    body: Value,
    proof: Option<&str>,
) -> (StatusCode, HeaderMap, String) {
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
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    (status, headers, body)
}

fn assert_no_credential_bytes(body: &str, credential: &str) {
    if credential.is_empty() {
        return;
    }
    let chars: Vec<char> = credential.chars().collect();
    let mut fragments = vec![credential.to_string()];
    if chars.len() >= 8 {
        fragments.extend([
            chars[..4].iter().collect::<String>(),
            chars[chars.len() - 4..].iter().collect::<String>(),
        ]);
    }
    for fragment in fragments {
        assert!(
            !body.contains(&fragment),
            "response exposed synthetic credential bytes {fragment:?}: {body:?}"
        );
    }
}

async fn assert_fixed_config_mask(app: &Router) {
    let (status, _, body) = request(
        app,
        "/config/read",
        json!({ "key": "GOOGLE_API_KEY", "is_secret": true }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let value: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value, json!({ "maskedValue": SECRET_MASK }));
}

async fn assert_preview_route_absent(app: &Router, credentials: &[&str]) {
    let (status, _, body) = request(
        app,
        "/config/provider-secret/preview",
        json!({ "provider": "google", "key": "GOOGLE_API_KEY" }),
        Some(USER_ACTION_KEY),
    )
    .await;
    for credential in credentials {
        assert_no_credential_bytes(&body, credential);
    }
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[serial]
async fn an_environment_only_key_cannot_be_retrieved_by_the_preview_path() {
    let app = app().await;
    let credential = "ENVO-synthetic-environment-only-ENDX";
    let _env = env_lock::lock_env([("GOOGLE_API_KEY", Some(credential))]);
    Config::global().delete_secret("GOOGLE_API_KEY").unwrap();

    assert_fixed_config_mask(&app).await;
    assert_preview_route_absent(&app, &[credential]).await;
}

#[tokio::test]
#[serial]
async fn neither_a_saved_key_nor_its_environment_override_is_retrievable() {
    let app = app().await;
    let saved = "SAVE-synthetic-saved-credential-ENDS";
    let environment = "ENVS-synthetic-environment-shadow-ENDE";
    let _env = env_lock::lock_env([("GOOGLE_API_KEY", Some(environment))]);
    Config::global()
        .set_secret("GOOGLE_API_KEY", &saved)
        .unwrap();
    assert_eq!(
        Config::global()
            .all_secrets()
            .unwrap()
            .get("GOOGLE_API_KEY"),
        Some(&json!(saved))
    );

    assert_fixed_config_mask(&app).await;
    assert_preview_route_absent(&app, &[saved, environment]).await;
}

#[tokio::test]
#[serial]
async fn stored_keys_of_different_lengths_and_unicode_are_always_fully_masked() {
    let app = app().await;
    let _env = env_lock::lock_env([("GOOGLE_API_KEY", None::<&str>)]);
    let credentials = [
        "LONG-synthetic-stored-credential-MARK",
        "",
        "短",
        "四字短鍵",
        "NINE0TAIL",
        "EIGHSTOP",
        "abcdefghijklmnop",
        "abcdefghijklmnopq",
        "甲乙丙丁戊己庚辛壬癸子丑寅卯辰巳",
    ];

    for credential in credentials {
        Config::global()
            .set_secret("GOOGLE_API_KEY", &credential)
            .unwrap();
        assert_fixed_config_mask(&app).await;
        assert_preview_route_absent(&app, &[credential]).await;
    }
}
