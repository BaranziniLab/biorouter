//! A turn the person cancelled leaves one note in the conversation, and the
//! model reads it on its next turn (owner ruling 2026-11-17).
//!
//! The note is a RECORD, not a guard: it is there so that the agent on its next
//! turn, and anyone reading the chat later, can see that a cancel happened at
//! that point. Whether the person then resumes the request or asks for
//! something else, both know the turn was cancelled. So the note is a plain
//! fact, and it tells the model nothing about what to do next.
//!
//! These tests drive the real reply loop against a streaming provider double
//! that stalls on its first request until the test cancels the turn, then
//! settle the stopped turn the way the daemon's runner does
//! (`Agent::settle_interrupted_turn`) and look at the next request. Every
//! `timeout` is a failure bound around a handshake, never a guess about
//! scheduling.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use biorouter::agents::stopped_turn::TURN_STOPPED_NOTICE;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::providers::base::{
    MessageStream, Provider, ProviderMetadata, ProviderStreamItem, ProviderUsage, Usage,
};
use biorouter::providers::errors::ProviderError;
use biorouter::providers::formats::{anthropic, openai};
use biorouter::providers::utils::ImageFormat;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use futures::StreamExt;
use rmcp::model::Tool;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Every bound in this file. It only decides how long a FAILING test takes to
/// say so.
const BOUND: Duration = Duration::from_secs(30);

/// What the stalled first reply had streamed when the person cancelled it.
const HALF_REPLY: &str = "The telescope was invented in";

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

fn usage() -> ProviderUsage {
    ProviderUsage::new(
        "mock-model".to_string(),
        Usage::new(Some(10), Some(5), Some(15)),
    )
}

/// A streaming provider whose first request streams `first_chunk` (when there
/// is one) and then never ends, so only a cancel can end that turn. Every later
/// request is answered at once, and every request is kept for the assertions.
struct StallingProvider {
    calls: AtomicUsize,
    first_chunk: Option<&'static str>,
    /// Told when the first request has streamed what it will stream.
    reached: Mutex<Option<mpsc::UnboundedSender<()>>>,
    seen: Mutex<Vec<Vec<Message>>>,
}

impl StallingProvider {
    fn new(first_chunk: Option<&'static str>) -> (Arc<Self>, mpsc::UnboundedReceiver<()>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                first_chunk,
                reached: Mutex::new(Some(tx)),
                seen: Mutex::new(Vec::new()),
            }),
            rx,
        )
    }

    fn request(&self, n: usize) -> Vec<Message> {
        self.seen
            .lock()
            .unwrap()
            .get(n)
            .cloned()
            .unwrap_or_default()
    }
}

#[async_trait]
impl Provider for StallingProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        // A title or summary request: answered, never counted as a turn.
        Ok((Message::assistant().with_text("A title"), usage()))
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
        _tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(messages.to_vec());
        if n == 0 {
            let reached = self.reached.lock().unwrap().take();
            let first: Vec<Result<ProviderStreamItem, ProviderError>> = self
                .first_chunk
                .map(|text| {
                    Ok((
                        Some(Message::assistant().with_id("reply-1").with_text(text)),
                        None,
                        None,
                    ))
                })
                .into_iter()
                .collect();
            let told = futures::stream::once(async move {
                if let Some(reached) = reached {
                    let _ = reached.send(());
                }
            })
            .filter_map(|()| async { None::<Result<ProviderStreamItem, ProviderError>> });
            return Ok(Box::pin(
                futures::stream::iter(first)
                    .chain(told)
                    .chain(futures::stream::pending()),
            ));
        }
        let item: ProviderStreamItem = (
            Some(Message::assistant().with_text("Answered.")),
            Some(usage()),
            None,
        );
        Ok(Box::pin(futures::stream::iter(vec![Ok(item)])))
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new("mock-model").unwrap()
    }

    fn metadata() -> ProviderMetadata {
        ProviderMetadata {
            name: "mock".to_string(),
            display_name: "Mock Provider".to_string(),
            description: "Mock provider for the cancel note tests".to_string(),
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
        "mock-cancel-note"
    }
}

struct Chat {
    agent: Arc<Agent>,
    session_id: String,
    sessions: Arc<SessionManager>,
    _dirs: (TempDir, TempDir),
}

async fn chat(provider: Arc<StallingProvider>) -> Chat {
    let work_dir = TempDir::new().unwrap();
    let data_dir = TempDir::new().unwrap();
    let sessions = Arc::new(SessionManager::new(data_dir.path().to_path_buf()));
    let agent = Agent::with_config(AgentConfig::new(
        Arc::clone(&sessions),
        PermissionManager::instance(),
        None,
        BioRouterMode::Auto,
    ));
    let session = sessions
        .create_session(
            work_dir.path().to_path_buf(),
            "cancelled-turn-note".to_string(),
            SessionType::Hidden,
        )
        .await
        .unwrap();
    agent
        .update_provider(provider as Arc<dyn Provider>, &session.id)
        .await
        .unwrap();
    agent
        .override_system_prompt("You are a test agent.".to_string())
        .await;
    Chat {
        agent: Arc::new(agent),
        session_id: session.id,
        sessions,
        _dirs: (work_dir, data_dir),
    }
}

fn config(session_id: &str) -> SessionConfig {
    SessionConfig {
        id: session_id.to_string(),
        schedule_id: None,
        max_turns: Some(4),
        max_tool_calls: None,
        retry_config: None,
        budget: None,
        reasoning_effort: None,
    }
}

impl Chat {
    /// Start a turn, wait until its stalled request has streamed, then cancel it
    /// and settle it the way the daemon's runner settles a Stop: drop the
    /// stream, then hand the agent what it had streamed.
    async fn cancel_a_turn(&self, reached: &mut mpsc::UnboundedReceiver<()>) {
        let cancel = CancellationToken::new();
        let mut in_flight: Vec<Message> = Vec::new();
        {
            let stream = self
                .agent
                .reply(
                    Message::user().with_text("When was the telescope invented?"),
                    config(&self.session_id),
                    Some(cancel.clone()),
                )
                .await
                .expect("the reply stream opens");
            tokio::pin!(stream);
            tokio::time::timeout(BOUND, async {
                loop {
                    tokio::select! {
                        biased;
                        Some(()) = reached.recv() => break,
                        event = stream.next() => match event {
                            Some(Ok(AgentEvent::Message(message))) => in_flight.push(message),
                            Some(Ok(AgentEvent::MessagesPersisted(_))) => in_flight.clear(),
                            Some(Ok(_)) => {}
                            Some(Err(e)) => panic!("the turn failed before the cancel: {e}"),
                            None => panic!("the turn ended before it was cancelled"),
                        },
                    }
                }
                // Drain what the provider already handed over, so the half
                // reply the person was reading is what the runner holds.
                while let Ok(Some(event)) =
                    tokio::time::timeout(Duration::from_millis(200), stream.next()).await
                {
                    match event {
                        Ok(AgentEvent::Message(message)) => in_flight.push(message),
                        Ok(AgentEvent::MessagesPersisted(_)) => in_flight.clear(),
                        Ok(_) => {}
                        Err(e) => panic!("the turn failed before the cancel: {e}"),
                    }
                }
            })
            .await
            .expect("the first request streams");
            cancel.cancel();
        }
        self.agent
            .settle_interrupted_turn(&self.session_id, &in_flight, true)
            .await
            .expect("the stopped turn settles");
    }

    async fn run_turn(&self, text: &str) {
        let stream = self
            .agent
            .reply(
                Message::user().with_text(text),
                config(&self.session_id),
                None,
            )
            .await
            .expect("the reply stream opens");
        tokio::pin!(stream);
        tokio::time::timeout(BOUND, async {
            while let Some(event) = stream.next().await {
                event.expect("the turn does not error");
            }
        })
        .await
        .expect("the turn ends");
    }

    async fn stored(&self) -> Vec<Message> {
        self.sessions
            .get_session(&self.session_id, true)
            .await
            .unwrap()
            .conversation
            .unwrap_or_default()
            .messages()
            .to_vec()
    }
}

/// The stored rows that are the cancel note: by its text, in any shape, so the
/// count says nothing about the shape and the shape is asserted on its own.
fn notes(messages: &[Message]) -> Vec<&Message> {
    messages
        .iter()
        .filter(|message| {
            message.content.iter().any(|content| match content {
                MessageContent::Text(text) => text.text.trim() == TURN_STOPPED_NOTICE,
                MessageContent::SystemNotification(notice) => notice.msg == TURN_STOPPED_NOTICE,
                _ => false,
            })
        })
        .collect()
}

/// How often `needle` appears in what the next request hands the model, as the
/// provider got it and as two wire formats write it.
fn occurrences_in_request(messages: &[Message], needle: &str) -> [usize; 3] {
    let texts: usize = messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter(
            |content| matches!(content, MessageContent::Text(text) if text.text.contains(needle)),
        )
        .count();
    let anthropic = serde_json::to_string(&anthropic::format_messages(messages)).unwrap();
    let openai =
        serde_json::to_string(&openai::format_messages(messages, &ImageFormat::OpenAi)).unwrap();
    [
        texts,
        anthropic.matches(needle).count(),
        openai.matches(needle).count(),
    ]
}

/// Every place the note appears in a request opens its own paragraph. The half
/// reply and the note are two assistant rows the request sends as one reply,
/// and their texts are joined as written: without the break the model read
/// "The telescope was invented inThe person cancelled…".
fn assert_the_note_stands_apart(messages: &[Message]) {
    for content in messages.iter().flat_map(|message| message.content.iter()) {
        let MessageContent::Text(text) = content else {
            continue;
        };
        for (at, _) in text.text.match_indices(TURN_STOPPED_NOTICE) {
            assert!(
                at == 0
                    || text
                        .text
                        .get(..at)
                        .is_some_and(|prefix| prefix.ends_with('\n')),
                "the note runs on from the text before it: {:?}",
                text.text
            );
        }
    }
}

/// The whole promise for the commonest cancel: the person stops a reply they
/// were reading. One note is stored after the reply's prose, both the person
/// and the model can see it, and the next request carries it exactly once,
/// whatever the wire format.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_turn_leaves_one_note_the_model_reads() {
    let (provider, mut reached) = StallingProvider::new(Some(HALF_REPLY));
    let chat = chat(Arc::clone(&provider)).await;

    chat.cancel_a_turn(&mut reached).await;

    let stored = chat.stored().await;
    let found = notes(&stored);
    assert_eq!(found.len(), 1, "one note for one cancel: {stored:#?}");
    let note = found[0];
    assert!(
        note.is_user_visible() && note.is_agent_visible(),
        "the person and the model both read the note: {note:?}"
    );
    assert!(
        matches!(note.content.as_slice(), [MessageContent::Text(_)]),
        "the note is text, the one shape every wire format carries: {note:?}"
    );
    let at = stored
        .iter()
        .position(|message| std::ptr::eq(message, note))
        .unwrap();
    assert!(
        stored[..at]
            .iter()
            .any(|message| message.as_concat_text() == HALF_REPLY),
        "the note comes after the prose the person was reading: {stored:#?}"
    );

    chat.run_turn("Go on.").await;

    let next = provider.request(1);
    assert!(
        next.last()
            .is_some_and(|message| message.as_concat_text().contains("Go on.")),
        "premise: the second request is the next turn's: {next:#?}"
    );
    assert_eq!(
        occurrences_in_request(&next, TURN_STOPPED_NOTICE),
        [1, 1, 1],
        "the next request carries the note exactly once, as the provider gets it \
         and as the Anthropic and OpenAI formats write it: {next:#?}"
    );
    assert_eq!(
        occurrences_in_request(&next, HALF_REPLY),
        [1, 1, 1],
        "and the half reply once beside it: {next:#?}"
    );
    assert_the_note_stands_apart(&next);
    assert_eq!(
        notes(&chat.stored().await).len(),
        1,
        "the next turn adds no second note"
    );
}

/// The same when nothing had streamed yet: the person stopped a turn that was
/// still waiting on its model. The note is then the only thing in the agent's
/// place, and the next request still carries it once.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_before_any_reply_leaves_one_note_the_model_reads() {
    let (provider, mut reached) = StallingProvider::new(None);
    let chat = chat(Arc::clone(&provider)).await;

    chat.cancel_a_turn(&mut reached).await;
    assert_eq!(notes(&chat.stored().await).len(), 1);

    chat.run_turn("Try again, briefly.").await;

    let next = provider.request(1);
    assert_eq!(
        occurrences_in_request(&next, TURN_STOPPED_NOTICE),
        [1, 1, 1],
        "{next:#?}"
    );
    assert_the_note_stands_apart(&next);
}

/// A cancel that lands after the turn finished is not a cancel of anything:
/// the reply is whole, and nothing in the store says otherwise. The agent loop
/// itself never writes the note; only a door that saw its token end an
/// unfinished turn does (the daemon's runner, ACP, the terminal).
#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_after_the_turn_finished_leaves_no_note() {
    let (provider, _reached) = StallingProvider::new(None);
    // The first request is the stalled one; answer it by moving past it.
    provider.calls.store(1, Ordering::SeqCst);
    let chat = chat(Arc::clone(&provider)).await;

    let cancel = CancellationToken::new();
    {
        let stream = chat
            .agent
            .reply(
                Message::user().with_text("When was the telescope invented?"),
                config(&chat.session_id),
                Some(cancel.clone()),
            )
            .await
            .expect("the reply stream opens");
        tokio::pin!(stream);
        tokio::time::timeout(BOUND, async {
            while let Some(event) = stream.next().await {
                event.expect("the turn does not error");
            }
        })
        .await
        .expect("the turn ends");
    }
    cancel.cancel();

    let stored = chat.stored().await;
    assert!(
        stored
            .iter()
            .any(|message| message.as_concat_text() == "Answered."),
        "premise: the turn finished: {stored:#?}"
    );
    assert!(notes(&stored).is_empty(), "{stored:#?}");

    chat.run_turn("Thanks.").await;
    assert_eq!(
        occurrences_in_request(&provider.request(1), TURN_STOPPED_NOTICE),
        [0, 0, 0]
    );
}

/// The note is a record of what happened, not an instruction. It names who
/// did what, and nothing in it tells the model to ignore, drop, answer, resume
/// or continue the earlier request: the next message decides that.
#[test]
fn the_note_does_not_tell_the_model_what_to_do() {
    let note = TURN_STOPPED_NOTICE.to_lowercase();
    assert!(
        note.contains("person") && note.contains("canceled"),
        "the note says who cancelled the turn: {TURN_STOPPED_NOTICE:?}"
    );
    for instruction in [
        "ignore",
        "disregard",
        "drop",
        "do not",
        "don't",
        "answer",
        "respond",
        "resume",
        "continue",
        "retry",
        "should",
        "must",
        "please",
    ] {
        assert!(
            !note.contains(instruction),
            "the note tells the model what to do ({instruction:?}): {TURN_STOPPED_NOTICE:?}"
        );
    }
}
