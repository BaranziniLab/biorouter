//! Tests for `routes::decisions`. Nothing here reaches a model: every route that
//! asks one is driven over a fake decision source, and every route driven
//! through the served tree (`routes::configure`, which asks the process's real
//! service) is one that answers from the log, the settings or a refusal.

use super::*;
use crate::routes::session::diverge_tests::{install_test_user_action_key, TEST_USER_ACTION_KEY};
use crate::routes::session_reach::{CALLER_PROVIDER_HEADER, SESSION_OUT_OF_REACH};
use axum::body::{to_bytes, Body};
use axum::http::Request;
use biorouter::decision::log::SqliteDecisionSink;
use biorouter::decision::model::{
    DecisionCaps, DecisionModel, DecisionModelEntry, DecisionModelSource,
};
use biorouter::decision::types::{ModelVerdicts, QuestionVerdict};
use biorouter::decision::{
    Answer, DecisionKind, DecisionLogEntry, DecisionOutcome, DecisionResult, DecisionSink,
    DecisionUsage, DecisionUseGate, ProbabilitySource, Question,
};
use biorouter::model::ModelConfig;
use biorouter::privacy::affiliation::ModelAffiliation;
use biorouter::session::{SessionManager, SessionType};
use serde_json::json;
use serial_test::serial;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use tower::ServiceExt;

const PROOF: (&str, &str) = ("X-User-Action", TEST_USER_ACTION_KEY);
const PRIVATE_CAPABILITY: (&str, &str) = (CALLER_PROVIDER_HEADER, "versa_azure");

// ─── Pure corners ───

#[test]
fn every_decision_error_is_answered_with_one_status() {
    let cases = [
        (DecisionError::Invalid("x".into()), StatusCode::BAD_REQUEST),
        (DecisionError::Refused("x".into()), StatusCode::FORBIDDEN),
        (
            DecisionError::TooLarge {
                chunk: "c".into(),
                tokens: 9,
                limit: 1,
                model: "m".into(),
            },
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
        (DecisionError::NoModel("x".into()), StatusCode::NOT_FOUND),
        (
            DecisionError::Unsupported("x".into()),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            DecisionError::Timeout {
                model: "m".into(),
                seconds: 1,
            },
            StatusCode::GATEWAY_TIMEOUT,
        ),
        (
            DecisionError::Backend {
                model: "m".into(),
                status: Some(500),
                error_type: None,
                message: "x".into(),
            },
            StatusCode::BAD_GATEWAY,
        ),
        (DecisionError::malformed("m", "x"), StatusCode::BAD_GATEWAY),
    ];
    for (error, status) in cases {
        assert_eq!(decision_error_status(&error), status, "{error:?}");
    }
}

#[test]
fn a_settings_write_needs_the_proof_only_where_one_can_be_given() {
    assert!(settings_write_refusal(UserActionProof::Proven, true).is_none());
    assert!(settings_write_refusal(UserActionProof::Proven, false).is_none());
    let refused = settings_write_refusal(UserActionProof::Unproven, false)
        .expect("a keyed daemon refuses a write without the proof");
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let fault = settings_write_refusal(UserActionProof::NoKeyInstalled, true)
        .expect("a desktop daemon that lost its key keeps the refusal");
    assert_eq!(fault.status(), StatusCode::FORBIDDEN);
    assert!(
        settings_write_refusal(UserActionProof::NoKeyInstalled, false).is_none(),
        "a daemon that can never check a proof (`biorouter serve`) relies on the reach gate"
    );
}

#[test]
fn a_filter_outside_its_range_is_refused_with_a_sentence() {
    let query = |f: fn(&mut DecisionQuery)| {
        let mut query = DecisionQuery::default();
        f(&mut query);
        validate_query(&query)
    };
    assert!(query(|_| {}).is_ok());
    assert!(query(|q| q.limit = Some(MAX_PAGE)).is_ok());
    assert!(query(|q| {
        q.min_p = Some(0.2);
        q.max_p = Some(0.8);
    })
    .is_ok());
    for refused in [
        query(|q| q.limit = Some(0)),
        query(|q| q.limit = Some(MAX_PAGE + 1)),
        query(|q| {
            q.min_p = Some(0.8);
            q.max_p = Some(0.2);
        }),
    ] {
        let sentence = refused.expect_err("refused");
        assert!(sentence.ends_with('.'), "a sentence: {sentence}");
    }
}

// ─── A fake decision source: no network, ever ───

struct FakeModel {
    entry: DecisionModelEntry,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl DecisionModel for FakeModel {
    fn model_ref(&self) -> DecisionModelRef {
        self.entry.model_ref()
    }
    fn display_name(&self) -> String {
        self.entry.display_name.clone()
    }
    fn tier(&self) -> ProviderTier {
        self.entry.tier
    }
    fn affiliation(&self) -> Option<ModelAffiliation> {
        self.entry.affiliation
    }
    fn caps(&self) -> DecisionCaps {
        DecisionCaps {
            kinds: self.entry.kinds.clone(),
            max_questions: 64,
            context_tokens: self.entry.context_tokens,
            max_concurrency: 2,
            max_request_bytes: None,
            local: self.entry.local,
        }
    }
    async fn decide(&self, request: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ModelVerdicts {
            verdicts: request
                .questions
                .keys()
                .map(|name| QuestionVerdict {
                    question: name.clone(),
                    probabilities: [("yes".to_string(), 0.8), ("no".to_string(), 0.2)]
                        .into_iter()
                        .collect(),
                    source: ProbabilitySource::Native,
                    calibrated: true,
                    reported_choice: None,
                    raw: json!({ "noul": 0.8 }),
                })
                .collect(),
            usage: DecisionUsage {
                input_tokens: Some(12),
                output_tokens: Some(1),
            },
            served_model: None,
        })
    }
}

fn entry(name: &str, tier: ProviderTier, local: bool) -> DecisionModelEntry {
    DecisionModelEntry {
        provider: "test_decision".to_string(),
        model: name.to_string(),
        display_name: name.to_string(),
        context_tokens: 4_096,
        kinds: vec![DecisionKindTag::Binary],
        tier,
        affiliation: local.then_some(ModelAffiliation::Local),
        adapter: DecisionAdapter::SystemOne,
        local,
    }
}

struct FakeSource {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl DecisionModelSource for FakeSource {
    async fn list(&self) -> Vec<DecisionModelEntry> {
        vec![
            entry("cloud-checker", ProviderTier::Public, false),
            entry("local-checker", ProviderTier::Private, true),
        ]
    }
    async fn open(&self, wanted: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
        let entry = self
            .list()
            .await
            .into_iter()
            .find(|entry| entry.answers_to(wanted))
            .ok_or_else(|| DecisionError::NoModel(format!("no `{}` here", wanted.label())))?;
        Ok(Arc::new(FakeModel {
            entry,
            calls: self.calls.clone(),
        }))
    }
}

/// A Crew registry is not what these tests are about.
struct Allow;

#[async_trait]
impl DecisionUseGate for Allow {
    async fn check(&self, _: &DecisionSession, _: &dyn DecisionModel) -> DecisionResult<()> {
        Ok(())
    }
}

#[derive(Default)]
struct Recorded(Mutex<Vec<DecisionLogEntry>>);

#[async_trait]
impl DecisionSink for Recorded {
    async fn record(&self, entry: DecisionLogEntry) -> anyhow::Result<()> {
        self.0.lock().unwrap().push(entry);
        Ok(())
    }
}

struct Fake {
    service: &'static DecisionService,
    calls: Arc<AtomicUsize>,
    log: Arc<Recorded>,
}

fn fake() -> Fake {
    let calls = Arc::new(AtomicUsize::new(0));
    let log = Arc::new(Recorded::default());
    let service = DecisionService::new(Arc::new(FakeSource {
        calls: calls.clone(),
    }))
    .with_use_gate(Arc::new(Allow))
    .with_default_model(None)
    .with_sink(log.clone());
    Fake {
        // A route's state is `'static`; one small service per test.
        service: Box::leak(Box::new(service)),
        calls,
        log,
    }
}

// ─── Chats, over the binary's one session store ───

/// A chat seeded here, deleted when the test's scope ends, including on a
/// panic.
struct Seeded {
    state: Arc<AppState>,
    id: String,
}

impl Drop for Seeded {
    fn drop(&mut self) {
        let state = self.state.clone();
        let id = std::mem::take(&mut self.id);
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                if let Err(e) = state.session_manager().delete_session(&id).await {
                    eprintln!("could not clean up the seeded chat {id}: {e}");
                }
            })
        });
    }
}

async fn seed(state: &Arc<AppState>, tier: SessionClassification) -> Seeded {
    let manager = state.session_manager();
    let session = manager
        .create_session(
            PathBuf::from("/tmp/decisions_route"),
            "Decisions route (test fixture)".to_string(),
            SessionType::User,
        )
        .await
        .unwrap();
    if tier == SessionClassification::Private {
        manager
            .update(&session.id)
            .provider_name("versa_azure")
            .model_config(ModelConfig::new("gpt-4o").unwrap())
            .raise_privacy(SessionClassification::Private, "turn:versa_azure")
            .apply()
            .await
            .unwrap();
    }
    Seeded {
        state: state.clone(),
        id: session.id,
    }
}

async fn send(
    app: Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let body = match body {
        Some(json) => {
            builder = builder.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&json).unwrap())
        }
        None => Body::empty(),
    };
    let res = app.oneshot(builder.body(body).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The served tree, so a route registered on the wrong router is measured.
fn served(state: Arc<AppState>) -> Router {
    crate::routes::configure(state, "decisions-route-secret".to_string())
}

fn json_of(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or_else(|e| panic!("not JSON ({e}): {body}"))
}

fn yes_no(threshold: Option<f64>) -> Value {
    let mut question = json!({
        "instructions": "Does this note state the patient has multiple sclerosis?",
        "kind": { "binary": {} }
    });
    if let Some(p) = threshold {
        question["threshold"] = json!({ "yes": p });
    }
    question
}

/// Every route that names one chat, as `(method, uri, body)`.
fn chat_routes(id: &str) -> Vec<(&'static str, String, Option<Value>)> {
    vec![
        ("GET", format!("/decisions/models?sessionId={id}"), None),
        (
            "POST",
            "/decisions/decide".to_string(),
            Some(json!({
                "sessionId": id,
                "questions": { "has_ms": yes_no(None) },
                "context": { "text": "probe" }
            })),
        ),
        ("GET", format!("/sessions/{id}/decisions"), None),
        ("GET", format!("/sessions/{id}/decisions/summary"), None),
        (
            "GET",
            format!("/sessions/{id}/decisions/0195f4c2-0000-7000-8000-000000000000"),
            None,
        ),
        ("GET", format!("/sessions/{id}/decision-settings"), None),
        (
            "PUT",
            format!("/sessions/{id}/decision-settings"),
            Some(json!({ "monitors": [] })),
        ),
        ("GET", format!("/sessions/{id}/decision-chat"), None),
        (
            "PUT",
            format!("/sessions/{id}/decision-chat"),
            Some(json!({ "question": yes_no(None) })),
        ),
    ]
}

// ─── The reach gate ───

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn every_decision_route_refuses_a_private_chat_exactly_as_the_read_does() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let private = seed(&state, SessionClassification::Private).await;
    // Syntactically a session id, and not a row on this machine.
    let absent = "29990101_737373";
    let fake = fake();

    let mut leaks = Vec::new();
    for target in [private.id.as_str(), absent] {
        for (method, uri, body) in chat_routes(target) {
            // Both trees: the served one (its real service is never reached on a
            // refusal) and one over the fake service, whose call count proves
            // the refusal came before any model was asked.
            for app in [
                served(state.clone()),
                routes_with_service(state.clone(), fake.service),
            ] {
                let (status, got) = send(app, method, &uri, body.clone(), &[]).await;
                if status != StatusCode::FORBIDDEN || got != SESSION_OUT_OF_REACH {
                    leaks.push(format!("{method} {uri} -> {status}: {got:.160}"));
                }
            }
        }
    }
    assert!(
        leaks.is_empty(),
        "answered differently from `GET /sessions/{{id}}`:\n  {}",
        leaks.join("\n  ")
    );
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0, "no model was asked");
    assert!(
        fake.log.0.lock().unwrap().is_empty(),
        "a refused request logged nothing"
    );

    // The person at the keyboard, and a program stating a private model, both
    // still read it.
    for credential in [PROOF, PRIVATE_CAPABILITY] {
        for uri in [
            format!("/sessions/{}/decisions", private.id),
            format!("/sessions/{}/decisions/summary", private.id),
            format!("/sessions/{}/decision-settings", private.id),
            format!("/sessions/{}/decision-chat", private.id),
        ] {
            let (status, body) =
                send(served(state.clone()), "GET", &uri, None, &[credential]).await;
            assert_eq!(status, StatusCode::OK, "{credential:?} {uri}: {body}");
        }
    }
    // …and only an admitted caller learns that a chat is absent.
    let (status, _) = send(
        served(state.clone()),
        "GET",
        &format!("/sessions/{absent}/decisions"),
        None,
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(
        served(state.clone()),
        "GET",
        "/sessions/not.a.chat/decisions",
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ─── Shapes ───

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_chat_with_no_decisions_answers_in_the_documented_shapes() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let chat = seed(&state, SessionClassification::Public).await;
    let id = chat.id.as_str();

    let (status, body) = send(
        served(state.clone()),
        "GET",
        &format!("/sessions/{id}/decisions"),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let page = json_of(&body);
    assert_eq!(page["records"], json!([]), "{body}");
    assert_eq!(page["total"], 0, "{body}");
    assert!(page.get("nextCursor").is_some(), "{body}");

    let (status, body) = send(
        served(state.clone()),
        "GET",
        &format!("/sessions/{id}/decisions/summary"),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let summary = json_of(&body);
    assert_eq!(summary["total"], 0);
    assert_eq!(summary["aggregates"], json!([]));
    assert_eq!(summary["recentTurns"], json!([]));

    let (status, body) = send(
        served(state.clone()),
        "GET",
        &format!("/sessions/{id}/decision-settings"),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let settings = json_of(&body);
    assert!(settings["defaultModel"].is_null(), "{body}");
    assert_eq!(settings["monitors"], json!([]), "{body}");

    let (status, body) = send(
        served(state.clone()),
        "GET",
        &format!("/sessions/{id}/decision-chat"),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(json_of(&body), json!({ "settings": null }));

    let (status, body) = send(
        served(state.clone()),
        "GET",
        &format!("/sessions/{id}/decisions/0195f4c2-0000-7000-8000-000000000000"),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, _) = send(
        served(state.clone()),
        "GET",
        &format!("/sessions/{id}/decisions/not.an.id"),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ─── Writes ───

fn monitor(id: &str) -> Value {
    json!({
        "id": id,
        "name": "Form reached",
        "question": {
            "instructions": "Has the agent reached the consent form page?",
            "kind": { "binary": {} }
        },
        "model": null,
        "timing": "post_iteration",
        "window": { "entries": 24, "maxChars": 6000, "toolCalls": true },
        "threshold": 0.85,
        "option": null,
        "consecutive": 1,
        "rearmBelow": 0.7,
        "action": "notify",
        "onError": "notify",
        "maxEvalsPerTurn": 50,
        "enabled": true
    })
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_settings_write_without_the_proof_is_refused_on_a_keyed_daemon() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let chat = seed(&state, SessionClassification::Public).await;
    let id = chat.id.as_str();
    let before = state
        .session_manager()
        .get_session(id, false)
        .await
        .unwrap()
        .extension_data;

    for (uri, body, refusal) in [
        (
            format!("/sessions/{id}/decision-settings"),
            json!({ "monitors": [monitor("m1")] }),
            DECISION_SETTINGS_NEED_USER,
        ),
        (
            format!("/sessions/{id}/decision-chat"),
            json!({ "question": yes_no(Some(0.9)) }),
            DECISION_SETTINGS_NEED_USER,
        ),
    ] {
        // Reachable (a public chat) and still refused: the proof is a second
        // requirement, not the reach gate again.
        let (status, got) = send(served(state.clone()), "PUT", &uri, Some(body.clone()), &[]).await;
        assert_eq!((status, got.as_str()), (StatusCode::FORBIDDEN, refusal));
        // A stated private capability is reach, not a person.
        let (status, got) = send(
            served(state.clone()),
            "PUT",
            &uri,
            Some(body),
            &[PRIVATE_CAPABILITY],
        )
        .await;
        assert_eq!((status, got.as_str()), (StatusCode::FORBIDDEN, refusal));
    }
    let after = state
        .session_manager()
        .get_session(id, false)
        .await
        .unwrap()
        .extension_data;
    assert_eq!(
        serde_json::to_value(&after).unwrap(),
        serde_json::to_value(&before).unwrap(),
        "a refused write changed the chat's state"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_person_at_the_keyboard_stores_settings_that_read_back() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let chat = seed(&state, SessionClassification::Public).await;
    let id = chat.id.as_str();

    let settings = json!({
        "defaultModel": { "provider": "ollama_decision", "model": "tev1:0.8b" },
        "monitors": [monitor("m1")]
    });
    let (status, body) = send(
        served(state.clone()),
        "PUT",
        &format!("/sessions/{id}/decision-settings"),
        Some(settings),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let stored = json_of(&body);
    assert_eq!(stored["monitors"][0]["id"], "m1", "{body}");
    let (status, body) = send(
        served(state.clone()),
        "GET",
        &format!("/sessions/{id}/decision-settings"),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_of(&body), stored, "what was stored reads back");

    let (status, body) = send(
        served(state.clone()),
        "PUT",
        &format!("/sessions/{id}/decision-chat"),
        Some(json!({ "question": yes_no(Some(0.9)) })),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let stored = json_of(&body);
    assert_eq!(
        stored["settings"]["question"]["threshold"],
        json!({ "yes": 0.9 }),
        "{body}"
    );
    let (status, body) = send(
        served(state.clone()),
        "GET",
        &format!("/sessions/{id}/decision-chat"),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_of(&body), stored, "what was stored reads back");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn unreadable_or_invalid_writes_and_filters_are_400_with_the_sentence() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let chat = seed(&state, SessionClassification::Public).await;
    let id = chat.id.as_str();

    let put = |uri: String, body: Body| {
        let state = state.clone();
        async move {
            let res = served(state)
                .oneshot(
                    Request::builder()
                        .method("PUT")
                        .uri(uri)
                        .header("content-type", "application/json")
                        .header(PROOF.0, PROOF.1)
                        .body(body)
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = res.status();
            let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
            (status, String::from_utf8_lossy(&bytes).into_owned())
        }
    };

    let (status, body) = put(
        format!("/sessions/{id}/decision-settings"),
        Body::from("{not json"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.starts_with("The decision settings could not be read"),
        "{body}"
    );

    // Nine monitors, one over the limit: the settings' own validation answers.
    let nine: Vec<Value> = (0..9).map(|i| monitor(&format!("m{i}"))).collect();
    let (status, body) = put(
        format!("/sessions/{id}/decision-settings"),
        Body::from(serde_json::to_vec(&json!({ "monitors": nine })).unwrap()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(!body.is_empty(), "the refusal is a sentence");

    let (status, body) = put(
        format!("/sessions/{id}/decision-chat"),
        Body::from(
            serde_json::to_vec(&json!({
                "question": { "instructions": "   ", "kind": { "binary": {} } }
            }))
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("instructions"), "{body}");

    for filter in [
        "limit=0",
        "limit=201",
        "minP=1.5",
        "minP=0.8&maxP=0.2",
        "maxP=-0.1",
        "before=not-a-cursor",
        "origin=bogus",
        "errorsOnly=perhaps",
    ] {
        let (status, body) = send(
            served(state.clone()),
            "GET",
            &format!("/sessions/{id}/decisions?{filter}"),
            None,
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{filter}: {body}");
        assert!(!body.is_empty(), "{filter}: the refusal is a sentence");
    }
}

// ─── The log, aggregated and paged ───

fn decision(p_yes: f64, threshold: f64) -> Decision {
    Decision {
        question: "form".to_string(),
        answer: Answer::Binary {
            value: p_yes >= 0.5,
        },
        probabilities: [("yes".to_string(), p_yes), ("no".to_string(), 1.0 - p_yes)]
            .into_iter()
            .collect(),
        confidence: (2.0 * p_yes - 1.0).abs(),
        passed: Some(p_yes >= threshold),
        pass_probability: Some(p_yes),
        source: ProbabilitySource::Native,
        calibrated: true,
        model: DecisionModelRef::new("ollama_decision", "tev1:0.8b"),
        tier: ProviderTier::Private,
        // A row is stored under its request's id. Ids that sort in the order
        // they were made, as the service's uuid v7 ids do, so "newest" never
        // depends on a random id within one millisecond.
        request_id: {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            format!("req-{:012}", NEXT.fetch_add(1, Ordering::SeqCst))
        },
        latency_ms: 22,
        usage: DecisionUsage::default(),
        raw: json!({}),
    }
}

fn row(
    session_id: &str,
    origin: DecisionOrigin,
    origin_id: &str,
    turn_id: &str,
    p_yes: f64,
) -> DecisionLogEntry {
    let decision = decision(p_yes, 0.85);
    let outcome = if decision.passed == Some(true) {
        DecisionOutcome::Passed
    } else {
        DecisionOutcome::Failed
    };
    DecisionLogEntry {
        session_id: Some(session_id.to_string()),
        origin,
        origin_id: Some(origin_id.to_string()),
        origin_label: Some(format!("label {origin_id}")),
        turn_id: Some(turn_id.to_string()),
        batch_id: None,
        chunk_index: None,
        chunk_id: None,
        model: Some(decision.model.clone()),
        served_model: None,
        tier: Some(ProviderTier::Private),
        questions: [(
            "form".to_string(),
            Question {
                instructions: "Has the form been reached?".to_string(),
                kind: DecisionKind::Binary {},
                threshold: Some(biorouter::decision::Threshold::Yes(0.85)),
            },
        )]
        .into_iter()
        .collect(),
        context: DecisionContext::Text(format!("window {p_yes}")),
        context_bytes: 10,
        outcome,
        decisions: vec![decision],
        error: None,
        latency_ms: Some(22),
        usage: DecisionUsage::default(),
        privacy_enforced: true,
    }
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_summary_aggregates_per_source_and_the_list_pages_newest_first() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let chat = seed(&state, SessionClassification::Public).await;
    let id = chat.id.as_str();

    // Five monitor evaluations and two tool decisions, over two turns.
    let sink = SqliteDecisionSink {
        session_manager: Arc::new(SessionManager::instance()),
    };
    for (i, p) in [0.1, 0.2, 0.3, 0.4, 0.9].into_iter().enumerate() {
        let turn = if i < 3 { "t1" } else { "t2" };
        sink.record(row(id, DecisionOrigin::Monitor, "m1", turn, p))
            .await
            .unwrap();
    }
    for p in [0.95, 0.5] {
        sink.record(row(id, DecisionOrigin::Tool, "call-1", "t2", p))
            .await
            .unwrap();
    }

    let (status, body) = send(
        served(state.clone()),
        "GET",
        &format!("/sessions/{id}/decisions/summary"),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let summary = json_of(&body);
    assert_eq!(summary["total"], 7, "{body}");
    let aggregates = summary["aggregates"].as_array().unwrap();
    assert_eq!(aggregates.len(), 2, "one per source and criterion: {body}");
    let monitor = aggregates
        .iter()
        .find(|a| a["origin"] == "monitor")
        .expect("the monitor's aggregate");
    assert_eq!(monitor["originId"], "m1");
    assert_eq!(monitor["count"], 5);
    assert_eq!(monitor["passed"], 1);
    assert_eq!(monitor["failed"], 4);
    let tool = aggregates.iter().find(|a| a["origin"] == "tool").unwrap();
    assert_eq!(tool["count"], 2);
    assert!(
        summary["recentTurns"].as_array().unwrap().len() >= 2,
        "per turn and source: {body}"
    );

    // Pages of two, newest first, until the cursor runs out.
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..10 {
        let uri = match &cursor {
            Some(c) => format!("/sessions/{id}/decisions?limit=2&before={c}"),
            None => format!("/sessions/{id}/decisions?limit=2"),
        };
        let (status, body) = send(served(state.clone()), "GET", &uri, None, &[]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let page = json_of(&body);
        assert_eq!(page["total"], 7, "the total counts every page: {body}");
        let records = page["records"].as_array().unwrap();
        assert!(records.len() <= 2, "{body}");
        for record in records {
            assert!(
                record.get("context").is_none(),
                "a list row carries a preview, never the full context: {record}"
            );
            seen.push(record["id"].as_str().unwrap().to_string());
        }
        cursor = page["nextCursor"].as_str().map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(seen.len(), 7, "every row exactly once: {seen:?}");
    let unique: std::collections::BTreeSet<_> = seen.iter().collect();
    assert_eq!(unique.len(), 7, "no row twice: {seen:?}");

    let (status, body) = send(
        served(state.clone()),
        "GET",
        &format!("/sessions/{id}/decisions?origin=tool"),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(json_of(&body)["total"], 2, "{body}");

    // `decisionId` selects one row, as a decision card's link asks for it.
    let (status, body) = send(
        served(state.clone()),
        "GET",
        &format!("/sessions/{id}/decisions?decisionId={}", seen[3]),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let one = json_of(&body);
    assert_eq!(one["total"], 1, "{body}");
    assert_eq!(one["records"][0]["id"], seen[3].as_str(), "{body}");

    // One row in full, with the context it was sent.
    let (status, body) = send(
        served(state.clone()),
        "GET",
        &format!("/sessions/{id}/decisions/{}", seen[0]),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let detail = json_of(&body);
    assert_eq!(detail["record"]["id"], seen[0].as_str());
    assert!(detail["context"]["text"].is_string(), "{body}");

    // Another chat cannot name this chat's row.
    let other = seed(&state, SessionClassification::Public).await;
    let (status, _) = send(
        served(state.clone()),
        "GET",
        &format!("/sessions/{}/decisions/{}", other.id, seen[0]),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ─── Ad hoc decisions and the catalog, over the fake source ───

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn an_adhoc_decision_without_a_chat_says_so_and_names_a_public_model() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake();

    let (status, body) = send(
        routes_with_service(state.clone(), fake.service),
        "POST",
        "/decisions/decide",
        Some(json!({
            "questions": { "has_ms": yes_no(Some(0.75)) },
            "context": { "text": "Neuro f/u. 34F, RRMS dx 2019." }
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let answer = json_of(&body);
    assert!(answer["sessionId"].is_null());
    let decisions = answer["decisions"].as_array().unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0]["question"], "has_ms");
    assert_eq!(decisions[0]["passed"], true);
    assert_eq!(
        decisions[0]["model"]["model"], "local-checker",
        "local first"
    );
    let note = answer["note"].as_str().unwrap();
    assert!(note.contains("No chat was named"), "{note}");
    assert!(!note.contains("public model"), "{note}");

    let (status, body) = send(
        routes_with_service(state.clone(), fake.service),
        "POST",
        "/decisions/decide",
        Some(json!({
            "model": { "provider": "test_decision", "model": "cloud-checker" },
            "questions": { "has_ms": yes_no(None) },
            "context": { "text": "x" }
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let note = json_of(&body)["note"].as_str().unwrap().to_string();
    assert!(
        note.contains("public model") && note.contains("cloud-checker"),
        "{note}"
    );
    assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn an_adhoc_decision_in_a_chat_is_gated_checked_against_its_tier_and_logged() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake();
    let public = seed(&state, SessionClassification::Public).await;
    let private = seed(&state, SessionClassification::Private).await;
    let ask = |session_id: &str, model: Option<&str>| {
        let mut body = json!({
            "sessionId": session_id,
            "questions": { "has_ms": yes_no(Some(0.75)) },
            "context": { "text": "Neuro f/u." }
        });
        if let Some(model) = model {
            body["model"] = json!({ "provider": "test_decision", "model": model });
        }
        body
    };

    // A public chat: admitted on the secret alone, decided, and logged in it.
    let (status, body) = send(
        routes_with_service(state.clone(), fake.service),
        "POST",
        "/decisions/decide",
        Some(ask(&public.id, None)),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let answer = json_of(&body);
    assert_eq!(answer["sessionId"], public.id.as_str());
    assert!(answer["note"].is_null(), "a chat was named: {body}");
    {
        let log = fake.log.0.lock().unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].session_id.as_deref(), Some(public.id.as_str()));
        assert_eq!(log[0].origin, DecisionOrigin::Adhoc);
        assert_eq!(log[0].origin_label.as_deref(), Some(ADHOC_LABEL));
        assert_eq!(log[0].outcome, DecisionOutcome::Passed);
    }

    // A private chat, by the person at the keyboard, naming a public model:
    // admitted by the reach gate, refused by Gate H, and nothing sent.
    let calls_before = fake.calls.load(Ordering::SeqCst);
    let (status, body) = send(
        routes_with_service(state.clone(), fake.service),
        "POST",
        "/decisions/decide",
        Some(ask(&private.id, Some("cloud-checker"))),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_ne!(
        body, SESSION_OUT_OF_REACH,
        "this is Gate H, not the reach gate"
    );
    assert!(
        body.contains("private") && body.contains("cloud-checker"),
        "{body}"
    );
    assert_eq!(
        fake.calls.load(Ordering::SeqCst),
        calls_before,
        "a private chat's text never reached a public model"
    );

    // …and with no model named, the one it may use is chosen.
    let (status, body) = send(
        routes_with_service(state.clone(), fake.service),
        "POST",
        "/decisions/decide",
        Some(ask(&private.id, None)),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        json_of(&body)["decisions"][0]["model"]["model"],
        "local-checker"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_batch_answers_every_chunk_in_order_and_a_malformed_request_is_400() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake();
    let app = || routes_with_service(state.clone(), fake.service);

    let (status, body) = send(
        app(),
        "POST",
        "/decisions/decide",
        Some(json!({
            "questions": { "has_ms": yes_no(Some(0.9)), "edss": yes_no(Some(0.5)) },
            "chunks": [
                { "id": "note-1", "context": { "text": "a" } },
                { "id": "note-2", "context": { "text": "b" } },
                { "id": "note-3", "context": { "text": "c" } }
            ]
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let answer = json_of(&body);
    assert!(answer["decisions"].is_null());
    let chunks = answer["chunks"].as_array().unwrap();
    let ids: Vec<&str> = chunks
        .iter()
        .map(|c| c["chunkId"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["note-1", "note-2", "note-3"]);
    for chunk in chunks {
        let decisions = chunk["decisions"].as_array().unwrap();
        assert_eq!(decisions.len(), 2, "both questions, one request: {chunk}");
        assert_eq!(decisions[0]["question"], "has_ms");
        assert_eq!(decisions[0]["passed"], false, "0.8 < 0.9");
        assert_eq!(decisions[1]["passed"], true, "0.8 >= 0.5");
    }
    let batches: std::collections::BTreeSet<_> = fake
        .log
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|entry| entry.batch_id.clone())
        .collect();
    assert_eq!(batches.len(), 1, "one batch id for the batch");
    assert!(batches.iter().all(Option::is_some));

    for (case, body) in [
        (
            "both",
            json!({ "questions": { "q": yes_no(None) }, "context": { "text": "x" },
                    "chunks": [{ "id": "a", "context": { "text": "x" } }] }),
        ),
        ("neither", json!({ "questions": { "q": yes_no(None) } })),
        (
            "no questions",
            json!({ "questions": {}, "context": { "text": "x" } }),
        ),
        (
            "an unknown field",
            json!({ "questions": { "q": yes_no(None) }, "context": { "text": "x" }, "extra": 1 }),
        ),
        (
            "a malformed question",
            json!({ "questions": { "q": { "instructions": "x" } }, "context": { "text": "x" } }),
        ),
    ] {
        let calls_before = fake.calls.load(Ordering::SeqCst);
        let (status, got) = send(app(), "POST", "/decisions/decide", Some(body), &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{case}: {got}");
        assert!(!got.is_empty(), "{case}: the refusal is a sentence");
        assert_eq!(fake.calls.load(Ordering::SeqCst), calls_before, "{case}");
    }
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_catalog_names_why_a_private_chat_may_not_use_a_model() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake();
    let private = seed(&state, SessionClassification::Private).await;
    let app = || routes_with_service(state.clone(), fake.service);

    let (status, body) = send(app(), "GET", "/decisions/models", None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed = json_of(&body);
    assert!(listed["sessionId"].is_null());
    let models = listed["models"].as_array().unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0]["model"], "local-checker", "local first: {body}");
    assert_eq!(models[0]["local"], true);
    assert_eq!(models[1]["tier"], "public");
    assert!(
        models.iter().all(|m| m["unavailableReason"].is_null()),
        "no chat named, nothing to refuse: {body}"
    );

    let uri = format!("/decisions/models?sessionId={}", private.id);
    let (status, body) = send(app(), "GET", &uri, None, &[]).await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::FORBIDDEN, SESSION_OUT_OF_REACH)
    );
    let (status, body) = send(app(), "GET", &uri, None, &[PROOF]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed = json_of(&body);
    assert_eq!(listed["sessionId"], private.id.as_str());
    let models = listed["models"].as_array().unwrap();
    assert!(models[0]["unavailableReason"].is_null(), "{body}");
    let why = models[1]["unavailableReason"].as_str().unwrap();
    assert!(
        why.contains("private") && why.contains("cloud-checker"),
        "{why}"
    );
    assert_eq!(
        fake.calls.load(Ordering::SeqCst),
        0,
        "a listing asks no model"
    );
}
