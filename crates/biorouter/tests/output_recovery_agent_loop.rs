//! Issue #87: output-length recovery is internal, bounded for the whole reply,
//! and terminal exhaustion is typed. These tests drive the real Agent::reply
//! loop; a counter-only unit test cannot catch a tool iteration resetting the
//! budget or a hidden continuation leaking onto SSE as a Message frame.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use biorouter::agents::extension::ExtensionConfig;
use biorouter::agents::turn_abort::TurnAbortCode;
use biorouter::agents::types::{RetryConfig, SuccessCheck};
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::{with_config_overrides, BioRouterMode};
use biorouter::conversation::message::{Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::privacy::affiliation::ModelAffiliation;
use biorouter::privacy::ProviderTier;
use biorouter::providers::base::{
    MessageStream, Provider, ProviderMetadata, ProviderStreamItem, ProviderUsage, Usage,
};
use biorouter::providers::errors::{ProviderError, ProviderErrorKind};
use biorouter::providers::utils::map_http_error_to_provider_error;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use futures::StreamExt;
use rmcp::model::{CallToolRequestParams, ErrorCode, ErrorData, Tool};
use rmcp::object;
use tempfile::TempDir;
use tracing::instrument::WithSubscriber;

const CONTINUATION: &str = "Your previous response was cut off because it reached the output length limit (finish_reason=\"length\"). Continue exactly where you left off, and do not repeat what you already wrote.";

#[ctor::ctor]
fn sandbox_global_config() {
    // An outer root wins only if `Paths` will honour it. `var_os(..).is_some()`
    // also accepted a blank one, which `Paths` reads as absent: this ctor then
    // returned, and every test resolved the developer's real directories.
    if biorouter::config::paths::Paths::path_root_override().is_some() {
        return;
    }
    let root = TempDir::new().expect("scratch config root");
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    static ROOT: std::sync::OnceLock<TempDir> = std::sync::OnceLock::new();
    let _ = ROOT.set(root);
}

/// `observe_private_provider_error` captures through a thread-local
/// subscriber, and tracing caches each callsite's interest process-wide from
/// whichever thread reaches it first: a sibling test's agent loop reaching the
/// agent's error diagnostic first, while one capture is the only subscriber
/// registered, caches `never` and that capture reads a real diagnostic as
/// absent. Mechanism and measurement: `biorouter_mcp::test_tracing`.
#[ctor::ctor]
fn register_inert_tracing_dispatchers_before_main() {
    biorouter_mcp::test_tracing::register_inert_dispatchers();
}

#[test]
fn the_inert_tracing_dispatchers_are_registered_before_main() {
    assert!(
        biorouter_mcp::test_tracing::inert_dispatchers_registered(),
        "the ctor no longer registers the inert tracing dispatchers, so a diagnostic \
         capture in this binary can miss an event another test reached first"
    );
}

struct AlternatingStormProvider {
    calls: AtomicUsize,
    responses: AtomicUsize,
}

struct LengthWithToolStormProvider {
    calls: AtomicUsize,
    responses: AtomicUsize,
    visible_each_time: bool,
}

#[async_trait]
impl Provider for LengthWithToolStormProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let raw_call = self.calls.fetch_add(1, Ordering::SeqCst);
        if raw_call == 2 {
            return Err(ProviderError::ServerError(
                "transient 503 during tool-bearing truncation".to_string(),
            ));
        }
        let response_index = self.responses.fetch_add(1, Ordering::SeqCst);
        let mut usage = ProviderUsage::new(
            "mock-model".to_string(),
            Usage::new(Some(10), Some(5), Some(15)),
        );
        usage.finish_reason = Some("length".to_string());

        let mut response = Message::assistant();
        if self.visible_each_time || response_index == 0 {
            response = response.with_text(format!("partial-{response_index}"));
        }
        response = if response_index.is_multiple_of(2) {
            response.with_tool_request(
                format!("malformed-{response_index}"),
                Err(ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "malformed synthetic tool call",
                    None,
                )),
            )
        } else {
            response.with_tool_request(
                format!("call-{response_index}"),
                Ok(CallToolRequestParams {
                    task: None,
                    meta: None,
                    name: "developer__shell".into(),
                    arguments: Some(object!({
                        "command": format!("printf tool-bearing-length-{response_index}")
                    })),
                }),
            )
        };
        Ok((response, usage))
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
        ModelConfig::new_or_fail("mock-model")
    }

    fn metadata() -> ProviderMetadata {
        AlternatingStormProvider::metadata()
    }

    fn get_name(&self) -> &str {
        "output-recovery-mock"
    }
}

impl AlternatingStormProvider {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            responses: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Provider for AlternatingStormProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let raw_call = self.calls.fetch_add(1, Ordering::SeqCst);
        if raw_call == 5 {
            return Err(ProviderError::ServerError(
                "transient 503 during output recovery".to_string(),
            ));
        }
        let call = self.responses.fetch_add(1, Ordering::SeqCst);
        let mut usage = ProviderUsage::new(
            "mock-model".to_string(),
            Usage::new(Some(10), Some(5), Some(15)),
        );
        if call.is_multiple_of(2) {
            usage.finish_reason = Some("length".to_string());
            return Ok((
                Message::assistant().with_text(format!("partial-{call}")),
                usage,
            ));
        }

        usage.finish_reason = Some("tool_calls".to_string());
        if call == 7 {
            return Ok((
                Message::assistant().with_tool_request(
                    format!("call-{call}"),
                    Err(ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "malformed synthetic tool call",
                        None,
                    )),
                ),
                usage,
            ));
        }
        let tool_call = CallToolRequestParams {
            task: None,
            meta: None,
            name: "developer__shell".into(),
            arguments: Some(object!({
                "command": if call == 9 {
                    "exit 7".to_string()
                } else {
                    format!("printf call-{call}")
                }
            })),
        };
        Ok((
            Message::assistant().with_tool_request(format!("call-{call}"), Ok(tool_call)),
            usage,
        ))
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
        ModelConfig::new_or_fail("mock-model")
    }

    fn metadata() -> ProviderMetadata {
        ProviderMetadata {
            name: "output-recovery-mock".to_string(),
            display_name: "Output recovery mock".to_string(),
            description: "test".to_string(),
            default_model: "mock-model".to_string(),
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
        "output-recovery-mock"
    }
}

struct EmptyStormProvider {
    calls: AtomicUsize,
}

struct MalformedSignedToolProvider {
    calls: Mutex<Vec<Vec<Message>>>,
}

struct SignedNaturalRetryProvider {
    calls: Mutex<Vec<Vec<Message>>>,
}

#[async_trait]
impl Provider for SignedNaturalRetryProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let call = {
            let mut calls = self.calls.lock().unwrap();
            let call = calls.len();
            calls.push(messages.to_vec());
            call
        };
        let mut usage = ProviderUsage::new(
            "mock-model".to_string(),
            Usage::new(Some(10), Some(5), Some(15)),
        );
        usage.finish_reason = Some("stop".to_string());
        if call == 0 {
            Ok((
                Message::assistant()
                    .with_thinking("completed private reasoning", "completed-signature")
                    .with_text("signed natural answer"),
                usage,
            ))
        } else {
            Ok((Message::assistant().with_text("retry answer"), usage))
        }
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
        ModelConfig::new_or_fail("mock-model")
    }

    fn metadata() -> ProviderMetadata {
        AlternatingStormProvider::metadata()
    }

    fn get_name(&self) -> &str {
        "aws_bedrock"
    }
}

#[async_trait]
impl Provider for MalformedSignedToolProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.calls.lock().unwrap().push(messages.to_vec());
        let mut usage = ProviderUsage::new(
            "mock-model".to_string(),
            Usage::new(Some(10), Some(5), Some(15)),
        );
        usage.finish_reason = Some("stop".to_string());
        Ok((
            Message::assistant()
                .with_thinking("partial signed reasoning", "signed")
                .with_tool_request(
                    "truncated-tool",
                    Err(ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "tool arguments ended mid-stream",
                        None,
                    )),
                ),
            usage,
        ))
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
        ModelConfig::new_or_fail("mock-model")
    }

    fn metadata() -> ProviderMetadata {
        AlternatingStormProvider::metadata()
    }

    fn get_name(&self) -> &str {
        "aws_bedrock"
    }
}

struct ReasoningContinuationProvider {
    calls: AtomicUsize,
    saw_signed_reasoning: AtomicBool,
    saw_hidden_continuation: AtomicBool,
}

#[async_trait]
impl Provider for ReasoningContinuationProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let mut usage = ProviderUsage::new(
            "mock-model".to_string(),
            Usage::new(Some(10), Some(5), Some(15)),
        );
        if call == 0 {
            usage.finish_reason = Some("length".to_string());
            return Ok((
                Message::assistant().with_content(MessageContent::thinking(
                    "private reasoning",
                    "signed-token",
                )),
                usage,
            ));
        }

        self.saw_signed_reasoning.store(
            messages.iter().any(|message| {
                message.content.iter().any(|content| {
                    content.as_thinking().is_some_and(|thinking| {
                        thinking.thinking == "private reasoning"
                            && thinking.signature == "signed-token"
                    })
                })
            }),
            Ordering::SeqCst,
        );
        self.saw_hidden_continuation.store(
            messages
                .iter()
                .any(|message| message_contains_text(message, CONTINUATION)),
            Ordering::SeqCst,
        );
        usage.finish_reason = Some("stop".to_string());
        Ok((Message::assistant().with_text("finished"), usage))
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
        ModelConfig::new_or_fail("mock-model")
    }

    fn metadata() -> ProviderMetadata {
        AlternatingStormProvider::metadata()
    }

    fn get_name(&self) -> &str {
        "reasoning-continuation-mock"
    }
}

#[async_trait]
impl Provider for EmptyStormProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut usage = ProviderUsage::new(
            "mock-model".to_string(),
            Usage::new(Some(10), Some(5), Some(15)),
        );
        usage.finish_reason = Some("length".to_string());
        Ok((Message::assistant().with_text(""), usage))
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
        ModelConfig::new_or_fail("mock-model")
    }

    fn metadata() -> ProviderMetadata {
        AlternatingStormProvider::metadata()
    }

    fn get_name(&self) -> &str {
        "empty-output-recovery-mock"
    }
}

async fn test_agent(
    provider: Arc<dyn Provider>,
) -> (
    Agent,
    Arc<SessionManager>,
    String,
    TempDir,
    TempDir,
    TempDir,
) {
    let working_dir = TempDir::new().unwrap();
    let data_dir = TempDir::new().unwrap();
    let permission_dir = TempDir::new().unwrap();
    let session_manager = Arc::new(SessionManager::new(data_dir.path().to_path_buf()));
    let agent = Agent::with_config(AgentConfig::new(
        session_manager.clone(),
        Arc::new(PermissionManager::new(permission_dir.path().to_path_buf())),
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
    let session = session_manager
        .create_session(
            working_dir.path().to_path_buf(),
            "output-recovery".to_string(),
            SessionType::Hidden,
        )
        .await
        .unwrap();
    agent.update_provider(provider, &session.id).await.unwrap();
    (
        agent,
        session_manager,
        session.id,
        working_dir,
        data_dir,
        permission_dir,
    )
}

fn text(message: &Message) -> Option<&str> {
    message.content.iter().find_map(|content| match content {
        MessageContent::Text(text) => Some(text.text.as_str()),
        _ => None,
    })
}

fn message_contains_text(message: &Message, expected: &str) -> bool {
    message
        .content
        .iter()
        .any(|content| matches!(content, MessageContent::Text(text) if text.text == expected))
}

#[derive(Clone, Default)]
struct PrivateProviderLogCapture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for PrivateProviderLogCapture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The `<info-msg>` block the agent adds to every request carries the clock to
/// the minute, so two attempts a second apart can differ only there.
fn without_per_request_context(text: &str) -> String {
    use biorouter::agents::extension_manager::{MOIM_CLOSE_TAG as CLOSE, MOIM_OPEN_TAG as OPEN};
    text.split_once(OPEN)
        .and_then(|(before, rest)| {
            rest.split_once(CLOSE)
                .map(|(_, after)| format!("{before}{after}"))
        })
        .unwrap_or_else(|| text.to_string())
}

struct PrivateErrorProvider {
    calls: AtomicUsize,
    recover: bool,
    sentinel: &'static str,
    /// The agent-visible text of every request, in call order.
    sent: Mutex<Vec<Vec<String>>>,
}

#[async_trait]
impl Provider for PrivateErrorProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.sent.lock().unwrap().push(
            messages
                .iter()
                .filter(|message| message.is_agent_visible())
                .map(|message| {
                    format!(
                        "{:?}: {}",
                        message.role,
                        without_per_request_context(&message.as_concat_text())
                    )
                })
                .collect(),
        );
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            let (status, message) = if self.recover {
                (
                    reqwest::StatusCode::SERVICE_UNAVAILABLE,
                    format!("Synthetic upstream unavailable: {}", self.sentinel),
                )
            } else {
                (
                    reqwest::StatusCode::BAD_REQUEST,
                    format!("Invalid 'tools': array too long. {}", self.sentinel),
                )
            };
            return Err(map_http_error_to_provider_error(
                status,
                Some(serde_json::json!({"error": {"message": message}})),
            ));
        }
        let mut usage = ProviderUsage::new(
            "mock-model".to_string(),
            Usage::new(Some(10), Some(5), Some(15)),
        );
        usage.finish_reason = Some("stop".to_string());
        Ok((
            Message::assistant().with_text("Synthetic provider recovered."),
            usage,
        ))
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
        ModelConfig::new_or_fail("mock-model")
    }

    fn metadata() -> ProviderMetadata {
        ProviderMetadata {
            name: "private-provider-error-logging-mock".to_string(),
            tier: ProviderTier::Private,
            ..AlternatingStormProvider::metadata()
        }
    }

    fn get_name(&self) -> &str {
        "private-provider-error-logging-mock"
    }

    fn tier(&self) -> ProviderTier {
        ProviderTier::Private
    }

    fn affiliation(&self) -> Option<ModelAffiliation> {
        Some(ModelAffiliation::Local)
    }
}

#[derive(Default)]
struct PrivateProviderReplyObservation {
    visible_text: Vec<String>,
    aborts: Vec<(TurnAbortCode, String)>,
    history_replacements: usize,
    tool_messages: usize,
}

async fn observe_private_provider_error(
    provider: Arc<PrivateErrorProvider>,
) -> (PrivateProviderReplyObservation, String) {
    let capture = PrivateProviderLogCapture::default();
    let writer = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(true)
        .with_level(true)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let overrides = HashMap::from([
        (
            "BIOROUTER_MISTAKE_STREAK_DETECTION".to_string(),
            "true".to_string(),
        ),
        (
            "BIOROUTER_PROVIDER_ERROR_RETRIES".to_string(),
            "1".to_string(),
        ),
    ]);
    let observation = with_config_overrides(
        overrides,
        async {
            let (agent, _sessions, session_id, _work, _data, _permissions) =
                test_agent(provider).await;
            tokio::time::timeout(Duration::from_secs(10), async {
                let stream = agent
                    .reply(
                        Message::user().with_text("Complete this synthetic local provider check."),
                        SessionConfig {
                            id: session_id,
                            schedule_id: None,
                            max_turns: Some(4),
                            max_tool_calls: None,
                            budget: None,
                            retry_config: None,
                            reasoning_effort: None,
                        },
                        None,
                    )
                    .await
                    .unwrap();
                tokio::pin!(stream);
                let mut observation = PrivateProviderReplyObservation::default();
                while let Some(event) = stream.next().await {
                    match event.unwrap() {
                        AgentEvent::Message(message) => {
                            for content in &message.content {
                                if matches!(
                                    content,
                                    MessageContent::ToolRequest(_)
                                        | MessageContent::ToolResponse(_)
                                ) {
                                    observation.tool_messages += 1;
                                }
                                if message.is_user_visible() {
                                    match content {
                                        MessageContent::Text(text) => {
                                            observation.visible_text.push(text.text.clone())
                                        }
                                        MessageContent::SystemNotification(notification) => {
                                            observation.visible_text.push(notification.msg.clone())
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        AgentEvent::TurnAborted { code, message } => {
                            observation.aborts.push((code, message))
                        }
                        AgentEvent::HistoryReplaced(_) => observation.history_replacements += 1,
                        _ => {}
                    }
                }
                observation
            })
            .await
            .expect("synthetic provider reply should settle without network or tools")
        }
        .with_subscriber(subscriber),
    )
    .await;
    let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    (observation, logs)
}

fn has_agent_log_level(logs: &str, level: &str) -> bool {
    logs.lines()
        .any(|line| line.contains(level) && line.contains("biorouter::agents::agent:"))
}

#[tokio::test]
async fn private_provider_error_logging_invalid_request_stops_without_diagnostic_payload() {
    const SENTINEL: &str = "PRIVATE_UPSTREAM_INVALID_TOOLS_7e54b3";
    let provider = Arc::new(PrivateErrorProvider {
        calls: AtomicUsize::new(0),
        recover: false,
        sentinel: SENTINEL,
        sent: Mutex::default(),
    });
    assert_eq!(provider.tier(), ProviderTier::Private);
    let (observation, logs) = observe_private_provider_error(provider.clone()).await;

    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(observation.history_replacements, 0);
    assert_eq!(observation.tool_messages, 0);
    assert_eq!(observation.aborts.len(), 1);
    assert_eq!(
        observation.aborts[0].0,
        TurnAbortCode::ProviderFailure {
            kind: ProviderErrorKind::InvalidRequest,
        }
    );
    assert!(observation.aborts[0].1.contains(SENTINEL));
    assert!(observation
        .visible_text
        .iter()
        .any(|text| { text.contains("Ran into this error:") && text.contains(SENTINEL) }));
    assert!(!observation
        .visible_text
        .iter()
        .any(|text| text.contains("Trying again (")));
    assert!(
        has_agent_log_level(&logs, "ERROR"),
        "the real Agent error diagnostic must be captured"
    );
    assert!(
        !logs.contains(SENTINEL),
        "private upstream error leaked into diagnostic tracing"
    );
    assert!(logs.lines().any(|line| {
        has_agent_log_level(line, "ERROR") && line.contains("error_type=\"request\"")
    }));
}

#[tokio::test]
async fn private_provider_error_logging_retry_keeps_details_out_of_diagnostics() {
    const SENTINEL: &str = "PRIVATE_UPSTREAM_TRANSIENT_95f02c";
    let provider = Arc::new(PrivateErrorProvider {
        calls: AtomicUsize::new(0),
        recover: true,
        sentinel: SENTINEL,
        sent: Mutex::default(),
    });
    assert_eq!(provider.tier(), ProviderTier::Private);
    let (observation, logs) = observe_private_provider_error(provider.clone()).await;

    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    // Nothing reached the conversation before the failure, so the retry is the
    // same request with no note: a note after the user's message used to be
    // what the model answered, instead of the user (measured on Gemma 4 E4B).
    let sent = provider.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0], sent[1], "the retry must re-send the same request");
    assert!(
        sent[1]
            .iter()
            .all(|row| !row.contains("biorouter-loop-guard")),
        "{sent:?}"
    );
    assert_eq!(observation.history_replacements, 0);
    assert_eq!(observation.tool_messages, 0);
    assert!(observation.aborts.is_empty());
    assert!(observation
        .visible_text
        .iter()
        .any(|text| { text.contains("Trying again (1/1)") && text.contains(SENTINEL) }));
    assert!(observation
        .visible_text
        .iter()
        .any(|text| text == "Synthetic provider recovered."));
    assert!(
        has_agent_log_level(&logs, "ERROR"),
        "the real Agent error diagnostic must be captured"
    );
    assert!(
        has_agent_log_level(&logs, "WARN"),
        "the real Agent retry diagnostic must be captured"
    );
    assert!(
        !logs.contains(SENTINEL),
        "private upstream error leaked into diagnostic tracing"
    );
    assert!(logs.lines().any(|line| {
        has_agent_log_level(line, "ERROR") && line.contains("error_type=\"server\"")
    }));
    assert!(logs.lines().any(|line| {
        has_agent_log_level(line, "WARN")
            && line.contains("error_type=\"server\"")
            && line.contains("attempt=1")
            && line.contains("limit=1")
    }));
}

#[tokio::test]
async fn tool_calls_cannot_reset_total_recovery_and_hidden_rows_stay_off_message_sse() {
    let provider = Arc::new(AlternatingStormProvider::new());
    let (agent, session_manager, session_id, _work, _data, _permissions) =
        test_agent(provider.clone()).await;
    let config = SessionConfig {
        id: session_id.clone(),
        schedule_id: None,
        max_turns: Some(50),
        max_tool_calls: Some(30),
        budget: None,
        retry_config: None,
        reasoning_effort: None,
    };
    let stream = agent
        .reply(Message::user().with_text("work"), config, None)
        .await
        .unwrap();
    tokio::pin!(stream);

    let mut abort = None;
    let mut visible_continuations = 0;
    let mut persisted_hidden_continuations = 0;
    let mut saw_terminal_message = false;
    let mut saw_partial_output = false;
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            AgentEvent::Message(message) => {
                visible_continuations += usize::from(text(&message) == Some(CONTINUATION));
                saw_partial_output |= text(&message) == Some("partial-0");
                saw_terminal_message |= text(&message)
                    .is_some_and(|value| value.contains("stopped automatic continuation"));
            }
            AgentEvent::MessagesPersisted(messages) => {
                persisted_hidden_continuations += messages
                    .iter()
                    .filter(|message| !message.user_visible)
                    .count();
            }
            AgentEvent::TurnAborted { code, message } => abort = Some((code, message)),
            _ => {}
        }
    }

    assert_eq!(
        provider.calls(),
        26,
        "12 continuations plus 12 tool iterations, one transient provider retry, then exhaustion"
    );
    assert_eq!(
        visible_continuations, 0,
        "internal prompts must not be Message frames"
    );
    assert!(persisted_hidden_continuations >= 12);
    assert!(
        saw_partial_output,
        "partial assistant work remains readable"
    );
    assert!(saw_terminal_message);
    assert!(matches!(
        abort.as_ref().map(|(code, _)| code),
        Some(TurnAbortCode::OutputRecoveryExhausted {
            continuations: 12,
            zero_progress: false,
        })
    ));

    let stored = session_manager
        .get_session(&session_id, true)
        .await
        .unwrap()
        .conversation
        .unwrap();
    let hidden = stored
        .messages()
        .iter()
        .filter(|message| text(message) == Some(CONTINUATION))
        .collect::<Vec<_>>();
    assert_eq!(hidden.len(), 12);
    assert!(hidden
        .iter()
        .all(|message| message.is_agent_visible() && !message.is_user_visible()));
}

#[tokio::test]
async fn length_with_a_tool_on_every_response_still_spends_the_total_budget() {
    let provider = Arc::new(LengthWithToolStormProvider {
        calls: AtomicUsize::new(0),
        responses: AtomicUsize::new(0),
        visible_each_time: true,
    });
    let (agent, _session_manager, session_id, _work, _data, _permissions) =
        test_agent(provider.clone()).await;
    let stream = agent
        .reply(
            Message::user().with_text("work"),
            SessionConfig {
                id: session_id,
                schedule_id: None,
                max_turns: Some(30),
                max_tool_calls: Some(30),
                budget: None,
                retry_config: None,
                reasoning_effort: None,
            },
            None,
        )
        .await
        .unwrap();
    tokio::pin!(stream);
    let mut abort = None;
    while let Some(event) = stream.next().await {
        if let AgentEvent::TurnAborted { code, .. } = event.unwrap() {
            abort = Some(code);
        }
    }

    assert_eq!(provider.calls.load(Ordering::SeqCst), 14);
    assert!(matches!(
        abort,
        Some(TurnAbortCode::OutputRecoveryExhausted {
            continuations: 12,
            zero_progress: false,
        })
    ));
}

#[tokio::test]
async fn tool_bearing_zero_progress_budget_preserves_an_earlier_partial_answer() {
    let provider = Arc::new(LengthWithToolStormProvider {
        calls: AtomicUsize::new(0),
        responses: AtomicUsize::new(0),
        visible_each_time: false,
    });
    let (agent, _session_manager, session_id, _work, _data, _permissions) =
        test_agent(provider.clone()).await;
    let stream = agent
        .reply(
            Message::user().with_text("work"),
            SessionConfig {
                id: session_id,
                schedule_id: None,
                max_turns: Some(20),
                max_tool_calls: Some(20),
                budget: None,
                retry_config: None,
                reasoning_effort: None,
            },
            None,
        )
        .await
        .unwrap();
    tokio::pin!(stream);
    let mut abort = None;
    let mut terminal = String::new();
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            AgentEvent::Message(message) => {
                if let Some(text) =
                    text(&message).filter(|text| text.contains("stopped automatic continuation"))
                {
                    terminal = text.to_string();
                }
            }
            AgentEvent::TurnAborted { code, .. } => abort = Some(code),
            _ => {}
        }
    }

    assert_eq!(provider.calls.load(Ordering::SeqCst), 6);
    assert!(matches!(
        abort,
        Some(TurnAbortCode::OutputRecoveryExhausted {
            continuations: 4,
            zero_progress: true,
        })
    ));
    assert!(terminal.contains("partial response above has been preserved"));
    assert!(!terminal.contains("No partial answer was available"));
}

#[tokio::test]
async fn repeated_reasoning_only_shape_uses_the_tighter_zero_progress_budget() {
    let provider = Arc::new(EmptyStormProvider {
        calls: AtomicUsize::new(0),
    });
    let (agent, _session_manager, session_id, _work, _data, _permissions) =
        test_agent(provider.clone()).await;
    let stream = agent
        .reply(
            Message::user().with_text("work"),
            SessionConfig {
                id: session_id,
                schedule_id: None,
                max_turns: Some(20),
                max_tool_calls: None,
                budget: None,
                retry_config: None,
                reasoning_effort: None,
            },
            None,
        )
        .await
        .unwrap();
    tokio::pin!(stream);
    let mut abort = None;
    while let Some(event) = stream.next().await {
        if let AgentEvent::TurnAborted { code, .. } = event.unwrap() {
            abort = Some(code);
        }
    }

    assert_eq!(provider.calls.load(Ordering::SeqCst), 4);
    assert!(matches!(
        abort,
        Some(TurnAbortCode::OutputRecoveryExhausted {
            continuations: 3,
            zero_progress: true,
        })
    ));
}

#[tokio::test]
async fn malformed_signed_tool_block_aborts_typed_and_is_never_replayed() {
    let provider = Arc::new(MalformedSignedToolProvider {
        calls: Mutex::new(Vec::new()),
    });
    let (agent, _session_manager, session_id, _work, _data, _permissions) =
        test_agent(provider.clone()).await;
    for expected_calls in [1, 2] {
        let stream = agent
            .reply(
                Message::user().with_text("work"),
                SessionConfig {
                    id: session_id.clone(),
                    schedule_id: None,
                    max_turns: Some(4),
                    max_tool_calls: None,
                    budget: None,
                    retry_config: None,
                    reasoning_effort: None,
                },
                None,
            )
            .await
            .unwrap();
        tokio::pin!(stream);
        let mut abort = None;
        while let Some(event) = stream.next().await {
            if let AgentEvent::TurnAborted { code, .. } = event.unwrap() {
                abort = Some(code);
            }
        }
        assert_eq!(provider.calls.lock().unwrap().len(), expected_calls);
        assert_eq!(abort, Some(TurnAbortCode::SignedReplayInvalidated));
    }
    let calls = provider.calls.lock().unwrap();
    assert!(!calls[1]
        .iter()
        .flat_map(|message| &message.content)
        .any(|content| matches!(
            content,
            MessageContent::Thinking(_) | MessageContent::RedactedThinking(_)
        )));
    assert!(!calls[1].iter().flat_map(|message| &message.content).any(
        |content| matches!(content, MessageContent::ToolRequest(request) if request.tool_call.is_err())
    ));
}

#[tokio::test]
async fn signed_natural_response_is_removed_before_retry_reset_provider_call() {
    let provider = Arc::new(SignedNaturalRetryProvider {
        calls: Mutex::new(Vec::new()),
    });
    let (agent, _session_manager, session_id, work, _data, _permissions) =
        test_agent(provider.clone()).await;
    let stream = agent
        .reply(
            Message::user().with_text("work"),
            SessionConfig {
                id: session_id,
                schedule_id: None,
                max_turns: Some(4),
                max_tool_calls: None,
                budget: None,
                retry_config: Some(RetryConfig {
                    max_retries: 1,
                    checks: vec![SuccessCheck::FileExists {
                        path: work
                            .path()
                            .join("retry-check-that-does-not-exist")
                            .to_string_lossy()
                            .into_owned(),
                    }],
                    on_failure: None,
                    timeout_seconds: Some(5),
                    on_failure_timeout_seconds: Some(5),
                }),
                reasoning_effort: None,
            },
            None,
        )
        .await
        .unwrap();
    tokio::pin!(stream);
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(event) = stream.next().await {
            event.unwrap();
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "retry stream did not terminate; provider calls: {}",
            provider.calls.lock().unwrap().len()
        )
    });

    let calls = provider.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(!calls[1]
        .iter()
        .flat_map(|message| &message.content)
        .any(|content| matches!(
            content,
            MessageContent::Thinking(_) | MessageContent::RedactedThinking(_)
        )));
    let retry_input = calls[1]
        .iter()
        .map(Message::as_concat_text)
        .collect::<String>();
    assert!(!retry_input.contains("signed natural answer"));
    assert!(retry_input.contains("work"));
    assert!(retry_input.contains("<info-msg>"));
}

#[tokio::test]
async fn reasoning_only_max_tokens_replays_signed_reasoning_once_and_finishes() {
    let provider = Arc::new(ReasoningContinuationProvider {
        calls: AtomicUsize::new(0),
        saw_signed_reasoning: AtomicBool::new(false),
        saw_hidden_continuation: AtomicBool::new(false),
    });
    let (agent, session_manager, session_id, _work, _data, _permissions) =
        test_agent(provider.clone()).await;
    let stream = agent
        .reply(
            Message::user().with_text("work"),
            SessionConfig {
                id: session_id.clone(),
                schedule_id: None,
                max_turns: Some(8),
                max_tool_calls: None,
                budget: None,
                retry_config: None,
                reasoning_effort: None,
            },
            None,
        )
        .await
        .unwrap();
    tokio::pin!(stream);
    let mut visible_continuations = 0;
    let mut final_text = false;
    while let Some(event) = stream.next().await {
        if let AgentEvent::Message(message) = event.unwrap() {
            visible_continuations += usize::from(text(&message) == Some(CONTINUATION));
            final_text |= text(&message) == Some("finished");
        }
    }

    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    assert!(provider.saw_signed_reasoning.load(Ordering::SeqCst));
    assert!(provider.saw_hidden_continuation.load(Ordering::SeqCst));
    assert_eq!(visible_continuations, 0);
    assert!(final_text);
    let counts = session_manager.get_token_counts(&session_id).await.unwrap();
    assert_eq!(counts.accumulated_output_tokens, Some(10));
}

/// What a tool call written as text looks like when llama-server's shared-cache
/// collision cuts it partway (live, against a scripted llama-server).
const CUT_CALL: &str = r#"{"name": "developer__shell", "arguments": {"comm"#;
const CUT_ANSWER: &str = "The command printed hi.";

/// A streaming provider whose first `failures` calls stream `first` in small
/// chunks and then fail with llama-server's in-stream shared-cache error, a
/// retryable server fault. With `recover`, the chunks go through the local
/// providers' text recovery, as on Llama Server and Ollama, which holds back
/// text that could still be a tool call.
struct CutStreamProvider {
    calls: AtomicUsize,
    failures: usize,
    first: &'static str,
    recover: bool,
    /// The agent-visible text of every request, `<info-msg>` block stripped.
    sent: Mutex<Vec<Vec<String>>>,
    /// Whether each request carried the `<info-msg>` block.
    with_block: Mutex<Vec<bool>>,
}

impl CutStreamProvider {
    fn new(first: &'static str, failures: usize, recover: bool) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            failures,
            first,
            recover,
            sent: Mutex::default(),
            with_block: Mutex::default(),
        }
    }

    fn record(&self, messages: &[Message]) {
        let visible: Vec<&Message> = messages
            .iter()
            .filter(|message| message.is_agent_visible())
            .collect();
        self.with_block
            .lock()
            .unwrap()
            .push(visible.iter().any(|message| {
                message
                    .as_concat_text()
                    .contains(biorouter::agents::extension_manager::MOIM_OPEN_TAG)
            }));
        self.sent.lock().unwrap().push(
            visible
                .iter()
                .map(|message| {
                    format!(
                        "{:?}: {}",
                        message.role,
                        without_per_request_context(&message.as_concat_text()).trim()
                    )
                })
                .collect(),
        );
    }
}

#[async_trait]
impl Provider for CutStreamProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        Ok((
            Message::assistant().with_text(CUT_ANSWER),
            ProviderUsage::new("mock-model".to_string(), Usage::default()),
        ))
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

    async fn stream(
        &self,
        _system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        use biorouter::providers::formats::text_tool_calls::{
            recover_leaked_tool_calls, RecoveryContext,
        };
        self.record(messages);
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let id = format!("reply-{call}");
        let items: Vec<Result<ProviderStreamItem, ProviderError>> = if call < self.failures {
            let mut items: Vec<Result<ProviderStreamItem, ProviderError>> = self
                .first
                .as_bytes()
                .chunks(5)
                .map(|chunk| {
                    let chunk = String::from_utf8(chunk.to_vec()).unwrap();
                    Ok((
                        Some(Message::assistant().with_id(id.clone()).with_text(chunk)),
                        None,
                        None,
                    ))
                })
                .collect();
            items.push(Err(ProviderError::ServerError(
                "Context size has been exceeded.".to_string(),
            )));
            items
        } else {
            let mut usage = ProviderUsage::new(
                "mock-model".to_string(),
                Usage::new(Some(10), Some(5), Some(15)),
            );
            usage.finish_reason = Some("stop".to_string());
            vec![Ok((
                Some(Message::assistant().with_id(id).with_text(CUT_ANSWER)),
                Some(usage),
                None,
            ))]
        };
        let stream: MessageStream = Box::pin(futures::stream::iter(items));
        Ok(if self.recover {
            recover_leaked_tool_calls(
                stream,
                RecoveryContext {
                    tools: tools.to_vec(),
                    provider: "llamacpp",
                    model: "mock-model".to_string(),
                },
            )
        } else {
            stream
        })
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("mock-model")
    }

    fn metadata() -> ProviderMetadata {
        AlternatingStormProvider::metadata()
    }

    fn get_name(&self) -> &str {
        "output-recovery-mock"
    }
}

#[derive(Default)]
struct CutStreamReply {
    /// User-visible text and notices, in the order they were yielded.
    shown: Vec<String>,
    /// The stored conversation's rows, as `"{role}: {text}"`.
    stored: Vec<String>,
    /// Whether any stored text item still carries a `_meta`.
    stored_meta: bool,
    aborted: bool,
}

/// One reply to "Run echo hi in the shell." against `provider`, with one
/// provider-error retry, as shipped.
async fn reply_through_a_cut_stream(provider: Arc<CutStreamProvider>) -> CutStreamReply {
    let overrides = HashMap::from([
        (
            "BIOROUTER_MISTAKE_STREAK_DETECTION".to_string(),
            "true".to_string(),
        ),
        (
            "BIOROUTER_PROVIDER_ERROR_RETRIES".to_string(),
            "1".to_string(),
        ),
    ]);
    with_config_overrides(overrides, async {
        let (agent, sessions, session_id, _work, _data, _permissions) = test_agent(provider).await;
        let mut reply = CutStreamReply::default();
        tokio::time::timeout(Duration::from_secs(10), async {
            let stream = agent
                .reply(
                    Message::user().with_text("Run echo hi in the shell."),
                    SessionConfig {
                        id: session_id.clone(),
                        schedule_id: None,
                        max_turns: Some(4),
                        max_tool_calls: None,
                        budget: None,
                        retry_config: None,
                        reasoning_effort: None,
                    },
                    None,
                )
                .await
                .unwrap();
            tokio::pin!(stream);
            while let Some(event) = stream.next().await {
                match event.unwrap() {
                    AgentEvent::Message(message) if message.is_user_visible() => {
                        for content in &message.content {
                            match content {
                                MessageContent::Text(text) => reply.shown.push(text.text.clone()),
                                MessageContent::SystemNotification(notice) => {
                                    reply.shown.push(notice.msg.clone())
                                }
                                _ => {}
                            }
                        }
                    }
                    AgentEvent::TurnAborted { .. } => reply.aborted = true,
                    _ => {}
                }
            }
        })
        .await
        .expect("the reply settles");
        let stored = sessions
            .get_session(&session_id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        for message in stored.messages() {
            reply
                .stored
                .push(format!("{:?}: {}", message.role, message.as_concat_text()));
            reply.stored_meta |= message.content.iter().any(
                |content| matches!(content, MessageContent::Text(text) if text.meta.is_some()),
            );
        }
        reply
    })
    .await
}

/// Live, against a scripted llama-server: the fragment was shown and saved as a
/// reply, the retry was told to "continue the reply from exactly where it
/// stopped", and the model wrote the rest of the JSON as text. Nobody had seen
/// the fragment (it was held back as a possible call), so a retried error now
/// drops it, and the retry is the request that was sent.
#[tokio::test]
async fn a_call_cut_by_a_retried_error_is_neither_shown_nor_kept() {
    let provider = Arc::new(CutStreamProvider::new(CUT_CALL, 1, true));
    let reply = reply_through_a_cut_stream(provider.clone()).await;

    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    let sent = provider.sent.lock().unwrap().clone();
    assert_eq!(sent[0], sent[1], "the retry re-sends the same request");
    assert!(
        sent[1]
            .iter()
            .all(|row| !row.contains("biorouter-loop-guard")),
        "{sent:?}"
    );
    assert!(
        reply
            .shown
            .iter()
            .all(|text| !text.contains("developer__shell")),
        "{:?}",
        reply.shown
    );
    assert!(reply
        .shown
        .iter()
        .any(|text| text.contains("Trying again (1/1)")));
    assert!(reply.shown.iter().any(|text| text == CUT_ANSWER));
    assert!(!reply.aborted);
    assert!(
        reply
            .stored
            .iter()
            .all(|row| !row.contains("developer__shell") && !row.contains("biorouter-loop-guard")),
        "{:?}",
        reply.stored
    );
}

/// When the error ends the turn, the person still sees what the model wrote,
/// ahead of the notice, and it is stored as plain text.
#[tokio::test]
async fn a_call_cut_by_an_error_that_ends_the_turn_is_shown_before_the_notice() {
    let provider = Arc::new(CutStreamProvider::new(CUT_CALL, 2, true));
    let reply = reply_through_a_cut_stream(provider.clone()).await;

    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    let sent = provider.sent.lock().unwrap().clone();
    assert_eq!(sent[0], sent[1], "the first retry was still a re-send");
    assert!(reply.aborted);
    let fragment = reply
        .shown
        .iter()
        .position(|text| text == CUT_CALL)
        .unwrap_or_else(|| panic!("the fragment is shown: {:?}", reply.shown));
    let notice = reply
        .shown
        .iter()
        .position(|text| text.starts_with("Ran into this error"))
        .unwrap_or_else(|| panic!("the stop notice: {:?}", reply.shown));
    assert!(fragment < notice, "{:?}", reply.shown);
    assert_eq!(
        reply
            .shown
            .iter()
            .filter(|text| text.contains("developer__shell"))
            .count(),
        1,
        "the first attempt's fragment was dropped: {:?}",
        reply.shown
    );
    assert!(
        reply
            .stored
            .iter()
            .any(|row| row == &format!("Assistant: {CUT_CALL}")),
        "{:?}",
        reply.stored
    );
    assert!(!reply.stored_meta, "the mark is not stored");
}

/// Live, against a scripted llama-server: a first call that streamed only
/// "\n\n" was told to continue a reply it had not begun, and the retry went
/// without the `<info-msg>` block because trimming that row counted as an
/// unexpected issue. Through a provider without text recovery, so the
/// whitespace reaches the conversation.
#[tokio::test]
async fn a_whitespace_partial_is_a_resend_that_keeps_the_block() {
    let provider = Arc::new(CutStreamProvider::new("\n\n", 1, false));
    let reply = reply_through_a_cut_stream(provider.clone()).await;

    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    let sent = provider.sent.lock().unwrap().clone();
    assert!(
        sent[1]
            .iter()
            .all(|row| !row.contains("biorouter-loop-guard")),
        "{sent:?}"
    );
    assert_eq!(sent[0], sent[1], "the retry re-sends the same request");
    assert_eq!(
        provider.with_block.lock().unwrap().as_slice(),
        [true, true],
        "both requests carry the block"
    );
    assert!(reply.shown.iter().any(|text| text == CUT_ANSWER));
}

/// The retry after a cut-off reply that ends in a space keeps the block.
#[tokio::test]
async fn a_partial_reply_ending_in_whitespace_keeps_the_block() {
    let provider = Arc::new(CutStreamProvider::new("The quick brown fox ", 1, false));
    let reply = reply_through_a_cut_stream(provider.clone()).await;

    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    let sent = provider.sent.lock().unwrap().clone();
    assert!(
        sent[1]
            .iter()
            .any(|row| row.contains("biorouter-loop-guard")),
        "a cut-off reply is still continued: {sent:?}"
    );
    assert_eq!(
        provider.with_block.lock().unwrap().as_slice(),
        [true, true],
        "both requests carry the block"
    );
    assert!(reply.shown.iter().any(|text| text == CUT_ANSWER));
}
