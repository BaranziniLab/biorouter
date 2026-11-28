//! The dictation history (DD-96) on a daemon that holds a user action key, as the desktop
//! starts one: what a stream records, and what `/dictation/history` lists to whom.
//!
//! What is asserted, as `docs/dictation/testing-and-release.md` lists it for this binary:
//!
//! * a stream the person started (the proof) records one entry, with its surface, model,
//!   duration, tier, institutions and the chat's incarnation;
//! * a stream nobody proved records nothing: a caller holding only the secret, and one
//!   that states a private provider;
//! * cancelled and crossed streams, a `no_speech` or empty transcript and an external
//!   `dictionary_only` one record nothing, and neither does any stream while
//!   **Dictation history** is off or under the managed `history: false` key;
//! * the listing follows 7.1 of the history design: public entries to every holder of the
//!   secret, private entries to the proof alone, never to a stated private capability;
//! * a chat id is shown only for a chat the caller may list that still holds the
//!   recorded incarnation, so a publicized entry from a private chat loses its chat id for
//!   a public caller, and a reissued id links to nothing;
//! * every write needs the proof;
//! * an entry that cannot be written does not fail the dictation: the transcript is
//!   delivered, the history is left as it was, and the one line logged names the stream's
//!   log id and never the text (F12, DD-102).
//!
//! The worker is this test binary, speaking the worker protocol with a fixed transcript
//! (`synthetic_worker`), so every stream yields text without a speech model. The tests
//! share the process's one dictation service and history, so they run one at a time.

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::http::{header, Request, StatusCode};
use axum::Router;
use biorouter::dictation::history::{
    self as history, EntryTier, HistoryEntry, HistoryStore, Surface, TimeWindow,
};
use biorouter::model::ModelConfig;
use biorouter::privacy::SessionClassification;
use biorouter::security::dictionary_store::{open_store, store};
use biorouter::session::SessionType;
use biorouter_server::routes::{dictation, dictation_history};
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use tower::ServiceExt;

const SECRET: &str = "dictation-history-routes-secret";
const USER_KEY: &str = "dictation-history-routes-user-action-key";
/// What the synthetic worker answers every decode with.
const TEXT_ENV: &str = "BIOROUTER_HISTORY_SYNTH_TEXT";
/// How long the synthetic worker waits before it answers a decode.
const DELAY_ENV: &str = "BIOROUTER_HISTORY_SYNTH_DELAY_MS";
/// The marker the synthetic worker answers to.
const WORKER_MARKER_ENV: &str = "BIOROUTER_HISTORY_SYNTH_WORKER";
const SPOKEN: &str = "Stanfordine";

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/whisper.cpp/source")
        .join(relative)
}

/// The worker is this test binary and the model the debug only `for-tests` fixture, which
/// the synthetic worker never reads. Set before `main`, so every spawn sees them.
#[ctor::ctor]
fn dictate_with_the_synthetic_worker() {
    if let Ok(exe) = std::env::current_exe() {
        std::env::set_var(biorouter::dictation::service::WORKER_ENV, exe);
    }
    std::env::set_var(
        biorouter::dictation::service::TEST_MODEL_ENV,
        fixture("models/for-tests-ggml-base.en.bin"),
    );
    std::env::set_var(WORKER_MARKER_ENV, "1");
}

/// A child of this test binary speaks the worker frame protocol: every decode answers
/// [`TEXT_ENV`]'s text (or [`SPOKEN`]) after [`DELAY_ENV`] milliseconds, and the voice
/// detector finds no speech, so a short stream's tail goes straight to a decode.
#[ctor::ctor]
fn synthetic_worker() {
    if std::env::var_os(WORKER_MARKER_ENV).is_none()
        || std::env::args_os().nth(1).as_deref()
            != Some(std::ffi::OsStr::new(
                biorouter_dictation::worker::WORKER_MARKER,
            ))
    {
        return;
    }
    use biorouter_dictation::protocol::{
        self, BiasReady, CpuFlags, FromWorker, Hello, Loaded, SegmentText, Segments, Timings,
        ToWorker, TranscribeResult,
    };
    let text = std::env::var(TEXT_ENV).unwrap_or_else(|_| SPOKEN.to_string());
    let delay = std::env::var(DELAY_ENV)
        .ok()
        .and_then(|ms| ms.parse::<u64>().ok())
        .map(Duration::from_millis);
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    let send = |output: &mut std::io::StdoutLock<'_>, message: FromWorker| {
        protocol::write_frame_blocking(output, &protocol::to_json(&message), &[])
            .expect("synthetic worker frame");
    };
    send(
        &mut output,
        FromWorker::Hello(Hello {
            protocol: protocol::PROTOCOL_VERSION,
            whisper: biorouter_dictation::build_info::WHISPER_COMMIT.into(),
            ggml: biorouter_dictation::build_info::GGML_VERSION.into(),
            backends: vec!["cpu".into()],
            cpu: CpuFlags::from(biorouter_dictation::cpu::detect()),
            isa: biorouter_dictation::build_info::ISA.into(),
            system_info: String::new(),
        }),
    );
    while let Ok(frame) = protocol::read_frame_blocking(&mut input) {
        let message: ToWorker = serde_json::from_slice(&frame.json).expect("worker request");
        match message {
            ToWorker::Load(_) => send(
                &mut output,
                FromWorker::Loaded(Loaded {
                    load_ms: 0,
                    warmup_ms: 0,
                    backend: "cpu".into(),
                    device: None,
                    rss_bytes: None,
                    n_vocab: 51_866,
                    multilingual: true,
                }),
            ),
            ToWorker::SetBias(bias) => send(
                &mut output,
                FromWorker::BiasReady(BiasReady {
                    revision: bias.revision,
                    scope: bias.scope,
                    nodes: 0,
                    build_ms: 0,
                }),
            ),
            ToWorker::Transcribe(request) => {
                if let Some(delay) = delay {
                    std::thread::sleep(delay);
                }
                send(
                    &mut output,
                    FromWorker::Result(TranscribeResult {
                        id: request.id,
                        text: text.clone(),
                        segments: vec![SegmentText {
                            t0_ms: 0,
                            t1_ms: 800,
                            text: text.clone(),
                        }],
                        prompt_terms_used: 0,
                        timings: Timings::default(),
                        language: None,
                    }),
                )
            }
            ToWorker::VadSegments(request) => send(
                &mut output,
                FromWorker::Segments(Segments {
                    id: request.id,
                    spans: Vec::new(),
                }),
            ),
            ToWorker::Cancel { .. } => {}
            ToWorker::Shutdown | ToWorker::Unknown => break,
        }
    }
    std::process::exit(0);
}

/// Whether this build and this processor can run the engine.
fn engine_runs() -> bool {
    biorouter::dictation::service::spawn_refusal().is_none()
}

fn install_the_desktop_key() {
    let digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(USER_KEY.as_bytes()).into();
    biorouter_server::auth::install_user_action_digest(Some(digest));
}

/// The history this binary's sandbox holds, emptied, with its switches at their defaults.
fn empty_history() -> Arc<HistoryStore> {
    let _ = std::fs::remove_dir_all(history::history_dir());
    history::open_history()
}

/// Every entry, oldest first.
fn entries() -> Vec<HistoryEntry> {
    let mut page = history::open_history()
        .search(None, TimeWindow::all(), None, 10_000, &|_| true)
        .expect("the history reads");
    page.entries.reverse();
    page.entries
}

/// Waits until the history holds `count` entries: the daemon appends off the decode's
/// task, after the answer.
async fn until_entries(count: usize) -> Vec<HistoryEntry> {
    let started = Instant::now();
    loop {
        let held = entries();
        if held.len() >= count {
            return held;
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the history holds {} entries, not {count}",
            held.len()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The daemon, with its worker loaded on this test's runtime.
async fn state() -> Arc<AppState> {
    install_the_desktop_key();
    let state = AppState::new().await.expect("app state");
    biorouter::dictation::service::service()
        .prepare(None)
        .await
        .expect("the synthetic worker loads");
    state
}

/// The dictation and history routes behind the daemon's own secret check.
fn app(state: &Arc<AppState>) -> Router {
    Router::new()
        .merge(dictation::routes(Arc::clone(state)))
        .merge(dictation_history::routes(Arc::clone(state)))
        .layer(axum::middleware::from_fn_with_state(
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

const CALLERS: [Caller; 3] = [Caller::SecretOnly, Caller::Proof, Caller::StatedPrivate];

struct Answer {
    status: StatusCode,
    raw: Bytes,
    body: Value,
}

async fn call(app: &Router, method: &str, uri: &str, caller: Caller, body: Body) -> Answer {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Secret-Key", SECRET)
        .header(header::CONTENT_TYPE, "application/json");
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
    let raw = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    Answer { status, raw, body }
}

async fn post(app: &Router, uri: &str, caller: Caller, body: Value) -> Answer {
    call(app, "POST", uri, caller, Body::from(body.to_string())).await
}

async fn get(app: &Router, uri: &str, caller: Caller) -> Answer {
    call(app, "GET", uri, caller, Body::empty()).await
}

fn request(target_kind: &str) -> Value {
    json!({"sample_rate": 16000, "format": "s16le", "target_kind": target_kind})
}

async fn open(app: &Router, caller: Caller, request: Value) -> String {
    let answer = post(app, "/dictation/streams", caller, request).await;
    assert_eq!(
        answer.status,
        StatusCode::CREATED,
        "{:?}",
        String::from_utf8_lossy(&answer.raw)
    );
    answer.body["stream_id"].as_str().unwrap().to_string()
}

async fn delete_stream(app: &Router, id: &str) -> Answer {
    call(
        app,
        "DELETE",
        &format!("/dictation/streams/{id}"),
        Caller::SecretOnly,
        Body::empty(),
    )
    .await
}

/// One whole dictation: create, `finish` carrying the audio, held reads while it answers
/// 202, delete. Answers the transcript.
async fn dictate(app: &Router, caller: Caller, request: Value, audio: Vec<u8>) -> Value {
    let started = Instant::now();
    let id = open(app, caller, request).await;
    let mut answer = call(
        app,
        "POST",
        &format!("/dictation/streams/{id}/finish?seq=0"),
        Caller::SecretOnly,
        Body::from(audio),
    )
    .await;
    while answer.status == StatusCode::ACCEPTED && started.elapsed() < Duration::from_secs(60) {
        answer = get(
            app,
            &format!(
                "/dictation/streams/{id}?wait_ms={}",
                dictation::MAX_READ_WAIT_MS
            ),
            Caller::SecretOnly,
        )
        .await;
    }
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "{:?}",
        String::from_utf8_lossy(&answer.raw)
    );
    assert_eq!(delete_stream(app, &id).await.status, StatusCode::NO_CONTENT);
    answer.body
}

/// `seconds` of audio that is almost silence: a quiet hum, never exact zeros (a tail of
/// zeros is answered without a decode, as `no_speech`).
fn hum(seconds: f64) -> Vec<u8> {
    let samples = (seconds * 16_000.0) as usize;
    (0..samples)
        .flat_map(|i| (((i % 16) as i16) - 8).to_le_bytes())
        .collect()
}

/// `seconds` of exact zeros.
fn zeros(seconds: f64) -> Vec<u8> {
    vec![0; (seconds * 32_000.0) as usize]
}

async fn seed_chat(state: &Arc<AppState>, private: bool) -> String {
    let manager = state.session_manager();
    let chat = manager
        .create_session(
            PathBuf::from("/tmp/dictation_history_routes"),
            "dictation history target (test fixture)".to_string(),
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

async fn incarnation(state: &Arc<AppState>, id: &str) -> i64 {
    state
        .session_manager()
        .session_incarnation(id)
        .await
        .unwrap()
        .expect("the chat has an incarnation")
}

fn chat_request(session_id: &str) -> Value {
    let mut body = request("chat");
    body["session_id"] = json!(session_id);
    body
}

/// An entry written straight into the store, as a stream would have recorded it.
fn seeded(text: &str, tier: EntryTier) -> HistoryEntry {
    HistoryEntry {
        id: history::store::new_entry_id(),
        created_at: chrono::Utc::now(),
        text: text.to_string(),
        tier,
        institutions: None,
        surface: Surface::Clipboard,
        duration_ms: 1_000,
        model: "for-tests".to_string(),
        private_terms: tier.is_private(),
        session_id: None,
        session_incarnation: None,
        truncated: None,
    }
}

fn add(entry: HistoryEntry) -> String {
    let id = entry.id.clone();
    history::open_history().append(entry).expect("appended");
    id
}

fn add_dictionary_row(written: &str, tier: store::EntryTier, institution: Option<&str>) {
    let dictionary = open_store();
    let id = match dictionary
        .write(|file| {
            file.add(
                store::NewTerm::person(written, tier),
                store::Reach::All,
                store::schema::now(),
            )
        })
        .expect("added")
    {
        store::AddOutcome::Created { id, .. } => id,
        other => panic!("{other:?}"),
    };
    if let Some(institution) = institution {
        dictionary
            .write(|file| {
                let entry = file.entries.iter_mut().find(|e| e.id == id).unwrap();
                entry.affiliation = Some(store::RowAffiliation {
                    institutions: vec![institution.to_string()],
                });
                Ok(())
            })
            .expect("stamped");
    }
}

/// Empties the dictionary when dropped, so a failed assertion leaves no row behind.
struct EmptiedDictionary;

impl Drop for EmptiedDictionary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(biorouter::security::dictionary_store::dictionary_dir());
    }
}

fn texts(listing: &Value) -> Vec<String> {
    listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["text"].as_str().unwrap().to_string())
        .collect()
}

fn listed<'a>(listing: &'a Value, id: &str) -> &'a Value {
    listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == id)
        .unwrap_or_else(|| panic!("{id} is not listed: {listing}"))
}

// ── recording ────────────────────────────────────────────────────────────────────────

/// A stream the person started records one entry: the corrected text, the surface the
/// target implies, the model, the duration, the default private tier, the chat with its
/// incarnation, and the institutions a private row written there would record (none
/// could be read here, since the chat has no live agent, so a model on this computer
/// only). A stream outside any chat names none.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_proven_stream_records_one_entry_with_its_facts() {
    if !engine_runs() {
        return;
    }
    empty_history();
    drop(EmptiedDictionary);
    let _emptied = EmptiedDictionary;
    let state = state().await;
    let app = app(&state);
    let chat = seed_chat(&state, false).await;

    let transcript = dictate(&app, Caller::Proof, chat_request(&chat), hum(1.0)).await;
    assert_eq!(transcript["text"], SPOKEN);
    let held = until_entries(1).await;
    assert_eq!(held.len(), 1, "one stream, one entry");
    let entry = &held[0];
    assert!(history::store::is_entry_id(&entry.id));
    assert_eq!(entry.text, SPOKEN);
    assert_eq!(entry.surface, Surface::Composer);
    assert_eq!(entry.model, transcript["model"].as_str().unwrap());
    let audio_ms = (transcript["audio_s"].as_f64().unwrap() * 1000.0).round() as u64;
    assert_eq!(entry.duration_ms, audio_ms);
    assert_eq!(entry.tier, EntryTier::Private, "private by default");
    assert_eq!(entry.institutions, Some(Vec::new()));
    assert!(!entry.private_terms);
    assert_eq!(entry.session_id.as_deref(), Some(chat.as_str()));
    assert_eq!(
        entry.session_incarnation,
        Some(incarnation(&state, &chat).await)
    );
    assert_eq!(entry.created_at.timestamp_subsec_nanos(), 0);

    // Outside any chat: the clipboard, no chat, no institution.
    dictate(&app, Caller::Proof, request("external"), hum(1.0)).await;
    let held = until_entries(2).await;
    let outside = &held[1];
    assert_eq!(outside.surface, Surface::Clipboard);
    assert_eq!(outside.tier, EntryTier::Private);
    assert_eq!(outside.institutions, None);
    assert_eq!(outside.session_id, None);
    assert_eq!(outside.session_incarnation, None);

    // The surface the client names wins over the one the target implies.
    let mut preview = request("external");
    preview["surface"] = json!("app_preview");
    dictate(&app, Caller::Proof, preview, hum(1.0)).await;
    let mut unknown = request("crew");
    unknown["surface"] = json!("a-surface-from-a-later-build");
    dictate(&app, Caller::Proof, unknown, hum(1.0)).await;
    let held = until_entries(4).await;
    assert_eq!(held[2].surface, Surface::AppPreview);
    assert_eq!(held[3].surface, Surface::Crew);
}

/// With **Save new dictations as public** on, a stream into a public chat is public and a
/// stream that had private terms eligible stays private, carrying those terms'
/// institutions (P-16).
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn public_by_default_keeps_private_terms_private_with_their_institutions() {
    if !engine_runs() {
        return;
    }
    let store = empty_history();
    drop(EmptiedDictionary);
    let _emptied = EmptiedDictionary;
    store
        .set_settings(history::SettingsChange {
            keep_history: None,
            public_by_default: Some(true),
        })
        .unwrap();
    add_dictionary_row("Zymoquine", store::EntryTier::Private, Some("stanford"));
    let state = state().await;
    let app = app(&state);
    let chat = seed_chat(&state, false).await;

    dictate(&app, Caller::Proof, chat_request(&chat), hum(1.0)).await;
    let mut with_terms = request("external");
    with_terms["include_private_terms"] = json!(true);
    let transcript = dictate(&app, Caller::Proof, with_terms, hum(1.0)).await;
    assert_eq!(transcript["bias_scope"], "private");
    let held = until_entries(2).await;
    assert_eq!(held[0].tier, EntryTier::Public);
    assert_eq!(held[0].institutions, None);
    assert!(!held[0].private_terms);
    assert_eq!(held[1].tier, EntryTier::Private);
    assert_eq!(held[1].institutions, Some(vec!["stanford".to_string()]));
    assert!(held[1].private_terms);
}

/// A stream nobody proved leaves no entry: the secret alone, and a stated private
/// provider, which is a model's capability and not a person.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_stream_nobody_proved_records_nothing() {
    if !engine_runs() {
        return;
    }
    empty_history();
    let state = state().await;
    let app = app(&state);
    for caller in [Caller::SecretOnly, Caller::StatedPrivate] {
        let transcript = dictate(&app, caller, request("external"), hum(1.0)).await;
        assert_eq!(transcript["text"], SPOKEN, "{caller:?} still gets its text");
    }
    // The fence: a proven stream after them is the only entry once it lands.
    dictate(&app, Caller::Proof, request("external"), hum(1.0)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(until_entries(1).await.len(), 1);
}

/// Nothing is kept of a stream cancelled before its transcript, a `no_speech` one, or an
/// external one whose text is only dictionary terms (an echo of the prompt).
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn cancelled_silent_and_echo_streams_record_nothing() {
    if !engine_runs() {
        return;
    }
    empty_history();
    drop(EmptiedDictionary);
    let _emptied = EmptiedDictionary;
    let state = state().await;
    let app = app(&state);

    // Cancelled: deleted before `finish`.
    let id = open(&app, Caller::Proof, request("external")).await;
    let put = call(
        &app,
        "PUT",
        &format!("/dictation/streams/{id}/audio?seq=0"),
        Caller::SecretOnly,
        Body::from(hum(1.0)),
    )
    .await;
    assert_eq!(put.status, StatusCode::OK);
    assert_eq!(
        delete_stream(&app, &id).await.status,
        StatusCode::NO_CONTENT
    );

    // No speech: exact zeros are answered without a decode.
    let silent = dictate(&app, Caller::Proof, request("external"), zeros(1.0)).await;
    assert_eq!(silent["no_speech"], true);

    // An echo: the text is one public dictionary term, for an external target.
    add_dictionary_row(SPOKEN, store::EntryTier::Public, None);
    let echo = dictate(&app, Caller::Proof, request("external"), hum(1.0)).await;
    assert_eq!(echo["dictionary_only"], true);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(entries().is_empty(), "{:?}", entries());

    // The fence: the same daemon records a plain proven stream into a chat, where the
    // marker is never computed.
    dictate(&app, Caller::Proof, request("crew"), hum(1.0)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(until_entries(1).await.len(), 1);
}

/// An engine that writes only blanks leaves an empty transcript, and nothing is kept.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn an_empty_transcript_records_nothing() {
    if !engine_runs() {
        return;
    }
    empty_history();
    let _env = env_lock::lock_env([(TEXT_ENV, Some("   "))]);
    let state = state().await;
    let app = app(&state);
    let empty = dictate(&app, Caller::Proof, request("chat"), hum(1.0)).await;
    assert_eq!(empty["text"], "");
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    assert!(entries().is_empty(), "{:?}", entries());
}

/// A stream whose `DELETE` lands while its decode still runs keeps nothing; the next
/// stream on the same slow engine is kept.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_crossed_stream_records_nothing() {
    if !engine_runs() {
        return;
    }
    empty_history();
    let _env = env_lock::lock_env([(DELAY_ENV, Some("4500"))]);
    let state = state().await;
    let app = app(&state);
    let id = open(&app, Caller::Proof, request("crew")).await;
    let finish = call(
        &app,
        "POST",
        &format!("/dictation/streams/{id}/finish?seq=0"),
        Caller::SecretOnly,
        Body::from(hum(1.0)),
    )
    .await;
    assert_eq!(
        finish.status,
        StatusCode::ACCEPTED,
        "the decode is still running"
    );
    assert_eq!(
        delete_stream(&app, &id).await.status,
        StatusCode::NO_CONTENT
    );
    tokio::time::sleep(Duration::from_millis(5_000)).await;
    assert!(entries().is_empty(), "{:?}", entries());

    // The fence.
    dictate(&app, Caller::Proof, request("crew"), hum(1.0)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(until_entries(1).await.len(), 1);
}

/// **Dictation history** off keeps nothing, and so does the managed `history: false` key,
/// which `GET /dictation/status` reports in `policy.history`.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn history_off_and_managed_history_false_record_nothing() {
    if !engine_runs() {
        return;
    }
    let store = empty_history();
    let state = state().await;
    let app = app(&state);

    store
        .set_settings(history::SettingsChange {
            keep_history: Some(false),
            public_by_default: None,
        })
        .unwrap();
    dictate(&app, Caller::Proof, request("external"), hum(1.0)).await;
    store
        .set_settings(history::SettingsChange {
            keep_history: Some(true),
            public_by_default: None,
        })
        .unwrap();

    #[cfg(unix)]
    {
        let _policy = StagedPolicy::new("dictation:\n  history: false\n");
        let status = get(&app, "/dictation/status", Caller::SecretOnly).await;
        assert_eq!(status.body["policy"]["history"]["allowed"], false);
        assert!(status.body["policy"]["history"]["reason"].is_string());
        dictate(&app, Caller::Proof, request("external"), hum(1.0)).await;
        // Listing stays open under the key.
        let listing = get(&app, "/dictation/history", Caller::Proof).await;
        assert_eq!(listing.status, StatusCode::OK);
        let settings = get(&app, "/dictation/history/settings", Caller::Proof).await;
        assert_eq!(settings.body["authority"]["write"]["allowed"], true);
        assert_eq!(settings.body["authority"]["tier_change"]["allowed"], false);
        assert_eq!(
            settings.body["authority"]["raise_settings"]["allowed"],
            false
        );
    }
    let status = get(&app, "/dictation/status", Caller::SecretOnly).await;
    assert_eq!(status.body["policy"]["history"]["allowed"], true);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(entries().is_empty(), "{:?}", entries());

    // The fence.
    dictate(&app, Caller::Proof, request("external"), hum(1.0)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(until_entries(1).await.len(), 1);
}

/// A managed policy staged where the daemon reads it under this binary's sandbox root,
/// trusted (0644 in a 0755 folder), and removed when dropped.
#[cfg(unix)]
struct StagedPolicy(PathBuf);

#[cfg(unix)]
impl StagedPolicy {
    fn new(yaml: &str) -> StagedPolicy {
        use std::os::unix::fs::PermissionsExt;
        let root = PathBuf::from(std::env::var("BIOROUTER_PATH_ROOT").expect("the sandbox root"));
        let dir = root.join("managed");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let file = dir.join("managed-policy.yaml");
        std::fs::write(&file, yaml).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        StagedPolicy(file)
    }
}

#[cfg(unix)]
impl Drop for StagedPolicy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Everything this binary logs from the moment a test asks, for the log hygiene of an entry
/// that cannot be written.
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

/// What was logged after `mark` bytes.
fn logged_since(mark: usize) -> String {
    let logs = captured_logs()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    String::from_utf8_lossy(logs.get(mark..).unwrap_or_default()).into_owned()
}

/// Makes the history file unwritable while it lives, and puts it back as it was after. A
/// read-only file, or, for a process that may write one anyway (one running as root), a
/// folder where the file was, which cannot be appended to either.
struct UnwritableHistory {
    file: PathBuf,
    permissions: std::fs::Permissions,
    kept: Vec<u8>,
    as_folder: bool,
}

impl UnwritableHistory {
    fn make(file: &Path) -> Self {
        let kept = std::fs::read(file).expect("the history file exists");
        let permissions = std::fs::metadata(file).unwrap().permissions();
        let mut read_only = permissions.clone();
        read_only.set_readonly(true);
        std::fs::set_permissions(file, read_only).unwrap();
        let as_folder = std::fs::OpenOptions::new().append(true).open(file).is_ok();
        if as_folder {
            std::fs::set_permissions(file, permissions.clone()).unwrap();
            std::fs::remove_file(file).unwrap();
            std::fs::create_dir(file).unwrap();
        }
        Self {
            file: file.to_path_buf(),
            permissions,
            kept,
            as_folder,
        }
    }
}

impl Drop for UnwritableHistory {
    fn drop(&mut self) {
        if self.as_folder {
            let _ = std::fs::remove_dir(&self.file);
            let _ = std::fs::write(&self.file, &self.kept);
        } else {
            let _ = std::fs::set_permissions(&self.file, self.permissions.clone());
        }
    }
}

/// The name a log line gives a stream: the first 8 hex characters of its id's SHA-256, as
/// `log_id_of` in `routes/dictation.rs` makes it.
fn log_id(stream_id: &str) -> String {
    <sha2::Sha256 as sha2::Digest>::digest(stream_id.as_bytes())[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// F12 (W-A6): a history that cannot take the entry never fails the dictation. The
/// transcript is delivered whole, the history is left as it was, and the stream logs one
/// fixed warning that names its log id; no line it logs carries the text, or any word of it.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn an_unwritable_history_still_delivers_the_dictation_and_logs_no_text() {
    if !engine_runs() {
        return;
    }
    const SAID: &str = "Quillborough tamarind lantern";
    let _ = captured_logs();
    empty_history();
    let _env = env_lock::lock_env([(TEXT_ENV, Some(SAID))]);
    let kept = add(seeded("an entry kept before", EntryTier::Public));
    let file = history::history_dir().join(history::store::HISTORY_FILE);
    let before = std::fs::read(&file).expect("the history file exists");
    let unwritable = UnwritableHistory::make(&file);
    let state = state().await;
    let app = app(&state);
    let mark = captured_logs().lock().unwrap().len();

    let started = Instant::now();
    let id = open(&app, Caller::Proof, request("crew")).await;
    let mut answer = call(
        &app,
        "POST",
        &format!("/dictation/streams/{id}/finish?seq=0"),
        Caller::SecretOnly,
        Body::from(hum(1.0)),
    )
    .await;
    while answer.status == StatusCode::ACCEPTED && started.elapsed() < Duration::from_secs(60) {
        answer = get(
            &app,
            &format!(
                "/dictation/streams/{id}?wait_ms={}",
                dictation::MAX_READ_WAIT_MS
            ),
            Caller::SecretOnly,
        )
        .await;
    }
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "{:?}",
        String::from_utf8_lossy(&answer.raw)
    );
    assert_eq!(answer.body["text"], SAID, "the dictation is delivered");

    // The entry is dropped off the decode's task, after the answer.
    const DROPPED: &str = "dictation history entry dropped";
    let waiting = Instant::now();
    while !logged_since(mark).contains(DROPPED) {
        assert!(
            waiting.elapsed() < Duration::from_secs(30),
            "no line says the entry was dropped: {}",
            logged_since(mark)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        delete_stream(&app, &id).await.status,
        StatusCode::NO_CONTENT
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let logged = logged_since(mark);
    let dropped: Vec<&str> = logged
        .lines()
        .filter(|line| line.contains(DROPPED))
        .collect();
    assert_eq!(dropped.len(), 1, "one line: {logged}");
    assert!(dropped[0].contains("WARN"), "{}", dropped[0]);
    assert!(
        dropped[0].contains(&format!("stream={}", log_id(&id))),
        "the line names the stream's log id: {}",
        dropped[0]
    );
    assert!(
        !logged.contains(&id),
        "no line carries the stream id itself"
    );
    for word in SAID.split(' ') {
        assert!(
            !logged.to_lowercase().contains(&word.to_lowercase()),
            "a line carries {word:?}: {logged}"
        );
    }

    drop(unwritable);
    assert_eq!(
        std::fs::read(&file).unwrap(),
        before,
        "the history is left as it was"
    );
    let held: Vec<String> = entries().into_iter().map(|entry| entry.id).collect();
    assert_eq!(held, vec![kept]);
}

// ── listing ──────────────────────────────────────────────────────────────────────────

/// 7.1 of the history design: public entries to every holder of the secret, private
/// entries to the proof alone; a stated private capability gets public entries only.
/// `total` counts what the caller may list, and `private_terms` is shown only to a caller
/// that may list private entries.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_listing_shows_private_entries_to_the_person_alone() {
    empty_history();
    let state = AppState::new().await.expect("app state");
    install_the_desktop_key();
    let app = app(&state);
    let public = add(seeded("public words about EPAS1", EntryTier::Public));
    let private = add(seeded("private words about EPAS1", EntryTier::Private));

    let person = get(&app, "/dictation/history", Caller::Proof).await;
    assert_eq!(person.status, StatusCode::OK);
    assert_eq!(person.body["total"], 2);
    assert_eq!(
        texts(&person.body),
        ["private words about EPAS1", "public words about EPAS1"],
        "newest first"
    );
    assert_eq!(listed(&person.body, &private)["private_terms"], true);
    assert_eq!(listed(&person.body, &public)["private_terms"], false);
    assert!(listed(&person.body, &private).get("institutions").is_none());
    assert!(listed(&person.body, &private)
        .get("session_incarnation")
        .is_none());

    for caller in [Caller::SecretOnly, Caller::StatedPrivate] {
        let listing = get(&app, "/dictation/history", caller).await;
        assert_eq!(listing.status, StatusCode::OK, "{caller:?}");
        assert_eq!(listing.body["total"], 1, "{caller:?}");
        assert_eq!(
            texts(&listing.body),
            ["public words about EPAS1"],
            "{caller:?}"
        );
        assert!(
            listed(&listing.body, &public)
                .get("private_terms")
                .is_none(),
            "{caller:?}"
        );
        let settings = get(&app, "/dictation/history/settings", caller).await;
        assert_eq!(
            settings.body["authority"]["list_private"]["allowed"], false,
            "{caller:?}"
        );
        assert!(settings.body["authority"]["list_private"]["reason"]
            .as_str()
            .unwrap()
            .starts_with("Private dictations are shown only to you in Biorouter on "));
    }

    // A search runs before the page is counted, for every caller.
    let found = get(&app, "/dictation/history?q=PRIVATE%20words", Caller::Proof).await;
    assert_eq!(texts(&found.body), ["private words about EPAS1"]);
    let hidden = get(
        &app,
        "/dictation/history?q=private%20words",
        Caller::SecretOnly,
    )
    .await;
    assert_eq!(hidden.body["total"], 0);
    assert!(texts(&hidden.body).is_empty());

    // Pages walk newest first and the cursor resumes after the last entry shown.
    let first = get(&app, "/dictation/history?limit=1", Caller::Proof).await;
    assert_eq!(texts(&first.body), ["private words about EPAS1"]);
    let cursor = first.body["next_cursor"].as_str().unwrap().to_string();
    let second = get(
        &app,
        &format!("/dictation/history?limit=1&cursor={cursor}"),
        Caller::Proof,
    )
    .await;
    assert_eq!(texts(&second.body), ["public words about EPAS1"]);
    assert!(second.body["next_cursor"].is_null());
    // A cursor this daemon did not give, and a search over 200 characters, are refused.
    let bad = get(&app, "/dictation/history?cursor=nonsense", Caller::Proof).await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
    let long = get(
        &app,
        &format!("/dictation/history?q={}", "a".repeat(201)),
        Caller::Proof,
    )
    .await;
    assert_eq!(long.status, StatusCode::UNPROCESSABLE_ENTITY);
}

/// A chat id is shown only while the chat holding it is the one the entry recorded and
/// the caller may list it: a private chat's id never reaches a public caller, not even on
/// an entry the person made public, and an id now held by another chat (a reissued id),
/// or by none, is shown to nobody.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_chat_id_is_shown_only_for_a_listable_chat_that_still_holds_its_incarnation() {
    empty_history();
    let state = AppState::new().await.expect("app state");
    install_the_desktop_key();
    let app = app(&state);
    let public_chat = seed_chat(&state, false).await;
    let private_chat = seed_chat(&state, true).await;
    let linked = |text: &str, chat: &str, incarnation: i64| HistoryEntry {
        session_id: Some(chat.to_string()),
        session_incarnation: Some(incarnation),
        surface: Surface::Composer,
        ..seeded(text, EntryTier::Public)
    };
    let public_incarnation = incarnation(&state, &public_chat).await;
    let private_incarnation = incarnation(&state, &private_chat).await;
    let in_public = add(linked(
        "into the public chat",
        &public_chat,
        public_incarnation,
    ));
    let reissued = add(linked(
        "into a chat whose id was reissued",
        &public_chat,
        public_incarnation.wrapping_add(1),
    ));
    let gone = add(linked("into a chat that is gone", "20000101_1", 7));
    // Dictated into the private chat, private, then made public by the person.
    let publicized = add(HistoryEntry {
        tier: EntryTier::Private,
        ..linked(
            "into the private chat, then made public",
            &private_chat,
            private_incarnation,
        )
    });
    let made_public = post(
        &app,
        &format!("/dictation/history/{publicized}/tier"),
        Caller::Proof,
        json!({"tier": "public"}),
    )
    .await;
    assert_eq!(made_public.status, StatusCode::OK, "{}", made_public.body);
    assert_eq!(made_public.body["entry"]["tier"], "public");
    assert_eq!(
        made_public.body["entry"]["session_id"],
        private_chat.as_str()
    );

    let person = get(&app, "/dictation/history", Caller::Proof).await;
    assert_eq!(
        listed(&person.body, &in_public)["session_id"],
        public_chat.as_str()
    );
    assert_eq!(
        listed(&person.body, &publicized)["session_id"],
        private_chat.as_str()
    );
    for id in [&reissued, &gone] {
        assert!(listed(&person.body, id).get("session_id").is_none());
    }
    for caller in [Caller::SecretOnly, Caller::StatedPrivate] {
        let listing = get(&app, "/dictation/history", caller).await;
        assert_eq!(
            listing.body["total"], 4,
            "{caller:?}: every entry is public now"
        );
        assert_eq!(
            listed(&listing.body, &in_public)["session_id"],
            public_chat.as_str(),
            "{caller:?}"
        );
        for id in [&reissued, &gone] {
            assert!(
                listed(&listing.body, id).get("session_id").is_none(),
                "{caller:?}"
            );
        }
        if matches!(caller, Caller::SecretOnly) {
            assert!(
                listed(&listing.body, &publicized)
                    .get("session_id")
                    .is_none(),
                "a private chat's id reached a public caller"
            );
        }
    }
}

/// On a daemon that holds a key every write needs the proof, and the tier route refuses
/// before it reads the id, so a missing id and a real one answer alike.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn every_write_needs_the_proof() {
    empty_history();
    let state = AppState::new().await.expect("app state");
    install_the_desktop_key();
    let app = app(&state);
    let public = add(seeded("a public dictation", EntryTier::Public));
    let missing = history::store::new_entry_id();
    for caller in [Caller::SecretOnly, Caller::StatedPrivate] {
        let refused = |answer: &Answer| {
            assert_eq!(answer.status, StatusCode::FORBIDDEN, "{caller:?}");
            assert_eq!(answer.body["code"], "agent", "{caller:?}");
            assert_eq!(
                answer.body["message"],
                dictation_history::MSG_HISTORY_AGENT,
                "{caller:?}"
            );
        };
        refused(
            &call(
                &app,
                "DELETE",
                &format!("/dictation/history/{public}"),
                caller,
                Body::empty(),
            )
            .await,
        );
        refused(
            &post(
                &app,
                "/dictation/history/delete",
                caller,
                json!({"ids": [public]}),
            )
            .await,
        );
        refused(
            &post(
                &app,
                "/dictation/history/clear",
                caller,
                json!({"through": chrono::Utc::now()}),
            )
            .await,
        );
        let real = post(
            &app,
            &format!("/dictation/history/{public}/tier"),
            caller,
            json!({"tier": "private"}),
        )
        .await;
        let made_up = post(
            &app,
            &format!("/dictation/history/{missing}/tier"),
            caller,
            json!({"tier": "private"}),
        )
        .await;
        refused(&real);
        assert_eq!(real.raw, made_up.raw, "{caller:?}");
        refused(
            &post(
                &app,
                "/dictation/history/settings",
                caller,
                json!({"keep_history": false}),
            )
            .await,
        );
    }
    assert_eq!(entries().len(), 1, "nothing was deleted");

    // The person: a tier change, then the settings, then a delete.
    let private = post(
        &app,
        &format!("/dictation/history/{public}/tier"),
        Caller::Proof,
        json!({"tier": "private"}),
    )
    .await;
    assert_eq!(private.status, StatusCode::OK);
    assert_eq!(private.body["entry"]["tier"], "private");
    let settings = post(
        &app,
        "/dictation/history/settings",
        Caller::Proof,
        json!({"keep_history": false, "public_by_default": true}),
    )
    .await;
    assert_eq!(settings.status, StatusCode::OK, "{}", settings.body);
    assert_eq!(settings.body["keep_history"], false);
    assert_eq!(settings.body["public_by_default"], true);
    let settings = post(
        &app,
        "/dictation/history/settings",
        Caller::Proof,
        json!({"keep_history": true, "public_by_default": false}),
    )
    .await;
    assert_eq!(settings.body["keep_history"], true);
    assert_eq!(settings.body["public_by_default"], false);
    let deleted = call(
        &app,
        "DELETE",
        &format!("/dictation/history/{public}"),
        Caller::Proof,
        Body::empty(),
    )
    .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert!(entries().is_empty());

    // Bulk delete and clear answer how many went.
    let a = add(seeded("first", EntryTier::Public));
    let _b = add(seeded("second", EntryTier::Private));
    let bulk = post(
        &app,
        "/dictation/history/delete",
        Caller::Proof,
        json!({"ids": [a, missing, "not-an-id"]}),
    )
    .await;
    assert_eq!(bulk.body["deleted"], 1);
    let cleared = post(
        &app,
        "/dictation/history/clear",
        Caller::Proof,
        json!({"through": chrono::Utc::now() + chrono::Duration::seconds(1)}),
    )
    .await;
    assert_eq!(cleared.body["deleted"], 1);
    assert!(entries().is_empty());
    // Every caller's listing still answers.
    for caller in CALLERS {
        assert_eq!(
            get(&app, "/dictation/history", caller).await.status,
            StatusCode::OK
        );
    }
}
