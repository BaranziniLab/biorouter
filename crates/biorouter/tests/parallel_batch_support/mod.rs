//! Shared harness for the parallel tool-batch gates: a provider that emits one
//! scripted batch of `developer__shell` calls as a SINGLE assistant message, an
//! agent wired to it, and helpers to drive a turn and read the results.
//!
//! Lives in its own module because the batch cases are split across several test
//! binaries — the env-var kill switches (`BIOROUTER_TOOL_RESPONSE_STREAMING`,
//! `BIOROUTER_TOOL_MAX_CONCURRENT`) are process-global, so each needs a process
//! of its own to set them without racing a sibling test.

#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use biorouter::agents::extension::ExtensionConfig;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{ActionRequiredData, Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use futures::StreamExt;
use rmcp::model::{CallToolRequestParams, Tool};
use rmcp::object;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// A provider that emits one scripted batch of `developer__shell` calls, then
// ends the turn. `(request_id, command)` pairs, dispatched as a SINGLE assistant
// message so the agent treats them as one parallel batch.
// ---------------------------------------------------------------------------

pub struct ScriptedBatchProvider {
    batch: Vec<(String, String)>,
    calls: AtomicUsize,
}

impl ScriptedBatchProvider {
    pub fn new(batch: Vec<(String, String)>) -> Self {
        Self {
            batch,
            calls: AtomicUsize::new(0),
        }
    }

    fn usage() -> ProviderUsage {
        ProviderUsage::new(
            "mock-model".to_string(),
            Usage::new(Some(10), Some(5), Some(15)),
        )
    }
}

#[async_trait]
impl Provider for ScriptedBatchProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            let mut message = Message::assistant();
            for (id, command) in &self.batch {
                message = message.with_tool_request(
                    id,
                    Ok(CallToolRequestParams {
                        task: None,
                        meta: None,
                        name: "developer__shell".into(),
                        arguments: Some(object!({ "command": command.clone() })),
                    }),
                );
            }
            return Ok((message, Self::usage()));
        }
        Ok((Message::assistant().with_text("done"), Self::usage()))
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
        ModelConfig::new("mock-model").unwrap()
    }

    fn metadata() -> ProviderMetadata {
        ProviderMetadata {
            name: "mock".to_string(),
            display_name: "Mock Provider".to_string(),
            description: "Mock provider for parallel-batch stress tests".to_string(),
            default_model: "mock-model".to_string(),
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
        "mock-scripted-batch"
    }
}

pub async fn agent_with_batch(batch: Vec<(String, String)>) -> (Arc<Agent>, String, TempDir) {
    let work_dir = TempDir::new().unwrap();
    let data_dir = TempDir::new().unwrap();
    let session_manager = Arc::new(SessionManager::new(data_dir.path().to_path_buf()));
    let config = AgentConfig::new(
        session_manager.clone(),
        PermissionManager::instance(),
        None,
        BioRouterMode::Auto,
    );
    let agent = Agent::with_config(config);

    let session = session_manager
        .create_session(
            work_dir.path().to_path_buf(),
            "parallel-batch-stress".to_string(),
            SessionType::Hidden,
        )
        .await
        .unwrap();

    agent
        .update_provider(Arc::new(ScriptedBatchProvider::new(batch)), &session.id)
        .await
        .unwrap();

    agent
        .add_extension(ExtensionConfig::Builtin {
            name: "developer".to_string(),
            description: "Developer tools".to_string(),
            display_name: None,
            timeout: None,
            bundled: Some(true),
            available_tools: vec![],
        })
        .await
        .expect("developer extension registers");

    // The session data dir must outlive the agent's background writers.
    std::mem::forget(data_dir);
    (Arc::new(agent), session.id, work_dir)
}

/// Drive one turn to completion, auto-approving any tool confirmation card.
pub async fn drain(agent: &Agent, session_id: &str) -> Result<Vec<Message>> {
    drain_with_token(agent, session_id, None).await
}

/// As [`drain`], but the turn runs under `cancel_token` so a test can cancel it
/// mid-batch.
pub async fn drain_with_token(
    agent: &Agent,
    session_id: &str,
    cancel_token: Option<tokio_util::sync::CancellationToken>,
) -> Result<Vec<Message>> {
    let session_config = SessionConfig {
        id: session_id.to_string(),
        schedule_id: None,
        max_turns: Some(4),
        max_tool_calls: None,
        budget: None,
        retry_config: None,
        reasoning_effort: None,
    };
    let stream = agent
        .reply(
            Message::user().with_text("run the batch"),
            session_config,
            cancel_token,
        )
        .await?;
    tokio::pin!(stream);
    let mut out = Vec::new();
    while let Some(ev) = stream.next().await {
        if let AgentEvent::Message(m) = ev? {
            if let Some(MessageContent::ActionRequired(action)) = m.content.first() {
                if let ActionRequiredData::ToolConfirmation { id, .. } = &action.data {
                    agent
                        .handle_confirmation(
                            id.clone(),
                            biorouter::permission::PermissionConfirmation {
                                principal_type:
                                    biorouter::permission::permission_confirmation::PrincipalType::Tool,
                                permission: biorouter::permission::Permission::AllowOnce,
                            },
                        )
                        .await;
                }
            }
            out.push(m);
        }
    }
    Ok(out)
}

/// Every `(tool_response_id, concatenated_text, is_error)` across `messages`.
pub fn tool_responses(messages: &[Message]) -> Vec<(String, String, bool)> {
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            MessageContent::ToolResponse(r) => {
                let (text, is_error) = match &r.tool_result {
                    Ok(result) => {
                        // A tool result carries one content item per AUDIENCE
                        // (the shell tool emits an Assistant copy and a User
                        // copy of the same output), so only the first is taken —
                        // joining them all would double every line.
                        let text = result
                            .content
                            .iter()
                            .find_map(|c| c.as_text().map(|t| t.text.clone()))
                            .unwrap_or_default();
                        (text, result.is_error.unwrap_or(false))
                    }
                    Err(e) => (e.to_string(), true),
                };
                Some((r.id.clone(), text, is_error))
            }
            _ => None,
        })
        .collect()
}

/// The persisted `("req"|"resp", id)` sequence for a session, in stored order.
/// This is what a provider replays, so an unmatched entry here is what makes
/// Anthropic reject the next turn with a 400.
pub async fn persisted_tool_blocks(agent: &Agent, session_id: &str) -> Vec<(&'static str, String)> {
    let session = agent
        .config
        .session_manager
        .get_session(session_id, true)
        .await
        .unwrap();
    let convo = session.conversation.expect("session has a conversation");
    convo
        .messages()
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            MessageContent::ToolRequest(r) => Some(("req", r.id.clone())),
            MessageContent::ToolResponse(r) => Some(("resp", r.id.clone())),
            _ => None,
        })
        .collect()
}
