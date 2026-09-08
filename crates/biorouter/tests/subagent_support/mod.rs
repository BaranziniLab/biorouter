//! Shared harness for the subagent gates (SUB-NN).
//!
//! One scripted provider serves BOTH sides of a delegation: the parent turn and
//! every child it spawns. That is not a shortcut — it is how the product wires
//! it. `dispatch_tool_call` hands the subagent tool the parent's own
//! `Arc<dyn Provider>`, so a child literally completes against the same object.
//!
//! The two roles are told apart by the system prompt: a child's is rendered from
//! `subagent_system.md`, which always opens "You are a specialized subagent".
//! Each child's task is carried in that prompt (an ad-hoc subagent workflow puts
//! its `instructions` in the system prompt and sends the bare user turn
//! "Begin."), so the harness embeds a `TASK:<script>` marker in the instructions
//! and reads it back out to decide how that particular child behaves.
//!
//! Scripts (the part after `TASK:`):
//!
//! | script | child behaviour |
//! |---|---|
//! | `ok:<name>` | answers `child <name> done` on its first turn |
//! | `slow:<name>:<ms>` | sleeps `<ms>` inside the provider, then answers |
//! | `fail:<name>` | the provider returns an execution error |
//! | `silent:<name>` | ends on a tool call, never emitting text |
//! | `nest:<name>` | tries to spawn a subagent of its own, then reports back |

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

/// The opening line of `subagent_system.md`. A system prompt carrying it belongs
/// to a child, not to the parent.
const CHILD_PROMPT_MARKER: &str = "You are a specialized subagent";

/// One entry of a parent's scripted tool batch.
#[derive(Clone, Debug)]
pub enum Call {
    /// A `subagent` call carrying these instructions.
    Subagent(String),
    /// An ordinary `developer__shell` call, so a batch can mix the two.
    Shell(String),
}

impl Call {
    /// A subagent whose child runs `script` and is identifiable by `name`.
    pub fn sub(name: &str, script: &str) -> Self {
        Call::Subagent(format!(
            "Handle the {name} workstream and report back. TASK:{script}"
        ))
    }
}

/// What a child did, recorded as it happens so a test can assert on the run
/// itself rather than only on what came back.
#[derive(Default)]
pub struct ChildLedger {
    /// Every `TASK:` script the provider was asked to run, in arrival order.
    pub started: Mutex<Vec<String>>,
    /// Highest number of children inside the provider at the same moment.
    pub peak_concurrent: AtomicUsize,
    in_flight: AtomicUsize,
    /// Per-script turn counter, so a child can answer differently on turn 2.
    turns: Mutex<HashMap<String, usize>>,
}

impl ChildLedger {
    fn enter(&self, script: &str) -> usize {
        self.started.lock().unwrap().push(script.to_string());
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak_concurrent.fetch_max(now, Ordering::SeqCst);
        let mut turns = self.turns.lock().unwrap();
        let turn = turns.entry(script.to_string()).or_insert(0);
        *turn += 1;
        *turn
    }

    fn leave(&self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }

    pub fn started_scripts(&self) -> Vec<String> {
        self.started.lock().unwrap().clone()
    }

    /// Distinct scripts started (a child takes several turns, so the raw list
    /// repeats).
    pub fn distinct_started(&self) -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();
        for script in self.started_scripts() {
            if !seen.contains(&script) {
                seen.push(script);
            }
        }
        seen
    }

    pub fn peak(&self) -> usize {
        self.peak_concurrent.load(Ordering::SeqCst)
    }
}

pub struct ScriptedSubagentProvider {
    batch: Vec<(String, Call)>,
    parent_calls: AtomicUsize,
    pub ledger: Arc<ChildLedger>,
}

impl ScriptedSubagentProvider {
    pub fn new(batch: Vec<(String, Call)>) -> Self {
        Self {
            batch,
            parent_calls: AtomicUsize::new(0),
            ledger: Arc::new(ChildLedger::default()),
        }
    }

    fn usage() -> ProviderUsage {
        ProviderUsage::new(
            "mock-model".to_string(),
            Usage::new(Some(10), Some(5), Some(15)),
        )
    }

    fn parent_turn(&self) -> Message {
        let mut message = Message::assistant();
        for (id, call) in &self.batch {
            let params = match call {
                Call::Subagent(instructions) => CallToolRequestParams {
                    task: None,
                    meta: None,
                    name: "subagent".into(),
                    arguments: Some(object!({ "instructions": instructions.clone() })),
                },
                Call::Shell(command) => CallToolRequestParams {
                    task: None,
                    meta: None,
                    name: "developer__shell".into(),
                    arguments: Some(object!({ "command": command.clone() })),
                },
            };
            message = message.with_tool_request(id, Ok(params));
        }
        message
    }

    async fn child_turn(&self, script: &str) -> Result<Message, ProviderError> {
        let turn = self.ledger.enter(script);
        let outcome = self.child_outcome(script, turn).await;
        self.ledger.leave();
        outcome
    }

    async fn child_outcome(&self, script: &str, turn: usize) -> Result<Message, ProviderError> {
        let mut parts = script.split(':');
        let kind = parts.next().unwrap_or("");
        let name = parts.next().unwrap_or("anon").to_string();
        match kind {
            "slow" => {
                let ms: u64 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(500);
                tokio::time::sleep(Duration::from_millis(ms)).await;
                Ok(Message::assistant().with_text(format!("child {name} done")))
            }
            "fail" => Err(ProviderError::ExecutionError(format!(
                "scripted provider failure for child {name}"
            ))),
            // Ends every turn on a tool call and never writes text, so the run
            // stops at the child turn cap with no summary. The command varies
            // per turn on purpose: repeating one would trip the tool-loop guard
            // and abort the turn, which is a different failure mode.
            "silent" => Ok(Message::assistant().with_tool_request(
                format!("{name}-silent-{turn}"),
                Ok(CallToolRequestParams {
                    task: None,
                    meta: None,
                    name: "developer__shell".into(),
                    arguments: Some(object!({ "command": format!("echo {name}-{turn}") })),
                }),
            )),
            "nest" if turn == 1 => Ok(Message::assistant().with_tool_request(
                format!("{name}-nested"),
                Ok(CallToolRequestParams {
                    task: None,
                    meta: None,
                    name: "subagent".into(),
                    arguments: Some(
                        object!({ "instructions": format!("grandchild of {name}. TASK:ok:grand-{name}") }),
                    ),
                }),
            )),
            "nest" => Ok(Message::assistant()
                .with_text(format!("child {name} done (could not nest, reported it)"))),
            _ => Ok(Message::assistant().with_text(format!("child {name} done"))),
        }
    }
}

/// Pull the `TASK:<script>` marker out of a child's rendered system prompt.
fn script_of(system_prompt: &str) -> Option<String> {
    let rest = system_prompt.split("TASK:").nth(1)?;
    Some(
        rest.lines()
            .next()
            .unwrap_or("")
            .trim()
            .trim_end_matches('.')
            .to_string(),
    )
}

#[async_trait]
impl Provider for ScriptedSubagentProvider {
    async fn complete(
        &self,
        system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        if system_prompt.contains(CHILD_PROMPT_MARKER) {
            let script = script_of(system_prompt).unwrap_or_else(|| "ok:unscripted".to_string());
            return Ok((self.child_turn(&script).await?, Self::usage()));
        }
        if self.parent_calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Ok((self.parent_turn(), Self::usage()));
        }
        Ok((
            Message::assistant().with_text("parent collected every subagent result"),
            Self::usage(),
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
        ModelConfig::new("mock-model").unwrap()
    }

    fn metadata() -> ProviderMetadata {
        ProviderMetadata {
            name: "mock".to_string(),
            display_name: "Mock Provider".to_string(),
            description: "Mock provider for subagent stress tests".to_string(),
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
        "mock-scripted-subagent"
    }
}

/// An agent whose next turn emits `batch`, plus the ledger of what its children
/// did and the session id to drive.
pub struct Harness {
    pub agent: Arc<Agent>,
    pub session_id: String,
    pub ledger: Arc<ChildLedger>,
    pub work_dir: TempDir,
}

pub async fn harness(batch: Vec<(String, Call)>) -> Harness {
    // Keep the child turn cap small so a child that never writes a summary
    // (`silent:`) reaches the cap in a test-sized run rather than in 25 turns.
    // Always the same value, so tests running concurrently cannot disagree.
    std::env::set_var("BIOROUTER_SUBAGENT_MAX_TURNS", "3");

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
            "subagent-stress".to_string(),
            SessionType::Hidden,
        )
        .await
        .unwrap();

    let provider = Arc::new(ScriptedSubagentProvider::new(batch));
    let ledger = provider.ledger.clone();
    agent.update_provider(provider, &session.id).await.unwrap();

    // The subagent tool is only offered when at least one extension is loaded,
    // and the `silent` child needs a real `shell` to end a turn on.
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
    Harness {
        agent: Arc::new(agent),
        session_id: session.id,
        ledger,
        work_dir,
    }
}

/// Drive one turn to completion, auto-approving any tool confirmation card.
pub async fn drain(agent: &Agent, session_id: &str) -> Result<Vec<Message>> {
    drain_with_token(agent, session_id, None).await
}

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
            Message::user().with_text("delegate the work"),
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

/// Every `(tool_response_id, text, is_error)` across `messages`.
pub fn tool_responses(messages: &[Message]) -> Vec<(String, String, bool)> {
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            MessageContent::ToolResponse(r) => {
                let (text, is_error) = match &r.tool_result {
                    Ok(result) => {
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

/// The structured envelope a subagent response carried, by tool-response id.
pub fn structured_results(messages: &[Message]) -> HashMap<String, serde_json::Value> {
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            MessageContent::ToolResponse(r) => {
                let result = r.tool_result.as_ref().ok()?;
                Some((r.id.clone(), result.structured_content.clone()?))
            }
            _ => None,
        })
        .collect()
}

/// The persisted `("req"|"resp", id)` sequence for a session, in stored order.
/// An unmatched `req` here is what makes a provider reject the next turn.
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
