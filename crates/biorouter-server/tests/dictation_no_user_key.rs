//! `/dictation/*` on a daemon that holds no user action key, as `biorouter serve` starts
//! one (SD-7): installing a model is admitted on the secret unless the operator locked it
//! (`biorouter serve --no-dictation-install`, which sets
//! `BIOROUTER_DICTATION_BROWSER_INSTALL=false`, O-2); a private host's stated provider
//! gives private standing; and a served page whose only private input is the operator's
//! tier (`served_operator`, SD-10) dictates with public terms (DD-30).
//!
//! ⚠ Its own binary on purpose: the user action digest and the served operator are
//! process globals, and nothing here installs a digest, which is how `serve` starts its
//! daemon. No test here reaches the network: an admitted install talks to a proxy on a
//! closed loopback port.

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::Router;
use biorouter::model::ModelConfig;
use biorouter::privacy::{ProviderTier, SessionClassification};
use biorouter::providers::versa_azure::VERSA_AZURE_DEFAULT_MODEL;
use biorouter::session::SessionType;
use biorouter_server::auth::{user_action_proof, UserActionProof};
use biorouter_server::routes::dictation::{self, BROWSER_INSTALL_ENV, MSG_INSTALL_LOCKED};
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use tower::ServiceExt;

const SECRET: &str = "dictation-keyless-secret";
/// The browser token `biorouter serve` would have minted for this launch.
const BROWSER_TOKEN: &str = "4b8d0c2e6a1f3e5d7c9b0a2f4e6d8c1b";
const MODEL: &str = "whisper-small.en-q8_0";

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/whisper.cpp/source")
        .join(relative)
}

#[ctor::ctor]
fn offline() {
    std::env::set_var(
        biorouter::dictation::service::WORKER_ENV,
        env!("CARGO_BIN_EXE_biorouterd"),
    );
    std::env::set_var(
        biorouter::dictation::service::TEST_MODEL_ENV,
        fixture("models/for-tests-ggml-base.en.bin"),
    );
    for proxy in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] {
        std::env::set_var(proxy, "http://127.0.0.1:9");
    }
    for bypass in ["NO_PROXY", "no_proxy"] {
        std::env::remove_var(bypass);
    }
}

fn engine_runs() -> bool {
    biorouter::dictation::service::spawn_refusal().is_none()
}

async fn daemon() -> (Arc<AppState>, Router) {
    assert_eq!(
        user_action_proof(&HeaderMap::new()),
        UserActionProof::NoKeyInstalled,
        "something in this binary installed a user action digest"
    );
    let state = AppState::new().await.expect("app state");
    if engine_runs() {
        // The worker loaded on this test's runtime, as a daemon that dictated before
        // holds it (a fresh binary's first spawn can wait on the system's scan).
        let _ = biorouter::dictation::service::service().prepare(None).await;
    }
    let app = dictation::routes(Arc::clone(&state)).layer(axum::middleware::from_fn_with_state(
        SECRET.to_string(),
        biorouter_server::auth::check_token,
    ));
    (state, app)
}

#[derive(Clone, Copy, Debug)]
enum Caller {
    SecretOnly,
    /// `userActionHeaders()` on a `serve` page of a private host (SD-12).
    StatedPrivate,
    /// The served document's cookie and nothing else: the operator's tier only.
    ServedPage,
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    caller: Caller,
    body: Body,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Secret-Key", SECRET);
    builder = match caller {
        Caller::SecretOnly => builder,
        Caller::StatedPrivate => builder.header("X-Caller-Provider", "versa_azure"),
        Caller::ServedPage => {
            builder.header("cookie", format!("biorouter_session={BROWSER_TOKEN}"))
        }
    };
    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn post(app: &Router, uri: &str, caller: Caller, body: Value) -> (StatusCode, Value) {
    send(app, "POST", uri, caller, Body::from(body.to_string())).await
}

/// One dictation of near silence into `request`; answers its scope.
async fn scope(app: &Router, caller: Caller, request: Value) -> String {
    let (status, created) = post(app, "/dictation/streams", caller, request).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["stream_id"].as_str().unwrap().to_string();
    let audio: Vec<u8> = (0..16_000i32)
        .flat_map(|i| (((i % 16) - 8) as i16).to_le_bytes())
        .collect();
    let (mut status, mut transcript) = send(
        app,
        "POST",
        &format!("/dictation/streams/{id}/finish?seq=0"),
        Caller::SecretOnly,
        Body::from(audio),
    )
    .await;
    // A loaded machine can take more than `finish`'s 3 s; then a held read, as a client.
    while status == StatusCode::ACCEPTED {
        (status, transcript) = send(
            app,
            "GET",
            &format!("/dictation/streams/{id}?wait_ms=3000"),
            Caller::SecretOnly,
            Body::empty(),
        )
        .await;
    }
    assert_eq!(status, StatusCode::OK, "{transcript}");
    send(
        app,
        "DELETE",
        &format!("/dictation/streams/{id}"),
        Caller::SecretOnly,
        Body::empty(),
    )
    .await;
    transcript["bias_scope"].as_str().unwrap().to_string()
}

/// O-2: an install is admitted on the secret, and refused with MSG-INSTALL-LOCKED once
/// the operator locked it; the status says which before anyone presses a button. Select
/// and delete need nothing more than the secret here.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn installs_are_admitted_on_the_secret_unless_the_operator_locked_them() {
    let (_state, app) = daemon().await;
    let (status, answer) = post(
        &app,
        "/dictation/models/select",
        Caller::SecretOnly,
        json!({"language": "en"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let (status, answer) = post(
        &app,
        "/dictation/models/delete",
        Caller::SecretOnly,
        json!({"model": MODEL}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    if !engine_runs() {
        return;
    }

    let (_, status) = send(
        &app,
        "GET",
        "/dictation/status",
        Caller::SecretOnly,
        Body::empty(),
    )
    .await;
    assert_eq!(status["install"], json!({"allowed": true, "reason": null}));
    let (code, answer) = post(
        &app,
        "/dictation/models/install",
        Caller::SecretOnly,
        json!({"model": MODEL}),
    )
    .await;
    assert_eq!(code, StatusCode::ACCEPTED, "{answer}");
    // The dead proxy is a computer that cannot reach the repository: the job ends with
    // MSG-OFFLINE for this account's folder, as every other surface words it.
    let started = Instant::now();
    loop {
        let (_, status) = send(
            &app,
            "GET",
            "/dictation/status",
            Caller::SecretOnly,
            Body::empty(),
        )
        .await;
        let job = &status["install_job"];
        if job["phase"] == "failed" {
            assert_eq!(
                job["error"],
                biorouter::dictation::msg_offline(&biorouter::dictation::paths::user_models_dir()),
                "{job}"
            );
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(90), "{status}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let _locked = env_lock::lock_env([(BROWSER_INSTALL_ENV, Some("false"))]);
    let (_, status) = send(
        &app,
        "GET",
        "/dictation/status",
        Caller::SecretOnly,
        Body::empty(),
    )
    .await;
    assert_eq!(
        status["install"],
        json!({"allowed": false, "reason": MSG_INSTALL_LOCKED})
    );
    assert_eq!(
        status["policy"]["install"]["allowed"], true,
        "the operator lock is not managed policy"
    );
    let (code, answer) = post(
        &app,
        "/dictation/models/install",
        Caller::SecretOnly,
        json!({"model": MODEL}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    assert_eq!(answer["code"], "installLocked");
    assert_eq!(answer["message"], MSG_INSTALL_LOCKED);
}

fn versa() -> HashMap<String, String> {
    HashMap::from([
        ("BIOROUTER_PROVIDER".to_string(), "versa_azure".to_string()),
        (
            "BIOROUTER_MODEL".to_string(),
            VERSA_AZURE_DEFAULT_MODEL.to_string(),
        ),
        (
            "VERSA_AZURE_API_KEY".to_string(),
            "placeholder-never-sent".to_string(),
        ),
    ])
}

/// DD-30 on a keyless daemon: a private host's stated provider is private standing, so a
/// private chat it may reach and the private bind of a new chat give private scope; the
/// served document's cookie alone, whose only private input is the operator's tier, gives
/// public scope everywhere, and so does the secret alone.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn standing_is_the_stated_provider_never_the_operators_tier() {
    if !engine_runs() {
        return;
    }
    biorouter_server::auth::install_served_operator(
        BROWSER_TOKEN.to_string(),
        ProviderTier::Private,
    );
    let (state, app) = daemon().await;
    let manager = state.session_manager();
    let chat = manager
        .create_session(
            PathBuf::from("/tmp/dictation_no_user_key"),
            "dictation keyless target (test fixture)".to_string(),
            SessionType::User,
        )
        .await
        .unwrap();
    manager
        .update(&chat.id)
        .provider_name("versa_azure")
        .model_config(ModelConfig::new("gpt-4o").unwrap())
        .raise_privacy(SessionClassification::Private, "turn:versa_azure")
        .apply()
        .await
        .unwrap();
    let into_chat = json!({"sample_rate": 16000, "format": "s16le", "target_kind": "chat",
                           "session_id": chat.id});
    let new_chat = json!({"sample_rate": 16000, "format": "s16le", "target_kind": "new_chat"});

    assert_eq!(
        scope(&app, Caller::StatedPrivate, into_chat.clone()).await,
        "private"
    );
    assert_eq!(
        scope(&app, Caller::ServedPage, into_chat.clone()).await,
        "public"
    );
    assert_eq!(scope(&app, Caller::SecretOnly, into_chat).await, "public");
    for (caller, want) in [
        (Caller::StatedPrivate, "private"),
        (Caller::ServedPage, "public"),
        (Caller::SecretOnly, "public"),
    ] {
        // In the environment, not a task local override: the target is decided on a
        // task of its own after the 201, which an override does not reach.
        let vars: Vec<(String, String)> = versa().into_iter().collect();
        let got = {
            let _env = env_lock::lock_env(vars.iter().map(|(k, v)| (k.as_str(), Some(v))));
            scope(&app, caller, new_chat.clone()).await
        };
        assert_eq!(got, want, "new chat, {caller:?}");
    }
    // A person's inclusion needs the proof, which no caller of this daemon can hold.
    let mut crew = json!({"sample_rate": 16000, "format": "s16le", "target_kind": "crew"});
    crew["include_private_terms"] = json!(true);
    assert_eq!(scope(&app, Caller::StatedPrivate, crew).await, "public");
    manager.delete_session(&chat.id).await.unwrap();
}
