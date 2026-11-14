//! Gate H on decision models: a private chat's content never reaches a Public
//! decision model. The check runs before anything is sent, so the mock server
//! in front of the public model must record **zero** requests. Offline.
//!
//! The session is a real one in an ephemeral store, raised to Private the way
//! a turn raises it, and read through `DecisionSession::load`, so the tier the
//! gate sees is the stored one. The Crew check is the production one
//! ([`biorouter::decision::service::CrewUseGate`]) against this binary's
//! sandboxed, empty Crew registry.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::sync::Arc;

use async_trait::async_trait;
use biorouter::decision::{
    DecisionChunk, DecisionContext, DecisionError, DecisionKind, DecisionModel, DecisionModelEntry,
    DecisionModelRef, DecisionModelSource, DecisionRequest, DecisionResult, DecisionService,
    DecisionSession, Question, Threshold,
};
use biorouter::privacy::SessionClassification;
use biorouter::providers::decision::catalog::{baseten_entry, BASETEN_DECISION_PROVIDER};
use biorouter::providers::decision::SystemOneAdapter;
use biorouter::session::{SessionManager, SessionType};
use indexmap::IndexMap;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[ctor::ctor]
fn sandbox() {
    let root = tempfile::TempDir::new().expect("a sandbox");
    for sub in ["config", "data", "state"] {
        std::fs::create_dir_all(root.path().join(sub)).expect("a sandbox tree");
    }
    std::fs::write(root.path().join("config/config.yaml"), "{}\n").expect("a config file");
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
    static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let _ = ROOT.set(root);
}

const JOEY: &str = "decision-models/baseten-joey-2b-latest";

/// Lists and opens one Baseten adapter pointed at the mock server.
struct OnlyBaseten {
    uri: String,
}

impl OnlyBaseten {
    fn adapter(&self) -> Arc<dyn DecisionModel> {
        Arc::new(
            SystemOneAdapter::baseten(
                &self.uri,
                "synthetic-test-key".into(),
                baseten_entry(JOEY, 16_384),
            )
            .expect("an adapter"),
        )
    }
}

#[async_trait]
impl DecisionModelSource for OnlyBaseten {
    async fn list(&self) -> Vec<DecisionModelEntry> {
        vec![baseten_entry(JOEY, 16_384)]
    }
    async fn open(&self, _: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
        Ok(self.adapter())
    }
}

async fn public_model_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "answers": {"has_ms": {"type": "noul", "noul": 0.97}},
            "usage": {"input_tokens": 42, "output_tokens": 1}
        })))
        .mount(&server)
        .await;
    server
}

fn request() -> DecisionRequest {
    DecisionRequest {
        questions: questions(),
        context: DecisionContext::Text("SYNTHETIC PRIVATE NOTE: 34F, RRMS dx 2019.".into()),
    }
}

fn questions() -> IndexMap<String, Question> {
    IndexMap::from([(
        "has_ms".to_string(),
        Question {
            instructions: "Does this note state the patient has multiple sclerosis?".into(),
            kind: DecisionKind::Binary {},
            threshold: Some(Threshold::Yes(0.9)),
        },
    )])
}

async fn chat(
    classification: SessionClassification,
) -> (Arc<SessionManager>, String, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().unwrap();
    let manager = Arc::new(SessionManager::new_ephemeral(dir.path().into()));
    let session = manager
        .create_session(
            dir.path().into(),
            "Synthetic decision test".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    if classification == SessionClassification::Private {
        manager
            .update(&session.id)
            .raise_privacy(SessionClassification::Private, "fixture")
            .apply()
            .await
            .unwrap();
    }
    (manager, session.id, dir)
}

fn service(server: &MockServer) -> DecisionService {
    DecisionService::new(Arc::new(OnlyBaseten { uri: server.uri() })).with_default_model(None)
}

#[tokio::test]
async fn a_private_chat_with_a_public_decision_model_makes_zero_requests() {
    let server = public_model_server().await;
    let (manager, id, _dir) = chat(SessionClassification::Private).await;
    let session = DecisionSession::load(manager, &id, true).await.unwrap();
    assert_eq!(session.classification, SessionClassification::Private);
    let svc = service(&server);

    // Named explicitly.
    let named = DecisionModelRef::new(BASETEN_DECISION_PROVIDER, JOEY);
    let err = svc
        .decide(Some(&session), Some(&named), &request())
        .await
        .unwrap_err();
    assert!(matches!(err, DecisionError::Refused(_)), "{err:?}");
    let text = err.to_string();
    assert!(
        text.contains("private") && text.contains("joey-1"),
        "{text}"
    );
    assert!(
        text.contains("Name a private decision model (decision_models lists them)"),
        "the refusal names the fix for a model the caller named: {text}"
    );
    assert!(
        !text.contains("BIOROUTER_DECISION_MODEL"),
        "setting the default would not change a named model: {text}"
    );

    // Taken from the configured default: the refusal names that setting.
    let err = service(&server)
        .with_default_model(Some(named.clone()))
        .decide(Some(&session), None, &request())
        .await
        .unwrap_err();
    assert!(matches!(err, DecisionError::Refused(_)), "{err:?}");
    assert!(
        err.to_string()
            .contains("Set BIOROUTER_DECISION_MODEL to a private model"),
        "{err}"
    );

    // A batch: refused as a whole, before any chunk is sent.
    let batch = svc
        .decide_many(
            Some(&session),
            Some(&named),
            &questions(),
            vec![
                DecisionChunk {
                    id: "note-0001".into(),
                    context: DecisionContext::Text("SYNTHETIC PRIVATE NOTE 1".into()),
                },
                DecisionChunk {
                    id: "note-0002".into(),
                    context: DecisionContext::Text("SYNTHETIC PRIVATE NOTE 2".into()),
                },
            ],
        )
        .await;
    assert!(matches!(batch, Err(DecisionError::Refused(_))), "{batch:?}");

    // Chosen automatically: the only model is public, so privacy rules out
    // every listed model, which is a refusal, as naming it is.
    let err = svc
        .decide(Some(&session), None, &request())
        .await
        .unwrap_err();
    assert!(matches!(err, DecisionError::Refused(_)), "{err:?}");
    assert!(err.to_string().contains("private chat"), "{err}");

    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "a private chat's content reached a public decision model"
    );
}

#[tokio::test]
async fn a_chat_raised_after_it_was_loaded_is_refused_at_the_next_request() {
    let server = public_model_server().await;
    let (manager, id, _dir) = chat(SessionClassification::Public).await;
    let session = DecisionSession::load(manager.clone(), &id, true)
        .await
        .unwrap();
    assert_eq!(session.classification, SessionClassification::Public);
    manager
        .update(&id)
        .raise_privacy(SessionClassification::Private, "a private tool ran")
        .apply()
        .await
        .unwrap();
    let err = service(&server)
        .decide(
            Some(&session),
            Some(&DecisionModelRef::new(BASETEN_DECISION_PROVIDER, JOEY)),
            &request(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DecisionError::Refused(_)), "{err:?}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_public_chat_reaches_the_public_model_through_the_crew_check() {
    let server = public_model_server().await;
    let (manager, id, _dir) = chat(SessionClassification::Public).await;
    let session = DecisionSession::load(manager, &id, true).await.unwrap();
    let decisions = service(&server)
        .decide(Some(&session), None, &request())
        .await
        .expect("a public chat may use Baseten");
    assert_eq!(decisions[0].passed, Some(true));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn with_the_master_switch_off_as_sampled_nothing_is_refused() {
    let server = public_model_server().await;
    let (manager, id, _dir) = chat(SessionClassification::Private).await;
    // The caller sampled the switch OFF for this call (`cap.enforced()`).
    let session = DecisionSession::load(manager, &id, false).await.unwrap();
    assert!(service(&server)
        .decide(
            Some(&session),
            Some(&DecisionModelRef::new(BASETEN_DECISION_PROVIDER, JOEY)),
            &request(),
        )
        .await
        .is_ok());
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
