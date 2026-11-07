//! `/dictation/*` streams on a daemon that holds a user action key (PR-15, DD-27 to
//! DD-32), driven offline: the worker is this build's own `biorouterd`, and the model is
//! the vendored `for-tests` fixture, which loads, decodes and answers nothing (so every
//! transcript here is empty and `no_speech`).
//!
//! What is asserted, as `docs/dictation/testing-and-release.md` lists it for this binary:
//! the status codes of the stream routes, a repeated `seq` answered as before, the idle
//! expiry, the stream and queue limits with `Retry-After`, `finish` carrying the tail
//! chunk, a held read answering 202 then 200, a worker lost mid stream answering
//! `500 engineFailed` from `finish` and from a read, MSG-SERVER-MEMORY, nothing left on
//! disk or in a log line, and DD-30's target decision: a refused `session_id` and a
//! secret only caller give public scope and the same answer (P-4, P-6), the per tab
//! `new_chat_provider` counts only with the proof, a new chat is judged by the tier its
//! bind would produce, a chat bound to a private provider is private before its row
//! says so, and `include_private_terms` needs the proof and private standing.
//!
//! The tests share the process's one dictation service, so they run one at a time.

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::Router;
use biorouter::config::with_config_overrides;
use biorouter::model::ModelConfig;
use biorouter::privacy::SessionClassification;
use biorouter::providers::versa_azure::VERSA_AZURE_DEFAULT_MODEL;
use biorouter::security::dictionary_store::{open_store, store};
use biorouter::session::SessionType;
use biorouter_server::routes::dictation;
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use tower::ServiceExt;

const SECRET: &str = "dictation-routes-secret";
const USER_KEY: &str = "dictation-routes-user-action-key";

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/whisper.cpp/source")
        .join(relative)
}

/// The worker is this build's `biorouterd` (the test binary itself is not one), and the
/// model the debug only `for-tests` fixture. Set before `main`, so every spawn sees them.
#[ctor::ctor]
fn dictate_with_the_offline_fixtures() {
    std::env::set_var(
        biorouter::dictation::service::WORKER_ENV,
        env!("CARGO_BIN_EXE_biorouterd"),
    );
    std::env::set_var(
        biorouter::dictation::service::TEST_MODEL_ENV,
        fixture("models/for-tests-ggml-base.en.bin"),
    );
}

/// Whether this build and this processor can run the engine; a build without one answers
/// `409 notCompiled`, which `dictation_policy_routes.rs` covers.
fn engine_runs() -> bool {
    biorouter::dictation::service::spawn_refusal().is_none()
}

fn install_the_desktop_key() {
    let digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(USER_KEY.as_bytes()).into();
    biorouter_server::auth::install_user_action_digest(Some(digest));
}

/// Everything these tests log, for the log hygiene test (DD-93).
fn captured_logs() -> &'static Arc<Mutex<Vec<u8>>> {
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&buffer);
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || LogWriter(Arc::clone(&sink)))
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
        buffer
    })
}

struct LogWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The daemon, with its worker loaded on this test's runtime, as a daemon that has
/// dictated before holds it. The first spawn of a freshly built `biorouterd` can take
/// seconds on macOS while the system scans it, which is no stream's to wait for.
async fn state() -> Arc<AppState> {
    install_the_desktop_key();
    let _ = captured_logs();
    let state = AppState::new().await.expect("app state");
    if engine_runs() {
        biorouter::dictation::service::service()
            .prepare(None)
            .await
            .expect("the fixture loads");
    }
    state
}

/// The dictation routes behind the daemon's own secret check.
fn app(state: &Arc<AppState>) -> Router {
    dictation::routes(Arc::clone(state)).layer(axum::middleware::from_fn_with_state(
        SECRET.to_string(),
        biorouter_server::auth::check_token,
    ))
}

#[derive(Clone, Copy, Debug)]
enum Caller {
    /// The daemon secret and nothing else: an agent's shell, a script.
    SecretOnly,
    /// The desktop's proof of a person.
    Proof,
    /// A private provider stated in `X-Caller-Provider`, without the proof.
    StatedPrivate,
}

struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    raw: Bytes,
    body: Value,
}

async fn call(app: &Router, method: &str, uri: &str, caller: Caller, body: Body) -> Answer {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Secret-Key", SECRET);
    builder = match caller {
        Caller::SecretOnly => builder,
        Caller::Proof => builder.header("X-User-Action", USER_KEY),
        Caller::StatedPrivate => builder.header("X-Caller-Provider", "versa_azure"),
    };
    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let raw = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    Answer {
        status,
        headers,
        raw,
        body,
    }
}

async fn create(app: &Router, caller: Caller, request: Value) -> Answer {
    call(
        app,
        "POST",
        "/dictation/streams",
        caller,
        Body::from(request.to_string()),
    )
    .await
}

fn request(target_kind: &str) -> Value {
    json!({"sample_rate": 16000, "format": "s16le", "target_kind": target_kind})
}

async fn open(app: &Router, caller: Caller, request: Value) -> String {
    let answer = create(app, caller, request).await;
    assert_eq!(
        answer.status,
        StatusCode::CREATED,
        "{:?}",
        String::from_utf8_lossy(&answer.raw)
    );
    answer.body["stream_id"].as_str().unwrap().to_string()
}

async fn put(app: &Router, id: &str, seq: u64, bytes: Vec<u8>) -> Answer {
    call(
        app,
        "PUT",
        &format!("/dictation/streams/{id}/audio?seq={seq}"),
        Caller::SecretOnly,
        Body::from(bytes),
    )
    .await
}

async fn finish(app: &Router, id: &str) -> Answer {
    call(
        app,
        "POST",
        &format!("/dictation/streams/{id}/finish"),
        Caller::SecretOnly,
        Body::empty(),
    )
    .await
}

async fn finish_with(app: &Router, id: &str, seq: u64, bytes: Vec<u8>) -> Answer {
    call(
        app,
        "POST",
        &format!("/dictation/streams/{id}/finish?seq={seq}"),
        Caller::SecretOnly,
        Body::from(bytes),
    )
    .await
}

async fn read(app: &Router, id: &str, wait_ms: u64) -> Answer {
    call(
        app,
        "GET",
        &format!("/dictation/streams/{id}?wait_ms={wait_ms}"),
        Caller::SecretOnly,
        Body::empty(),
    )
    .await
}

/// `answer` or, while it is a 202, the held reads after it, as the client sends them. A
/// loaded machine can take more than `finish`'s 3 s even for the fixture.
async fn settled(app: &Router, id: &str, mut answer: Answer) -> Answer {
    let started = Instant::now();
    while answer.status == StatusCode::ACCEPTED && started.elapsed() < Duration::from_secs(120) {
        answer = read(app, id, dictation::MAX_READ_WAIT_MS).await;
    }
    answer
}

async fn delete(app: &Router, id: &str) -> Answer {
    call(
        app,
        "DELETE",
        &format!("/dictation/streams/{id}"),
        Caller::SecretOnly,
        Body::empty(),
    )
    .await
}

async fn status(app: &Router) -> Value {
    call(
        app,
        "GET",
        "/dictation/status",
        Caller::SecretOnly,
        Body::empty(),
    )
    .await
    .body
}

/// One whole dictation: create, `PUT`s of at most 524,288 B, `finish` carrying the last
/// chunk, a held read while it answers 202, delete. Answers the transcript.
async fn dictate(app: &Router, caller: Caller, request: Value, audio: Vec<u8>) -> Value {
    let started = Instant::now();
    let id = open(app, caller, request).await;
    let opened = started.elapsed();
    let chunks: Vec<&[u8]> = audio.chunks(dictation::MAX_CHUNK_BYTES).collect();
    let (last, before) = chunks.split_last().expect("some audio");
    for (seq, chunk) in before.iter().enumerate() {
        let put = put(app, &id, seq as u64, chunk.to_vec()).await;
        assert_eq!(put.status, StatusCode::OK, "{:?}", put.body);
    }
    let mut answer = finish_with(app, &id, before.len() as u64, last.to_vec()).await;
    while answer.status == StatusCode::ACCEPTED && started.elapsed() < Duration::from_secs(60) {
        answer = read(app, &id, dictation::MAX_READ_WAIT_MS).await;
    }
    if started.elapsed() > Duration::from_secs(3) {
        eprintln!(
            "slow dictation: opened in {opened:?}, answered after {:?}: {}",
            started.elapsed(),
            String::from_utf8_lossy(&answer.raw)
        );
    }
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "{:?}",
        String::from_utf8_lossy(&answer.raw)
    );
    assert_eq!(delete(app, &id).await.status, StatusCode::NO_CONTENT);
    answer.body
}

/// The transcript without what differs between any two runs: how long they took.
fn untimed(transcript: &Value) -> Value {
    let mut transcript = transcript.clone();
    if let Some(fields) = transcript.as_object_mut() {
        fields.remove("elapsed_ms");
        fields.remove("timings");
    }
    transcript
}

/// `jfk.wav`'s samples (16 kHz mono s16le), as raw little endian bytes.
fn jfk() -> Vec<u8> {
    wav_samples(&fixture("samples/jfk.wav"))
}

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

/// `seconds` of audio that is almost silence: a quiet hum, never exact zeros (a tail of
/// zeros is answered without a decode).
fn near_silence(seconds: f64) -> Vec<u8> {
    let samples = (seconds * 16_000.0) as usize;
    (0..samples)
        .flat_map(|i| (((i % 16) as i16) - 8).to_le_bytes())
        .collect()
}

/// `seconds` of `jfk.wav`, repeated as needed.
fn speech(seconds: f64) -> Vec<u8> {
    let clip = jfk();
    let bytes = (seconds * 32_000.0) as usize & !1;
    clip.iter().copied().cycle().take(bytes).collect()
}

async fn until_ready() {
    let started = Instant::now();
    while biorouter::dictation::service::service().status().state
        != biorouter::dictation::EngineState::Ready
    {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "the worker did not load"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn signal(pid: u32, name: &str) {
    let status = std::process::Command::new("kill")
        .arg(format!("-{name}"))
        .arg(pid.to_string())
        .status()
        .expect("kill");
    assert!(status.success(), "kill -{name} {pid}");
}

// ── the stream routes ────────────────────────────────────────────────────────────────

/// 201 with the stream's limits; 200 for each chunk; a repeated `seq` answered as before
/// with its body ignored; 409 with `expected_seq` for a `seq` past the next one; 413 for a
/// chunk over 524,288 B; 422 for an odd byte count; `finish` answers the transcript; after
/// `DELETE`, 410 for everything.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_stream_takes_its_chunks_in_order_and_answers_its_transcript() {
    if !engine_runs() {
        return;
    }
    let state = state().await;
    let app = app(&state);

    let created = create(&app, Caller::SecretOnly, request("external")).await;
    assert_eq!(created.status, StatusCode::CREATED);
    assert_eq!(created.body["max_chunk_bytes"], 524_288);
    assert_eq!(created.body["idle_timeout_s"], 60);
    assert_eq!(created.body["max_seconds"], 600);
    let id = created.body["stream_id"].as_str().unwrap().to_string();
    assert_eq!(id.len(), 32, "128 bits in hex");

    let audio = speech(2.0);
    let first = put(&app, &id, 0, audio.clone()).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(first.body["received_s"], 2.0);
    let again = put(&app, &id, 0, near_silence(1.0)).await;
    assert_eq!(
        again.status,
        StatusCode::OK,
        "a repeat is answered as before"
    );
    assert_eq!(again.body["received_s"], 2.0, "and its body ignored");

    let ahead = put(&app, &id, 5, audio.clone()).await;
    assert_eq!(ahead.status, StatusCode::CONFLICT);
    assert_eq!(ahead.body["code"], "outOfOrder");
    assert_eq!(ahead.body["expected_seq"], 1);

    let big = put(&app, &id, 1, vec![0; dictation::MAX_CHUNK_BYTES + 2]).await;
    assert_eq!(big.status, StatusCode::PAYLOAD_TOO_LARGE);
    let odd = put(&app, &id, 1, vec![0; 3]).await;
    assert_eq!(odd.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(odd.body["code"], "badRequest");

    let second = put(&app, &id, 1, speech(1.0)).await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(second.body["received_s"], 3.0);

    let done = settled(&app, &id, finish(&app, &id).await).await;
    assert_eq!(done.status, StatusCode::OK, "{:?}", done.body);
    let transcript = &done.body;
    assert_eq!(transcript["model"], "for-tests-ggml-base.en");
    assert_eq!(transcript["audio_s"], 3.0);
    assert_eq!(transcript["bias_scope"], "public");
    assert_eq!(transcript["no_speech"], true, "the fixture decodes nothing");
    assert_eq!(transcript["text"], "");
    assert!(transcript["timings"]["segments"].as_u64().unwrap() >= 1);
    for field in ["entries", "eligible", "withheld", "eligible_count"] {
        assert!(
            transcript.get(field).is_none(),
            "no field counts dictionary entries (P-6): {field}"
        );
    }
    // A second read answers the same transcript at once.
    let again = read(&app, &id, 0).await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(untimed(&again.body), untimed(transcript));
    let late = put(&app, &id, 2, speech(1.0)).await;
    assert_eq!(late.status, StatusCode::CONFLICT, "no audio after the stop");

    assert_eq!(delete(&app, &id).await.status, StatusCode::NO_CONTENT);
    for answer in [
        put(&app, &id, 2, speech(1.0)).await,
        finish(&app, &id).await,
        read(&app, &id, 0).await,
    ] {
        assert_eq!(answer.status, StatusCode::GONE);
        assert_eq!(answer.body["code"], "streamGone");
    }
    assert_eq!(
        delete(&app, &id).await.status,
        StatusCode::NO_CONTENT,
        "a second delete changes nothing"
    );
}

/// The creation body's limits (DD-28): JSON over 8 KiB and `context` over 2 KiB answer
/// 413; another format, a rate outside 8,000 to 96,000 and a malformed body answer 422.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn creation_refuses_what_the_contract_does_not_take() {
    if !engine_runs() {
        return;
    }
    let state = state().await;
    let app = app(&state);
    let mut big = request("external");
    big["context"] = json!("x".repeat(9 * 1024));
    assert_eq!(
        create(&app, Caller::SecretOnly, big).await.status,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let mut context = request("external");
    context["context"] = json!("x".repeat(2 * 1024 + 1));
    let answer = create(&app, Caller::SecretOnly, context).await;
    assert_eq!(answer.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(answer.body["code"], "tooLarge");
    for bad in [
        json!({"sample_rate": 16000, "format": "f32le", "target_kind": "external"}),
        json!({"sample_rate": 7999, "format": "s16le", "target_kind": "external"}),
        json!({"sample_rate": 96001, "format": "s16le", "target_kind": "external"}),
        json!({"sample_rate": 16000, "format": "s16le", "target_kind": "nowhere"}),
        json!({"format": "s16le"}),
    ] {
        let answer = create(&app, Caller::SecretOnly, bad.clone()).await;
        assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
        assert_eq!(answer.body["code"], "badRequest");
    }
    assert_eq!(state.dictation.open(), 0, "a refusal opens no stream");
}

/// With no speech model, creation answers 409 `noModel`, its message this caller's
/// `install.reason`: empty for the person, who may install (the interface opens the setup
/// popover), and MSG-INSTALL-AGENT for a caller without the proof.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn no_model_answers_409_with_this_callers_install_reason() {
    if !engine_runs() {
        return;
    }
    let state = state().await;
    let app = app(&state);
    let _env = env_lock::lock_env([(biorouter::dictation::service::TEST_MODEL_ENV, None::<&str>)]);
    for (caller, message) in [
        (Caller::Proof, ""),
        (Caller::SecretOnly, dictation::MSG_INSTALL_AGENT),
    ] {
        let answer = create(&app, caller, request("chat")).await;
        assert_eq!(answer.status, StatusCode::CONFLICT, "{caller:?}");
        assert_eq!(answer.body["code"], "noModel");
        assert_eq!(answer.body["message"], message, "{caller:?}");
    }
    assert_eq!(state.dictation.open(), 0);
}

/// DD-32: eight open streams, then 429 `tooManyStreams` with `Retry-After`; deleting one
/// makes room.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_ninth_stream_waits_for_a_free_one() {
    if !engine_runs() {
        return;
    }
    let state = state().await;
    let app = app(&state);
    let mut ids = Vec::new();
    for _ in 0..dictation::MAX_STREAMS {
        ids.push(open(&app, Caller::SecretOnly, request("external")).await);
    }
    let ninth = create(&app, Caller::SecretOnly, request("external")).await;
    assert_eq!(ninth.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(ninth.body["code"], "tooManyStreams");
    assert_eq!(
        ninth.headers.get(header::RETRY_AFTER).unwrap(),
        &dictation::RETRY_AFTER_SECONDS.to_string()
    );
    assert_eq!(status(&app).await["queue"]["streams"], 8);
    assert_eq!(
        delete(&app, &ids.pop().unwrap()).await.status,
        StatusCode::NO_CONTENT
    );
    ids.push(open(&app, Caller::SecretOnly, request("external")).await);
    for id in ids {
        delete(&app, &id).await;
    }
}

/// `finish` carries the tail chunk, so the stop costs one round trip; a repeat of an
/// acknowledged `seq` in `finish` is a repeat. A chunk that would pass 600 s is refused
/// and the audio already there is still transcribed.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn finish_carries_the_tail_and_the_cap_keeps_what_came_before() {
    if !engine_runs() {
        return;
    }
    let state = state().await;
    let app = app(&state);

    let id = open(&app, Caller::SecretOnly, request("external")).await;
    assert_eq!(put(&app, &id, 0, speech(1.0)).await.status, StatusCode::OK);
    let done = settled(&app, &id, finish_with(&app, &id, 1, speech(0.5)).await).await;
    assert_eq!(done.status, StatusCode::OK, "{:?}", done.body);
    assert_eq!(done.body["audio_s"], 1.5);
    delete(&app, &id).await;

    let id = open(&app, Caller::SecretOnly, request("external")).await;
    assert_eq!(put(&app, &id, 0, speech(1.0)).await.status, StatusCode::OK);
    let done = settled(&app, &id, finish_with(&app, &id, 0, speech(1.0)).await).await;
    assert_eq!(done.status, StatusCode::OK);
    assert_eq!(
        done.body["audio_s"], 1.0,
        "a resent seq in finish is a repeat"
    );
    delete(&app, &id).await;

    // 600 s of near silence, then one more chunk.
    let id = open(&app, Caller::SecretOnly, request("external")).await;
    let chunk = near_silence(16.384);
    let mut seq = 0;
    let mut received = 0.0;
    while received + 16.384 <= 600.0 {
        let answer = put(&app, &id, seq, chunk.clone()).await;
        assert_eq!(answer.status, StatusCode::OK, "{seq}");
        received = answer.body["received_s"].as_f64().unwrap();
        seq += 1;
    }
    let over = put(&app, &id, seq, chunk.clone()).await;
    assert_eq!(over.status, StatusCode::PAYLOAD_TOO_LARGE, "past 600 s");
    let done = settled(&app, &id, finish(&app, &id).await).await;
    assert_eq!(done.status, StatusCode::OK, "{:?}", done.body);
    assert!((done.body["audio_s"].as_f64().unwrap() - received).abs() < 1e-6);
    delete(&app, &id).await;
}

/// A stream nobody touches for its idle time is dropped and answers 410 (DD-28). The
/// idle time is shortened for this test through the debug only switch the routes read.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn an_idle_stream_expires() {
    if !engine_runs() {
        return;
    }
    let state = {
        let _env = env_lock::lock_env([(dictation::TEST_IDLE_ENV, Some("1000"))]);
        state().await
    };
    assert_eq!(state.dictation.limits().idle, Duration::from_millis(1_000));
    let app = app(&state);
    let id = open(&app, Caller::SecretOnly, request("external")).await;
    assert_eq!(put(&app, &id, 0, speech(1.0)).await.status, StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        put(&app, &id, 1, speech(1.0)).await.status,
        StatusCode::OK,
        "a chunk keeps it alive"
    );
    tokio::time::sleep(Duration::from_millis(3_000)).await;
    let gone = put(&app, &id, 2, speech(1.0)).await;
    assert_eq!(gone.status, StatusCode::GONE);
    assert_eq!(state.dictation.open(), 0);

    // A transcript that was read is kept until its idle time after the last read, and a
    // stream still decoding never expires.
    let id = open(&app, Caller::SecretOnly, request("external")).await;
    let done = settled(&app, &id, finish_with(&app, &id, 0, speech(1.0)).await).await;
    assert_eq!(done.status, StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(read(&app, &id, 0).await.status, StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(3_000)).await;
    assert_eq!(read(&app, &id, 0).await.status, StatusCode::GONE);
}

/// The voice detector's spans in `audio` (16 kHz s16le), in milliseconds, read with the
/// streams' own model and detector settings.
async fn spans_of(audio: &[u8]) -> Vec<(u64, u64)> {
    let service = biorouter::dictation::service::service();
    let spec = service.resolve(None).await.expect("the fixture");
    let samples: Vec<i16> = audio
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    service
        .vad_segments_with(
            spec,
            biorouter::dictation::audio::PcmSlice::whole(Arc::from(samples)),
        )
        .await
        .expect("the detector reads")
        .into_iter()
        .map(|span| (span.t0_ms, span.t1_ms))
        .collect()
}

/// About `seconds` of speech with no pause the detector closes: `jfk.wav`'s own spans of
/// speech, end to end, repeated as needed.
async fn unbroken_speech(seconds: f64) -> Vec<u8> {
    let clip = jfk();
    let voiced: Vec<u8> = spans_of(&clip)
        .await
        .into_iter()
        .flat_map(|(t0, t1)| {
            let end = ((t1 * 32) as usize).min(clip.len());
            clip[((t0 * 32) as usize).min(end)..end].to_vec()
        })
        .collect();
    let bytes = (seconds * 32_000.0) as usize & !1;
    voiced.iter().copied().cycle().take(bytes).collect()
}

/// Whether `spans` are one stretch of speech still open at `end_ms`: what the segmenter
/// cannot cut until a pause comes or 33 s lie past its mark.
fn one_open_span(spans: &[(u64, u64)], end_ms: u64) -> bool {
    matches!(spans, [(_, t1)] if t1 + biorouter::dictation::audio::OPEN_TAIL_MS > end_ms)
}

/// A detector pass that cuts nothing (open speech under 33 s past the mark) waits for new
/// audio, and a stream left open after one expires on its idle time. Before, the pass
/// started again at once for as long as nothing came: 20 s of unbroken speech and then
/// silence from the client ran 127 passes in 5 s on the test fixture, each queued ahead of
/// every stream's decodes, and the idle expiry counted them as work, so the stream was
/// still open 3 s after a 2 s idle time. A later chunk starts a pass again, 33 s past the
/// mark still cuts at 25 s, and `finish` carries the rest as the tail.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_pass_that_cuts_nothing_waits_for_new_audio() {
    if !engine_runs() {
        return;
    }
    let state = {
        let _env = env_lock::lock_env([(dictation::TEST_IDLE_ENV, Some("3000"))]);
        state().await
    };
    let app = app(&state);
    let audio = unbroken_speech(34.0).await;
    let first = &audio[..20 * 32_000];
    let spans = spans_of(first).await;
    assert!(one_open_span(&spans, 20_000), "{spans:?}");
    let spans = spans_of(&audio).await;
    assert!(one_open_span(&spans, 34_000), "{spans:?}");

    async fn put_all(app: &Router, id: &str, seq: &mut u64, audio: &[u8]) {
        for chunk in audio.chunks(dictation::MAX_CHUNK_BYTES) {
            assert_eq!(
                put(app, id, *seq, chunk.to_vec()).await.status,
                StatusCode::OK
            );
            *seq += 1;
        }
    }

    // Kept alive by held reads, as a client waiting on it would: the stall, new audio, the
    // forced cut and the tail.
    let id = open(&app, Caller::SecretOnly, request("external")).await;
    let progress = |id: &str| state.dictation.progress(id).expect("the stream");
    let settle = |id: String, decoded: u64| {
        let app = app.clone();
        let state = Arc::clone(&state);
        async move {
            let started = Instant::now();
            loop {
                let now = state.dictation.progress(&id).expect("the stream");
                if now.caught_up && now.segments_decoded >= decoded {
                    return now;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(60),
                    "the detector did not settle: {now:?}"
                );
                assert_eq!(read(&app, &id, 0).await.status, StatusCode::ACCEPTED);
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    };
    let mut seq = 0;
    put_all(&app, &id, &mut seq, first).await;
    let stalled = settle(id.clone(), 0).await;
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    assert_eq!(read(&app, &id, 0).await.status, StatusCode::ACCEPTED);
    let later = progress(&id);
    assert_eq!(
        later.detector_passes, stalled.detector_passes,
        "no pass over the same audio"
    );
    assert!(
        stalled.detector_passes <= 2,
        "a pass per chunk at most: {stalled:?}"
    );
    assert_eq!(
        later.segments_decoded, 0,
        "open speech under 33 s is not cut"
    );

    put_all(&app, &id, &mut seq, &audio[first.len()..]).await;
    let cut = settle(id.clone(), 1).await;
    assert!(
        cut.detector_passes > stalled.detector_passes,
        "new audio starts a pass: {cut:?}"
    );
    assert_eq!(
        cut.segments_decoded, 1,
        "cut at 25 s once 33 s lay past the mark"
    );
    let done = settled(&app, &id, finish(&app, &id).await).await;
    assert_eq!(done.status, StatusCode::OK, "{:?}", done.body);
    assert_eq!(done.body["audio_s"], 34.0);
    assert_eq!(
        done.body["timings"]["segments"], 2,
        "the 25 s segment, then the tail"
    );
    assert_eq!(delete(&app, &id).await.status, StatusCode::NO_CONTENT);

    // Left alone after the stall, the stream expires on its idle time.
    let id = open(&app, Caller::SecretOnly, request("external")).await;
    let mut seq = 0;
    put_all(&app, &id, &mut seq, first).await;
    let sent = Instant::now();
    let mut passes = 0;
    while let Some(now) = state.dictation.progress(&id) {
        passes = now.detector_passes;
        assert!(
            sent.elapsed() < Duration::from_secs(8),
            "a 3 s idle stream still open after {:?}: {now:?}",
            sent.elapsed()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(passes <= 2, "{passes} passes while nothing arrived");
    let gone = put(&app, &id, seq, speech(1.0)).await;
    assert_eq!(gone.status, StatusCode::GONE);
}

/// `finish` answers 202 after 3 s, and a held read then answers the transcript the
/// moment it is ready, with no client sleep between polls. A worker lost while the tail
/// decodes answers `500 engineFailed` from the read, and the next stream gets a fresh
/// worker.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_slow_tail_answers_202_then_a_held_read_and_a_lost_worker_answers_500() {
    if !engine_runs() {
        return;
    }
    let service = biorouter::dictation::service::service();
    let state = state().await;
    let app = app(&state);

    let id = open(&app, Caller::SecretOnly, request("external")).await;
    assert_eq!(put(&app, &id, 0, speech(1.0)).await.status, StatusCode::OK);
    until_ready().await;
    let pid = service.worker_pid().expect("a loaded worker");
    signal(pid, "STOP");
    let started = Instant::now();
    let pending = finish(&app, &id).await;
    assert_eq!(pending.status, StatusCode::ACCEPTED);
    assert_eq!(pending.body, json!({"pending": true}));
    assert!(started.elapsed() >= dictation::FINISH_WAIT);
    let still = read(&app, &id, 200).await;
    assert_eq!(still.status, StatusCode::ACCEPTED);
    signal(pid, "CONT");
    let resumed = Instant::now();
    let done = read(&app, &id, dictation::MAX_READ_WAIT_MS).await;
    assert_eq!(done.status, StatusCode::OK, "{:?}", done.body);
    assert!(
        resumed.elapsed() < Duration::from_secs(2),
        "the held read answered when the transcript was ready, not at its limit"
    );
    delete(&app, &id).await;

    let id = open(&app, Caller::SecretOnly, request("external")).await;
    assert_eq!(put(&app, &id, 0, speech(1.0)).await.status, StatusCode::OK);
    until_ready().await;
    let pid = service.worker_pid().expect("a loaded worker");
    signal(pid, "STOP");
    assert_eq!(finish(&app, &id).await.status, StatusCode::ACCEPTED);
    signal(pid, "KILL");
    let failed = read(&app, &id, dictation::MAX_READ_WAIT_MS).await;
    assert_eq!(failed.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(failed.body["code"], "engineFailed");
    let again = finish(&app, &id).await;
    assert_eq!(
        again.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "finish too"
    );
    // A failed stream takes no more audio: a chunk answers the failure, not 200 (or
    // `finished`), so a client still sending says so at once.
    let late = put(&app, &id, 1, speech(1.0)).await;
    assert_eq!(late.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(late.body["code"], "engineFailed");
    let repeat = put(&app, &id, 0, speech(1.0)).await;
    assert_eq!(
        repeat.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a repeat too"
    );
    delete(&app, &id).await;

    let after = dictate(&app, Caller::SecretOnly, request("external"), speech(1.0)).await;
    assert_eq!(
        after["no_speech"], true,
        "a fresh worker took the next stream"
    );
}

/// Streams take turns on the one worker: a 5 s dictation that stops while a 10 minute one
/// still has closed segments queued finishes within two segment times, never behind the
/// long stream's whole queue. The long stream's segments are cut while its audio
/// arrives, so at its own stop only its tail is left.
///
/// The 600 s here arrive at once, so for a while the long stream's detector reads a
/// backlog: a pass of up to 60 s after each of its segments, and `vad_segments` jobs go
/// before any transcription (DD-6). A tail that stopped then also waited for the pass in
/// flight, the long segment queued before it and the next pass; a replay of this pattern
/// on the service measured 1.1 to 2.3 segment times, with never more than one long
/// segment ahead (2026-11-07, after a CI run waited 13.5 s against a 3.2 s segment). A
/// live 10 minute stream's detector keeps up with its audio, so the short stream stops
/// once this one's detector has read all of it, and only segments are ahead of its tail.
/// The segment time is the long stream's own, timed just before the short stream starts,
/// because one decode measured alone before the long stream starts is not what a decode
/// costs while it runs on a shared 4 core runner.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_short_stream_finishes_while_a_long_one_runs() {
    if !engine_runs() {
        return;
    }
    let state = state().await;
    let app = app(&state);

    let long = open(&app, Caller::SecretOnly, request("external")).await;
    let chunk_s = 16.384;
    let audio = speech(600.0);
    let chunk = (chunk_s * 32_000.0) as usize;
    for (seq, piece) in audio.chunks(chunk).enumerate() {
        if (seq as f64 + 1.0) * chunk_s > 600.0 {
            break;
        }
        let answer = put(&app, &long, seq as u64, piece.to_vec()).await;
        assert_eq!(answer.status, StatusCode::OK);
    }
    let progress = || state.dictation.progress(&long).expect("the long stream");
    let started = Instant::now();
    while !progress().caught_up {
        assert!(
            started.elapsed() < Duration::from_secs(240),
            "the long stream's detector did not read its 600 s: {:?}",
            progress()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let caught_up_after = started.elapsed();

    // One segment's time: the median of three of the long stream's own decodes, each
    // from one answered segment to the next.
    let mut decoded = progress().segments_decoded;
    let mut since: Option<Instant> = None;
    let mut segment_times = Vec::new();
    while segment_times.len() < 3 {
        let now = progress().segments_decoded;
        if now != decoded {
            if let Some(since) = since {
                segment_times.push(since.elapsed() / (now - decoded) as u32);
            }
            decoded = now;
            since = Some(Instant::now());
        }
        assert!(
            started.elapsed() < Duration::from_secs(300),
            "the long stream stopped decoding: {:?}",
            progress()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    segment_times.sort();
    let segment_time = segment_times[1];

    let short_started = Instant::now();
    let short = open(&app, Caller::SecretOnly, request("external")).await;
    let queued = status(&app).await["queue"]["segments"].as_u64().unwrap();
    let before = progress().segments_decoded;
    let answer = settled(
        &app,
        &short,
        finish_with(&app, &short, 0, speech(5.0)).await,
    )
    .await;
    let short_time = short_started.elapsed();
    let during = progress().segments_decoded - before;
    let left = status(&app).await["queue"]["segments"].as_u64().unwrap();
    eprintln!(
        "detector read 600 s after {caught_up_after:?}; long stream segments \
         {segment_times:?}; queued {queued}; short stream {short_time:?} ({}); long \
         segments decoded meanwhile {during}, left after it: {left}",
        answer.body["timings"]
    );
    assert_eq!(answer.status, StatusCode::OK, "{:?}", answer.body);
    assert_eq!(
        answer.body["no_speech"], true,
        "the fixture decodes nothing"
    );
    assert_eq!(delete(&app, &short).await.status, StatusCode::NO_CONTENT);
    assert!(
        queued >= 4,
        "the long stream's segments were cut while it arrived"
    );
    assert!(
        left > 0,
        "the short stream finished only after the long stream's whole queue"
    );
    // Round robin: the long segment in flight at the stop, and the one after it only when
    // the first was answered just as the tail was being queued.
    assert!(
        during <= 2,
        "the long stream decoded {during} segments while the short one waited"
    );
    // Two segment times (the long stream's segment in flight, then the short tail), and a
    // second for the round trips.
    assert!(
        short_time <= segment_time * 2 + Duration::from_secs(1),
        "the short stream waited {short_time:?}, one segment is {segment_time:?}"
    );

    let mut done = finish(&app, &long).await;
    while done.status == StatusCode::ACCEPTED {
        done = read(&app, &long, dictation::MAX_READ_WAIT_MS).await;
    }
    assert_eq!(done.status, StatusCode::OK, "{:?}", done.body);
    assert!(done.body["timings"]["segments"].as_u64().unwrap() >= 20);
    delete(&app, &long).await;
}

/// MSG-SERVER-MEMORY (DD-32): a load the free memory cannot hold answers 503 before
/// anything starts. The figure is injected through the debug only switch the check
/// reads, on every system.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_load_that_does_not_fit_answers_server_memory() {
    if !engine_runs() {
        return;
    }
    let state = state().await;
    let app = app(&state);
    biorouter::dictation::service::service().unload().await;
    let refused = {
        let _env = env_lock::lock_env([(
            biorouter::dictation::service::TEST_FREE_MEMORY_ENV,
            Some("1000000"),
        )]);
        create(&app, Caller::SecretOnly, request("external")).await
    };
    assert_eq!(refused.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(refused.body["code"], "serverMemory");
    let message = refused.body["message"].as_str().unwrap();
    assert!(
        message.starts_with("This server does not have enough free memory to load"),
        "{message}"
    );
    assert_eq!(state.dictation.open(), 0);
    let id = open(&app, Caller::SecretOnly, request("external")).await;
    delete(&app, &id).await;
}

/// P-1, P-2, DD-93: a stream leaves no file behind, and nothing it logs holds its
/// `context`, a dictionary term or its transcript.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_stream_leaves_nothing_on_disk_or_in_a_log_line() {
    if !engine_runs() {
        return;
    }
    let state = state().await;
    let app = app(&state);
    let term = "Zorblattine";
    open_store()
        .write(|file| {
            file.add(
                store::NewTerm::person(term, store::EntryTier::Public),
                store::Reach::All,
                store::schema::now(),
            )
        })
        .expect("a term");
    let context = "CONTEXT-SENTINEL-7f3a about Zorblattine";
    let mut body = request("external");
    body["context"] = json!(context);
    // The first stream creates what any stream needs once (the dictionary's folder).
    dictate(&app, Caller::SecretOnly, body.clone(), speech(9.0)).await;

    let root = PathBuf::from(std::env::var("BIOROUTER_PATH_ROOT").unwrap());
    let before = files_under(&root);
    let logged_before = captured_logs().lock().unwrap().len();
    let transcript = dictate(&app, Caller::SecretOnly, body, speech(9.0)).await;
    // The usage the dictation recorded is written after the answer, off its task.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let after = files_under(&root);
    let new: Vec<_> = after
        .keys()
        .filter(|path| !before.contains_key(*path))
        .collect();
    assert!(new.is_empty(), "a stream wrote files: {new:?}");
    // Nor did it write into a file that was there: the fixture hears no speech, so no
    // usage is recorded and the dictionary is read, never written.
    let changed: Vec<_> = before
        .iter()
        .filter(|(path, seen)| after.get(*path) != Some(*seen))
        .map(|(path, _)| path)
        .collect();
    assert!(changed.is_empty(), "a stream wrote into files: {changed:?}");

    let logs =
        String::from_utf8_lossy(&captured_logs().lock().unwrap()[logged_before..]).into_owned();
    assert!(
        logs.contains("dictation stream opened") && logs.contains("dictation stream transcribed"),
        "the capture saw the stream: {logs}"
    );
    for secret in ["CONTEXT-SENTINEL", term, "Zorblatt"] {
        assert!(
            !logs.contains(secret),
            "{secret} reached a log line: {logs}"
        );
    }
    if let Some(text) = transcript["text"].as_str().filter(|t| !t.is_empty()) {
        assert!(!logs.contains(text));
    }
}

/// Every file under `root` with its length, modification time and SHA-256, so a write
/// into a file that was already there shows as well as a new file.
fn files_under(root: &Path) -> BTreeMap<PathBuf, (u64, Option<std::time::SystemTime>, String)> {
    let mut found = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let meta = entry.metadata().ok();
                let digest = std::fs::read(&path)
                    .map(|bytes| hex_digest(&bytes))
                    .unwrap_or_default();
                found.insert(
                    path,
                    (
                        meta.as_ref().map_or(0, |m| m.len()),
                        meta.and_then(|m| m.modified().ok()),
                        digest,
                    ),
                );
            }
        }
    }
    found
}

fn hex_digest(bytes: &[u8]) -> String {
    <sha2::Sha256 as sha2::Digest>::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ── DD-30: the target and its bias scope ─────────────────────────────────────────────

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

fn configured(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    let mut config = versa();
    config.remove("BIOROUTER_PROVIDER");
    config.remove("BIOROUTER_MODEL");
    for (key, value) in pairs {
        config.insert(key.to_string(), value.to_string());
    }
    config
}

async fn seed_chat(state: &Arc<AppState>, private: bool) -> String {
    let manager = state.session_manager();
    let chat = manager
        .create_session(
            PathBuf::from("/tmp/dictation_routes"),
            "dictation target (test fixture)".to_string(),
            SessionType::User,
        )
        .await
        .unwrap();
    if private {
        manager
            .update(&chat.id)
            .provider_name("versa_azure")
            .model_config(ModelConfig::new("gpt-4o").unwrap())
            .raise_privacy(SessionClassification::Private, "turn:versa_azure")
            .apply()
            .await
            .unwrap();
    }
    chat.id
}

fn chat_request(session_id: &str) -> Value {
    let mut body = request("chat");
    body["session_id"] = json!(session_id);
    body
}

/// P-4, P-6, the echo oracle: a caller holding only the secret, near silence and a
/// private chat's id get public scope and the same answer as for a chat that does not
/// exist, or no chat at all. With the proof, the same private chat is private.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_refused_chat_gives_public_scope_and_the_same_answer() {
    if !engine_runs() {
        return;
    }
    let state = state().await;
    let app = app(&state);
    let private = seed_chat(&state, true).await;

    let into_private = dictate(
        &app,
        Caller::SecretOnly,
        chat_request(&private),
        near_silence(2.0),
    )
    .await;
    let into_nothing = dictate(
        &app,
        Caller::SecretOnly,
        chat_request("20261106_no_such_chat"),
        near_silence(2.0),
    )
    .await;
    let no_chat = dictate(&app, Caller::SecretOnly, request("chat"), near_silence(2.0)).await;
    assert_eq!(into_private["bias_scope"], "public");
    assert_eq!(untimed(&into_private), untimed(&into_nothing));
    assert_eq!(untimed(&into_private), untimed(&no_chat));
    // A private provider stated in `X-Caller-Provider` reaches a private chat, as it may
    // list one (DD-29), and is private standing.
    let stated = dictate(
        &app,
        Caller::StatedPrivate,
        chat_request(&private),
        near_silence(2.0),
    )
    .await;
    assert_eq!(stated["bias_scope"], "private");

    let proven = dictate(
        &app,
        Caller::Proof,
        chat_request(&private),
        near_silence(2.0),
    )
    .await;
    assert_eq!(proven["bias_scope"], "private");
    state
        .session_manager()
        .delete_session(&private)
        .await
        .unwrap();
}

/// A chat bound to a private provider whose row is still public (its first turn has not
/// run) is private through `chat_reach`, which reads the live agent's provider.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_chat_bound_to_a_private_model_is_private_before_its_row_says_so() {
    if !engine_runs() {
        return;
    }
    let state = state().await;
    let app = app(&state);
    let chat = seed_chat(&state, false).await;
    let before = dictate(&app, Caller::Proof, chat_request(&chat), near_silence(1.0)).await;
    assert_eq!(
        before["bias_scope"], "public",
        "a public chat with no agent"
    );

    let agent = state.get_agent(chat.clone()).await.unwrap();
    let provider = with_config_overrides(versa(), async {
        biorouter::providers::create(
            "versa_azure",
            ModelConfig::new(VERSA_AZURE_DEFAULT_MODEL).unwrap(),
        )
        .await
    })
    .await
    .expect("a versa provider");
    assert!(provider.tier().is_private());
    agent.update_provider(provider, &chat).await.unwrap();
    let row = state
        .session_manager()
        .get_session(&chat, false)
        .await
        .unwrap();
    assert_eq!(row.privacy_tier, SessionClassification::Public);

    let bound = dictate(&app, Caller::Proof, chat_request(&chat), near_silence(1.0)).await;
    assert_eq!(bound["bias_scope"], "private");
    let unproven = dictate(
        &app,
        Caller::SecretOnly,
        chat_request(&chat),
        near_silence(1.0),
    )
    .await;
    assert_eq!(
        unproven["bias_scope"], "public",
        "a private target without private standing"
    );
    state.clear_cached_agents().await;
    state.session_manager().delete_session(&chat).await.unwrap();
}

/// Runs `fut` with `config` in the process environment and every other key a tier input
/// file reads unset. The target decision runs on a task of its own after the 201, which a
/// task local override (`with_config_overrides`) does not reach, so these tests configure
/// the daemon the way an operator's environment does. The tests run one at a time.
async fn with_config_env<T>(
    config: HashMap<String, String>,
    fut: impl std::future::Future<Output = T>,
) -> T {
    use biorouter::privacy::config_keys::NOT_CAPABILITY_CONFIG_KEYS;
    let names: BTreeSet<String> = biorouter::privacy::CAPABILITY_CONFIG_KEYS
        .iter()
        .map(|name| name.to_string())
        .chain(
            NOT_CAPABILITY_CONFIG_KEYS
                .iter()
                .map(|(name, _)| name.to_string()),
        )
        .chain(config.keys().cloned())
        .collect();
    let vars: Vec<(String, Option<String>)> = names
        .into_iter()
        .map(|name| {
            let value = config.get(&name).cloned();
            (name, value)
        })
        .collect();
    let _env = env_lock::lock_env(vars.iter().map(|(k, v)| (k.as_str(), v.as_deref())));
    fut.await
}

async fn new_chat_scope(
    app: &Router,
    caller: Caller,
    config: HashMap<String, String>,
    pick: Option<(&str, &str)>,
) -> String {
    let mut body = request("new_chat");
    if let Some((provider, model)) = pick {
        body["new_chat_provider"] = json!(provider);
        body["new_chat_model"] = json!(model);
    }
    let transcript = with_config_env(config, dictate(app, caller, body, near_silence(1.0))).await;
    transcript["bias_scope"].as_str().unwrap().to_string()
}

/// DD-30 for `new_chat`: the tier the new chat's bind would produce. The tab's pick
/// counts only with the proof; an `ollama` cloud tag, a remote `OLLAMA_HOST` and a
/// private worker under a public lead are public, whatever `ollama` declares.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_new_chat_is_judged_by_the_tier_its_bind_would_produce() {
    if !engine_runs() {
        return;
    }
    let state = state().await;
    let app = app(&state);
    let remote = configured(&[
        ("BIOROUTER_PROVIDER", "ollama"),
        ("BIOROUTER_MODEL", "qwen3.8:27b"),
        ("OLLAMA_HOST", "https://ollama.example"),
    ]);
    let pick = Some(("versa_azure", VERSA_AZURE_DEFAULT_MODEL));

    assert_eq!(
        new_chat_scope(&app, Caller::Proof, remote.clone(), None).await,
        "public",
        "a remote OLLAMA_HOST"
    );
    assert_eq!(
        new_chat_scope(&app, Caller::Proof, remote.clone(), pick).await,
        "private",
        "the tab's private pick, with the proof"
    );
    assert_eq!(
        new_chat_scope(&app, Caller::StatedPrivate, remote.clone(), pick).await,
        "public",
        "the pick is ignored without the proof"
    );
    assert_eq!(
        new_chat_scope(&app, Caller::Proof, versa(), None).await,
        "private",
        "the configured private provider"
    );
    assert_eq!(
        new_chat_scope(&app, Caller::SecretOnly, versa(), None).await,
        "public",
        "a private bind without private standing"
    );
    let cloud = configured(&[
        ("BIOROUTER_PROVIDER", "ollama"),
        ("BIOROUTER_MODEL", "gpt-oss:120b-cloud"),
        ("OLLAMA_HOST", "127.0.0.1"),
    ]);
    assert_eq!(
        new_chat_scope(&app, Caller::Proof, cloud, None).await,
        "public",
        "an ollama cloud tag"
    );
    let pair = configured(&[
        ("BIOROUTER_PROVIDER", "llamacpp"),
        ("BIOROUTER_MODEL", "qwen3.8-27b"),
        ("BIOROUTER_LEAD_PROVIDER", "ollama"),
        ("BIOROUTER_LEAD_MODEL", "qwen3.8:27b"),
        ("OLLAMA_HOST", "https://ollama.example"),
    ]);
    assert_eq!(
        new_chat_scope(&app, Caller::Proof, pair, None).await,
        "public",
        "a private worker under a public lead"
    );
}

/// O-8: `include_private_terms` on a Crew or outside target admits private entries only
/// from a caller with the proof and private standing; anything less is public.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn private_terms_reach_crew_and_outside_targets_only_when_the_person_includes_them() {
    if !engine_runs() {
        return;
    }
    let state = state().await;
    let app = app(&state);
    for kind in ["crew", "external"] {
        let mut included = request(kind);
        included["include_private_terms"] = json!(true);
        let scope = |transcript: Value| transcript["bias_scope"].as_str().unwrap().to_string();
        assert_eq!(
            scope(dictate(&app, Caller::Proof, included.clone(), near_silence(1.0)).await),
            "private",
            "{kind}: the person included them"
        );
        assert_eq!(
            scope(dictate(&app, Caller::Proof, request(kind), near_silence(1.0)).await),
            "public",
            "{kind}: not included"
        );
        for caller in [Caller::SecretOnly, Caller::StatedPrivate] {
            assert_eq!(
                scope(dictate(&app, caller, included.clone(), near_silence(1.0)).await),
                "public",
                "{kind}: {caller:?} cannot include them"
            );
        }
    }
}

/// Adds a private row for `written` that names `institution`, as one written in a chat
/// whose model that institution runs.
fn add_institution_row(written: &str, institution: &str) {
    let store = open_store();
    let id = match store
        .write(|file| {
            file.add(
                store::NewTerm::person(written, store::EntryTier::Private),
                store::Reach::All,
                store::schema::now(),
            )
        })
        .expect("added")
    {
        store::AddOutcome::Created { id, .. } => id,
        other => panic!("{other:?}"),
    };
    store
        .write(|file| {
            let entry = file.entries.iter_mut().find(|e| e.id == id).unwrap();
            entry.affiliation = Some(store::RowAffiliation {
                institutions: vec![institution.to_string()],
            });
            Ok(())
        })
        .expect("stamped");
}

/// The written forms of the entries a stream into `request` may use, read once its
/// target was decided, from the daemon's own table (no route answers them).
async fn eligible_terms(state: &Arc<AppState>, app: &Router, request: Value) -> Vec<String> {
    let id = open(app, Caller::Proof, request).await;
    let started = Instant::now();
    let terms = loop {
        if let Some(terms) = state.dictation.eligible_terms(&id) {
            break terms;
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the target was never decided"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    delete(app, &id).await;
    terms
}

/// P-16: in a chat whose live model UCSF runs (Versa), a private row that names another
/// institution is left out of the bias set and one that names UCSF is kept, and the answer
/// has the same fields as one where the left out row is not there. The live agent is
/// bound on purpose: with none, the target's affiliation is unstated and every row that
/// names an institution is left out, which would pass whether or not the route passed
/// the live model's affiliation.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn an_institution_row_left_out_changes_no_field() {
    if !engine_runs() {
        return;
    }
    let state = state().await;
    let app = app(&state);
    let chat = seed_chat(&state, true).await;
    let agent = state.get_agent(chat.clone()).await.unwrap();
    let provider = with_config_overrides(versa(), async {
        biorouter::providers::create(
            "versa_azure",
            ModelConfig::new(VERSA_AZURE_DEFAULT_MODEL).unwrap(),
        )
        .await
    })
    .await
    .expect("a versa provider");
    agent.update_provider(provider, &chat).await.unwrap();

    add_institution_row("Ucsfanine", "ucsf");
    let without = dictate(&app, Caller::Proof, chat_request(&chat), near_silence(1.0)).await;
    add_institution_row("Stanfordine", "stanford");
    let terms = eligible_terms(&state, &app, chat_request(&chat)).await;
    assert!(
        terms.iter().any(|t| t == "Ucsfanine"),
        "a row UCSF's model may read: {terms:?}"
    );
    assert!(
        !terms.iter().any(|t| t == "Stanfordine"),
        "a row another institution wrote: {terms:?}"
    );
    let with = dictate(&app, Caller::Proof, chat_request(&chat), near_silence(1.0)).await;
    assert_eq!(with["bias_scope"], "private");
    assert_eq!(untimed(&with), untimed(&without));

    // With no live agent the affiliation is unstated, and both rows are left out.
    state.clear_cached_agents().await;
    let unstated = eligible_terms(&state, &app, chat_request(&chat)).await;
    assert!(
        !unstated
            .iter()
            .any(|t| t == "Ucsfanine" || t == "Stanfordine"),
        "{unstated:?}"
    );
    state.session_manager().delete_session(&chat).await.unwrap();
}

/// DD-24: `POST /dictation/models/rescan` identifies the files in a models folder by
/// SHA-256, whatever they are called: the vendored detector fixture, byte identical to
/// the catalog's `silero-vad-v6.2.0`, is placed from a stray name and moved, not copied
/// (the stray is gone once its copy is in place), and a file no pin matches is listed in
/// `unrecognized` and left where it is.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_rescan_places_what_its_hash_names_and_lists_the_rest() {
    use biorouter::dictation::catalog::{CATALOG, VAD_ID};
    use biorouter::dictation::{models, paths};

    let state = state().await;
    let app = app(&state);
    let folder = paths::user_models_dir();
    std::fs::create_dir_all(&folder).unwrap();
    let stray = folder.join("downloaded-detector.bin");
    let junk = folder.join("notes-not-a-model.txt");
    std::fs::copy(fixture("models/for-tests-silero-v6.2.0-ggml.bin"), &stray).unwrap();
    std::fs::write(&junk, b"not a model").unwrap();
    let vad = CATALOG.entry(VAD_ID).unwrap();
    let placed = models::model_path(&folder, vad);
    assert!(!placed.exists(), "the detector starts out not installed");

    let answer = call(
        &app,
        "POST",
        "/dictation/models/rescan",
        Caller::SecretOnly,
        Body::empty(),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{:?}", answer.body);
    assert_eq!(answer.body["added"], json!([VAD_ID]));
    assert_eq!(
        answer.body["unrecognized"],
        json!(["notes-not-a-model.txt"])
    );
    assert!(placed.exists(), "placed under its catalog name");
    assert!(
        !stray.exists(),
        "moved, not copied: the model is not kept twice"
    );
    assert!(junk.exists(), "a file no pin matches stays where it is");

    models::remove(&folder, vad).unwrap();
    std::fs::remove_file(&junk).unwrap();
}

/// DD-32's second half: the dictation worker takes half its threads while a turn in this
/// daemon runs on an Ollama model served from this computer. The daemon installs that
/// question when its state is built; a cloud tag through a loopback host, a remote
/// `OLLAMA_HOST` and a local model with no turn running do not count. Building these
/// providers opens no connection.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_turn_on_a_loopback_ollama_halves_the_dictation_threads() {
    use biorouter::dictation::service;

    install_the_desktop_key();
    let state = AppState::new().await.expect("app state");
    let bind = |host: &'static str, model: &'static str| {
        let state = Arc::clone(&state);
        async move {
            let chat = seed_chat(&state, false).await;
            let agent = state.get_agent(chat.clone()).await.unwrap();
            let config = HashMap::from([("OLLAMA_HOST".to_string(), host.to_string())]);
            let provider = with_config_overrides(config, async {
                biorouter::providers::create("ollama", ModelConfig::new(model).unwrap()).await
            })
            .await
            .expect("an ollama provider");
            agent.update_provider(provider, &chat).await.unwrap();
            chat
        }
    };
    let local = bind("127.0.0.1", "qwen3.8:27b").await;
    let cloud = bind("127.0.0.1", "gpt-oss:120b-cloud").await;
    let remote = bind("https://ollama.example", "qwen3.8:27b").await;
    let threads = || async {
        service::service().forget_thread_count();
        service::service().threads().await
    };
    let full = service::threads_rule(service::base_threads(), false);
    let halved = service::threads_rule(service::base_threads(), true);

    assert!(!state.local_ollama_serving().await, "no turn runs");
    assert_eq!(threads().await, full);
    for chat in [&cloud, &remote] {
        let turn = state
            .try_begin_turn_idempotent(chat, tokio_util::sync::CancellationToken::new(), None)
            .expect("a turn");
        assert!(!state.local_ollama_serving().await, "{chat}");
        drop(turn);
    }
    let turn = state
        .try_begin_turn_idempotent(&local, tokio_util::sync::CancellationToken::new(), None)
        .expect("a turn");
    assert!(state.local_ollama_serving().await);
    assert!(
        service::local_serving_now().await,
        "the daemon installed its probe"
    );
    assert_eq!(threads().await, halved);
    drop(turn);
    assert_eq!(threads().await, full);
    service::service().forget_thread_count();

    state.clear_cached_agents().await;
    for chat in [local, cloud, remote] {
        state.session_manager().delete_session(&chat).await.unwrap();
    }
}

/// The no path rule (DD-24): no request body under `/dictation/*` in the OpenAPI
/// document names a filesystem path or a URL.
#[test]
fn no_dictation_request_names_a_path() {
    let spec = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/desktop/openapi.json"),
    )
    .expect("openapi.json");
    let spec: Value = serde_json::from_str(&spec).unwrap();
    let schemas = &spec["components"]["schemas"];
    let mut checked = 0;
    for (name, schema) in schemas.as_object().unwrap() {
        if !name.starts_with("Dictation") {
            continue;
        }
        checked += 1;
        for property in schema["properties"]
            .as_object()
            .map(|p| p.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
        {
            if name.ends_with("Request") {
                assert!(
                    !["path", "file", "dir", "url"].contains(&property.as_str()),
                    "{name}.{property}"
                );
            }
        }
    }
    assert!(checked >= 10, "the dictation schemas are in the document");
}

// ── PERF-1: the store read on a stream's creation ────────────────────────────────────

/// One "dictation stream target decided" line's figures.
#[derive(Debug)]
#[allow(dead_code)]
struct DictionaryTiming {
    store_us: u64,
    request_us: u64,
    dictionary_us: u64,
}

/// The figures of each "dictation stream target decided" line logged since `from` bytes
/// into the captured log.
fn dictionary_timings(from: usize) -> Vec<DictionaryTiming> {
    let logs = captured_logs()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let text = String::from_utf8_lossy(&logs[from.min(logs.len())..]).into_owned();
    let field = |line: &str, name: &str| -> u64 {
        line.split(&format!(" {name}="))
            .nth(1)
            .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|digits| digits.parse().ok())
            .unwrap_or(u64::MAX)
    };
    text.lines()
        .filter(|line| line.contains("dictation stream target decided"))
        .map(|line| DictionaryTiming {
            store_us: field(line, "store_us"),
            request_us: field(line, "request_us"),
            dictionary_us: field(line, "dictionary_us"),
        })
        .collect()
}

fn log_length() -> usize {
    captured_logs()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .len()
}

/// Removes the dictionary folder when dropped, so a test that fills the store leaves the
/// other tests their few rows even when it fails.
struct EmptiedStore;

impl Drop for EmptiedStore {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(biorouter::security::dictionary_store::dictionary_dir());
    }
}

/// The `/dictionary/*` routes of `state`, behind the secret.
fn dictionary_routes(state: &Arc<AppState>) -> Router {
    biorouter_server::routes::dictionary::routes(Arc::clone(state)).layer(
        axum::middleware::from_fn_with_state(
            SECRET.to_string(),
            biorouter_server::auth::check_token,
        ),
    )
}

/// The daemon's store parses and its dictionary cache's counters, together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Builds {
    parses: u64,
    copies: u64,
    indexes: u64,
}

/// The counters once the daemon's background rebuild after a write, the one counted past
/// `prewarms`, has finished.
async fn prewarmed_after(prewarms: u64) -> Builds {
    let started = Instant::now();
    while dictation::dictionary_cache().prewarms() <= prewarms {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "the rebuild after the write never ran"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    builds()
}

fn builds() -> Builds {
    let cache = dictation::dictionary_cache();
    Builds {
        parses: biorouter_server::routes::dictionary::shared_store().store_parses(),
        copies: cache.rows_copied(),
        indexes: cache.indexes_built(),
    }
}

/// PR-16's live check found every stream parsing the whole store (112 to 130 ms at 4,993
/// rows in a debug build, against 0 ms at 3 rows): a stream opened a new handle each time.
/// Now a stream reads the daemon's shared handle, so a dictation over an unchanged store of
/// 4,990 rows parses nothing, and a term added or deleted over HTTP leaves the next stream
/// nothing to parse either. Past the parse, the stream finds the rows the bias set admitted,
/// with their ranking and correction indexes, in the daemon's dictionary cache, and builds
/// only its own part; after a write over HTTP the daemon builds the new set in the
/// background, so the next stream builds nothing at all. Asserted on the counters, because
/// a timing assertion would not hold on a loaded runner; the logged times are printed.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_stream_over_an_unchanged_store_parses_nothing() {
    if !engine_runs() {
        return;
    }
    let state = state().await;
    let app = app(&state);
    drop(EmptiedStore);
    let _emptied = EmptiedStore;
    // Ten below the active cap, so the add below fits.
    let lines: String = (0..4_990).map(|i| format!("BRL{i:04}X\n")).collect();
    let rows = store::transfer::parse_import(store::ImportFormat::Lines, &lines).unwrap();
    open_store()
        .write(|file| {
            Ok(file.import(
                rows,
                store::EntryTier::Public,
                &|_| false,
                store::schema::now(),
            ))
        })
        .unwrap();

    // The first stream after another handle wrote the file parses it, once, and builds
    // its set.
    let from = log_length();
    let before = builds();
    let first = eligible_terms(&state, &app, request("external")).await;
    assert_eq!(first.len(), 4_990);
    let built = builds();
    assert_eq!(
        built.parses,
        before.parses + 1,
        "the first stream reads the new file"
    );
    assert_eq!(built.copies, before.copies + 1, "and copies its rows once");
    assert_eq!(built.indexes, before.indexes + 1, "and indexes them once");
    for _ in 0..3 {
        assert_eq!(
            eligible_terms(&state, &app, request("external"))
                .await
                .len(),
            4_990
        );
    }
    assert_eq!(
        builds(),
        built,
        "a stream over an unchanged store parses, copies and indexes nothing"
    );

    // A term added over HTTP: the write parses once, the daemon builds the new file's set
    // after the answer, and the next stream builds nothing.
    let prewarms = dictation::dictionary_cache().prewarms();
    let dictionary_app = dictionary_routes(&state);
    let added = call(
        &dictionary_app,
        "POST",
        "/dictionary/entries",
        Caller::Proof,
        Body::from(json!({"written": "Biorouter"}).to_string()),
    )
    .await;
    assert_eq!(added.status, StatusCode::CREATED, "{:?}", added.body);
    let prewarmed = prewarmed_after(prewarms).await;
    assert_eq!(prewarmed.parses, built.parses + 1, "the write's own read");
    assert!(
        prewarmed.indexes > built.indexes,
        "the new file's set, built"
    );
    let after = eligible_terms(&state, &app, request("external")).await;
    assert_eq!(after.len(), 4_991);
    assert!(after.iter().any(|t| t == "Biorouter"));
    assert_eq!(
        builds(),
        prewarmed,
        "the stream after a write parses, copies and indexes nothing"
    );

    // A delete over HTTP, then a dictation: the same.
    let id = added.body["entry"]["id"]
        .as_str()
        .expect("an id")
        .to_string();
    let prewarms = dictation::dictionary_cache().prewarms();
    let deleted = call(
        &dictionary_app,
        "DELETE",
        &format!("/dictionary/entries/{id}"),
        Caller::Proof,
        Body::empty(),
    )
    .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{:?}", deleted.body);
    let deleted_built = prewarmed_after(prewarms).await;
    assert_eq!(
        deleted_built.parses,
        prewarmed.parses + 1,
        "the delete's own read"
    );
    let after = eligible_terms(&state, &app, request("external")).await;
    assert_eq!(after.len(), 4_990);
    assert_eq!(
        builds(),
        deleted_built,
        "the stream after a delete parses, copies and indexes nothing"
    );
    let timings = dictionary_timings(from);
    eprintln!("per stream at 4,990 rows: {timings:?}");
    assert_eq!(timings.len(), 6, "{timings:?}");
    assert!(
        timings.iter().all(|t| t.dictionary_us != u64::MAX),
        "{timings:?}"
    );
}

/// The PERF-1 review found a stream opened at once after a write, while the daemon's
/// rebuild of the new file's sets still ran, building the same set again beside it: 6.7 ms
/// with bare rows and 22 to 23 ms with forms in a release build, where the set once built
/// costs 0.3 ms. Now the stream waits for the rebuild and takes its set. Asserted on the
/// counters: a write followed at once by a stream copies and indexes as much as a write
/// whose rebuild the stream let finish, which is what the rebuild alone builds. The store
/// holds 4,990 rows with a spoken form each, so a set takes long enough to build that the
/// stream meets the rebuild; the logged times and the waits are printed.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_stream_opened_during_the_rebuild_after_a_write_builds_nothing_twice() {
    if !engine_runs() {
        return;
    }
    let state = state().await;
    let app = app(&state);
    drop(EmptiedStore);
    let _emptied = EmptiedStore;
    let lines: String = (0..4_990).map(|i| format!("BRL{i:04}X\n")).collect();
    let rows = store::transfer::parse_import(store::ImportFormat::Lines, &lines).unwrap();
    open_store()
        .write(|file| {
            let _ = file.import(
                rows,
                store::EntryTier::Public,
                &|_| false,
                store::schema::now(),
            );
            for (i, entry) in file.entries.iter_mut().enumerate() {
                entry.spoken_forms = vec![format!("barrel {i} spoken")];
            }
            Ok(())
        })
        .unwrap();
    let dictionary_app = dictionary_routes(&state);
    // The stream's input is the most recent one, and its set is built.
    assert_eq!(
        eligible_terms(&state, &app, request("external"))
            .await
            .len(),
        4_990
    );
    let cache = dictation::dictionary_cache();
    let mut built = Vec::new();
    for (written, at_once) in [("Biorouter", false), ("Modulite", true)] {
        let before = builds();
        let prewarms = cache.prewarms();
        let waits = cache.waits();
        let from = log_length();
        let added = call(
            &dictionary_app,
            "POST",
            "/dictionary/entries",
            Caller::Proof,
            Body::from(json!({ "written": written }).to_string()),
        )
        .await;
        assert_eq!(added.status, StatusCode::CREATED, "{:?}", added.body);
        let terms = if at_once {
            let terms = eligible_terms(&state, &app, request("external")).await;
            prewarmed_after(prewarms).await;
            terms
        } else {
            prewarmed_after(prewarms).await;
            eligible_terms(&state, &app, request("external")).await
        };
        assert!(terms.iter().any(|t| t == written), "{written}");
        let after = builds();
        built.push((after.copies - before.copies, after.indexes - before.indexes));
        eprintln!(
            "{written}, the stream {}: {:?}, {} wait(s)",
            if at_once {
                "at once"
            } else {
                "after the rebuild"
            },
            dictionary_timings(from),
            cache.waits() - waits
        );
    }
    assert!(built[0].1 >= 1, "the rebuild indexed the new file's set");
    assert_eq!(
        built[1], built[0],
        "a stream at once builds no set the rebuild builds (copies, indexes)"
    );
}

// ── live: stop to text on the catalog models (`--ignored`) ───────────────────────────

/// The median of `values`.
fn median(values: &mut [u64]) -> u64 {
    values.sort_unstable();
    values.get(values.len() / 2).copied().unwrap_or(0)
}

/// Sends `audio` into `id` as a client does in real time: a `PUT` every 2 s of audio,
/// 2 s apart, and the remainder with `finish`. Answers the transcript and the time from
/// the stop (the `finish` leaving) to the text.
async fn replay(app: &Router, id: &str, audio: &[u8]) -> (Value, Duration) {
    let per_put = 64_000;
    let mut seq = 0;
    let mut sent = 0;
    while audio.len() - sent > per_put {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let answer = put(app, id, seq, audio[sent..sent + per_put].to_vec()).await;
        assert_eq!(answer.status, StatusCode::OK, "{:?}", answer.body);
        sent += per_put;
        seq += 1;
    }
    let rest = audio.len() - sent;
    tokio::time::sleep(Duration::from_millis(rest as u64 / 32)).await;
    let stop = Instant::now();
    let mut answer = finish_with(app, id, seq, audio[sent..].to_vec()).await;
    while answer.status == StatusCode::ACCEPTED {
        answer = read(app, id, dictation::MAX_READ_WAIT_MS).await;
    }
    let elapsed = stop.elapsed();
    assert_eq!(answer.status, StatusCode::OK, "{:?}", answer.body);
    (answer.body, elapsed)
}

/// PR-15's latency note, live, through the routes in this process: each bench clip
/// dictated in real time into a warm worker, and one 60 s dictation, on both catalog
/// models. `BIOROUTER_DICTATION_LIVE_MODELS` names a folder holding the catalog's model
/// files (read as an `env` folder, flat names allowed) and `BIOROUTER_DICTATION_LIVE_AUDIO`
/// a folder of 16 kHz mono WAV clips. Prints the figures; asserts only that text came
/// back.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
#[serial]
async fn stop_to_text_on_the_catalog_models() {
    let models = std::env::var("BIOROUTER_DICTATION_LIVE_MODELS")
        .expect("BIOROUTER_DICTATION_LIVE_MODELS names the model folder");
    let clips_dir = PathBuf::from(
        std::env::var("BIOROUTER_DICTATION_LIVE_AUDIO")
            .expect("BIOROUTER_DICTATION_LIVE_AUDIO names the clip folder"),
    );
    let mut clips: Vec<PathBuf> = std::fs::read_dir(&clips_dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            name.ends_with("_Daniel.wav") || name.ends_with("_Karen.wav")
        })
        .collect();
    clips.sort();
    clips.truncate(12);
    let _env = env_lock::lock_env([
        (biorouter::dictation::service::TEST_MODEL_ENV, None),
        (
            biorouter::dictation::paths::MODELS_DIR_ENV,
            Some(models.as_str()),
        ),
    ]);
    for model in ["whisper-small.en-q8_0", "whisper-large-v3-turbo-q5_0"] {
        biorouter::config::Config::global()
            .set_param(biorouter::dictation::service::MODEL_KEY, model)
            .unwrap();
        biorouter::dictation::service::service().unload().await;
        install_the_desktop_key();
        let state = AppState::new().await.expect("app state");
        let app = app(&state);

        // A cold worker: the stream starts it, and the clip is dictated at once.
        let cold = Instant::now();
        let id = open(&app, Caller::SecretOnly, request("external")).await;
        let (first, stop) = replay(&app, &id, &wav_samples(&clips[0])).await;
        let cold = cold.elapsed();
        delete(&app, &id).await;
        eprintln!(
            "{model}: first dictation into a stopped worker: {cold:?} from the create to the \
             text, {stop:?} from the stop ({})",
            first["timings"]
        );

        let mut stop_ms = Vec::new();
        let mut tail_ms = Vec::new();
        for clip in &clips {
            let audio = wav_samples(clip);
            let id = open(&app, Caller::SecretOnly, request("external")).await;
            let (transcript, elapsed) = replay(&app, &id, &audio).await;
            delete(&app, &id).await;
            stop_ms.push(elapsed.as_millis() as u64);
            tail_ms.push(transcript["timings"]["tail_ms"].as_u64().unwrap());
            eprintln!(
                "{model} {} {:.1} s: stop to text {elapsed:?}, {} -> {:?}",
                clip.file_name().unwrap().to_string_lossy(),
                audio.len() as f64 / 32_000.0,
                transcript["timings"],
                transcript["text"].as_str().unwrap()
            );
            assert!(!transcript["text"].as_str().unwrap().is_empty());
        }
        eprintln!(
            "{model}: stop to text median {} ms (min {}, max {}); tail decode median {} ms",
            median(&mut stop_ms.clone()),
            stop_ms.iter().min().unwrap(),
            stop_ms.iter().max().unwrap(),
            median(&mut tail_ms)
        );

        // A 60 s dictation, the clips end to end: with a 400 ms pause between sentences,
        // and with none (the synthetic voices then leave no pause the detector closes).
        for pause_ms in [400, 0] {
            let mut minute = Vec::new();
            for clip in clips.iter().cycle() {
                if minute.len() >= 60 * 32_000 {
                    break;
                }
                minute.extend(wav_samples(clip));
                minute.extend(std::iter::repeat_n(0u8, pause_ms * 32));
            }
            minute.truncate(60 * 32_000);
            let id = open(&app, Caller::SecretOnly, request("external")).await;
            let (transcript, elapsed) = replay(&app, &id, &minute).await;
            delete(&app, &id).await;
            eprintln!(
                "{model} 60 s, {pause_ms} ms pauses: stop to text {elapsed:?}, {}, {} characters",
                transcript["timings"],
                transcript["text"].as_str().unwrap().len()
            );
        }
        drop(state);
    }
}

/// PERF-1, live: after a dictation that used a term, the usage flush goes through the
/// daemon's one store handle. It reads the entries from that handle's warm cache under the
/// lock, where a fresh handle parsed all of `dictionary.json`, and leaves the counters it
/// wrote there, so the next stream parses neither file. In the same background task the
/// dictionary cache then copies the rows again for their new counters and keeps its
/// indexes (the PERF-1 review moved that copy there from the next stream's creation), so
/// the next stream copies nothing either. The fixture model decodes nothing, so this needs
/// a real one: `BIOROUTER_DICTATION_LIVE_MODELS` and `BIOROUTER_DICTATION_LIVE_AUDIO` as for
/// `stop_to_text_on_the_catalog_models`.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
#[serial]
async fn the_usage_flush_goes_through_the_shared_handle() {
    let models = std::env::var("BIOROUTER_DICTATION_LIVE_MODELS")
        .expect("BIOROUTER_DICTATION_LIVE_MODELS names the model folder");
    let clips_dir = PathBuf::from(
        std::env::var("BIOROUTER_DICTATION_LIVE_AUDIO")
            .expect("BIOROUTER_DICTATION_LIVE_AUDIO names the clip folder"),
    );
    let mut clips: Vec<PathBuf> = std::fs::read_dir(&clips_dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with("_Karen.wav"))
        .collect();
    clips.sort();
    let clip = wav_samples(clips.first().expect("a clip"));
    let _env = env_lock::lock_env([
        (biorouter::dictation::service::TEST_MODEL_ENV, None),
        (
            biorouter::dictation::paths::MODELS_DIR_ENV,
            Some(models.as_str()),
        ),
    ]);
    biorouter::config::Config::global()
        .set_param(
            biorouter::dictation::service::MODEL_KEY,
            "whisper-small.en-q8_0",
        )
        .unwrap();
    biorouter::dictation::service::service().unload().await;
    install_the_desktop_key();
    let state = AppState::new().await.expect("app state");
    let app = app(&state);
    drop(EmptiedStore);
    let _emptied = EmptiedStore;

    // The clip's own words, to make one of them a term.
    let heard = dictate(&app, Caller::SecretOnly, request("external"), clip.clone()).await;
    let text = heard["text"].as_str().unwrap().to_string();
    let word = text
        .split(|c: char| !c.is_alphanumeric())
        .find(|w| w.chars().count() >= 4)
        .unwrap_or_else(|| panic!("a word in {text:?}"))
        .to_string();
    let rows = store::transfer::parse_import(store::ImportFormat::Lines, &word).unwrap();
    open_store()
        .write(|file| {
            Ok(file.import(
                rows,
                store::EntryTier::Public,
                &|_| false,
                store::schema::now(),
            ))
        })
        .unwrap();
    let shared = biorouter_server::routes::dictionary::shared_store();
    let id = shared
        .view()
        .unwrap()
        .entries()
        .iter()
        .find(|e| e.written == word)
        .expect("the term")
        .id
        .clone();
    // One stream so the shared handle and the dictionary cache hold the new file.
    assert_eq!(
        eligible_terms(&state, &app, request("external")).await,
        std::slice::from_ref(&word)
    );

    let before = builds();
    let usage_reads = shared.usage_parses();
    let prewarms = dictation::dictionary_cache().prewarms();
    let used = dictate(&app, Caller::SecretOnly, request("external"), clip).await;
    assert!(
        used["text"].as_str().unwrap().contains(&word),
        "{word:?} in {:?}",
        used["text"]
    );
    // The flush runs after the text is out, and the cache's copy for the new counters
    // after it.
    let refreshed = prewarmed_after(prewarms).await;
    assert!(
        shared
            .view()
            .unwrap()
            .usage(&id, store::UsageScope::Public)
            .is_some(),
        "the usage was written"
    );
    assert_eq!(
        shared.usage_parses(),
        usage_reads,
        "the flush left its counters in the shared cache"
    );
    assert_eq!(refreshed.parses, before.parses, "and parsed no entries");
    assert_eq!(
        refreshed.copies,
        before.copies + 1,
        "the rows copied for the counters, after the text"
    );
    assert_eq!(refreshed.indexes, before.indexes, "and the indexes kept");
    let from = log_length();
    assert_eq!(
        eligible_terms(&state, &app, request("external")).await,
        std::slice::from_ref(&word)
    );
    assert_eq!(
        builds(),
        refreshed,
        "the next stream parses, copies and indexes nothing"
    );
    assert_eq!(shared.usage_parses(), usage_reads);
    eprintln!("the stream after the flush: {:?}", dictionary_timings(from));
    drop(state);
}
