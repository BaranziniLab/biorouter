#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode};
use axum::{middleware, Router};
use biorouter::config::paths::Paths;
use biorouter_server::auth::{self, UserActionProof};
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use tower::ServiceExt;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

const DAEMON_SECRET: &str = "synthetic-custom-metadata-daemon-secret";
const USER_ACTION_KEY: &str = "synthetic-custom-metadata-user-action";
const PROVIDER_ID: &str = "custom_synthetic_metadata_without_secret_access";
const URL: &str = "https://metadata-without-secret-access.fixture.invalid/v1";
const DISPLAY_NAME: &str = "Synthetic metadata original";
const UPDATED_DISPLAY_NAME: &str = "Synthetic metadata updated";
const UPDATED_MODEL: &str = "synthetic-updated-model";
const VALID_SECRETS: &[u8] = b"synthetic_unrelated_secret: preserved\n";
const MALFORMED_SECRETS: &[u8] = b"synthetic_unrelated_secret: [malformed\n";
const DIRECTORY_MARKER: &[u8] = b"synthetic directory marker\n";

fn private_secrets_path() -> PathBuf {
    let root = PathBuf::from(std::env::var_os("BIOROUTER_PATH_ROOT").unwrap());
    let path = Paths::config_dir().join("secrets.yaml");
    assert!(
        path.starts_with(&root),
        "private secret path {} escaped test root {}",
        path.display(),
        root.display()
    );
    path
}

fn definition_path() -> PathBuf {
    let root = PathBuf::from(std::env::var_os("BIOROUTER_PATH_ROOT").unwrap());
    let path = biorouter::config::declarative_providers::custom_providers_dir()
        .join(format!("{PROVIDER_ID}.json"));
    assert!(
        path.starts_with(&root),
        "custom definition path {} escaped test root {}",
        path.display(),
        root.display()
    );
    path
}

fn seed_owned_definition() -> PathBuf {
    let path = definition_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "name": PROVIDER_ID,
            "engine": "openai",
            "display_name": DISPLAY_NAME,
            "description": "Synthetic metadata fixture",
            "api_key_env": "SYNTHETIC_METADATA_SAVED_KEY",
            "base_url": URL,
            "models": [],
            "headers": null,
            "timeout_seconds": null,
            "supports_streaming": false
        }))
        .unwrap(),
    )
    .unwrap();
    path
}

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

fn install_user_action_proof() {
    let digest: [u8; 32] =
        <sha2::Sha256 as sha2::Digest>::digest(USER_ACTION_KEY.as_bytes()).into();
    auth::install_user_action_digest(Some(digest));
    let mut headers = HeaderMap::new();
    headers.insert("X-User-Action", USER_ACTION_KEY.parse().unwrap());
    assert_eq!(auth::user_action_proof(&headers), UserActionProof::Proven);
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
                "engine": "openai_compatible",
                "display_name": UPDATED_DISPLAY_NAME,
                "api_url": URL,
                "api_key": "",
                "models": [UPDATED_MODEL],
                "supports_streaming": true
            })
            .to_string(),
        ))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "192.0.2.71:47000".parse::<SocketAddr>().unwrap(),
    ));

    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

fn valid_headers() -> [(&'static str, &'static str); 2] {
    [
        ("X-Secret-Key", DAEMON_SECRET),
        ("X-User-Action", USER_ACTION_KEY),
    ]
}

fn assert_unauthorized_without_mutation(
    status: StatusCode,
    body: &str,
    definition: &Path,
    original_definition: &[u8],
    secrets: &Path,
    original_secrets: &[u8],
) {
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "unauthorized request body: {body}"
    );
    assert!(
        !body.contains("synthetic_unrelated_secret"),
        "unauthorized response exposed synthetic secret-store content: {body:?}"
    );
    assert_eq!(
        std::fs::read(definition).unwrap(),
        original_definition,
        "unauthorized request changed the owned provider definition"
    );
    assert_eq!(
        std::fs::read(secrets).unwrap(),
        original_secrets,
        "unauthorized request changed the private secret store"
    );
}

fn assert_updated_definition(path: &Path) {
    let value: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(value["name"], PROVIDER_ID);
    assert_eq!(value["engine"], "openai");
    assert_eq!(value["display_name"], UPDATED_DISPLAY_NAME);
    assert_eq!(value["base_url"], URL);
    assert_eq!(value["supports_streaming"], true);
    assert_eq!(value["models"][0]["name"], UPDATED_MODEL);
}

#[tokio::test]
#[serial]
async fn metadata_edit_with_valid_saved_store_is_a_real_http_control() {
    if !test_sandbox::in_a_process_of_its_own() {
        return;
    }

    let definition = seed_owned_definition();
    let secrets = private_secrets_path();
    std::fs::create_dir_all(secrets.parent().unwrap()).unwrap();
    std::fs::write(&secrets, VALID_SECRETS).unwrap();
    let original_definition = std::fs::read(&definition).unwrap();
    let original_secrets = std::fs::read(&secrets).unwrap();

    let app = daemon().await;
    install_user_action_proof();

    let (status, body) = update(&app, &valid_headers()).await;
    assert_eq!(status, StatusCode::OK, "metadata control response: {body}");
    assert_updated_definition(&definition);
    assert_ne!(std::fs::read(&definition).unwrap(), original_definition);
    assert_eq!(
        std::fs::read(&secrets).unwrap(),
        original_secrets,
        "metadata-only edit changed an unrelated valid secret store"
    );
}

#[tokio::test]
#[serial]
async fn metadata_edit_does_not_parse_malformed_unrelated_secret_store() {
    if !test_sandbox::in_a_process_of_its_own() {
        return;
    }

    let definition = seed_owned_definition();
    let original_definition = std::fs::read(&definition).unwrap();
    let secrets = private_secrets_path();
    std::fs::create_dir_all(secrets.parent().unwrap()).unwrap();
    std::fs::write(&secrets, VALID_SECRETS).unwrap();

    let app = daemon().await;
    std::fs::write(&secrets, MALFORMED_SECRETS).unwrap();
    let malformed = std::fs::read(&secrets).unwrap();
    install_user_action_proof();

    for headers in [
        Vec::new(),
        vec![("X-Secret-Key", "synthetic-wrong-daemon-secret")],
    ] {
        let (status, body) = update(&app, &headers).await;
        assert_unauthorized_without_mutation(
            status,
            &body,
            &definition,
            &original_definition,
            &secrets,
            &malformed,
        );
    }

    let (status, body) = update(&app, &valid_headers()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "same-URL metadata edit must not require parsing unrelated malformed secrets: {body}"
    );
    assert_updated_definition(&definition);
    assert_eq!(
        std::fs::read(&secrets).unwrap(),
        malformed,
        "metadata-only edit changed malformed unrelated secret-store bytes"
    );
}

#[tokio::test]
#[serial]
async fn metadata_edit_does_not_read_unavailable_unrelated_secret_store() {
    if !test_sandbox::in_a_process_of_its_own() {
        return;
    }

    let definition = seed_owned_definition();
    let original_definition = std::fs::read(&definition).unwrap();
    let secrets = private_secrets_path();
    std::fs::create_dir_all(secrets.parent().unwrap()).unwrap();
    std::fs::write(&secrets, VALID_SECRETS).unwrap();
    let app = daemon().await;
    std::fs::remove_file(&secrets).unwrap();
    std::fs::create_dir(&secrets).unwrap();
    let marker = secrets.join("synthetic-marker");
    std::fs::write(&marker, DIRECTORY_MARKER).unwrap();

    let original_marker = std::fs::read(&marker).unwrap();
    install_user_action_proof();

    let (status, body) = update(&app, &valid_headers()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "same-URL metadata edit must not read an unrelated secret-store directory: {body}"
    );
    assert_updated_definition(&definition);
    assert_ne!(
        std::fs::read(&definition).unwrap(),
        original_definition,
        "metadata edit did not persist the requested fields"
    );
    assert!(
        secrets.is_dir(),
        "unrelated secret-store directory was replaced"
    );
    assert_eq!(
        std::fs::read(&marker).unwrap(),
        original_marker,
        "unrelated secret-store directory marker changed"
    );
}
