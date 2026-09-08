//! §6.2c gate — per-tool response emission in COMPLETION order, while the
//! PERSISTED transcript keeps REQUEST order (the highest-risk ordering item in
//! the tool-call-latency plan).
//!
//! The provider replies with ONE assistant message carrying TWO `developer__shell`
//! tool calls dispatched as a single batch:
//!   - `call_slow` (request order #1): `sleep 0.4 && echo slow`  → finishes LAST
//!   - `call_fast` (request order #2): `sleep 0.02 && echo fast` → finishes FIRST
//!
//! so completion order (`fast`, `slow`) genuinely differs from request order
//! (`slow`, `fast`). If the two shells ever serialized in dispatch order the slow
//! one would finish first and this test would fail — so a pass also proves the
//! §6.2a/§6.2b parallel batch dispatch it builds on.
//!
//! Two assertions, one per invariant:
//!   (1) STREAMED transcript: the tool responses are yielded in COMPLETION order
//!       (`call_fast` before `call_slow`). This is the behaviour §6.2c adds —
//!       before the change every response was yielded from the post-batch loop in
//!       request order, so this assertion FAILS on the pre-§6.2c agent.
//!   (2) PERSISTED conversation (§6.5 invariant 2): the SQLite session keeps
//!       REQUEST order — `call_slow`'s tool_use+tool_result pair precedes
//!       `call_fast`'s — so the next provider call's tool_result blocks still line
//!       up with the preceding assistant turn's tool_use blocks (Anthropic 400s
//!       otherwise). This holds both before and after the change; it is the
//!       invariant the streaming reorder must not break.

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

const SLOW_ID: &str = "call_slow";
const FAST_ID: &str = "call_fast";

/// Emits a two-shell batch on the first call, then ends the turn.
struct BatchShellProvider {
    calls: AtomicUsize,
}

impl BatchShellProvider {
    fn new() -> Self {
        Self {
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
impl Provider for BatchShellProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            // ONE assistant message, TWO tool_use blocks: a single batch.
            // Request order is [slow, fast]; completion order is [fast, slow].
            let slow = CallToolRequestParams {
                task: None,
                meta: None,
                name: "developer__shell".into(),
                arguments: Some(object!({"command": "sleep 0.4 && echo slow"})),
            };
            let fast = CallToolRequestParams {
                task: None,
                meta: None,
                name: "developer__shell".into(),
                arguments: Some(object!({"command": "sleep 0.02 && echo fast"})),
            };
            let message = Message::assistant()
                .with_tool_request(SLOW_ID, Ok(slow))
                .with_tool_request(FAST_ID, Ok(fast));
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
            description: "Mock provider for testing".to_string(),
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
        "mock-batch-shell"
    }
}

async fn agent_with_provider(provider: Arc<dyn Provider>) -> (Arc<Agent>, String, TempDir) {
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
            "tool-response-ordering-test".to_string(),
            SessionType::Hidden,
        )
        .await
        .unwrap();

    agent.update_provider(provider, &session.id).await.unwrap();

    // The `developer` builtin supplies the real `developer__shell` tool the
    // provider drives; its `sleep` commands give the two batch tools genuinely
    // different completion times.
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

    let agent = Arc::new(agent);
    std::mem::forget(data_dir);
    (agent, session.id, work_dir)
}

/// Collect every streamed `AgentEvent::Message`, auto-approving any tool
/// confirmation so the loop progresses even outside Auto's auto-approve path.
async fn drain(agent: &Agent, user: &str, session_id: &str) -> Result<Vec<Message>> {
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
        .reply(Message::user().with_text(user), session_config, None)
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

/// Tool-response ids in the order they appear across `messages`.
fn tool_response_ids(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            MessageContent::ToolResponse(r) => Some(r.id.clone()),
            _ => None,
        })
        .collect()
}

/// Tool-request ids in the order they appear across `messages`.
fn tool_request_ids(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            MessageContent::ToolRequest(r) => Some(r.id.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn streamed_responses_follow_completion_order_persisted_keeps_request_order() {
    let provider = Arc::new(BatchShellProvider::new());
    let (agent, session_id, _work) = agent_with_provider(provider.clone()).await;

    let streamed = drain(&agent, "run both shells", &session_id).await.unwrap();

    // Both tools must actually have executed — otherwise every ordering
    // assertion below is vacuous.
    let streamed_resp_ids = tool_response_ids(&streamed);
    assert_eq!(
        streamed_resp_ids.len(),
        2,
        "expected exactly two tool responses in the streamed transcript, got {streamed_resp_ids:?}"
    );

    // (1) GATE — the streamed transcript yields responses in COMPLETION order.
    //     `call_fast` (sleep 0.02) finishes well before `call_slow` (sleep 0.4),
    //     so the fast response must be streamed FIRST. On the pre-§6.2c agent both
    //     responses are yielded from the post-batch loop in REQUEST order
    //     (`call_slow` first), so this assertion fails there.
    assert_eq!(
        streamed_resp_ids,
        vec![FAST_ID.to_string(), SLOW_ID.to_string()],
        "streamed tool responses must arrive in completion order (fast, slow); \
         request order (slow, fast) means the per-tool emission did not move into \
         the execution loop"
    );

    // (2) INVARIANT §6.5-2 — the PERSISTED conversation keeps REQUEST order.
    let session = agent
        .config
        .session_manager
        .get_session(&session_id, true)
        .await
        .unwrap();
    let convo = session.conversation.expect("session has a conversation");
    let msgs = convo.messages();

    let persisted_req_ids = tool_request_ids(msgs);
    assert_eq!(
        persisted_req_ids,
        vec![SLOW_ID.to_string(), FAST_ID.to_string()],
        "persisted tool_use blocks must stay in request order (slow, fast)"
    );
    let persisted_resp_ids = tool_response_ids(msgs);
    assert_eq!(
        persisted_resp_ids,
        vec![SLOW_ID.to_string(), FAST_ID.to_string()],
        "persisted tool_result blocks must stay in request order (slow, fast) even \
         though the streamed transcript used completion order; the tool_result \
         blocks must correspond to the preceding assistant turn's tool_use blocks"
    );

    // Pairing: each tool_use is immediately followed by its own tool_result, so a
    // provider replaying this transcript never sees an unmatched block.
    let ordered: Vec<(&str, &str)> = msgs
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            MessageContent::ToolRequest(r) => Some(("req", r.id.as_str())),
            MessageContent::ToolResponse(r) => Some(("resp", r.id.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        ordered,
        vec![
            ("req", SLOW_ID),
            ("resp", SLOW_ID),
            ("req", FAST_ID),
            ("resp", FAST_ID),
        ],
        "each persisted tool_use must be immediately followed by its matching \
         tool_result, in request order"
    );

    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        2,
        "one batch turn + one continuation turn"
    );
}
