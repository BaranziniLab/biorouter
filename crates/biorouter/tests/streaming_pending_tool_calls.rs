//! §6.1b safety gate — pending tool-call streaming (the most dangerous item in
//! the tool-call-latency plan).
//!
//! A tool call announced *before its arguments finish* must be able to draw a UI
//! card without ever being executable. The guarantee is structural: pending
//! state travels as [`AgentEvent::ToolCallPending`] / a third stream-tuple slot,
//! never as a `MessageContent::ToolRequest`, so `categorize_tools`,
//! `num_tool_requests` and the dispatch loop — which only ever walk `Message`
//! contents — cannot see it.
//!
//! This test drives the *real* reply loop with a mock streaming provider (no
//! network, no keychain) and asserts:
//!   (a) a pending notification with the correct tool NAME arrives BEFORE any
//!       argument bytes;
//!   (b) exactly ONE authoritative `ToolRequest` reaches the transcript per
//!       tool block;
//!   (c) a PENDING-ONLY stream dispatches NOTHING — no tool executes, and the
//!       turn does not loop for a tool result. This is the load-bearing
//!       assertion that prevents executing a tool with truncated arguments.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::providers::base::{
    MessageStream, PendingToolCall, Provider, ProviderMetadata, ProviderUsage, Usage,
};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use futures::StreamExt;
use rmcp::model::{CallToolRequestParams, Tool};
use tempfile::TempDir;

/// One scripted provider stream item (the 3-tuple the real decoders yield).
#[derive(Clone)]
enum Item {
    /// A pending tool-call notification. Never a Message — cannot be dispatched.
    Pending(PendingToolCall),
    /// A plain assistant text message.
    Text(String),
    /// An authoritative, complete tool request (what `content_block_stop` emits).
    ToolRequest {
        id: String,
        name: String,
        args: serde_json::Value,
    },
}

/// A streaming provider that replays a fixed script on its first `stream()` call
/// and returns a bare "done" on every later call (so a turn that loops for tool
/// results terminates instead of hanging).
struct ScriptedStreamProvider {
    script: Vec<Item>,
    calls: AtomicUsize,
}

impl ScriptedStreamProvider {
    fn new(script: Vec<Item>) -> Self {
        Self {
            script,
            calls: AtomicUsize::new(0),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn usage() -> ProviderUsage {
        ProviderUsage::new(
            "mock-model".to_string(),
            Usage::new(Some(10), Some(5), Some(15)),
        )
    }
}

#[async_trait]
impl Provider for ScriptedStreamProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        // Only the streaming path is exercised, but complete() must exist.
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

    fn supports_streaming(&self) -> bool {
        true
    }

    async fn stream(
        &self,
        _system: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let usage = Self::usage();

        // Later calls (a tool-result continuation) just end the turn.
        let script = if n == 0 {
            self.script.clone()
        } else {
            Vec::new()
        };

        let stream = async_stream::try_stream! {
            for item in script {
                match item {
                    Item::Pending(p) => {
                        // The dangerous shape: a partial tool call. It rides the
                        // third slot with NO Message, so it is structurally
                        // incapable of reaching dispatch.
                        yield (None, None, Some(p));
                    }
                    Item::Text(t) => {
                        yield (Some(Message::assistant().with_text(&t)), None, None);
                    }
                    Item::ToolRequest { id, name, args } => {
                        let call = CallToolRequestParams {
                            task: None,
                            name: name.into(),
                            arguments: Some(rmcp::model::object(args)),
                            meta: None,
                        };
                        let msg = Message::assistant().with_tool_request(id, Ok(call));
                        yield (Some(msg), None, None);
                    }
                }
            }
            // Terminal usage.
            yield (None, Some(usage), None);
        };
        Ok(Box::pin(stream))
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new("mock-model").unwrap()
    }

    fn metadata() -> ProviderMetadata {
        ProviderMetadata {
            name: "mock".to_string(),
            display_name: "Mock Streaming Provider".to_string(),
            description: "Scripted streaming provider for testing".to_string(),
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
        "mock-streaming-test"
    }
}

async fn agent_with_provider(
    provider: Arc<ScriptedStreamProvider>,
) -> (Arc<Agent>, String, TempDir) {
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
            "pending-tool-call-test".to_string(),
            SessionType::Hidden,
        )
        .await
        .unwrap();

    agent
        .update_provider(provider.clone() as Arc<dyn Provider>, &session.id)
        .await
        .unwrap();

    let agent = Arc::new(agent);
    std::mem::forget(data_dir);
    (agent, session.id, work_dir)
}

/// Collected agent output: every pending notification and every streamed message.
struct Collected {
    pendings: Vec<PendingToolCall>,
    messages: Vec<Message>,
}

impl Collected {
    fn tool_requests(&self) -> Vec<&biorouter::conversation::message::ToolRequest> {
        self.messages
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|c| match c {
                MessageContent::ToolRequest(r) => Some(r),
                _ => None,
            })
            .collect()
    }

    fn tool_responses(&self) -> Vec<&MessageContent> {
        self.messages
            .iter()
            .flat_map(|m| m.content.iter())
            .filter(|c| matches!(c, MessageContent::ToolResponse(_)))
            .collect()
    }
}

async fn drain(agent: &Agent, user: &str, session_id: &str) -> Result<Collected> {
    let session_config = SessionConfig {
        id: session_id.to_string(),
        schedule_id: None,
        max_turns: Some(4),
        max_tool_calls: None,
        retry_config: None,
        budget: None,
        reasoning_effort: None,
    };
    let stream = agent
        .reply(Message::user().with_text(user), session_config, None)
        .await?;
    tokio::pin!(stream);
    let mut pendings = Vec::new();
    let mut messages = Vec::new();
    while let Some(ev) = stream.next().await {
        match ev? {
            AgentEvent::ToolCallPending(p) => pendings.push(p),
            AgentEvent::Message(m) => messages.push(m),
            _ => {}
        }
    }
    Ok(Collected { pendings, messages })
}

/// (c) The load-bearing assertion: a pending-only stream dispatches NOTHING.
///
/// The provider announces a `shell` tool call with dangerous partial arguments
/// and then ends the turn with plain text — it NEVER emits the authoritative
/// `ToolRequest`. If any part of the pipeline treated the pending notification
/// as a real request, the agent would dispatch `shell` (executing
/// `rm -rf /…`-shaped truncated input) and loop for a tool result. Neither may
/// happen.
#[tokio::test(flavor = "multi_thread")]
async fn pending_only_stream_dispatches_nothing() {
    let provider = Arc::new(ScriptedStreamProvider::new(vec![
        Item::Pending(PendingToolCall {
            id: "toolu_danger".to_string(),
            name: "developer__shell".to_string(),
            partial_args: None,
        }),
        Item::Pending(PendingToolCall {
            id: "toolu_danger".to_string(),
            name: "developer__shell".to_string(),
            // A truncated, invalid fragment — exactly what would destroy data if
            // it ever reached a dispatcher.
            partial_args: Some("{\"command\": \"rm -rf /tm".to_string()),
        }),
        Item::Text("All set.".to_string()),
    ]));
    let (agent, session_id, _work_dir) = agent_with_provider(provider.clone()).await;

    let out = drain(&agent, "Delete my files", &session_id).await.unwrap();

    // (c) THE LOAD-BEARING ASSERTIONS — checked FIRST so a regression that routes
    //     a partial into dispatch is caught here, on the safety property itself,
    //     rather than on a downstream pending-event check.
    //
    // (c1) Nothing was dispatched: no ToolRequest ever entered the transcript...
    assert!(
        out.tool_requests().is_empty(),
        "a pending-only stream must NOT produce any authoritative ToolRequest \
         (a partial reached dispatch!), saw {:?}",
        out.tool_requests()
    );
    // (c2) ...and therefore no tool response exists — no tool executed.
    assert!(
        out.tool_responses().is_empty(),
        "a pending-only stream must execute no tool (no ToolResponse), saw {} response(s)",
        out.tool_responses().len()
    );
    // (c3) The turn did not loop for a tool result: exactly one provider call.
    assert_eq!(
        provider.call_count(),
        1,
        "a pending-only stream must not trigger a tool-result continuation turn"
    );

    // (a) The pending notification carried the tool name, and the first one
    //     arrived before any argument bytes.
    assert!(
        !out.pendings.is_empty(),
        "expected at least one ToolCallPending event"
    );
    assert_eq!(out.pendings[0].name, "developer__shell");
    assert!(
        out.pendings[0].partial_args.is_none(),
        "the first pending notification must precede any argument bytes"
    );
}

/// (a)+(b): a pending notification precedes the authoritative request, and the
/// completed tool block yields exactly ONE `ToolRequest` in the transcript.
#[tokio::test(flavor = "multi_thread")]
async fn pending_then_completed_request_yields_one_authoritative_request() {
    let provider = Arc::new(ScriptedStreamProvider::new(vec![
        // Announce first, no args.
        Item::Pending(PendingToolCall {
            id: "toolu_ok".to_string(),
            name: "developer__text_editor".to_string(),
            partial_args: None,
        }),
        // Throttled preview update (still not a Message).
        Item::Pending(PendingToolCall {
            id: "toolu_ok".to_string(),
            name: "developer__text_editor".to_string(),
            partial_args: Some("{\"command\": \"vi".to_string()),
        }),
        // The authoritative, complete request.
        Item::ToolRequest {
            id: "toolu_ok".to_string(),
            name: "developer__text_editor".to_string(),
            args: serde_json::json!({"command": "view", "path": "/tmp/x"}),
        },
    ]));
    let (agent, session_id, _work_dir) = agent_with_provider(provider.clone()).await;

    let out = drain(&agent, "Show me the file", &session_id)
        .await
        .unwrap();

    // (a) name-before-args on the pending channel.
    assert!(out
        .pendings
        .iter()
        .any(|p| p.name == "developer__text_editor"));
    assert!(
        out.pendings[0].partial_args.is_none(),
        "first pending notification precedes argument bytes"
    );

    // (b) exactly one authoritative ToolRequest for the block, carrying the id
    //     shared with the pending notifications and the COMPLETE arguments.
    let requests = out.tool_requests();
    assert_eq!(
        requests.len(),
        1,
        "exactly one authoritative ToolRequest per tool block, saw {}",
        requests.len()
    );
    assert_eq!(requests[0].id, "toolu_ok");
    let call = requests[0]
        .tool_call
        .as_ref()
        .expect("authoritative request parses");
    assert_eq!(call.name, "developer__text_editor");
    assert_eq!(
        call.arguments.as_ref().unwrap().get("command").unwrap(),
        "view"
    );
}
