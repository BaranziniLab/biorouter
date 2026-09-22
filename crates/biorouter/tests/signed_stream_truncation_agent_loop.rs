//! A Bedrock stream that dies mid-`tool_use` on a SIGNED turn must be
//! recoverable.
//!
//! The user-reported failure (app 1.89.10, `aws_bedrock` /
//! `us.anthropic.claude-sonnet-5`): the connection dropped while
//! `developer__text_editor`'s arguments were still arriving, and the chat became
//! unusable — the terminal message said "Start a new chat", and the desktop
//! offered no Retry because the abort was classified non-retryable.
//!
//! Nothing ran and no arguments were ever complete, so the only correct
//! resting place is the state the turn started from. The provider's own error
//! says "Please retry"; this file is the contract that the agent loop agrees.
//!
//! The scripted response is not invented — its shape is pinned by
//! `providers::formats::bedrock::bedrock_stream_tests::signed_truncated_tool_block_reports_incomplete_stream_under_the_signed_id`,
//! which drives the real `BedrockStreamDecoder` over a truncated event stream:
//! two messages sharing one id, the first carrying signed reasoning, the second
//! a single `Err` tool request tagged `incomplete_stream`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use biorouter::agents::turn_abort::TurnAbortCode;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::providers::base::{MessageStream, Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session::{Session, SessionManager};
use futures::StreamExt;
use rmcp::model::{ErrorCode, ErrorData, Tool};
use serde_json::json;
use tempfile::TempDir;

const SIGNATURE: &str = "bedrock-signature";
const TRUNCATED_TOOL: &str = "developer__text_editor";

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

/// What the decoder emits when the connection dies mid-`tool_use`, and what a
/// *complete* signed tool turn looks like for the same scripted provider.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Script {
    /// The reported bug: signed reasoning closed, tool arguments cut off.
    TruncatedStream,
    /// The block list arrived whole; its arguments simply were not usable. The
    /// signed history genuinely cannot be replayed unchanged, so this one must
    /// stay terminal.
    InvalidArguments,
}

struct SignedBedrockProvider {
    script: Script,
    calls: Mutex<Vec<Vec<Message>>>,
}

impl SignedBedrockProvider {
    fn new(script: Script) -> Self {
        Self {
            script,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    /// The two messages one truncated Bedrock response decodes to — same id,
    /// signed reasoning first, the unfinished call second.
    fn scripted_response(&self, call: usize) -> Vec<Message> {
        let id = format!("bedrock-response-{call}");
        let (code, message, kind) = match self.script {
            Script::TruncatedStream => (
                ErrorCode::INTERNAL_ERROR,
                format!(
                    "The Bedrock response stream ended before completion of the tool block \
                     for `{TRUNCATED_TOOL}` was confirmed, so the call was not made. Please retry."
                ),
                "incomplete_stream",
            ),
            Script::InvalidArguments => (
                ErrorCode::INVALID_PARAMS,
                "Could not parse tool arguments".to_string(),
                "invalid_arguments",
            ),
        };
        vec![
            Message::assistant()
                .with_id(id.clone())
                .with_thinking("I will rewrite the file.", SIGNATURE),
            Message::assistant().with_id(id).with_tool_request(
                "toolu_trunc",
                Err(ErrorData::new(
                    code,
                    message,
                    Some(json!({ "biorouterToolCallFailure": kind })),
                )),
            ),
        ]
    }
}

#[async_trait]
impl Provider for SignedBedrockProvider {
    async fn complete(
        &self,
        _system: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        // Only reached if the agent stops streaming; the scripted turn is a
        // stream, so keep this honest rather than silently succeeding.
        self.calls.lock().unwrap().push(messages.to_vec());
        Err(ProviderError::ExecutionError(
            "this provider only streams".to_string(),
        ))
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

    fn supports_streaming(&self) -> bool {
        true
    }

    async fn stream(
        &self,
        _system: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let call = {
            let mut calls = self.calls.lock().unwrap();
            calls.push(messages.to_vec());
            calls.len() - 1
        };
        // A truncated stream never reaches `messageStop`, so it reports no
        // finish reason — only whatever usage arrived before the cut.
        let usage = ProviderUsage::new(
            "us.anthropic.claude-sonnet-5".to_string(),
            Usage::new(Some(10), Some(5), Some(15)),
        );
        let mut items: Vec<Result<_, ProviderError>> = self
            .scripted_response(call)
            .into_iter()
            .map(|message| Ok((Some(message), None, None)))
            .collect();
        items.push(Ok((None, Some(usage), None)));
        Ok(Box::pin(futures::stream::iter(items)))
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("us.anthropic.claude-sonnet-5")
    }

    fn metadata() -> ProviderMetadata {
        ProviderMetadata::empty()
    }

    fn get_name(&self) -> &str {
        "aws_bedrock"
    }
}

struct Harness {
    agent: Agent,
    manager: Arc<SessionManager>,
    session: Session,
    _work: TempDir,
    _data: TempDir,
    _permissions: TempDir,
}

async fn harness(provider: Arc<SignedBedrockProvider>) -> Harness {
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
            "signed-stream-truncation".to_string(),
            SessionType::Hidden,
        )
        .await
        .unwrap();
    agent.update_provider(provider, &session.id).await.unwrap();
    Harness {
        agent,
        manager,
        session,
        _work: work,
        _data: data,
        _permissions: permissions,
    }
}

struct TurnOutcome {
    abort: Option<TurnAbortCode>,
    abort_message: String,
    stored: Vec<Message>,
    /// Every assistant row the turn streamed to a live consumer, in order.
    streamed_assistant_rows: Vec<Message>,
    /// The last `HistoryReplaced` resync the turn emitted, if any.
    resync: Option<Vec<Message>>,
}

async fn run_turn(harness: &Harness, prompt: &str) -> TurnOutcome {
    let stream = harness
        .agent
        .reply(
            Message::user().with_text(prompt),
            SessionConfig {
                id: harness.session.id.clone(),
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
    let mut abort_message = String::new();
    let mut streamed_assistant_rows = Vec::new();
    let mut resync = None;
    tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                AgentEvent::TurnAborted { code, message } => {
                    abort = Some(code);
                    abort_message = message;
                }
                AgentEvent::Message(message) => {
                    if message.role == rmcp::model::Role::Assistant {
                        streamed_assistant_rows.push(message);
                    }
                }
                AgentEvent::HistoryReplaced(conversation) => {
                    resync = Some(conversation.into_messages());
                }
                _ => {}
            }
        }
    })
    .await
    .expect("the truncated signed turn must terminate, not hang");

    let stored = harness
        .manager
        .get_session(&harness.session.id, true)
        .await
        .unwrap()
        .conversation
        .unwrap()
        .into_messages();
    TurnOutcome {
        abort,
        abort_message,
        stored,
        streamed_assistant_rows,
        resync,
    }
}

fn has_signed_thinking(message: &Message) -> bool {
    message.content.iter().any(|content| {
        matches!(content, MessageContent::Thinking(thinking) if thinking.signature == SIGNATURE)
    })
}

fn has_failed_tool_request(message: &Message) -> bool {
    message
        .content
        .iter()
        .any(|content| matches!(content, MessageContent::ToolRequest(r) if r.tool_call.is_err()))
}

/// The whole point: a truncated signed turn leaves the conversation exactly
/// where it started, so the desktop's Retry — which re-sends the row at the tail
/// of the transcript and gives up unless that row is the user's — has something
/// to re-send.
#[tokio::test]
async fn a_truncated_signed_tool_call_leaves_the_chat_retryable() {
    let provider = Arc::new(SignedBedrockProvider::new(Script::TruncatedStream));
    let harness = harness(provider.clone()).await;
    let outcome = run_turn(&harness, "Rewrite the file.").await;

    assert_eq!(
        provider.call_count(),
        1,
        "a signed turn must never be silently re-issued with a mutated prefix"
    );
    assert_eq!(outcome.abort, Some(TurnAbortCode::SignedStreamTruncated));

    // The abort text is what the desktop renders in the error card. It must not
    // send the user off to a new chat for something a retry fixes.
    assert!(
        !outcome.abort_message.contains("Start a new chat"),
        "a recoverable truncation must not tell the user to abandon the chat: {}",
        outcome.abort_message
    );
    assert!(
        outcome.abort_message.to_lowercase().contains("retry"),
        "the abort text must offer the retry it is now classified for: {}",
        outcome.abort_message
    );

    // Nothing of the partial response survived.
    assert!(
        !outcome.stored.iter().any(has_signed_thinking),
        "the partial signed row must not be persisted: a retry would replay a \
         block list the signature never covered"
    );
    assert!(
        !outcome.stored.iter().any(has_failed_tool_request),
        "the unfinished tool request must not be persisted"
    );
    assert!(
        !outcome
            .stored
            .iter()
            .any(|message| message.as_concat_text().contains("Start a new chat")),
        "no terminal assistant row is persisted for the recoverable case"
    );

    // `chatStreamStore.retryTurnOnce` reads the LAST row of the transcript and
    // returns early unless it is the user's. An assistant row there is a dead
    // Retry button, whatever the error frame claims.
    let last = outcome.stored.last().expect("the prompt is stored");
    assert_eq!(
        last.role,
        rmcp::model::Role::User,
        "the tail of the transcript must still be the user's prompt, or Retry is inert"
    );
    assert_eq!(last.as_concat_text(), "Rewrite the file.");

    // …and the transcript `retryTurnOnce` reads is the LIVE one, which is fed by
    // the streamed frames, not by the store. The partial reply was streamed
    // before it was abandoned, so without a resync the client is still holding
    // assistant rows that no longer exist.
    assert!(
        !outcome.streamed_assistant_rows.is_empty(),
        "the partial reply really was streamed before the abort — otherwise this \
         test proves nothing about the resync"
    );
    let resync = outcome
        .resync
        .expect("the rollback must resync every live transcript to the store");
    assert_eq!(
        resync
            .iter()
            .map(|m| (m.role.clone(), m.as_concat_text()))
            .collect::<Vec<_>>(),
        vec![(rmcp::model::Role::User, "Rewrite the file.".to_string())],
        "the resync must carry what the store holds, so a reload agrees with it"
    );
}

/// The rollback holds ACROSS a retry, which is the property the user actually
/// experiences: two truncated turns in a row leave a transcript of two prompts
/// and nothing else.
///
/// This is deliberately asserted on the stored transcript rather than on what
/// the provider received. `fix_conversation` drops an orphaned tool request
/// before every provider call, so the abandoned row is scrubbed out of the
/// model's view whether or not it was persisted — a provider-side assertion
/// here passes on the unfixed code and measures nothing. What the scrub does
/// NOT do is take the row off the user's screen, or give `retryTurn` back the
/// user message it needs at the tail.
#[tokio::test]
async fn a_retry_after_a_truncated_signed_turn_leaves_no_residue() {
    let provider = Arc::new(SignedBedrockProvider::new(Script::TruncatedStream));
    let harness = harness(provider.clone()).await;
    run_turn(&harness, "Rewrite the file.").await;
    let outcome = run_turn(&harness, "Rewrite the file.").await;

    assert_eq!(provider.call_count(), 2);
    assert_eq!(outcome.abort, Some(TurnAbortCode::SignedStreamTruncated));

    let roles: Vec<_> = outcome.stored.iter().map(|m| m.role.clone()).collect();
    assert_eq!(
        roles,
        vec![rmcp::model::Role::User, rmcp::model::Role::User],
        "two abandoned turns must leave only the two prompts: {:?}",
        outcome
            .stored
            .iter()
            .map(|m| (m.role.clone(), m.as_concat_text(), has_signed_thinking(m)))
            .collect::<Vec<_>>()
    );

    let second = provider.calls.lock().unwrap()[1].clone();
    assert!(
        !second.iter().any(has_signed_thinking),
        "the retry must not carry the abandoned turn's signed reasoning"
    );
    assert!(
        !second.iter().any(has_failed_tool_request),
        "the retry must not carry the abandoned turn's unfinished tool request"
    );
}

/// The guard this change must NOT weaken. A signed response whose block list
/// arrived complete but unusable cannot be replayed without mutation, so it
/// stays terminal and non-retryable.
#[tokio::test]
async fn a_complete_signed_turn_with_unusable_arguments_stays_terminal() {
    let provider = Arc::new(SignedBedrockProvider::new(Script::InvalidArguments));
    let harness = harness(provider.clone()).await;
    let outcome = run_turn(&harness, "Rewrite the file.").await;

    assert_eq!(provider.call_count(), 1);
    assert_eq!(outcome.abort, Some(TurnAbortCode::SignedReplayInvalidated));
    assert!(outcome.abort_message.contains("Start a new chat"));
    assert!(
        outcome.stored.iter().any(has_failed_tool_request),
        "the unreplayable signed row stays on the record"
    );
}
