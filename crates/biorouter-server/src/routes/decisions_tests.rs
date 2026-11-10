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
                usable: 1,
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
    for need_user in [
        DECISION_SETTINGS_NEED_USER,
        PHASE_GATES_NEED_USER,
        WORKFLOW_SUPERVISION_NEED_USER,
    ] {
        assert_eq!(
            proof_refusal(UserActionProof::Proven, true, need_user),
            None
        );
        assert_eq!(
            proof_refusal(UserActionProof::Proven, false, need_user),
            None
        );
        assert_eq!(
            proof_refusal(UserActionProof::Unproven, false, need_user),
            Some(need_user),
            "a keyed daemon refuses a write without the proof, in the write's own words"
        );
        assert_eq!(
            proof_refusal(UserActionProof::NoKeyInstalled, true, need_user),
            Some(DECISION_SETTINGS_KEY_MISSING),
            "a desktop daemon that lost its key keeps the refusal"
        );
        assert_eq!(
            proof_refusal(UserActionProof::NoKeyInstalled, false, need_user),
            None,
            "a daemon that can never check a proof (`biorouter serve`) relies on the reach gate"
        );
    }
}

/// `/agent/start`'s door: the proof rule without the subagent arm, in the
/// words for a workflow that brings monitors or gates. This binary holds a
/// user-action key, so a request without the proof is refused and one with it
/// is not.
#[tokio::test]
#[serial]
async fn a_workflow_start_that_writes_supervision_needs_the_proof() {
    install_test_user_action_key();
    let refused = decision_write_refusal(&HeaderMap::new())
        .expect("an unproven start is refused on a keyed daemon");
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let body = to_bytes(refused.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        String::from_utf8_lossy(&body),
        WORKFLOW_SUPERVISION_NEED_USER
    );
    assert_eq!(
        decision_write_refusal_sentence(&HeaderMap::new()),
        Some(WORKFLOW_SUPERVISION_NEED_USER)
    );
    let mut proven = HeaderMap::new();
    proven.insert(PROOF.0, PROOF.1.parse().unwrap());
    assert!(decision_write_refusal(&proven).is_none());
    assert_eq!(decision_write_refusal_sentence(&proven), None);
}

#[test]
fn a_phase_id_is_a_slug_and_the_refusal_says_how_long() {
    for id in [
        "research",
        "verify-api",
        "a",
        "phase-2",
        &"x".repeat(MAX_PHASE_ID_CHARS),
    ] {
        assert!(valid_phase_id(id), "{id}");
    }
    for id in [
        "",
        "Research",
        "verify_api",
        "phase 2",
        "../x",
        "é",
        &"x".repeat(MAX_PHASE_ID_CHARS + 1),
    ] {
        assert!(!valid_phase_id(id), "{id:?}");
    }
    assert!(
        NOT_A_PHASE_ID.contains(&format!("1 to {MAX_PHASE_ID_CHARS} ")),
        "{NOT_A_PHASE_ID}"
    );
}

#[test]
fn every_gate_write_refusal_is_answered_with_one_status() {
    for (error, status) in [
        (GateWriteError::NoPlan, StatusCode::NOT_FOUND),
        (GateWriteError::NoPhase("x".into()), StatusCode::NOT_FOUND),
        (GateWriteError::Closed("x".into()), StatusCode::CONFLICT),
        (GateWriteError::NoGate("x".into()), StatusCode::CONFLICT),
        (GateWriteError::Unreadable("x".into()), StatusCode::CONFLICT),
        (GateWriteError::Invalid("x".into()), StatusCode::BAD_REQUEST),
    ] {
        assert_eq!(gate_error_status(&error), status, "{error:?}");
    }
}

#[tokio::test]
async fn a_keyless_write_to_a_subagents_chat_is_refused_with_the_subagent_sentence() {
    for session_type in [
        SessionType::User,
        SessionType::Scheduled,
        SessionType::Hidden,
        SessionType::Terminal,
    ] {
        for proof in [
            UserActionProof::Proven,
            UserActionProof::Unproven,
            UserActionProof::NoKeyInstalled,
        ] {
            assert!(
                subagent_write_refusal(proof, session_type).is_none(),
                "{session_type:?} is not a subagent's chat ({proof:?})"
            );
        }
    }
    // On a keyed daemon the proof admits a subagent's chat (an unproven write
    // was refused one step earlier, by `settings_write_refusal`).
    assert!(subagent_write_refusal(UserActionProof::Proven, SessionType::SubAgent).is_none());
    assert!(subagent_write_refusal(UserActionProof::Unproven, SessionType::SubAgent).is_none());
    let refused = subagent_write_refusal(UserActionProof::NoKeyInstalled, SessionType::SubAgent)
        .expect("a keyless daemon refuses a subagent's chat");
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let body = to_bytes(refused.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        String::from_utf8_lossy(&body),
        crate::routes::agent::SUBAGENT_CONTROL_NO_KEY,
        "one sentence for a subagent's tab, whichever control refuses it"
    );
}

#[test]
fn an_adhoc_note_says_a_public_chats_log_now_holds_the_text() {
    let chat = |tier, enforced| DecisionSession::new("s1", tier, enforced);
    let note = decide_note(Some(&chat(SessionClassification::Public, true)), &[])
        .expect("a public chat with privacy tiers on is told");
    assert_eq!(note, PUBLIC_CHAT_LOG_NOTE);
    assert!(
        decide_note(Some(&chat(SessionClassification::Private, true)), &[]).is_none(),
        "a private chat's log stays at private reach"
    );
    assert!(
        decide_note(Some(&chat(SessionClassification::Public, false)), &[]).is_none(),
        "with privacy tiers off no tier separates one chat's log from another's"
    );
    let note = decide_note(None, &[]).unwrap();
    assert!(
        !note.contains(PUBLIC_CHAT_LOG_NOTE),
        "no chat, no log: {note}"
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
        load_refusal: None,
    }
}

/// What Ollama says of a copy it will not load, as a listing carries it.
const STALE_COPY: &str = "'stale-checker' was downloaded in a format that is no longer \
     supported. To re-download, run 'ollama pull stale-checker'.";

/// The shipped default decision model as the fake lists it: local, private,
/// yes/no questions only.
fn tev1_entry() -> DecisionModelEntry {
    DecisionModelEntry {
        provider: "ollama_decision".to_string(),
        ..entry("tev1:0.8b", ProviderTier::Private, true)
    }
}

/// A local model the server will not load.
fn stale_entry() -> DecisionModelEntry {
    DecisionModelEntry {
        load_refusal: Some(STALE_COPY.to_string()),
        ..entry("stale-checker", ProviderTier::Private, true)
    }
}

struct FakeSource {
    calls: Arc<AtomicUsize>,
    /// Listed after the two every fake has.
    extra: Vec<DecisionModelEntry>,
}

#[async_trait]
impl DecisionModelSource for FakeSource {
    async fn list(&self) -> Vec<DecisionModelEntry> {
        let mut listed = vec![
            entry("cloud-checker", ProviderTier::Public, false),
            entry("local-checker", ProviderTier::Private, true),
        ];
        listed.extend(self.extra.iter().cloned());
        listed
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
    fake_listing(Vec::new())
}

/// [`fake`], also listing `ollama_decision/tev1:0.8b` (yes/no only) and a
/// local model that will not load ([`STALE_COPY`]).
fn fake_full() -> Fake {
    fake_listing(vec![tev1_entry(), stale_entry()])
}

fn fake_listing(extra: Vec<DecisionModelEntry>) -> Fake {
    fake_gated(extra, Arc::new(Allow))
}

fn fake_gated(extra: Vec<DecisionModelEntry>, gate: Arc<dyn DecisionUseGate>) -> Fake {
    let calls = Arc::new(AtomicUsize::new(0));
    let log = Arc::new(Recorded::default());
    let service = DecisionService::new(Arc::new(FakeSource {
        calls: calls.clone(),
        extra,
    }))
    .with_use_gate(gate)
    .with_default_model(None)
    .with_sink(log.clone());
    Fake {
        // A route's state is `'static`; one small service per test.
        service: Box::leak(Box::new(service)),
        calls,
        log,
    }
}

impl Fake {
    /// This module's routes over the fake service.
    fn app(&self, state: &Arc<AppState>) -> Router {
        app(state, self.service)
    }
}

/// An installer that can never install: the routes that do not install are
/// not about it, and no test may reach the configured Ollama.
static NO_INSTALLER: std::sync::LazyLock<DecisionInstaller> = std::sync::LazyLock::new(|| {
    DecisionInstaller::new(|| Err("no Ollama in this test".to_string()))
});

/// This module's routes over `service`, with an installer that installs
/// nothing.
fn app(state: &Arc<AppState>, service: &'static DecisionService) -> Router {
    routes_with_service(state.clone(), service, &NO_INSTALLER)
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
        (
            "PUT",
            format!("/sessions/{id}/phase-gates/research"),
            Some(json!({ "questions": { "done": "Is the research done?" } })),
        ),
        (
            "POST",
            format!("/sessions/{id}/phase-gates/research/decision"),
            Some(json!({ "decision": "pass" })),
        ),
        (
            "DELETE",
            format!("/sessions/{id}/phase-gates/research"),
            None,
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
            for app in [served(state.clone()), fake.app(&state)] {
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
    // Over the fake source: storing an enabled monitor asks whether it can
    // run, and CI has no Ollama.
    let fake = fake_full();
    let chat = seed(&state, SessionClassification::Public).await;
    let id = chat.id.as_str();

    let settings = json!({
        "defaultModel": { "provider": "ollama_decision", "model": "tev1:0.8b" },
        "monitors": [monitor("m1")]
    });
    let (status, body) = send(
        fake.app(&state),
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
        fake.app(&state),
        "GET",
        &format!("/sessions/{id}/decision-settings"),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_of(&body), stored, "what was stored reads back");

    let (status, body) = send(
        fake.app(&state),
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
        fake.app(&state),
        "GET",
        &format!("/sessions/{id}/decision-chat"),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_of(&body), stored, "what was stored reads back");
}

// The subagent rule refuses only a keyless daemon's write: on a keyed one the
// proof is what admits a subagent's chat, as it is for `/agent/stop`.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_proven_write_reaches_a_subagents_chat() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake();
    let session = state
        .session_manager()
        .create_session(
            PathBuf::from("/tmp/decisions_route"),
            "Decisions route, a subagent's chat (test fixture)".to_string(),
            SessionType::SubAgent,
        )
        .await
        .unwrap();
    let chat = Seeded {
        state: state.clone(),
        id: session.id,
    };
    let id = chat.id.as_str();

    let (status, body) = send(
        fake.app(&state),
        "PUT",
        &format!("/sessions/{id}/decision-settings"),
        Some(json!({ "monitors": [monitor("m1")] })),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(json_of(&body)["monitors"][0]["id"], "m1", "{body}");
    let (status, body) = send(
        fake.app(&state),
        "PUT",
        &format!("/sessions/{id}/decision-chat"),
        Some(json!({ "question": yes_no(Some(0.9)) })),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Unproven, it is the proof's refusal, not the subagent sentence.
    let (status, body) = send(
        fake.app(&state),
        "PUT",
        &format!("/sessions/{id}/decision-settings"),
        Some(json!({ "monitors": [monitor("m2")] })),
        &[],
    )
    .await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::FORBIDDEN, DECISION_SETTINGS_NEED_USER)
    );
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

/// A PUT of raw bytes, with the proof, through the served tree.
async fn put_raw(state: &Arc<AppState>, uri: &str, body: &'static str) -> (StatusCode, String) {
    let res = served(state.clone())
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(uri)
                .header("content-type", "application/json")
                .header(PROOF.0, PROOF.1)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// A misspelled key used to be dropped and its field stored at the default,
/// and the defaults (`notify` on a firing, `notify` on an error) let the turn
/// carry on: the opposite of `"on_error": "pause"`. Now the write is refused
/// with a sentence naming the key, where it is and how it is spelled, and
/// nothing is stored.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_misspelled_settings_key_is_refused_by_name_and_nothing_is_stored() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake();
    let chat = seed(&state, SessionClassification::Public).await;
    let id = chat.id.as_str();
    let settings_uri = format!("/sessions/{id}/decision-settings");
    let chat_uri = format!("/sessions/{id}/decision-chat");

    let mut fail_closed = monitor("m1");
    fail_closed["action"] = json!("stop");
    fail_closed["onError"] = json!("pause");
    let valid = json!({ "defaultModel": null, "monitors": [fail_closed] });
    let (status, body) = send(
        fake.app(&state),
        "PUT",
        &settings_uri,
        Some(valid.clone()),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let stored = json_of(&body);
    assert_eq!(stored["monitors"][0]["onError"], "pause", "{body}");
    let (status, body) = send(
        fake.app(&state),
        "PUT",
        &chat_uri,
        Some(json!({ "question": yes_no(Some(0.9)) })),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let stored_chat = json_of(&body);
    let before = state
        .session_manager()
        .get_session(id, false)
        .await
        .unwrap()
        .extension_data;

    /// Rename a key of the first monitor, as a person misspelling it would.
    fn respell(value: &mut Value, from: &str, to: &str) {
        let monitor = value["monitors"][0].as_object_mut().unwrap();
        let field = monitor.remove(from).unwrap();
        monitor.insert(to.to_string(), field);
    }
    /// An edit to the valid body, and the sentence its write is refused with.
    type Case = (fn(&mut Value), &'static str);
    let cases: [Case; 8] = [
        (
            |v| respell(v, "onError", "on_error"),
            "`on_error` is not a field of monitor `m1`; the field is spelled `onError`.",
        ),
        (
            |v| respell(v, "action", "Action"),
            "`Action` is not a field of monitor `m1`; the field is spelled `action`.",
        ),
        (
            |v| respell(v, "maxEvalsPerTurn", "max_evals_per_turn"),
            "`max_evals_per_turn` is not a field of monitor `m1`; the field is spelled \
             `maxEvalsPerTurn`.",
        ),
        (
            |v| {
                let window = v["monitors"][0]["window"].as_object_mut().unwrap();
                let field = window.remove("maxChars").unwrap();
                window.insert("max_chars".to_string(), field);
            },
            "`max_chars` is not a field of `window` in monitor `m1`; the field is spelled \
             `maxChars`.",
        ),
        (
            |v| v["monitors"][0]["question"]["Threshold"] = json!({ "yes": 0.5 }),
            "`Threshold` is not a field of `question` in monitor `m1`; the field is spelled \
             `threshold`.",
        ),
        (
            |v| {
                let root = v.as_object_mut().unwrap();
                let field = root.remove("defaultModel").unwrap();
                root.insert("default_model".to_string(), field);
            },
            "`default_model` is not a field of the decision settings; the field is spelled \
             `defaultModel`.",
        ),
        (
            |v| v["monitors"][0]["colour"] = json!("red"),
            "`colour` is not a field of monitor `m1`. The fields accepted there are `id`, \
             `name`, `question`, `model`, `timing`, `window`, `threshold`, `direction`, \
             `option`, `consecutive`, `rearmBelow`, `rearmAbove`, `action`, `onError`, \
             `maxEvalsPerTurn`, `enabled`, `setBy`, `setBySession`, `inheritedFrom`, \
             `applyToSubagents`.",
        ),
        (
            // A monitor with no id is named by its place in the list.
            |v| {
                let mut second = v["monitors"][0].clone();
                second.as_object_mut().unwrap().remove("id");
                second["on_error"] = json!("pause");
                v["monitors"].as_array_mut().unwrap().push(second);
            },
            "`on_error` is not a field of monitor 2 in the list; the field is spelled \
             `onError`.",
        ),
    ];
    for (edit, sentence) in cases {
        let mut body = valid.clone();
        edit(&mut body);
        let (status, got) =
            send(fake.app(&state), "PUT", &settings_uri, Some(body), &[PROOF]).await;
        assert_eq!(
            (status, got.as_str()),
            (
                StatusCode::BAD_REQUEST,
                format!("The decision settings could not be read: {sentence}").as_str()
            )
        );
    }

    for (body, sentence) in [
        (
            json!({ "Question": yes_no(None) }),
            "`Question` is not a field of the decision-chat settings; the field is spelled \
             `question`.",
        ),
        (
            json!({ "question": yes_no(None), "questions": yes_no(None) }),
            "`questions` is not a field of the decision-chat settings. The fields accepted \
             there are `question`.",
        ),
    ] {
        let (status, got) = send(fake.app(&state), "PUT", &chat_uri, Some(body), &[PROOF]).await;
        assert_eq!(
            (status, got.as_str()),
            (
                StatusCode::BAD_REQUEST,
                format!("The decision-chat settings could not be read: {sentence}").as_str()
            )
        );
    }

    // Every other problem keeps serde's own text, as before: a mistyped
    // value, trailing bytes and a key given twice are still refused.
    let mut mistyped = valid.clone();
    mistyped["monitors"][0]["consecutive"] = json!("two");
    let (status, got) = send(
        fake.app(&state),
        "PUT",
        &settings_uri,
        Some(mistyped),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{got}");
    assert!(
        got.starts_with("The decision settings could not be read: invalid type: string \"two\""),
        "{got}"
    );
    for (raw, needle) in [
        (r#"{"monitors": []} x"#, "trailing characters"),
        (
            r#"{"monitors": [], "monitors": []}"#,
            "duplicate field `monitors`",
        ),
    ] {
        let (status, got) = put_raw(&state, &settings_uri, raw).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{raw}: {got}");
        assert!(
            got.starts_with("The decision settings could not be read: ") && got.contains(needle),
            "{raw}: {got}"
        );
    }

    let (status, body) = send(fake.app(&state), "GET", &settings_uri, None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json_of(&body),
        stored,
        "a refused write changed the settings"
    );
    let (status, body) = send(fake.app(&state), "GET", &chat_uri, None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json_of(&body),
        stored_chat,
        "a refused write changed the question"
    );
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

/// What `GET` answers is a body `PUT` accepts and stores unchanged, every
/// field set: refusing unknown keys refuses nothing a reader is handed.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn what_the_settings_read_answers_is_written_back_unchanged() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake_full();
    let chat = seed(&state, SessionClassification::Public).await;
    let id = chat.id.as_str();
    let settings_uri = format!("/sessions/{id}/decision-settings");
    let chat_uri = format!("/sessions/{id}/decision-chat");

    let mut choice = monitor("subtype");
    choice["name"] = json!("Secondary progressive");
    choice["question"] = json!({
        "instructions": "Which MS subtype does the note describe?",
        "kind": { "choice": { "options": { "rrms": "Relapsing-remitting", "spms": null } } }
    });
    choice["model"] = json!({ "provider": "ollama_decision", "model": "tev1:0.8b" });
    choice["timing"] = json!("turn_end");
    choice["window"] = json!({ "entries": 6, "maxChars": 1200, "toolCalls": false });
    choice["option"] = json!("spms");
    choice["consecutive"] = json!(2);
    choice["action"] = json!("steer");
    choice["onError"] = json!("pause");
    choice["maxEvalsPerTurn"] = json!(12);
    choice["enabled"] = json!(false);
    let written = json!({
        "defaultModel": { "provider": "ollama_decision", "model": "tev1:0.8b" },
        "monitors": [monitor("m1"), choice]
    });

    for (uri, body) in [
        (&settings_uri, written),
        (&chat_uri, json!({ "question": yes_no(Some(0.9)) })),
    ] {
        let (status, put) = send(fake.app(&state), "PUT", uri, Some(body), &[PROOF]).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {put}");
        let (status, read) = send(fake.app(&state), "GET", uri, None, &[]).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {read}");
        let read = json_of(&read);
        // The decision-chat read wraps the settings as `{ settings }`; the
        // write takes them bare.
        let back = if uri == &chat_uri {
            read["settings"].clone()
        } else {
            read.clone()
        };
        let (status, again) = send(fake.app(&state), "PUT", uri, Some(back), &[PROOF]).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {again}");
        assert_eq!(
            json_of(&again),
            read,
            "{uri}: the write-back stored what was read"
        );
        let (status, reread) = send(fake.app(&state), "GET", uri, None, &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json_of(&reread), read, "{uri}: unchanged by the round trip");
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
        fake.app(&state),
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
        fake.app(&state),
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

/// An ad hoc decision over a state the model's window cannot hold is 413 with
/// the size sentence, and nothing is sent (F9: the status was only ever
/// checked against the error table, never through a request).
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn an_oversized_adhoc_state_is_413_and_nothing_is_sent() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake_listing(vec![DecisionModelEntry {
        context_tokens: 2_048,
        ..entry("small-checker", ProviderTier::Private, true)
    }]);
    let (status, body) = send(
        fake.app(&state),
        "POST",
        "/decisions/decide",
        Some(json!({
            "model": { "provider": "test_decision", "model": "small-checker" },
            "questions": { "has_ms": yes_no(None) },
            "context": { "text": "Neuro follow-up, EDSS 2.0. ".repeat(600) }
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert!(
        body.starts_with("The text is about ")
            && body.contains(
                "`small-checker` takes beside these questions (its window is 2048 tokens)"
            ),
        "{body}"
    );
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
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
        fake.app(&state),
        "POST",
        "/decisions/decide",
        Some(ask(&public.id, None)),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let answer = json_of(&body);
    assert_eq!(answer["sessionId"], public.id.as_str());
    // A chat was named, and it is public: the caller's text is now in that
    // chat's log, where public chats can read it, and the answer says so. The
    // local model answered, so nothing says the text left this computer.
    assert_eq!(answer["note"], PUBLIC_CHAT_LOG_NOTE, "{body}");
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
        fake.app(&state),
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
        fake.app(&state),
        "POST",
        "/decisions/decide",
        Some(ask(&private.id, None)),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let answer = json_of(&body);
    assert_eq!(answer["decisions"][0]["model"]["model"], "local-checker");
    assert!(
        answer["note"].is_null(),
        "a private chat's log stays at private reach, and the model is local: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_batch_answers_every_chunk_in_order_and_a_malformed_request_is_400() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake();
    let app = || fake.app(&state);

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
    let app = || fake.app(&state);

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

/// A workspace rule that holds for one chat whatever model is asked.
struct ScopedTo(String);

#[async_trait]
impl DecisionUseGate for ScopedTo {
    async fn check(&self, session: &DecisionSession, _: &dyn DecisionModel) -> DecisionResult<()> {
        match self.session_refusal(session).await {
            Some(why) => Err(DecisionError::Refused(why)),
            None => Ok(()),
        }
    }

    async fn session_refusal(&self, session: &DecisionSession) -> Option<String> {
        (session.session_id == self.0)
            .then(|| biorouter::decision::service::CREW_SCOPED_REFUSAL.to_string())
    }
}

/// A chat under a Crew grant reaches no decision model, so the catalog says so
/// once for the chat as well as on every row: a monitor left on the chat's
/// decision model names no row, and the Monitors dialog must still refuse it
/// before it is saved. Another chat, and no chat, get `null`.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_catalog_names_a_refusal_that_holds_for_the_whole_chat() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let scoped = seed(&state, SessionClassification::Public).await;
    let other = seed(&state, SessionClassification::Public).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let service: &'static DecisionService = Box::leak(Box::new(
        DecisionService::new(Arc::new(FakeSource {
            calls: calls.clone(),
            extra: Vec::new(),
        }))
        .with_use_gate(Arc::new(ScopedTo(scoped.id.clone())))
        .with_default_model(None),
    ));
    let routes = || app(&state, service);
    let crew = biorouter::decision::service::CREW_SCOPED_REFUSAL;

    let uri = format!("/decisions/models?sessionId={}", scoped.id);
    let (status, body) = send(routes(), "GET", &uri, None, &[PROOF]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed = json_of(&body);
    assert_eq!(listed["sessionUnavailableReason"], crew, "{body}");
    assert!(
        listed["models"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["unavailableReason"] == crew),
        "{body}"
    );

    let uri = format!("/decisions/models?sessionId={}", other.id);
    let (status, body) = send(routes(), "GET", &uri, None, &[PROOF]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        json_of(&body)["sessionUnavailableReason"].is_null(),
        "{body}"
    );

    let (status, body) = send(routes(), "GET", "/decisions/models", None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed = json_of(&body);
    assert!(
        listed
            .as_object()
            .unwrap()
            .contains_key("sessionUnavailableReason")
            && listed["sessionUnavailableReason"].is_null(),
        "{body}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0, "a listing asks no model");
}

// ─── The save check (item 6) ───

/// [`monitor`] on the fake's model `model`, or on the chat's decision model.
fn monitor_on(id: &str, model: Option<&str>) -> Value {
    let mut monitor = monitor(id);
    monitor["model"] = match model {
        Some(model) => json!({ "provider": "test_decision", "model": model }),
        None => Value::Null,
    };
    monitor
}

/// The chat's decision settings and question as stored now.
async fn stored_settings(state: &Arc<AppState>, id: &str) -> (Value, Value) {
    let (status, settings) = send(
        served(state.clone()),
        "GET",
        &format!("/sessions/{id}/decision-settings"),
        None,
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{settings}");
    let (status, chat) = send(
        served(state.clone()),
        "GET",
        &format!("/sessions/{id}/decision-chat"),
        None,
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{chat}");
    (json_of(&settings), json_of(&chat))
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn an_unusable_save_is_422_with_the_sentence_and_nothing_is_stored() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake_full();
    let private = seed(&state, SessionClassification::Private).await;
    let uri = format!("/sessions/{}/decision-settings", private.id);

    let (status, body) = send(
        fake.app(&state),
        "PUT",
        &uri,
        Some(json!({ "monitors": [monitor_on("m1", Some("local-checker"))] })),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "what can run is stored: {body}");
    let before = stored_settings(&state, &private.id).await;

    let cases = [
        (
            json!({ "monitors": [
                monitor_on("m1", Some("local-checker")),
                monitor_on("m2", Some("cloud-checker"))
            ] }),
            "Nothing was saved: monitor \"Form reached\" could not run. This chat is private",
        ),
        (
            json!({ "monitors": [monitor_on("m2", Some("stale-checker"))] }),
            STALE_COPY,
        ),
        (
            json!({ "monitors": [monitor_on("m2", Some("gone"))] }),
            "Nothing was saved: monitor \"Form reached\" could not run.",
        ),
        (
            json!({
                "defaultModel": { "provider": "test_decision", "model": "cloud-checker" },
                "monitors": [monitor_on("m1", Some("local-checker"))]
            }),
            "Nothing was saved: the chat's default decision model test_decision/cloud-checker \
             could not run.",
        ),
    ];
    for (settings, needle) in cases {
        let (status, body) = send(fake.app(&state), "PUT", &uri, Some(settings), &[PROOF]).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert!(body.starts_with("Nothing was saved"), "{body}");
        assert!(body.contains(needle), "{needle} in {body}");
    }
    assert_eq!(
        stored_settings(&state, &private.id).await,
        before,
        "a refused save stored nothing"
    );
    assert_eq!(
        fake.calls.load(Ordering::SeqCst),
        0,
        "a check sends nothing"
    );
    assert!(fake.log.0.lock().unwrap().is_empty(), "nor logs a decision");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_save_check_comes_after_the_proof_and_the_key_refusal() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake();
    let chat = seed(&state, SessionClassification::Public).await;
    let uri = format!("/sessions/{}/decision-settings", chat.id);
    let before = stored_settings(&state, &chat.id).await;

    // A model that is not there would be refused 422, so a 422 below would
    // mean the check ran first.
    let gone = json!({ "monitors": [monitor_on("m1", Some("gone"))] });
    let (status, body) = send(fake.app(&state), "PUT", &uri, Some(gone.clone()), &[]).await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::FORBIDDEN, DECISION_SETTINGS_NEED_USER),
        "an unproven caller learns nothing about which models exist"
    );

    let mut misspelled = gone.clone();
    let monitor = misspelled["monitors"][0].as_object_mut().unwrap();
    let on_error = monitor.remove("onError").unwrap();
    monitor.insert("on_error".to_string(), on_error);
    let (status, body) = send(fake.app(&state), "PUT", &uri, Some(misspelled), &[PROOF]).await;
    assert_eq!(
        (status, body.as_str()),
        (
            StatusCode::BAD_REQUEST,
            "The decision settings could not be read: `on_error` is not a field of monitor \
             `m1`; the field is spelled `onError`."
        )
    );

    let mut out_of_bounds = gone.clone();
    out_of_bounds["monitors"][0]["threshold"] = json!(7.0);
    let (status, body) = send(fake.app(&state), "PUT", &uri, Some(out_of_bounds), &[PROOF]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(!body.starts_with("Nothing was saved"), "{body}");

    let (status, body) = send(fake.app(&state), "PUT", &uri, Some(gone), &[PROOF]).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "proven and well formed, the check runs: {body}"
    );
    assert_eq!(stored_settings(&state, &chat.id).await, before);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_decision_chats_question_its_model_cannot_answer_is_422() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake_full();
    let chat = seed(&state, SessionClassification::Public).await;
    // A decision chat: bound to a decision model that takes yes/no questions
    // only.
    state
        .session_manager()
        .update(&chat.id)
        .provider_name("ollama_decision")
        .model_config(ModelConfig::new("tev1:0.8b").unwrap())
        .apply()
        .await
        .unwrap();
    let uri = format!("/sessions/{}/decision-chat", chat.id);

    let choice = json!({ "question": {
        "instructions": "Which MS subtype does the note describe?",
        "kind": { "choice": { "options": { "rrms": "Relapsing-remitting", "spms": null } } }
    } });
    let (status, body) = send(fake.app(&state), "PUT", &uri, Some(choice), &[PROOF]).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body.starts_with(
            "Nothing was saved: this decision chat's model ollama_decision/tev1:0.8b could not \
             answer this question."
        ),
        "{body}"
    );
    let (_, question) = stored_settings(&state, &chat.id).await;
    assert_eq!(question, json!({ "settings": null }), "nothing was stored");

    let (status, body) = send(
        fake.app(&state),
        "PUT",
        &uri,
        Some(json!({ "question": yes_no(Some(0.9)) })),
        &[PROOF],
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a yes/no question it can answer: {body}"
    );
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
}

/// A workspace rule that refuses every model.
struct RefuseAll;

const REFUSE_ALL: &str = "This test's workspace lets no decision model read this chat.";

#[async_trait]
impl DecisionUseGate for RefuseAll {
    async fn check(&self, _: &DecisionSession, _: &dyn DecisionModel) -> DecisionResult<()> {
        Err(DecisionError::Refused(REFUSE_ALL.to_string()))
    }
}

/// Switching a monitor off, removing it, or saving it unchanged asks nothing,
/// so a monitor whose model went away can always be dealt with.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_save_that_turns_nothing_on_is_not_checked() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake();
    let chat = seed(&state, SessionClassification::Public).await;
    let uri = format!("/sessions/{}/decision-settings", chat.id);

    let mut off = monitor_on("m1", Some("gone"));
    off["enabled"] = json!(false);
    for settings in [
        json!({ "monitors": [off.clone()] }),
        json!({ "monitors": [] }),
    ] {
        let (status, body) = send(fake.app(&state), "PUT", &uri, Some(settings), &[PROOF]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let (status, body) = send(
        fake.app(&state),
        "PUT",
        &uri,
        Some(json!({ "monitors": [monitor_on("m1", Some("gone"))] })),
        &[PROOF],
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "on, the same monitor is checked: {body}"
    );

    // A monitor stored while it could run, whose model this chat may no
    // longer use: saved unchanged, switched off and removed, it is not asked.
    let on = json!({ "monitors": [monitor_on("m1", Some("local-checker"))] });
    let (status, body) = send(fake.app(&state), "PUT", &uri, Some(on.clone()), &[PROOF]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let refusing = fake_gated(Vec::new(), Arc::new(RefuseAll));
    let mut switched_off = on.clone();
    switched_off["monitors"][0]["enabled"] = json!(false);
    for settings in [on.clone(), switched_off, json!({ "monitors": [] })] {
        let (status, body) =
            send(refusing.app(&state), "PUT", &uri, Some(settings), &[PROOF]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let (status, body) = send(refusing.app(&state), "PUT", &uri, Some(on), &[PROOF]).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body.contains(REFUSE_ALL), "{body}");
}

/// A real provider carrying a chosen name and tier; nothing here completes a
/// turn with it.
struct Tiered {
    name: &'static str,
    tier: ProviderTier,
}

#[async_trait]
impl biorouter::providers::base::Provider for Tiered {
    fn metadata() -> biorouter::providers::base::ProviderMetadata {
        biorouter::providers::base::ProviderMetadata::empty()
    }
    fn get_name(&self) -> &str {
        self.name
    }
    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new("test-model").unwrap()
    }
    fn tier(&self) -> ProviderTier {
        self.tier
    }
    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        _system: &str,
        _messages: &[biorouter::conversation::message::Message],
        _tools: &[rmcp::model::Tool],
    ) -> anyhow::Result<
        (
            biorouter::conversation::message::Message,
            biorouter::providers::base::ProviderUsage,
        ),
        biorouter::providers::errors::ProviderError,
    > {
        Ok((
            biorouter::conversation::message::Message::assistant().with_text("ok"),
            biorouter::providers::base::ProviderUsage::new(
                self.name.to_string(),
                biorouter::providers::base::Usage::default(),
            ),
        ))
    }
}

/// A chat still Public but bound to a private model becomes private at its
/// first turn, where a public decision model would be refused at every check,
/// so the save refuses it now. Read off the agent this daemon holds; a chat
/// bound to no model is checked on its classification alone (a chat this
/// daemon does not hold:
/// [`a_chat_this_daemon_does_not_hold_is_checked_on_the_model_its_row_binds`]).
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_public_chat_bound_to_a_private_model_cannot_save_a_public_monitor() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake();
    let bound = seed(&state, SessionClassification::Public).await;
    let agent = state.get_agent(bound.id.clone()).await.unwrap();
    agent
        .update_provider(
            Arc::new(Tiered {
                name: "versa_azure",
                tier: ProviderTier::Private,
            }),
            &bound.id,
        )
        .await
        .unwrap();
    let uri = format!("/sessions/{}/decision-settings", bound.id);
    let public_monitor = json!({ "monitors": [monitor_on("m1", Some("cloud-checker"))] });

    let (status, body) = send(
        fake.app(&state),
        "PUT",
        &uri,
        Some(public_monitor.clone()),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body.contains("This chat runs on a private model, so it becomes private at its first turn"),
        "{body}"
    );
    let (status, body) = send(
        fake.app(&state),
        "PUT",
        &uri,
        Some(json!({ "monitors": [monitor_on("m1", Some("local-checker"))] })),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let unbound = seed(&state, SessionClassification::Public).await;
    let (status, body) = send(
        fake.app(&state),
        "PUT",
        &format!("/sessions/{}/decision-settings", unbound.id),
        Some(public_monitor),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// The public decision model a live save stored on a chat bound to a private
/// model: `baseten_decision/joey-1`, listed by the fake (nothing reaches
/// Baseten).
fn joey_entry() -> DecisionModelEntry {
    DecisionModelEntry {
        provider: "baseten_decision".to_string(),
        ..entry("joey-1", ProviderTier::Public, false)
    }
}

fn joey_model() -> Value {
    json!({ "provider": "baseten_decision", "model": "joey-1" })
}

/// A chat whose session row binds `provider` while it is still classified
/// Public: it has had no turn, so nothing has ratcheted it.
async fn seed_bound_row(state: &Arc<AppState>, provider: &str) -> Seeded {
    let chat = seed(state, SessionClassification::Public).await;
    state
        .session_manager()
        .update(&chat.id)
        .provider_name(provider)
        .model_config(ModelConfig::new("gpt-5.5").unwrap())
        .apply()
        .await
        .unwrap();
    let row = state
        .session_manager()
        .get_session(&chat.id, false)
        .await
        .unwrap();
    assert_eq!(row.provider_name.as_deref(), Some(provider));
    assert_eq!(row.privacy_tier, SessionClassification::Public);
    chat
}

/// Item 6, checked saves: a daemon that does not hold the chat's agent (right
/// after a restart, or once it was evicted) reads the chat model the session
/// row binds, so a chat still Public but bound to `versa_azure` is refused a
/// public decision model at the settings and the phase-gate doors with the
/// sentence the same save gets with the agent loaded, and nothing is stored.
/// An agent with no model bound yet reads the row too. A row on a public
/// provider still saves, and no check makes an agent or sends anything.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_chat_this_daemon_does_not_hold_is_checked_on_the_model_its_row_binds() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake_listing(vec![joey_entry()]);
    let mut joey_monitor = monitor("m1");
    joey_monitor["model"] = joey_model();
    let joey = json!({ "monitors": [joey_monitor] });
    let refusal = "Nothing was saved: monitor \"Form reached\" could not run. This chat runs on \
                   a private model, so it becomes private at its first turn, and `joey-1` is a \
                   public model a private chat cannot use.";

    let bound = seed_bound_row(&state, "versa_azure").await;
    assert!(
        state.peek_agent(&bound.id).await.is_none(),
        "this daemon holds no agent for the chat"
    );
    let uri = format!("/sessions/{}/decision-settings", bound.id);
    let before = stored_settings(&state, &bound.id).await;
    let (status, body) = send(fake.app(&state), "PUT", &uri, Some(joey.clone()), &[PROOF]).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body.starts_with(refusal), "{body}");
    assert_eq!(
        stored_settings(&state, &bound.id).await,
        before,
        "the read answers what it answered before"
    );

    // The phase-gate door reads the same bound tier.
    seed_plan(&state, &bound.id).await;
    let plan = stored_plan(&state, &bound.id).await;
    let mut gate = person_gate(None);
    gate["model"] = joey_model();
    let (status, body) = send(
        fake.app(&state),
        "PUT",
        &format!("/sessions/{}/phase-gates/research", bound.id),
        Some(gate),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body.starts_with("Phase \"research\"'s gate cannot be answered in this chat: ")
            && body.contains(
                "This chat runs on a private model, so it becomes private at its first turn"
            )
            && body.ends_with(". Nothing was changed."),
        "{body}"
    );
    assert_eq!(stored_plan(&state, &bound.id).await, plan, "nothing stored");

    // A private decision model is what such a chat may save.
    let (status, body) = send(
        fake.app(&state),
        "PUT",
        &uri,
        Some(json!({ "monitors": [monitor_on("m1", Some("local-checker"))] })),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        state.peek_agent(&bound.id).await.is_none(),
        "a check makes no agent"
    );

    // An agent this daemon holds with no model bound yet: the row says.
    let held = seed_bound_row(&state, "versa_azure").await;
    let agent = state.get_agent(held.id.clone()).await.unwrap();
    assert!(agent.provider().await.is_err(), "no model is bound yet");
    let (status, body) = send(
        fake.app(&state),
        "PUT",
        &format!("/sessions/{}/decision-settings", held.id),
        Some(joey.clone()),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body.starts_with(refusal), "{body}");

    // A row on a public provider still saves.
    let public = seed_bound_row(&state, "anthropic").await;
    assert!(state.peek_agent(&public.id).await.is_none());
    let (status, body) = send(
        fake.app(&state),
        "PUT",
        &format!("/sessions/{}/decision-settings", public.id),
        Some(joey),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (settings, _) = stored_settings(&state, &public.id).await;
    assert_eq!(settings["monitors"][0]["model"], joey_model(), "{settings}");
    assert_eq!(
        fake.calls.load(Ordering::SeqCst),
        0,
        "a check sends nothing"
    );
}

// ─── Monitors that also watch the subagents (item A) ───

/// A subagent's chat, deleted when the test's scope ends.
async fn subagent_chat(state: &Arc<AppState>, title: &str) -> Seeded {
    let session = state
        .session_manager()
        .create_session(
            PathBuf::from("/tmp/decisions_route"),
            title.to_string(),
            SessionType::SubAgent,
        )
        .await
        .unwrap();
    Seeded {
        state: state.clone(),
        id: session.id,
    }
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn put_syncs_subagent_monitors_to_live_children() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake();
    let parent = seed(&state, SessionClassification::Public).await;
    let running = subagent_chat(&state, "Decisions route, a running subagent (test fixture)").await;
    let idle = subagent_chat(&state, "Decisions route, an idle subagent (test fixture)").await;
    let handle = biorouter::agents::subagent_handle::BackgroundSubagent::register(
        parent.id.clone(),
        running.id.clone(),
        "delegated work",
        tokio_util::sync::CancellationToken::new(),
    );
    let uri = format!("/sessions/{}/decision-settings", parent.id);
    let shared = |threshold: f64| {
        let mut monitor = monitor_on("m1", Some("local-checker"));
        monitor["threshold"] = json!(threshold);
        monitor["applyToSubagents"] = json!(true);
        json!({ "monitors": [monitor] })
    };

    let (status, body) = send(fake.app(&state), "PUT", &uri, Some(shared(0.9)), &[PROOF]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let copied = stored_settings(&state, &running.id).await.0["monitors"].clone();
    assert_eq!(copied.as_array().map(Vec::len), Some(1), "{copied}");
    assert_eq!(copied[0]["id"], "m1", "{copied}");
    assert_eq!(copied[0]["inheritedFrom"], parent.id.as_str(), "{copied}");
    assert_eq!(copied[0]["threshold"], json!(0.9), "{copied}");
    assert!(copied[0].get("applyToSubagents").is_none(), "{copied}");
    assert_eq!(
        stored_settings(&state, &idle.id).await.0["monitors"],
        json!([]),
        "only a subagent the chat is running takes the copy"
    );

    // A changed monitor replaces the copy while the subagent runs…
    let (status, body) = send(fake.app(&state), "PUT", &uri, Some(shared(0.95)), &[PROOF]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        stored_settings(&state, &running.id).await.0["monitors"][0]["threshold"],
        json!(0.95)
    );

    // …and not once it has finished.
    handle.complete(biorouter::agents::subagent_result::SubagentResult::from_error("finished"));
    let (status, body) = send(fake.app(&state), "PUT", &uri, Some(shared(0.99)), &[PROOF]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        stored_settings(&state, &running.id).await.0["monitors"][0]["threshold"],
        json!(0.95)
    );
}

// ─── The person's phase-gate controls (item C) ───

/// A plan of three phases: `intro`, completed; `research`, gated by the
/// agent; `write`, after it, so held by its gate.
async fn seed_plan(state: &Arc<AppState>, id: &str) {
    let phases: Vec<orchestration_plan::OrchestratePhaseInput> = serde_json::from_value(json!([
        { "id": "intro", "title": "Intro", "kind": "plan" },
        {
            "id": "research",
            "title": "Research",
            "kind": "research",
            "gate": { "questions": { "done": "Is the research done?" }, "threshold": 0.8 }
        },
        { "id": "write", "title": "Write", "kind": "implementation" }
    ]))
    .unwrap();
    let plan = orchestration_plan::apply_gated_plan_as(
        orchestration_plan::PlanAuthor::Agent,
        None,
        Some("Gate route plan"),
        None,
        &phases,
    )
    .unwrap();
    let (plan, _) = orchestration_plan::apply_agent_complete(
        Some(plan),
        "intro",
        orchestration_plan::PhaseStatus::Completed,
        Some("Introduced."),
        &[],
        chrono::Utc::now(),
    )
    .unwrap();
    let record = orchestration_plan::plan_record(&plan);
    state
        .session_manager()
        .update_extension_state(
            id,
            orchestration_plan::PLAN_STATE,
            orchestration_plan::PLAN_VERSION,
            move |_| Ok(record),
        )
        .await
        .unwrap()
        .expect("the chat exists");
}

/// The plan as stored, to compare before and after a refused write.
async fn stored_plan(state: &Arc<AppState>, id: &str) -> Option<Value> {
    state
        .session_manager()
        .get_extension_state(
            id,
            orchestration_plan::PLAN_STATE,
            orchestration_plan::PLAN_VERSION,
        )
        .await
        .unwrap()
}

/// One phase of a plan snapshot, by id.
fn phase_of<'a>(plan: &'a Value, id: &str) -> &'a Value {
    plan["phases"]
        .as_array()
        .and_then(|phases| phases.iter().find(|phase| phase["id"] == id))
        .unwrap_or_else(|| panic!("no phase `{id}` in {plan}"))
}

fn person_gate(model: Option<&str>) -> Value {
    let mut gate = json!({
        "questions": { "verified": "Did a second source confirm the finding?" },
        "threshold": 0.3,
        "maxAttempts": 2
    });
    if let Some(model) = model {
        gate["model"] = json!({ "provider": "test_decision", "model": model });
    }
    gate
}

/// The three gate writes on `phase`, as `(method, uri, body)`.
fn gate_writes(id: &str, phase: &str) -> [(&'static str, String, Option<Value>); 3] {
    [
        (
            "PUT",
            format!("/sessions/{id}/phase-gates/{phase}"),
            Some(person_gate(None)),
        ),
        (
            "POST",
            format!("/sessions/{id}/phase-gates/{phase}/decision"),
            Some(json!({ "decision": "retry" })),
        ),
        (
            "DELETE",
            format!("/sessions/{id}/phase-gates/{phase}"),
            None,
        ),
    ]
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn put_phase_gate_needs_the_users_proof() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake();
    let chat = seed(&state, SessionClassification::Public).await;
    seed_plan(&state, &chat.id).await;
    let before = stored_plan(&state, &chat.id).await;

    for (method, uri, body) in gate_writes(&chat.id, "research")
        .into_iter()
        .chain(gate_writes(&chat.id, "Not_A_Phase"))
    {
        // Reachable (a public chat) and still refused; a stated private
        // capability is reach, not a person; and the proof comes before the
        // phase id is even read.
        for headers in [&[][..], &[PRIVATE_CAPABILITY][..]] {
            let (status, got) = send(fake.app(&state), method, &uri, body.clone(), headers).await;
            assert_eq!(
                (status, got.as_str()),
                (StatusCode::FORBIDDEN, PHASE_GATES_NEED_USER),
                "{method} {uri} {headers:?}"
            );
        }
    }
    assert_eq!(
        stored_plan(&state, &chat.id).await,
        before,
        "nothing changed"
    );
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn put_phase_gate_answers_404_without_a_plan_or_phase_and_409_on_a_closed_phase() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake();
    let bare = seed(&state, SessionClassification::Public).await;
    for (method, uri, body) in gate_writes(&bare.id, "research") {
        let (status, got) = send(fake.app(&state), method, &uri, body, &[PROOF]).await;
        assert_eq!(
            (status, got.as_str()),
            (
                StatusCode::NOT_FOUND,
                "This chat has no orchestration plan, so it has no phases to gate."
            ),
            "{method} {uri}"
        );
    }
    for (method, uri, body) in gate_writes("29990101_565656", "research") {
        let (status, got) = send(fake.app(&state), method, &uri, body, &[PROOF]).await;
        assert_eq!(
            (status, got.as_str()),
            (StatusCode::NOT_FOUND, "There is no chat with that id."),
            "{method} {uri}"
        );
    }

    let chat = seed(&state, SessionClassification::Public).await;
    seed_plan(&state, &chat.id).await;
    let before = stored_plan(&state, &chat.id).await;
    let mut cases: Vec<(&str, String, Option<Value>, StatusCode)> = Vec::new();
    for (method, uri, body) in gate_writes(&chat.id, "nope") {
        cases.push((method, uri, body, StatusCode::NOT_FOUND));
    }
    for (method, uri, body) in gate_writes(&chat.id, "intro") {
        cases.push((method, uri, body, StatusCode::CONFLICT));
    }
    // `write` has no gate to decide or remove.
    for (method, uri, body) in gate_writes(&chat.id, "write").into_iter().skip(1) {
        cases.push((method, uri, body, StatusCode::CONFLICT));
    }
    for (method, uri, body) in gate_writes(&chat.id, "Not_A_Phase") {
        cases.push((method, uri, body, StatusCode::BAD_REQUEST));
    }
    cases.push((
        "PUT",
        format!("/sessions/{}/phase-gates/research", chat.id),
        Some(json!({ "questions": {} })),
        StatusCode::BAD_REQUEST,
    ));
    for (method, uri, body, expected) in cases {
        let (status, got) = send(fake.app(&state), method, &uri, body, &[PROOF]).await;
        assert_eq!(status, expected, "{method} {uri}: {got}");
        assert!(got.ends_with('.'), "{method} {uri}: a sentence: {got}");
    }
    let (status, got) = send(
        fake.app(&state),
        "PUT",
        &format!("/sessions/{}/phase-gates/Not_A_Phase", chat.id),
        Some(person_gate(None)),
        &[PROOF],
    )
    .await;
    assert_eq!(
        (status, got.as_str()),
        (StatusCode::BAD_REQUEST, NOT_A_PHASE_ID)
    );
    let (status, got) = send(
        fake.app(&state),
        "PUT",
        &format!("/sessions/{}/phase-gates/research", chat.id),
        Some(json!({ "questions": { "done": "Done?" }, "treshold": 0.9 })),
        &[PROOF],
    )
    .await;
    assert_eq!(
        (status, got.as_str()),
        (
            StatusCode::BAD_REQUEST,
            "The phase gate could not be read: `treshold` is not a field of the phase gate. The \
             fields accepted there are `questions`, `threshold`, `maxAttempts`, `model`, \
             `allowUncalibrated`."
        )
    );
    assert_eq!(
        stored_plan(&state, &chat.id).await,
        before,
        "nothing changed"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn put_phase_gate_refuses_a_public_model_for_a_private_chat_422() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake();
    let private = seed(&state, SessionClassification::Private).await;
    seed_plan(&state, &private.id).await;
    let before = stored_plan(&state, &private.id).await;
    let uri = format!("/sessions/{}/phase-gates/research", private.id);

    for (model, needle) in [("cloud-checker", "This chat is private"), ("gone", "gone")] {
        let (status, body) = send(
            fake.app(&state),
            "PUT",
            &uri,
            Some(person_gate(Some(model))),
            &[PROOF],
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert!(
            body.starts_with("Phase \"research\"'s gate cannot be answered in this chat: ")
                && body.contains(needle)
                && body.ends_with(". Nothing was changed.")
                && !body.contains(".. "),
            "{body}"
        );
    }
    assert_eq!(
        stored_plan(&state, &private.id).await,
        before,
        "nothing stored"
    );

    let (status, body) = send(
        fake.app(&state),
        "PUT",
        &uri,
        Some(person_gate(Some("local-checker"))),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        fake.calls.load(Ordering::SeqCst),
        0,
        "a check sends nothing"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn put_phase_gate_stores_a_person_owned_gate_and_returns_the_plan() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake();
    let chat = seed(&state, SessionClassification::Public).await;
    seed_plan(&state, &chat.id).await;

    let (status, body) = send(
        fake.app(&state),
        "PUT",
        &format!("/sessions/{}/phase-gates/research", chat.id),
        Some(person_gate(None)),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let answer = json_of(&body);
    assert_eq!(answer["applied"], true, "{body}");
    assert!(
        answer["message"].as_str().is_some_and(|m| !m.is_empty()),
        "{body}"
    );
    let gate = &phase_of(&answer["plan"], "research")["gate"];
    assert_eq!(gate["owner"], "person", "{body}");
    assert_eq!(
        gate["threshold"],
        json!(0.3),
        "a person may set a gate under the agent's 0.50 floor: {body}"
    );
    assert_eq!(gate["maxAttempts"], 2, "{body}");
    assert_eq!(gate["attempts"], 0, "{body}");
    assert_eq!(
        gate["questions"],
        json!({ "verified": "Did a second source confirm the finding?" }),
        "{body}"
    );
    assert_eq!(phase_of(&answer["plan"], "write")["heldBy"], "research");

    // The plan returned is the one `GET …/subagents` shows this caller.
    let (status, roster) = send(
        served(state.clone()),
        "GET",
        &format!("/sessions/{}/subagents", chat.id),
        None,
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{roster}");
    assert_eq!(json_of(&roster)["plan"], answer["plan"]);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn delete_phase_gate_releases_the_phases_after_it() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake();
    let chat = seed(&state, SessionClassification::Public).await;
    seed_plan(&state, &chat.id).await;
    let uri = format!("/sessions/{}/phase-gates/research", chat.id);

    let (status, body) = send(fake.app(&state), "DELETE", &uri, None, &[PROOF]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let answer = json_of(&body);
    assert_eq!(answer["applied"], true, "{body}");
    assert!(
        phase_of(&answer["plan"], "research")["gate"].is_null(),
        "{body}"
    );
    assert!(
        phase_of(&answer["plan"], "write")["heldBy"].is_null(),
        "the phase after it is released: {body}"
    );
    let (status, body) = send(fake.app(&state), "DELETE", &uri, None, &[PROOF]).await;
    assert_eq!(status, StatusCode::CONFLICT, "no gate left: {body}");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn decide_phase_gate_pass_applies_or_stands() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake();
    let chat = seed(&state, SessionClassification::Public).await;
    seed_plan(&state, &chat.id).await;
    let child = subagent_chat(&state, "Decisions route, a phase's subagent (test fixture)").await;
    orchestration_plan::record_spawn(
        state.session_manager(),
        &chat.id,
        "research",
        &child.id,
        None,
    )
    .await
    .unwrap();
    let handle = biorouter::agents::subagent_handle::BackgroundSubagent::register(
        chat.id.clone(),
        child.id.clone(),
        "research work",
        tokio_util::sync::CancellationToken::new(),
    );
    let uri = format!("/sessions/{}/phase-gates/research/decision", chat.id);
    let pass = json!({ "decision": "pass" });

    // Its subagent is still working: the pass stands.
    let (status, body) = send(fake.app(&state), "POST", &uri, Some(pass.clone()), &[PROOF]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let answer = json_of(&body);
    assert_eq!(answer["applied"], false, "{body}");
    let research = phase_of(&answer["plan"], "research");
    assert_ne!(research["status"], "completed", "{body}");
    assert_eq!(research["gate"]["personDecision"], "pass", "{body}");

    // Once it has finished, the pass closes the phase and releases `write`.
    handle.complete(biorouter::agents::subagent_result::SubagentResult::from_error("finished"));
    let (status, body) = send(fake.app(&state), "POST", &uri, Some(pass), &[PROOF]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let answer = json_of(&body);
    assert_eq!(answer["applied"], true, "{body}");
    assert_eq!(phase_of(&answer["plan"], "research")["status"], "completed");
    assert!(
        phase_of(&answer["plan"], "write")["heldBy"].is_null(),
        "{body}"
    );
    let (status, body) = send(
        fake.app(&state),
        "POST",
        &uri,
        Some(json!({ "decision": "retry" })),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "a closed phase: {body}");

    // A retry gives an open gate its attempts again.
    let other = seed(&state, SessionClassification::Public).await;
    seed_plan(&state, &other.id).await;
    let (status, body) = send(
        fake.app(&state),
        "POST",
        &format!("/sessions/{}/phase-gates/research/decision", other.id),
        Some(json!({ "decision": "retry" })),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let answer = json_of(&body);
    assert_eq!(answer["applied"], true, "{body}");
    let gate = &phase_of(&answer["plan"], "research")["gate"];
    assert_eq!(gate["attempts"], 0, "{body}");
    assert_eq!(gate["retriesGranted"], 1, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn decide_phase_gate_body_refuses_unknown_keys() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let fake = fake();
    let chat = seed(&state, SessionClassification::Public).await;
    seed_plan(&state, &chat.id).await;
    let before = stored_plan(&state, &chat.id).await;
    let uri = format!("/sessions/{}/phase-gates/research/decision", chat.id);

    for (body, expected) in [
        (
            json!({ "decision": "pass", "reason": "looks done" }),
            "The gate decision could not be read: `reason` is not a field of the gate decision. \
             The fields accepted there are `decision`.",
        ),
        (
            json!({ "Decision": "pass" }),
            "The gate decision could not be read: `Decision` is not a field of the gate \
             decision; the field is spelled `decision`.",
        ),
    ] {
        let (status, got) = send(fake.app(&state), "POST", &uri, Some(body), &[PROOF]).await;
        assert_eq!((status, got.as_str()), (StatusCode::BAD_REQUEST, expected));
    }
    for body in [json!({ "decision": "maybe" }), json!({})] {
        let (status, got) = send(fake.app(&state), "POST", &uri, Some(body), &[PROOF]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{got}");
        assert!(
            got.starts_with("The gate decision could not be read: "),
            "{got}"
        );
    }
    assert_eq!(
        stored_plan(&state, &chat.id).await,
        before,
        "nothing changed"
    );
}

// ─── Installing a local decision model (item 5) ───

/// Ollama's load-probe answer for a copy it loads.
fn loads() -> wiremock::ResponseTemplate {
    wiremock::ResponseTemplate::new(400)
        .set_body_json(json!({ "error": "questions must contain 1\u{2013}64 fields" }))
}

/// A successful pull of two layers, streamed after `delay`.
fn pulled(delay: std::time::Duration) -> wiremock::ResponseTemplate {
    let lines = [
        json!({ "status": "pulling manifest" }),
        json!({ "status": "pulling aaa", "digest": "sha256:aaa", "total": 1000, "completed": 500 }),
        json!({ "status": "pulling bbb", "digest": "sha256:bbb", "total": 200, "completed": 200 }),
        json!({ "status": "pulling aaa", "digest": "sha256:aaa", "total": 1000, "completed": 1000 }),
        json!({ "status": "verifying sha256 digest" }),
        json!({ "status": "success" }),
    ];
    wiremock::ResponseTemplate::new(200)
        .insert_header("content-type", "application/x-ndjson")
        .set_body_string(
            lines
                .iter()
                .map(|line| format!("{line}\n"))
                .collect::<String>(),
        )
        .set_delay(delay)
}

/// A local Ollama that answers its version, pulls with `pull`, reports the
/// `decision` capability and loads every copy, listing nothing pulled.
async fn mock_ollama(pull: wiremock::ResponseTemplate) -> wiremock::MockServer {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/version"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "version": "0.40.15" })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/pull"))
        .respond_with(pull)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/show"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "capabilities": ["decision"] })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(loads())
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "models": [] })))
        .with_priority(5)
        .mount(&server)
        .await;
    server
}

/// This module's routes with an installer for the Ollama at `base`.
fn install_app(state: &Arc<AppState>, fake: &Fake, base: &str) -> Router {
    let base = url::Url::parse(base).unwrap();
    let installer: &'static DecisionInstaller = Box::leak(Box::new(
        DecisionInstaller::new(move || Ok(base.clone()))
            .with_idle_timeout(std::time::Duration::from_secs(5)),
    ));
    routes_with_service(state.clone(), fake.service, installer)
}

async fn pull_requests(server: &wiremock::MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|request| request.url.path() == "/api/pull")
        .count()
}

/// Poll `GET /decisions/models/install` until the job is no longer running,
/// returning every job state seen on the way, in order, and the last answer.
async fn install_states(app: &Router) -> (Vec<String>, Value) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut seen: Vec<String> = Vec::new();
    loop {
        let (status, body) = send(app.clone(), "GET", "/decisions/models/install", None, &[]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let report = json_of(&body);
        let state = report["job"]["state"]
            .as_str()
            .unwrap_or("none")
            .to_string();
        if seen.last() != Some(&state) {
            seen.push(state.clone());
        }
        if state == "installed" || state == "failed" {
            return (seen, report);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the install never finished: {seen:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_install_route_refuses_an_unlisted_model() {
    let state = AppState::new().await.unwrap();
    let fake = fake();
    let server = mock_ollama(pulled(std::time::Duration::ZERO)).await;
    let app = install_app(&state, &fake, &server.uri());

    let (status, body) = send(
        app.clone(),
        "POST",
        "/decisions/models/install",
        Some(json!({ "model": "llama3.1:8b" })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body,
        "`llama3.1:8b` is not a decision model Biorouter installs. It installs tev1:0.8b and \
         clef-flash:latest; pull any other with `ollama pull llama3.1:8b`."
    );
    let (status, body) = send(
        app,
        "POST",
        "/decisions/models/install",
        Some(json!({ "name": "tev1:0.8b" })),
        &[],
    )
    .await;
    assert_eq!(
        (status, body.as_str()),
        (
            StatusCode::BAD_REQUEST,
            "The install request could not be read: `name` is not a field of the install \
             request. The fields accepted there are `model`."
        )
    );
    assert_eq!(pull_requests(&server).await, 0, "nothing was pulled");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_install_route_says_ollama_is_not_running() {
    let state = AppState::new().await.unwrap();
    let fake = fake();
    // A local address that answers nothing as Ollama does.
    let server = wiremock::MockServer::start().await;
    let app = install_app(&state, &fake, &server.uri());

    let (status, body) = send(app.clone(), "GET", "/decisions/models/install", None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let report = json_of(&body);
    assert_eq!(report["ollama"]["state"], "not_running", "{body}");
    assert!(report["ollama"]["version"].is_null(), "{body}");
    let message = report["ollama"]["message"].as_str().unwrap().to_string();
    assert!(message.starts_with("Ollama is not running at "), "{body}");
    assert!(report["job"].is_null(), "{body}");
    assert!(
        report["offers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|offer| offer["installed"] == false),
        "{body}"
    );

    let (status, body) = send(app, "POST", "/decisions/models/install", None, &[]).await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, message.as_str())
    );
    assert_eq!(pull_requests(&server).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_install_route_refuses_a_remote_ollama() {
    let state = AppState::new().await.unwrap();
    let fake = fake();
    // Not this computer: decided from the address alone, with nothing sent.
    let app = install_app(&state, &fake, "http://10.255.255.1:11434");

    let (status, body) = send(app.clone(), "GET", "/decisions/models/install", None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let report = json_of(&body);
    assert_eq!(report["ollama"]["state"], "not_this_computer", "{body}");
    assert_eq!(
        report["ollama"]["baseUrl"], "http://10.255.255.1:11434/",
        "{body}"
    );

    let (status, body) = send(
        app,
        "POST",
        "/decisions/models/install",
        Some(json!({ "model": "tev1:0.8b" })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        body.starts_with("Biorouter installs decision models only into an Ollama on this computer"),
        "{body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_second_model_waits_for_the_first() {
    let state = AppState::new().await.unwrap();
    let fake = fake();
    // Long enough that all three requests land while the first one runs.
    let server = mock_ollama(pulled(std::time::Duration::from_secs(3))).await;
    let app = install_app(&state, &fake, &server.uri());

    let (status, body) = send(
        app.clone(),
        "POST",
        "/decisions/models/install",
        Some(json!({ "model": "tev1:0.8b" })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let (status, body) = send(
        app.clone(),
        "POST",
        "/decisions/models/install",
        Some(json!({ "model": "clef-flash" })),
        &[],
    )
    .await;
    assert_eq!(
        (status, body.as_str()),
        (
            StatusCode::CONFLICT,
            "tev1:0.8b is still downloading. Wait for it to finish, then try again."
        )
    );
    // Asking for the model already installing joins it.
    let (status, body) = send(
        app.clone(),
        "POST",
        "/decisions/models/install",
        Some(json!({ "model": "tev1:0.8b" })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(json_of(&body)["job"]["model"], "tev1:0.8b", "{body}");

    let (_, report) = install_states(&app).await;
    assert_eq!(report["job"]["state"], "installed", "{report}");
    assert_eq!(
        pull_requests(&server).await,
        1,
        "one pull for both requests"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_install_route_starts_and_reports_progress() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};
    let state = AppState::new().await.unwrap();
    let fake = fake();
    // Long enough that the first poll sees the download running.
    let server = mock_ollama(pulled(std::time::Duration::from_millis(1500))).await;
    let app = install_app(&state, &fake, &server.uri());

    let (status, body) = send(app.clone(), "GET", "/decisions/models/install", None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let report = json_of(&body);
    assert_eq!(report["ollama"]["state"], "running", "{body}");
    assert_eq!(report["ollama"]["version"], "0.40.15", "{body}");
    assert!(report["ollama"]["message"].is_null(), "{body}");
    assert_eq!(
        report["offers"],
        json!([
            {
                "model": "tev1:0.8b",
                "displayName": "tev1:0.8b",
                "approxSize": "about 1.5 GB",
                "note": "small and fast",
                "isDefault": true,
                "installed": false
            },
            {
                "model": "clef-flash:latest",
                "displayName": "clef-flash:latest",
                "approxSize": "about 12 GB",
                "note": "larger, also reads images",
                "isDefault": false,
                "installed": false
            }
        ]),
        "{body}"
    );

    // An empty body installs the default model.
    let (status, body) = send(app.clone(), "POST", "/decisions/models/install", None, &[]).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let started = json_of(&body);
    assert_eq!(started["job"]["model"], "tev1:0.8b", "{body}");
    assert_eq!(started["job"]["state"], "downloading", "{body}");

    let (seen, report) = install_states(&app).await;
    assert_eq!(
        seen.first().map(String::as_str),
        Some("downloading"),
        "{seen:?}"
    );
    assert_eq!(
        seen.last().map(String::as_str),
        Some("installed"),
        "{seen:?}"
    );
    assert_eq!(report["job"]["completedBytes"], 1200, "{report}");
    assert_eq!(report["job"]["totalBytes"], 1200, "{report}");
    assert!(report["job"]["error"].is_null(), "{report}");

    // Ollama lists it now, and so does the install status.
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "models": [
                { "name": "tev1:0.8b", "model": "tev1:0.8b", "digest": "route-test-tev1" }
            ] })))
        .with_priority(1)
        .mount(&server)
        .await;
    let (status, body) = send(app, "GET", "/decisions/models/install", None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let report = json_of(&body);
    assert_eq!(report["offers"][0]["installed"], true, "{body}");
    assert_eq!(report["offers"][1]["installed"], false, "{body}");
}

/// The Crew rule for decisions as a test gate: a chat under a grant reaches
/// only decision models that run on this computer.
struct CrewScoped(String);

impl CrewScoped {
    fn refusal(
        &self,
        session: &DecisionSession,
        affiliation: Option<ModelAffiliation>,
    ) -> Option<String> {
        (session.session_id == self.0
            && !biorouter::privacy::affiliation::runs_on_this_computer(affiliation.as_ref()))
        .then(|| biorouter::decision::service::CREW_SCOPED_REFUSAL.to_string())
    }
}

#[async_trait]
impl DecisionUseGate for CrewScoped {
    async fn check(
        &self,
        session: &DecisionSession,
        model: &dyn DecisionModel,
    ) -> DecisionResult<()> {
        match self.refusal(session, model.affiliation()) {
            Some(why) => Err(DecisionError::Refused(why)),
            None => Ok(()),
        }
    }

    async fn entry_refusal(
        &self,
        session: &DecisionSession,
        entry: &DecisionModelEntry,
    ) -> Option<String> {
        self.refusal(session, entry.affiliation)
    }
}

/// A Crew chat whose grant stands may use a decision model on this computer:
/// the catalog refuses only the hosted ones, row by row, and nothing for the
/// chat as a whole, and a save is checked the same way.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_catalog_for_a_crew_chat_refuses_only_hosted_models() {
    install_test_user_action_key();
    let state = AppState::new().await.unwrap();
    let crew_chat = seed(&state, SessionClassification::Public).await;
    let fake = fake_gated(Vec::new(), Arc::new(CrewScoped(crew_chat.id.clone())));
    let crew = biorouter::decision::service::CREW_SCOPED_REFUSAL;

    let (status, body) = send(
        fake.app(&state),
        "GET",
        &format!("/decisions/models?sessionId={}", crew_chat.id),
        None,
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed = json_of(&body);
    assert!(listed["sessionUnavailableReason"].is_null(), "{body}");
    for model in listed["models"].as_array().unwrap() {
        let expected = if model["local"] == true {
            Value::Null
        } else {
            json!(crew)
        };
        assert_eq!(model["unavailableReason"], expected, "{model}");
    }

    let uri = format!("/sessions/{}/decision-settings", crew_chat.id);
    let (status, body) = send(
        fake.app(&state),
        "PUT",
        &uri,
        Some(json!({ "monitors": [monitor_on("m1", Some("cloud-checker"))] })),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body.contains(crew), "{body}");
    let (status, body) = send(
        fake.app(&state),
        "PUT",
        &uri,
        Some(json!({ "monitors": [monitor_on("m1", Some("local-checker"))] })),
        &[PROOF],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
}

// ─── Census: every door that writes decision settings checks the save ───

/// `src` with its comments blanked and its string literals emptied, line for
/// line, except that a literal naming the settings key reads as the key's
/// constant, so a write that spells the key out is still found.
fn code_only(src: &str) -> Vec<String> {
    fn push(lines: &mut Vec<String>, ch: char) {
        if ch == '\n' {
            lines.push(String::new());
        } else if let Some(line) = lines.last_mut() {
            line.push(ch);
        }
    }
    let chars: Vec<char> = src.chars().collect();
    let mut lines = vec![String::new()];
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        let next = chars.get(i + 1).copied();
        if ch == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if ch == '/' && next == Some('*') {
            let mut depth = 0usize;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if chars[i] == '\n' {
                        push(&mut lines, '\n');
                    }
                    i += 1;
                }
            }
        } else if ch == '"' || (ch == 'r' && matches!(next, Some('"') | Some('#'))) {
            // A string or raw string literal: kept as `""`, its newlines kept.
            let hashes = if ch == 'r' {
                let mut j = i + 1;
                while chars.get(j) == Some(&'#') {
                    j += 1;
                }
                if chars.get(j) != Some(&'"') {
                    // `r#ident`, not a string.
                    push(&mut lines, ch);
                    i += 1;
                    continue;
                }
                let n = j - i - 1;
                i = j + 1;
                Some(n)
            } else {
                i += 1;
                None
            };
            let mut text = String::new();
            while i < chars.len() {
                let c = chars[i];
                match hashes {
                    None if c == '\\' => {
                        text.push(c);
                        if let Some(&escaped) = chars.get(i + 1) {
                            text.push(escaped);
                        }
                        i += 2;
                        continue;
                    }
                    None if c == '"' => {
                        i += 1;
                        break;
                    }
                    Some(n) if c == '"' && (0..n).all(|k| chars.get(i + 1 + k) == Some(&'#')) => {
                        i += 1 + n;
                        break;
                    }
                    _ => {}
                }
                text.push(c);
                i += 1;
            }
            if text.contains("decision_settings") {
                for c in "DECISION_SETTINGS_STATE".chars() {
                    push(&mut lines, c);
                }
            } else {
                push(&mut lines, '"');
                push(&mut lines, '"');
            }
            for _ in text.matches('\n') {
                push(&mut lines, '\n');
            }
        } else if ch == '\'' {
            // A character literal (`'"'`, `'\''`), or a lifetime.
            if next == Some('\\') {
                let close = (i + 2..chars.len()).find(|&j| chars[j] == '\'');
                i = close.map_or(chars.len(), |j| j + 1);
                push(&mut lines, '\'');
                push(&mut lines, '\'');
            } else if chars.get(i + 2) == Some(&'\'') {
                i += 3;
                push(&mut lines, '\'');
                push(&mut lines, '\'');
            } else {
                push(&mut lines, ch);
                i += 1;
            }
        } else {
            push(&mut lines, ch);
            i += 1;
        }
    }
    lines
}

/// `lines` with every `#[cfg(test)]` item blanked (a module, a function, a
/// `mod tests;`), found by brace depth as the privacy census does, so the
/// production code after a test module is still read.
fn without_test_items(mut lines: Vec<String>) -> Vec<String> {
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].trim_start().starts_with("#[cfg(test)]") {
            i += 1;
            continue;
        }
        let mut depth = 0i64;
        let mut opened = false;
        let mut j = i;
        loop {
            let line = std::mem::take(&mut lines[j]);
            for c in line.chars() {
                if c == '{' {
                    depth += 1;
                    opened = true;
                } else if c == '}' {
                    depth -= 1;
                }
            }
            let ends_item = (opened && depth <= 0) || (!opened && line.trim_end().ends_with(';'));
            if ends_item || j + 1 == lines.len() {
                break;
            }
            j += 1;
        }
        i = j + 1;
    }
    lines
}

/// The production `.rs` files under `root`: test files and `tests/` folders
/// left out.
fn production_sources(root: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(root).expect("a readable source folder") {
        let path = entry.expect("a readable entry").path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if name != "tests" {
                production_sources(&path, out);
            }
        } else if name.ends_with(".rs")
            && name != "tests.rs"
            && !name.ends_with("_tests.rs")
            && !name.ends_with("_test.rs")
        {
            out.push(path);
        }
    }
}

/// The text of the call whose arguments start at `from` (just past its `(`),
/// up to its closing parenthesis.
fn call_arguments(code: &str, from: usize) -> &str {
    let rest = code.get(from..).unwrap_or_default();
    let mut depth = 1i64;
    for (offset, c) in rest.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return rest.get(..offset).unwrap_or(rest);
                }
            }
            _ => {}
        }
    }
    rest
}

/// Every door a person or an agent saves decision settings through runs the
/// save check, so nothing stores a monitor that cannot run without saying so.
/// A raw write (`settings::save`, `save_chat`, `update`, or an extension-state
/// write naming the settings key) may appear only in `decision/settings.rs`
/// and `decision/save.rs`, which define them, or in a listed door whose file
/// also asks the service (`check_settings(` or `check_model_use(`). A new door
/// calls `decision::save_checked`, or joins the list with that check.
#[test]
fn every_door_that_writes_decision_settings_runs_the_save_check() {
    const DEFINED_IN: [&str; 2] = [
        "crates/biorouter/src/decision/settings.rs",
        "crates/biorouter/src/decision/save.rs",
    ];
    // Doors that write through `settings::update` or the raw key inside a
    // transaction of their own, each after asking the service.
    const CHECKED_DOORS: [&str; 3] = [
        "crates/biorouter/src/agents/subagent_tool.rs",
        "crates/biorouter/src/workflow/supervision.rs",
        "crates/biorouter/src/agents/workspace_extension.rs",
    ];
    // Called as `<…settings>::<name>(`: `settings::save(`,
    // `decision_settings::save(`, `crate::decision::settings::update(`.
    const WRITERS: [&str; 3] = ["save", "save_chat", "update"];
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let mut files = Vec::new();
    for root in [
        "crates/biorouter/src",
        "crates/biorouter-server/src",
        "crates/biorouter-cli/src",
    ] {
        production_sources(&repo.join(root), &mut files);
    }
    files.sort();
    assert!(
        files.len() > 100,
        "the scan found the source tree: {}",
        files.len()
    );

    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut found: Vec<(String, usize, String)> = Vec::new();
    for file in &files {
        let rel = file
            .strip_prefix(&repo)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let src = std::fs::read_to_string(file).unwrap();
        let code = without_test_items(code_only(&src)).join("\n");
        let before = |at: usize| code.get(..at).unwrap_or_default();
        let line_of = |at: usize| before(at).matches('\n').count() + 1;
        for writer in WRITERS {
            let call = format!("::{writer}(");
            for (at, _) in code.match_indices(&call) {
                // The path segment just before `::`.
                let start = before(at)
                    .char_indices()
                    .rev()
                    .find(|(_, c)| !is_word(*c))
                    .map_or(0, |(i, c)| i + c.len_utf8());
                let module = before(at).get(start..).unwrap_or_default();
                if module.ends_with("settings") {
                    found.push((rel.clone(), line_of(at), format!("{module}{call}")));
                }
            }
        }
        // `save_chat` imported and called bare. A whole name (`xsave_chat(`
        // is another function), not a path (counted above), not the
        // definition.
        for (at, _) in code.match_indices("save_chat(") {
            let before = before(at);
            if before.chars().next_back().is_some_and(is_word)
                || before.ends_with("::")
                || before.ends_with("fn ")
            {
                continue;
            }
            found.push((rel.clone(), line_of(at), "save_chat(".to_string()));
        }
        for opener in [
            "update_extension_state(",
            "update_extension_data(",
            "set_extension_state(",
        ] {
            for (at, _) in code.match_indices(opener) {
                if before(at).ends_with("fn ") {
                    continue;
                }
                let arguments = call_arguments(&code, at + opener.len());
                if arguments.contains("DECISION_SETTINGS_STATE") {
                    found.push((rel.clone(), line_of(at), opener.to_string()));
                }
            }
        }
    }

    assert!(
        found
            .iter()
            .filter(|(file, ..)| file == "crates/biorouter/src/decision/save.rs")
            .count()
            >= 2,
        "the scan sees the two checked writes in decision/save.rs: {found:#?}"
    );
    let checked = |file: &str| {
        let src = std::fs::read_to_string(repo.join(file)).unwrap_or_default();
        let code = without_test_items(code_only(&src)).join("\n");
        code.contains("check_settings(") || code.contains("check_model_use(")
    };
    let allowed =
        |file: &str| DEFINED_IN.contains(&file) || (CHECKED_DOORS.contains(&file) && checked(file));
    let unchecked: Vec<String> = found
        .iter()
        .filter(|(file, ..)| !allowed(file))
        .map(|(file, line, writer)| format!("{file}:{line}: `{writer}`"))
        .collect();
    assert!(
        unchecked.is_empty(),
        "these store decision settings without the save check; call \
         `biorouter::decision::save_checked` (or `save_chat_checked`) instead, or, for a door \
         that must write inside its own transaction, ask `DecisionService::check_settings` or \
         `check_model_use` first and add the file to CHECKED_DOORS:\n  {}",
        unchecked.join("\n  ")
    );
}

/// The census's readers, on the shapes they must handle: a writer in a
/// comment, a string or a test module is not one; one after a test module
/// is; a write that spells the settings key out is found.
#[test]
fn the_settings_census_reads_code_and_only_code() {
    let src = r##"
// settings::save( in a comment
/* settings::save( in a block */
fn a() { let s = "settings::save(x)"; let c = '"'; }
#[cfg(test)]
mod tests {
    fn t() { settings::save(&m, id, s); }
}
fn b() { settings::save(&m, id, s); let r = r#"save_chat("#; }
fn c() { m.update_extension_state(id, "decision_settings", "v1", f); }
"##;
    let code = without_test_items(code_only(src)).join("\n");
    assert_eq!(code.matches("settings::save(").count(), 1, "{code}");
    assert!(!code.contains("save_chat("), "{code}");
    let at = code.find("update_extension_state(").unwrap();
    assert!(
        call_arguments(&code, at + "update_extension_state(".len())
            .contains("DECISION_SETTINGS_STATE"),
        "{code}"
    );
    assert_eq!(code.lines().count(), src.lines().count());
}
