//! End-to-end agent-loop coverage for the conversation-writeback freshness
//! discipline.
//!
//! `replace_conversation` DELETEs and re-INSERTs a session's ENTIRE message set,
//! so any caller that computed its new conversation from a snapshot destroys
//! whatever landed in between. BR-12 gave the *background* eager-compaction path
//! a freshness check; the in-turn compaction sites never got one.
//!
//! These tests drive the **real** reply loop with a mock provider (no network,
//! no keychain). The provider appends a foreign message from inside its own
//! completion call, which makes the race deterministic — no sleeps, no barriers.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::Result;
use async_trait::async_trait;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::{SessionType, DB_NAME, SESSIONS_FOLDER};
use biorouter::session::SessionManager;
use futures::StreamExt;
use rmcp::model::Tool;
use tempfile::TempDir;

/// A summary that clears `summary_is_usable` (>= 40 chars, >= 3 mandated
/// section headings), so compaction accepts it on the first attempt.
const GOOD_SUMMARY: &str = "## User Intent\nThe user asked for a plot.\n\
     ## Technical Concepts\nPlotting, data frames.\n\
     ## Files\nnone\n## Pending Tasks\nnone\n";

/// The one user message text every test seeds the turn with.
const USER_PROMPT: &str = "Plot the data";

/// A mock provider that can (a) append a foreign message to the session from
/// inside a completion — the deterministic stand-in for a concurrent appender
/// such as BR-71's note tool or `biorouter term log` — and (b) fail a given
/// number of main-loop completions with `ContextLengthExceeded` to drive the
/// overflow-recovery ladder.
struct RaceProvider {
    session_manager: Arc<SessionManager>,
    session_id: OnceLock<String>,
    /// Completions that are the agent loop's own (not the summarizer's).
    main_calls: AtomicUsize,
    /// Completions issued by the compaction summarizer.
    summarizer_calls: AtomicUsize,
    /// Zero-based main-call indices that return `ContextLengthExceeded`.
    overflow_on: Vec<usize>,
    /// Notes to append, keyed by the zero-based *summarizer* call index they
    /// should land during. Appending from inside the summarizer call is exactly
    /// the window the freshness discipline protects.
    notes_during_summarization: Vec<(usize, String)>,
    /// #51: the same, but the appended note carries the PRESERVATION MARKER.
    /// Together these two cover the whole promise — part (a) has to carry the
    /// note over the write-back, part (b) has to keep it out of the summary
    /// once it ages past the verbatim window.
    pinned_notes_during_summarization: Vec<(usize, String)>,
    /// Summarizer call indices during which the session's whole history is
    /// wholesale-rewritten by someone else, which moves the basis and makes the
    /// caller's write-back stale.
    rewrites_during_summarization: Vec<usize>,
    /// Notes to append, keyed by the zero-based MAIN-loop call index they land
    /// during. Models a writer that persists to the session mid-turn without
    /// the reply loop ever pushing the message into its in-memory conversation
    /// — `drain_elicitation_messages`, `biorouter term log`, a note tool.
    notes_during_main_call: Vec<(usize, String)>,
    /// #59: main-call indices that answer with THINKING plus two tool calls.
    /// One streamed message, which the loop splits into three stored assistant
    /// rows under three different ids.
    tools_on: Vec<usize>,
    /// #59: main-call indices that fail with a *recoverable* provider error, so
    /// BR-66 absorbs it and persists its hint as a model-only row the user is
    /// deliberately never shown.
    server_error_on: Vec<usize>,
    /// Whether a `tools_on` reply signs its reasoning. A NON-EMPTY signature is
    /// what `has_signed_reasoning` keys on, and it selects a different storage
    /// path in the reply loop: signed replies are rebuilt into one canonical
    /// assistant row (the signature authenticates that exact block list), while
    /// unsigned ones keep the historical split into several rows. Default
    /// **false**, so a test opts in to the signed path deliberately rather than
    /// drifting onto it because a fixture happened to carry a signature.
    sign_reasoning: bool,
    /// Zero-based *summarizer* call indices that fail outright. Models the
    /// provider going away between a first summarization and its retry.
    fail_summarization_on: Vec<usize>,
    context_limit: usize,
    /// Message texts the provider saw on each main-loop call.
    seen: Mutex<Vec<Vec<String>>>,
    seen_reasoning: Mutex<Vec<bool>>,
    /// #51: the summarizer carries the history it is asked to condense in the
    /// SYSTEM prompt, so this is how a test asserts what did — and did not —
    /// reach it.
    summarizer_payloads: Mutex<Vec<String>>,
}

impl RaceProvider {
    fn new(session_manager: Arc<SessionManager>) -> Self {
        Self {
            session_manager,
            session_id: OnceLock::new(),
            main_calls: AtomicUsize::new(0),
            summarizer_calls: AtomicUsize::new(0),
            overflow_on: Vec::new(),
            notes_during_summarization: Vec::new(),
            pinned_notes_during_summarization: Vec::new(),
            rewrites_during_summarization: Vec::new(),
            notes_during_main_call: Vec::new(),
            tools_on: Vec::new(),
            server_error_on: Vec::new(),
            sign_reasoning: false,
            fail_summarization_on: Vec::new(),
            context_limit: 200_000,
            seen: Mutex::new(Vec::new()),
            seen_reasoning: Mutex::new(Vec::new()),
            summarizer_payloads: Mutex::new(Vec::new()),
        }
    }

    fn overflow_on(mut self, calls: &[usize]) -> Self {
        self.overflow_on = calls.to_vec();
        self
    }

    fn fail_summarization_on(mut self, call: usize) -> Self {
        self.fail_summarization_on.push(call);
        self
    }

    fn note_during_summarization(mut self, call: usize, text: &str) -> Self {
        self.notes_during_summarization
            .push((call, text.to_string()));
        self
    }

    /// #51: append a note that carries the preservation marker.
    fn pinned_note_during_summarization(mut self, call: usize, text: &str) -> Self {
        self.pinned_notes_during_summarization
            .push((call, text.to_string()));
        self
    }

    fn note_during_main_call(mut self, call: usize, text: &str) -> Self {
        self.notes_during_main_call.push((call, text.to_string()));
        self
    }

    /// #59: answer this main call with thinking + two tool calls.
    fn tools_on(mut self, call: usize) -> Self {
        self.tools_on.push(call);
        self
    }

    /// Sign the reasoning on `tools_on` replies, selecting the signed-provider
    /// storage path (Bedrock extended thinking, Anthropic direct, Snowflake).
    fn sign_reasoning(mut self) -> Self {
        self.sign_reasoning = true;
        self
    }

    /// #59: fail this main call with a recoverable provider error.
    fn server_error_on(mut self, call: usize) -> Self {
        self.server_error_on.push(call);
        self
    }

    fn rewrite_during_summarization(mut self, call: usize) -> Self {
        self.rewrites_during_summarization.push(call);
        self
    }

    fn context_limit(mut self, limit: usize) -> Self {
        self.context_limit = limit;
        self
    }

    fn main_call_count(&self) -> usize {
        self.main_calls.load(Ordering::SeqCst)
    }

    fn summarizer_call_count(&self) -> usize {
        self.summarizer_calls.load(Ordering::SeqCst)
    }

    /// Everything every summarization round-trip was handed, concatenated.
    fn all_summarizer_payloads(&self) -> String {
        self.summarizer_payloads.lock().unwrap().join("\n")
    }

    fn texts_seen_on_main_call(&self, n: usize) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .get(n)
            .cloned()
            .unwrap_or_default()
    }

    /// The summarizer sends exactly one user message with a fixed prompt and
    /// carries the history in the system prompt; the agent loop never does.
    fn is_summarizer_call(messages: &[Message]) -> bool {
        messages.len() == 1
            && messages[0].content.iter().any(|c| {
                matches!(c, MessageContent::Text(t)
                    if t.text.contains("Please summarize the conversation history"))
            })
    }
}

#[async_trait]
impl Provider for RaceProvider {
    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        system_prompt: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let usage = ProviderUsage::new(
            "mock-model".to_string(),
            Usage::new(Some(10), Some(5), Some(15)),
        );

        if Self::is_summarizer_call(messages) {
            let n = self.summarizer_calls.fetch_add(1, Ordering::SeqCst);
            self.summarizer_payloads
                .lock()
                .unwrap()
                .push(system_prompt.to_string());
            if self.fail_summarization_on.contains(&n) {
                return Err(ProviderError::RequestFailed(
                    "mock summarizer failure".to_string(),
                ));
            }
            // Append the foreign message *during* the summarization round-trip:
            // after the caller took its snapshot, before it writes back.
            for (call, text) in &self.notes_during_summarization {
                if *call == n {
                    let id = self.session_id.get().expect("session id set");
                    self.session_manager
                        .add_message(id, &Message::user().with_text(text.clone()))
                        .await
                        .expect("note append");
                }
            }
            for (call, text) in &self.pinned_notes_during_summarization {
                if *call == n {
                    let id = self.session_id.get().expect("session id set");
                    self.session_manager
                        .add_message(id, &Message::user().with_text(text.clone()).pinned())
                        .await
                        .expect("pinned note append");
                }
            }
            if self.rewrites_during_summarization.contains(&n) {
                let id = self.session_id.get().expect("session id set");
                let current = self
                    .session_manager
                    .get_session(id, true)
                    .await
                    .expect("read")
                    .conversation
                    .expect("conversation");
                // A wholesale rewrite renumbers every row, so the caller's
                // basis prefix vanishes: the definition of a moved basis.
                self.session_manager
                    .replace_conversation(id, &current)
                    .await
                    .expect("rewrite");
            }
            return Ok((Message::assistant().with_text(GOOD_SUMMARY), usage));
        }

        let n = self.main_calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(
            messages
                .iter()
                .flat_map(|m| m.content.iter())
                .filter_map(|c| match c {
                    MessageContent::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .collect(),
        );
        self.seen_reasoning.lock().unwrap().push(
            messages
                .iter()
                .flat_map(|message| &message.content)
                .any(|content| {
                    matches!(
                        content,
                        MessageContent::Thinking(_) | MessageContent::RedactedThinking(_)
                    )
                }),
        );

        for (call, text) in &self.notes_during_main_call {
            if *call == n {
                let id = self.session_id.get().expect("session id set");
                self.session_manager
                    .add_message(id, &Message::user().with_text(text.clone()))
                    .await
                    .expect("mid-turn append");
            }
        }

        if self.overflow_on.contains(&n) {
            return Err(ProviderError::ContextLengthExceeded(
                "mock overflow".to_string(),
            ));
        }

        if self.server_error_on.contains(&n) {
            // Recoverable (see `mistakes::is_recoverable`), so BR-66 absorbs it
            // and stores a model-only hint instead of ending the turn.
            return Err(ProviderError::ServerError("mock 503".to_string()));
        }

        if self.tools_on.contains(&n) {
            let mut reply = Message::assistant()
                .with_thinking(
                    "weighing two options",
                    if self.sign_reasoning { "sig-mock" } else { "" },
                )
                .with_text("Calling two tools.");
            for i in 0..2 {
                reply = reply.with_tool_request(
                    format!("call-{i}"),
                    Ok(rmcp::model::CallToolRequestParams {
                        task: None,
                        name: "definitely_not_a_real_tool".into(),
                        arguments: None,
                        meta: None,
                    }),
                );
            }
            return Ok((reply, usage));
        }

        Ok((Message::assistant().with_text("All done."), usage))
    }

    fn get_model_config(&self) -> ModelConfig {
        let mut config = ModelConfig::new("mock-model").unwrap();
        config.context_limit = Some(self.context_limit);
        config
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
        "mock-test"
    }
}

struct Harness {
    agent: Arc<Agent>,
    session_id: String,
    session_manager: Arc<SessionManager>,
    /// Where the session store's sqlite file lives, so a test can reach the
    /// database directly (see `fail_to_store_the_summary`).
    data_dir: PathBuf,
    _work_dir: TempDir,
}

async fn harness(build: impl FnOnce(RaceProvider) -> RaceProvider) -> (Harness, Arc<RaceProvider>) {
    let work_dir = TempDir::new().unwrap();
    let data_dir = TempDir::new().unwrap();
    let data_path = data_dir.path().to_path_buf();
    let session_manager = Arc::new(SessionManager::new(data_dir.path().to_path_buf()));

    let provider = Arc::new(build(RaceProvider::new(session_manager.clone())));

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
            "writeback-freshness-test".to_string(),
            SessionType::Hidden,
        )
        .await
        .unwrap();
    provider.session_id.set(session.id.clone()).unwrap();

    agent
        .update_provider(provider.clone() as Arc<dyn Provider>, &session.id)
        .await
        .unwrap();

    // The session store lives under data_dir for the life of the agent.
    std::mem::forget(data_dir);

    (
        Harness {
            agent: Arc::new(agent),
            session_id: session.id,
            session_manager,
            data_dir: data_path,
            _work_dir: work_dir,
        },
        provider,
    )
}

impl Harness {
    async fn run_turn(&self, prompt: &str) -> Result<Vec<AgentEvent>> {
        self.run_turn_with(Message::user().with_text(prompt)).await
    }

    /// The same turn, for a prompt that is not plain text — an elicitation
    /// answer, say, which `reply()` handles before it reaches the loop at all.
    async fn run_turn_with(&self, message: Message) -> Result<Vec<AgentEvent>> {
        let session_config = SessionConfig {
            id: self.session_id.clone(),
            schedule_id: None,
            max_turns: Some(8),
            max_tool_calls: None,
            retry_config: None,
            budget: None,
            reasoning_effort: None,
        };
        let stream = self.agent.reply(message, session_config, None).await?;
        tokio::pin!(stream);
        let mut out = Vec::new();
        while let Some(ev) = stream.next().await {
            out.push(ev?);
        }
        Ok(out)
    }

    async fn stored_texts(&self) -> Vec<String> {
        self.session_manager
            .get_session(&self.session_id, true)
            .await
            .unwrap()
            .conversation
            .unwrap()
            .messages()
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|c| match c {
                MessageContent::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect()
    }

    /// How many stored messages carry `needle`.
    ///
    /// A recovered row is re-inserted, so "did it survive" and "was it survived
    /// exactly once" are different questions: a basis split can equally well
    /// produce a DUPLICATE (the row is recovered onto the tail of a replacement
    /// that already contains it) instead of a deletion.
    async fn stored_occurrences(&self, needle: &str) -> usize {
        self.stored_texts()
            .await
            .iter()
            .filter(|t| t.contains(needle))
            .count()
    }
}

/// The user-facing system notifications the turn emitted.
fn notification_texts(events: &[AgentEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|ev| match ev {
            AgentEvent::Message(m) => Some(m),
            _ => None,
        })
        .flat_map(|m| m.content.iter())
        .filter_map(|c| c.as_system_notification().map(|n| n.msg.clone()))
        .collect()
}

fn history_replaced_texts(events: &[AgentEvent]) -> Option<Vec<String>> {
    events.iter().rev().find_map(|ev| match ev {
        AgentEvent::HistoryReplaced(conv) => Some(
            conv.messages()
                .iter()
                .flat_map(|m| m.content.iter())
                .filter_map(|c| match c {
                    MessageContent::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .collect(),
        ),
        _ => None,
    })
}

// ── F1: the headline case ────────────────────────────────────────────────────

/// A message appended to a live session while the overflow-recovery summarizer
/// runs must survive the writeback. This is the BR-71 `mode: "note"` case, and
/// the shipped `biorouter term log` case: the tool already returned success, so
/// losing the row here is a silent lie.
#[tokio::test(flavor = "multi_thread")]
async fn overflow_recovery_preserves_a_concurrent_note() {
    let (h, provider) = harness(|p| {
        p.overflow_on(&[0])
            .note_during_summarization(0, "NOTE: reviewer asked for log scale")
    })
    .await;

    let events = h.run_turn(USER_PROMPT).await.unwrap();

    // Liveness: the turn still completed after recovering from the overflow.
    assert_eq!(
        provider.main_call_count(),
        2,
        "the loop must retry after the recovery compaction"
    );
    assert_eq!(provider.summarizer_call_count(), 1);

    let stored = h.stored_texts().await;
    assert!(
        stored
            .iter()
            .any(|t| t.contains("NOTE: reviewer asked for log scale")),
        "the concurrently appended note must survive the writeback; stored: {stored:#?}"
    );

    // ...and the client's view agrees with the store.
    let replaced = history_replaced_texts(&events).expect("HistoryReplaced emitted");
    assert!(
        replaced
            .iter()
            .any(|t| t.contains("NOTE: reviewer asked for log scale")),
        "HistoryReplaced must carry the preserved note; got: {replaced:#?}"
    );
}

// ── F2: the anti-false-conflict twin ─────────────────────────────────────────

/// With no concurrent writer the recovery compaction must still be PERSISTED.
/// A freshness guard that misfires here silently disables durable compaction —
/// nothing user-visible breaks, the session just grows forever.
#[tokio::test(flavor = "multi_thread")]
async fn overflow_recovery_persists_when_nothing_else_wrote() {
    let (h, provider) = harness(|p| p.overflow_on(&[0])).await;

    let events = h.run_turn(USER_PROMPT).await.unwrap();

    assert_eq!(provider.main_call_count(), 2);
    let stored = h.stored_texts().await;
    assert!(
        stored.iter().any(|t| t.contains("User Intent")),
        "the compaction summary must be persisted; stored: {stored:#?}"
    );
    assert!(
        history_replaced_texts(&events).is_some(),
        "HistoryReplaced must be emitted when the store really was replaced"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn overflow_retry_omits_historical_reasoning_but_durable_compaction_keeps_it() {
    let (h, provider) = harness(|p| p.overflow_on(&[0])).await;
    h.session_manager
        .add_message(
            &h.session_id,
            &Message::assistant()
                .with_thinking("historical private reasoning", "historical-signature")
                .with_redacted_thinking("historical-redacted-bytes")
                .with_text("historical visible answer"),
        )
        .await
        .unwrap();

    let events = h.run_turn(USER_PROMPT).await.unwrap();

    assert_eq!(provider.main_call_count(), 2);
    assert_eq!(
        *provider.seen_reasoning.lock().unwrap(),
        vec![false, false],
        "neither the initial new-user call nor its immediate overflow retry may receive historical Bedrock reasoning"
    );
    let stored = h
        .session_manager
        .get_session(&h.session_id, true)
        .await
        .unwrap()
        .conversation
        .unwrap();
    assert!(stored.iter().flat_map(|message| &message.content).any(
        |content| matches!(content, MessageContent::Thinking(value) if value.signature == "historical-signature")
    ));
    assert!(stored
        .iter()
        .flat_map(|message| &message.content)
        .any(|content| matches!(content, MessageContent::RedactedThinking(value) if value.data == "historical-redacted-bytes")));
    let replaced = events
        .iter()
        .find_map(|event| match event {
            AgentEvent::HistoryReplaced(conversation) => Some(conversation),
            _ => None,
        })
        .expect("persisted overflow compaction emits raw replacement history");
    assert!(replaced.iter().flat_map(|message| &message.content).any(
        |content| matches!(content, MessageContent::Thinking(value) if value.signature == "historical-signature")
    ));
    assert!(replaced
        .iter()
        .flat_map(|message| &message.content)
        .any(|content| matches!(content, MessageContent::RedactedThinking(value) if value.data == "historical-redacted-bytes")));
}

// ── F4: two overflows in one turn ────────────────────────────────────────────

/// A second overflow later in the same turn must ALSO persist. The first
/// recovery rewrote every row (new rowids, a minted uid for the summary), so a
/// basis captured once at turn start is stale by construction — it has to be
/// re-seeded after each successful swap.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_overflow_in_one_turn_still_persists() {
    let (h, provider) = harness(|p| p.overflow_on(&[0, 1])).await;

    h.run_turn(USER_PROMPT).await.unwrap();

    assert_eq!(
        provider.main_call_count(),
        3,
        "two overflows, then a success"
    );
    assert_eq!(
        provider.summarizer_call_count(),
        2,
        "each overflow runs its own recovery compaction"
    );

    let stored = h.stored_texts().await;
    assert!(
        stored.iter().any(|t| t.contains("User Intent")),
        "the second recovery compaction must be persisted too; stored: {stored:#?}"
    );
}

// ── F3: overflow recovery on a turn that also auto-compacted ─────────────────

/// The in-turn auto-compaction at the top of `reply()` rewrites every row
/// before the loop starts. Overflow recovery later in the same turn must still
/// persist — i.e. its freshness basis is captured AFTER that rewrite.
#[tokio::test(flavor = "multi_thread")]
async fn overflow_recovery_persists_on_a_turn_that_also_auto_compacted() {
    // A tiny context limit plus a live gauge over it forces the synchronous
    // auto-compaction at the top of reply().
    let (h, provider) = harness(|p| p.overflow_on(&[0]).context_limit(100)).await;

    for i in 0..6 {
        h.session_manager
            .add_message(&h.session_id, &Message::user().with_text(format!("q{i}")))
            .await
            .unwrap();
        h.session_manager
            .add_message(
                &h.session_id,
                &Message::assistant().with_text(format!("a{i}")),
            )
            .await
            .unwrap();
    }
    h.session_manager
        .update(&h.session_id)
        .total_tokens(Some(95))
        .apply()
        .await
        .unwrap();

    h.run_turn(USER_PROMPT).await.unwrap();

    assert!(
        provider.summarizer_call_count() >= 2,
        "an auto-compaction and a recovery compaction should both have run, saw {}",
        provider.summarizer_call_count()
    );

    // Both summaries are on disk: the auto-compaction's and the recovery's. A
    // basis captured before the auto-compaction would make the recovery swap
    // look stale, and only the first summary would survive.
    let stored = h.stored_texts().await;
    let summaries = stored.iter().filter(|t| t.contains("User Intent")).count();
    assert!(
        summaries >= 2,
        "the recovery compaction must persist even after an auto-compaction \
         renumbered every row (expected both summaries, saw {summaries}); stored: {stored:#?}"
    );
}

// ── auto-compaction (agent.rs:3061) ──────────────────────────────────────────

/// The same race one level up: a note that lands while the *auto*-compaction
/// summarizer runs must survive, and must reach the model this turn.
#[tokio::test(flavor = "multi_thread")]
async fn auto_compaction_preserves_a_concurrent_note() {
    let (h, provider) = harness(|p| {
        p.context_limit(100)
            .note_during_summarization(0, "NOTE: use the 2024 cohort")
    })
    .await;

    for i in 0..6 {
        h.session_manager
            .add_message(&h.session_id, &Message::user().with_text(format!("q{i}")))
            .await
            .unwrap();
        h.session_manager
            .add_message(
                &h.session_id,
                &Message::assistant().with_text(format!("a{i}")),
            )
            .await
            .unwrap();
    }
    h.session_manager
        .update(&h.session_id)
        .total_tokens(Some(95))
        .apply()
        .await
        .unwrap();

    h.run_turn(USER_PROMPT).await.unwrap();

    assert_eq!(provider.summarizer_call_count(), 1);
    let stored = h.stored_texts().await;
    assert!(
        stored
            .iter()
            .any(|t| t.contains("NOTE: use the 2024 cohort")),
        "the note appended during auto-compaction must survive; stored: {stored:#?}"
    );
    assert!(
        provider
            .texts_seen_on_main_call(0)
            .iter()
            .any(|t| t.contains("NOTE: use the 2024 cohort")),
        "the preserved note must also be in the model's context for this turn, saw: {:?}",
        provider.texts_seen_on_main_call(0)
    );
}

/// Anti-false-conflict twin for the auto-compaction site.
#[tokio::test(flavor = "multi_thread")]
async fn auto_compaction_still_persists_with_no_concurrent_writer() {
    let (h, provider) = harness(|p| p.context_limit(100)).await;

    for i in 0..6 {
        h.session_manager
            .add_message(&h.session_id, &Message::user().with_text(format!("q{i}")))
            .await
            .unwrap();
        h.session_manager
            .add_message(
                &h.session_id,
                &Message::assistant().with_text(format!("a{i}")),
            )
            .await
            .unwrap();
    }
    h.session_manager
        .update(&h.session_id)
        .total_tokens(Some(95))
        .apply()
        .await
        .unwrap();

    h.run_turn(USER_PROMPT).await.unwrap();

    assert_eq!(provider.summarizer_call_count(), 1);
    let stored = h.stored_texts().await;
    assert!(
        stored.iter().any(|t| t.contains("User Intent")),
        "the auto-compaction summary must be persisted; stored: {stored:#?}"
    );
}

/// When the basis moves out from under the auto-compaction (a checkpoint
/// restore, a message edit, another turn's rewrite) the swap is declined — and
/// that must NOT fail the turn. Trading a data-loss bug for a liveness bug is
/// not a fix. The turn proceeds on the FRESH history, so whatever landed is in
/// the model's context this turn.
#[tokio::test(flavor = "multi_thread")]
async fn auto_compaction_stale_continues_the_turn_uncompacted() {
    let (h, provider) = harness(|p| p.context_limit(100).rewrite_during_summarization(0)).await;

    for i in 0..6 {
        h.session_manager
            .add_message(&h.session_id, &Message::user().with_text(format!("q{i}")))
            .await
            .unwrap();
        h.session_manager
            .add_message(
                &h.session_id,
                &Message::assistant().with_text(format!("a{i}")),
            )
            .await
            .unwrap();
    }
    h.session_manager
        .update(&h.session_id)
        .total_tokens(Some(95))
        .apply()
        .await
        .unwrap();

    let events = h.run_turn(USER_PROMPT).await.unwrap();

    // Liveness: the turn ran to completion.
    assert_eq!(provider.main_call_count(), 1, "the turn must still run");
    assert!(
        notification_texts(&events)
            .iter()
            .any(|t| t.contains("Compaction skipped")),
        "the skipped compaction must be surfaced, not swallowed; saw: {:?}",
        notification_texts(&events)
    );

    // The store was NOT clobbered: every seeded message is still there.
    let stored = h.stored_texts().await;
    for i in 0..6 {
        assert!(
            stored.iter().any(|t| t == &format!("q{i}")),
            "q{i} must survive a declined compaction; stored: {stored:#?}"
        );
    }
    // ...and the turn ran on the full history, not the discarded summary.
    assert!(
        provider
            .texts_seen_on_main_call(0)
            .iter()
            .any(|t| t == "q0"),
        "the turn must continue from the fresh history, saw: {:?}",
        provider.texts_seen_on_main_call(0)
    );
}

/// A message persisted mid-turn that the reply loop never pushed into its
/// in-memory conversation — the shape of `drain_elicitation_messages`, of
/// `biorouter term log` writing from a shell hook in another process, and of
/// BR-71's note tool — must survive the overflow-recovery writeback.
///
/// This is the case where a freshness discipline built on "every id the loop
/// knows about" would report a FALSE conflict and quietly stop persisting
/// compactions forever. Carrying the message over instead is both correct and
/// self-correcting.
#[tokio::test(flavor = "multi_thread")]
async fn overflow_recovery_preserves_a_message_persisted_mid_turn() {
    let (h, provider) = harness(|p| {
        p.overflow_on(&[0])
            .note_during_main_call(0, "NOTE: persisted but never pushed")
    })
    .await;

    h.run_turn(USER_PROMPT).await.unwrap();

    assert_eq!(provider.main_call_count(), 2, "the turn must complete");
    let stored = h.stored_texts().await;
    assert!(
        stored
            .iter()
            .any(|t| t.contains("NOTE: persisted but never pushed")),
        "a message the loop persisted but never held in memory must survive; \
         stored: {stored:#?}"
    );
    assert!(
        stored.iter().any(|t| t.contains("User Intent")),
        "and the compaction must still be persisted; stored: {stored:#?}"
    );
}

/// Twice declined: retry exactly once (each attempt re-spends a billed
/// summarization call), then keep going in memory rather than clobbering. The
/// stored history must be untouched and no HistoryReplaced may be emitted —
/// claiming the store was replaced when it was not would make a reload look
/// like it resurrected messages.
#[tokio::test(flavor = "multi_thread")]
async fn overflow_recovery_retries_once_then_keeps_going_without_clobbering() {
    let (h, provider) = harness(|p| {
        p.overflow_on(&[0])
            .rewrite_during_summarization(0)
            .rewrite_during_summarization(1)
    })
    .await;

    h.session_manager
        .add_message(&h.session_id, &Message::user().with_text("earlier turn"))
        .await
        .unwrap();
    let before = h.stored_texts().await;

    let events = h.run_turn(USER_PROMPT).await.unwrap();

    assert_eq!(
        provider.summarizer_call_count(),
        2,
        "exactly one retry: every attempt re-spends a billed summarization call"
    );
    assert_eq!(
        provider.main_call_count(),
        2,
        "the turn must still complete"
    );

    let after = h.stored_texts().await;
    for text in &before {
        assert!(
            after.contains(text),
            "a twice-declined swap must leave the stored history intact; \
             {text:?} vanished. before: {before:#?} after: {after:#?}"
        );
    }
    assert!(
        !after.iter().any(|t| t.contains("User Intent")),
        "nothing was persisted, so no summary may appear on disk; after: {after:#?}"
    );
    assert!(
        history_replaced_texts(&events).is_none(),
        "HistoryReplaced must not claim a replacement that never happened"
    );
}

/// A retry that FAILS must not throw away the first summarization's spend.
///
/// The first swap is declined, so the ladder recomputes and retries — and the
/// retry's summarization errors. By then the provider has already charged for
/// the first round-trip, and there is a perfectly usable compaction in memory.
/// Bailing with `?` there dropped both: the spend never reached the budget or
/// the session gauge (contradicting `OverflowCompactionSwap::usages`' own
/// contract, "all of it is spend whether or not the result was kept"), and the
/// caller's `Err` arm ended the turn instead of continuing on the compaction it
/// already had.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_retry_still_bills_the_first_summarization_and_finishes_the_turn() {
    let (h, provider) = harness(|p| {
        p.overflow_on(&[0])
            // Declines the first swap: the basis moves under the caller.
            .rewrite_during_summarization(0)
            // ...and the retry's summarizer is gone.
            .fail_summarization_on(1)
    })
    .await;

    h.session_manager
        .add_message(&h.session_id, &Message::user().with_text("earlier turn"))
        .await
        .unwrap();
    let before = h.stored_texts().await;

    let events = h.run_turn(USER_PROMPT).await.unwrap();

    assert_eq!(
        provider.summarizer_call_count(),
        2,
        "the ladder must still attempt exactly one retry"
    );
    assert_eq!(
        provider.main_call_count(),
        2,
        "a failed RETRY must not end the turn: the first compaction is still \
         usable in memory"
    );

    // The billed spend: the first summarization (15) plus the main call that
    // finished the turn (15). The failed retry charged nothing and is absent.
    let session = h
        .session_manager
        .get_session(&h.session_id, false)
        .await
        .unwrap();
    assert_eq!(
        session.accumulated_total_tokens,
        Some(30),
        "the first summarization was charged by the provider and must still be \
         reported, even though its result was never persisted"
    );

    // Nothing was persisted, so the stored history must be intact and no
    // HistoryReplaced may claim otherwise.
    let after = h.stored_texts().await;
    for text in &before {
        assert!(
            after.contains(text),
            "a failed retry must leave the stored history intact; {text:?} vanished"
        );
    }
    assert!(
        history_replaced_texts(&events).is_none(),
        "HistoryReplaced must not claim a replacement that never happened"
    );
}

// ── /compact (execute_commands.rs) ───────────────────────────────────────────

async fn seed_history(h: &Harness, turns: usize) {
    for i in 0..turns {
        h.session_manager
            .add_message(&h.session_id, &Message::user().with_text(format!("q{i}")))
            .await
            .unwrap();
        h.session_manager
            .add_message(
                &h.session_id,
                &Message::assistant().with_text(format!("a{i}")),
            )
            .await
            .unwrap();
    }
}

/// `/compact` is the third snapshot -> summarize -> write-back site, and the
/// most user-visible one. A note appended while it summarizes must survive.
#[tokio::test(flavor = "multi_thread")]
async fn manual_compact_preserves_a_concurrent_note() {
    let (h, provider) =
        harness(|p| p.note_during_summarization(0, "NOTE: landed during /compact")).await;
    seed_history(&h, 6).await;

    h.run_turn("/compact").await.unwrap();

    assert_eq!(provider.summarizer_call_count(), 1);
    let stored = h.stored_texts().await;
    assert!(
        stored
            .iter()
            .any(|t| t.contains("NOTE: landed during /compact")),
        "the note must survive /compact; stored: {stored:#?}"
    );
    assert!(
        stored.iter().any(|t| t.contains("User Intent")),
        "and the compaction must land; stored: {stored:#?}"
    );
}

/// The anti-false-conflict twin: with no concurrent writer `/compact` must
/// still compact and still report success.
#[tokio::test(flavor = "multi_thread")]
async fn manual_compact_still_compacts_with_no_concurrent_writer() {
    let (h, provider) = harness(|p| p).await;
    seed_history(&h, 6).await;

    let events = h.run_turn("/compact").await.unwrap();

    assert_eq!(provider.summarizer_call_count(), 1);
    assert!(
        notification_texts(&events)
            .iter()
            .any(|t| t.contains("Compaction complete")),
        "saw: {:?}",
        notification_texts(&events)
    );
    assert!(h
        .stored_texts()
        .await
        .iter()
        .any(|t| t.contains("User Intent")));
}

/// When the basis moved, `/compact` tells the user rather than silently doing
/// nothing — it is user-initiated and trivially re-runnable.
#[tokio::test(flavor = "multi_thread")]
async fn manual_compact_reports_a_skipped_compaction_to_the_user() {
    let (h, provider) = harness(|p| p.rewrite_during_summarization(0)).await;
    seed_history(&h, 6).await;
    let before = h.stored_texts().await;

    let events = h.run_turn("/compact").await.unwrap();

    assert_eq!(provider.summarizer_call_count(), 1);
    assert!(
        notification_texts(&events)
            .iter()
            .any(|t| t.contains("Run /compact again")),
        "the skipped compaction must be reported; saw: {:?}",
        notification_texts(&events)
    );
    let after = h.stored_texts().await;
    for text in &before {
        assert!(
            after.contains(text),
            "a skipped /compact must leave the history intact; {text:?} vanished"
        );
    }
    assert!(
        !after.iter().any(|t| t.contains("User Intent")),
        "nothing may be persisted when the swap was declined"
    );
}

// ── #51 part (b): the preservation marker, through the real reply loop ───────

/// The pinned-message texts stored for the session, with their agent visibility.
async fn stored_pins(h: &Harness) -> Vec<(String, bool)> {
    h.session_manager
        .get_session(&h.session_id, true)
        .await
        .unwrap()
        .conversation
        .unwrap()
        .messages()
        .iter()
        .filter(|m| m.is_pinned())
        .map(|m| {
            (
                m.content
                    .iter()
                    .filter_map(|c| match c {
                        MessageContent::Text(t) => Some(t.text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
                m.is_agent_visible(),
            )
        })
        .collect()
}

/// Is `text` still in the agent's context, per the store?
async fn agent_sees(h: &Harness, text: &str) -> bool {
    h.session_manager
        .get_session(&h.session_id, true)
        .await
        .unwrap()
        .conversation
        .unwrap()
        .agent_visible_messages()
        .iter()
        .flat_map(|m| m.content.iter())
        .any(|c| matches!(c, MessageContent::Text(t) if t.text.contains(text)))
}

/// **The whole promise, in one test.** A message appended concurrently AND
/// marked must survive both halves of #51: the write-back that would have
/// deleted it (part a), and the compaction that would have summarized it away
/// several turns later (part b).
///
/// Fixing only part (a) produces exactly the failure this test would catch — a
/// note that lands, is confirmed, and quietly evaporates once it falls out of
/// the four-turn verbatim window.
#[tokio::test(flavor = "multi_thread")]
async fn a_pinned_note_survives_both_the_writeback_and_a_later_compaction() {
    const NOTE: &str = "NOTE: always report the log-scale version";

    let (h, provider) = harness(|p| {
        p.overflow_on(&[0])
            .pinned_note_during_summarization(0, NOTE)
    })
    .await;

    // Turn 1: the overflow-recovery summarizer runs and the note lands inside
    // its round-trip. Part (a) has to carry it over the write-back.
    h.run_turn(USER_PROMPT).await.unwrap();
    assert_eq!(provider.summarizer_call_count(), 1);
    assert_eq!(
        stored_pins(&h).await,
        vec![(NOTE.to_string(), true)],
        "part (a): the pinned note must survive the write-back, still marked"
    );

    // Age it well out of the verbatim window: six more user-prompt turns plus
    // the next turn's own prompt, against a default keep-last of four.
    for i in 0..6 {
        h.session_manager
            .add_message(&h.session_id, &Message::user().with_text(format!("q{i}")))
            .await
            .unwrap();
        h.session_manager
            .add_message(
                &h.session_id,
                &Message::assistant().with_text(format!("a{i}")),
            )
            .await
            .unwrap();
    }
    // Push reported usage over the auto-compaction threshold so turn 2 compacts.
    h.session_manager
        .update(&h.session_id)
        .total_tokens(Some(190_000))
        .apply()
        .await
        .unwrap();

    h.run_turn("and now the second question").await.unwrap();

    assert_eq!(
        provider.summarizer_call_count(),
        2,
        "turn 2 must actually have compacted, or this proves nothing"
    );
    assert_eq!(
        stored_pins(&h).await,
        vec![(NOTE.to_string(), true)],
        "part (b): the note must still be in the agent's context after the \
         compaction that summarized everything around it"
    );
    // The control: an unmarked message from the same summarized prefix is gone
    // from the agent's context, so the compaction really did run over it.
    assert!(
        !agent_sees(&h, "q0").await,
        "an unmarked older message must have been summarized away"
    );
}

/// The in-turn auto-compaction site (`agent.rs`), reached when a turn starts
/// over the threshold.
#[tokio::test(flavor = "multi_thread")]
async fn auto_compaction_keeps_a_pinned_message() {
    const NOTE: &str = "NOTE: the cohort is 2019, not 2024";

    let (h, provider) = harness(|p| p.context_limit(100)).await;

    h.session_manager
        .add_message(&h.session_id, &Message::user().with_text(NOTE).pinned())
        .await
        .unwrap();
    for i in 0..6 {
        h.session_manager
            .add_message(&h.session_id, &Message::user().with_text(format!("q{i}")))
            .await
            .unwrap();
        h.session_manager
            .add_message(
                &h.session_id,
                &Message::assistant().with_text(format!("a{i}")),
            )
            .await
            .unwrap();
    }
    h.session_manager
        .update(&h.session_id)
        .total_tokens(Some(95))
        .apply()
        .await
        .unwrap();

    h.run_turn(USER_PROMPT).await.unwrap();

    assert_eq!(provider.summarizer_call_count(), 1);
    assert_eq!(stored_pins(&h).await, vec![(NOTE.to_string(), true)]);
    assert!(
        !agent_sees(&h, "q0").await,
        "control: q0 was summarized away"
    );
    // ...and it reached the model on the very turn that compacted.
    assert!(
        provider
            .texts_seen_on_main_call(0)
            .iter()
            .any(|t| t.contains(NOTE)),
        "the pinned note must be in this turn's request, saw: {:?}",
        provider.texts_seen_on_main_call(0)
    );
}

/// The `/compact` and `/summarize` site (`execute_commands.rs`). A user who
/// explicitly compacts must not thereby destroy the note they pinned.
#[tokio::test(flavor = "multi_thread")]
async fn manual_compact_keeps_a_pinned_message() {
    const NOTE: &str = "NOTE: keep the units in mg/dL";

    let (h, provider) = harness(|p| p).await;
    h.session_manager
        .add_message(&h.session_id, &Message::user().with_text(NOTE).pinned())
        .await
        .unwrap();
    seed_history(&h, 6).await;

    h.run_turn("/compact").await.unwrap();

    assert_eq!(provider.summarizer_call_count(), 1);
    assert_eq!(stored_pins(&h).await, vec![(NOTE.to_string(), true)]);
    assert!(
        h.stored_texts()
            .await
            .iter()
            .any(|t| t.contains("User Intent")),
        "the compaction itself must still have landed"
    );
    assert!(
        !agent_sees(&h, "q0").await,
        "control: q0 was summarized away"
    );
}

/// The marker must not be sent to the summarizer as well as kept. Its text
/// survives verbatim, so summarizing it too spends the window on the same
/// content twice — and on a long-lived session that cost recurs every
/// compaction.
#[tokio::test(flavor = "multi_thread")]
async fn a_pinned_message_is_not_also_summarized() {
    const NOTE: &str = "NOTE: unique-marker-string-for-the-summarizer-payload";

    let (h, provider) = harness(|p| p).await;
    h.session_manager
        .add_message(&h.session_id, &Message::user().with_text(NOTE).pinned())
        .await
        .unwrap();
    seed_history(&h, 6).await;

    h.run_turn("/compact").await.unwrap();

    assert_eq!(provider.summarizer_call_count(), 1);
    let payload = provider.all_summarizer_payloads();
    assert!(
        !payload.contains(NOTE),
        "the pinned note must not be summarized as well as kept; payload: {payload}"
    );
    assert!(
        payload.contains("q0"),
        "control: the unpinned history WAS handed to the summarizer"
    );
    // And exactly one copy is stored — no duplicate re-appended alongside it.
    let occurrences = h
        .stored_texts()
        .await
        .iter()
        .filter(|t| t.contains(NOTE))
        .count();
    assert_eq!(occurrences, 1, "the pinned note must appear exactly once");
}

// ── W1: `basis` and `known` must come from ONE snapshot ──────────────────────
//
// The store's guard splits the history at `basis.max_rowid`: rows ABOVE the
// watermark that `known` does not name are another writer's and are carried
// over; everything at or below it is deleted and replaced. That is only sound
// when `known` was read WITH `basis` (revision first, then the conversation —
// `SessionManager::snapshot_for_rewrite`). Source the two independently and an
// append that lands in between is counted by `basis.max_rowid` — so
// `scan_foreign_tail` never looks at it — while `known` does not contain it
// either. The DELETE destroys it, after `add_message` already returned success.
//
// Both tests below hit that window with no sleeps and no barriers, by choosing
// where in the event stream the concurrent append happens.

/// The turn-start window: `reply()` takes its snapshot inside the async fn, and
/// the reply loop reads its overflow-recovery basis in the stream body, which
/// does not run until the stream is polled. An append between the two is
/// invisible to the loop's view and already inside its basis.
///
/// **The handshake.** `reply()` is an `async fn` returning a stream, and the
/// stream body does not run until it is polled. So awaiting `reply()` parks the
/// writer — the whole turn — at exactly that boundary, the test's `add_message`
/// runs to COMMIT while it is parked, and only then is the turn allowed to
/// proceed. No sleep, no barrier, no scheduler overlap to hope for: the ordering
/// is a data dependency, not a race that usually goes the right way.
///
/// **The vacuity guard** is `texts_seen_on_main_call(0)`. The window's whole
/// premise is that the note landed AFTER the turn's view of the history was
/// fixed. If a future refactor moved the basis read later, the note would simply
/// be part of `known`, the store would preserve it for free, and this test would
/// pass while testing nothing. Asserting the model never saw it pins the
/// precondition instead of hoping for it.
#[tokio::test(flavor = "multi_thread")]
async fn overflow_recovery_preserves_a_note_appended_before_the_basis_was_read() {
    const NOTE: &str = "NOTE: landed between the snapshot and the basis read";

    let (h, provider) = harness(|p| p.overflow_on(&[0])).await;

    let stream = h
        .agent
        .reply(
            Message::user().with_text(USER_PROMPT),
            turn_config(&h),
            None,
        )
        .await
        .unwrap();

    // The window. `reply()` has snapshotted the conversation; nothing of the
    // loop has run yet, and nothing of it CAN run until this await returns and
    // the stream below is polled.
    h.session_manager
        .add_message(&h.session_id, &Message::user().with_text(NOTE))
        .await
        .unwrap();

    tokio::pin!(stream);
    while let Some(event) = stream.next().await {
        event.unwrap();
    }

    assert_eq!(
        provider.summarizer_call_count(),
        1,
        "the overflow recovery must actually have run, or this proves nothing"
    );
    assert_eq!(
        provider.main_call_count(),
        2,
        "and the turn must still complete"
    );
    assert!(
        !provider
            .texts_seen_on_main_call(0)
            .iter()
            .any(|t| t.contains(NOTE)),
        "the append must land OUTSIDE the turn's view, or the window under test \
         was never entered and the store preserves the note for free; saw: {:?}",
        provider.texts_seen_on_main_call(0)
    );

    let stored = h.stored_texts().await;
    assert!(
        stored.iter().any(|t| t.contains(NOTE)),
        "a message appended before the turn read its freshness basis must \
         survive the overflow-recovery writeback; stored: {stored:#?}"
    );
    assert_eq!(
        h.stored_occurrences(NOTE).await,
        1,
        "and exactly once: a split basis can duplicate a row as easily as it \
         can delete one; stored: {stored:#?}"
    );
}

/// The post-swap window: a landed swap renumbers every row, so the basis has to
/// be re-seeded — and re-seeding the REVISION alone, after the conversation the
/// turn adopts was already decided, reopens the same split one iteration later.
/// The turn reports its new token state between the two, which is where this
/// test appends.
///
/// **The handshake.** The stream is polled by this task and by nothing else, so
/// the turn is parked at the yield for as long as the append takes; it resumes
/// only when the append has COMMITTED and the loop below asks for the next
/// event.
///
/// **Why this yield and no other.** The window is bounded below by the swap's
/// commit and above by the basis refresh. In the shape this test exists to pin,
/// the refresh sat between the `TokenUsage` yield and the `HistoryReplaced`
/// yield — so `TokenUsage` is the only stream position inside it, and the test
/// asserts it appended before ever seeing a `HistoryReplaced`. Move the refresh
/// and that assertion fires rather than the test going quietly vacuous.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_overflow_preserves_a_note_appended_right_after_the_first_swap() {
    const NOTE: &str = "NOTE: landed between the swap and the basis refresh";

    let (h, provider) = harness(|p| p.overflow_on(&[0, 1])).await;

    let stream = h
        .agent
        .reply(
            Message::user().with_text(USER_PROMPT),
            turn_config(&h),
            None,
        )
        .await
        .unwrap();
    tokio::pin!(stream);

    let mut appended = false;
    let mut history_replaced_before_append = false;
    while let Some(event) = stream.next().await {
        let event = event.unwrap();
        if !appended && matches!(event, AgentEvent::HistoryReplaced(_)) {
            history_replaced_before_append = true;
        }
        // The first recovery swap has committed by the time the turn reports the
        // token state it moved; the basis is refreshed after this yield.
        if !appended && matches!(event, AgentEvent::TokenUsage(_)) {
            h.session_manager
                .add_message(&h.session_id, &Message::user().with_text(NOTE))
                .await
                .unwrap();
            appended = true;
        }
    }

    assert!(
        appended,
        "the turn must have reported a token state after the first swap"
    );
    assert!(
        !history_replaced_before_append,
        "the append must land BEFORE the loop adopts the swapped history, or it \
         is past the window this test exists to cover"
    );
    assert_eq!(
        provider.summarizer_call_count(),
        2,
        "two overflows must each run their own recovery compaction"
    );
    assert_eq!(
        provider.main_call_count(),
        3,
        "two overflows, then a success"
    );
    assert!(
        !provider
            .texts_seen_on_main_call(1)
            .iter()
            .any(|t| t.contains(NOTE)),
        "the append must land outside the view the SECOND swap writes against, \
         or the window under test was never entered; saw: {:?}",
        provider.texts_seen_on_main_call(1)
    );

    let stored = h.stored_texts().await;
    assert!(
        stored.iter().any(|t| t.contains(NOTE)),
        "a message appended after the first swap committed must survive the \
         second one; stored: {stored:#?}"
    );
    assert_eq!(
        h.stored_occurrences(NOTE).await,
        1,
        "and exactly once; stored: {stored:#?}"
    );
}

/// The same window on the OTHER handoff into the reply loop: the one an
/// auto-compaction goes through.
///
/// `reply()` compacts before the loop starts, and the conversation it hands down
/// is fixed at that rewrite's commit. Everything the loop then does — reading
/// its own freshness basis included — happens later. So an append that lands
/// after the auto-compaction committed but before the loop has a basis is inside
/// the turn's watermark and outside its view: the exact split, reached without
/// touching `reply()`'s own snapshot at all.
///
/// This matters separately from the turn-start test above because the two enter
/// the loop by different routes. A fix that paired the basis only on the
/// no-compaction route would leave this one open, and the largest sessions —
/// the ones that auto-compact every turn — take precisely this route.
#[tokio::test(flavor = "multi_thread")]
async fn overflow_recovery_preserves_a_note_appended_between_the_auto_compaction_and_the_loop() {
    const NOTE: &str = "NOTE: landed between the auto-compaction and the loop";

    // Small context limit + a live gauge over it => `reply()` auto-compacts
    // before the loop; `overflow_on(&[0])` then forces a second, in-loop swap.
    let (h, provider) = harness(|p| p.overflow_on(&[0]).context_limit(100)).await;
    seed_history(&h, 6).await;
    h.session_manager
        .update(&h.session_id)
        .total_tokens(Some(95))
        .apply()
        .await
        .unwrap();

    let stream = h
        .agent
        .reply(
            Message::user().with_text(USER_PROMPT),
            turn_config(&h),
            None,
        )
        .await
        .unwrap();
    tokio::pin!(stream);

    let mut appended = false;
    while let Some(event) = stream.next().await {
        let event = event.unwrap();
        // The auto-compaction announces its replacement only after the rewrite
        // committed. The turn is parked here until the append lands.
        if !appended && matches!(event, AgentEvent::HistoryReplaced(_)) {
            h.session_manager
                .add_message(&h.session_id, &Message::user().with_text(NOTE))
                .await
                .unwrap();
            appended = true;
        }
    }

    assert!(
        appended,
        "the auto-compaction must have replaced the history, or there was no \
         window to append into"
    );
    assert!(
        provider.summarizer_call_count() >= 2,
        "an auto-compaction AND an overflow recovery must both have run, saw {}",
        provider.summarizer_call_count()
    );
    assert_eq!(
        provider.main_call_count(),
        2,
        "one overflow, then a success"
    );
    assert!(
        !provider
            .texts_seen_on_main_call(0)
            .iter()
            .any(|t| t.contains(NOTE)),
        "the append must land outside the view the loop inherited, or the window \
         under test was never entered; saw: {:?}",
        provider.texts_seen_on_main_call(0)
    );

    let stored = h.stored_texts().await;
    assert!(
        stored.iter().any(|t| t.contains(NOTE)),
        "a message appended after the auto-compaction committed, but before the \
         loop read its basis, must survive the loop's own writeback; \
         stored: {stored:#?}"
    );
    assert_eq!(
        h.stored_occurrences(NOTE).await,
        1,
        "and exactly once; stored: {stored:#?}"
    );
}

// ── W2: the FIRST persistence attempt is billed spend too ────────────────────

/// Reject exactly the compaction summary's own INSERT, in the store itself.
/// Every other write on the turn still succeeds, so this is a real database
/// error on the swap — not a declined swap, and not a broken session.
async fn fail_to_store_the_summary(h: &Harness) {
    let db = h.data_dir.join(SESSIONS_FOLDER).join(DB_NAME);
    let pool =
        sqlx::SqlitePool::connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&db))
            .await
            .unwrap();
    sqlx::query(
        "CREATE TRIGGER reject_the_summary BEFORE INSERT ON messages \
         WHEN NEW.content_json LIKE '%User Intent%' \
         BEGIN SELECT RAISE(ABORT, 'injected store failure'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
}

/// A database error on the FIRST persistence attempt must not throw away the
/// summarization the provider has already charged for, and must not end the
/// turn.
///
/// This is the same defect class as `a_failed_retry_still_bills_...` one rung
/// earlier on the ladder: the caller bills only the `Ok` branch and `break`s on
/// `Err`, so an `await?` here loses the spend from both the budget and the
/// session gauge, leaves the `PreCompact` it fired without a `PostCompact`, and
/// ends a turn that had a perfectly usable compaction in memory — at the last
/// rung before "context limit still exceeded".
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_first_persist_still_bills_the_summarization_and_finishes_the_turn() {
    let (h, provider) = harness(|p| p.overflow_on(&[0])).await;

    h.session_manager
        .add_message(&h.session_id, &Message::user().with_text("earlier turn"))
        .await
        .unwrap();
    let before = h.stored_texts().await;
    fail_to_store_the_summary(&h).await;

    let events = h.run_turn(USER_PROMPT).await.unwrap();

    assert_eq!(
        provider.summarizer_call_count(),
        1,
        "a store error is not a moved basis: there is nothing to recompute \
         against, so it must not re-spend a summarization"
    );
    assert_eq!(
        provider.main_call_count(),
        2,
        "a failed persist must not end the turn: the compaction is still \
         usable in memory"
    );

    // The billed spend: the summarization (15) plus the main call that finished
    // the turn (15). The overflowing call charged nothing.
    let session = h
        .session_manager
        .get_session(&h.session_id, false)
        .await
        .unwrap();
    assert_eq!(
        session.accumulated_total_tokens,
        Some(30),
        "the summarization was charged by the provider and must still be \
         reported even though the store refused it"
    );

    // Nothing landed, so the stored history must be intact and no
    // HistoryReplaced may claim otherwise.
    let after = h.stored_texts().await;
    for text in &before {
        assert!(
            after.contains(text),
            "a failed persist must leave the stored history intact; \
             {text:?} vanished. before: {before:#?} after: {after:#?}"
        );
    }
    assert!(
        !after.iter().any(|t| t.contains("User Intent")),
        "nothing was persisted, so no summary may appear on disk; after: {after:#?}"
    );
    assert!(
        history_replaced_texts(&events).is_none(),
        "HistoryReplaced must not claim a replacement that never happened"
    );
}

/// The `SessionConfig` `Harness::run_turn` uses, for the tests that have to
/// drive the stream themselves to land an append in a specific window.
fn turn_config(h: &Harness) -> SessionConfig {
    SessionConfig {
        id: h.session_id.clone(),
        schedule_id: None,
        max_turns: Some(8),
        max_tool_calls: None,
        retry_config: None,
        budget: None,
        reasoning_effort: None,
    }
}

// ── #51 W6/W7/W8: the pin rule, through the loop that actually persists ──────
//
// The three unit-level fixes for these all live under `crates/biorouter/src`,
// where the input is a `Vec<Message>` somebody wrote by hand. The tests below
// exercise them from the outside instead: a real session, the real reply loop,
// and — crucially — the OVERFLOW-recovery site, which is the one compaction
// whose input is the NORMALIZED transcript and whose output is written to disk.
// A pin that the normalizer destroys is destroyed durably there, and no unit
// test of `merge_consecutive_messages` can say whether anything reaches it.

/// W6: a pin next to another message of the same role.
///
/// `merge_consecutive_messages` extends the FIRST message and keeps only its
/// metadata and its durable id, so an unpinned neighbour that absorbs a pinned
/// note deletes the marker — and the note is then summarized away like anything
/// else. Every pre-existing pin test in this file goes through a compaction site
/// that sees the RAW stored history (`reply()`'s auto-compaction, `/compact`),
/// so none of them could reach the merge pass at all.
#[tokio::test(flavor = "multi_thread")]
async fn a_pinned_note_next_to_an_unpinned_one_survives_the_normalized_writeback() {
    const NOTE: &str = "NOTE: units are mg/dL, never mmol/L";
    const NEIGHBOUR: &str = "for context, the cohort is the 2019 one";

    let (h, provider) = harness(|p| p.overflow_on(&[0])).await;

    // Adjacent, same role, pin SECOND: the merge makes the unpinned one the
    // carrier, so the marker is what gets dropped.
    h.session_manager
        .add_message(&h.session_id, &Message::user().with_text(NEIGHBOUR))
        .await
        .unwrap();
    h.session_manager
        .add_message(&h.session_id, &Message::user().with_text(NOTE).pinned())
        .await
        .unwrap();
    seed_history(&h, 6).await;

    h.run_turn(USER_PROMPT).await.unwrap();

    assert_eq!(
        provider.summarizer_call_count(),
        1,
        "the overflow recovery must have compacted, or this proves nothing"
    );
    assert_eq!(
        stored_pins(&h).await,
        vec![(NOTE.to_string(), true)],
        "the pinned note must survive a compaction whose input went through the \
         merge pass, still marked and still in the agent's context"
    );
    assert!(
        !agent_sees(&h, "q0").await,
        "control: the unmarked history WAS summarized away"
    );
    // ...and the pin did not broaden the other way either: the neighbour is not
    // exempt just because it sat next to a marked message.
    assert!(
        !agent_sees(&h, NEIGHBOUR).await,
        "an unpinned neighbour must not inherit the exemption; it is ordinary \
         history and belongs in the summary"
    );
}

/// W7: flipping the marker on a message the incremental normalizer has already
/// frozen into its cached prefix.
///
/// `ConversationNormalizer` reuses a frozen prefix whenever a per-message
/// fingerprint still matches, and serves that prefix's OUTPUT messages rather
/// than re-deriving them. A fingerprint blind to `pinned` therefore hands back
/// the marker's OLD value for as long as the session lives — and the overflow
/// path writes that transcript to the store, so a pin set on an older message is
/// not merely ignored, it is erased.
///
/// The normalizer lives on the `Agent`, so both turns below have to run on ONE
/// harness for the cache to be warm at all.
#[tokio::test(flavor = "multi_thread")]
async fn a_pin_set_on_a_message_the_normalizer_already_froze_is_still_honoured() {
    // `MIN_MESSAGES_TO_CACHE` is 16 and `TAIL_SLACK` is 8, so 20 seeded
    // messages put "q2" comfortably inside the frozen prefix.
    const MARKED: &str = "q2";

    let (h, provider) = harness(|p| p.overflow_on(&[1])).await;
    seed_history(&h, 10).await;

    // Turn 1: no compaction, so nothing invalidates the cache — it just warms.
    h.run_turn("first question").await.unwrap();
    assert_eq!(
        provider.summarizer_call_count(),
        0,
        "turn 1 must not compact, or the cache is reset and the seam is never \
         exercised"
    );

    // The marker goes on afterwards — the shape of BR-71's note tool marking an
    // earlier message, or a user pinning something from the transcript.
    let current = h
        .session_manager
        .get_session(&h.session_id, true)
        .await
        .unwrap()
        .conversation
        .unwrap();
    let marked: Vec<Message> = current
        .messages()
        .iter()
        .cloned()
        .map(|m| {
            let is_target = m
                .content
                .iter()
                .any(|c| matches!(c, MessageContent::Text(t) if t.text == MARKED));
            if is_target {
                m.pinned()
            } else {
                m
            }
        })
        .collect();
    assert_eq!(
        marked.iter().filter(|m| m.is_pinned()).count(),
        1,
        "exactly one message must have been marked"
    );
    h.session_manager
        .replace_conversation(
            &h.session_id,
            &biorouter::conversation::Conversation::new_unvalidated(marked),
        )
        .await
        .unwrap();

    // Turn 2 overflows, so the compaction runs over the NORMALIZED transcript
    // and persists it.
    h.run_turn("second question").await.unwrap();

    assert_eq!(
        provider.summarizer_call_count(),
        1,
        "turn 2 must actually have compacted, or this proves nothing"
    );
    assert_eq!(
        stored_pins(&h).await,
        vec![(MARKED.to_string(), true)],
        "a marker set after the normalizer froze that message must still be \
         honoured; a stale cached prefix erases it durably"
    );
    assert!(
        !agent_sees(&h, "q0").await,
        "control: an unmarked message from the same frozen prefix WAS summarized"
    );
}

/// W8: a marker on content that can never be honoured must not be honoured.
///
/// `FrontendToolRequest` is a real provider `tool_use` block — every formatter
/// emits one — and the normalizer strips it only from ASSISTANT messages, so a
/// user-role instance arriving through the API survives to the selector.
/// Preserving one past a compaction that summarized its response away leaves a
/// dangling tool call in the durable history, which the next request carries to
/// the provider.
#[tokio::test(flavor = "multi_thread")]
async fn a_pinned_frontend_tool_request_is_not_preserved_by_a_real_compaction() {
    const FTR_ID: &str = "ftr-must-not-be-preserved";
    const TEXT_NOTE: &str = "NOTE: this one is preservable and must survive";

    // `/compact` sees the RAW stored history, which is what makes a user-role
    // frontend tool request reachable at all. It is used here in preference to
    // the auto-compaction site for a second reason: forcing that one needs a
    // tiny `context_limit`, and `PinLimits::max_tokens` is a SHARE of the same
    // number — at 100 the text note below alone exhausts the pinned-set budget
    // and the frontend tool request is evicted for that reason instead, so the
    // test passes whatever the eligibility rule says. (Observed, on the way to
    // writing this: "the marked set exceeded the preserved-set token budget".)
    let (h, provider) = harness(|p| p).await;

    h.session_manager
        .add_message(
            &h.session_id,
            &Message::user()
                .with_content(MessageContent::frontend_tool_request(
                    FTR_ID,
                    Ok(rmcp::model::CallToolRequestParams {
                        task: None,
                        name: "read_file".into(),
                        arguments: None,
                        meta: None,
                    }),
                ))
                .pinned(),
        )
        .await
        .unwrap();
    // The control, marked the same way: a text note IS preservable, so the
    // compaction below is demonstrably honouring markers at all.
    h.session_manager
        .add_message(
            &h.session_id,
            &Message::user().with_text(TEXT_NOTE).pinned(),
        )
        .await
        .unwrap();
    seed_history(&h, 6).await;

    h.run_turn("/compact").await.unwrap();

    assert_eq!(provider.summarizer_call_count(), 1);
    assert!(
        agent_sees(&h, TEXT_NOTE).await,
        "control: a marked TEXT note must be preserved, or this compaction is \
         not honouring markers and the assertion below means nothing"
    );
    // ...and nothing was evicted, so whatever happens to the frontend tool
    // request below is the ELIGIBILITY rule talking and not the budget.
    assert!(
        !h.stored_texts()
            .await
            .iter()
            .any(|t| t.contains("could not all be kept")),
        "the pinned-set budget must not have evicted anything, or this test \
         cannot tell an ineligible pin from a crowded-out one; stored: {:#?}",
        h.stored_texts().await
    );
    assert!(
        !agent_visible_frontend_tool_request(&h, FTR_ID).await,
        "a marked frontend tool request must NOT be exempt from summarization: \
         it is half a provider tool pair, and preserving it past the compaction \
         that hid its response leaves a dangling tool call on disk"
    );
    assert!(
        !agent_sees(&h, "q0").await,
        "control: the unmarked history WAS summarized away"
    );
}

/// Is a `FrontendToolRequest` with this id still in the agent's context?
async fn agent_visible_frontend_tool_request(h: &Harness, id: &str) -> bool {
    h.session_manager
        .get_session(&h.session_id, true)
        .await
        .unwrap()
        .conversation
        .unwrap()
        .agent_visible_messages()
        .iter()
        .flat_map(|m| m.content.iter())
        .any(|c| matches!(c, MessageContent::FrontendToolRequest(r) if r.id == id))
}

// ── NF-D follow-on: can a live client actually satisfy `expectedMessageIds`? ──

/// Every message id a client could possibly know after streaming a whole turn:
/// the `id` on each streamed `Message`, the ids in a `HistoryReplaced` (which
/// the desktop applies wholesale as `UpdateConversation`), and the ids in a
/// `MessagesPersisted` (#59 — the frame the loop publishes after each persist,
/// forwarded on the wire as `MessagesPersisted`).
fn ids_a_client_can_learn(events: &[AgentEvent]) -> std::collections::HashSet<String> {
    let mut ids = std::collections::HashSet::new();
    for ev in events {
        match ev {
            AgentEvent::Message(m) => {
                if let Some(id) = &m.id {
                    ids.insert(id.clone());
                }
            }
            AgentEvent::HistoryReplaced(conversation) => {
                ids.extend(conversation.iter().filter_map(|m| m.id.clone()));
            }
            AgentEvent::MessagesPersisted(persisted) => {
                ids.extend(persisted.iter().map(|p| p.id.clone()));
            }
            _ => {}
        }
    }
    ids
}

async fn stored_ids(h: &Harness) -> Vec<(String, String)> {
    h.session_manager
        .get_session(&h.session_id, true)
        .await
        .unwrap()
        .conversation
        .unwrap()
        .messages()
        .iter()
        .map(|m| {
            (
                m.id.clone().unwrap_or_else(|| "<none>".to_string()),
                m.as_concat_text().chars().take(48).collect::<String>(),
            )
        })
        .collect()
}

/// `POST /sessions/{id}/edit_message` with `edit_type: "edit"` accepts
/// `expectedMessageIds` — the ids of every message the client's view holds — and
/// answers 409 if the store holds one the view does not name. That check is only
/// satisfiable if a client that has watched a whole turn go by actually ends up
/// knowing every id the store persisted.
///
/// It did not (#59). `Message::user()` / `Message::assistant()` mint no id, the
/// reply loop yields those messages *before* persisting them, and `add_message`
/// then mints a UUIDv7 that was never published back to the stream. On the
/// minimal turn below the client could learn **zero** of the two stored ids.
/// Three independent sources of the same gap:
///
/// 1. the ordinary assistant reply — `response_to_message` builds a bare
///    `Message::assistant()`, so the client saw `id: null` while the store held
///    a uid it was never told;
/// 2. the tool-call split (`agent.rs`, `next_assistant_id`) — one streamed
///    response becomes up to three stored rows, and every row after the first
///    gets a freshly minted uuid;
/// 3. the BR-47 post-edit diagnostics and the loop-guard nudges, which are
///    `with_visibility(false, true)` and so are persisted without ever being
///    yielded at all.
///
/// Consequence, and THE REASON THE FIELD IS OPTIONAL: while the endpoint
/// required it, the desktop's "Edit in Place" button 409'd on a session nobody
/// else had touched, from the first assistant reply onward, until the session
/// was reloaded — loud and safe, and dead in a live chat. So `expectedMessageIds`
/// is enforced when sent and absent-tolerated when not (the cut still runs
/// under the turn lock and still bounded to the rows the handler read), and the
/// desktop sends it only for a view in which every message names itself. This
/// test is what says when that condition can become unconditional again.
///
/// The fix is on the server side of the stream: (1) is closed by naming a reply
/// BEFORE it is yielded, the inverse of the rule `add_message_adopting_uid`
/// already encodes for the six sites that persist before yielding; (2) and (3)
/// cannot be expressed as a yielded copy at all — there is no yielded copy of a
/// row the client is deliberately not shown, and one streamed message cannot
/// carry three ids — so every persist site publishes what it stored as
/// `AgentEvent::MessagesPersisted`. Re-reading the session from the client
/// instead would be theatre for the same reason spelled out for this endpoint in
/// `docs/agent-loop/conversation-writeback-freshness.md` — the re-read happens
/// *after* the concurrent append has committed, so it would name the very row
/// the guard exists to protect and pass every time.
#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_watched_the_turn_knows_every_stored_message_id() {
    let (h, _provider) = harness(|p| p).await;

    let events = h.run_turn(USER_PROMPT).await.unwrap();
    let known = ids_a_client_can_learn(&events);
    let stored = stored_ids(&h).await;

    let unknowable: Vec<&(String, String)> = stored
        .iter()
        .filter(|(id, _)| !known.contains(id))
        .collect();

    assert!(
        unknowable.is_empty(),
        "the store holds {} message(s) whose id never reached the client, so \
         `expectedMessageIds` can never name them and every in-place edit 409s.\n\
         unknowable: {:#?}\nstreamed ids: {:#?}\nstored: {:#?}",
        unknowable.len(),
        unknowable,
        known,
        stored
    );

    // The assistant reply is not merely *named somewhere* — the copy the client
    // was handed carries the id of the row it became. That is the half of #59
    // `MessagesPersisted` cannot express: a client merges streamed deltas by id,
    // so a reply that arrives as `id: null` is unmergeable as well as unnameable.
    let streamed_assistant_ids: Vec<Option<String>> = events
        .iter()
        .filter_map(|ev| match ev {
            AgentEvent::Message(m) if m.role == rmcp::model::Role::Assistant => Some(m.id.clone()),
            _ => None,
        })
        .collect();
    assert!(
        !streamed_assistant_ids.is_empty()
            && streamed_assistant_ids.iter().all(Option::is_some),
        "every assistant message the client is shown must name itself; got {streamed_assistant_ids:#?}"
    );
}

/// #59, the shape the minimal turn cannot reach: **one** streamed assistant
/// message becomes **three** stored assistant rows.
///
/// A reply carrying thinking plus two tool calls is yielded once, and the loop
/// then rebuilds it as thinking / request / request rows — only the first of
/// which keeps the reply's id (`next_assistant_id` mints a fresh uuid for each
/// later one, because two rows may not share a `msg_uid`). No id stamped on the
/// streamed copy can cover that, which is exactly why the persist site publishes
/// what it stored rather than the stream trying to pre-announce it.
#[tokio::test(flavor = "multi_thread")]
async fn a_reply_split_into_several_stored_rows_publishes_every_one_of_their_ids() {
    let (h, provider) = harness(|p| p.tools_on(0)).await;

    let events = h.run_turn(USER_PROMPT).await.unwrap();
    assert_eq!(
        provider.main_call_count(),
        2,
        "the tool call must have run and the loop come back for a second reply"
    );

    let stored = stored_ids(&h).await;
    let streamed_messages = events
        .iter()
        .filter(|ev| matches!(ev, AgentEvent::Message(_)))
        .count();
    assert!(
        stored.len() > streamed_messages,
        "this test is only meaningful when the store holds MORE rows than the \
         client was streamed messages; stored {} vs streamed {}",
        stored.len(),
        streamed_messages
    );

    let known = ids_a_client_can_learn(&events);
    let unknowable: Vec<&(String, String)> = stored
        .iter()
        .filter(|(id, _)| !known.contains(id))
        .collect();
    assert!(
        unknowable.is_empty(),
        "a reply split across several stored rows must publish every row's id.\n\
         unknowable: {unknowable:#?}\nstreamed ids: {known:#?}\nstored: {stored:#?}"
    );
}

/// #59: a row the client is deliberately **not shown** must still be
/// distinguishable from one it simply was not told about.
///
/// BR-66 absorbs a recoverable provider error by storing its hint as a
/// `with_visibility(false, true)` user message — model-visible plumbing that is
/// never yielded as a `Message`. Before this it was invisible to the client in
/// the strong sense: not drawn *and* not nameable, so `expectedMessageIds` could
/// not include it and the guard refused the edit. It is now published with
/// `user_visible: false` — named, and marked "draw nothing for this".
#[tokio::test(flavor = "multi_thread")]
async fn a_row_the_user_is_never_shown_is_published_as_not_user_visible() {
    let (h, provider) = harness(|p| p.server_error_on(0)).await;

    let events = h.run_turn(USER_PROMPT).await.unwrap();
    assert_eq!(
        provider.main_call_count(),
        2,
        "the recoverable error must have been absorbed and the turn retried"
    );

    // The hidden row exists on disk and was never streamed as a Message.
    let hidden: Vec<String> = h
        .session_manager
        .get_session(&h.session_id, true)
        .await
        .unwrap()
        .conversation
        .unwrap()
        .messages()
        .iter()
        .filter(|m| !m.is_user_visible())
        .map(|m| m.id.clone().expect("a stored row always has an id"))
        .collect();
    assert_eq!(
        hidden.len(),
        1,
        "expected exactly one model-only row; stored: {:#?}",
        stored_ids(&h).await
    );
    let streamed_message_ids: std::collections::HashSet<String> = events
        .iter()
        .filter_map(|ev| match ev {
            AgentEvent::Message(m) => m.id.clone(),
            _ => None,
        })
        .collect();
    assert!(
        !streamed_message_ids.contains(&hidden[0]),
        "the model-only row must NOT be drawn: it is not yielded as a Message"
    );

    // ...and it is published, flagged as not-for-drawing.
    let published: Vec<(String, bool)> = events
        .iter()
        .filter_map(|ev| match ev {
            AgentEvent::MessagesPersisted(rows) => Some(rows),
            _ => None,
        })
        .flatten()
        .map(|p| (p.id.clone(), p.user_visible))
        .collect();
    assert!(
        published.contains(&(hidden[0].clone(), false)),
        "the hidden row must be published as user_visible=false, so a client can \
         name it without drawing it; published: {published:#?}"
    );

    // The general promise still holds on this turn too.
    let known = ids_a_client_can_learn(&events);
    let stored = stored_ids(&h).await;
    let unknowable: Vec<&(String, String)> = stored
        .iter()
        .filter(|(id, _)| !known.contains(id))
        .collect();
    assert!(
        unknowable.is_empty(),
        "unknowable: {unknowable:#?}\nstreamed ids: {known:#?}\nstored: {stored:#?}"
    );
}

/// #59: nothing may be published that is not durable.
///
/// The published id is what a client will hand back as `expectedMessageIds`, so
/// claiming a row that was never written would make the guard refuse a perfectly
/// fresh session. Every id the turn published must name a row that is actually
/// on disk when the stream ends — the converse of the assertion above.
#[tokio::test(flavor = "multi_thread")]
async fn nothing_is_published_that_the_store_does_not_hold() {
    let (h, _provider) = harness(|p| p.tools_on(0)).await;

    let events = h.run_turn(USER_PROMPT).await.unwrap();
    let stored: std::collections::HashSet<String> =
        stored_ids(&h).await.into_iter().map(|(id, _)| id).collect();

    let published: Vec<String> = events
        .iter()
        .filter_map(|ev| match ev {
            AgentEvent::MessagesPersisted(rows) => Some(rows),
            _ => None,
        })
        .flatten()
        .map(|p| p.id.clone())
        .collect();
    assert!(
        !published.is_empty(),
        "the turn must have published something"
    );

    let phantom: Vec<&String> = published
        .iter()
        .filter(|id| !stored.contains(*id))
        .collect();
    assert!(
        phantom.is_empty(),
        "published {} id(s) the store does not hold: {phantom:#?}",
        phantom.len()
    );
}

/// #59 contract: `userVisible: true` on a published row is **not** an
/// instruction to draw it.
///
/// `PersistedMessage::user_visible` is `Message::is_user_visible()`, and the
/// rows one streamed reply is split into — the rebuilt thinking row and one
/// `tool_use` row per request — are built with `Message::assistant()` /
/// `Message::new`, i.e. `MessageMetadata::default()`, i.e. `user_visible: true`.
/// Only the first of them keeps the reply's id; every later one is stored under
/// a fresh uuid and is therefore an ASSISTANT row, marked visible, that the
/// client was never streamed — even though it was shown that exact content once
/// already, inside the single `filtered_response` it did receive. A client
/// implementing "true means draw" renders the same tool request twice.
///
/// The flag is exact in ONE direction only: `false` means "must not appear in
/// the transcript". `true` means "not hidden", and the frame is for accounting —
/// naming rows so `expectedMessageIds` can be complete.
///
/// This pins both halves, because the tempting "fix" is to flip the split rows
/// to `user_visible: false` so the flag can mean "draw this" — which would be a
/// real regression: those rows ARE the transcript when the session is re-read
/// from disk, so hiding them erases the assistant side of every tool call.
#[tokio::test(flavor = "multi_thread")]
async fn a_published_user_visible_row_is_not_an_instruction_to_draw() {
    let (h, provider) = harness(|p| p.tools_on(0)).await;

    let events = h.run_turn(USER_PROMPT).await.unwrap();
    assert_eq!(
        provider.main_call_count(),
        2,
        "the tool call must have run and the loop come back for a second reply"
    );

    let drawn: std::collections::HashSet<String> = events
        .iter()
        .filter_map(|ev| match ev {
            AgentEvent::Message(m) => m.id.clone(),
            _ => None,
        })
        .collect();
    let published: std::collections::HashMap<String, bool> = events
        .iter()
        .filter_map(|ev| match ev {
            AgentEvent::MessagesPersisted(rows) => Some(rows),
            _ => None,
        })
        .flatten()
        .map(|p| (p.id.clone(), p.user_visible))
        .collect();

    // Assistant rows only: the user's own prompt is also visible-and-unstreamed
    // (the client authored it), and counting it would make this pass whatever
    // the split rows are flagged as.
    let split_rows: Vec<String> = h
        .session_manager
        .get_session(&h.session_id, true)
        .await
        .unwrap()
        .conversation
        .unwrap()
        .messages()
        .iter()
        .filter(|m| m.role == rmcp::model::Role::Assistant && m.is_user_visible())
        .filter_map(|m| m.id.clone())
        .filter(|id| !drawn.contains(id))
        .collect();

    assert!(
        !split_rows.is_empty(),
        "expected at least one user-visible ASSISTANT row the client was never \
         streamed: the split `tool_use` row. If that stopped being true, the \
         split rows were probably re-flagged hidden, which erases the assistant \
         side of every tool call from a re-read session.\nstored: {:#?}\n\
         streamed ids: {drawn:#?}",
        stored_ids(&h).await
    );
    for id in &split_rows {
        assert_eq!(
            published.get(id),
            Some(&true),
            "row {id} is a user-visible assistant row the client never saw, so \
             `MessagesPersisted` must publish it as user_visible=true, which is \
             precisely why that flag cannot mean \"draw this\".\npublished: \
             {published:#?}"
        );
    }
}

/// The signed-provider counterpart to
/// `a_published_user_visible_row_is_not_an_instruction_to_draw`.
///
/// When a provider signs its reasoning — Bedrock extended thinking, Anthropic
/// direct, Snowflake — the signature authenticates the exact assistant block
/// list it emitted, so the reply loop must rebuild that grouping into ONE row
/// instead of splitting it. Splitting mutates the signed prefix and the next
/// request is rejected.
///
/// The protection the split-path test guards has to survive that change: those
/// `ToolRequest` blocks ARE the assistant half of the transcript when the
/// session is re-read from disk. If a future change drops them, re-splits them,
/// or hides them behind `user_visible: false` so the flag can be read as "draw
/// this", a reloaded chat shows tool results answering nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_signed_reply_keeps_the_assistant_side_of_its_tool_calls_in_one_row() {
    let (h, provider) = harness(|p| p.tools_on(0).sign_reasoning()).await;

    let events = h.run_turn(USER_PROMPT).await.unwrap();
    assert_eq!(provider.main_call_count(), 2);

    let published: std::collections::HashMap<String, bool> = events
        .iter()
        .filter_map(|ev| match ev {
            AgentEvent::MessagesPersisted(rows) => Some(rows),
            _ => None,
        })
        .flatten()
        .map(|p| (p.id.clone(), p.user_visible))
        .collect();
    let stored = h
        .session_manager
        .get_session(&h.session_id, true)
        .await
        .unwrap()
        .conversation
        .unwrap();

    // 1. Exactly one stored assistant row carries the signature, and the client
    //    was streamed that same id.
    let signed_rows: Vec<&Message> = stored
        .messages()
        .iter()
        .filter(|m| m.role == rmcp::model::Role::Assistant)
        .filter(|m| {
            m.content
                .iter()
                .any(|c| matches!(c, MessageContent::Thinking(t) if t.signature == "sig-mock"))
        })
        .collect();
    assert_eq!(
        signed_rows.len(),
        1,
        "a signed reply must be rebuilt into exactly one assistant row; \
         splitting it mutates the block list the signature covers.\nstored: {:#?}",
        stored_ids(&h).await
    );
    let signed = signed_rows[0];
    let signed_id = signed.id.clone().expect("a stored row is always named");
    let streamed_ids: std::collections::HashSet<String> = events
        .iter()
        .filter_map(|ev| match ev {
            AgentEvent::Message(m) => m.id.clone(),
            _ => None,
        })
        .collect();
    assert!(streamed_ids.contains(&signed_id));

    // 2. Canonical order, asserted positionally — a set would not catch a
    //    reordering, and reordering is what the signature actually forbids.
    let kinds: Vec<&str> = signed
        .content
        .iter()
        .map(|c| match c {
            MessageContent::Text(_) => "Text",
            MessageContent::ToolRequest(_) => "ToolRequest",
            MessageContent::Thinking(_) => "Thinking",
            MessageContent::RedactedThinking(_) => "RedactedThinking",
            _ => "Other",
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["Thinking", "Text", "ToolRequest", "ToolRequest"],
        "signed content must keep the provider's own order"
    );

    // 3. Visible in the store AND published as visible — which is why the flag
    //    still cannot mean "draw this".
    assert!(signed.is_user_visible());
    assert_eq!(published.get(&signed_id), Some(&true));

    // 4. The load-bearing anti-erasure assertion: both requests survive on the
    //    assistant side, however many rows that side occupies.
    let assistant_requests: usize = stored
        .messages()
        .iter()
        .filter(|m| m.role == rmcp::model::Role::Assistant)
        .flat_map(|m| &m.content)
        .filter(|c| matches!(c, MessageContent::ToolRequest(_)))
        .count();
    assert_eq!(
        assistant_requests, 2,
        "both tool requests must remain on the assistant side of a re-read \
         session, or the stored tool responses answer nothing"
    );

    // 5. Every request is answered by a stored response.
    let request_ids: std::collections::HashSet<String> = signed
        .content
        .iter()
        .filter_map(|c| match c {
            MessageContent::ToolRequest(request) => Some(request.id.clone()),
            _ => None,
        })
        .collect();
    let response_ids: std::collections::HashSet<String> = stored
        .messages()
        .iter()
        .flat_map(|m| &m.content)
        .filter_map(|c| match c {
            MessageContent::ToolResponse(response) => Some(response.id.clone()),
            _ => None,
        })
        .collect();
    assert!(
        request_ids.is_subset(&response_ids),
        "unanswered signed tool requests: {:#?}",
        request_ids.difference(&response_ids).collect::<Vec<_>>()
    );

    // 6. The #59 net still holds: nothing is stored that the client was never named.
    let unpublished: Vec<String> = stored
        .messages()
        .iter()
        .filter_map(|m| m.id.clone())
        .filter(|id| !published.contains_key(id))
        .collect();
    assert!(unpublished.is_empty(), "unpublished rows: {unpublished:#?}");
}

// ── #59 ordering: content first, then the frame that names it ────────────────
//
// Every assertion above reduces a turn to a SET of ids and asks only *whether*
// an id arrived. Ordering is invisible to that shape, and ordering is half the
// contract: `MessagesPersisted` is an accounting frame, and accounting that
// arrives before the thing it accounts for is how a client ends up believing a
// truncated transcript is complete.

/// A one-line shape per event, so an ordering assertion reads the way the wire
/// does: `Message` frames carry their role, `MessagesPersisted` its row count.
fn event_shapes(events: &[AgentEvent]) -> Vec<String> {
    events
        .iter()
        .map(|ev| match ev {
            AgentEvent::Message(m) => match m.role {
                rmcp::model::Role::User => "Message(user)".to_string(),
                rmcp::model::Role::Assistant => "Message(assistant)".to_string(),
            },
            AgentEvent::MessagesPersisted(rows) => format!("MessagesPersisted[{}]", rows.len()),
            AgentEvent::HistoryReplaced(_) => "HistoryReplaced".to_string(),
            other => format!("{other:?}"),
        })
        .collect()
}

/// Every id a turn published in a `MessagesPersisted` that a **later** `Message`
/// frame turns out to carry — i.e. every id the client was handed before the
/// message it names.
///
/// This is the ordering the server's own SSE adapter is built around: it flushes
/// the coalescer before forwarding a `MessagesPersisted`, "so the client never
/// learns an id before the message it belongs to"
/// (`crates/biorouter-server/src/routes/reply.rs`). The adapter can only flush
/// what it is *holding* — an order the agent loop emits backwards passes through
/// it untouched, so the invariant has to hold at the source.
fn ids_named_before_their_own_message(events: &[AgentEvent]) -> Vec<String> {
    let mut named: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut early = Vec::new();
    for ev in events {
        match ev {
            AgentEvent::MessagesPersisted(rows) => {
                named.extend(rows.iter().map(|r| r.id.clone()));
            }
            AgentEvent::Message(m) => {
                if let Some(id) = m.id.as_ref().filter(|id| named.contains(*id)) {
                    early.push(id.clone());
                }
            }
            _ => {}
        }
    }
    early
}

/// #59 ordering: a slash command that answers inline must hand over both
/// messages BEFORE it names them.
///
/// The inline-answer path persisted the user message and the response, published
/// both ids, and only then yielded the two `Message` frames. A client consuming
/// the new frame the way it is meant to be consumed — union its ids into
/// `expectedMessageIds` — that lost the connection in between (any send failure
/// ends the stream, `routes/reply.rs`) would hold the id of **every** stored row
/// while missing the response body. `POST /sessions/{id}/edit_message` checks
/// only `stored ∖ client`, so that client passes the guard with an incomplete
/// transcript and the server truncates rows still on the user's screen. The
/// desktop is insulated today only because it ignores the frame, which is not a
/// property of the wire contract.
///
/// `/clear` is the whole shape in one turn: two rows persisted inline, both
/// yielded, and a history rewrite to announce afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn a_slash_command_hands_over_its_messages_before_it_names_them() {
    let (h, _provider) = harness(|p| p).await;

    let events = h.run_turn("/clear").await.unwrap();

    assert_eq!(
        event_shapes(&events),
        vec![
            "Message(user)",
            "Message(assistant)",
            "MessagesPersisted[2]",
            "HistoryReplaced",
        ],
        "content first, then the accounting frame that names it, then the \
         rewrite: a client that stops reading at any point holds a transcript \
         no shorter than the id set it would claim"
    );

    let early = ids_named_before_their_own_message(&events);
    assert!(
        early.is_empty(),
        "{} id(s) were published before the `Message` frame carrying them: \
         {early:#?}\nevents: {:#?}",
        early.len(),
        event_shapes(&events)
    );
}

/// #59 ordering at the second batch `reply()` can return instead of running the
/// loop: an elicitation answer with nowhere to go.
///
/// BR-41 — a daemon restart between the elicitation and the reply leaves no live
/// request for the answer, so the turn ends immediately: the prompt is stored and
/// an "it was interrupted" notice is streamed. The stored row is never yielded as
/// a `Message` here, so this batch could not corrupt a client's id set the way
/// the slash-command one could. It is ordered the same way regardless, because a
/// rule carrying a per-site exemption ("this one's trailing frame happens not to
/// be persisted") is the reasoning that produced the defect above.
#[tokio::test(flavor = "multi_thread")]
async fn an_undeliverable_elicitation_answer_is_ordered_the_same_way() {
    let (h, _provider) = harness(|p| p).await;

    // Nothing ever elicited, so no live request is waiting on this id — the
    // post-restart state exactly.
    let events = h
        .run_turn_with(Message::user().with_content(
            MessageContent::action_required_elicitation_response(
                "no-such-elicitation",
                serde_json::json!({ "answer": "yes" }),
            ),
        ))
        .await
        .unwrap();

    assert_eq!(
        event_shapes(&events),
        vec!["Message(assistant)", "MessagesPersisted[1]"],
        "content first, then the accounting frame, in the same order every batch \
         uses, so the rule can be checked by reading rather than by re-deriving \
         which rows happen to be yielded at each site"
    );
}

/// The same ordering on every other turn shape #59 drives — the regression net
/// for the audit of the remaining publication sites, not the gate for the defect
/// above (these already held; the inline slash-command path did not).
///
/// A row is allowed to be published without ever being yielded (the model-only
/// plumbing) and to be yielded without being published in that batch. What is
/// never allowed is publishing an id and yielding its message afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn no_turn_shape_names_a_row_before_it_hands_it_over() {
    let (plain, _p) = harness(|p| p).await;
    let plain_events = plain.run_turn(USER_PROMPT).await.unwrap();

    let (with_tools, _p) = harness(|p| p.tools_on(0)).await;
    let tool_events = with_tools.run_turn(USER_PROMPT).await.unwrap();

    let (recovered, _p) = harness(|p| p.server_error_on(0)).await;
    let recovered_events = recovered.run_turn(USER_PROMPT).await.unwrap();

    for (shape, events) in [
        ("an ordinary reply", plain_events),
        ("a reply split across several stored rows", tool_events),
        ("a recovered provider error", recovered_events),
    ] {
        let early = ids_named_before_their_own_message(&events);
        assert!(
            early.is_empty(),
            "{shape}: {} id(s) were published before the `Message` frame \
             carrying them: {early:#?}\nevents: {:#?}",
            early.len(),
            event_shapes(&events)
        );
    }
}
