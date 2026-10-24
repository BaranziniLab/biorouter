//! Issue #435: long tasks are bounded by progress and explicit user limits.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use biorouter::agents::extension::ExtensionConfig;
use biorouter::agents::turn_abort::TurnAbortCode;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::Message;
use biorouter::model::ModelConfig;
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use futures::StreamExt;
use rmcp::model::{CallToolRequestParams, Tool};
use rmcp::object;
use tempfile::TempDir;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

const FINISHED: &str = "LONG_HORIZON_TASK_FINISHED";

#[ctor::ctor]
fn sandbox_global_config() {
    if biorouter::config::paths::Paths::path_root_override().is_some() {
        return;
    }
    let root = TempDir::new().expect("scratch config root");
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    static ROOT: std::sync::OnceLock<TempDir> = std::sync::OnceLock::new();
    let _ = ROOT.set(root);
}

#[derive(Clone, Copy)]
enum Work {
    Progress { iterations: usize, batch: usize },
    Repeat,
    Oscillate,
    Hold,
}

struct LongTaskProvider {
    work: Work,
    calls: AtomicUsize,
    started: Notify,
}

impl LongTaskProvider {
    fn new(work: Work) -> Arc<Self> {
        Arc::new(Self {
            work,
            calls: AtomicUsize::new(0),
            started: Notify::new(),
        })
    }

    fn command(&self, iteration: usize, item: usize) -> String {
        match self.work {
            Work::Progress { .. } => format!("printf completed-{iteration}-{item}"),
            Work::Repeat => "printf unchanged".to_string(),
            Work::Oscillate => format!("printf state-{}", iteration % 2),
            Work::Hold => unreachable!(),
        }
    }
}

#[async_trait]
impl Provider for LongTaskProvider {
    async fn complete(
        &self,
        system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let usage = ProviderUsage::new("long-horizon-mock".to_string(), Usage::default());
        if system_prompt.contains("You are a loop monitor") {
            return Ok((
                Message::assistant().with_text(r#"{"looping":false}"#),
                usage,
            ));
        }
        let iteration = self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        if matches!(self.work, Work::Hold) {
            std::future::pending::<()>().await;
        }
        let batch = match self.work {
            Work::Progress { iterations, .. } if iteration >= iterations => {
                return Ok((Message::assistant().with_text(FINISHED), usage));
            }
            Work::Progress { batch, .. } => batch,
            _ => 1,
        };
        let mut message = Message::assistant();
        for item in 0..batch {
            message = message.with_tool_request(
                format!("step-{iteration}-{item}"),
                Ok(CallToolRequestParams {
                    task: None,
                    meta: None,
                    name: "developer__shell".into(),
                    arguments: Some(object!({"command": self.command(iteration, item)})),
                }),
            );
        }
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
        let mut config = ModelConfig::new_or_fail("long-horizon-mock");
        config.context_limit = Some(1_000_000);
        config
    }

    fn metadata() -> ProviderMetadata {
        ProviderMetadata {
            name: "long-horizon-mock".to_string(),
            display_name: "Long horizon mock".to_string(),
            description: "test".to_string(),
            default_model: "long-horizon-mock".to_string(),
            known_models: vec![],
            model_doc_link: String::new(),
            config_keys: vec![],
            allows_unlisted_models: false,
            tier: Default::default(),
            runs_locally: true,
            institutions: Vec::new(),
        }
    }

    fn get_name(&self) -> &str {
        "long-horizon-mock"
    }
}

struct TestAgent {
    agent: Agent,
    sessions: Arc<SessionManager>,
    session_id: String,
    _work: TempDir,
    _data: TempDir,
    _permissions: TempDir,
}

impl TestAgent {
    async fn new(provider: Arc<LongTaskProvider>) -> Self {
        let work = TempDir::new().unwrap();
        let data = TempDir::new().unwrap();
        let permissions = TempDir::new().unwrap();
        let sessions = Arc::new(SessionManager::new(data.path().to_path_buf()));
        let agent = Agent::with_config(AgentConfig::new(
            sessions.clone(),
            Arc::new(PermissionManager::new(permissions.path().to_path_buf())),
            None,
            BioRouterMode::Auto,
        ));
        agent
            .add_extension(ExtensionConfig::Builtin {
                name: "developer".to_string(),
                description: "developer".to_string(),
                display_name: Some("Developer".to_string()),
                timeout: Some(30),
                bundled: Some(true),
                available_tools: vec![],
            })
            .await
            .unwrap();
        let session = sessions
            .create_session(
                work.path().to_path_buf(),
                "long-horizon".to_string(),
                SessionType::Hidden,
            )
            .await
            .unwrap();
        agent.update_provider(provider, &session.id).await.unwrap();
        Self {
            agent,
            sessions,
            session_id: session.id,
            _work: work,
            _data: data,
            _permissions: permissions,
        }
    }

    fn config(&self) -> SessionConfig {
        SessionConfig {
            id: self.session_id.clone(),
            schedule_id: None,
            max_turns: None,
            max_tool_calls: None,
            budget: None,
            retry_config: None,
            reasoning_effort: None,
        }
    }

    async fn run(
        &self,
        config: SessionConfig,
        cancel: Option<CancellationToken>,
    ) -> Vec<AgentEvent> {
        let stream = self
            .agent
            .reply(
                Message::user().with_text("Finish every step of this task."),
                config,
                cancel,
            )
            .await
            .unwrap();
        tokio::pin!(stream);
        tokio::time::timeout(Duration::from_secs(240), async {
            let mut events = Vec::new();
            while let Some(event) = stream.next().await {
                events.push(event.unwrap());
            }
            events
        })
        .await
        .expect("the synthetic agent loop must finish")
    }
}

fn visible_text(events: &[AgentEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::Message(message) => Some(message.as_concat_text()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn progressing_default_reply_crosses_both_old_limits() {
    let provider = LongTaskProvider::new(Work::Progress {
        iterations: 110,
        batch: 2,
    });
    let fixture = TestAgent::new(provider.clone()).await;
    let events = fixture.run(fixture.config(), None).await;
    assert!(visible_text(&events).contains(FINISHED));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 111);
    assert!(!events
        .iter()
        .any(|event| matches!(event, AgentEvent::TurnAborted { .. })));
    let stored = fixture
        .sessions
        .get_session(&fixture.session_id, true)
        .await
        .unwrap()
        .conversation
        .unwrap();
    let responses = stored
        .messages()
        .iter()
        .flat_map(|message| &message.content)
        .filter_map(|part| part.as_tool_response())
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 220);
    assert!(responses.iter().all(|response| response
        .tool_result
        .as_ref()
        .is_ok_and(|result| result.is_error != Some(true))));
}

#[tokio::test]
async fn an_explicit_turn_limit_still_stops_a_progressing_reply() {
    let provider = LongTaskProvider::new(Work::Progress {
        iterations: 20,
        batch: 1,
    });
    let fixture = TestAgent::new(provider.clone()).await;
    let mut config = fixture.config();
    config.max_turns = Some(2);
    let events = fixture.run(config, None).await;
    assert!(!visible_text(&events).contains(FINISHED));
    assert!(visible_text(&events).contains("2 actions without user input"));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn an_explicit_tool_limit_still_stops_a_progressing_reply() {
    let provider = LongTaskProvider::new(Work::Progress {
        iterations: 20,
        batch: 2,
    });
    let fixture = TestAgent::new(provider.clone()).await;
    let mut config = fixture.config();
    config.max_tool_calls = Some(3);
    let events = fixture.run(config, None).await;
    assert!(!visible_text(&events).contains(FINISHED));
    assert!(visible_text(&events).contains("per-turn limit of 3"));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn an_unlimited_reply_still_obeys_cancellation() {
    let provider = LongTaskProvider::new(Work::Hold);
    let fixture = TestAgent::new(provider.clone()).await;
    let cancel = CancellationToken::new();
    let stop = async {
        provider.started.notified().await;
        cancel.cancel();
    };
    let (events, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(fixture.run(fixture.config(), Some(cancel.clone())), stop)
    })
    .await
    .expect("cancellation must interrupt an in-flight provider call");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert!(!visible_text(&events).contains(FINISHED));
}

#[tokio::test]
async fn repeated_unchanged_work_stops_the_real_agent_loop() {
    for work in [Work::Repeat, Work::Oscillate] {
        let provider = LongTaskProvider::new(work);
        let fixture = TestAgent::new(provider.clone()).await;
        let events = fixture.run(fixture.config(), None).await;
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::TurnAborted {
                code: TurnAbortCode::ToolLoop { .. },
                ..
            }
        )));
        assert!(provider.calls.load(Ordering::SeqCst) <= 10);
        assert!(!visible_text(&events).contains(FINISHED));
    }
}
