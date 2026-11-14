//! Baseten decision models opened the way the product opens them, through
//! `ConfiguredDecisionModels`: the key from `BASETEN_API_KEY` and the host from
//! `BASETEN_DECISION_HOST`. Offline: the host is a local mock server.
//!
//! A binary of its own with one test, because the configuration is read from
//! the process environment, and setting a variable while other test threads
//! read the environment is a data race. Every variable is set in the
//! constructor, before `main` and before any client exists.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::net::TcpListener;
use std::sync::{Mutex, OnceLock};

use biorouter::decision::{
    DecisionContext, DecisionKind, DecisionModelRef, DecisionModelSource, DecisionRequest,
    Question, Threshold,
};
use biorouter::privacy::ProviderTier;
use biorouter::providers::decision::catalog::BASETEN_DECISION_CONTEXT;
use biorouter::providers::decision::ConfiguredDecisionModels;
use indexmap::IndexMap;
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "cfg-test-key";
const JOEY: &str = "decision-models/baseten-joey-2b-latest";

/// The port the mock server takes, bound before `main` so the host can be
/// exported with the rest of the configuration.
static LISTENER: OnceLock<Mutex<Option<TcpListener>>> = OnceLock::new();

#[ctor::ctor]
fn sandbox() {
    let root = tempfile::TempDir::new().expect("a sandbox");
    for sub in ["config", "data", "state"] {
        std::fs::create_dir_all(root.path().join(sub)).expect("a sandbox tree");
    }
    std::fs::write(root.path().join("config/config.yaml"), "{}\n").expect("a config file");
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let port = listener.local_addr().expect("its address").port();
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
    std::env::set_var("BASETEN_API_KEY", KEY);
    std::env::set_var("BASETEN_DECISION_HOST", format!("http://127.0.0.1:{port}"));
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    let _ = ROOT.set(root);
    let _ = LISTENER.set(Mutex::new(Some(listener)));
}

#[tokio::test]
async fn the_baseten_key_and_host_come_from_config() {
    let listener = LISTENER
        .get()
        .and_then(|slot| slot.lock().ok()?.take())
        .expect("the listener the constructor bound");
    let server = MockServer::builder().listener(listener).start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .and(body_partial_json(json!({"model": JOEY})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "answers": {"has_ms": {"type": "noul", "noul": 0.97}},
            "usage": {"input_tokens": 40, "output_tokens": 1}
        })))
        .expect(1)
        .mount(&server)
        .await;

    // The alias resolves to the slug, and the entry is Public with Baseten's window.
    let model = ConfiguredDecisionModels
        .open(&DecisionModelRef::new("baseten_decision", "joey-1"))
        .await
        .expect("a Baseten decision model");
    assert_eq!(model.model_ref().model, JOEY);
    assert_eq!(model.tier(), ProviderTier::Public);
    assert_eq!(model.caps().context_tokens, BASETEN_DECISION_CONTEXT);

    let verdicts = model
        .decide(&DecisionRequest {
            questions: IndexMap::from([(
                "has_ms".to_string(),
                Question {
                    instructions: "Does this note state the patient has multiple sclerosis?".into(),
                    kind: DecisionKind::Binary {},
                    threshold: Some(Threshold::Yes(0.9)),
                },
            )]),
            context: DecisionContext::Text("Neuro f/u. RRMS dx 2019.".into()),
        })
        .await
        .expect("the configured host answers with the configured key");
    assert_eq!(verdicts.verdicts.len(), 1);
    assert_eq!(verdicts.usage.input_tokens, Some(40));
}
