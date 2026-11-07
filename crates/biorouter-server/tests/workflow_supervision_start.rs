//! Item 8 at `POST /agent/start`, on a daemon that HOLDS a user-action key
//! (the desktop's): a workflow that declares monitors or a phase plan
//! (`decision_settings`, `orchestration_plan`) has them applied to the new
//! chat before anything is sent in it, or is not started at all.
//!
//! - With the person's proof, the chat is seeded: `GET …/decision-settings`
//!   and `GET …/subagents` show the monitors and the plan.
//! - Without it, 403 with the workflow-supervision sentence, before any chat
//!   is made.
//! - A malformed block is 400, a model the chat may not use 422, a plan
//!   outside the Autonomous approval mode 409; each reads "The workflow was
//!   not started: <sentence>" and leaves no chat behind.
//!
//! The decision models are a fake source behind the daemon's decision
//! service (`AppState::set_decision_service`), so nothing here reaches a
//! model; a seed only checks, and the fake counts every request to prove it.
//!
//! ⚠ Its own binary: the installed digest is a process global (`OnceLock`), so
//! the keyless half lives in `workflow_supervision_start_no_key.rs`.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can open the developer's real `sessions.db`.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use biorouter::decision::model::DecisionCaps;
use biorouter::decision::types::{DecisionKindTag, ModelVerdicts};
use biorouter::decision::{
    DecisionAdapter, DecisionError, DecisionModel, DecisionModelEntry, DecisionModelRef,
    DecisionModelSource, DecisionRequest, DecisionResult, DecisionService, DecisionSession,
    DecisionUseGate,
};
use biorouter::privacy::{ModelAffiliation, ProviderTier};
use biorouter::providers::versa_azure::VERSA_AZURE_DEFAULT_MODEL;
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use tower::ServiceExt;

/// The user-action key the desktop app would hold. The daemon keeps only its
/// digest, exactly as `commands::agent` installs one read off stdin.
const USER_KEY: &str = "workflow-supervision-user-action-key";

fn install_the_desktop_key() {
    let digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(USER_KEY.as_bytes()).into();
    biorouter_server::auth::install_user_action_digest(Some(digest));
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("X-User-Action", USER_KEY.parse().unwrap());
    assert!(
        biorouter_server::auth::is_user_action(&headers),
        "the digest this binary installs did not take, so every positive arm below would be \
         measuring a refusal"
    );
}

// ---------------------------------------------------------------------------
// The fake decision models: one on this computer (private), one hosted
// (public). A seed never asks either, so `decide` counts and fails.
// ---------------------------------------------------------------------------

fn entry(provider: &str, model: &str, tier: ProviderTier) -> DecisionModelEntry {
    DecisionModelEntry {
        provider: provider.to_string(),
        model: model.to_string(),
        display_name: model.to_string(),
        context_tokens: 16_384,
        kinds: vec![DecisionKindTag::Binary],
        tier,
        affiliation: (tier == ProviderTier::Private).then_some(ModelAffiliation::Local),
        adapter: DecisionAdapter::SystemOne,
        local: tier == ProviderTier::Private,
        load_refusal: None,
    }
}

fn listed() -> Vec<DecisionModelEntry> {
    vec![
        entry("ollama_decision", "tev1:0.8b", ProviderTier::Private),
        entry("baseten_decision", "joey-1", ProviderTier::Public),
    ]
}

struct FakeModel {
    entry: DecisionModelEntry,
    requests: Arc<AtomicUsize>,
}

#[async_trait]
impl DecisionModel for FakeModel {
    fn model_ref(&self) -> DecisionModelRef {
        self.entry.model_ref()
    }
    fn display_name(&self) -> String {
        self.entry.model.clone()
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
            max_questions: 8,
            context_tokens: self.entry.context_tokens,
            max_concurrency: 1,
            max_request_bytes: None,
            local: self.entry.local,
        }
    }
    async fn decide(&self, _request: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        Err(DecisionError::Unsupported(
            "a seed must never ask a decision model".to_string(),
        ))
    }
}

struct FakeSource(Arc<AtomicUsize>);

#[async_trait]
impl DecisionModelSource for FakeSource {
    async fn list(&self) -> Vec<DecisionModelEntry> {
        listed()
    }
    async fn open(&self, wanted: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
        let entry = listed()
            .into_iter()
            .find(|entry| entry.answers_to(wanted))
            .ok_or_else(|| DecisionError::NoModel(format!("no `{}` here", wanted.label())))?;
        Ok(Arc::new(FakeModel {
            entry,
            requests: self.0.clone(),
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

/// The daemon's state over the fake service, and the count of requests it was
/// sent.
async fn daemon() -> (Arc<AppState>, Arc<AtomicUsize>) {
    install_the_desktop_key();
    biorouter_server::launch::record_launch_state(true);
    let requests = Arc::new(AtomicUsize::new(0));
    let service = DecisionService::new(Arc::new(FakeSource(requests.clone())))
        .with_use_gate(Arc::new(Allow))
        .with_default_model(None);
    let state = AppState::new().await.unwrap();
    // A route's state is `'static`; one small service per test.
    state.set_decision_service(Box::leak(Box::new(service)));
    (state, requests)
}

// ---------------------------------------------------------------------------
// Requests.
// ---------------------------------------------------------------------------

/// One request through the served tree, with the proof when `proven`, inside
/// `overrides` (a task-local configuration, as a probe would set it).
async fn send(
    state: &Arc<AppState>,
    method: &str,
    uri: &str,
    body: Option<Value>,
    proven: bool,
    overrides: HashMap<String, String>,
) -> (StatusCode, String) {
    let app = biorouter_server::routes::configure(state.clone(), "test-secret".to_string());
    let mut request = Request::builder().method(method).uri(uri);
    if proven {
        request = request.header("X-User-Action", USER_KEY);
    }
    let body = match body {
        Some(json) => {
            request = request.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&json).unwrap())
        }
        None => Body::empty(),
    };
    let request = request.body(body).unwrap();
    let response = biorouter::config::with_config_overrides(overrides, app.oneshot(request)).await;
    let response = response.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The monitor and the gated plan of `stop.yaml`, as the desktop posts them.
fn supervised_workflow(threshold: f64, monitor_model: &str) -> Value {
    let (provider, model) = monitor_model.split_once('/').unwrap();
    json!({
        "version": "1.0.0",
        "title": "Count to thirty",
        "description": "probe",
        "prompt": "Using the shell, echo the numbers 1 to 30, one echo command per tool call.",
        "decision_settings": {
            "defaultModel": {"provider": provider, "model": model},
            "monitors": [{
                "id": "reached-ten",
                "name": "Reached ten",
                "question": {
                    "instructions": "Has the agent already echoed 10 or a larger number?",
                    "kind": {"binary": {}}
                },
                "threshold": threshold,
                "action": "stop",
                "onError": "pause"
            }]
        },
        "orchestration_plan": {
            "title": "Count plan",
            "phases": [
                {"id": "count", "title": "Count", "kind": "research"},
                {
                    "id": "check",
                    "title": "Check",
                    "kind": "verification",
                    "depends_on": ["count"],
                    "gate": {
                        "questions": {"stopped": "Did the counting stop at ten?"},
                        "threshold": 0.8,
                        "max_attempts": 2
                    }
                }
            ]
        }
    })
}

fn start(working_dir: &std::path::Path, workflow: Value) -> Value {
    // No extensions: the chat under test needs its supervision and nothing
    // else, and the machine default set is not this test's subject.
    json!({
        "working_dir": working_dir,
        "workflow": workflow,
        "extension_overrides": []
    })
}

fn message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| value["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| body.to_string())
}

async fn chats(state: &Arc<AppState>) -> u64 {
    state.session_manager().count_all_sessions().await.unwrap()
}

async fn discard(state: &Arc<AppState>, session_id: &str) {
    state.clear_cached_agents().await;
    let _ = state.session_manager().delete_session(session_id).await;
}

const NOT_STARTED: &str = "The workflow was not started: ";

// ---------------------------------------------------------------------------
// The tests.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn start_with_a_supervised_workflow_and_proof_seeds_the_chat() {
    let (state, requests) = daemon().await;
    let dir = tempfile::tempdir().unwrap();

    let (status, body) = send(
        &state,
        "POST",
        "/agent/start",
        Some(start(
            dir.path(),
            supervised_workflow(0.7, "baseten_decision/joey-1"),
        )),
        true,
        HashMap::new(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = serde_json::from_str::<Value>(&body).unwrap()["id"]
        .as_str()
        .expect("the new chat's id")
        .to_string();

    let (status, body) = send(
        &state,
        "GET",
        &format!("/sessions/{id}/decision-settings"),
        None,
        true,
        HashMap::new(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let settings: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        settings["defaultModel"],
        json!({"provider": "baseten_decision", "model": "joey-1"}),
        "{body}"
    );
    assert_eq!(settings["monitors"][0]["id"], "reached-ten", "{body}");
    assert_eq!(settings["monitors"][0]["action"], "stop", "{body}");

    let (status, body) = send(
        &state,
        "GET",
        &format!("/sessions/{id}/subagents"),
        None,
        true,
        HashMap::new(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let subagents: Value = serde_json::from_str(&body).unwrap();
    let plan = &subagents["plan"];
    assert_eq!(plan["title"], "Count plan", "{body}");
    let phases = plan["phases"].as_array().expect("the plan's phases");
    assert_eq!(phases.len(), 2, "{body}");
    assert_eq!(phases[1]["id"], "check", "{body}");
    assert_eq!(
        phases[1]["gate"]["owner"], "workflow",
        "the workflow's gate binds the agent: {body}"
    );
    assert_eq!(
        requests.load(Ordering::SeqCst),
        0,
        "a seed asks no decision model"
    );

    discard(&state, &id).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn start_without_proof_on_a_keyed_daemon_is_403_and_leaves_no_chat() {
    let (state, _) = daemon().await;
    let dir = tempfile::tempdir().unwrap();
    let before = chats(&state).await;

    let (status, body) = send(
        &state,
        "POST",
        "/agent/start",
        Some(start(
            dir.path(),
            supervised_workflow(0.7, "baseten_decision/joey-1"),
        )),
        false,
        HashMap::new(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        message(&body).starts_with(
            "Starting a workflow that declares monitors or phase gates is the user's decision"
        ) && message(&body).contains("No chat was created."),
        "{body}"
    );
    assert_eq!(chats(&state).await, before, "no chat was made");

    // A workflow that declares neither still starts without the proof, as it
    // always has: the rule is the supervision's.
    let (status, body) = send(
        &state,
        "POST",
        "/agent/start",
        Some(start(
            dir.path(),
            json!({
                "version": "1.0.0",
                "title": "Plain",
                "description": "probe",
                "prompt": "Count to three."
            }),
        )),
        false,
        HashMap::new(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = serde_json::from_str::<Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    discard(&state, &id).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn start_with_an_invalid_block_is_400_with_the_sentence() {
    let (state, _) = daemon().await;
    let dir = tempfile::tempdir().unwrap();
    let before = chats(&state).await;

    let (status, body) = send(
        &state,
        "POST",
        "/agent/start",
        Some(start(
            dir.path(),
            supervised_workflow(1.5, "baseten_decision/joey-1"),
        )),
        true,
        HashMap::new(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let text = message(&body);
    assert!(
        text.starts_with(&format!(
            "{NOT_STARTED}The workflow's `decision_settings`: "
        )) && text.contains("threshold"),
        "{text}"
    );
    assert_eq!(chats(&state).await, before, "no chat was made");
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn start_refused_by_privacy_is_422_and_discards_the_chat() {
    let (state, requests) = daemon().await;
    let dir = tempfile::tempdir().unwrap();
    let before = chats(&state).await;

    // The new chat is bound to Versa, a private model, so its first turn
    // makes it Private: the hosted decision model may not read it. The key is
    // a placeholder; the provider is built and never sent a request.
    let mut request = start(
        dir.path(),
        supervised_workflow(0.7, "baseten_decision/joey-1"),
    );
    request["provider"] = json!("versa_azure");
    request["model"] = json!(VERSA_AZURE_DEFAULT_MODEL);
    let (status, body) = send(
        &state,
        "POST",
        "/agent/start",
        Some(request),
        true,
        HashMap::from([(
            "VERSA_AZURE_API_KEY".to_string(),
            "placeholder-never-sent".to_string(),
        )]),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let text = message(&body);
    assert!(
        text.starts_with(NOT_STARTED) && text.contains("joey-1"),
        "{text}"
    );
    assert_eq!(chats(&state).await, before, "the chat was discarded");
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    state.clear_cached_agents().await;

    // The same workflow on a local decision model starts on the same model:
    // the refusal above is the hosted model's.
    let mut request = start(
        dir.path(),
        supervised_workflow(0.7, "ollama_decision/tev1:0.8b"),
    );
    request["provider"] = json!("versa_azure");
    request["model"] = json!(VERSA_AZURE_DEFAULT_MODEL);
    let (status, body) = send(
        &state,
        "POST",
        "/agent/start",
        Some(request),
        true,
        HashMap::from([(
            "VERSA_AZURE_API_KEY".to_string(),
            "placeholder-never-sent".to_string(),
        )]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = serde_json::from_str::<Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    discard(&state, &id).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn start_with_a_plan_outside_auto_is_409() {
    let (state, _) = daemon().await;
    let dir = tempfile::tempdir().unwrap();
    let before = chats(&state).await;

    let (status, body) = send(
        &state,
        "POST",
        "/agent/start",
        Some(start(
            dir.path(),
            supervised_workflow(0.7, "baseten_decision/joey-1"),
        )),
        true,
        HashMap::from([("BIOROUTER_MODE".to_string(), "approve".to_string())]),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let text = message(&body);
    assert!(
        text.starts_with(&format!(
            "{NOT_STARTED}it declares a phased plan, which runs only in the Autonomous \
             approval mode"
        )) && text.contains("the Manual approval mode"),
        "{text}"
    );
    assert_eq!(chats(&state).await, before, "the chat was discarded");
}
