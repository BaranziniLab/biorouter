//! System One through the decision service, against wiremock, in both server
//! shapes: Baseten (bearer key, `inference.baseten.co`) and Ollama 0.35+ (no
//! key, `keep_alive`, loopback and so Private). Offline: every request goes to
//! a local mock server.
//!
//! The response bodies are the documented ones: Baseten's System One reference
//! (`docs.baseten.co/reference/inference-api/system-one`) and the shapes the
//! Ollama `nimble` / `tev1` library pages show. PR #469's
//! `tests/baseten_decisions.rs` is where these tests started.

use std::sync::Arc;

use async_trait::async_trait;
use biorouter::decision::{
    Answer, DecisionContext, DecisionError, DecisionKind, DecisionModel, DecisionModelEntry,
    DecisionModelRef, DecisionModelSource, DecisionRequest, DecisionResult, DecisionService,
    DecisionSession, DecisionUseGate, ProbabilitySource, Question, Threshold,
};
use biorouter::privacy::{ProviderTier, SessionClassification};
use biorouter::providers::decision::catalog::{
    baseten_entry, discover_ollama, ollama_entry, ollama_facts, OllamaDecisionFacts,
};
use biorouter::providers::decision::SystemOneAdapter;
use indexmap::IndexMap;
use serde_json::{json, Value};
use url::Url;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

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

/// Opens exactly the adapters it was given.
struct Pinned(Vec<Arc<dyn DecisionModel>>);

#[async_trait]
impl DecisionModelSource for Pinned {
    async fn list(&self) -> Vec<DecisionModelEntry> {
        Vec::new()
    }
    async fn open(&self, wanted: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
        self.0
            .iter()
            .find(|m| {
                m.model_ref().provider == wanted.provider && m.model_ref().model == wanted.model
            })
            .cloned()
            .ok_or_else(|| DecisionError::NoModel("not pinned".into()))
    }
}

struct Allow;

#[async_trait]
impl DecisionUseGate for Allow {
    async fn check(&self, _: &DecisionSession, _: &dyn DecisionModel) -> DecisionResult<()> {
        Ok(())
    }
}

fn service(model: Arc<dyn DecisionModel>) -> DecisionService {
    DecisionService::new(Arc::new(Pinned(vec![model])))
        .with_use_gate(Arc::new(Allow))
        .with_default_model(None)
}

fn baseten(server: &MockServer) -> Arc<dyn DecisionModel> {
    Arc::new(
        SystemOneAdapter::baseten(
            &server.uri(),
            "synthetic-test-key".into(),
            baseten_entry(JOEY, 16_384),
        )
        .expect("a Baseten adapter"),
    )
}

fn questions() -> IndexMap<String, Question> {
    IndexMap::from([
        (
            "has_ms".to_string(),
            Question {
                instructions: "Does this note state the patient has multiple sclerosis?".into(),
                kind: DecisionKind::Binary {},
                threshold: Some(Threshold::Yes(0.9)),
            },
        ),
        (
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
                        ("ppms".to_string(), Some("Primary progressive".to_string())),
                        ("none".to_string(), Some("Not stated".to_string())),
                    ]),
                },
                threshold: Some(Threshold::Option {
                    name: "rrms".into(),
                    p: 0.8,
                }),
            },
        ),
        (
            "rating".to_string(),
            Question {
                instructions: "How well documented is the exam?".into(),
                kind: DecisionKind::Score {
                    levels: vec!["Poor".into(), "Fair".into(), "Good".into()],
                },
                threshold: Some(Threshold::AtLeastLevel { level: 2, p: 0.5 }),
            },
        ),
    ])
}

const NOTE: &str = "Neuro f/u. 34F, RRMS dx 2019 on ocrelizumab. Exam stable, EDSS 2.0.";

fn ms_answers() -> Value {
    json!({
        "has_ms": {"type": "noul", "noul": 0.98},
        "course": {"type": "choice", "choice": "rrms",
                   "probabilities": {"rrms": 0.93, "spms": 0.03, "ppms": 0.02, "none": 0.02},
                   "confidence": 0.9},
        "rating": {"type": "score", "score": 1.25,
                   "probabilities": {"0": 0.25, "1": 0.25, "2": 0.5},
                   "legend": {"0": "Poor", "1": "Fair", "2": "Good"}, "confidence": 0.1}
    })
}

#[tokio::test]
async fn baseten_gets_every_question_in_one_request_and_the_whole_distribution_comes_back() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(header("authorization", "Bearer synthetic-test-key"))
        .and(body_partial_json(json!({
            "model": JOEY,
            "state": NOTE,
            "questions": {
                "has_ms": {"type": "noul", "instructions": "Does this note state the patient has multiple sclerosis?"},
                "course": {"type": "choice", "criteria": {"rrms": "Relapsing-remitting", "none": "Not stated"}},
                "rating": {"type": "score", "criteria": ["Poor", "Fair", "Good"]}
            }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "backend-id-may-differ",
            "answers": ms_answers(),
            "usage": {"input_tokens": 180, "output_tokens": 3}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let svc = service(baseten(&server));
    let decisions = svc
        .decide(
            None,
            Some(&DecisionModelRef::new("baseten_decision", JOEY)),
            &DecisionRequest {
                questions: questions(),
                context: DecisionContext::Text(NOTE.into()),
            },
        )
        .await
        .expect("a decision");
    assert_eq!(decisions.len(), 3);

    let has_ms = &decisions[0];
    assert_eq!(has_ms.answer, Answer::Binary { value: true });
    assert!((has_ms.probabilities["yes"] - 0.98).abs() < 1e-9);
    // Binary confidence is BioRouter's |2p - 1|; System One sends none.
    assert!((has_ms.confidence - 0.96).abs() < 1e-9);
    assert_eq!(has_ms.passed, Some(true));
    assert_eq!(has_ms.source, ProbabilitySource::Native);
    assert!(has_ms.calibrated);
    assert_eq!(has_ms.tier, ProviderTier::Public);

    let course = &decisions[1];
    assert_eq!(
        course.answer,
        Answer::Choice {
            option: "rrms".into()
        }
    );
    // (0.93 - 1/4) / (3/4), computed here; the native 0.9 stays in raw.
    assert!((course.confidence - (0.93 - 0.25) / 0.75).abs() < 1e-9);
    assert_eq!(course.raw["confidence"], 0.9);
    assert_eq!(course.passed, Some(true));

    let rating = &decisions[2];
    let Answer::Score {
        level,
        label,
        expected,
    } = &rating.answer
    else {
        panic!("a score answer");
    };
    assert_eq!((*level, label.as_str()), (2, "Good"));
    assert!((expected - 1.25).abs() < 1e-9);
    // P(level >= 2) = 0.5 meets p = 0.5.
    assert_eq!(rating.passed, Some(true));
    assert_eq!(rating.pass_probability, Some(0.5));

    // One request, so every decision shares its id, usage and latency.
    assert!(decisions.iter().all(|d| d.request_id == has_ms.request_id));
    assert_eq!(has_ms.usage.input_tokens, Some(180));
    assert_eq!(has_ms.usage.output_tokens, Some(3));
}

#[tokio::test]
async fn ollama_needs_no_key_keeps_the_model_loaded_and_is_private_on_loopback() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(body_partial_json(json!({"model": "tev1:0.8b", "keep_alive": "30m"})))
        .respond_with(move |request: &Request| {
            assert!(
                request.headers.get("authorization").is_none(),
                "Ollama is sent no credential"
            );
            ResponseTemplate::new(200).set_body_json(json!({
                "model": "tev1:0.8b",
                "answers": {
                    "has_ms": {"type": "noul", "noul": 0.98},
                    "edss_stated": {"type": "noul", "noul": 0.95},
                    "course": {"type": "choice", "choice": "rrms",
                               "probabilities": {"rrms": 0.93, "spms": 0.03, "ppms": 0.02, "none": 0.02},
                               "confidence": 0.9}
                },
                "usage": {"input_tokens": 180, "output_tokens": 3}
            }))
        })
        .expect(1)
        .mount(&server)
        .await;

    let base = Url::parse(&server.uri()).unwrap();
    let facts = OllamaDecisionFacts {
        decision: true,
        thinking: false,
        remote: false,
        context_tokens: 8_192,
    };
    let entry = ollama_entry(
        &base,
        "tev1:0.8b",
        &facts,
        biorouter::decision::DecisionAdapter::SystemOne,
    );
    assert_eq!(
        entry.tier,
        ProviderTier::Private,
        "a loopback Ollama is Private"
    );
    let model: Arc<dyn DecisionModel> =
        Arc::new(SystemOneAdapter::ollama(&base, entry).expect("an Ollama adapter"));

    let mut asked = questions();
    asked.shift_remove("rating");
    asked.insert(
        "edss_stated".into(),
        Question {
            instructions: "Is an EDSS score stated?".into(),
            kind: DecisionKind::Binary {},
            threshold: Some(Threshold::Yes(0.8)),
        },
    );
    // A private chat may use a private decision model.
    let chat = DecisionSession::new("s1", SessionClassification::Private, true);
    let decisions = service(model)
        .decide(
            Some(&chat),
            Some(&DecisionModelRef::new("ollama_decision", "tev1:0.8b")),
            &DecisionRequest {
                questions: asked,
                context: DecisionContext::Text(NOTE.into()),
            },
        )
        .await
        .expect("a decision");
    assert_eq!(
        decisions
            .iter()
            .map(|d| d.question.as_str())
            .collect::<Vec<_>>(),
        ["has_ms", "course", "edss_stated"]
    );
    assert!(decisions.iter().all(|d| d.passed == Some(true)));
    assert!(decisions.iter().all(|d| d.tier == ProviderTier::Private));
}

#[tokio::test]
async fn scalar_text_is_sent_wrapped() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(body_partial_json(json!({"state": {"text": "42"}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "answers": {"has_ms": {"type": "noul", "noul": 0.1}}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let mut asked = questions();
    asked.truncate(1);
    let decisions = service(baseten(&server))
        .decide(
            None,
            Some(&DecisionModelRef::new("baseten_decision", JOEY)),
            &DecisionRequest {
                questions: asked,
                context: DecisionContext::Text("42".into()),
            },
        )
        .await
        .expect("a decision");
    assert_eq!(decisions[0].answer, Answer::Binary { value: false });
    assert_eq!(decisions[0].passed, Some(false));
}

#[tokio::test]
async fn an_answer_of_the_wrong_type_is_refused_not_guessed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "answers": {"has_ms": {"type": "choice", "choice": "continue"}},
            "usage": {"input_tokens": 42, "output_tokens": 1}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let mut asked = questions();
    asked.truncate(1);
    let err = service(baseten(&server))
        .decide(
            None,
            Some(&DecisionModelRef::new("baseten_decision", JOEY)),
            &DecisionRequest {
                questions: asked,
                context: DecisionContext::Text(NOTE.into()),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DecisionError::Malformed { .. }), "{err:?}");
    assert!(err.to_string().contains("`noul`"), "{err}");
}

#[tokio::test]
async fn an_error_body_is_never_echoed_but_its_error_type_is_kept() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "error": "SYNTHETIC_SENSITIVE_STATE_SENTINEL",
            "error_type": "invalid_request_error"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let mut asked = questions();
    asked.truncate(1);
    let err = service(baseten(&server))
        .decide(
            None,
            Some(&DecisionModelRef::new("baseten_decision", JOEY)),
            &DecisionRequest {
                questions: asked,
                context: DecisionContext::Text("SYNTHETIC_SENSITIVE_STATE_SENTINEL".into()),
            },
        )
        .await
        .unwrap_err();
    let DecisionError::Backend {
        status, error_type, ..
    } = &err
    else {
        panic!("{err:?}");
    };
    assert_eq!(*status, Some(422));
    assert_eq!(error_type.as_deref(), Some("invalid_request_error"));
    assert!(err.to_string().contains("422"), "{err}");
    assert!(
        !err.to_string()
            .contains("SYNTHETIC_SENSITIVE_STATE_SENTINEL"),
        "{err}"
    );
}

#[tokio::test]
async fn a_rate_limit_is_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "answers": {"has_ms": {"type": "noul", "noul": 0.93}}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let mut asked = questions();
    asked.truncate(1);
    let decisions = service(baseten(&server))
        .decide(
            None,
            Some(&DecisionModelRef::new("baseten_decision", JOEY)),
            &DecisionRequest {
                questions: asked,
                context: DecisionContext::Text(NOTE.into()),
            },
        )
        .await
        .expect("the retry answers");
    assert_eq!(decisions[0].passed, Some(true));
}

#[tokio::test]
async fn an_oversized_chunk_is_named_and_nothing_is_sent_for_it() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "answers": {"has_ms": {"type": "noul", "noul": 0.93}}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let mut asked = questions();
    asked.truncate(1);
    let outcomes = service(baseten(&server))
        .decide_many(
            None,
            Some(&DecisionModelRef::new("baseten_decision", JOEY)),
            &asked,
            vec![
                biorouter::decision::DecisionChunk {
                    id: "note-0001".into(),
                    context: DecisionContext::Text(NOTE.into()),
                },
                biorouter::decision::DecisionChunk {
                    id: "note-0002".into(),
                    context: DecisionContext::Text("word ".repeat(20_000)),
                },
            ],
        )
        .await
        .expect("a batch");
    assert!(outcomes[0].decisions.is_some());
    let error = outcomes[1].error.as_deref().expect("the big chunk fails");
    assert!(
        error.contains("`note-0002`") && error.contains("16384"),
        "{error}"
    );
}

#[tokio::test]
async fn ollama_discovery_lists_only_decision_models() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
            {"name": "clef-flash:latest", "model": "clef-flash:latest", "digest": "a"},
            {"name": "qwen3.8:27b", "model": "qwen3.8:27b", "digest": "b"}
        ]})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/show"))
        .and(body_partial_json(json!({"model": "clef-flash:latest"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "capabilities": ["decision", "vision"],
            "parameters": "num_ctx                        16384",
            "model_info": {"general.architecture": "qwen35", "qwen35.context_length": 262144}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/show"))
        .and(body_partial_json(json!({"model": "qwen3.8:27b"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "capabilities": ["completion", "vision", "tools", "thinking"]
        })))
        .mount(&server)
        .await;

    let base = Url::parse(&server.uri()).unwrap();
    let entries = discover_ollama(&base).await;
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].model, "clef-flash:latest");
    assert_eq!(entries[0].context_tokens, 16_384);
    assert_eq!(entries[0].tier, ProviderTier::Private);
    assert!(entries[0].local);

    let facts = ollama_facts(&base, "qwen3.8:27b").await.unwrap().unwrap();
    assert!(!facts.decision && facts.thinking);
    assert!(ollama_facts(&base, "missing:latest")
        .await
        .unwrap()
        .is_none());
}
