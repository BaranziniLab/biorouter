#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode};
use axum::{middleware, Router};
use biorouter::config::Config;
use biorouter::privacy::ProviderTier;
use biorouter_server::auth::{self, UserActionProof};
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use tower::ServiceExt;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

const DAEMON_SECRET: &str = "synthetic-config-classification-daemon-secret";
const USER_ACTION_KEY: &str = "synthetic-config-classification-user-action";
const BROWSER_TOKEN: &str = "synthetic-config-classification-browser-token";
const SAVED: &str = "SAVE-synthetic-config-read-fixture-ENDS";
const ENVIRONMENT: &str = "ENVR-synthetic-config-read-fixture-ENDE";
const HOST: &str = "https://nonsecret.fixture.invalid";
const SECRET_MASK: &str = "••••••••";

async fn daemon() -> Router {
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

async fn read(
    app: &Router,
    key: &str,
    is_secret: bool,
    headers: &[(&str, &str)],
) -> (StatusCode, String) {
    let mut builder = Request::post("/config/read")
        .header("content-type", "application/json")
        .header("host", "127.0.0.1:9380")
        .header("origin", "http://127.0.0.1:9380");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let mut request = builder
        .body(Body::from(
            json!({ "key": key, "is_secret": is_secret }).to_string(),
        ))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "192.0.2.70:47000".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn assert_no_credential_bytes(body: &str) {
    for fragment in [SAVED, ENVIRONMENT, "SAVE", "ENDS", "ENVR", "ENDE"] {
        assert!(
            !body.contains(fragment),
            "HTTP response exposed synthetic credential bytes {fragment:?}: {body:?}"
        );
    }
}

fn assert_masked(status: StatusCode, body: &str) {
    assert_no_credential_bytes(body);
    assert_eq!(status, StatusCode::OK);
    let value: Value = serde_json::from_str(body).unwrap();
    assert_eq!(value, json!({ "maskedValue": SECRET_MASK }));
}

async fn exercise(app: &Router, person: &[(&str, &str)]) {
    let metadata = biorouter::providers::providers().await;
    assert!(metadata.into_iter().any(|(provider, _)| {
        provider.name == "google"
            && provider
                .config_keys
                .iter()
                .any(|key| key.name == "GOOGLE_API_KEY" && key.secret)
    }));
    Config::global().delete("GOOGLE_API_KEY").unwrap();
    Config::global()
        .set_secret("GOOGLE_API_KEY", &SAVED)
        .unwrap();
    let bearer = [("X-Secret-Key", DAEMON_SECRET)];
    let mut person_with_bearer = person.to_vec();
    person_with_bearer.push(("X-Secret-Key", DAEMON_SECRET));
    let model = [
        ("X-Secret-Key", DAEMON_SECRET),
        ("X-Caller-Provider", "anthropic"),
    ];

    {
        let _env = env_lock::lock_env([
            ("GOOGLE_API_KEY", None::<&str>),
            ("GOOGLE_HOST", Some(HOST)),
        ]);
        let (status, body) = read(app, "GOOGLE_API_KEY", true, &bearer).await;
        assert_masked(status, &body);
        let (status, body) = read(app, "GOOGLE_API_KEY", false, &bearer).await;
        assert_no_credential_bytes(&body);
        assert_eq!(status, StatusCode::OK);
        let value: Value = serde_json::from_str(&body).unwrap();
        assert!(value.is_null() || value == json!({ "maskedValue": SECRET_MASK }));
        let (status, body) = read(app, "GOOGLE_HOST", false, &bearer).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(serde_json::from_str::<Value>(&body).unwrap(), json!(HOST));
    }

    let _env = env_lock::lock_env([("GOOGLE_API_KEY", Some(ENVIRONMENT))]);
    Config::global().delete_secret("GOOGLE_API_KEY").unwrap();
    let launch = auth::mint_app_launch("synthetic-config-read-app");
    let cookie = auth::redeem_app_launch("synthetic-config-read-app", &launch).unwrap();
    let cookie = cookie.split(';').next().unwrap();
    for denied in [
        Vec::new(),
        vec![("X-Secret-Key", "synthetic-wrong-daemon-secret")],
        person.to_vec(),
        vec![("cookie", cookie)],
    ] {
        let (status, body) = read(app, "GOOGLE_API_KEY", false, &denied).await;
        assert_no_credential_bytes(&body);
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, body) = read(app, "GOOGLE_API_KEY", true, &bearer).await;
    assert_masked(status, &body);

    let mut observations = Vec::new();
    for (label, headers) in [
        ("daemon bearer", bearer.as_slice()),
        ("bearer plus person proof", person_with_bearer.as_slice()),
        ("bearer plus model caller", model.as_slice()),
    ] {
        for key in ["GOOGLE_API_KEY", "google_api_key"] {
            let (status, body) = read(app, key, false, headers).await;
            observations.push((label, key, status, body));
        }
    }
    Config::global()
        .set_secret("GOOGLE_API_KEY", &SAVED)
        .unwrap();
    assert_eq!(
        Config::global()
            .all_secrets()
            .unwrap()
            .get("GOOGLE_API_KEY"),
        Some(&json!(SAVED))
    );
    let (status, body) = read(app, "GOOGLE_API_KEY", false, &bearer).await;
    observations.push((
        "saved plus environment shadow",
        "GOOGLE_API_KEY",
        status,
        body,
    ));

    for (label, key, status, body) in observations {
        assert!(
            !body.contains(ENVIRONMENT),
            "{label} retrieved the synthetic environment credential under {key}: {body:?}"
        );
        assert_masked(status, &body);
    }
}

#[tokio::test]
#[serial]
async fn desktop_config_read_classifies_secrets_before_trusting_the_false_flag() {
    if !test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let digest: [u8; 32] =
        <sha2::Sha256 as sha2::Digest>::digest(USER_ACTION_KEY.as_bytes()).into();
    auth::install_user_action_digest(Some(digest));
    let mut headers = HeaderMap::new();
    headers.insert("X-User-Action", USER_ACTION_KEY.parse().unwrap());
    assert_eq!(auth::user_action_proof(&headers), UserActionProof::Proven);
    exercise(&daemon().await, &[("X-User-Action", USER_ACTION_KEY)]).await;
}

#[tokio::test]
#[serial]
async fn served_config_read_classifies_secrets_before_trusting_the_false_flag() {
    if !test_sandbox::in_a_process_of_its_own() {
        return;
    }
    biorouter_server::launch::record_launch_state(false);
    auth::install_served_operator(BROWSER_TOKEN.to_string(), ProviderTier::Public);
    let cookie = format!("biorouter_session={BROWSER_TOKEN}");
    let person = [
        ("cookie", cookie.as_str()),
        ("sec-fetch-site", "same-origin"),
        ("sec-fetch-mode", "cors"),
        ("sec-fetch-dest", "empty"),
    ];
    let mut headers = HeaderMap::new();
    for (name, value) in person {
        headers.insert(name, value.parse().unwrap());
    }
    assert_eq!(
        auth::user_action_proof(&headers),
        UserActionProof::NoKeyInstalled
    );
    assert!(auth::served_person_present(&headers));
    exercise(&daemon().await, &person).await;
}
