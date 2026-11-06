//! A priced decision model's spend counts toward the per-reply dollar cap
//! (BR-35), through the real reply loop: a monitor on a Baseten decision model
//! spends inside the reply, and the loop folds those dollars into the budget
//! at its next iteration boundary. Offline: the chat model is a scripted
//! provider priced as Claude, and the decision model a scripted
//! [`DecisionModel`] behind the agent's own decision service.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, ReplyBudget, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{Message, MessageContent};
use biorouter::decision::model::DecisionCaps;
use biorouter::decision::settings::{
    DecisionSettings, MonitorAction, MonitorCriterion, MonitorErrorPolicy, MonitorTiming,
    MonitorWindow,
};
use biorouter::decision::types::{DecisionKindTag, ModelVerdicts, QuestionVerdict};
use biorouter::decision::{
    DecisionAdapter, DecisionKind, DecisionModel, DecisionModelEntry, DecisionModelRef,
    DecisionModelSource, DecisionRequest, DecisionResult, DecisionService, DecisionUsage,
    ProbabilitySource, Question,
};
use biorouter::model::ModelConfig;
use biorouter::privacy::{ModelAffiliation, ProviderTier};
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use futures::StreamExt;
use indexmap::IndexMap;
use rmcp::model::{CallToolRequestParams, Tool};
use tempfile::TempDir;

#[ctor::ctor]
fn sandbox() {
    let root = TempDir::new().expect("a sandbox");
    for sub in ["config", "data", "state"] {
        std::fs::create_dir_all(root.path().join(sub)).expect("a sandbox tree");
    }
    std::fs::write(root.path().join("config/config.yaml"), "{}\n").expect("a config file");
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
    static ROOT: OnceLock<TempDir> = OnceLock::new();
    let _ = ROOT.set(root);
}

/// A decision model that answers P(yes) 0.1 to every question and reports
/// `input_tokens` of usage per request, under `provider`/`model`.
struct Billed {
    provider: &'static str,
    model: &'static str,
    tier: ProviderTier,
    input_tokens: i64,
    requests: AtomicUsize,
}

impl Billed {
    fn entry(&self) -> DecisionModelEntry {
        DecisionModelEntry {
            provider: self.provider.to_string(),
            model: self.model.to_string(),
            display_name: self.model.to_string(),
            context_tokens: 16_384,
            kinds: vec![DecisionKindTag::Binary],
            tier: self.tier,
            affiliation: (self.tier == ProviderTier::Private).then_some(ModelAffiliation::Local),
            adapter: DecisionAdapter::SystemOne,
            local: false,
            load_refusal: None,
        }
    }
}

struct Model(Arc<Billed>);

#[async_trait]
impl DecisionModel for Model {
    fn model_ref(&self) -> DecisionModelRef {
        self.0.entry().model_ref()
    }
    fn display_name(&self) -> String {
        self.0.model.to_string()
    }
    fn tier(&self) -> ProviderTier {
        self.0.tier
    }
    fn affiliation(&self) -> Option<ModelAffiliation> {
        self.0.entry().affiliation
    }
    fn caps(&self) -> DecisionCaps {
        DecisionCaps {
            kinds: self.0.entry().kinds,
            max_questions: 64,
            context_tokens: 16_384,
            max_concurrency: 8,
            max_request_bytes: None,
            local: false,
        }
    }
    async fn decide(&self, request: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
        self.0.requests.fetch_add(1, Ordering::SeqCst);
        Ok(ModelVerdicts {
            verdicts: request
                .questions
                .keys()
                .map(|name| QuestionVerdict {
                    question: name.clone(),
                    probabilities: IndexMap::from([
                        ("yes".to_string(), 0.1),
                        ("no".to_string(), 0.9),
                    ]),
                    source: ProbabilitySource::Native,
                    calibrated: true,
                    reported_choice: None,
                    raw: serde_json::Value::Null,
                })
                .collect(),
            usage: DecisionUsage {
                input_tokens: Some(self.0.input_tokens),
                output_tokens: Some(1),
            },
            served_model: None,
        })
    }
}

struct Source(Arc<Billed>);

#[async_trait]
impl DecisionModelSource for Source {
    async fn list(&self) -> Vec<DecisionModelEntry> {
        vec![self.0.entry()]
    }
    async fn open(&self, _: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
        Ok(Arc::new(Model(self.0.clone())))
    }
}

/// A chat model priced as Claude Sonnet 4.5 that calls a tool nobody offers on
/// every turn, so the reply keeps going until something stops it.
struct Llm {
    calls: AtomicUsize,
}

#[async_trait]
impl Provider for Llm {
    async fn complete(
        &self,
        system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let usage = ProviderUsage::new(
            "claude-sonnet-4-5".to_string(),
            Usage::new(Some(10), Some(5), Some(15)),
        );
        if system_prompt.contains("You are a loop monitor") {
            return Ok((
                Message::assistant().with_text(r#"{"looping":false}"#),
                usage,
            ));
        }
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let message = Message::assistant()
            .with_text("Working.")
            .with_tool_request(
                format!("call-{n}"),
                Ok(CallToolRequestParams {
                    task: None,
                    meta: None,
                    name: "nothing__missing".into(),
                    arguments: Some(rmcp::object!({"step": n})),
                }),
            );
        Ok((message, usage))
    }

    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        system_prompt: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.complete(system_prompt, messages, tools).await
    }

    fn get_model_config(&self) -> ModelConfig {
        let mut config = ModelConfig::new_or_fail("claude-sonnet-4-5");
        config.context_limit = Some(1_000_000);
        config
    }

    fn metadata() -> ProviderMetadata {
        ProviderMetadata {
            name: "anthropic".to_string(),
            display_name: "Scripted Claude".to_string(),
            description: "test".to_string(),
            default_model: "claude-sonnet-4-5".to_string(),
            known_models: vec![],
            model_doc_link: String::new(),
            config_keys: vec![],
            allows_unlisted_models: false,
            tier: Default::default(),
            runs_locally: false,
            institutions: Vec::new(),
        }
    }

    fn get_name(&self) -> &str {
        "anthropic"
    }

    fn tier(&self) -> ProviderTier {
        ProviderTier::Public
    }

    fn affiliation(&self) -> Option<ModelAffiliation> {
        None
    }
}

fn monitor() -> MonitorCriterion {
    MonitorCriterion {
        id: "m1".into(),
        name: "Form reached".into(),
        question: Question {
            instructions: "Has the agent reached the consent form page?".into(),
            kind: DecisionKind::Binary {},
            threshold: None,
        },
        model: None,
        timing: MonitorTiming::PostIteration,
        window: MonitorWindow::default(),
        threshold: 0.9,
        option: None,
        consecutive: 1,
        rearm_below: None,
        action: MonitorAction::Notify,
        on_error: MonitorErrorPolicy::Notify,
        max_evals_per_turn: 50,
        enabled: true,
    }
}

/// One reply with a $0.01 cap and a monitor on `decisions`: the notices it
/// showed, and how many times the chat model was called.
async fn reply_under_a_cent_cap(decisions: Arc<Billed>) -> (Vec<String>, usize) {
    let dir = TempDir::new().unwrap();
    let sessions = Arc::new(SessionManager::new_ephemeral(dir.path().join("data")));
    let agent = Agent::with_config(AgentConfig::new(
        sessions.clone(),
        Arc::new(PermissionManager::new(dir.path().join("permissions"))),
        None,
        BioRouterMode::Auto,
    ));
    let session = sessions
        .create_session(
            dir.path().to_path_buf(),
            "decision budget test".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    biorouter::decision::settings::save(
        &sessions,
        &session.id,
        DecisionSettings {
            default_model: None,
            monitors: vec![monitor()],
        },
    )
    .await
    .expect("a valid monitor is stored");
    agent.set_decision_service(Arc::new(
        DecisionService::new(Arc::new(Source(decisions)))
            .with_default_model(None)
            .with_timeouts(Duration::from_secs(5), Duration::from_secs(5)),
    ));
    let llm = Arc::new(Llm {
        calls: AtomicUsize::new(0),
    });
    agent
        .update_provider(llm.clone() as Arc<dyn Provider>, &session.id)
        .await
        .unwrap();
    let config = SessionConfig {
        id: session.id.clone(),
        schedule_id: None,
        max_turns: Some(8),
        max_tool_calls: None,
        retry_config: None,
        budget: Some(ReplyBudget {
            max_seconds: None,
            max_tokens: None,
            max_usd: Some(0.01),
        }),
        reasoning_effort: None,
    };
    let stream = agent
        .reply(Message::user().with_text("Fill in the form."), config, None)
        .await
        .expect("a reply stream");
    tokio::pin!(stream);
    let mut notices = Vec::new();
    while let Some(event) = stream.next().await {
        if let AgentEvent::Message(message) = event.expect("a reply event") {
            notices.extend(message.content.iter().filter_map(|c| match c {
                MessageContent::SystemNotification(n) => Some(n.msg.clone()),
                _ => None,
            }));
        }
    }
    (notices, llm.calls.load(Ordering::SeqCst))
}

fn budget_notices(notices: &[String]) -> usize {
    notices
        .iter()
        .filter(|n| n.contains("Budget reached"))
        .count()
}

/// mammoth-1 bills $0.10 per million input tokens, so a monitor request of
/// 200,000 tokens costs $0.02, twice the reply's cap. The chat model's own
/// turns cost a hundredth of a cent each, so only the decision spend can
/// reach the cap.
#[tokio::test(flavor = "multi_thread")]
async fn a_priced_monitor_spends_toward_the_reply_dollar_cap() {
    let decisions = Arc::new(Billed {
        provider: "baseten_decision",
        model: "mammoth-1",
        tier: ProviderTier::Public,
        input_tokens: 200_000,
        requests: AtomicUsize::new(0),
    });
    let (notices, calls) = reply_under_a_cent_cap(decisions.clone()).await;
    assert!(decisions.requests.load(Ordering::SeqCst) >= 1);
    assert_eq!(budget_notices(&notices), 1, "{notices:#?}");
    assert!(
        calls < 8,
        "the reply wrapped up before max_turns: {calls} calls"
    );
}

/// A local decision model has no price: its tokens never reach the dollar
/// cap, and the chat's own small spend stays under it.
#[tokio::test(flavor = "multi_thread")]
async fn an_unpriced_local_monitor_adds_nothing_to_the_dollar_cap() {
    let decisions = Arc::new(Billed {
        provider: "ollama_decision",
        model: "tev1:0.8b",
        tier: ProviderTier::Private,
        input_tokens: 5_000_000,
        requests: AtomicUsize::new(0),
    });
    let (notices, _) = reply_under_a_cent_cap(decisions.clone()).await;
    assert!(decisions.requests.load(Ordering::SeqCst) >= 1);
    assert_eq!(budget_notices(&notices), 0, "{notices:#?}");
}
