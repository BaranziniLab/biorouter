//! Item 8 at `POST /agent/start`, on a daemon that holds NO user-action key
//! (`biorouter serve`, or a hand-run `biorouterd`).
//!
//! There nothing can produce the proof a keyed daemon asks a supervised start
//! for, so the rule is the one the decision-settings writes keep (SD-11): a
//! `serve` host starts a workflow that declares monitors or a phase plan, and
//! seeds them. A desktop daemon that was meant to receive a key and did not
//! keeps the refusal, because that is a fault a restart repairs.
//!
//! ⚠ Its own binary on purpose: the installed digest is a process global
//! (`OnceLock`), so a binary that installs a key can never observe a keyless
//! daemon. Nothing here installs one. The keyed half is
//! `workflow_supervision_start.rs`.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can open the developer's real `sessions.db`.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use biorouter::decision::model::DecisionCaps;
use biorouter::decision::types::{DecisionKindTag, ModelVerdicts};
use biorouter::decision::{
    DecisionAdapter, DecisionError, DecisionModel, DecisionModelEntry, DecisionModelRef,
    DecisionModelSource, DecisionRequest, DecisionResult, DecisionService, DecisionSession,
    DecisionUseGate,
};
use biorouter::privacy::{ModelAffiliation, ProviderTier};
use biorouter_server::auth::{user_action_proof, UserActionProof};
use biorouter_server::routes::decisions::DECISION_SETTINGS_KEY_MISSING;
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use tower::ServiceExt;

fn assert_the_daemon_is_keyless() {
    assert_eq!(
        user_action_proof(&HeaderMap::new()),
        UserActionProof::NoKeyInstalled,
        "something in this binary installed a user-action digest, so these tests would be \
         measuring a desktop daemon rather than a `biorouter serve` one"
    );
}

/// The one decision model: local, so a public chat may use it.
fn tev1() -> DecisionModelEntry {
    DecisionModelEntry {
        provider: "ollama_decision".to_string(),
        model: "tev1:0.8b".to_string(),
        display_name: "tev1:0.8b".to_string(),
        context_tokens: 16_384,
        kinds: vec![DecisionKindTag::Binary],
        tier: ProviderTier::Private,
        affiliation: Some(ModelAffiliation::Local),
        adapter: DecisionAdapter::SystemOne,
        local: true,
        load_refusal: None,
    }
}

struct FakeModel(Arc<AtomicUsize>);

#[async_trait]
impl DecisionModel for FakeModel {
    fn model_ref(&self) -> DecisionModelRef {
        tev1().model_ref()
    }
    fn display_name(&self) -> String {
        "tev1:0.8b".to_string()
    }
    fn tier(&self) -> ProviderTier {
        ProviderTier::Private
    }
    fn affiliation(&self) -> Option<ModelAffiliation> {
        Some(ModelAffiliation::Local)
    }
    fn caps(&self) -> DecisionCaps {
        DecisionCaps {
            kinds: vec![DecisionKindTag::Binary],
            max_questions: 8,
            context_tokens: 16_384,
            max_concurrency: 1,
            max_request_bytes: None,
            local: true,
        }
    }
    async fn decide(&self, _request: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(DecisionError::Unsupported(
            "a seed must never ask a decision model".to_string(),
        ))
    }
}

struct FakeSource(Arc<AtomicUsize>);

#[async_trait]
impl DecisionModelSource for FakeSource {
    async fn list(&self) -> Vec<DecisionModelEntry> {
        vec![tev1()]
    }
    async fn open(&self, wanted: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
        if tev1().answers_to(wanted) {
            Ok(Arc::new(FakeModel(self.0.clone())))
        } else {
            Err(DecisionError::NoModel(format!(
                "no `{}` here",
                wanted.label()
            )))
        }
    }
}

struct Allow;

#[async_trait]
impl DecisionUseGate for Allow {
    async fn check(&self, _: &DecisionSession, _: &dyn DecisionModel) -> DecisionResult<()> {
        Ok(())
    }
}

async fn daemon(launcher_promised_a_key: bool) -> (Arc<AppState>, Arc<AtomicUsize>) {
    assert_the_daemon_is_keyless();
    biorouter_server::launch::record_launch_state(launcher_promised_a_key);
    let requests = Arc::new(AtomicUsize::new(0));
    let service = DecisionService::new(Arc::new(FakeSource(requests.clone())))
        .with_use_gate(Arc::new(Allow))
        .with_default_model(None);
    let state = AppState::new().await.unwrap();
    state.set_decision_service(Box::leak(Box::new(service)));
    (state, requests)
}

/// One request with the headers a browser tab on a `serve` daemon sends: its
/// shim answers the user-action key with an empty string.
async fn send(
    state: &Arc<AppState>,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, String) {
    let app = biorouter_server::routes::configure(state.clone(), "keyless-secret".to_string());
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-User-Action", "");
    let body = match body {
        Some(json) => {
            request = request.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&json).unwrap())
        }
        None => Body::empty(),
    };
    let response = app.oneshot(request.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn start(working_dir: &std::path::Path) -> Value {
    json!({
        "working_dir": working_dir,
        "extension_overrides": [],
        "workflow": {
            "version": "1.0.0",
            "title": "Count to thirty",
            "description": "probe",
            "prompt": "Using the shell, echo the numbers 1 to 30, one echo command per tool call.",
            "decision_settings": {
                "defaultModel": {"provider": "ollama_decision", "model": "tev1:0.8b"},
                "monitors": [{
                    "id": "reached-ten",
                    "name": "Reached ten",
                    "question": {
                        "instructions": "Has the agent already echoed 10 or a larger number?",
                        "kind": {"binary": {}}
                    },
                    "threshold": 0.7,
                    "action": "stop",
                    "onError": "pause"
                }]
            },
            "orchestration_plan": {
                "title": "Count plan",
                "phases": [{"id": "count", "title": "Count", "kind": "research"}]
            }
        }
    })
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_keyless_daemon_starts_a_supervised_workflow() {
    let (state, requests) = daemon(false).await;
    let dir = tempfile::tempdir().unwrap();

    let (status, body) = send(&state, "POST", "/agent/start", Some(start(dir.path()))).await;
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
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let settings: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(settings["monitors"][0]["id"], "reached-ten", "{body}");
    let (status, body) = send(&state, "GET", &format!("/sessions/{id}/subagents"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let subagents: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(subagents["plan"]["title"], "Count plan", "{body}");
    assert_eq!(requests.load(Ordering::SeqCst), 0);

    state.clear_cached_agents().await;
    let _ = state.session_manager().delete_session(&id).await;
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_desktop_daemon_that_lost_its_key_refuses_a_supervised_workflow() {
    // The desktop launcher declared it would hand this daemon a key.
    let (state, _) = daemon(true).await;
    let dir = tempfile::tempdir().unwrap();
    let before = state.session_manager().count_all_sessions().await.unwrap();

    let (status, body) = send(&state, "POST", "/agent/start", Some(start(dir.path()))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let message = serde_json::from_str::<Value>(&body).unwrap()["message"]
        .as_str()
        .map(str::to_string);
    assert_eq!(message.as_deref(), Some(DECISION_SETTINGS_KEY_MISSING));
    assert_eq!(
        state.session_manager().count_all_sessions().await.unwrap(),
        before,
        "no chat was made"
    );
}
