//! `/dictation/*` authority on a daemon that holds a user action key, as the desktop's
//! own daemon does (DD-20, DD-29, O-17, P-7):
//!
//! * installing, selecting and deleting a model need the proof of a person, and a caller
//!   without it is refused with MSG-INSTALL-AGENT;
//! * `GET /dictation/status` tells that caller `install: {allowed: false, reason}`, so no
//!   interface offers a button that would fail;
//! * streams are admitted on the daemon secret alone (O-17, decided by the owner on
//!   2026-11-05), and the proof decides only whether private entries may bias.
//!
//! Its own binary, because the installed digest is a process global. No test here
//! reaches the network: an admitted install talks to a proxy on a closed loopback port.

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use biorouter_server::routes::dictation::{self, MSG_INSTALL_AGENT};
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use tower::ServiceExt;

const SECRET: &str = "dictation-keyed-secret";
const USER_KEY: &str = "dictation-keyed-user-action-key";

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/whisper.cpp/source")
        .join(relative)
}

/// The offline worker and model, and a dead proxy for every download (a closed port on
/// loopback answers at once, so an admitted install fails without leaving the machine).
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
    let digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(USER_KEY.as_bytes()).into();
    biorouter_server::auth::install_user_action_digest(Some(digest));
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
    /// A key that is not the one this daemon holds the digest of.
    WrongKey,
    /// A private provider stated without the proof: standing, never authority.
    StatedPrivate,
    Proof,
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    caller: Caller,
    body: Value,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Secret-Key", SECRET)
        .header("content-type", "application/json");
    builder = match caller {
        Caller::SecretOnly => builder,
        Caller::WrongKey => builder.header("X-User-Action", "not-the-desktop-key"),
        Caller::StatedPrivate => builder.header("X-Caller-Provider", "versa_azure"),
        Caller::Proof => builder.header("X-User-Action", USER_KEY),
    };
    let body = if body.is_null() {
        Body::empty()
    } else {
        Body::from(body.to_string())
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

const MODEL: &str = "whisper-small.en-q8_0";

/// P-7: without the proof, install, select and delete are refused with
/// MSG-INSTALL-AGENT, whatever else the caller states, and the status says so first.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn changing_the_models_needs_the_proof() {
    let (_state, app) = daemon().await;
    for caller in [Caller::SecretOnly, Caller::WrongKey, Caller::StatedPrivate] {
        for (uri, body) in [
            ("/dictation/models/install", json!({"model": MODEL})),
            ("/dictation/models/select", json!({"model": MODEL})),
            ("/dictation/models/select", json!({"language": "en"})),
            ("/dictation/models/delete", json!({"model": MODEL})),
        ] {
            let (status, answer) = call(&app, "POST", uri, caller, body.clone()).await;
            if !engine_runs() && uri.ends_with("/install") {
                // MSG-NO-ENGINE comes first in a build without the engine.
                assert_eq!(status, StatusCode::CONFLICT, "{caller:?} {uri}");
                continue;
            }
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?} {uri} {body}");
            assert_eq!(answer["code"], "proofRequired");
            assert_eq!(answer["message"], MSG_INSTALL_AGENT);
        }
        let (_, status) = call(&app, "GET", "/dictation/status", caller, Value::Null).await;
        assert_eq!(status["install"]["allowed"], false, "{caller:?}");
        if engine_runs() {
            assert_eq!(status["install"]["reason"], MSG_INSTALL_AGENT, "{caller:?}");
        }
    }
    let (_, status) = call(&app, "GET", "/dictation/status", Caller::Proof, Value::Null).await;
    assert_eq!(status["install"]["allowed"], engine_runs(), "{status}");
    assert_eq!(status["policy"]["install"]["allowed"], true);
}

/// With the proof: select and delete are admitted (a model that is not installed is not
/// selectable, and deleting one is a no-op), and an install starts. The proxy refuses
/// the download, which the install job reports.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_person_changes_the_models() {
    let (state, app) = daemon().await;
    let (status, answer) = call(
        &app,
        "POST",
        "/dictation/models/select",
        Caller::Proof,
        json!({"language": "en"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["language"], "en");
    let (status, answer) = call(
        &app,
        "POST",
        "/dictation/models/select",
        Caller::Proof,
        json!({"model": MODEL}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{answer}");
    assert_eq!(answer["code"], "noModel");
    let (status, answer) = call(
        &app,
        "POST",
        "/dictation/models/delete",
        Caller::Proof,
        json!({"model": MODEL}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["removed"], json!([]));

    if !engine_runs() {
        return;
    }
    let (status, answer) = call(
        &app,
        "POST",
        "/dictation/models/install",
        Caller::Proof,
        json!({"model": MODEL}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{answer}");
    assert_eq!(answer, json!({"model": MODEL, "phase": "downloading"}));
    let (status, busy) = call(
        &app,
        "POST",
        "/dictation/models/install",
        Caller::Proof,
        json!({"model": "whisper-large-v3-turbo-q5_0"}),
    )
    .await;
    if status == StatusCode::CONFLICT {
        assert_eq!(busy["code"], "installBusy");
    }
    // The dead proxy is a computer that cannot reach the repository: the job ends with
    // MSG-OFFLINE for this account's folder, the sentence every other surface shows.
    let job = failed_job(&app).await;
    assert_eq!(
        job["error"],
        biorouter::dictation::msg_offline(&biorouter::dictation::paths::user_models_dir()),
        "{job}"
    );

    // A running install is cancelled on the secret.
    let (status, answer) = call(
        &app,
        "POST",
        "/dictation/models/install",
        Caller::Proof,
        json!({"model": MODEL}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{answer}");
    let (status, _) = call(
        &app,
        "POST",
        "/dictation/models/cancel",
        Caller::SecretOnly,
        json!({"model": MODEL}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let job = failed_job(&app).await;
    assert!(job["error"].is_string(), "{job}");
    drop(state);
}

/// The install job, once it failed.
async fn failed_job(app: &Router) -> Value {
    let started = Instant::now();
    loop {
        let (_, status) = call(app, "GET", "/dictation/status", Caller::Proof, Value::Null).await;
        let job = status["install_job"].clone();
        assert_eq!(job["model"], MODEL);
        if job["phase"] == "failed" {
            return job;
        }
        assert!(
            started.elapsed() < Duration::from_secs(90),
            "the install never stopped: {job}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// O-17: a stream is admitted on the secret alone; the proof decides only private bias.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn streams_are_admitted_on_the_secret_alone() {
    if !engine_runs() {
        return;
    }
    let (_state, app) = daemon().await;
    for (caller, scope) in [(Caller::SecretOnly, "public"), (Caller::Proof, "private")] {
        let (status, created) = call(
            &app,
            "POST",
            "/dictation/streams",
            caller,
            json!({"sample_rate": 16000, "format": "s16le", "target_kind": "external",
                   "include_private_terms": true}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{caller:?} {created}");
        let id = created["stream_id"].as_str().unwrap().to_string();
        let audio: Vec<u8> = (0..16_000i32)
            .flat_map(|i| (((i % 16) - 8) as i16).to_le_bytes())
            .collect();
        let request = Request::builder()
            .method("POST")
            .uri(format!("/dictation/streams/{id}/finish?seq=0"))
            .header("X-Secret-Key", SECRET)
            .header("content-type", "application/octet-stream")
            .body(Body::from(audio))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let mut status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let mut transcript: Value = serde_json::from_slice(&bytes).unwrap();
        // A loaded machine can take more than `finish`'s 3 s; then a held read.
        while status == StatusCode::ACCEPTED {
            (status, transcript) = call(
                &app,
                "GET",
                &format!("/dictation/streams/{id}?wait_ms=3000"),
                Caller::SecretOnly,
                Value::Null,
            )
            .await;
        }
        assert_eq!(status, StatusCode::OK, "{transcript}");
        assert_eq!(transcript["bias_scope"], scope, "{caller:?}");
        call(
            &app,
            "DELETE",
            &format!("/dictation/streams/{id}"),
            Caller::SecretOnly,
            Value::Null,
        )
        .await;
    }
}
