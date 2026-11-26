//! The dictation history (DD-96) on a daemon that holds no user action key, as `biorouter
//! serve` starts one (SD-7), under SD-20:
//!
//! * the person in the served page (the served document's cookie and the page script's
//!   Fetch Metadata) lists private entries, changes an entry's tier, raises the switches
//!   and deletes, and the streams that person starts are kept;
//! * a caller holding only the secret, one that states a private provider, and the served
//!   cookie alone without the page's Fetch Metadata get public entries only; their tier
//!   changes and raises are refused with MSG-HISTORY-HOST-TIER and
//!   MSG-HISTORY-HOST-SETTINGS before any entry is read, their streams are not kept, and
//!   lowering a switch and deleting a public entry are admitted.
//!
//! ⚠ Its own binary on purpose: the user action digest and the served operator are
//! process globals, and nothing here installs a digest, which is how `serve` starts its
//! daemon.

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::Router;
use biorouter::dictation::history::{self as history, EntryTier, HistoryEntry, Surface};
use biorouter::privacy::ProviderTier;
use biorouter_server::auth::{user_action_proof, UserActionProof};
use biorouter_server::routes::{dictation, dictation_history};
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use tower::ServiceExt;

const SECRET: &str = "dictation-history-keyless-secret";
/// The browser token `biorouter serve` would have minted for this launch.
const BROWSER_TOKEN: &str = "7a1c3e5f9b2d4f6a8c0e2a4c6e8a0c2e";
const WORKER_MARKER_ENV: &str = "BIOROUTER_HISTORY_KEYLESS_SYNTH_WORKER";
const SPOKEN: &str = "Stanfordine";

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/whisper.cpp/source")
        .join(relative)
}

/// The worker is this test binary, which answers every decode with [`SPOKEN`].
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
            ToWorker::Transcribe(request) => send(
                &mut output,
                FromWorker::Result(TranscribeResult {
                    id: request.id,
                    text: SPOKEN.into(),
                    segments: vec![SegmentText {
                        t0_ms: 0,
                        t1_ms: 800,
                        text: SPOKEN.into(),
                    }],
                    prompt_terms_used: 0,
                    timings: Timings::default(),
                    language: None,
                }),
            ),
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

fn engine_runs() -> bool {
    biorouter::dictation::service::spawn_refusal().is_none()
}

/// A `serve` daemon: no user action key, and its interface served behind a browser token
/// whose operator runs a public provider.
async fn daemon() -> (Arc<AppState>, Router) {
    assert_eq!(
        user_action_proof(&HeaderMap::new()),
        UserActionProof::NoKeyInstalled,
        "something in this binary installed a user action digest"
    );
    biorouter_server::launch::record_launch_state(false);
    biorouter_server::auth::install_served_operator(
        BROWSER_TOKEN.to_string(),
        ProviderTier::Public,
    );
    let _ = std::fs::remove_dir_all(history::history_dir());
    let state = AppState::new().await.expect("app state");
    let app = Router::new()
        .merge(dictation::routes(Arc::clone(&state)))
        .merge(dictation_history::routes(Arc::clone(&state)))
        .layer(axum::middleware::from_fn_with_state(
            SECRET.to_string(),
            biorouter_server::auth::check_token,
        ));
    (state, app)
}

#[derive(Clone, Copy, Debug)]
enum Caller {
    SecretOnly,
    /// `userActionHeaders()` on a `serve` page of a private host (SD-12): a capability.
    StatedPrivate,
    /// The served document's cookie without the page script's Fetch Metadata.
    ServedCookieOnly,
    /// The person in the served page (SD-20).
    ServedPerson,
}

const NOT_THE_PERSON: [Caller; 3] = [
    Caller::SecretOnly,
    Caller::StatedPrivate,
    Caller::ServedCookieOnly,
];

struct Answer {
    status: StatusCode,
    raw: axum::body::Bytes,
    body: Value,
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    caller: Caller,
    body: Option<Value>,
) -> Answer {
    let cookie = format!("biorouter_session={BROWSER_TOKEN}");
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Secret-Key", SECRET)
        .header(header::CONTENT_TYPE, "application/json");
    builder = match caller {
        Caller::SecretOnly => builder,
        Caller::StatedPrivate => builder.header("X-Caller-Provider", "versa_azure"),
        Caller::ServedCookieOnly => builder.header("cookie", cookie),
        Caller::ServedPerson => builder
            .header("cookie", cookie)
            .header("sec-fetch-site", "same-origin")
            .header("sec-fetch-mode", "cors")
            .header("sec-fetch-dest", "empty"),
    };
    let body = body.map_or_else(Body::empty, |b| Body::from(b.to_string()));
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

async fn get(app: &Router, uri: &str, caller: Caller) -> Answer {
    send(app, "GET", uri, caller, None).await
}

async fn post(app: &Router, uri: &str, caller: Caller, body: Value) -> Answer {
    send(app, "POST", uri, caller, Some(body)).await
}

fn add(text: &str, tier: EntryTier) -> String {
    let entry = HistoryEntry {
        id: history::store::new_entry_id(),
        created_at: chrono::Utc::now(),
        text: text.to_string(),
        tier,
        institutions: None,
        surface: Surface::Clipboard,
        duration_ms: 1_000,
        model: "for-tests".to_string(),
        private_terms: false,
        session_id: None,
        session_incarnation: None,
        truncated: None,
    };
    let id = entry.id.clone();
    history::open_history().append(entry).expect("appended");
    id
}

fn texts(listing: &Value) -> Vec<String> {
    listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["text"].as_str().unwrap().to_string())
        .collect()
}

fn tier_of(id: &str) -> Option<EntryTier> {
    history::open_history()
        .get(id)
        .unwrap()
        .map(|entry| entry.tier)
}

/// SD-20: the person in the served page has the history as the desktop's person does.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_served_person_lists_and_manages_private_history() {
    let (_state, app) = daemon().await;
    let public = add("a public dictation", EntryTier::Public);
    let private = add("a private dictation", EntryTier::Private);

    let listing = get(&app, "/dictation/history", Caller::ServedPerson).await;
    assert_eq!(listing.status, StatusCode::OK);
    assert_eq!(
        texts(&listing.body),
        ["a private dictation", "a public dictation"]
    );
    assert_eq!(listing.body["total"], 2);
    let settings = get(&app, "/dictation/history/settings", Caller::ServedPerson).await;
    for control in ["list_private", "write", "tier_change", "raise_settings"] {
        assert_eq!(
            settings.body["authority"][control]["allowed"], true,
            "{control}: {}",
            settings.body
        );
    }

    let changed = post(
        &app,
        &format!("/dictation/history/{private}/tier"),
        Caller::ServedPerson,
        json!({"tier": "public"}),
    )
    .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    assert_eq!(tier_of(&private), Some(EntryTier::Public));
    let changed = post(
        &app,
        &format!("/dictation/history/{private}/tier"),
        Caller::ServedPerson,
        json!({"tier": "private"}),
    )
    .await;
    assert_eq!(changed.status, StatusCode::OK);
    assert_eq!(tier_of(&private), Some(EntryTier::Private));

    for (body, keep, public_by_default) in [
        (json!({"keep_history": false}), false, false),
        (json!({"keep_history": true}), true, false),
        (json!({"public_by_default": true}), true, true),
        (json!({"public_by_default": false}), true, false),
    ] {
        let answer = post(
            &app,
            "/dictation/history/settings",
            Caller::ServedPerson,
            body,
        )
        .await;
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
        assert_eq!(answer.body["keep_history"], keep);
        assert_eq!(answer.body["public_by_default"], public_by_default);
    }

    let deleted = send(
        &app,
        "DELETE",
        &format!("/dictation/history/{private}"),
        Caller::ServedPerson,
        None,
    )
    .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert_eq!(tier_of(&private), None);
    assert_eq!(tier_of(&public), Some(EntryTier::Public));
}

/// Anyone else holding the secret, on the same daemon: public entries only, the host
/// sentences for a tier change and a raise, decided before the id is read, and lowering
/// and deleting what it can list admitted.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_caller_that_is_not_the_person_gets_public_entries_and_the_host_sentences() {
    let (_state, app) = daemon().await;
    let missing = history::store::new_entry_id();
    for caller in NOT_THE_PERSON {
        let _ = std::fs::remove_dir_all(history::history_dir());
        let public = add("a public dictation", EntryTier::Public);
        let private = add("a private dictation", EntryTier::Private);

        let listing = get(&app, "/dictation/history", caller).await;
        assert_eq!(listing.status, StatusCode::OK, "{caller:?}");
        assert_eq!(texts(&listing.body), ["a public dictation"], "{caller:?}");
        assert_eq!(listing.body["total"], 1, "{caller:?}");

        let settings = get(&app, "/dictation/history/settings", caller).await;
        let authority = &settings.body["authority"];
        assert_eq!(authority["list_private"]["allowed"], false, "{caller:?}");
        assert_eq!(authority["write"]["allowed"], true, "{caller:?}");
        assert_eq!(
            authority["tier_change"]["reason"],
            dictation_history::msg_history_host_tier(EntryTier::Public),
            "{caller:?}"
        );
        assert_eq!(
            authority["raise_settings"]["reason"],
            dictation_history::msg_history_host_settings("<option>"),
            "{caller:?}"
        );

        // A tier change: the host sentence with its verb, the same for every id.
        let mut answers = Vec::new();
        for id in [&public, &private, &missing] {
            for (tier, verb) in [("private", "privatize"), ("public", "publicize")] {
                let answer = post(
                    &app,
                    &format!("/dictation/history/{id}/tier"),
                    caller,
                    json!({"tier": tier}),
                )
                .await;
                assert_eq!(answer.status, StatusCode::FORBIDDEN, "{caller:?}");
                assert_eq!(answer.body["code"], "host_tier", "{caller:?}");
                assert_eq!(
                    answer.body["message"],
                    dictation_history::MSG_HISTORY_HOST_TIER.replace("<verb>", verb),
                    "{caller:?}"
                );
                answers.push((tier, answer.raw));
            }
        }
        for (tier, raw) in &answers {
            let first = answers.iter().find(|(t, _)| t == tier).unwrap();
            assert_eq!(
                raw, &first.1,
                "{caller:?}: a tier refusal depends on the id"
            );
        }
        assert_eq!(tier_of(&public), Some(EntryTier::Public));
        assert_eq!(tier_of(&private), Some(EntryTier::Private));

        // Lowering is admitted; raising is the served person's.
        let lowered = post(
            &app,
            "/dictation/history/settings",
            caller,
            json!({"keep_history": false, "public_by_default": false}),
        )
        .await;
        assert_eq!(
            lowered.status,
            StatusCode::OK,
            "{caller:?}: {}",
            lowered.body
        );
        assert_eq!(lowered.body["keep_history"], false);
        for (body, option) in [
            (json!({"keep_history": true}), "--keep on"),
            (json!({"public_by_default": true}), "--public-by-default on"),
        ] {
            let raised = post(&app, "/dictation/history/settings", caller, body).await;
            assert_eq!(raised.status, StatusCode::FORBIDDEN, "{caller:?}");
            assert_eq!(raised.body["code"], "host_settings");
            assert_eq!(
                raised.body["message"],
                dictation_history::msg_history_host_settings(option)
            );
        }
        let still = get(&app, "/dictation/history/settings", caller).await;
        assert_eq!(still.body["keep_history"], false, "{caller:?}");
        assert_eq!(still.body["public_by_default"], false, "{caller:?}");

        // Deleting: a public entry goes; a private one answers as a missing one.
        let hidden = send(
            &app,
            "DELETE",
            &format!("/dictation/history/{private}"),
            caller,
            None,
        )
        .await;
        let absent = send(
            &app,
            "DELETE",
            &format!("/dictation/history/{missing}"),
            caller,
            None,
        )
        .await;
        assert_eq!(hidden.status, StatusCode::NOT_FOUND, "{caller:?}");
        assert_eq!(hidden.raw, absent.raw, "{caller:?}");
        assert_eq!(tier_of(&private), Some(EntryTier::Private));
        let bulk = post(
            &app,
            "/dictation/history/delete",
            caller,
            json!({"ids": [private]}),
        )
        .await;
        assert_eq!(bulk.body["deleted"], 0, "{caller:?}");
        let cleared = post(
            &app,
            "/dictation/history/clear",
            caller,
            json!({"through": chrono::Utc::now() + chrono::Duration::seconds(1)}),
        )
        .await;
        assert_eq!(cleared.status, StatusCode::OK, "{caller:?}");
        assert_eq!(cleared.body["deleted"], 1, "{caller:?}: the public entry");
        assert_eq!(tier_of(&public), None, "{caller:?}");
        assert_eq!(tier_of(&private), Some(EntryTier::Private), "{caller:?}");
    }
}

fn hum(seconds: f64) -> Vec<u8> {
    let samples = (seconds * 16_000.0) as usize;
    (0..samples)
        .flat_map(|i| (((i % 16) as i16) - 8).to_le_bytes())
        .collect()
}

async fn dictate(app: &Router, caller: Caller) {
    let started = Instant::now();
    let created = post(
        app,
        "/dictation/streams",
        caller,
        json!({"sample_rate": 16000, "format": "s16le", "target_kind": "external"}),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let id = created.body["stream_id"].as_str().unwrap().to_string();
    let mut answer = Answer {
        status: StatusCode::ACCEPTED,
        raw: Default::default(),
        body: Value::Null,
    };
    let finish = Request::builder()
        .method("POST")
        .uri(format!("/dictation/streams/{id}/finish?seq=0"))
        .header("X-Secret-Key", SECRET)
        .body(Body::from(hum(1.0)))
        .unwrap();
    let response = app.clone().oneshot(finish).await.unwrap();
    answer.status = response.status();
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
    assert_eq!(answer.status, StatusCode::OK);
    send(
        app,
        "DELETE",
        &format!("/dictation/streams/{id}"),
        Caller::SecretOnly,
        None,
    )
    .await;
}

fn held() -> usize {
    history::open_history()
        .search(None, history::TimeWindow::all(), None, 10_000, &|_| true)
        .unwrap()
        .entries
        .len()
}

/// SD-20 and 6.1 of the history design: a `serve` daemon keeps the streams the served
/// person starts and no other.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn only_the_served_persons_streams_are_kept() {
    if !engine_runs() {
        return;
    }
    let (_state, app) = daemon().await;
    biorouter::dictation::service::service()
        .prepare(None)
        .await
        .expect("the synthetic worker loads");
    for caller in NOT_THE_PERSON {
        dictate(&app, caller).await;
    }
    dictate(&app, Caller::ServedPerson).await;
    let started = Instant::now();
    while held() == 0 {
        assert!(started.elapsed() < Duration::from_secs(30), "nothing kept");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(held(), 1, "only the served person's stream is kept");
    let entry = history::open_history()
        .search(None, history::TimeWindow::all(), None, 1, &|_| true)
        .unwrap()
        .entries
        .remove(0);
    assert_eq!(entry.text, SPOKEN);
    assert_eq!(entry.tier, EntryTier::Private);
}
