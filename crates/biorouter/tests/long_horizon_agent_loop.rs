//! Issue #435: long tasks are bounded by progress and explicit user limits.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

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
const REPETITION_DISPATCHES: &str = "repetition-dispatches";
const REPEATED_COMMAND: &str = "printf unchanged; printf . >> repetition-dispatches";
const PROGRESS_COMMAND: &str = "wc -c repetition-dispatches";

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
    RecoverAfterDenial,
    RetryBlockedAfterChangedSibling,
    TwoDenialsInOneBatch,
    Hold,
}

struct LongTaskProvider {
    work: Work,
    calls: AtomicUsize,
    denial_feedback_calls: AtomicUsize,
    started: Notify,
}

impl LongTaskProvider {
    fn new(work: Work) -> Arc<Self> {
        Arc::new(Self {
            work,
            calls: AtomicUsize::new(0),
            denial_feedback_calls: AtomicUsize::new(0),
            started: Notify::new(),
        })
    }

    fn command(&self, iteration: usize, item: usize) -> String {
        match self.work {
            Work::Progress { .. } => format!("printf completed-{iteration}-{item}"),
            Work::Repeat => "printf unchanged".to_string(),
            Work::Oscillate => format!("printf state-{}", iteration % 2),
            Work::RecoverAfterDenial
            | Work::RetryBlockedAfterChangedSibling
            | Work::TwoDenialsInOneBatch => REPEATED_COMMAND.to_string(),
            Work::Hold => unreachable!(),
        }
    }
}

#[async_trait]
impl Provider for LongTaskProvider {
    async fn complete(
        &self,
        system_prompt: &str,
        messages: &[Message],
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
        let feedback_work = matches!(
            self.work,
            Work::RecoverAfterDenial
                | Work::RetryBlockedAfterChangedSibling
                | Work::TwoDenialsInOneBatch
        );
        if feedback_work && saw_repetition_denial(messages) {
            let feedback = self.denial_feedback_calls.fetch_add(1, Ordering::SeqCst);
            if matches!(self.work, Work::RetryBlockedAfterChangedSibling) && feedback == 0 {
                return Ok((
                    with_shell_call(
                        Message::assistant(),
                        "blocked-retry".to_string(),
                        REPEATED_COMMAND.to_string(),
                    ),
                    usage,
                ));
            }
            return Ok((Message::assistant().with_text(FINISHED), usage));
        }
        if feedback_work && iteration >= 8 {
            return Ok((
                Message::assistant().with_text("REPETITION_FIXTURE_EXHAUSTED"),
                usage,
            ));
        }
        if matches!(self.work, Work::RetryBlockedAfterChangedSibling) && iteration == 0 {
            return Ok((
                with_shell_call(
                    Message::assistant(),
                    "progress-before".to_string(),
                    PROGRESS_COMMAND.to_string(),
                ),
                usage,
            ));
        }
        let batch = match self.work {
            Work::TwoDenialsInOneBatch => 6,
            Work::Progress { iterations, .. } if iteration >= iterations => {
                return Ok((Message::assistant().with_text(FINISHED), usage));
            }
            Work::Progress { batch, .. } => batch,
            _ => 1,
        };
        let mut message = Message::assistant();
        for item in 0..batch {
            message = with_shell_call(
                message,
                format!("step-{iteration}-{item}"),
                self.command(iteration, item),
            );
        }
        // The default fifth A call is denied; B still returns changed output in that batch.
        if matches!(self.work, Work::RetryBlockedAfterChangedSibling) && iteration == 5 {
            message = with_shell_call(
                message,
                "progress-after".to_string(),
                PROGRESS_COMMAND.to_string(),
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

fn with_shell_call(message: Message, id: String, command: String) -> Message {
    message.with_tool_request(
        id,
        Ok(CallToolRequestParams {
            task: None,
            meta: None,
            name: "developer__shell".into(),
            arguments: Some(object!({"command": command})),
        }),
    )
}

fn saw_repetition_denial(messages: &[Message]) -> bool {
    messages
        .iter()
        .flat_map(|message| &message.content)
        .any(|part| {
            part.as_tool_response().is_some_and(|response| {
                response.tool_result.as_ref().is_ok_and(|result| {
                    result.is_error == Some(true)
                        && result.content.iter().any(|content| {
                            content.as_text().is_some_and(|text| {
                                text.text.contains("automatic repetition guard")
                            })
                        })
                })
            })
        })
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
            .extension_manager
            .set_working_dir(work.path().to_path_buf())
            .await;
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

fn repetition_fixture_overrides() -> std::collections::HashMap<String, String> {
    let semantic = biorouter::tool_monitor::SemanticLoopConfig::default();
    let failure = biorouter::tool_monitor::FailureLoopConfig::default();
    [
        ("BIOROUTER_REPETITION_SOFT_WARN", "3".to_string()),
        ("BIOROUTER_REPETITION_HARD_STOP", "5".to_string()),
        (
            "BIOROUTER_LOOP_SEMANTIC_DETECTION",
            semantic.enabled.to_string(),
        ),
        (
            "BIOROUTER_LOOP_ARG_SIMILARITY",
            semantic.similarity_threshold.to_string(),
        ),
        (
            "BIOROUTER_LOOP_NEAR_DUP_SOFT_WARN",
            semantic.near_dup_soft_warn.unwrap_or(0).to_string(),
        ),
        (
            "BIOROUTER_LOOP_NEAR_DUP_HARD_STOP",
            semantic.near_dup_hard_stop.unwrap_or(0).to_string(),
        ),
        (
            "BIOROUTER_LOOP_OSCILLATION_SOFT_WARN",
            semantic.oscillation_soft_warn.unwrap_or(0).to_string(),
        ),
        (
            "BIOROUTER_LOOP_OSCILLATION_HARD_STOP",
            semantic.oscillation_hard_stop.unwrap_or(0).to_string(),
        ),
        (
            "BIOROUTER_FAILURE_LOOP_DETECTION",
            failure.enabled.to_string(),
        ),
        (
            "BIOROUTER_FAILURE_ERROR_SIMILARITY",
            failure.similarity_threshold.to_string(),
        ),
        (
            "BIOROUTER_FAILURE_LOOP_SOFT_WARN",
            failure.soft_warn_at.unwrap_or(0).to_string(),
        ),
        (
            "BIOROUTER_FAILURE_LOOP_ESCALATE",
            failure.escalate_at.unwrap_or(0).to_string(),
        ),
        (
            "BIOROUTER_FAILURE_LOOP_HARD_STOP",
            failure.hard_stop_at.unwrap_or(0).to_string(),
        ),
        (
            "BIOROUTER_FAILURE_LOOP_DENY_RETRYABLE",
            failure.deny_retryable.to_string(),
        ),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value))
    .collect()
}

fn is_tool_loop(events: &[AgentEvent]) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            AgentEvent::TurnAborted {
                code: TurnAbortCode::ToolLoop { .. },
                ..
            }
        )
    })
}

fn response_text(messages: &[Message], id: &str) -> String {
    messages
        .iter()
        .flat_map(|message| &message.content)
        .find(|part| {
            part.as_tool_response()
                .is_some_and(|response| response.id == id)
        })
        .unwrap_or_else(|| panic!("missing tool response {id}"))
        .as_tool_response_text()
        .unwrap_or_else(|| panic!("missing tool response text {id}"))
}

#[tokio::test]
#[cfg_attr(not(unix), ignore = "requires the POSIX shell fixture")]
async fn first_repetition_denial_reaches_one_feedback_request_and_can_finish() {
    biorouter::config::with_config_overrides(repetition_fixture_overrides(), async {
        let provider = LongTaskProvider::new(Work::RecoverAfterDenial);
        let fixture = TestAgent::new(provider.clone()).await;
        let events = fixture.run(fixture.config(), None).await;

        assert_eq!(provider.denial_feedback_calls.load(Ordering::SeqCst), 1);
        assert_eq!(provider.calls.load(Ordering::SeqCst), 6);
        assert!(!is_tool_loop(&events));
        assert!(visible_text(&events).contains(FINISHED));
        assert_eq!(
            std::fs::read(fixture._work.path().join(REPETITION_DISPATCHES)).unwrap(),
            b"....",
            "the denied fifth call must not dispatch"
        );
        let stored = fixture
            .sessions
            .get_session(&fixture.session_id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        assert!(saw_repetition_denial(stored.messages()));
        assert!(stored
            .messages()
            .iter()
            .any(|message| message.as_concat_text().contains(FINISHED)));
    })
    .await;
}

#[tokio::test]
#[cfg_attr(not(unix), ignore = "requires the POSIX shell fixture")]
async fn a_changed_sibling_cannot_reopen_a_blocked_signature_on_feedback() {
    biorouter::config::with_config_overrides(repetition_fixture_overrides(), async {
        let provider = LongTaskProvider::new(Work::RetryBlockedAfterChangedSibling);
        let fixture = TestAgent::new(provider.clone()).await;
        std::fs::write(fixture._work.path().join(REPETITION_DISPATCHES), b"").unwrap();
        let events = fixture.run(fixture.config(), None).await;

        assert_eq!(provider.denial_feedback_calls.load(Ordering::SeqCst), 1);
        assert_eq!(provider.calls.load(Ordering::SeqCst), 7);
        assert!(is_tool_loop(&events));
        assert!(!visible_text(&events).contains(FINISHED));
        assert_eq!(
            std::fs::read(fixture._work.path().join(REPETITION_DISPATCHES)).unwrap(),
            b"....",
            "neither the denied call nor its blocked retry may dispatch"
        );
        let stored = fixture
            .sessions
            .get_session(&fixture.session_id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        for id in ["progress-before", "progress-after"] {
            let response = stored
                .messages()
                .iter()
                .flat_map(|message| &message.content)
                .filter_map(|part| part.as_tool_response())
                .find(|response| response.id == id)
                .unwrap_or_else(|| panic!("missing changed-result sibling {id}"));
            assert!(response
                .tool_result
                .as_ref()
                .is_ok_and(|result| result.is_error != Some(true)));
        }
        assert!(
            response_text(stored.messages(), "progress-before").contains("0 repetition-dispatches")
        );
        assert!(
            response_text(stored.messages(), "progress-after").contains("4 repetition-dispatches")
        );
        assert_ne!(
            response_text(stored.messages(), "progress-before"),
            response_text(stored.messages(), "progress-after"),
            "the allowed sibling must supply actual changed-result history"
        );
        let retry = stored
            .messages()
            .iter()
            .flat_map(|message| &message.content)
            .filter_map(|part| part.as_tool_response())
            .find(|response| response.id == "blocked-retry")
            .expect("the withheld retry must have a persisted tool response");
        assert!(retry
            .tool_result
            .as_ref()
            .is_ok_and(|result| result.is_error == Some(true)));
    })
    .await;
}

#[tokio::test]
#[cfg_attr(not(unix), ignore = "requires the POSIX shell fixture")]
async fn two_repetition_denials_in_one_batch_leave_no_feedback_request() {
    biorouter::config::with_config_overrides(repetition_fixture_overrides(), async {
        let provider = LongTaskProvider::new(Work::TwoDenialsInOneBatch);
        let fixture = TestAgent::new(provider.clone()).await;
        let events = fixture.run(fixture.config(), None).await;

        assert!(is_tool_loop(&events));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        assert_eq!(provider.denial_feedback_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            std::fs::read(fixture._work.path().join(REPETITION_DISPATCHES)).unwrap(),
            b"...."
        );
        let stored = fixture
            .sessions
            .get_session(&fixture.session_id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        for id in ["step-0-4", "step-0-5"] {
            assert!(response_text(stored.messages(), id).contains("automatic repetition guard"));
        }
    })
    .await;
}

struct CurrentUserTurnProvider {
    inner: Arc<LongTaskProvider>,
}

#[async_trait]
impl Provider for CurrentUserTurnProvider {
    async fn complete_with_model(
        &self,
        model_config: &ModelConfig,
        system_prompt: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let start = messages
            .iter()
            .rposition(|message| {
                message.role == rmcp::model::Role::User
                    && message.metadata.user_visible
                    && !message
                        .content
                        .iter()
                        .any(|content| content.as_tool_response().is_some())
            })
            .unwrap_or(0);
        self.inner
            .complete_with_model(model_config, system_prompt, &messages[start..], tools)
            .await
    }

    fn get_model_config(&self) -> ModelConfig {
        self.inner.get_model_config()
    }

    fn metadata() -> ProviderMetadata {
        LongTaskProvider::metadata()
    }

    fn get_name(&self) -> &str {
        self.inner.get_name()
    }
}

#[tokio::test]
#[cfg_attr(not(unix), ignore = "requires the POSIX shell fixture")]
async fn a_new_user_turn_gets_its_own_repetition_feedback_opportunity() {
    biorouter::config::with_config_overrides(repetition_fixture_overrides(), async {
        let first_provider = LongTaskProvider::new(Work::RecoverAfterDenial);
        let fixture = TestAgent::new(first_provider.clone()).await;
        let first = fixture.run(fixture.config(), None).await;

        assert_eq!(
            first_provider.denial_feedback_calls.load(Ordering::SeqCst),
            1
        );
        assert!(visible_text(&first).contains(FINISHED));
        assert!(!is_tool_loop(&first));

        let second_provider = LongTaskProvider::new(Work::RecoverAfterDenial);
        fixture
            .agent
            .update_provider(
                Arc::new(CurrentUserTurnProvider {
                    inner: second_provider.clone(),
                }),
                &fixture.session_id,
            )
            .await
            .unwrap();
        let second = fixture.run(fixture.config(), None).await;

        assert_eq!(
            second_provider.denial_feedback_calls.load(Ordering::SeqCst),
            1
        );
        assert_eq!(second_provider.calls.load(Ordering::SeqCst), 6);
        assert!(visible_text(&second).contains(FINISHED));
        assert!(!is_tool_loop(&second));
        assert_eq!(
            std::fs::read(fixture._work.path().join(REPETITION_DISPATCHES)).unwrap(),
            b"........",
            "each distinct user turn admits four calls and denies its own fifth"
        );
        let stored = fixture
            .sessions
            .get_session(&fixture.session_id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        assert_eq!(
            stored
                .messages()
                .iter()
                .filter(|message| {
                    message.role == rmcp::model::Role::User
                        && message.metadata.user_visible
                        && message.as_concat_text() == "Finish every step of this task."
                })
                .count(),
            2,
            "the two replies must use one persisted session with two genuine user turns"
        );
    })
    .await;
}
