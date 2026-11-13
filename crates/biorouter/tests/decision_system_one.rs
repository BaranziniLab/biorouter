//! System One through the decision service, against wiremock, in both server
//! shapes: Baseten (bearer key, `inference.baseten.co`) and Ollama 0.35+ (no
//! key, `keep_alive`, loopback and so Private). Offline: every request goes to
//! a local mock server.
//!
//! The response bodies are the documented ones: Baseten's System One reference
//! (`docs.baseten.co/reference/inference-api/system-one`) and the shapes the
//! Ollama `nimble` / `tev1` library pages show. PR #469's
//! `tests/baseten_decisions.rs` is where these tests started.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use biorouter::decision::{
    Answer, DecisionContext, DecisionError, DecisionKind, DecisionLogEntry, DecisionModel,
    DecisionModelEntry, DecisionModelRef, DecisionModelSource, DecisionOutcome, DecisionRequest,
    DecisionResult, DecisionService, DecisionSession, DecisionSink, DecisionUsage, DecisionUseGate,
    ProbabilitySource, Question, Threshold,
};
use biorouter::privacy::{ProviderTier, SessionClassification};
use biorouter::providers::decision::catalog::{
    baseten_entry, discover_ollama, forget_discovery, ollama_entry, ollama_facts,
    ollama_load_refusal, OllamaDecisionFacts,
};
use biorouter::providers::decision::system_one::ollama_load_refusal_sentence;
use biorouter::providers::decision::{ConfiguredDecisionModels, SystemOneAdapter};
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
    // The configured Ollama is one that never answers (nothing listens on
    // port 1, and no test server is given a port that low), so nothing here
    // reaches a real Ollama through the configuration, and a test can read
    // what a stopped one looks like. Set before any thread starts.
    std::env::set_var("OLLAMA_HOST", STOPPED_OLLAMA);
    std::env::remove_var("BASETEN_API_KEY");
    static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let _ = ROOT.set(root);
}

/// The `OLLAMA_HOST` this binary configures: an address nothing answers on.
const STOPPED_OLLAMA: &str = "http://127.0.0.1:1";

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
    // The model answered, so the request was billed: what it reported rides
    // the error, and the sentence a person reads does not change.
    assert_eq!(
        err.billed_usage(),
        DecisionUsage {
            input_tokens: Some(42),
            output_tokens: Some(1)
        }
    );
    assert!(!err.to_string().contains("42"), "{err}");
}

#[tokio::test]
async fn a_missing_answers_object_still_reports_its_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "joey-2b-0.3",
            "usage": {"input_tokens": 42, "output_tokens": 1}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let err = baseten(&server)
        .decide(&one_question(NOTE))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, DecisionError::Malformed { detail, .. } if detail.contains("`answers`")),
        "{err:?}"
    );
    assert_eq!(
        err.billed_usage(),
        DecisionUsage {
            input_tokens: Some(42),
            output_tokens: Some(1)
        }
    );
}

/// A 2xx body that is not JSON was billed too, but says for how much in no
/// form that can be read: nothing is counted, and nothing is estimated.
#[tokio::test]
async fn a_body_that_is_not_json_bills_nothing_it_cannot_read() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"usage: {"input_tokens": 42, "output_tokens": 1}"#),
        )
        .expect(1)
        .mount(&server)
        .await;
    let err = baseten(&server)
        .decide(&one_question(NOTE))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, DecisionError::Malformed { detail, .. } if detail == "the body was not JSON"),
        "{err:?}"
    );
    assert_eq!(err.billed_usage(), DecisionUsage::default());
}

/// What a sink was handed: each row with the id it was stored under (empty
/// for an unnamed one), and each late answer, which also wakes `arrived`.
#[derive(Default)]
struct Captured {
    rows: Mutex<Vec<(String, DecisionLogEntry)>>,
    late: Mutex<Vec<(String, DecisionLogEntry)>>,
    arrived: tokio::sync::Notify,
}

struct Capturing(Arc<Captured>);

#[async_trait]
impl DecisionSink for Capturing {
    async fn record(&self, entry: DecisionLogEntry) -> anyhow::Result<()> {
        self.0.rows.lock().unwrap().push((String::new(), entry));
        Ok(())
    }
    async fn record_as(&self, id: &str, entry: DecisionLogEntry) -> anyhow::Result<()> {
        self.0.rows.lock().unwrap().push((id.to_string(), entry));
        Ok(())
    }
    async fn record_late(&self, id: &str, entry: DecisionLogEntry) -> anyhow::Result<()> {
        self.0.late.lock().unwrap().push((id.to_string(), entry));
        self.0.arrived.notify_one();
        Ok(())
    }
}

/// Ask Baseten with a 50 ms deadline for an answer that takes 400 ms, in a
/// chat with a sink, and wait (up to a generous cap) for the late answer to
/// reach it. Returns the caller's error and the late row, after checking the
/// late row names the timeout row.
async fn ask_too_late(body: Value) -> (DecisionError, DecisionLogEntry) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(body)
                .set_delay(Duration::from_millis(400)),
        )
        .expect(1)
        .mount(&server)
        .await;
    let captured = Arc::new(Captured::default());
    let svc = service(baseten(&server))
        .with_timeouts(Duration::from_millis(50), Duration::from_millis(50))
        .with_sink(Arc::new(Capturing(Arc::clone(&captured))));
    let chat = DecisionSession::new("late-chat", SessionClassification::Public, true);
    let err = svc
        .decide(
            Some(&chat),
            Some(&DecisionModelRef::new("baseten_decision", JOEY)),
            &one_question(NOTE),
        )
        .await
        .unwrap_err();
    tokio::time::timeout(Duration::from_secs(20), captured.arrived.notified())
        .await
        .expect("the late answer reaches the sink");

    let rows = captured.rows.lock().unwrap().clone();
    assert_eq!(rows.len(), 1, "only the timeout row is written in time");
    let (timeout_id, timeout_row) = &rows[0];
    assert!(!timeout_id.is_empty());
    assert_eq!(timeout_row.outcome, DecisionOutcome::Error);
    assert_eq!(timeout_row.usage, DecisionUsage::default());
    let late = captured.late.lock().unwrap().clone();
    assert_eq!(late.len(), 1, "settled once");
    let (late_id, late_row) = late.into_iter().next().unwrap();
    assert_eq!(&late_id, timeout_id, "billed beside the row it belongs to");
    assert_eq!(late_row.outcome, DecisionOutcome::Error);
    assert!(late_row.decisions.is_empty(), "a late answer is never used");
    assert!(
        late_row
            .error
            .as_deref()
            .is_some_and(|e| e.contains("too late to use")),
        "{:?}",
        late_row.error
    );
    (err, late_row)
}

#[tokio::test]
async fn a_late_baseten_answer_reaches_the_sink() {
    let (err, late) = ask_too_late(json!({
        "model": "joey-2b-0.3",
        "answers": {"has_ms": {"type": "noul", "noul": 0.97}},
        "usage": {"input_tokens": 180, "output_tokens": 3}
    }))
    .await;
    assert!(matches!(err, DecisionError::Timeout { .. }), "{err:?}");
    assert_eq!(
        late.usage,
        DecisionUsage {
            input_tokens: Some(180),
            output_tokens: Some(3)
        }
    );
    assert_eq!(late.served_model.as_deref(), Some("joey-2b-0.3"));
}

/// A late answer that cannot be read was billed all the same, and only the
/// adapter can read what it cost: without that, nothing reaches the sink.
#[tokio::test]
async fn a_late_answer_that_cannot_be_read_still_reaches_the_sink() {
    let (err, late) = ask_too_late(json!({
        "answers": {"has_ms": {"type": "choice", "choice": "continue"}},
        "usage": {"input_tokens": 42, "output_tokens": 1}
    }))
    .await;
    assert!(matches!(err, DecisionError::Timeout { .. }), "{err:?}");
    assert_eq!(
        late.usage,
        DecisionUsage {
            input_tokens: Some(42),
            output_tokens: Some(1)
        }
    );
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

/// Discovery's cache answers for one server and the copies it lists, not for
/// an address. A second server on the same address (a restarted Ollama, or a
/// mock server on a port a finished test had, which is how CI caught it) used
/// to be handed the first one's models for a minute.
#[tokio::test]
#[serial_test::serial(ollama_discovery)]
async fn a_server_listing_other_copies_at_the_same_address_is_asked_again() {
    let server = MockServer::start().await;
    let decision_show = |context: u64| {
        ResponseTemplate::new(200).set_body_json(json!({
            "capabilities": ["decision"],
            "parameters": format!("num_ctx {context}"),
        }))
    };
    let probe = ResponseTemplate::new(400)
        .set_body_json(json!({"error": "questions must contain 1-64 fields"}));
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
            {"name": "same-address-first:1b", "model": "same-address-first:1b", "digest": "same-address-1"}
        ]})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/show"))
        .respond_with(decision_show(4096))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(probe.clone())
        .mount(&server)
        .await;
    let base = Url::parse(&server.uri()).unwrap();
    let first = discover_ollama(&base).await;
    assert_eq!(
        first.iter().map(|e| e.model.as_str()).collect::<Vec<_>>(),
        ["same-address-first:1b"]
    );

    // The same address now lists another copy: the next discovery sees it at
    // once, inside the cache's minute.
    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
            {"name": "same-address-second:1b", "model": "same-address-second:1b", "digest": "same-address-2"}
        ]})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/show"))
        .respond_with(decision_show(8192))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(probe)
        .mount(&server)
        .await;
    let second = discover_ollama(&base).await;
    assert_eq!(
        second.iter().map(|e| e.model.as_str()).collect::<Vec<_>>(),
        ["same-address-second:1b"]
    );
    assert_eq!(second[0].context_tokens, 8192);
}

/// Discovery keeps one answer for the whole process, so the tests that read
/// or clear it take turns (`serial(ollama_discovery)`): a discovery another
/// test runs in between would replace the answer one of them is checking.
#[tokio::test]
#[serial_test::serial(ollama_discovery)]
async fn a_model_left_out_of_a_discovery_lists_once_discovery_is_forgotten() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
            {"name": "slow-show:1b", "model": "slow-show:1b", "digest": "slow-show-1"}
        ]})))
        .mount(&server)
        .await;
    // The first `/api/show` fails, so the first discovery leaves it out.
    Mock::given(method("POST"))
        .and(path("/api/show"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/show"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"capabilities": ["decision"]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(empty_questions_refusal())
        .mount(&server)
        .await;
    let base = Url::parse(&server.uri()).unwrap();
    assert!(discover_ollama(&base).await.is_empty(), "its show failed");
    // The server lists the same copies, so the omission is answered from the
    // cache within its minute.
    assert!(discover_ollama(&base).await.is_empty(), "cached");

    // An install that verified the model forgets the discovery: it lists at
    // once.
    forget_discovery();
    let listed = discover_ollama(&base).await;
    assert_eq!(
        listed.iter().map(|e| e.model.as_str()).collect::<Vec<_>>(),
        ["slow-show:1b"]
    );
}

/// With the configured Ollama stopped, nothing local is listed, and a private
/// chat may use nothing else. The refusal says the server is not answering,
/// where advising an install would be wrong for a model already pulled.
#[tokio::test]
async fn a_private_chat_is_told_when_ollama_is_not_answering() {
    let svc = DecisionService::new(Arc::new(ConfiguredDecisionModels))
        .with_use_gate(Arc::new(Allow))
        .with_default_model(None);
    let chat = DecisionSession::new("stopped-ollama-chat", SessionClassification::Private, true);
    let err = svc
        .decide(Some(&chat), None, &one_question(NOTE))
        .await
        .unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains(&format!(
            "Ollama is not answering at {STOPPED_OLLAMA}/, so no local decision model could be \
             listed. Start Ollama, then try again."
        )),
        "{text}"
    );
    assert!(
        !text.contains(biorouter::decision::INSTALL_ADVICE),
        "{text}"
    );
}

#[tokio::test]
#[serial_test::serial(ollama_discovery)]
async fn ollama_discovery_lists_only_decision_models() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
            {"name": "clef-flash:latest", "model": "clef-flash:latest", "digest": "listing-a"},
            {"name": "qwen3.8:27b", "model": "qwen3.8:27b", "digest": "listing-b"}
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

/// Ollama 0.40.14's answer for a decision model whose copy it cannot load
/// (measured 2026-11-05 on `clef-flash:latest`, built from a GGUF before the
/// format change).
fn format_refusal() -> ResponseTemplate {
    ResponseTemplate::new(400).set_body_json(json!({
        "error": "'clef-flash:latest' was downloaded in a format that is no longer supported. \
                  To re-download, run 'ollama pull clef-flash:latest'"
    }))
}

/// Ollama's answer to the load probe for a copy it can load (measured the same
/// day on `tev1:0.8b`).
fn empty_questions_refusal() -> ResponseTemplate {
    ResponseTemplate::new(400)
        .set_body_json(json!({"error": "questions must contain 1\u{2013}64 fields"}))
}

fn decision_facts() -> OllamaDecisionFacts {
    OllamaDecisionFacts {
        decision: true,
        thinking: false,
        remote: false,
        context_tokens: 16_384,
    }
}

fn ollama(server: &MockServer, name: &str) -> SystemOneAdapter {
    let base = Url::parse(&server.uri()).unwrap();
    let entry = ollama_entry(
        &base,
        name,
        &decision_facts(),
        biorouter::decision::DecisionAdapter::SystemOne,
    );
    SystemOneAdapter::ollama(&base, entry).expect("an Ollama adapter")
}

fn one_question(context: &str) -> DecisionRequest {
    let mut asked = questions();
    asked.truncate(1);
    DecisionRequest {
        questions: asked,
        context: DecisionContext::Text(context.into()),
    }
}

/// The parts of a backend error, or a panic naming what came back instead.
fn backend(err: &DecisionError) -> (Option<u16>, Option<&str>, &str) {
    let DecisionError::Backend {
        status,
        error_type,
        message,
        ..
    } = err
    else {
        panic!("a backend error, not {err:?}");
    };
    (*status, error_type.as_deref(), message.as_str())
}

#[tokio::test]
async fn an_ollama_model_in_an_old_format_is_reported_as_such() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(format_refusal())
        .expect(2)
        .mount(&server)
        .await;
    let err = ollama(&server, "clef-flash:latest")
        .decide(&one_question(NOTE))
        .await
        .unwrap_err();
    let (status, error_type, message) = backend(&err);
    assert_eq!(status, Some(400));
    // BioRouter's own reading, spelled so it is not taken for a server token.
    assert_eq!(error_type, Some("biorouter.model_format_unsupported"));
    assert_eq!(message, ollama_load_refusal_sentence("clef-flash:latest"));
    assert!(
        message.contains("`clef-flash:latest` cannot be loaded")
            && message.contains("ollama pull clef-flash:latest")
            && message.contains("ollama rm clef-flash:latest"),
        "{message}"
    );
    assert!(!message.contains("never truncates"), "{message}");
    assert!(!message.contains("was downloaded"), "{message}");
    assert!(!err.is_transient());

    // Baseten's reading of the same answer is unchanged.
    let err = baseten(&server)
        .decide(&one_question(NOTE))
        .await
        .unwrap_err();
    let (status, error_type, message) = backend(&err);
    assert_eq!((status, error_type), (Some(400), None));
    assert!(message.contains("never truncates"), "{message}");
}

#[tokio::test]
#[serial_test::serial(ollama_discovery)]
async fn a_copy_ollama_cannot_load_is_listed_last_with_the_fix() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
            {"name": "clef-flash:latest", "model": "clef-flash:latest", "digest": "refused-a"},
            {"name": "bespoke:1b", "model": "bespoke:1b", "digest": "refused-b"},
            {"name": "tev1:0.8b", "model": "tev1:0.8b", "digest": "refused-c"}
        ]})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/show"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"capabilities": ["decision"]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(|request: &Request| {
            let body: Value = request.body_json().expect("a JSON probe");
            // The probe asks nothing and carries no state.
            assert_eq!(body["questions"], json!({}), "{body}");
            assert_eq!(body["state"], json!(""), "{body}");
            if body["model"] == "clef-flash:latest" {
                format_refusal()
            } else {
                empty_questions_refusal()
            }
        })
        .expect(3)
        .mount(&server)
        .await;

    let base = Url::parse(&server.uri()).unwrap();
    let entries = discover_ollama(&base).await;
    let order: Vec<_> = entries.iter().map(|e| e.model.as_str()).collect();
    // The shipped default first, then by name, and the refused copy last.
    assert_eq!(order, ["tev1:0.8b", "bespoke:1b", "clef-flash:latest"]);
    assert_eq!(entries[0].load_refusal, None);
    assert_eq!(entries[1].load_refusal, None);
    assert_eq!(
        entries[2].load_refusal.as_deref(),
        Some(ollama_load_refusal_sentence("clef-flash:latest").as_str())
    );
    // The served shape only gains the field when it is set.
    let served = serde_json::to_value(&entries).unwrap();
    assert!(served[0].get("loadRefusal").is_none(), "{served}");
    assert!(served[2]["loadRefusal"]
        .as_str()
        .is_some_and(|s| s.contains("ollama pull clef-flash:latest")));
    let back: Vec<DecisionModelEntry> = serde_json::from_value(served).unwrap();
    assert_eq!(
        back.iter()
            .map(|e| e.load_refusal.clone())
            .collect::<Vec<_>>(),
        entries
            .iter()
            .map(|e| e.load_refusal.clone())
            .collect::<Vec<_>>()
    );
    // A shape stored before the field existed still reads, as loadable.
    let older: DecisionModelEntry = serde_json::from_value(json!({
        "provider": "ollama_decision", "model": "tev1:0.8b", "displayName": "tev1:0.8b",
        "contextTokens": 8192, "kinds": ["binary"], "tier": "private",
        "adapter": "system_one", "local": true
    }))
    .unwrap();
    assert_eq!(older.load_refusal, None);
}

#[tokio::test]
async fn the_load_probe_is_asked_once_per_copy_and_never_remembers_an_unclear_answer() {
    let refusing = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(format_refusal())
        .expect(2)
        .mount(&refusing)
        .await;
    let base = Url::parse(&refusing.uri()).unwrap();
    let sentence = ollama_load_refusal_sentence("clef-flash:latest");
    for version in ["a", "a", "a-repulled"] {
        assert_eq!(
            ollama_load_refusal(&base, "clef-flash:latest", Some(version))
                .await
                .as_deref(),
            Some(sentence.as_str()),
            "copy {version}"
        );
    }

    // A server fault says nothing about the copy: never marked, asked again.
    let faulty = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(500))
        .expect(2)
        .mount(&faulty)
        .await;
    let base = Url::parse(&faulty.uri()).unwrap();
    for _ in 0..2 {
        assert_eq!(
            ollama_load_refusal(&base, "tev1:0.8b", Some("b")).await,
            None
        );
    }

    // Nor does a missing model or an unreachable server.
    let missing = MockServer::start().await;
    let base = Url::parse(&missing.uri()).unwrap();
    assert_eq!(
        ollama_load_refusal(&base, "gone:latest", Some("c")).await,
        None
    );
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let base = Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
    assert_eq!(
        ollama_load_refusal(&base, "tev1:0.8b", Some("b")).await,
        None
    );
}

#[tokio::test]
async fn a_refused_key_says_so_and_never_shows_the_key() {
    let server = MockServer::start().await;
    // The body inference.baseten.co sent to a request with no key
    // (POST /v1/systemone, recorded 2026-11-04).
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({"error": "Authentication required."})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let err = baseten(&server)
        .decide(&one_question(NOTE))
        .await
        .unwrap_err();
    let (status, error_type, message) = backend(&err);
    assert_eq!((status, error_type), (Some(401), None));
    assert!(
        message.contains("refused the credential (HTTP 401)"),
        "{message}"
    );
    assert!(!message.contains("synthetic-test-key"), "{message}");
    assert!(!message.contains("Authentication required"), "{message}");
    assert!(!err.is_transient());
}

#[tokio::test]
async fn a_rate_limit_that_persists_is_reported_after_the_retries() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
        // The first try and two retries.
        .expect(3)
        .mount(&server)
        .await;
    let err = baseten(&server)
        .decide(&one_question(NOTE))
        .await
        .unwrap_err();
    let (status, _, message) = backend(&err);
    assert_eq!(status, Some(429));
    assert!(message.contains("rate limited"), "{message}");
    assert!(err.is_transient());
}

#[tokio::test]
async fn a_server_error_is_reported_without_its_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(503)
                .set_body_json(json!({"error": "SYNTHETIC_SENSITIVE_STATE_SENTINEL"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let err = baseten(&server)
        .decide(&one_question("SYNTHETIC_SENSITIVE_STATE_SENTINEL"))
        .await
        .unwrap_err();
    let (status, error_type, message) = backend(&err);
    assert_eq!((status, error_type), (Some(503), None));
    assert_eq!(
        message,
        "Baseten decision model `joey-1` failed (HTTP 503)."
    );
    assert!(err.is_transient());
}

#[tokio::test]
async fn an_unreachable_server_is_named_and_may_be_retried() {
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let adapter = SystemOneAdapter::baseten(
        &format!("http://127.0.0.1:{port}"),
        "synthetic-test-key".into(),
        baseten_entry(JOEY, 16_384),
    )
    .unwrap();
    let err = adapter.decide(&one_question(NOTE)).await.unwrap_err();
    let (status, _, message) = backend(&err);
    assert_eq!(status, None);
    // Plain words, never the transport library's text (it goes to the log).
    assert_eq!(
        message, "joey-1 on Baseten could not be reached because nothing answered at its address",
        "{message}"
    );
    assert!(!message.contains("synthetic-test-key"), "{message}");
    assert!(err.is_transient());
}

#[tokio::test]
async fn an_answer_naming_an_outcome_the_question_lacks_is_refused() {
    let cases = [
        (
            "course",
            json!({"type": "choice", "choice": "rrms",
                   "probabilities": {"rrms": 0.5, "mystery": 0.5}}),
            "`mystery`",
        ),
        (
            "rating",
            json!({"type": "score", "score": 1.6,
                   "probabilities": {"0": 0.2, "7": 0.8}}),
            "`7`",
        ),
    ];
    for (question, answer, named) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"answers": {question: answer.clone()}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let mut asked = questions();
        asked.retain(|name, _| name == question);
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
        assert!(
            err.to_string().contains("unknown outcome") && err.to_string().contains(named),
            "{err}"
        );
    }
}

/// F5: a server's refusal of a prompt over its window, said its own way by
/// each server, reaches the caller as the check's over-size error, named for
/// the chunk, from the two numbers alone: the body is never echoed.
#[tokio::test]
async fn a_servers_over_window_refusal_reads_as_the_size_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": "prompt 0 has 2503 tokens; expected 1\u{2013}2048 (input is never \
                      truncated) SYNTHETIC_SENSITIVE_STATE_SENTINEL"
        })))
        .mount(&server)
        .await;
    let model: Arc<dyn DecisionModel> = Arc::new(ollama(&server, "tev1:0.8b"));
    let mut asked = questions();
    asked.truncate(1);
    let outcomes = service(model.clone())
        .decide_many(
            None,
            Some(&DecisionModelRef::new("ollama_decision", "tev1:0.8b")),
            &asked,
            vec![biorouter::decision::DecisionChunk {
                id: "note-0003".into(),
                context: DecisionContext::Text(NOTE.into()),
            }],
        )
        .await
        .expect("a batch");
    let error = outcomes[0].error.as_deref().expect("the chunk fails");
    // The server's 2,503 prompt tokens, said in the unit decision_models uses
    // (R3-F2): before the margin's factor and less the questions' share. The
    // model by its plain name, never in backticks.
    assert!(
        error.starts_with("Chunk `note-0003` is about ")
            && error.contains(
                " tokens of text tev1:0.8b takes beside these questions (its window is 2048 \
                 tokens)"
            ),
        "{error}"
    );
    assert!(!error.contains("SENTINEL"), "{error}");

    let single = service(model)
        .decide(
            None,
            Some(&DecisionModelRef::new("ollama_decision", "tev1:0.8b")),
            &one_question(NOTE),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&single, DecisionError::TooLarge { chunk, tokens, usable, limit: 2048, .. }
            if chunk == "The text" && *tokens > *usable && *usable < 1_669),
        "{single:?}"
    );
}

/// Baseten's 422 for a prompt over its 16,000 (measured 2026-11-10) reads the
/// same way.
#[tokio::test]
async fn basetens_over_window_refusal_reads_as_the_size_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "error": "OneJev prompt has 16094 tokens, exceeding max_len 16000; prompts are \
                      never truncated",
            "error_type": "Validation"
        })))
        .mount(&server)
        .await;
    let err = service(baseten(&server))
        .decide(
            None,
            Some(&DecisionModelRef::new("baseten_decision", JOEY)),
            &one_question(NOTE),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            DecisionError::TooLarge {
                tokens,
                usable,
                limit: 16000,
                ..
            } if tokens > usable && usable < 13_478
        ),
        "{err:?}"
    );
}

/// W6: Ollama's 413 for a body over 64 KiB is said in bytes, never as a
/// window.
#[tokio::test]
async fn an_ollama_body_over_its_limit_is_refused_in_bytes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(413)
                .set_body_json(json!({"error": "text and schema must not exceed 64 KiB"})),
        )
        .mount(&server)
        .await;
    let err = ollama(&server, "clef-flash:latest")
        .decide(&one_question(NOTE))
        .await
        .unwrap_err();
    let DecisionError::Invalid(text) = &err else {
        panic!("a byte refusal, not {err:?}");
    };
    assert!(
        text.contains("65536-byte request limit") && !text.contains("window"),
        "{text}"
    );
}

/// W6: the request limit is judged on the body as it is sent. A state of
/// quotes and newlines doubles when escaped, so a count of its raw bytes let
/// it through and the server refused it with 413; now nothing is sent.
#[tokio::test]
async fn an_escaped_body_over_the_limit_is_refused_before_sending() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let model: Arc<dyn DecisionModel> = Arc::new(ollama(&server, "clef-flash:latest"));
    let state = format!("A patient note.{}", "\n".repeat(33_000));
    let err = service(model)
        .decide(
            None,
            Some(&DecisionModelRef::new(
                "ollama_decision",
                "clef-flash:latest",
            )),
            &one_question(&state),
        )
        .await
        .unwrap_err();
    let DecisionError::Invalid(text) = &err else {
        panic!("a byte refusal, not {err:?}");
    };
    assert!(text.contains("over the 65536-byte request limit"), "{text}");
}

/// WIN-2: a decision model whose Modelfile states no `num_ctx` is held to the
/// window its loaded copy was given (`/api/ps`), or, not loaded, to 8,192,
/// never to its 262k trained window.
#[tokio::test]
#[serial_test::serial(ollama_discovery)]
async fn a_model_that_states_no_window_takes_the_loaded_one_or_a_capped_default() {
    forget_discovery();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
            {"name": "loaded:latest", "digest": "win2-a"},
            {"name": "idle:latest", "digest": "win2-b"}
        ]})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/show"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "capabilities": ["decision"],
            "model_info": {"general.architecture": "qwen35", "qwen35.context_length": 262144}
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/ps"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
            {"name": "loaded:latest", "model": "loaded:latest", "context_length": 4096}
        ]})))
        .mount(&server)
        .await;
    let base = Url::parse(&server.uri()).unwrap();
    let entries = discover_ollama(&base).await;
    let window = |name: &str| {
        entries
            .iter()
            .find(|e| e.model == name)
            .map(|e| e.context_tokens)
    };
    assert_eq!(window("loaded:latest"), Some(4_096));
    assert_eq!(window("idle:latest"), Some(8_192));
    let facts = ollama_facts(&base, "loaded:latest").await.unwrap().unwrap();
    assert_eq!(facts.context_tokens, 4_096);
    forget_discovery();
}
