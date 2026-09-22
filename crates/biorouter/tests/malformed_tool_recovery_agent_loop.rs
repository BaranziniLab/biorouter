//! Malformed tool calls must teach the next model request what failed without
//! inventing executable arguments or replaying the provider's raw input.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use biorouter::agents::turn_abort::TurnAbortCode;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use futures::StreamExt;
use rmcp::model::{ErrorCode, ErrorData, Tool};
use serde_json::json;
use tempfile::TempDir;

const RAW_SECRET: &str = "MALFORMED_CALL_RAW_SECRET_MUST_NOT_REPLAY";
const INVALID_FEEDBACK: &str = "A tool call was not executed because its arguments were invalid. Emit a new tool call with valid JSON object arguments.";
const INCOMPLETE_FEEDBACK: &str = "A tool call was not executed because its response stream did not complete. Emit a new, complete tool call; do not assume the earlier call ran.";
const UNKNOWN_FEEDBACK: &str = "A tool call could not be accepted and was not executed. Emit a new tool call with complete, valid JSON object arguments.";

#[ctor::ctor]
fn sandbox_config() {
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

struct MalformedProvider {
    failure_kind: Option<&'static str>,
    signed: bool,
    calls: Mutex<Vec<Vec<Message>>>,
}

#[async_trait]
impl Provider for MalformedProvider {
    async fn complete(
        &self,
        _system: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let first = {
            let mut calls = self.calls.lock().unwrap();
            calls.push(messages.to_vec());
            calls.len() == 1
        };
        let mut usage = ProviderUsage::new(
            "mock-model".to_string(),
            Usage::new(Some(10), Some(5), Some(15)),
        );
        usage.finish_reason = Some(if first { "tool_calls" } else { "stop" }.to_string());
        if !first {
            return Ok((Message::assistant().with_text("recovered"), usage));
        }
        let data = self.failure_kind.map(|kind| {
            json!({
                "biorouterToolCallFailure": kind,
                "raw_arguments": RAW_SECRET,
            })
        });
        let code = if self.failure_kind == Some("incomplete_stream") {
            ErrorCode::INTERNAL_ERROR
        } else {
            ErrorCode::INVALID_PARAMS
        };
        let mut response = Message::assistant()
            .with_tool_request("failed-call", Err(ErrorData::new(code, RAW_SECRET, data)));
        if self.signed {
            response = response.with_thinking("signed reasoning", "signature");
        }
        Ok((response, usage))
    }

    async fn complete_with_model(
        &self,
        _model: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.complete(system, messages, tools).await
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("mock-model")
    }

    fn metadata() -> ProviderMetadata {
        ProviderMetadata::empty()
    }

    fn get_name(&self) -> &str {
        if self.signed {
            "aws_bedrock"
        } else {
            "malformed-recovery-test"
        }
    }
}

async fn check_recovery(failure_kind: Option<&'static str>, expected: &str, signed: bool) {
    let work = TempDir::new().unwrap();
    let data = TempDir::new().unwrap();
    let permissions = TempDir::new().unwrap();
    let manager = Arc::new(SessionManager::new(data.path().to_path_buf()));
    let agent = Agent::with_config(AgentConfig::new(
        manager.clone(),
        Arc::new(PermissionManager::new(permissions.path().to_path_buf())),
        None,
        BioRouterMode::Auto,
    ));
    let session = manager
        .create_session(
            work.path().to_path_buf(),
            "malformed-call-recovery".to_string(),
            SessionType::Hidden,
        )
        .await
        .unwrap();
    let provider = Arc::new(MalformedProvider {
        failure_kind,
        signed,
        calls: Mutex::new(Vec::new()),
    });
    agent
        .update_provider(provider.clone(), &session.id)
        .await
        .unwrap();
    let stream = agent
        .reply(
            Message::user().with_text("Complete the task."),
            SessionConfig {
                id: session.id.clone(),
                schedule_id: None,
                max_turns: Some(3),
                max_tool_calls: Some(3),
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
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                AgentEvent::Message(message) => {
                    assert!(
                        !message.content.iter().any(|content| matches!(
                            content,
                            MessageContent::ToolResponse(_)
                                | MessageContent::FrontendToolRequest(_)
                        )),
                        "a malformed request must never dispatch or produce a tool result"
                    );
                    assert!(
                        !message.content.iter().any(|content| matches!(
                            content,
                            MessageContent::ToolRequest(request) if request.tool_call.is_ok()
                        )),
                        "repair must never fabricate a callable request"
                    );
                    assert!(
                        !message.as_concat_text().contains(expected),
                        "repair feedback stays off user Message events"
                    );
                }
                AgentEvent::TurnAborted { code, .. } => abort = Some(code),
                _ => {}
            }
        }
    })
    .await
    .expect("bounded malformed-call recovery");

    let stored = manager
        .get_session(&session.id, true)
        .await
        .unwrap()
        .conversation
        .unwrap();
    let calls = provider.calls.lock().unwrap();
    if signed {
        assert_eq!(
            calls.len(),
            1,
            "signed malformed content must not be retried"
        );
        assert_eq!(abort, Some(TurnAbortCode::SignedReplayInvalidated));
        assert!(!stored
            .iter()
            .any(|message| message.as_concat_text().contains(expected)));
        return;
    }
    assert!(abort.is_none());
    assert_eq!(
        calls.len(),
        2,
        "the first failed call should immediately recover"
    );
    let next_context = &calls[1];
    assert!(
        next_context
            .iter()
            .any(|message| message.as_concat_text().contains(expected)),
        "the next provider request needs explicit repair feedback"
    );
    assert!(
        !serde_json::to_string(next_context)
            .unwrap()
            .contains(RAW_SECRET),
        "raw provider errors and arguments must not enter repair context"
    );
    assert!(
        !next_context
            .iter()
            .flat_map(|message| &message.content)
            .any(|content| matches!(
                content,
                MessageContent::ToolRequest(_) | MessageContent::ToolResponse(_)
            )),
        "repair must not invent a structured tool-call pair"
    );
    let feedback = stored
        .iter()
        .filter(|message| message.as_concat_text().contains(expected))
        .collect::<Vec<_>>();
    assert_eq!(
        feedback.len(),
        1,
        "repair feedback must be durable exactly once"
    );
    assert!(feedback[0].is_agent_visible());
    assert!(!feedback[0].is_user_visible());
    assert!(feedback[0].id.is_some());
}

#[tokio::test]
async fn invalid_arguments_receive_first_call_feedback_without_raw_input() {
    check_recovery(Some("invalid_arguments"), INVALID_FEEDBACK, false).await;
}

#[tokio::test]
async fn missing_completion_is_not_misreported_as_invalid_json() {
    check_recovery(Some("incomplete_stream"), INCOMPLETE_FEEDBACK, false).await;
}

#[tokio::test]
async fn legacy_unclassified_errors_receive_generic_feedback() {
    check_recovery(None, UNKNOWN_FEEDBACK, false).await;
}

#[tokio::test]
async fn signed_malformed_calls_keep_the_existing_terminal_abort() {
    check_recovery(Some("invalid_arguments"), INVALID_FEEDBACK, true).await;
}
