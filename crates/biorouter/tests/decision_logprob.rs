//! The logprob adapter against wiremock: Ollama's native `/api/chat` with
//! `logprobs`, an OpenAI-format endpoint with and without them, and the
//! verbalized fallback a missing field must produce instead of a made-up 1.0.
//! Offline: every request goes to a local mock server.
//!
//! The `/api/chat` response is the shape Ollama documents for 0.12.11 and later
//! (`docs.ollama.com/api/chat`): a top-level `logprobs` array of
//! `{token, logprob, bytes, top_logprobs}`.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use biorouter::decision::{
    Answer, DecisionAdapter, DecisionContext, DecisionError, DecisionKind, DecisionLogEntry,
    DecisionModel, DecisionModelEntry, DecisionModelRef, DecisionModelSource, DecisionOutcome,
    DecisionRequest, DecisionResult, DecisionService, DecisionSession, DecisionSink, DecisionUsage,
    DecisionUseGate, ProbabilitySource, Question, Threshold,
};
use biorouter::privacy::ProviderTier;
use biorouter::providers::api_client::{ApiClient, AuthMethod};
use biorouter::providers::decision::catalog::{ollama_entry, OllamaDecisionFacts};
use biorouter::providers::decision::logprob::OPENAI_CHAT_PATH;
use biorouter::providers::decision::LogprobAdapter;
use indexmap::IndexMap;
use serde_json::json;
use url::Url;
use wiremock::matchers::{body_partial_json, header, method, path};
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

struct Pinned(Arc<dyn DecisionModel>);

#[async_trait]
impl DecisionModelSource for Pinned {
    async fn list(&self) -> Vec<DecisionModelEntry> {
        Vec::new()
    }
    async fn open(&self, _: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
        Ok(self.0.clone())
    }
}

struct Allow;

#[async_trait]
impl DecisionUseGate for Allow {
    async fn check(&self, _: &DecisionSession, _: &dyn DecisionModel) -> DecisionResult<()> {
        Ok(())
    }
}

async fn decide(
    model: Arc<dyn DecisionModel>,
    questions: IndexMap<String, Question>,
) -> DecisionResult<Vec<biorouter::decision::Decision>> {
    let wanted = model.model_ref();
    DecisionService::new(Arc::new(Pinned(model)))
        .with_use_gate(Arc::new(Allow))
        .with_default_model(None)
        .decide(
            None,
            Some(&wanted),
            &DecisionRequest {
                questions,
                context: DecisionContext::Text(
                    "Document: The patient was diagnosed with MS in 2019.".into(),
                ),
            },
        )
        .await
}

fn claim(threshold: Option<Threshold>) -> IndexMap<String, Question> {
    IndexMap::from([(
        "supported".to_string(),
        Question {
            instructions: "Claim: the patient has multiple sclerosis. Is the claim supported?"
                .into(),
            kind: DecisionKind::Binary {},
            threshold,
        },
    )])
}

fn minicheck(server: &MockServer, thinking: bool) -> Arc<dyn DecisionModel> {
    let base = Url::parse(&server.uri()).unwrap();
    let facts = OllamaDecisionFacts {
        decision: false,
        thinking,
        remote: false,
        context_tokens: 32_768,
    };
    let entry = ollama_entry(
        &base,
        "bespoke-minicheck:7b",
        &facts,
        DecisionAdapter::Logprob,
    );
    Arc::new(LogprobAdapter::ollama(&base, entry, thinking).expect("an adapter"))
}

#[tokio::test]
async fn ollama_logprobs_are_summed_over_variants_and_renormalised() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .and(body_partial_json(json!({
            "model": "bespoke-minicheck:7b",
            "stream": false,
            "logprobs": true,
            "top_logprobs": 20,
            "truncate": false,
            "options": {"num_predict": 1, "temperature": 0.0, "num_ctx": 32768}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "bespoke-minicheck:7b",
            "message": {"role": "assistant", "content": "Yes"},
            "logprobs": [{"token": "Yes", "logprob": -0.05, "bytes": [89, 101, 115],
                "top_logprobs": [
                    {"token": "Yes", "logprob": -0.05, "bytes": [89, 101, 115]},
                    {"token": " yes", "logprob": -5.0, "bytes": [32, 121, 101, 115]},
                    {"token": "No", "logprob": -3.9, "bytes": [78, 111]}
                ]}],
            "done": true, "done_reason": "length",
            "prompt_eval_count": 40, "eval_count": 1
        })))
        .expect(1)
        .mount(&server)
        .await;

    let decisions = decide(minicheck(&server, false), claim(Some(Threshold::Yes(0.9))))
        .await
        .expect("a decision");
    let d = &decisions[0];
    // Realistic numbers: the alternatives sum to under 1.
    let yes = (-0.05f64).exp() + (-5.0f64).exp();
    let no = (-3.9f64).exp();
    assert!((d.probabilities["yes"] - yes / (yes + no)).abs() < 1e-9);
    let ProbabilitySource::Logprob { mass } = d.source else {
        panic!("{:?}", d.source);
    };
    assert!((mass - (yes + no)).abs() < 1e-9);
    assert!(d.calibrated, "mass {mass} is above 0.9");
    assert_eq!(d.answer, Answer::Binary { value: true });
    assert_eq!(d.passed, Some(true));
    assert_eq!(d.usage.input_tokens, Some(40));
    assert_eq!(d.tier, ProviderTier::Private);
}

#[tokio::test]
async fn think_false_is_sent_only_to_a_thinking_model() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .and(body_partial_json(json!({"think": false, "logprobs": true})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": {"role": "assistant", "content": "No"},
            "logprobs": [{"token": "No", "logprob": -0.1,
                "top_logprobs": [{"token": "No", "logprob": -0.1}, {"token": "Yes", "logprob": -2.4}]}]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let decisions = decide(minicheck(&server, true), claim(None)).await.unwrap();
    assert_eq!(decisions[0].answer, Answer::Binary { value: false });
}

#[tokio::test]
async fn ollama_without_logprobs_downgrades_to_a_verbalized_answer_with_a_reason() {
    let server = MockServer::start().await;
    // An Ollama before 0.12.11 ignores the field and answers without it.
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .and(body_partial_json(json!({"logprobs": true})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": {"role": "assistant", "content": "Yes"},
            "done": true
        })))
        .expect(1)
        .mount(&server)
        .await;
    // The fallback asks for JSON through `format`.
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .and(body_partial_json(json!({
            "format": {"type": "object", "required": ["answer", "confidence"]}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": {"role": "assistant", "content": "{\"answer\": \"yes\", \"confidence\": 0.8}"},
            "done": true
        })))
        .expect(1)
        .mount(&server)
        .await;

    let decisions = decide(minicheck(&server, false), claim(None))
        .await
        .unwrap();
    let d = &decisions[0];
    let ProbabilitySource::Verbalized { reason } = &d.source else {
        panic!("{:?}", d.source);
    };
    assert!(
        reason
            .as_deref()
            .unwrap_or_default()
            .contains("no logprobs"),
        "{reason:?}"
    );
    assert!(!d.calibrated);
    assert!(
        (d.probabilities["yes"] - 0.8).abs() < 1e-9,
        "not a made-up 1.0"
    );
}

/// Every row the service hands its sink.
struct Rows(Arc<Mutex<Vec<DecisionLogEntry>>>);

#[async_trait]
impl DecisionSink for Rows {
    async fn record(&self, entry: DecisionLogEntry) -> anyhow::Result<()> {
        self.0.lock().unwrap().push(entry);
        Ok(())
    }
}

/// One request per question, so a reply that cannot be read partway follows
/// requests already answered and billed: the error, and the failure row,
/// carry every one of them, the unreadable reply's own included.
#[tokio::test]
async fn a_malformed_reply_partway_keeps_the_usage_of_questions_already_asked() {
    let server = MockServer::start().await;
    // The first question is answered with logprobs.
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": {"role": "assistant", "content": "Yes"},
            "logprobs": [{"token": "Yes", "logprob": -0.05,
                "top_logprobs": [{"token": "Yes", "logprob": -0.05}, {"token": "No", "logprob": -3.9}]}],
            "done": true,
            "prompt_eval_count": 20, "eval_count": 1
        })))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    // Every later request comes back 2xx with no logprobs and a reply that
    // cannot be read: the second question's logprob request, then its
    // verbalized fallback. Both were billed.
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": {"role": "assistant", "content": "I cannot tell from this."},
            "done": true,
            "prompt_eval_count": 30, "eval_count": 4
        })))
        .expect(2)
        .mount(&server)
        .await;

    let model = minicheck(&server, false);
    let wanted = model.model_ref();
    let rows = Arc::new(Mutex::new(Vec::new()));
    let mut questions = claim(None);
    questions.insert(
        "stated".to_string(),
        Question {
            instructions: "Is a diagnosis year stated?".into(),
            kind: DecisionKind::Binary {},
            threshold: None,
        },
    );
    let err = DecisionService::new(Arc::new(Pinned(model)))
        .with_use_gate(Arc::new(Allow))
        .with_default_model(None)
        .with_sink(Arc::new(Rows(Arc::clone(&rows))))
        .decide(
            None,
            Some(&wanted),
            &DecisionRequest {
                questions,
                context: DecisionContext::Text(
                    "Document: The patient was diagnosed with MS in 2019.".into(),
                ),
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, DecisionError::Malformed { detail, .. } if detail.contains("`stated`")),
        "{err:?}"
    );
    let billed = DecisionUsage {
        input_tokens: Some(20 + 30 + 30),
        output_tokens: Some(1 + 4 + 4),
    };
    assert_eq!(err.billed_usage(), billed);

    let rows = rows.lock().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, DecisionOutcome::Error);
    assert_eq!(rows[0].usage, billed, "the failure row carries it");
}

fn openai_model(server: &MockServer) -> Arc<dyn DecisionModel> {
    let client = ApiClient::new(
        server.uri(),
        AuthMethod::BearerToken("synthetic-openai-key".into()),
    )
    .unwrap();
    let entry = DecisionModelEntry {
        provider: "openai_logprob".into(),
        model: "gpt-4.1-mini".into(),
        display_name: "gpt-4.1-mini".into(),
        context_tokens: 128_000,
        kinds: vec![
            biorouter::decision::types::DecisionKindTag::Binary,
            biorouter::decision::types::DecisionKindTag::Choice,
        ],
        tier: ProviderTier::Public,
        affiliation: None,
        adapter: DecisionAdapter::Logprob,
        local: false,
        load_refusal: None,
    };
    Arc::new(LogprobAdapter::openai_compatible(
        client,
        OPENAI_CHAT_PATH,
        entry,
    ))
}

fn course() -> IndexMap<String, Question> {
    IndexMap::from([(
        "course".to_string(),
        Question {
            instructions: "Which MS course is described?".into(),
            kind: DecisionKind::Choice {
                options: IndexMap::from([
                    ("rrms".to_string(), Some("Relapsing-remitting".to_string())),
                    (
                        "spms".to_string(),
                        Some("Secondary progressive".to_string()),
                    ),
                    ("none".to_string(), Some("Not stated".to_string())),
                ]),
            },
            threshold: None,
        },
    )])
}

#[tokio::test]
async fn openai_logprobs_map_letters_to_options() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer synthetic-openai-key"))
        .and(body_partial_json(json!({
            "model": "gpt-4.1-mini", "max_tokens": 1, "temperature": 0,
            "logprobs": true, "top_logprobs": 20
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "A"},
                "logprobs": {"content": [{"token": "A", "logprob": -0.4,
                    "top_logprobs": [
                        {"token": "A", "logprob": -0.4},
                        {"token": " A", "logprob": -3.0},
                        {"token": "C", "logprob": -2.5},
                        {"token": "The", "logprob": -2.2}
                    ]}]},
                "finish_reason": "length"}],
            "usage": {"prompt_tokens": 90, "completion_tokens": 1}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let decisions = decide(openai_model(&server), course()).await.unwrap();
    let d = &decisions[0];
    assert_eq!(
        d.answer,
        Answer::Choice {
            option: "rrms".into()
        }
    );
    assert_eq!(d.probabilities["spms"], 0.0);
    let ProbabilitySource::Logprob { mass } = d.source else {
        panic!("{:?}", d.source);
    };
    let expected = (-0.4f64).exp() + (-3.0f64).exp() + (-2.5f64).exp();
    assert!((mass - expected).abs() < 1e-9);
    assert!(!d.calibrated, "mass {mass} is under 0.9, so not calibrated");
}

#[tokio::test]
async fn an_openai_backend_that_drops_logprobs_downgrades_never_to_one() {
    let server = MockServer::start().await;
    // OpenRouter, xAI and DeepSeek ignore the field and answer without it.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(
            json!({"logprobs": true, "max_tokens": 1}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "A"},
                         "logprobs": null, "finish_reason": "length"}]
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(json!({"max_tokens": 160})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{"index": 0, "message": {"role": "assistant",
                "content": "```json\n{\"answer\": \"spms\", \"confidence\": 0.7}\n```"},
                "finish_reason": "stop"}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let decisions = decide(openai_model(&server), course()).await.unwrap();
    let d = &decisions[0];
    assert_eq!(
        d.answer,
        Answer::Choice {
            option: "spms".into()
        }
    );
    assert!(!d.calibrated);
    assert!(
        matches!(&d.source, ProbabilitySource::Verbalized { reason: Some(r) } if r.contains("no logprobs"))
    );
    assert!(d.probabilities.values().all(|p| *p < 1.0));
}

#[tokio::test]
async fn a_backend_that_refuses_logprobs_downgrades_and_one_that_fails_errors() {
    let refusing = MockServer::start().await;
    // Groq answers a logprobs request with 400.
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"logprobs": true})))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "unsupported"})))
        .expect(1)
        .mount(&refusing)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"max_tokens": 160})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{"message": {"content": "{\"answer\": \"none\", \"confidence\": 0.9}"}}]
        })))
        .expect(1)
        .mount(&refusing)
        .await;
    let decisions = decide(openai_model(&refusing), course()).await.unwrap();
    assert!(
        matches!(&decisions[0].source, ProbabilitySource::Verbalized { reason: Some(r) } if r.contains("HTTP 400"))
    );

    let failing = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&failing)
        .await;
    let err = decide(openai_model(&failing), course()).await.unwrap_err();
    assert!(
        matches!(
            err,
            DecisionError::Backend {
                status: Some(503),
                ..
            }
        ),
        "{err:?}"
    );
}
