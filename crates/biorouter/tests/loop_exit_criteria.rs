use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use async_trait::async_trait;
use biorouter::agents::exit_criteria::{
    self, DecisionType, EvaluationTiming, ExitCriteria, LoopDecisionEvaluator,
};
use biorouter::conversation::{message::Message, Conversation};
use biorouter::model::ModelConfig;
use biorouter::privacy::{ProviderTier, SessionClassification};
use biorouter::providers::{
    base::{Provider, ProviderMetadata, ProviderUsage, Usage},
    errors::ProviderError,
};
use biorouter::session::{SessionManager, SessionType};
use rmcp::model::Tool;
use serde_json::json;
use tempfile::TempDir;

fn criteria(decision: DecisionType) -> ExitCriteria {
    ExitCriteria {
        timing: biorouter::agents::exit_criteria::EvaluationTiming::PostResponse,
        prompt: "Is useful progress being made?".into(),
        provider: "fixture".into(),
        model: "joey-1".into(),
        decision,
    }
}

fn boolean() -> ExitCriteria {
    criteria(DecisionType::Boolean {
        interrupt_when: false,
    })
}

#[test]
fn boolean_requires_real_boolean_and_supports_both_interrupt_values() {
    let rule = boolean();
    assert!(
        rule.interpret(r#"{"value":false,"reason":"No progress"}"#)
            .unwrap()
            .interrupt
    );
    assert!(
        !rule
            .interpret(r#"{"value":true,"reason":"Progress"}"#)
            .unwrap()
            .interrupt
    );
    assert!(rule
        .interpret(r#"{"value":"false","reason":"No progress"}"#)
        .is_err());
    let inverted = criteria(DecisionType::Boolean {
        interrupt_when: true,
    });
    assert!(
        inverted
            .interpret(r#"{"value":true,"reason":"Stop"}"#)
            .unwrap()
            .interrupt
    );
}

#[test]
fn choice_matches_exact_value_and_refuses_unknown_choices() {
    let rule = criteria(DecisionType::Choice {
        choices: vec!["continue".into(), "adjust".into()],
        interrupt_when: "adjust".into(),
    });
    assert!(
        rule.interpret(r#"{"value":"adjust","reason":"Wrong approach"}"#)
            .unwrap()
            .interrupt
    );
    assert!(
        !rule
            .interpret(r#"{"value":"continue","reason":"On track"}"#)
            .unwrap()
            .interrupt
    );
    assert!(rule
        .interpret(r#"{"value":"maybe","reason":"Unknown"}"#)
        .is_err());
    let invalid = criteria(DecisionType::Choice {
        choices: vec!["same".into(), "same".into()],
        interrupt_when: "same".into(),
    });
    assert!(invalid.validate().is_err());
}

#[test]
fn ranking_has_strict_cutoff_and_validates_range() {
    let rule = criteria(DecisionType::Ranking {
        min: 0.0,
        max: 10.0,
        interrupt_below: 5.0,
        levels: vec![],
    });
    for (value, interrupts) in [(0.0, true), (4.99, true), (5.0, false), (10.0, false)] {
        assert_eq!(
            rule.interpret(&json!({"value":value,"reason":"Evidence"}).to_string())
                .unwrap()
                .interrupt,
            interrupts
        );
    }
    for value in [json!(-1), json!(11), json!("5"), json!(null)] {
        assert!(rule
            .interpret(&json!({"value":value,"reason":"Evidence"}).to_string())
            .is_err());
    }
    for (min, max, cutoff) in [
        (1.0, 1.0, 1.0),
        (0.0, 10.0, 11.0),
        (0.0, f64::INFINITY, 1.0),
    ] {
        assert!(criteria(DecisionType::Ranking {
            min,
            max,
            levels: vec![],
            interrupt_below: cutoff
        })
        .validate()
        .is_err());
    }
}

#[test]
fn malformed_or_incomplete_verdict_is_not_a_pass() {
    for response in [
        "not json",
        "{}",
        r#"{"value":true}"#,
        r#"{"value":true,"reason":""}"#,
        r#"{"value":true,"reason":"OK","extra":true}"#,
    ] {
        assert!(boolean().interpret(response).is_err(), "{response}");
    }
    let mut invalid = boolean();
    invalid.prompt = " ".into();
    assert!(invalid.validate().is_err());
    invalid.prompt = "x".repeat(8001);
    assert!(invalid.validate().is_err());
}

async fn fixture() -> (TempDir, SessionManager, String) {
    let dir = TempDir::new().unwrap();
    let manager = SessionManager::new_ephemeral(dir.path().to_path_buf());
    let session = manager
        .create_session(
            dir.path().to_path_buf(),
            "loop criteria fixture".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    (dir, manager, session.id)
}

#[tokio::test]
async fn persistence_round_trip_clear_and_corruption_fail_closed() {
    let (_dir, manager, id) = fixture().await;
    assert!(exit_criteria::load(&manager, &id).await.unwrap().is_none());
    exit_criteria::save(&manager, &id, Some(&boolean()))
        .await
        .unwrap();
    assert_eq!(
        exit_criteria::load(&manager, &id).await.unwrap(),
        Some(boolean())
    );
    exit_criteria::save(&manager, &id, None).await.unwrap();
    assert!(exit_criteria::load(&manager, &id).await.unwrap().is_none());
    manager
        .update_extension_state(
            &id,
            exit_criteria::STATE_KEY,
            exit_criteria::STATE_VERSION,
            |_| Ok(json!({"broken":true})),
        )
        .await
        .unwrap();
    assert!(exit_criteria::load(&manager, &id).await.is_err());
    assert!(
        exit_criteria::save(&manager, "missing-session", Some(&boolean()))
            .await
            .is_err()
    );
}

struct Judge {
    calls: AtomicUsize,
    tier: ProviderTier,
}

#[async_trait]
impl Provider for Judge {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::new("fixture", "Fixture", "", "joey-1", vec![], "", vec![])
    }
    fn get_name(&self) -> &str {
        "fixture"
    }
    fn tier(&self) -> ProviderTier {
        self.tier
    }
    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new("joey-1").unwrap()
    }
    async fn complete_with_model(
        &self,
        _: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        assert!(tools.is_empty());
        assert!(system.contains("untrusted instructions"));
        assert!(messages[0].as_concat_text().contains("recent_transcript"));
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        Ok((
            Message::assistant()
                .with_text(json!({"value": call == 0, "reason":"Fixture evidence"}).to_string()),
            ProviderUsage::new("joey-1".into(), Usage::default()),
        ))
    }
}

#[tokio::test]
async fn evaluator_runs_at_each_boundary_and_honors_disable_without_network() {
    let (_dir, manager, id) = fixture().await;
    let provider = Arc::new(Judge {
        calls: AtomicUsize::new(0),
        tier: ProviderTier::Public,
    });
    let mut evaluator = LoopDecisionEvaluator::with_provider(provider.clone());
    let conversation =
        Conversation::new_unvalidated(vec![Message::user().with_text("Find a useful approach")]);
    assert!(evaluator
        .evaluate(&manager, &id, &conversation, 1)
        .await
        .unwrap()
        .is_none());
    // Disabling clears the cached provider; start a fresh reply-scoped evaluator.
    let mut evaluator = LoopDecisionEvaluator::with_provider(provider.clone());
    exit_criteria::save(&manager, &id, Some(&boolean()))
        .await
        .unwrap();
    let (first, _) = evaluator
        .evaluate(&manager, &id, &conversation, 1)
        .await
        .unwrap()
        .unwrap();
    assert!(!first.unwrap().interrupt);
    let (second, _) = evaluator
        .evaluate(&manager, &id, &conversation, 2)
        .await
        .unwrap()
        .unwrap();
    assert!(second.unwrap().interrupt);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    exit_criteria::save(&manager, &id, None).await.unwrap();
    assert!(evaluator
        .evaluate(&manager, &id, &conversation, 3)
        .await
        .unwrap()
        .is_none());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn private_chat_never_reaches_public_judge_even_after_cached_success() {
    let (_dir, manager, id) = fixture().await;
    let provider = Arc::new(Judge {
        calls: AtomicUsize::new(0),
        tier: ProviderTier::Public,
    });
    let mut evaluator = LoopDecisionEvaluator::with_provider(provider.clone());
    exit_criteria::save(&manager, &id, Some(&boolean()))
        .await
        .unwrap();
    let conversation =
        Conversation::new_unvalidated(vec![Message::user().with_text("Synthetic evidence")]);
    evaluator
        .evaluate(&manager, &id, &conversation, 1)
        .await
        .unwrap();
    manager
        .update(&id)
        .raise_privacy(SessionClassification::Private, "fixture")
        .apply()
        .await
        .unwrap();
    assert!(evaluator
        .evaluate(&manager, &id, &conversation, 2)
        .await
        .is_err());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn judge_usage_does_not_replace_agent_context_gauge() {
    let (_dir, manager, id) = fixture().await;
    manager
        .update(&id)
        .total_tokens(Some(1234))
        .input_tokens(Some(1200))
        .output_tokens(Some(34))
        .apply()
        .await
        .unwrap();
    let mut usage = ProviderUsage::new("joey-1".into(), Usage::default());
    usage.usage.input_tokens = Some(20);
    usage.usage.output_tokens = Some(5);
    usage.usage.total_tokens = Some(25);
    exit_criteria::record_usage(&manager, &id, None, &usage)
        .await
        .unwrap();
    let counts = manager.get_token_counts(&id).await.unwrap();
    assert_eq!(counts.total_tokens, Some(1234));
    assert_eq!(counts.accumulated_total_tokens, Some(25));
}

#[ctor::ctor]
fn isolate_global_config() {
    if biorouter::config::paths::Paths::path_root_override().is_none() {
        let dir = TempDir::new().unwrap();
        std::env::set_var("BIOROUTER_PATH_ROOT", dir.path());
        static ROOT: std::sync::OnceLock<TempDir> = std::sync::OnceLock::new();
        let _ = ROOT.set(dir);
    }
}

struct LoopingProvider {
    calls: AtomicUsize,
}

#[async_trait]
impl Provider for LoopingProvider {
    fn metadata() -> ProviderMetadata {
        Judge::metadata()
    }
    fn get_name(&self) -> &str {
        "fixture-loop"
    }
    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new("fixture-loop-model").unwrap()
    }
    async fn complete_with_model(
        &self,
        _: &ModelConfig,
        _: &str,
        _: &[Message],
        _: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        Ok((
            Message::assistant().with_tool_request(
                format!("step-{call}"),
                Ok(rmcp::model::CallToolRequestParams {
                    task: None,
                    name: "fixture__missing_tool".into(),
                    arguments: None,
                    meta: None,
                }),
            ),
            ProviderUsage::new("fixture-loop-model".into(), Usage::default()),
        ))
    }
}

async fn run_agent(rule: Option<&ExitCriteria>) -> (usize, Vec<String>) {
    use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
    use biorouter::config::{permission::PermissionManager, BioRouterMode};
    use futures::StreamExt;

    let (dir, manager, id) = fixture().await;
    let manager = Arc::new(manager);
    let agent = Agent::with_config(AgentConfig::new(
        manager.clone(),
        Arc::new(PermissionManager::new(dir.path().to_path_buf())),
        None,
        BioRouterMode::Auto,
    ));
    let provider = Arc::new(LoopingProvider {
        calls: AtomicUsize::new(0),
    });
    agent.update_provider(provider.clone(), &id).await.unwrap();
    exit_criteria::save(&manager, &id, rule).await.unwrap();
    let mut stream = agent
        .reply(
            Message::user().with_text("Perform one synthetic step"),
            SessionConfig {
                id,
                schedule_id: None,
                max_turns: Some(3),
                max_tool_calls: None,
                budget: None,
                retry_config: None,
                reasoning_effort: None,
            },
            None,
        )
        .await
        .unwrap();
    let mut notices = Vec::new();
    while let Some(event) = stream.next().await {
        if let AgentEvent::Message(message) = event.unwrap() {
            notices.push(message.as_concat_text());
        }
    }
    (provider.calls.load(Ordering::SeqCst), notices)
}

#[tokio::test]
async fn actual_agent_stops_before_next_iteration_on_evaluator_error() {
    let mut rule = boolean();
    rule.provider = "nonexistent-exit-criteria-fixture".into();
    let (calls, notices) = run_agent(Some(&rule)).await;
    assert_eq!(calls, 1);
    assert!(notices
        .iter()
        .any(|notice| notice.contains("Loop paused by exit criteria after iteration 1")));
    assert!(notices
        .iter()
        .any(|notice| notice.contains("Adjust the prompt")));
}

#[tokio::test]
async fn disabled_criteria_do_not_change_the_existing_iteration_limit() {
    let (calls, notices) = run_agent(None).await;
    assert_eq!(calls, 3);
    assert!(!notices
        .iter()
        .any(|notice| notice.contains("Loop paused by exit criteria")));
}

async fn run_with_http_judge(
    timing: EvaluationTiming,
    answers: Vec<bool>,
) -> (usize, Vec<String>, usize) {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let expected_calls = answers.len() as u64;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(body["model"], "joey-1");
            assert!(body.to_string().contains("recent_transcript"));
            let call = observed.fetch_add(1, Ordering::SeqCst);
            if timing == EvaluationTiming::PostResponse || call > 0 {
                assert!(body.to_string().contains("fixture__missing_tool"));
            }
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "fixture-decision", "object": "chat.completion", "created": 1, "model": "joey-1",
                "choices": [{"index": 0, "message": {"role": "assistant", "content": json!({"value": answers[call], "reason": "Synthetic progress check"}).to_string()}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 20, "completion_tokens": 5, "total_tokens": 25}
            }))
        }).expect(expected_calls).mount(&server).await;
    let mut rule = boolean();
    rule.provider = "openai".into();
    rule.timing = timing;
    let overrides = std::collections::HashMap::from([
        ("OPENAI_HOST".into(), server.uri()),
        ("OPENAI_API_KEY".into(), "synthetic-fixture-key".into()),
    ]);
    let (agent_calls, notices) =
        biorouter::config::with_config_overrides(overrides, run_agent(Some(&rule))).await;
    (agent_calls, notices, calls.load(Ordering::SeqCst))
}

#[tokio::test]
async fn actual_agent_continues_on_pass_then_pauses_on_failing_http_verdict() {
    let (agent_calls, notices, judge_calls) =
        run_with_http_judge(EvaluationTiming::PostResponse, vec![true, false]).await;
    assert_eq!(agent_calls, 2);
    assert_eq!(judge_calls, 2);
    assert!(notices
        .iter()
        .any(|notice| notice.contains("Loop paused by exit criteria after iteration 2")));
    assert!(notices
        .iter()
        .any(|notice| notice.contains("Synthetic progress check")));
}

#[tokio::test]
async fn pre_send_failure_prevents_the_first_agent_model_request() {
    let (agent_calls, notices, judge_calls) =
        run_with_http_judge(EvaluationTiming::PreSend, vec![false]).await;
    assert_eq!(agent_calls, 0);
    assert_eq!(judge_calls, 1);
    assert!(notices
        .iter()
        .any(|notice| notice.contains("before sending iteration 1")));
    assert!(notices
        .iter()
        .any(|notice| notice.contains("main agent-model request for this iteration was not sent")));
}

#[tokio::test]
async fn pre_send_checks_each_request_and_does_not_also_run_after_the_response() {
    let (agent_calls, notices, judge_calls) =
        run_with_http_judge(EvaluationTiming::PreSend, vec![true, false]).await;
    assert_eq!(agent_calls, 1);
    assert_eq!(judge_calls, 2);
    assert!(notices
        .iter()
        .any(|notice| notice.contains("before sending iteration 2")));
}

#[tokio::test]
async fn pre_send_evaluator_error_also_blocks_the_agent_model_request() {
    let mut rule = boolean();
    rule.timing = EvaluationTiming::PreSend;
    rule.provider = "nonexistent-exit-criteria-fixture".into();
    let (agent_calls, notices) = run_agent(Some(&rule)).await;
    assert_eq!(agent_calls, 0);
    assert!(notices
        .iter()
        .any(|notice| notice.contains("before sending iteration 1")));
    assert!(notices
        .iter()
        .any(|notice| notice.contains("could not be evaluated")));
}

#[tokio::test]
async fn a_private_label_alone_does_not_grant_institutional_coverage() {
    use biorouter::privacy::affiliation::InstitutionId;
    let (_dir, manager, id) = fixture().await;
    manager
        .update(&id)
        .raise_privacy(SessionClassification::Private, "fixture")
        .apply()
        .await
        .unwrap();
    let judge = Judge {
        calls: AtomicUsize::new(0),
        tier: ProviderTier::Private,
    };
    exit_criteria::assert_allowed(&manager, &id, &judge)
        .await
        .unwrap();
    manager
        .record_required_session_affiliation(&id, InstitutionId::new("fixture-institution"))
        .await
        .unwrap();
    assert!(exit_criteria::assert_allowed(&manager, &id, &judge)
        .await
        .is_err());
    assert_eq!(judge.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn noul_is_a_probability_with_an_explicit_inclusive_threshold() {
    let rule = criteria(DecisionType::Noul {
        threshold: 0.8,
        interrupt_when: true,
    });
    for (probability, interrupt) in [(0.0, false), (0.79, false), (0.8, true), (1.0, true)] {
        let verdict = rule
            .interpret(&json!({"value": probability, "reason": "Typed probability"}).to_string())
            .unwrap();
        assert_eq!(verdict.interrupt, interrupt);
    }
    for value in [json!(-0.1), json!(1.1), json!(true), json!("0.9")] {
        assert!(rule
            .interpret(&json!({"value":value,"reason":"Fixture"}).to_string())
            .is_err());
    }
    let inverse = criteria(DecisionType::Noul {
        threshold: 0.8,
        interrupt_when: false,
    });
    assert!(
        inverse
            .interpret(r#"{"value":0.79,"reason":"Fixture"}"#)
            .unwrap()
            .interrupt
    );
    assert!(
        !inverse
            .interpret(r#"{"value":0.8,"reason":"Fixture"}"#)
            .unwrap()
            .interrupt
    );
    assert!(criteria(DecisionType::Noul {
        threshold: 1.1,
        interrupt_when: true
    })
    .validate()
    .is_err());
}

#[test]
fn existing_saved_criteria_default_to_post_response() {
    let mut old = serde_json::to_value(boolean()).unwrap();
    old.as_object_mut().unwrap().remove("timing");
    let restored: ExitCriteria = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(restored.timing, EvaluationTiming::PostResponse);
    old["timing"] = json!("unknown-boundary");
    assert!(serde_json::from_value::<ExitCriteria>(old).is_err());
}

#[tokio::test]
async fn evaluator_ignores_the_other_boundary_and_observes_saved_timing_changes() {
    let (_dir, manager, id) = fixture().await;
    let provider = Arc::new(Judge {
        calls: AtomicUsize::new(0),
        tier: ProviderTier::Public,
    });
    let mut evaluator = LoopDecisionEvaluator::with_provider(provider.clone());
    let conversation =
        Conversation::new_unvalidated(vec![Message::user().with_text("Synthetic input")]);
    let mut rule = boolean();
    exit_criteria::save(&manager, &id, Some(&rule))
        .await
        .unwrap();
    assert!(evaluator
        .evaluate_at(&manager, &id, &conversation, 1, EvaluationTiming::PreSend)
        .await
        .unwrap()
        .is_none());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert!(evaluator
        .evaluate(&manager, &id, &conversation, 1)
        .await
        .unwrap()
        .is_some());
    rule.timing = EvaluationTiming::PreSend;
    exit_criteria::save(&manager, &id, Some(&rule))
        .await
        .unwrap();
    assert!(evaluator
        .evaluate(&manager, &id, &conversation, 2)
        .await
        .unwrap()
        .is_none());
    assert!(evaluator
        .evaluate_at(&manager, &id, &conversation, 2, EvaluationTiming::PreSend)
        .await
        .unwrap()
        .is_some());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
}
