#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode};
use axum::{middleware, Router};
use biorouter::config::Config;
use biorouter_server::auth::{self, UserActionProof};
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use tower::ServiceExt;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

const DAEMON_SECRET: &str = "synthetic-custom-retention-daemon-secret";
const USER_ACTION_KEY: &str = "synthetic-custom-retention-user-action";
const PROVIDER_ID: &str = "custom_synthetic_replacement_retention";
const KEY: &str = "SYNTHETIC_CUSTOM_RETENTION_CREDENTIAL";
const OLD: &str = "OLDK-synthetic-working-custom-key-ENDOLD";
const CANDIDATE: &str = "NEWK-synthetic-candidate-custom-key-ENDNEW";
const URL: &str = "https://custom-retention.fixture.invalid/v1";
const INVALID_ENGINE: &str = "unsupported-fixture-engine";

async fn daemon() -> Router {
    assert!(test_sandbox::global_config_reads_secrets_from_a_file());
    assert!(biorouter::config::test_binary_credentials::os_store_is_kept_off());
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin: &HeaderValue, _| {
            biorouter_server::routes::is_local_origin(origin.to_str().unwrap_or(""))
        }))
        .allow_methods(Any)
        .allow_headers(Any)
        .max_age(std::time::Duration::from_secs(600));
    biorouter_server::routes::configure(AppState::new().await.unwrap(), DAEMON_SECRET.to_string())
        .layer(middleware::from_fn_with_state(
            DAEMON_SECRET.to_string(),
            auth::check_token,
        ))
        .layer(cors)
}

async fn update(app: &Router, headers: &[(&str, &str)]) -> (StatusCode, String) {
    let mut builder = Request::put(format!("/config/custom-providers/{PROVIDER_ID}"))
        .header("content-type", "application/json")
        .header("host", "127.0.0.1:9380")
        .header("origin", "http://127.0.0.1:9380");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let mut request = builder
        .body(Body::from(
            json!({
                "engine": INVALID_ENGINE,
                "display_name": "Synthetic replacement retention",
                "api_url": URL,
                "api_key": CANDIDATE,
                "models": ["synthetic-model"],
                "supports_streaming": true,
            })
            .to_string(),
        ))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "192.0.2.71:47000".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn assert_no_credential_bytes(body: &str) {
    for fragment in [OLD, CANDIDATE, "OLDK", "ENDOLD", "NEWK", "ENDNEW"] {
        assert!(
            !body.contains(fragment),
            "custom-provider error exposed synthetic credential bytes: {body:?}"
        );
    }
    if let Ok(decoded) = serde_json::from_str::<Value>(body) {
        let normalized = decoded.to_string();
        for fragment in [OLD, CANDIDATE, "OLDK", "ENDOLD", "NEWK", "ENDNEW"] {
            assert!(
                !normalized.contains(fragment),
                "decoded custom-provider error exposed synthetic credential bytes: {body:?}"
            );
        }
    }
}

#[tokio::test]
#[serial]
async fn failed_custom_provider_replacement_preserves_the_saved_key() {
    if !test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let app = daemon().await;
    let digest: [u8; 32] =
        <sha2::Sha256 as sha2::Digest>::digest(USER_ACTION_KEY.as_bytes()).into();
    auth::install_user_action_digest(Some(digest));
    let mut proof = HeaderMap::new();
    proof.insert("X-User-Action", USER_ACTION_KEY.parse().unwrap());
    assert_eq!(auth::user_action_proof(&proof), UserActionProof::Proven);

    let _env = env_lock::lock_env([(KEY, None::<&str>)]);
    let root = std::path::PathBuf::from(std::env::var_os("BIOROUTER_PATH_ROOT").unwrap());
    let dir = biorouter::config::declarative_providers::custom_providers_dir();
    assert!(dir.starts_with(root));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("{PROVIDER_ID}.json"));
    let writer = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file)
        .unwrap();
    serde_json::to_writer(
        writer,
        &json!({
            "name": PROVIDER_ID,
            "engine": "openai",
            "display_name": "Synthetic replacement retention",
            "api_key_env": KEY,
            "base_url": URL,
            "models": [],
        }),
    )
    .unwrap();
    let original_definition = std::fs::read(&file).unwrap();
    let loaded = biorouter::config::declarative_providers::load_provider(PROVIDER_ID).unwrap();
    assert!(loaded.is_editable);
    assert_eq!(loaded.config.api_key_env, KEY);
    assert_eq!(loaded.config.base_url, URL);
    Config::global().set_secret(KEY, &OLD).unwrap();
    assert_eq!(
        Config::global().all_secrets().unwrap().get(KEY),
        Some(&json!(OLD))
    );
    assert!(
        biorouter::config::declarative_providers::config_for_updated_provider(
            PROVIDER_ID,
            INVALID_ENGINE,
            URL.to_string(),
        )
        .is_err()
    );
    assert_eq!(
        Config::global().all_secrets().unwrap().get(KEY),
        Some(&json!(OLD))
    );

    for headers in [
        Vec::new(),
        vec![("X-Secret-Key", "synthetic-wrong-daemon-secret")],
    ] {
        let (status, body) = update(&app, &headers).await;
        assert_no_credential_bytes(&body);
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            Config::global().all_secrets().unwrap().get(KEY),
            Some(&json!(OLD))
        );
        assert_eq!(std::fs::read(&file).unwrap(), original_definition);
    }

    let (status, body) = update(
        &app,
        &[
            ("X-Secret-Key", DAEMON_SECRET),
            ("X-User-Action", USER_ACTION_KEY),
        ],
    )
    .await;
    assert!(status.is_client_error() || status.is_server_error());
    assert_no_credential_bytes(&body);
    assert_eq!(std::fs::read(&file).unwrap(), original_definition);
    Config::global().invalidate_secrets_cache();
    assert_eq!(
        Config::global().all_secrets().unwrap().get(KEY),
        Some(&json!(OLD)),
        "failed custom-provider replacement changed the saved working credential"
    );
}
