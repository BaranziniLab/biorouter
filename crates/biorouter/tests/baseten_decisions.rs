use biorouter::agents::baseten_decisions::{self, BasetenDecisionProvider};
use biorouter::agents::exit_criteria::{self, DecisionType, ExitCriteria};
use biorouter::conversation::message::Message;
use biorouter::privacy::{ProviderTier, SessionClassification};
use biorouter::providers::api_client::{ApiClient, AuthMethod};
use biorouter::providers::base::Provider;
use biorouter::session::{SessionManager, SessionType};
use serde_json::{json, Value};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn criteria(decision: DecisionType) -> ExitCriteria {
    ExitCriteria {
        timing: biorouter::agents::exit_criteria::EvaluationTiming::PostResponse,
        provider: "baseten".into(),
        model: "decision-models/baseten-joey-2b-latest".into(),
        prompt: "Is the synthetic test state acceptable?".into(),
        decision,
    }
}

fn provider(rule: &ExitCriteria, server: &MockServer) -> BasetenDecisionProvider {
    BasetenDecisionProvider::new(
        rule.clone(),
        ApiClient::new(
            server.uri(),
            AuthMethod::BearerToken("synthetic-test-key".into()),
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn dedicated_catalog_contains_only_decision_models_and_resolves_display_aliases() {
    let models = baseten_decisions::models();
    assert_eq!(
        models.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
        ["elephant-1", "mammoth-1", "joey-1"]
    );
    for model in models {
        assert!(model.slug.starts_with("decision-models/"));
        assert_eq!(
            baseten_decisions::model_slug(&model.name),
            Some(model.slug.clone())
        );
        assert_eq!(baseten_decisions::model_slug(&model.slug), Some(model.slug));
    }
    assert!(baseten_decisions::model_slug("zai-org/GLM-5.3").is_none());
}

async fn evaluate(
    rule: &ExitCriteria,
    answer: Value,
    question: Value,
) -> (Value, biorouter::providers::base::ProviderUsage) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(header("authorization", "Bearer synthetic-test-key"))
        .and(body_partial_json(json!({"model":rule.model, "state":{"iteration":1,"recent_transcript":"Synthetic tool result"}, "questions":{"loop_exit":question}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"model":"backend-id-may-differ", "answers":{"loop_exit":answer}, "usage":{"input_tokens":42,"output_tokens":1}})))
        .expect(1).mount(&server).await;
    let provider = provider(rule, &server);
    assert_eq!(provider.tier(), ProviderTier::Public);
    let evidence = Message::user()
        .with_text(json!({"iteration":1,"recent_transcript":"Synthetic tool result"}).to_string());
    let (message, usage) = provider
        .complete(
            "Do not use the generic JSON chat contract",
            &[evidence],
            &[],
        )
        .await
        .unwrap();
    (
        serde_json::from_str(&message.as_concat_text()).unwrap(),
        usage,
    )
}

#[tokio::test]
async fn native_noul_uses_system_one_not_chat_and_preserves_probability_and_usage() {
    let rule = criteria(DecisionType::Noul {
        threshold: 0.9,
        interrupt_when: true,
    });
    let (response, usage) = evaluate(
        &rule,
        json!({"type":"noul","noul":0.97}),
        json!({"type":"noul","instructions":rule.prompt}),
    )
    .await;
    assert_eq!(response["value"], 0.97);
    assert!(rule.interpret(&response.to_string()).unwrap().interrupt);
    assert_eq!(usage.model, rule.model);
    assert_eq!(usage.provider.as_deref(), Some("baseten"));
    assert_eq!(usage.usage.total_tokens, Some(43));
    assert!(response["reason"]
        .as_str()
        .unwrap()
        .contains("not a generated explanation"));
}

#[tokio::test]
async fn native_choice_uses_option_names_not_distribution_confidence() {
    let rule = criteria(DecisionType::Choice {
        choices: vec!["continue".into(), "review".into()],
        interrupt_when: "review".into(),
    });
    let (response, _) = evaluate(&rule, json!({"type":"choice","choice":"review","probabilities":{"review":0.6,"continue":0.4},"confidence":0.1}), json!({"type":"choice","instructions":rule.prompt,"criteria":{"continue":"continue","review":"review"}})).await;
    assert_eq!(response["value"], "review");
    assert!(rule.interpret(&response.to_string()).unwrap().interrupt);
}

#[tokio::test]
async fn native_ranking_sends_an_ordered_zero_based_score_rubric() {
    let rule = criteria(DecisionType::Ranking {
        min: 0.0,
        max: 2.0,
        interrupt_below: 1.5,
        levels: vec!["Poor".into(), "Fair".into(), "Good".into()],
    });
    let (response, _) = evaluate(
        &rule,
        json!({"type":"score","score":1.25,"legend":{"0":"Poor","1":"Fair","2":"Good"}}),
        json!({"type":"score","instructions":rule.prompt,"criteria":["Poor","Fair","Good"]}),
    )
    .await;
    assert!(rule.interpret(&response.to_string()).unwrap().interrupt);
    let server = MockServer::start().await;
    let missing_rubric = criteria(DecisionType::Ranking {
        min: 0.0,
        max: 10.0,
        interrupt_below: 5.0,
        levels: vec![],
    });
    let client = ApiClient::new(server.uri(), AuthMethod::BearerToken("fixture".into())).unwrap();
    assert!(BasetenDecisionProvider::new(missing_rubric, client).is_err());
}

#[tokio::test]
async fn wrong_answer_type_fails_closed_without_discarding_reported_usage() {
    let rule = criteria(DecisionType::Noul {
        threshold: 0.8,
        interrupt_when: true,
    });
    let (response, usage) = evaluate(
        &rule,
        json!({"type":"choice","choice":"continue"}),
        json!({"type":"noul","instructions":rule.prompt}),
    )
    .await;
    assert!(rule.interpret(&response.to_string()).is_err());
    assert_eq!(usage.usage.total_tokens, Some(43));
}

#[tokio::test]
async fn private_chat_refuses_native_public_decision_provider_before_sending_state() {
    let server = MockServer::start().await;
    let dir = tempfile::TempDir::new().unwrap();
    let manager = SessionManager::new_ephemeral(dir.path().into());
    let session = manager
        .create_session(
            dir.path().into(),
            "Synthetic private test".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    manager
        .update(&session.id)
        .raise_privacy(SessionClassification::Private, "fixture")
        .apply()
        .await
        .unwrap();
    let rule = criteria(DecisionType::Noul {
        threshold: 0.9,
        interrupt_when: true,
    });
    assert!(
        exit_criteria::assert_allowed(&manager, &session.id, &provider(&rule, &server))
            .await
            .is_err()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn remote_validation_body_is_not_echoed_back_to_chat() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(422)
                .set_body_json(json!({"error":"SYNTHETIC_SENSITIVE_STATE_SENTINEL"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let rule = criteria(DecisionType::Noul {
        threshold: 0.9,
        interrupt_when: true,
    });
    let error = provider(&rule, &server)
        .complete("", &[Message::user().with_text("Synthetic evidence")], &[])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("422"));
    assert!(!error
        .to_string()
        .contains("SYNTHETIC_SENSITIVE_STATE_SENTINEL"));
}
