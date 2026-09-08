//! The mirror path's load-bearing safety gate: a tool call the **child already
//! ran** must never be dispatched a second time by the agent loop.
//!
//! The two coding-agent providers (`claude_code`, `codex`) drive a vendor CLI
//! that executes its tool calls over Biorouter's HTTP tool bridge *before* the
//! frame describing the call ever reaches this crate. Streaming those calls into
//! the transcript so the user can see them means putting a `ToolRequest` into the
//! provider stream — and a `ToolRequest` in a response message is exactly what
//! `categorize_tools` dispatches. Without the mirror marker the loop would run
//! the call again: a shell command executed twice, a file written twice.
//!
//! This drives the **real** reply loop with a mock streaming provider and asserts
//! the two halves that together make the feature safe:
//!
//!   (a) a MARKED request/response pair is recorded and shown but **dispatched
//!       nothing** — proved by the provider being called exactly once, because a
//!       dispatch would send the loop back for a tool result;
//!   (b) the SAME script without the marker **does** dispatch — proving the new
//!       branch is scoped to mirrored content and has not quietly disabled tool
//!       calling for every other provider.
//!
//! (b) is not decoration. A gate that suppresses everything would pass (a) while
//! breaking the entire product, and no assertion in (a) alone can tell the
//! difference.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::providers::base::{MessageStream, Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::coding_agent::mirror::{self, Execution};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use futures::StreamExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, Content, Tool};
use tempfile::TempDir;

/// Build the assistant message carrying the tool call the child made, and the
/// user message carrying the result the child got back — the exact pair a
/// decoder mints, marked or not.
fn mirrored_pair(marked: bool) -> (Message, Message) {
    let call = CallToolRequestParams {
        name: "developer__shell".into(),
        arguments: Some(rmcp::model::object(serde_json::json!({ "command": "ls" }))),
        meta: None,
        task: None,
    };

    let mut request = biorouter::conversation::message::ToolRequest {
        id: "toolu_mirror_1".to_string(),
        tool_call: Ok(call),
        metadata: None,
        tool_meta: None,
    };
    let mut response = biorouter::conversation::message::ToolResponse {
        id: "toolu_mirror_1".to_string(),
        tool_result: Ok(CallToolResult::success(vec![Content::text("a.txt\nb.txt")])),
        metadata: None,
    };

    if marked {
        mirror::mark_request(&mut request, Execution::Bridged);
        mirror::mark_response(&mut response, Execution::Bridged);
    }

    (
        Message::assistant().with_content(MessageContent::ToolRequest(request)),
        Message::user().with_content(MessageContent::ToolResponse(response)),
    )
}

/// Replays one mirrored turn on its first `stream()` and ends every later turn
/// immediately, so a loop that *did* dispatch terminates (and is counted)
/// instead of hanging the test.
struct MirrorProvider {
    marked: bool,
    calls: AtomicUsize,
}

impl MirrorProvider {
    fn new(marked: bool) -> Self {
        Self {
            marked,
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
impl Provider for MirrorProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
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
        let marked = self.marked;
        let usage = Self::usage();

        let stream = async_stream::try_stream! {
            if n == 0 {
                let (request, response) = mirrored_pair(marked);
                yield (Some(request), None, None);
                yield (Some(response), None, None);
                yield (Some(Message::assistant().with_text("Listed the directory.")), None, None);
            } else {
                yield (Some(Message::assistant().with_text("done")), None, None);
            }
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
            display_name: "Mock Mirror Provider".to_string(),
            description: "Scripted mirrored-tool-call provider for testing".to_string(),
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
        "mock-mirror-test"
    }
}

async fn agent_with(provider: Arc<MirrorProvider>) -> (Arc<Agent>, String, TempDir) {
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
            "mirror-loop-test".to_string(),
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

async fn drain(agent: &Agent, session_id: &str) -> Result<Vec<Message>> {
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
        .reply(
            Message::user().with_text("list the directory"),
            session_config,
            None,
        )
        .await?;
    tokio::pin!(stream);
    let mut messages = Vec::new();
    while let Some(ev) = stream.next().await {
        if let AgentEvent::Message(m) = ev? {
            messages.push(m);
        }
    }
    Ok(messages)
}

/// (a) A marked pair is shown and recorded, and dispatches nothing.
///
/// The provider is called exactly ONCE. That is the whole proof: had the loop
/// dispatched the request, it would have needed a tool result and come back for
/// a second turn. One call means the pair was taken as a record, not a request.
#[tokio::test(flavor = "multi_thread")]
async fn a_marked_pair_is_recorded_and_never_dispatched() {
    let provider = Arc::new(MirrorProvider::new(true));
    let (agent, session_id, _work) = agent_with(provider.clone()).await;

    let messages = drain(&agent, &session_id).await.expect("reply");

    assert_eq!(
        provider.call_count(),
        1,
        "a mirrored pair must not send the loop back for a tool result — a second \
         provider call means the already-executed call was dispatched again"
    );

    let requests: Vec<_> = messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            MessageContent::ToolRequest(r) => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(requests.len(), 1, "the call is shown exactly once");
    assert_eq!(
        mirror::request_execution(requests[0]),
        Some(Execution::Bridged),
        "and it reaches the client still carrying its marker, so the card can say \
         who ran it"
    );

    let responses: Vec<_> = messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            MessageContent::ToolResponse(r) => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(
        responses.len(),
        1,
        "exactly one result — the child's own, not a second one the loop produced"
    );
    assert_eq!(
        responses[0].id, "toolu_mirror_1",
        "the response keeps the request's id, which is what pairs the card in the GUI"
    );
    assert!(
        responses[0].tool_result.is_ok(),
        "the child's successful result must arrive as a success, so the card is green"
    );
}

/// (b) The control: the identical script WITHOUT the marker still dispatches.
///
/// This is what proves the branch is narrow. `developer__shell` is not loaded in
/// this bare test agent, so the dispatch fails — but failing is a dispatch, and
/// it sends the loop back for a second turn. Two provider calls is the signal.
#[tokio::test(flavor = "multi_thread")]
async fn an_unmarked_request_still_dispatches() {
    let provider = Arc::new(MirrorProvider::new(false));
    let (agent, session_id, _work) = agent_with(provider.clone()).await;

    let _ = drain(&agent, &session_id).await.expect("reply");

    assert!(
        provider.call_count() >= 2,
        "an unmarked ToolRequest must still be dispatched and drive a follow-up \
         turn (got {} provider calls) — otherwise the mirror branch has disabled \
         tool calling for every provider",
        provider.call_count()
    );
}
