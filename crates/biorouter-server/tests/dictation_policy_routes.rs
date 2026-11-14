//! The administrator's `dictation:` block at the `/dictation/*` doors (DD-90, P-15): every
//! key only narrows, every refusal carries the policy's own sentence, deleting a model is
//! never refused, and an unknown key turns dictation off with one warning that names it.
//!
//! The policy is staged where the daemon reads it under this binary's sandbox root
//! (`<root>/managed/managed-policy.yaml`, with the modes the trust check requires), so
//! Unix only.
#![cfg(unix)]

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use biorouter::managed::{MANAGED_DICTATION_CAPTURE_OFF, MANAGED_DICTATION_OFF};
use biorouter_server::routes::dictation;
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use tower::ServiceExt;

const SECRET: &str = "dictation-policy-secret";

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
}

fn engine_runs() -> bool {
    biorouter::dictation::service::spawn_refusal().is_none()
}

fn root() -> PathBuf {
    PathBuf::from(std::env::var("BIOROUTER_PATH_ROOT").expect("the sandbox root"))
}

/// Writes `yaml` as the managed policy, trusted: 0644 in a 0755 folder.
fn stage(yaml: &str) {
    let dir = root().join("managed");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let file = dir.join("managed-policy.yaml");
    std::fs::write(&file, yaml).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
}

fn unstage() {
    let _ = std::fs::remove_file(root().join("managed").join("managed-policy.yaml"));
}

/// Every warning logged, for the unknown key test.
fn warnings() -> &'static Arc<Mutex<Vec<u8>>> {
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&buffer);
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .with_writer(move || Sink(Arc::clone(&sink)))
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
        buffer
    })
}

struct Sink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

async fn daemon() -> Router {
    let _ = warnings();
    let state: Arc<AppState> = AppState::new().await.expect("app state");
    dictation::routes(state).layer(axum::middleware::from_fn_with_state(
        SECRET.to_string(),
        biorouter_server::auth::check_token,
    ))
}

async fn send(app: &Router, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Secret-Key", SECRET)
        .header("content-type", "application/json");
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

fn stream() -> Value {
    json!({"sample_rate": 16000, "format": "s16le", "target_kind": "external"})
}

const SMALL: &str = "whisper-small.en-q8_0";
const TURBO: &str = "whisper-large-v3-turbo-q5_0";

/// The `data` chunk of a 16 kHz mono 16 bit WAV file.
fn wav_samples(path: &Path) -> Vec<u8> {
    let wav = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut at = 12;
    while at + 8 <= wav.len() {
        let size = u32::from_le_bytes(wav[at + 4..at + 8].try_into().unwrap()) as usize;
        if &wav[at..at + 4] == b"data" {
            return wav[at + 8..(at + 8 + size).min(wav.len())].to_vec();
        }
        at += 8 + size + (size & 1);
    }
    panic!("{} has no data chunk", path.display())
}

/// A catalog file under a name nobody gave it, in this account's models folder: the
/// vendored detector fixture, byte identical to `silero-vad-v6.2.0`. Answers the stray
/// and where a rescan that placed it would have put it.
fn stray_detector() -> (PathBuf, PathBuf) {
    use biorouter::dictation::catalog::{CATALOG, VAD_ID};
    let folder = biorouter::dictation::paths::user_models_dir();
    std::fs::create_dir_all(&folder).unwrap();
    let stray = folder.join("a-detector-someone-copied.bin");
    std::fs::copy(fixture("models/for-tests-silero-v6.2.0-ggml.bin"), &stray).unwrap();
    let placed = biorouter::dictation::models::model_path(&folder, CATALOG.entry(VAD_ID).unwrap());
    assert!(!placed.exists(), "{}", placed.display());
    (stray, placed)
}

fn assert_refused(answer: &(StatusCode, Value), sentence: &str, what: &str) {
    assert_eq!(answer.0, StatusCode::FORBIDDEN, "{what}: {}", answer.1);
    assert_eq!(answer.1["code"], "policy", "{what}");
    assert_eq!(answer.1["message"], sentence, "{what}");
}

/// Every `policy` entry in the status is `{allowed, reason}`, the reason the sentence to
/// show and `null` when allowed.
fn assert_allowances(status: &Value) {
    for key in [
        "enabled",
        "capture",
        "install",
        "external_engine",
        "dictionary",
        "agent_writes",
    ] {
        let entry = &status["policy"][key];
        let fields: Vec<&String> = entry.as_object().unwrap().keys().collect();
        assert_eq!(fields, ["allowed", "reason"], "{key}: {entry}");
        assert_eq!(
            entry["allowed"].as_bool().unwrap(),
            entry["reason"].is_null(),
            "{key}"
        );
    }
}

/// `enabled: false` refuses streams, install, select and rescan with MSG-POLICY, and a
/// refused rescan moves nothing; delete is never refused.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn dictation_off_closes_every_door_but_delete() {
    stage("dictation:\n  enabled: false\n");
    let app = daemon().await;
    let (stray, placed) = stray_detector();
    let refused = send(&app, "POST", "/dictation/models/rescan", Value::Null).await;
    assert_refused(&refused, MANAGED_DICTATION_OFF, "rescan");
    assert!(!placed.exists(), "a refused rescan placed the stray");
    std::fs::remove_file(&stray).unwrap();
    if engine_runs() {
        let refused = send(&app, "POST", "/dictation/streams", stream()).await;
        assert_refused(&refused, MANAGED_DICTATION_OFF, "stream");
        let refused = send(
            &app,
            "POST",
            "/dictation/models/install",
            json!({"model": SMALL}),
        )
        .await;
        assert_refused(&refused, MANAGED_DICTATION_OFF, "install");
    }
    for body in [json!({"model": SMALL}), json!({"language": "en"})] {
        let refused = send(&app, "POST", "/dictation/models/select", body.clone()).await;
        assert_refused(&refused, MANAGED_DICTATION_OFF, &format!("select {body}"));
    }
    let deleted = send(
        &app,
        "POST",
        "/dictation/models/delete",
        json!({"model": SMALL}),
    )
    .await;
    assert_eq!(deleted.0, StatusCode::OK, "{}", deleted.1);
    let (_, status) = send(&app, "GET", "/dictation/status", Value::Null).await;
    assert_allowances(&status);
    assert_eq!(status["policy"]["managed"], true);
    assert_eq!(status["policy"]["enabled"]["reason"], MANAGED_DICTATION_OFF);
    assert_eq!(status["install"]["allowed"], false);
    unstage();
}

/// `capture: false` refuses live streams with MSG-POLICY-CAPTURE and nothing else.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn capture_off_refuses_live_streams_only() {
    stage("dictation:\n  capture: false\n");
    let app = daemon().await;
    if engine_runs() {
        let refused = send(&app, "POST", "/dictation/streams", stream()).await;
        assert_refused(&refused, MANAGED_DICTATION_CAPTURE_OFF, "stream");
    }
    let selected = send(
        &app,
        "POST",
        "/dictation/models/select",
        json!({"language": "en"}),
    )
    .await;
    assert_eq!(selected.0, StatusCode::OK, "{}", selected.1);
    let (_, status) = send(&app, "GET", "/dictation/status", Value::Null).await;
    assert_allowances(&status);
    assert_eq!(
        status["policy"]["capture"]["reason"],
        MANAGED_DICTATION_CAPTURE_OFF
    );
    assert_eq!(status["policy"]["enabled"]["allowed"], true);
    unstage();
}

/// `model_downloads: false` with a `models_dir` refuses installs with MSG-POLICY-DOWNLOADS
/// naming that folder, while streams stay open; a rescan identifies a catalog file and
/// places nothing (nor lists it as unrecognized). That a model in `models_dir` loads under
/// this key is the live arm below: a debug build's streams here load the `for-tests`
/// fixture, which `models_dir` is never asked for.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn downloads_off_refuses_installs_and_dictation_still_runs() {
    let models = root().join("org-models");
    std::fs::create_dir_all(&models).unwrap();
    stage(&format!(
        "dictation:\n  model_downloads: false\n  models_dir: {}\n",
        models.display()
    ));
    let app = daemon().await;
    let (stray, placed) = stray_detector();
    let (code, rescan) = send(&app, "POST", "/dictation/models/rescan", Value::Null).await;
    assert_eq!(code, StatusCode::OK, "{rescan}");
    assert_eq!(rescan, json!({"added": [], "unrecognized": []}));
    assert!(
        !placed.exists(),
        "a rescan under downloads off placed the stray"
    );
    std::fs::remove_file(&stray).unwrap();
    let (_, status) = send(&app, "GET", "/dictation/status", Value::Null).await;
    assert_allowances(&status);
    let sentence = status["policy"]["install"]["reason"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        sentence.contains(&models.display().to_string()),
        "{sentence}"
    );
    if engine_runs() {
        assert_eq!(status["install"]["reason"], sentence.as_str());
        let refused = send(
            &app,
            "POST",
            "/dictation/models/install",
            json!({"model": SMALL}),
        )
        .await;
        assert_refused(&refused, &sentence, "install");
        let (code, created) = send(&app, "POST", "/dictation/streams", stream()).await;
        assert_eq!(code, StatusCode::CREATED, "{created}");
        let id = created["stream_id"].as_str().unwrap();
        send(
            &app,
            "DELETE",
            &format!("/dictation/streams/{id}"),
            Value::Null,
        )
        .await;
    }
    unstage();
}

/// `allowed_models` refuses installing and selecting any other model with
/// MSG-POLICY-MODELS.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn allowed_models_refuses_the_others() {
    stage(&format!("dictation:\n  allowed_models: [{SMALL}]\n"));
    let app = daemon().await;
    let sentence = format!(
        "Your organization's managed policy allows only these speech models here: {SMALL}."
    );
    if engine_runs() {
        let refused = send(
            &app,
            "POST",
            "/dictation/models/install",
            json!({"model": TURBO}),
        )
        .await;
        assert_refused(&refused, &sentence, "install");
    }
    let refused = send(
        &app,
        "POST",
        "/dictation/models/select",
        json!({"model": TURBO}),
    )
    .await;
    assert_refused(&refused, &sentence, "select");
    let (_, status) = send(&app, "GET", "/dictation/status", Value::Null).await;
    assert_allowances(&status);
    assert_eq!(status["policy"]["allowed_models"], json!([SMALL]));
    unstage();
}

/// Live (`--ignored`): under `model_downloads: false`, the speech model and detector an
/// administrator put in `models_dir` load, and a stream into them answers text.
/// `BIOROUTER_DICTATION_LIVE_MODELS` names a folder holding the catalog's files under
/// their release names (the model cache). The debug only fixture is switched off, so the
/// stream resolves its model the way a release build does.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
#[serial]
async fn downloads_off_loads_the_models_in_models_dir() {
    use biorouter::dictation::catalog::{CATALOG, VAD_ID};
    use biorouter::dictation::service;

    let source = PathBuf::from(
        std::env::var("BIOROUTER_DICTATION_LIVE_MODELS")
            .expect("BIOROUTER_DICTATION_LIVE_MODELS names the model folder"),
    );
    let models = root().join("org-models-live");
    std::fs::create_dir_all(&models).unwrap();
    for id in [SMALL, VAD_ID] {
        let entry = CATALOG.entry(id).unwrap();
        let (from, to) = (source.join(entry.file_name), models.join(entry.file_name));
        if !to.exists() && std::fs::hard_link(&from, &to).is_err() {
            std::fs::copy(&from, &to).unwrap();
        }
    }
    stage(&format!(
        "dictation:\n  model_downloads: false\n  models_dir: {}\n",
        models.display()
    ));
    let _env = env_lock::lock_env([(service::TEST_MODEL_ENV, None::<&str>)]);
    biorouter::config::Config::global()
        .set_param(service::MODEL_KEY, SMALL)
        .unwrap();
    service::service().unload().await;
    let app = daemon().await;

    let (code, created) = send(&app, "POST", "/dictation/streams", stream()).await;
    assert_eq!(code, StatusCode::CREATED, "{created}");
    let id = created["stream_id"].as_str().unwrap().to_string();
    let request = Request::builder()
        .method("POST")
        .uri(format!("/dictation/streams/{id}/finish?seq=0"))
        .header("X-Secret-Key", SECRET)
        .header("content-type", "application/octet-stream")
        .body(Body::from(wav_samples(&fixture("samples/jfk.wav"))))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let mut code = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let mut transcript: Value = serde_json::from_slice(&bytes).unwrap();
    while code == StatusCode::ACCEPTED {
        (code, transcript) = send(
            &app,
            "GET",
            &format!("/dictation/streams/{id}?wait_ms=3000"),
            Value::Null,
        )
        .await;
    }
    assert_eq!(code, StatusCode::OK, "{transcript}");
    assert_eq!(transcript["model"], SMALL);
    let text = transcript["text"].as_str().unwrap();
    assert!(text.to_lowercase().contains("country"), "{text}");
    send(
        &app,
        "DELETE",
        &format!("/dictation/streams/{id}"),
        Value::Null,
    )
    .await;
    service::service().unload().await;
    let _ = biorouter::config::Config::global().delete(service::MODEL_KEY);
    unstage();
    let _ = std::fs::remove_dir_all(&models);
}

/// O-27: an unknown key turns dictation off, with one warning naming it.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn an_unknown_key_turns_dictation_off_with_one_warning() {
    let app = daemon().await;
    let logged = warnings().lock().unwrap().len();
    stage("dictation:\n  enabled: true\n  a_newer_key: true\n");
    let (_, status) = send(&app, "GET", "/dictation/status", Value::Null).await;
    assert_allowances(&status);
    assert_eq!(status["policy"]["enabled"]["reason"], MANAGED_DICTATION_OFF);
    let logs = String::from_utf8_lossy(&warnings().lock().unwrap()[logged..]).into_owned();
    let naming = logs.lines().filter(|l| l.contains("a_newer_key")).count();
    assert_eq!(naming, 1, "one warning per read names the key: {logs}");
    if engine_runs() {
        let refused = send(&app, "POST", "/dictation/streams", stream()).await;
        assert_refused(&refused, MANAGED_DICTATION_OFF, "stream");
    }
    unstage();
}
