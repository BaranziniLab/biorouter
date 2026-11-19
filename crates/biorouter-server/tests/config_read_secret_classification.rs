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

const UNKNOWN_ENV_KEY: &str = "CONFIG_READ_OPAQUE_FIXTURE_7C6B";
const UNKNOWN_ENV_VALUE: &str = "UNKN-synthetic-unclassified-environment-only-ENDU";

async fn exercise_unknown_environment(app: &Router, person: &[(&str, &str)]) {
    let metadata = biorouter::providers::providers().await;
    assert!(metadata.into_iter().all(|(provider, _)| provider
        .config_keys
        .iter()
        .all(|key| !key.name.eq_ignore_ascii_case(UNKNOWN_ENV_KEY))));
    assert!(!Config::global()
        .all_values()
        .unwrap()
        .contains_key(UNKNOWN_ENV_KEY));
    assert!(!Config::global()
        .all_secrets()
        .unwrap()
        .contains_key(UNKNOWN_ENV_KEY));
    let _env = env_lock::lock_env([
        (UNKNOWN_ENV_KEY, Some(UNKNOWN_ENV_VALUE)),
        ("GOOGLE_HOST", Some(HOST)),
    ]);
    let bearer = [("X-Secret-Key", DAEMON_SECRET)];
    let mut person_with_bearer = person.to_vec();
    person_with_bearer.push(("X-Secret-Key", DAEMON_SECRET));
    let model = [
        ("X-Secret-Key", DAEMON_SECRET),
        ("X-Caller-Provider", "anthropic"),
    ];
    let (status, body) = read(app, "GOOGLE_HOST", false, &bearer).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap(), json!(HOST));
    for denied in [
        Vec::new(),
        vec![("X-Secret-Key", "synthetic-wrong-daemon-secret")],
        person.to_vec(),
    ] {
        let (status, body) = read(app, UNKNOWN_ENV_KEY, false, &denied).await;
        assert!(!body.contains(UNKNOWN_ENV_VALUE));
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let lower_key = UNKNOWN_ENV_KEY.to_ascii_lowercase();
    let mut observations = Vec::new();
    for (label, headers) in [
        ("daemon bearer", bearer.as_slice()),
        ("bearer plus person proof", person_with_bearer.as_slice()),
        ("bearer plus model caller", model.as_slice()),
    ] {
        for key in [UNKNOWN_ENV_KEY, lower_key.as_str()] {
            let (status, body) = read(app, key, false, headers).await;
            observations.push((label, key, status, body));
        }
    }
    for (label, key, status, body) in observations {
        for fragment in [UNKNOWN_ENV_VALUE, "UNKN", "ENDU"] {
            assert!(
                !body.contains(fragment),
                "{label} retrieved synthetic unclassified environment bytes under {key}: {body:?}"
            );
        }
        if let Ok(decoded) = serde_json::from_str::<Value>(&body) {
            let normalized = decoded.to_string();
            for fragment in [UNKNOWN_ENV_VALUE, "UNKN", "ENDU"] {
                assert!(
                    !normalized.contains(fragment),
                    "{label} retrieved decoded synthetic unclassified environment bytes under {key}: {body:?}"
                );
            }
        }
        if status == StatusCode::OK {
            let value: Value = serde_json::from_str(&body).unwrap();
            assert!(value.is_null() || value == json!({ "maskedValue": SECRET_MASK }));
        } else {
            assert!(
                status.is_client_error(),
                "unexpected status {status}: {body:?}"
            );
        }
    }
}

#[tokio::test]
#[serial]
async fn desktop_config_read_does_not_export_an_unclassified_environment_value() {
    if !test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let digest: [u8; 32] =
        <sha2::Sha256 as sha2::Digest>::digest(USER_ACTION_KEY.as_bytes()).into();
    auth::install_user_action_digest(Some(digest));
    let mut headers = HeaderMap::new();
    headers.insert("X-User-Action", USER_ACTION_KEY.parse().unwrap());
    assert_eq!(auth::user_action_proof(&headers), UserActionProof::Proven);
    exercise_unknown_environment(&daemon().await, &[("X-User-Action", USER_ACTION_KEY)]).await;
}

#[tokio::test]
#[serial]
async fn served_config_read_does_not_export_an_unclassified_environment_value() {
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
    exercise_unknown_environment(&daemon().await, &person).await;
}

mod classification_acceptance {
    use super::*;

    async fn core_acceptance_router_controls(app: &Router) {
        let bearer = [("X-Secret-Key", DAEMON_SECRET)];
        let (status, body) = read(app, "GOOGLE_HOST", false, &bearer).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(serde_json::from_str::<Value>(&body).unwrap(), json!(HOST));
        for headers in [
            Vec::new(),
            vec![("X-Secret-Key", "synthetic-core-control-wrong-daemon-secret")],
        ] {
            let (status, _) = read(app, "GOOGLE_HOST", false, &headers).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
    }

    fn core_acceptance_unclassified_hidden(
        key: &str,
        status: StatusCode,
        body: &str,
        fragments: &[&str],
    ) {
        for fragment in fragments {
            assert!(
                !body.contains(*fragment),
                "unclassified config key {key} exposed fixture fragment {fragment:?}: {body:?}"
            );
        }
        if let Ok(decoded) = serde_json::from_str::<Value>(body) {
            let normalized = decoded.to_string();
            for fragment in fragments {
                assert!(
                    !normalized.contains(*fragment),
                    "unclassified config key {key} exposed decoded fixture fragment {fragment:?}: {body:?}"
                );
            }
        }
        if status == StatusCode::OK {
            let value: Value = serde_json::from_str(body).unwrap();
            assert!(value.is_null() || value == json!({ "maskedValue": SECRET_MASK }));
        } else {
            assert!(
                status.is_client_error(),
                "unexpected status {status}: {body:?}"
            );
        }
    }

    #[tokio::test]
    #[serial]
    async fn core_plaintext_config_reads_preserve_saved_values_and_environment_precedence() {
        if !test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let app = daemon().await;
        let settings = [
            (
                "BIOROUTER_PROVIDER",
                json!("google"),
                "anthropic",
                json!("anthropic"),
            ),
            (
                "BIOROUTER_MODEL",
                json!("saved-core-model"),
                "environment-core-model",
                json!("environment-core-model"),
            ),
            ("BIOROUTER_MODE", json!("auto"), "approve", json!("approve")),
            ("BIOROUTER_MAX_TURNS", json!(17), "23", json!(23)),
            ("WORKSPACE_ANNOUNCE_ONLY", json!(false), "true", json!(true)),
            (
                "BIOROUTER_WORKSPACE_OPEN_SUBAGENT_TABS",
                json!(false),
                "true",
                json!(true),
            ),
            (
                "BIOROUTER_WORKER_PROVIDER",
                json!("google"),
                "anthropic",
                json!("anthropic"),
            ),
            (
                "BIOROUTER_WORKER_MODEL",
                json!("saved-core-worker-model"),
                "environment-core-worker-model",
                json!("environment-core-worker-model"),
            ),
            (
                "CREW_TASK_MODEL",
                json!({ "provider": "google", "model": "saved-core-crew-model" }),
                r#"{"provider":"anthropic","model":"environment-core-crew-model"}"#,
                json!({ "provider": "anthropic", "model": "environment-core-crew-model" }),
            ),
            (
                "BIOROUTER_ONBOARDING_SKIPPED",
                json!(false),
                "true",
                json!(true),
            ),
            (
                "BIOROUTER_AUTO_COMPACT_THRESHOLD",
                json!(0.7),
                "0.82",
                json!(0.82),
            ),
            (
                "BIOROUTER_AUTO_COMPACT_MAX_TOKENS",
                json!(8192),
                "16384",
                json!(16384),
            ),
        ];
        let bearer = [("X-Secret-Key", DAEMON_SECRET)];
        {
            let mut environment: Vec<(&str, Option<&str>)> =
                settings.iter().map(|(key, _, _, _)| (*key, None)).collect();
            environment.push(("GOOGLE_HOST", Some(HOST)));
            let _env = env_lock::lock_env(environment);
            let saved: Vec<(&str, Value)> = settings
                .iter()
                .map(|(key, value, _, _)| (*key, value.clone()))
                .collect();
            Config::global().set_params(&saved).unwrap();
            core_acceptance_router_controls(&app).await;
            for (key, expected, _, _) in &settings {
                let (status, body) = read(&app, key, false, &bearer).await;
                assert_eq!(status, StatusCode::OK, "saved {key}: {body}");
                assert_eq!(
                    serde_json::from_str::<Value>(&body).unwrap(),
                    *expected,
                    "saved plaintext value changed for {key}"
                );
            }
            let (status, body) = read(&app, "biorouter_model", false, &bearer).await;
            assert_eq!(status, StatusCode::OK);
            assert!(
                serde_json::from_str::<Value>(&body).unwrap().is_null(),
                "case-normalized classification changed exact saved-file lookup: {body}"
            );
        }
        {
            let mut environment: Vec<(&str, Option<&str>)> = settings
                .iter()
                .map(|(key, _, value, _)| (*key, Some(*value)))
                .collect();
            environment.push(("GOOGLE_HOST", Some(HOST)));
            let _env = env_lock::lock_env(environment);
            for (key, _, _, expected) in &settings {
                let (status, body) = read(&app, key, false, &bearer).await;
                assert_eq!(status, StatusCode::OK, "environment {key}: {body}");
                assert_eq!(
                    serde_json::from_str::<Value>(&body).unwrap(),
                    *expected,
                    "environment did not win for allowed plaintext key {key}"
                );
            }
        }
    }

    #[tokio::test]
    #[serial]
    async fn config_read_does_not_export_an_unclassified_saved_parameter() {
        if !test_sandbox::in_a_process_of_its_own() {
            return;
        }
        const KEY: &str = "CONFIG_READ_SAVED_OPAQUE_FIXTURE_B468";
        const VALUE: &str = "SSAV-synthetic-unclassified-saved-only-ENDSAV";
        let app = daemon().await;
        let _env = env_lock::lock_env([(KEY, None::<&str>), ("GOOGLE_HOST", Some(HOST))]);
        assert!(biorouter::providers::providers()
            .await
            .into_iter()
            .all(|(provider, _)| {
                provider
                    .config_keys
                    .iter()
                    .all(|field| !field.name.eq_ignore_ascii_case(KEY))
            }));
        Config::global().set_param(KEY, VALUE).unwrap();
        assert_eq!(Config::global().get_param::<String>(KEY).unwrap(), VALUE);
        core_acceptance_router_controls(&app).await;
        let bearer = [("X-Secret-Key", DAEMON_SECRET)];
        let (status, body) = read(&app, KEY, false, &bearer).await;
        core_acceptance_unclassified_hidden(KEY, status, &body, &[VALUE, "SSAV", "ENDSAV"]);
    }

    #[tokio::test]
    #[serial]
    async fn config_read_does_not_export_an_unclassified_task_override() {
        if !test_sandbox::in_a_process_of_its_own() {
            return;
        }
        const KEY: &str = "CONFIG_READ_OVERRIDE_OPAQUE_FIXTURE_C7D2";
        const SAVED_VALUE: &str = "OSAV-synthetic-unclassified-saved-shadow-EOSAV";
        const ENV_VALUE: &str = "EOVR-synthetic-unclassified-env-shadow-ENDEV";
        const OVERRIDE_VALUE: &str = "TOVR-synthetic-unclassified-task-only-ENDOVR";
        const OVERRIDE_HOST: &str = "https://task-override-host.fixture.invalid";
        let app = daemon().await;
        let _env = env_lock::lock_env([(KEY, Some(ENV_VALUE)), ("GOOGLE_HOST", Some(HOST))]);
        assert!(biorouter::providers::providers()
            .await
            .into_iter()
            .all(|(provider, _)| {
                provider
                    .config_keys
                    .iter()
                    .all(|field| !field.name.eq_ignore_ascii_case(KEY))
            }));
        Config::global().set_param(KEY, SAVED_VALUE).unwrap();
        assert_eq!(
            Config::global().get_param::<String>(KEY).unwrap(),
            ENV_VALUE
        );
        core_acceptance_router_controls(&app).await;
        let bearer = [("X-Secret-Key", DAEMON_SECRET)];
        let overrides = std::collections::HashMap::from([
            (KEY.to_string(), OVERRIDE_VALUE.to_string()),
            ("GOOGLE_HOST".to_string(), OVERRIDE_HOST.to_string()),
        ]);
        let (status, body) = biorouter::config::with_config_overrides(overrides, async {
            assert_eq!(
                Config::global().get_param::<String>(KEY).unwrap(),
                OVERRIDE_VALUE
            );
            let (status, body) = read(&app, "GOOGLE_HOST", false, &bearer).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(
                serde_json::from_str::<Value>(&body).unwrap(),
                json!(OVERRIDE_HOST)
            );
            read(&app, KEY, false, &bearer).await
        })
        .await;
        assert_eq!(
            Config::global().get_param::<String>(KEY).unwrap(),
            ENV_VALUE
        );
        core_acceptance_unclassified_hidden(
            KEY,
            status,
            &body,
            &[
                OVERRIDE_VALUE,
                "TOVR",
                "ENDOVR",
                ENV_VALUE,
                "EOVR",
                "ENDEV",
                SAVED_VALUE,
                "OSAV",
                "EOSAV",
            ],
        );
    }

    const CLASSIFICATION_COLLISION_VALUE: &str = "COLL-synthetic-custom-provider-key-only-ENDC";

    struct ClassificationCollisionFile(std::path::PathBuf);

    impl Drop for ClassificationCollisionFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn classification_collision_file(id: &str, key: &str) -> ClassificationCollisionFile {
        assert!(test_sandbox::global_config_reads_secrets_from_a_file());
        assert!(biorouter::config::test_binary_credentials::os_store_is_kept_off());
        let root = std::path::PathBuf::from(std::env::var_os("BIOROUTER_PATH_ROOT").unwrap());
        let dir = biorouter::config::declarative_providers::custom_providers_dir();
        assert!(dir.starts_with(root));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{id}.json"));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let fixture = ClassificationCollisionFile(path);
        serde_json::to_writer(
            file,
            &json!({
                "name": id,
                "engine": "openai",
                "display_name": "Synthetic config classification collision",
                "api_key_env": key,
                "base_url": "https://collision.fixture.invalid/v1",
                "models": [],
            }),
        )
        .unwrap();
        fixture
    }

    fn classification_collision_verify_file_metadata(id: &str, key: &str) {
        let loaded = biorouter::config::declarative_providers::load_provider(id).unwrap();
        assert!(loaded.is_editable);
        assert_eq!(loaded.config.api_key_env, key);
        let mut registry = biorouter::providers::provider_registry::ProviderRegistry::new();
        biorouter::config::declarative_providers::register_declarative_provider(
            &mut registry,
            loaded.config,
            biorouter::providers::base::ProviderType::Custom,
        );
        assert!(registry
            .all_metadata_with_types()
            .into_iter()
            .any(|(metadata, _)| metadata.name == id
                && metadata
                    .config_keys
                    .iter()
                    .any(|entry| entry.name == key && entry.secret)));
    }

    async fn classification_collision_nonsecret_control(app: &Router, key: &str, value: &str) {
        let _env = env_lock::lock_env([(key, Some(value))]);
        let (status, body) = read(app, key, false, &[("X-Secret-Key", DAEMON_SECRET)]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(serde_json::from_str::<Value>(&body).unwrap(), json!(value));
    }

    fn classification_collision_no_credential(label: &str, key: &str, body: &str) {
        for fragment in [CLASSIFICATION_COLLISION_VALUE, "COLL", "ENDC"] {
            assert!(
                !body.contains(fragment),
                "classification collision exposed synthetic credential to {label} under {key}: {body:?}"
            );
        }
        if let Ok(decoded) = serde_json::from_str::<Value>(body) {
            let normalized = decoded.to_string();
            for fragment in [CLASSIFICATION_COLLISION_VALUE, "COLL", "ENDC"] {
                assert!(
                    !normalized.contains(fragment),
                    "classification collision exposed decoded synthetic credential to {label} under {key}: {body:?}"
                );
            }
        }
    }

    async fn classification_collision_http_observations(
        app: &Router,
        key: &str,
    ) -> Vec<(&'static str, String, StatusCode, String)> {
        let _env = env_lock::lock_env([(key, Some(CLASSIFICATION_COLLISION_VALUE))]);
        for (label, headers) in [
            ("missing bearer", Vec::new()),
            (
                "wrong bearer",
                vec![("X-Secret-Key", "synthetic-wrong-daemon-secret")],
            ),
            (
                "model label without bearer",
                vec![("X-Caller-Provider", "anthropic")],
            ),
        ] {
            let (status, body) = read(app, key, false, &headers).await;
            classification_collision_no_credential(label, key, &body);
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        let bearer = [("X-Secret-Key", DAEMON_SECRET)];
        let model = [
            ("X-Secret-Key", DAEMON_SECRET),
            ("X-Caller-Provider", "anthropic"),
        ];
        let lower_key = key.to_ascii_lowercase();
        let mut observations = Vec::new();
        for (label, headers) in [
            ("daemon bearer", bearer.as_slice()),
            ("bearer plus model caller", model.as_slice()),
        ] {
            for query_key in [key, lower_key.as_str()] {
                let (status, body) = read(app, query_key, false, headers).await;
                observations.push((label, query_key.to_string(), status, body));
            }
        }
        observations
    }

    fn classification_collision_assert_masked(
        observations: Vec<(&'static str, String, StatusCode, String)>,
    ) {
        for (label, key, status, body) in observations {
            classification_collision_no_credential(label, &key, &body);
            assert_eq!(status, StatusCode::OK, "{label} under {key}: {body:?}");
            assert_eq!(
                serde_json::from_str::<Value>(&body).unwrap(),
                json!({ "maskedValue": SECRET_MASK }),
                "{label} under {key}"
            );
        }
    }

    #[tokio::test]
    #[serial]
    async fn config_read_secret_metadata_wins_over_a_nonsecret_provider_declaration() {
        if !test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let app = daemon().await;
        let id = "classification_collision_provider";
        let key = "GOOGLE_HOST";
        let cached = biorouter::providers::providers().await;
        assert!(cached.iter().all(|(metadata, _)| metadata.name != id));
        assert!(cached.iter().any(|(metadata, _)| metadata.name == "google"
            && metadata
                .config_keys
                .iter()
                .any(|entry| entry.name == key && !entry.secret)));
        assert!(!cached.iter().any(|(metadata, _)| metadata
            .config_keys
            .iter()
            .any(|entry| entry.name.eq_ignore_ascii_case(key) && entry.secret)));
        classification_collision_nonsecret_control(&app, key, HOST).await;
        let _file = classification_collision_file(id, key);
        classification_collision_verify_file_metadata(id, key);
        biorouter::providers::refresh_custom_providers()
            .await
            .unwrap();
        let metadata = biorouter::providers::providers().await;
        assert!(metadata
            .iter()
            .any(|(provider, _)| provider.name == "google"
                && provider
                    .config_keys
                    .iter()
                    .any(|entry| entry.name == key && !entry.secret)));
        assert!(metadata.iter().any(|(provider, _)| provider.name == id
            && provider
                .config_keys
                .iter()
                .any(|entry| entry.name == key && entry.secret)));
        let observations = classification_collision_http_observations(&app, key).await;
        classification_collision_assert_masked(observations);
    }

    #[tokio::test]
    #[serial]
    async fn config_read_fresh_custom_secret_overrides_a_cached_core_nonsecret_key() {
        if !test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let app = daemon().await;
        let id = "classification_collision_fresh_core";
        let key = "BIOROUTER_MODEL";
        let cached = biorouter::providers::providers().await;
        assert!(cached.iter().all(|(metadata, _)| metadata.name != id));
        assert!(!cached.iter().any(|(metadata, _)| metadata
            .config_keys
            .iter()
            .any(|entry| entry.name.eq_ignore_ascii_case(key) && entry.secret)));
        classification_collision_nonsecret_control(&app, key, "synthetic-control-model").await;
        let _file = classification_collision_file(id, key);
        classification_collision_verify_file_metadata(id, key);
        let still_cached = biorouter::providers::providers().await;
        assert!(still_cached.iter().all(|(metadata, _)| metadata.name != id));
        assert!(!still_cached.iter().any(|(metadata, _)| metadata
            .config_keys
            .iter()
            .any(|entry| entry.name.eq_ignore_ascii_case(key) && entry.secret)));
        let observations = classification_collision_http_observations(&app, key).await;
        classification_collision_assert_masked(observations);
    }

    const EXT_ENV_VALUE: &str = "EXTC-synthetic-config-reference-environment-ENDX";
    const EXT_BAD_METADATA: &str = "MPJF-synthetic-malformed-present-json-ENDJ";

    fn ext_disabled_entry() -> Value {
        let entry = biorouter::config::ExtensionEntry {
            enabled: false,
            config: biorouter::agents::ExtensionConfig::Stdio {
                name: "config_read_ext_fixture".to_string(),
                description: "Synthetic config-read reference".to_string(),
                cmd: "must-not-execute-config-read-fixture".to_string(),
                args: Vec::new(),
                envs: Default::default(),
                env_keys: vec!["GOOGLE_HOST".to_string()],
                timeout: Some(30),
                bundled: Some(false),
                available_tools: Vec::new(),
            },
        };
        serde_json::to_value(entry).unwrap()
    }

    fn ext_write_metadata(extensions: &Value) -> Value {
        Config::global()
            .set("extensions", extensions, false)
            .unwrap();
        let bytes = std::fs::read_to_string(Config::global().path()).unwrap();
        let saved: Value = serde_yaml::from_str(&bytes).unwrap();
        let section = saved.get("extensions").expect("present extension metadata");
        assert_eq!(section, extensions);
        section.clone()
    }

    async fn ext_assert_plaintext_host(app: &Router, headers: &[(&str, &str)]) {
        let (status, body) = read(app, "GOOGLE_HOST", false, headers).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap(),
            json!(EXT_ENV_VALUE)
        );
    }

    fn ext_assert_no_reference_bytes(label: &str, body: &str) {
        let normalized = serde_json::from_str::<Value>(body)
            .ok()
            .map(|value| value.to_string());
        for candidate in [Some(body), normalized.as_deref()].into_iter().flatten() {
            for fragment in [
                EXT_ENV_VALUE,
                "EXTC",
                "ENDX",
                EXT_BAD_METADATA,
                "MPJF",
                "ENDJ",
            ] {
                assert!(
                    !candidate.contains(fragment),
                    "{label} exported synthetic reference bytes despite metadata: {body:?}"
                );
            }
        }
    }

    #[tokio::test]
    #[serial]
    async fn disabled_extension_reference_masks_an_otherwise_plaintext_field() {
        if !test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let app = daemon().await;
        let _env = env_lock::lock_env([("GOOGLE_HOST", Some(EXT_ENV_VALUE))]);
        let bearer = [("X-Secret-Key", DAEMON_SECRET)];
        ext_write_metadata(&json!({}));
        ext_assert_plaintext_host(&app, &bearer).await;

        let saved = ext_write_metadata(&json!({ "config_read_ext_fixture": ext_disabled_entry() }));
        let entry: biorouter::config::ExtensionEntry =
            serde_json::from_value(saved["config_read_ext_fixture"].clone()).unwrap();
        assert!(!entry.enabled);
        assert!(matches!(
            entry.config,
            biorouter::agents::ExtensionConfig::Stdio { env_keys, .. }
                if env_keys == vec!["GOOGLE_HOST".to_string()]
        ));

        let mut observations = Vec::new();
        for key in ["GOOGLE_HOST", "google_host"] {
            let (status, body) = read(&app, key, false, &bearer).await;
            observations.push((key, status, body));
        }
        for (key, status, body) in observations {
            ext_assert_no_reference_bytes(key, &body);
            assert_eq!(status, StatusCode::OK);
            assert_eq!(
                serde_json::from_str::<Value>(&body).unwrap(),
                json!({ "maskedValue": SECRET_MASK })
            );
        }
    }

    #[tokio::test]
    #[serial]
    async fn malformed_present_reference_metadata_does_not_restore_plaintext_access() {
        if !test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let app = daemon().await;
        let _env = env_lock::lock_env([("GOOGLE_HOST", Some(EXT_ENV_VALUE))]);
        let bearer = [("X-Secret-Key", DAEMON_SECRET)];
        ext_write_metadata(&json!({}));
        let _ = biorouter::providers::providers().await;
        ext_assert_plaintext_host(&app, &bearer).await;

        let mut malformed_entry = ext_disabled_entry();
        malformed_entry["env_keys"] = json!([42]);
        let saved = ext_write_metadata(&json!({ "config_read_ext_fixture": malformed_entry }));
        assert!(serde_json::from_value::<biorouter::config::ExtensionEntry>(
            saved["config_read_ext_fixture"].clone()
        )
        .is_err());
        let (status, body) = read(&app, "GOOGLE_HOST", false, &bearer).await;
        let mut observations = vec![("malformed present extension env_keys", status, body)];

        let restored = ext_write_metadata(&json!({}));
        assert!(restored.as_object().unwrap().is_empty());
        let directory = biorouter::config::declarative_providers::custom_providers_dir();
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config-read-malformed-reference-fixture.json");
        let bytes = b"{\"fixture\":\"MPJF-synthetic-malformed-present-json-ENDJ\"";
        assert!(serde_json::from_slice::<Value>(bytes).is_err());
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        std::io::Write::write_all(&mut file, bytes).unwrap();
        drop(file);
        assert_eq!(std::fs::read(&path).unwrap(), bytes.to_vec());
        let (status, body) = read(&app, "GOOGLE_HOST", false, &bearer).await;
        observations.push(("malformed present custom provider JSON", status, body));
        std::fs::remove_file(&path).unwrap();
        assert!(!path.exists());
        ext_assert_plaintext_host(&app, &bearer).await;

        for (label, status, body) in observations {
            ext_assert_no_reference_bytes(label, &body);
            if status == StatusCode::OK {
                let value: Value = serde_json::from_str(&body).unwrap();
                assert!(value.is_null() || value == json!({ "maskedValue": SECRET_MASK }));
            } else {
                assert!(
                    status.is_client_error() || status.is_server_error(),
                    "unexpected metadata-fault status {status}: {body:?}"
                );
            }
        }
    }
}
