//! Async subagent handles (BR-40, second half).
//!
//! Wave 1 landed the structured result envelope ([`crate::agents::subagent_result`]);
//! this is the other half of the proposal: a **spawn → poll** model, so a long
//! subagent no longer stalls the parent's turn.
//!
//! A `subagent` call with `background: true` creates the child session, spawns
//! the run on a detached task, registers a [`BackgroundSubagent`] here, and
//! returns the child's *session id* to the parent immediately. The parent then
//! waits on it with `workspace_watch` (or reads it with
//! `workspace_read_conversation`) to collect the same [`SubagentResult`]
//! envelope it would have got synchronously.
//!
//! Invariants worth knowing:
//!
//! * **The parent's cancellation token is not inherited.** A background child
//!   outlives the turn that spawned it by design, so it gets a fresh token —
//!   otherwise the parent's turn ending would kill the very thing that was made
//!   detachable. The token is reachable through the handle (`workspace_close`,
//!   BR-71 decision 23's replacement for the old poll tool's `cancel: true`) and
//!   through the BR-42 active-work view, which
//!   [`crate::agents::subagent_handler::run_complete_subagent_task`] registers.
//! * **Stopping the parent stops what it started, through one function.**
//!   Because nothing inherits the parent's token, a stop of the parent (the
//!   person's Stop, or a decision-model monitor's) reaches its subagents only
//!   through [`stop_delegated_children`], which closes the parent to new
//!   continuations, records why on each handle ([`ChildStopCause`]) and stops
//!   every child with work in flight through [`stop_child_turn`], the same
//!   per-child step `workspace_close` takes.
//! * **The fork-bomb guards still apply.** The in-flight counter is incremented
//!   before the spawn (so a storm of background spawns is refused just like a
//!   storm of blocking ones) and the concurrency semaphore is acquired *inside*
//!   the detached task, so background work queues rather than bypassing the cap.
//! * **Handles are process-local and in-memory.** The child *session* is
//!   persisted as always, but a handle does not survive a restart; a poll after
//!   a restart reports the handle as unknown rather than inventing a result.
//!   What does survive is the child's durable outcome record, written when the
//!   delegated run ends (`subagent_roster::record_outcome`), which is how the
//!   side panel, `biorouter session subagents` and the terminal UI still show a
//!   finished child's status after a restart.
//! * **Every parent-facing spawn runs here.** `handle_subagent_tool` and the
//!   coding-agent bridge both force `background` on (the supervised default),
//!   so every delegated child is a handle in this registry.
//!   `BIOROUTER_SUBAGENT_BACKGROUND` (config or env, default off) now matters
//!   only to a caller that does not force it, which in this tree is a test.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::agents::subagent_result::SubagentResult;
use crate::config::Config;
use crate::conversation::message::Message;
use crate::workspace_services::WorkspaceServices;

/// Is the async-handle path available at all?
///
/// Default off: enabling it adds the `background` parameter to the `subagent`
/// tool's surface, which is a behaviour change for every existing session.
/// (Before BR-71 decision 23 it also added a second, dedicated poll tool; that
/// tool is gone and its jobs are workspace tools now.)
pub fn background_enabled() -> bool {
    Config::global()
        .get_param::<bool>("BIOROUTER_SUBAGENT_BACKGROUND")
        .unwrap_or(false)
}

/// How many *finished* handles are retained **per parent session**. Running
/// handles are never evicted. Beyond the cap the oldest finished, collected
/// handles of that session are dropped. Uncollected results stay authoritative
/// even when the watch lease that originally waited for them has expired.
///
/// Per session, not per process, on purpose: a global cap would let a busy chat
/// evict a quiet chat's uncollected result.
const MAX_RETAINED_FINISHED: usize = 32;

/// Belt-and-braces ceiling on collected finished handles across the registry.
/// It is well above the per-session cap, so it only bites the pathological case
/// without discarding a result the parent still has to collect.
const MAX_RETAINED_FINISHED_TOTAL: usize = 512;

static NEXT_HANDLE_ID: AtomicU64 = AtomicU64::new(1);
static HANDLES: LazyLock<Mutex<Vec<Arc<BackgroundSubagent>>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));
/// Serializes a reaper's supervision decision with every transition that can
/// make an exact child generation newly authoritative. The reaper holds this
/// through cancellation, so registration cannot land between "not supervised"
/// and `cancel()`.
static SUPERVISION_DECISION_GATE: Mutex<()> = Mutex::new(());

#[derive(Debug, Default)]
struct ContinuationPendingState {
    generation: u64,
    reservations: usize,
    committed: bool,
    result_was_current: bool,
    superseded_generation: u64,
    superseded_turn_id: Option<String>,
}

#[derive(Debug, Default)]
struct InitialRunState {
    runtime_ready: bool,
    finished: bool,
    human_intervened: bool,
    pending_user_inputs: Vec<PendingInitialInput>,
}

#[derive(Debug)]
struct PendingInitialInput {
    turn_id: Option<String>,
    message: Message,
}

/// How many distinct pre-start inputs one initializing child will retain.
///
/// A bound rather than an unbounded `Vec`, because this queue is fed by clients
/// that retry: the exact-identity check in [`queue_initializing_child_input`]
/// already collapses a retry storm of the SAME text, so reaching this cap means
/// a client is generating genuinely different messages in a loop. Every message
/// here is concatenated into ONE delegated prompt, so 64 distinct pre-start
/// steers is already far past anything a human produces before a permit frees.
///
/// A count cap and not also a byte cap: each message arrives through a request
/// body the server already bounds, so bounding the count bounds the memory.
const MAX_PENDING_INITIAL_INPUTS: usize = 64;

/// Whether two accepted pre-start inputs are the SAME user turn.
///
/// One definition, shared by the queue's admission check and the transcript
/// idempotency probe in [`crate::agents::subagent_handler`], because those two
/// answer the same question at opposite ends of the same pipe — "have we
/// already taken this?" — and two definitions would disagree exactly where it
/// hurts: the queue accepting a retry the transcript then stores twice.
///
/// Identity is the client's `id` when both sides carry one, and otherwise the
/// `(role, content, provenance)` tuple.
///
/// ⚠ `created` is deliberately NOT part of the tuple, and that is a departure
/// from `SessionManager::add_message`'s replay detector, which does compare it.
/// The clients that send id-less input restamp `created` on every attempt —
/// `biorouter session send` builds its body with `Utc::now().timestamp()` — so a
/// one-second-granularity timestamp makes two attempts at the same text look
/// like two different turns whenever they straddle a second boundary, which is
/// every SSE retry. Including it would make this predicate blind to precisely
/// the duplicates it exists to catch. What it costs is that a user who
/// deliberately sends identical text twice *before the child starts* has it
/// carried once — and since every pre-start input is concatenated into one
/// prompt, the second copy carried no meaning the first did not.
pub(crate) fn is_same_accepted_input(candidate: &Message, existing: &Message) -> bool {
    let ids_agree = match (candidate.id.as_deref(), existing.id.as_deref()) {
        // Two client-supplied ids settle identity outright.
        (Some(candidate_id), Some(existing_id)) => candidate_id == existing_id,
        // Either side unnamed: the content tuple below is the only identity
        // there is.
        _ => true,
    };
    ids_agree
        && candidate.role == existing.role
        && candidate.content == existing.content
        && candidate.metadata.provenance == existing.metadata.provenance
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitialInputDisposition {
    NotInitializing,
    Queued,
    Duplicate,
}

/// Why a subagent was stopped from outside its own run: the chat that started
/// it was stopped, so it was stopped with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildStopCause {
    /// A decision-model monitor on the chat that started the subagent stopped
    /// that chat's turn.
    ParentMonitor { monitor: String },
    /// The person stopped the chat that started the subagent.
    ParentStopped,
    /// A copy of the person's monitor, set on the chat that started the
    /// subagent to also watch every subagent it starts, stopped a sibling. The
    /// whole run stops with it.
    SiblingMonitor { monitor: String, sibling: String },
}

impl ChildStopCause {
    /// The sentence the stopped subagent's result opens with, and the notice
    /// stored in its chat.
    pub fn sentence(&self) -> String {
        match self {
            Self::ParentMonitor { monitor } => format!(
                "Stopped because monitor '{}' stopped the chat that started this subagent.",
                one_line(monitor)
            ),
            Self::ParentStopped => {
                "Stopped because the person stopped the chat that started this subagent."
                    .to_string()
            }
            Self::SiblingMonitor { monitor, sibling } => format!(
                "Stopped because monitor '{}', set by the person to watch every subagent of \
                 that chat, stopped subagent '{}'.",
                one_line(monitor),
                one_line(sibling)
            ),
        }
    }
}

/// `text` on one line, so a name cannot break the sentence it sits in.
fn one_line(text: &str) -> String {
    text.split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// One subagent [`stop_delegated_children`] stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoppedChild {
    pub child_session_id: String,
    /// The handle's title (for a spawn, its task on one line, cut at 80
    /// characters).
    pub title: String,
}

/// What [`stop_child_turn`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildTurnStop {
    /// The delegated run (running, or still queued for a slot) was asked to
    /// stop through its handle. It finishes with whatever it produced so far.
    DelegatedRun,
    /// A later turn (one the person or the parent started after the delegated
    /// run) was cancelled through the daemon; its turn id.
    Turn(String),
    /// Nothing was running.
    Idle,
    /// No delegated run is in flight and there is no daemon to cancel a later
    /// turn through.
    NoDaemon,
}

#[derive(Debug, Clone)]
pub struct TerminalGeneration {
    pub generation: u64,
    pub result: SubagentResult,
}

/// A detached subagent run the parent can poll.
pub struct BackgroundSubagent {
    pub id: String,
    pub parent_session_id: String,
    pub child_session_id: String,
    pub title: String,
    parent_closing: Arc<AtomicBool>,
    started: Instant,
    cancel: CancellationToken,
    /// `None` until the run finishes; then the full result envelope. Using a
    /// watch channel means `wait` is a real await, not a poll loop.
    result: watch::Sender<Option<SubagentResult>>,
    terminal_generation: watch::Sender<Option<TerminalGeneration>>,
    state_version: watch::Sender<u64>,
    /// Whether this handle still represents the child's latest turn. A later
    /// turn invalidates the retained result explicitly; persisted message
    /// counts cannot safely identify generations because the spawn-context row
    /// can race with a very fast child completion.
    result_is_current: std::sync::atomic::AtomicBool,
    /// The first child turn belongs to this handle and must not invalidate it.
    /// Any subsequent turn does.
    initial_turn_started: std::sync::atomic::AtomicBool,
    /// A background child is registered before it receives a concurrency
    /// permit. Until its exact runtime profile is installed, an interactive
    /// reply must be retained as steering for the delegated initial turn, not
    /// allowed to claim that turn on a generic agent.
    initial_run: Mutex<InitialRunState>,
    /// Number of real provider turns that have begun in this child. Watchers
    /// snapshot it when they subscribe so they can distinguish a successor
    /// already in flight from lifecycle frames queued by the original turn.
    child_turn_generation: AtomicU64,
    /// The child-turn generation whose terminal result the parent actually
    /// collected through workspace_watch or workspace_read_conversation.
    /// Completion alone never advances this: a result that lands after a watch
    /// lease expires must still block the parent's final answer.
    collected_generation: Mutex<Option<u64>>,
    /// Stop-and-Send has admitted an exact-generation cancellation, but the
    /// replacement provider turn has not started yet. During this gap the
    /// original result is historical and the parent must keep supervising.
    continuation_pending: Mutex<ContinuationPendingState>,
    /// Why [`stop_delegated_children`] stopped the work in flight, read when
    /// that work ends. The first cause wins. A later turn beginning clears it,
    /// so a cause never outlives the work it stopped.
    stop_cause: Mutex<Option<ChildStopCause>>,
}

impl BackgroundSubagent {
    /// Register a handle whose initial runtime already exists. This remains the
    /// constructor for retained/running-handle bookkeeping; detached spawning
    /// uses [`Self::register_initializing`] because it queues before runtime
    /// installation.
    pub fn register(
        parent_session_id: impl Into<String>,
        child_session_id: impl Into<String>,
        title: impl Into<String>,
        cancel: CancellationToken,
    ) -> Arc<Self> {
        Self::register_with_runtime_state(parent_session_id, child_session_id, title, cancel, true)
    }

    /// Register a detached child before it receives a concurrency permit. The
    /// caller must install the runtime profile and call
    /// [`mark_initial_runtime_ready`] before its delegated provider turn.
    pub fn register_initializing(
        parent_session_id: impl Into<String>,
        child_session_id: impl Into<String>,
        title: impl Into<String>,
        cancel: CancellationToken,
    ) -> Arc<Self> {
        Self::register_with_runtime_state(parent_session_id, child_session_id, title, cancel, false)
    }

    fn register_with_runtime_state(
        parent_session_id: impl Into<String>,
        child_session_id: impl Into<String>,
        title: impl Into<String>,
        cancel: CancellationToken,
        runtime_ready: bool,
    ) -> Arc<Self> {
        let _decision = SUPERVISION_DECISION_GATE
            .lock()
            .expect("subagent supervision decision gate poisoned");
        let parent_session_id = parent_session_id.into();
        let mut handles = HANDLES.lock().expect("subagent handle registry poisoned");
        let parent_closing = handles
            .iter()
            .find(|handle| handle.parent_session_id == parent_session_id)
            .map(|handle| Arc::clone(&handle.parent_closing))
            .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        let id = format!("sub_{}", NEXT_HANDLE_ID.fetch_add(1, Ordering::SeqCst));
        let (result, _) = watch::channel(None);
        let (terminal_generation, _) = watch::channel(None);
        let (state_version, _) = watch::channel(0);
        let handle = Arc::new(Self {
            id,
            parent_session_id,
            child_session_id: child_session_id.into(),
            title: title.into(),
            parent_closing,
            started: Instant::now(),
            cancel,
            result,
            terminal_generation,
            state_version,
            result_is_current: std::sync::atomic::AtomicBool::new(true),
            initial_turn_started: std::sync::atomic::AtomicBool::new(false),
            initial_run: Mutex::new(InitialRunState {
                runtime_ready,
                ..InitialRunState::default()
            }),
            child_turn_generation: AtomicU64::new(0),
            collected_generation: Mutex::new(None),
            continuation_pending: Mutex::new(ContinuationPendingState::default()),
            stop_cause: Mutex::new(None),
        });

        handles.push(handle.clone());
        prune_locked(&mut handles, &handle.parent_session_id);
        handle
    }

    /// Publish the run's result. Idempotent — a second call overwrites, which is
    /// harmless (and never happens on the spawn path, which completes once).
    ///
    /// `send_replace`, not `send`: a `watch` sender with no live receivers treats
    /// `send` as a failure and **throws the value away**, which for a background
    /// subagent nobody happens to be waiting on would silently lose its result.
    pub fn complete(&self, mut result: SubagentResult) {
        // Closes admission WITHOUT draining. `complete` is synchronous and holds
        // no session manager, so anything it took it could only drop — which
        // would make the act of reporting a result destroy the user's steering,
        // and the report would not even say which words were lost. Ownership of
        // the drain stays with the detached completion path, the one caller that
        // can persist what it takes; leaving the messages queued keeps a later
        // `finish_initial_run_and_take_pending` able to recover them.
        //
        // The result is ANNOTATED rather than replaced for the same reason: a
        // completed run's summary is the only record of work the child actually
        // did, and swapping it for an error about the queue threw that away too.
        if self.finish_initial_run_retaining_pending() {
            result.human_intervened = true;
            result.summary = format!(
                "{}\n\n[accepted user steering was not settled before this subagent \
                 completed; it is still queued and was not applied to the run above]",
                result.summary
            );
        }
        if self.initial_run_human_intervened() {
            result.human_intervened = true;
        }

        let _decision = SUPERVISION_DECISION_GATE
            .lock()
            .expect("subagent supervision decision gate poisoned");
        let _continuation = self
            .continuation_pending
            .lock()
            .expect("subagent continuation state poisoned");
        self.result.send_replace(Some(result.clone()));
        self.terminal_generation
            .send_replace(Some(TerminalGeneration {
                generation: self.child_turn_generation(),
                result,
            }));
        self.signal_state_change();
    }

    /// Close initial-turn admission and report whether any accepted input is
    /// still unsettled, WITHOUT claiming it.
    ///
    /// The non-draining sibling of [`finish_initial_run_and_take_pending`], for
    /// the one caller ([`complete`](Self::complete)) that can observe the queue
    /// but cannot persist it.
    fn finish_initial_run_retaining_pending(&self) -> bool {
        let _decision = SUPERVISION_DECISION_GATE
            .lock()
            .expect("subagent supervision decision gate poisoned");
        let mut state = self
            .initial_run
            .lock()
            .expect("subagent initial-run state poisoned");
        state.finished = true;
        !state.pending_user_inputs.is_empty()
    }

    /// Close initial-turn admission and atomically claim every accepted input
    /// that could not reach the delegated runtime. The detached completion path
    /// persists and publishes these messages before publishing its result.
    pub(crate) fn finish_initial_run_and_take_pending(&self) -> Vec<Message> {
        let _decision = SUPERVISION_DECISION_GATE
            .lock()
            .expect("subagent supervision decision gate poisoned");
        let mut state = self
            .initial_run
            .lock()
            .expect("subagent initial-run state poisoned");
        state.finished = true;
        std::mem::take(&mut state.pending_user_inputs)
            .into_iter()
            .map(|pending| pending.message)
            .collect()
    }

    fn initial_run_human_intervened(&self) -> bool {
        self.initial_run
            .lock()
            .expect("subagent initial-run state poisoned")
            .human_intervened
    }

    fn signal_state_change(&self) {
        self.state_version
            .send_modify(|version| *version = version.wrapping_add(1));
    }

    pub fn state_version(&self) -> u64 {
        *self.state_version.borrow()
    }

    pub async fn wait_for_state_change(&self, observed: u64) -> u64 {
        let mut receiver = self.state_version.subscribe();
        let current = *receiver.borrow_and_update();
        if current != observed {
            return current;
        }
        loop {
            if receiver.changed().await.is_err() {
                return *receiver.borrow();
            }
            let current = *receiver.borrow_and_update();
            if current != observed {
                return current;
            }
        }
    }

    pub fn terminal_generation(&self) -> Option<TerminalGeneration> {
        self.terminal_generation.borrow().clone()
    }

    pub fn result_is_current(&self) -> bool {
        self.result_is_current.load(Ordering::Acquire)
    }

    pub fn continuation_pending(&self) -> bool {
        let state = self
            .continuation_pending
            .lock()
            .expect("subagent continuation state poisoned");
        state.committed || state.reservations > 0
    }

    fn has_unsettled_generation(&self) -> bool {
        self.is_running() || self.continuation_pending() || !self.latest_generation_collected()
    }

    pub fn child_turn_generation(&self) -> u64 {
        self.child_turn_generation.load(Ordering::Acquire)
    }

    pub fn latest_generation_collected(&self) -> bool {
        let collected = self
            .collected_generation
            .lock()
            .expect("subagent collection state poisoned");
        let generation = self.child_turn_generation();
        (*collected).is_some_and(|collected| collected == generation)
    }

    /// Record collection only if the result still belongs to `generation`.
    /// A successor beginning between a watch/read and this call makes the
    /// collection stale rather than accidentally collecting the successor.
    pub fn mark_collected_if_generation(&self, generation: u64) -> bool {
        let _decision = SUPERVISION_DECISION_GATE
            .lock()
            .expect("subagent supervision decision gate poisoned");
        let mut collected = self
            .collected_generation
            .lock()
            .expect("subagent collection state poisoned");
        if self.child_turn_generation() != generation {
            return false;
        }
        *collected = Some(generation);
        drop(collected);
        self.signal_state_change();
        true
    }

    /// Collect an exact retained result, never an idle lifecycle state or a
    /// generation already reserved for Stop-and-Send replacement.
    ///
    /// The continuation lock makes its admission atomic with
    /// `mark_continuation_pending_for_turn`. The generation is checked again
    /// after taking the collection lock because `begin_child_turn` advances it
    /// under that lock before invalidating the retained result.
    pub fn mark_current_result_collected_if_generation(&self, generation: u64) -> bool {
        let _decision = SUPERVISION_DECISION_GATE
            .lock()
            .expect("subagent supervision decision gate poisoned");
        let continuation = self
            .continuation_pending
            .lock()
            .expect("subagent continuation state poisoned");
        if continuation.committed
            || continuation.reservations > 0
            || self.is_running()
            || !self.result_is_current()
            || self.child_turn_generation() != generation
        {
            return false;
        }
        let mut collected = self
            .collected_generation
            .lock()
            .expect("subagent collection state poisoned");
        if self.child_turn_generation() != generation || !self.result_is_current() {
            return false;
        }
        *collected = Some(generation);
        drop(collected);
        drop(continuation);
        self.signal_state_change();
        true
    }

    pub fn mark_terminal_generation_collected_if_generation(&self, generation: u64) -> bool {
        let _decision = SUPERVISION_DECISION_GATE
            .lock()
            .expect("subagent supervision decision gate poisoned");
        let continuation = self
            .continuation_pending
            .lock()
            .expect("subagent continuation state poisoned");
        if continuation.committed
            || continuation.reservations > 0
            || self.child_turn_generation() != generation
            || self
                .terminal_generation()
                .is_none_or(|terminal| terminal.generation != generation)
        {
            return false;
        }
        let mut collected = self
            .collected_generation
            .lock()
            .expect("subagent collection state poisoned");
        if self.child_turn_generation() != generation
            || self
                .terminal_generation()
                .is_none_or(|terminal| terminal.generation != generation)
        {
            return false;
        }
        *collected = Some(generation);
        drop(collected);
        drop(continuation);
        self.signal_state_change();
        true
    }

    pub fn rollback_terminal_generation_collection(&self, generation: u64) {
        let _decision = SUPERVISION_DECISION_GATE
            .lock()
            .expect("subagent supervision decision gate poisoned");
        let mut collected = self
            .collected_generation
            .lock()
            .expect("subagent collection state poisoned");
        if self.child_turn_generation() == generation && *collected == Some(generation) {
            *collected = None;
            drop(collected);
            self.signal_state_change();
        }
    }

    pub fn superseded_turn_id(&self) -> Option<String> {
        self.continuation_pending
            .lock()
            .expect("subagent continuation state poisoned")
            .superseded_turn_id
            .clone()
    }

    pub fn superseded_child_turn_generation(&self) -> u64 {
        self.continuation_pending
            .lock()
            .expect("subagent continuation state poisoned")
            .superseded_generation
    }

    pub fn is_running(&self) -> bool {
        self.result.borrow().is_none()
    }

    /// Registered but not yet running its delegated turn: still waiting for a
    /// concurrency permit, or its runtime profile is not installed yet. The
    /// subagent roster shows this as "queued".
    pub fn is_initializing(&self) -> bool {
        let state = self
            .initial_run
            .lock()
            .expect("subagent initial-run state poisoned");
        !state.finished && !state.runtime_ready
    }

    /// The finished result, if the run has ended.
    pub fn result(&self) -> Option<SubagentResult> {
        self.result.borrow().clone()
    }

    /// Ask the run to stop. The child observes the token at its next checkpoint
    /// and finishes with whatever it produced so far.
    pub fn cancel(&self) {
        self.cancel.cancel();
        self.signal_state_change();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Why the chat that started this subagent stopped its work in flight, if
    /// it did ([`stop_delegated_children`]). Read when that work ends, to say
    /// so in its result.
    pub fn stop_cause(&self) -> Option<ChildStopCause> {
        self.stop_cause
            .lock()
            .expect("subagent stop cause poisoned")
            .clone()
    }

    /// Record why the work in flight is being stopped. The first cause wins:
    /// `false` when one was already recorded.
    fn record_stop_cause(&self, cause: &ChildStopCause) -> bool {
        let mut recorded = self
            .stop_cause
            .lock()
            .expect("subagent stop cause poisoned");
        if recorded.is_some() {
            return false;
        }
        *recorded = Some(cause.clone());
        true
    }

    /// Withdraw a cause this stop recorded when it turned out to stop nothing
    /// (the turn ended on its own first), so it is not read as the reason a
    /// turn that finished normally ended.
    fn withdraw_stop_cause(&self, cause: &ChildStopCause) {
        let mut recorded = self
            .stop_cause
            .lock()
            .expect("subagent stop cause poisoned");
        if recorded.as_ref() == Some(cause) {
            *recorded = None;
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Block for up to `timeout` for the run to finish. `None` means it is still
    /// running when the timeout expires — a poll, not a failure.
    pub async fn wait(&self, timeout: Duration) -> Option<SubagentResult> {
        // Subscribe *before* reading, so a result published between the read and
        // the subscribe cannot be missed (a fresh receiver marks the current
        // value seen, and `changed()` only reports later sends).
        let mut rx = self.result.subscribe();
        if let Some(result) = rx.borrow_and_update().clone() {
            return Some(result);
        }
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.checked_duration_since(Instant::now())?;
            match tokio::time::timeout(remaining, rx.changed()).await {
                // Sender dropped without a result: the task died. Report it
                // rather than parking the parent forever.
                Ok(Err(_)) => {
                    return Some(SubagentResult::from_error(
                        "Background subagent task ended without producing a result",
                    ))
                }
                Ok(Ok(())) => {
                    if let Some(result) = rx.borrow_and_update().clone() {
                        return Some(result);
                    }
                }
                Err(_) => return None,
            }
        }
    }

    /// Wait until this handle publishes its result, with no independent
    /// deadline. Callers must supply their own cancellation boundary. This is
    /// the supervision primitive used by `workspace_watch`: the watch tool has
    /// an explicit caller-visible lease, while the handle must never disappear
    /// at an unrelated hidden ten-minute deadline.
    pub async fn wait_until_complete(&self) -> SubagentResult {
        let mut rx = self.result.subscribe();
        if let Some(result) = rx.borrow_and_update().clone() {
            return result;
        }
        loop {
            if rx.changed().await.is_err() {
                return SubagentResult::from_error(
                    "Background subagent task ended without producing a result",
                );
            }
            if let Some(result) = rx.borrow_and_update().clone() {
                return result;
            }
        }
    }

    /// A serializable view for the tool's structured content.
    pub fn snapshot(&self) -> HandleSnapshot {
        let result = self.result();
        let state = match &result {
            None if self.is_cancelled() => HandleState::Cancelling,
            None => HandleState::Running,
            Some(_) => HandleState::Finished,
        };
        HandleSnapshot {
            handle: self.id.clone(),
            title: self.title.clone(),
            child_session_id: self.child_session_id.clone(),
            state,
            elapsed_seconds: self.elapsed().as_secs(),
            result,
        }
    }
}

/// Retain a proven user reply while a background child's delegated runtime is
/// still being assembled. The route returns 202 without taking the session turn
/// lock; the delegated runner drains this queue atomically when its runtime
/// profile is ready.
pub fn queue_initializing_child_input(
    child_session_id: &str,
    turn_id: Option<String>,
    message: Message,
) -> InitialInputDisposition {
    let handles = HANDLES.lock().expect("subagent handle registry poisoned");
    let Some(handle) = handles
        .iter()
        .find(|handle| handle.child_session_id == child_session_id && handle.is_running())
    else {
        return InitialInputDisposition::NotInitializing;
    };
    let mut state = handle
        .initial_run
        .lock()
        .expect("subagent initial-run state poisoned");
    if state.runtime_ready || state.finished {
        return InitialInputDisposition::NotInitializing;
    }
    // A client-supplied turn id is the cheapest and most exact answer, but it is
    // OPTIONAL on the wire: `/reply` forwards `request.turn_id`, and the CLI's
    // `session send` sends none. Without a fallback the push was unconditional,
    // so a renderer that re-POSTs after reading 202-with-no-event-stream as a
    // dropped connection appended another copy of the user's text on every
    // retry — and `mark_initial_runtime_ready` then spliced all of them into the
    // child's opening prompt.
    let already_accepted = state.pending_user_inputs.iter().any(|pending| {
        match (pending.turn_id.as_deref(), turn_id.as_deref()) {
            (Some(accepted), Some(candidate)) => accepted == candidate,
            _ => is_same_accepted_input(&message, &pending.message),
        }
    });
    if already_accepted {
        return InitialInputDisposition::Duplicate;
    }
    if state.pending_user_inputs.len() >= MAX_PENDING_INITIAL_INPUTS {
        // Reported as `Duplicate` because that is the disposition's only other
        // "accepted, nothing new queued" answer and the three variants are the
        // HTTP route's contract in another crate; a fourth would be a
        // cross-crate breaking change for a case that means a runaway client.
        // Logged rather than silent, so the drop is diagnosable.
        tracing::warn!(
            child_session_id,
            queued = state.pending_user_inputs.len(),
            "pre-start steering queue is at its cap; dropping further input for this child"
        );
        return InitialInputDisposition::Duplicate;
    }
    state
        .pending_user_inputs
        .push(PendingInitialInput { turn_id, message });
    state.human_intervened = true;
    InitialInputDisposition::Queued
}

/// Atomically make the child eligible to claim its delegated initial turn and
/// return every user reply that arrived while it was waiting for a permit.
pub fn mark_initial_runtime_ready(child_session_id: &str) -> Vec<Message> {
    let handles = HANDLES.lock().expect("subagent handle registry poisoned");
    let Some(handle) = handles
        .iter()
        .find(|handle| handle.child_session_id == child_session_id && handle.is_running())
    else {
        return Vec::new();
    };
    let mut state = handle
        .initial_run
        .lock()
        .expect("subagent initial-run state poisoned");
    if state.finished {
        return Vec::new();
    }
    state.runtime_ready = true;
    if !state.pending_user_inputs.is_empty() {
        state.human_intervened = true;
    }
    state
        .pending_user_inputs
        .iter()
        .map(|pending| pending.message.clone())
        .collect()
}

/// Acknowledge the initialization queue only after its combined delegated
/// message is durable. Until this point completion can still recover the exact
/// accepted inputs if setup fails or panics after runtime admission.
pub fn settle_initial_runtime_inputs(child_session_id: &str) {
    let handles = HANDLES.lock().expect("subagent handle registry poisoned");
    let Some(handle) = handles
        .iter()
        .find(|handle| handle.child_session_id == child_session_id && handle.is_running())
    else {
        return;
    };
    let mut state = handle
        .initial_run
        .lock()
        .expect("subagent initial-run state poisoned");
    if state.runtime_ready && !state.finished {
        state.pending_user_inputs.clear();
    }
}

pub fn is_child_initializing(child_session_id: &str) -> bool {
    let handles = HANDLES.lock().expect("subagent handle registry poisoned");
    handles.iter().any(|handle| {
        if handle.child_session_id != child_session_id {
            return false;
        }
        let state = handle
            .initial_run
            .lock()
            .expect("subagent initial-run state poisoned");
        !state.finished && !state.runtime_ready
    })
}

/// Cancel a child that is still waiting for its delegated initial runtime. Once
/// ready, the daemon's ordinary active-turn cancellation owns the lifecycle.
pub fn cancel_initializing_child(child_session_id: &str) -> bool {
    let handles = HANDLES.lock().expect("subagent handle registry poisoned");
    let Some(handle) = handles.iter().find(|handle| {
        if handle.child_session_id != child_session_id {
            return false;
        }
        let state = handle
            .initial_run
            .lock()
            .expect("subagent initial-run state poisoned");
        !state.finished && !state.runtime_ready
    }) else {
        return false;
    };
    handle.cancel();
    true
}

pub fn open_parent_continuation_admission(parent_session_id: &str) {
    let handles = HANDLES.lock().expect("subagent handle registry poisoned");
    for handle in handles
        .iter()
        .filter(|handle| handle.parent_session_id == parent_session_id)
    {
        handle.parent_closing.store(false, Ordering::Release);
        handle.signal_state_change();
    }
}

pub fn begin_parent_closing(parent_session_id: &str) {
    let handles = HANDLES.lock().expect("subagent handle registry poisoned");
    for handle in handles
        .iter()
        .filter(|handle| handle.parent_session_id == parent_session_id)
    {
        handle.parent_closing.store(true, Ordering::Release);
        handle.signal_state_change();
    }
}

/// Stop every subagent `parent_session_id` started that has work in flight,
/// because the parent itself was stopped: by the person, or by one of its
/// decision-model monitors. Returns the children it stopped, oldest first.
///
/// Children run on their own cancellation token (see the module doc), so
/// without this a stopped parent's subagents keep running and spending. For
/// the parent this:
/// 1. closes it to continuations ([`begin_parent_closing`]), so a
///    Stop-and-Send on one of its children is refused from here on;
/// 2. for each child with work in flight (its delegated run, running or still
///    queued; a later turn the daemon runs; or a continuation waiting to
///    start) records `cause` on its handle, where the run's result reads it,
///    and stops it through [`stop_child_turn`].
///
/// A finished child is left alone. A run something else already asked to stop
/// is still reported (it is still unwinding) but keeps its own reason.
///
/// Uses the daemon's services when they are installed. Never called with the
/// handle registry locked: the daemon's turn cancellation takes its own turn
/// registry lock.
pub fn stop_delegated_children(
    parent_session_id: &str,
    cause: ChildStopCause,
) -> Vec<StoppedChild> {
    let services = crate::workspace_services::get();
    stop_children_with(parent_session_id, None, &cause, services.as_deref())
}

/// [`stop_delegated_children`] for every child but `except_child_session_id`:
/// the subagent whose own monitor stopped it, and so stopped the rest of the
/// run with it.
pub fn stop_delegated_children_except(
    parent_session_id: &str,
    except_child_session_id: &str,
    cause: ChildStopCause,
) -> Vec<StoppedChild> {
    let services = crate::workspace_services::get();
    stop_children_with(
        parent_session_id,
        Some(except_child_session_id),
        &cause,
        services.as_deref(),
    )
}

fn stop_children_with(
    parent_session_id: &str,
    except_child_session_id: Option<&str>,
    cause: &ChildStopCause,
    services: Option<&dyn WorkspaceServices>,
) -> Vec<StoppedChild> {
    begin_parent_closing(parent_session_id);
    // A snapshot: the registry lock is released before any service is asked.
    // The daemon's turn cancellation takes its turn registry lock, and its
    // Stop-and-Send admission takes this registry while holding that lock
    // (`mark_continuation_pending_for_turn`), so holding this one across a
    // service call would invert the order.
    let handles = list_for_session(parent_session_id);
    let mut stopped: Vec<StoppedChild> = Vec::new();
    for handle in handles {
        let child = handle.child_session_id.as_str();
        if except_child_session_id == Some(child) {
            continue;
        }
        let delegated_run = handle.is_running();
        let later_turn = !delegated_run && services.is_some_and(|svc| svc.is_turn_active(child));
        let continuation = handle.continuation_pending();
        if !(delegated_run || later_turn || continuation) {
            continue;
        }
        // Recorded BEFORE the token trips, so the run's completion, which the
        // trip wakes, finds it. A run already asked to stop (another agent's
        // workspace_close, an earlier stop) keeps its own reason.
        let recorded = !(delegated_run && handle.is_cancelled()) && handle.record_stop_cause(cause);
        let outcome = stop_child_turn(Some(handle.as_ref()), child, services);
        let acted = continuation
            || matches!(
                outcome,
                ChildTurnStop::DelegatedRun | ChildTurnStop::Turn(_)
            );
        if !acted {
            // The later turn ended on its own before it could be cancelled.
            if recorded {
                handle.withdraw_stop_cause(cause);
            }
            continue;
        }
        if !stopped.iter().any(|done| done.child_session_id == child) {
            stopped.push(StoppedChild {
                child_session_id: child.to_string(),
                title: handle.title.clone(),
            });
        }
    }
    stopped
}

/// Stop the work a child session is doing right now: the one per-child step
/// of every stop that reaches a subagent from outside it (`workspace_close`
/// scope `turn`, and [`stop_delegated_children`]).
///
/// First abandons any Stop-and-Send continuation waiting to start, through
/// the daemon when there is one. Then, when `handle` (the caller's handle on
/// this child) has its delegated run in flight, running or still queued,
/// trips that run's token; otherwise asks the daemon to cancel the turn it
/// runs for the child.
pub fn stop_child_turn(
    handle: Option<&BackgroundSubagent>,
    child_session_id: &str,
    services: Option<&dyn WorkspaceServices>,
) -> ChildTurnStop {
    match services {
        Some(services) => {
            services.abandon_pending_continuations(child_session_id);
        }
        None => {
            abandon_continuation(child_session_id);
        }
    }
    if let Some(handle) = handle.filter(|handle| handle.is_running()) {
        handle.cancel();
        return ChildTurnStop::DelegatedRun;
    }
    match services {
        Some(services) => match services.cancel_turn(child_session_id) {
            Some(turn_id) => ChildTurnStop::Turn(turn_id),
            None => ChildTurnStop::Idle,
        },
        None => ChildTurnStop::NoDaemon,
    }
}

pub fn record_child_turn_terminal(child_session_id: &str, result: SubagentResult) {
    let _decision = SUPERVISION_DECISION_GATE
        .lock()
        .expect("subagent supervision decision gate poisoned");
    let handles = HANDLES.lock().expect("subagent handle registry poisoned");
    for handle in handles
        .iter()
        .filter(|handle| handle.child_session_id == child_session_id)
    {
        let _continuation = handle
            .continuation_pending
            .lock()
            .expect("subagent continuation state poisoned");
        handle
            .terminal_generation
            .send_replace(Some(TerminalGeneration {
                generation: handle.child_turn_generation(),
                result: result.clone(),
            }));
        handle.signal_state_change();
    }
}

/// Mark the start of a real provider turn in a child session.
///
/// A detached registration is ineligible until [`mark_initial_runtime_ready`]
/// atomically installs its runtime boundary and drains pre-start steering. Its
/// first eligible call claims the handle instead of invalidating it. A later
/// call means the user has continued or redirected the child and any retained
/// result from the original delegated run is now historical.
pub fn begin_child_turn(child_session_id: &str) {
    let _decision = SUPERVISION_DECISION_GATE
        .lock()
        .expect("subagent supervision decision gate poisoned");
    let handles = HANDLES.lock().expect("subagent handle registry poisoned");
    for handle in handles
        .iter()
        .filter(|handle| handle.child_session_id == child_session_id)
    {
        if !handle
            .initial_run
            .lock()
            .expect("subagent initial-run state poisoned")
            .runtime_ready
        {
            continue;
        }
        advance_child_turn(handle);
    }
}

/// Admit a child turn once, even when more than one runtime layer observes its
/// start. The server calls this before fallible setup;
/// [`crate::agents::Agent::reply`] calls it again as a fallback for non-server
/// callers. A generation with no matching terminal is already in flight and
/// must not be advanced twice.
pub fn admit_child_turn(child_session_id: &str) -> bool {
    let _decision = SUPERVISION_DECISION_GATE
        .lock()
        .expect("subagent supervision decision gate poisoned");
    let handles = HANDLES.lock().expect("subagent handle registry poisoned");
    let mut admitted = false;
    for handle in handles
        .iter()
        .filter(|handle| handle.child_session_id == child_session_id)
    {
        if !handle
            .initial_run
            .lock()
            .expect("subagent initial-run state poisoned")
            .runtime_ready
        {
            continue;
        }
        admitted = true;
        let current_generation = handle.child_turn_generation();
        let generation_in_flight = handle.initial_turn_started.load(Ordering::Acquire)
            && handle
                .terminal_generation()
                .is_none_or(|terminal| terminal.generation != current_generation);
        if generation_in_flight {
            continue;
        }
        advance_child_turn(handle);
    }
    admitted
}

fn advance_child_turn(handle: &BackgroundSubagent) {
    let mut state = handle
        .continuation_pending
        .lock()
        .expect("subagent continuation state poisoned");
    let _collection = handle
        .collected_generation
        .lock()
        .expect("subagent collection state poisoned");
    let next_generation = handle.child_turn_generation.fetch_add(1, Ordering::AcqRel) + 1;
    let initial_turn_was_unclaimed = !handle.initial_turn_started.swap(true, Ordering::AcqRel);
    let is_initial_running_turn = handle.is_running() && initial_turn_was_unclaimed;
    if is_initial_running_turn {
        // A fast Stop-and-Send can reserve the continuation after the server
        // publishes the initial TurnStarted but before the agent reaches this
        // hook. That initial turn is still the superseded generation, not the
        // promised replacement.
        if state.committed || state.reservations > 0 {
            state.superseded_generation = next_generation;
        }
        drop(state);
        handle.signal_state_change();
        return;
    }
    // A later turn is new work: a reason recorded for the work before it is
    // history and must not be read as this turn's.
    *handle
        .stop_cause
        .lock()
        .expect("subagent stop cause poisoned") = None;
    let was_pending = state.committed || state.reservations > 0;
    if was_pending {
        state.generation = state.generation.wrapping_add(1);
        state.reservations = 0;
        state.committed = false;
    }
    if !was_pending {
        state.generation = state.generation.wrapping_add(1);
        state.superseded_generation = next_generation.saturating_sub(1);
        state.superseded_turn_id = None;
    }
    handle.result_is_current.store(false, Ordering::Release);
    drop(state);
    handle.signal_state_change();
}

/// A reversible Stop-and-Send admission mark.
///
/// The route keeps this ticket until cancellation settles. Rolling it back only
/// restores state that this exact mark changed; if the replacement turn has
/// already begun, [`begin_child_turn`] has cleared the bit and rollback is a
/// no-op.
#[derive(Clone)]
pub struct ContinuationPendingMark {
    handles: Vec<ContinuationReservation>,
    refused_parent_closing: bool,
}

#[derive(Clone)]
struct ContinuationReservation {
    handle: Arc<BackgroundSubagent>,
    generation: u64,
    resolution: Arc<AtomicU8>,
}

impl std::fmt::Debug for ContinuationPendingMark {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContinuationPendingMark")
            .field("handles", &self.handles.len())
            .finish()
    }
}

impl ContinuationPendingMark {
    pub fn commit(&self) {
        for reservation in &self.handles {
            reservation.resolve(true);
        }
    }

    pub fn rollback(&self) {
        for reservation in &self.handles {
            reservation.resolve(false);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }

    pub fn refused_parent_closing(&self) -> bool {
        self.refused_parent_closing
    }
}

impl ContinuationReservation {
    fn resolve(&self, commit: bool) {
        let resolution = if commit { 1 } else { 2 };
        if self
            .resolution
            .compare_exchange(0, resolution, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        let _decision = SUPERVISION_DECISION_GATE
            .lock()
            .expect("subagent supervision decision gate poisoned");
        let mut state = self
            .handle
            .continuation_pending
            .lock()
            .expect("subagent continuation state poisoned");
        if state.generation != self.generation {
            return;
        }
        state.reservations = state.reservations.saturating_sub(1);
        if commit {
            state.committed = true;
        } else if state.reservations == 0 && !state.committed {
            self.handle
                .result_is_current
                .store(state.result_was_current, Ordering::Release);
            state.superseded_turn_id = None;
        }
        drop(state);
        self.handle.signal_state_change();
    }
}

/// Mark every retained delegated-run handle for `child_session_id` as waiting
/// for a replacement turn. The caller must have generation-matched the child's
/// active or safely retained turn before calling this.
pub fn mark_continuation_pending(child_session_id: &str) -> ContinuationPendingMark {
    mark_continuation_pending_for_turn(child_session_id, None)
}

/// Reserve supervision across a Stop-and-Send gap and identify the exact
/// canonical turn whose queued lifecycle frames must remain historical.
pub fn mark_continuation_pending_for_turn(
    child_session_id: &str,
    superseded_turn_id: Option<String>,
) -> ContinuationPendingMark {
    let _decision = SUPERVISION_DECISION_GATE
        .lock()
        .expect("subagent supervision decision gate poisoned");
    let handles = HANDLES.lock().expect("subagent handle registry poisoned");
    if handles.iter().any(|handle| {
        handle.child_session_id == child_session_id && handle.parent_closing.load(Ordering::Acquire)
    }) {
        return ContinuationPendingMark {
            handles: Vec::new(),
            refused_parent_closing: true,
        };
    }
    let mut marked = Vec::new();
    for handle in handles
        .iter()
        .filter(|handle| handle.child_session_id == child_session_id)
    {
        let mut state = handle
            .continuation_pending
            .lock()
            .expect("subagent continuation state poisoned");
        if state.reservations == 0 && !state.committed {
            state.generation = state.generation.wrapping_add(1);
            state.result_was_current = handle.result_is_current.swap(false, Ordering::AcqRel);
            state.superseded_generation = handle.child_turn_generation();
            state.superseded_turn_id.clone_from(&superseded_turn_id);
        } else if superseded_turn_id.is_some() && state.superseded_turn_id != superseded_turn_id {
            state.superseded_turn_id.clone_from(&superseded_turn_id);
        }
        state.reservations += 1;
        marked.push(ContinuationReservation {
            handle: Arc::clone(handle),
            generation: state.generation,
            resolution: Arc::new(AtomicU8::new(0)),
        });
        drop(state);
        handle.signal_state_change();
    }
    ContinuationPendingMark {
        handles: marked,
        refused_parent_closing: false,
    }
}

/// Explicitly abandon an admitted continuation, for example when
/// `workspace_close` cancels or evicts the child instead of starting the
/// promised replacement. There is deliberately no elapsed-time fallback:
/// supervision changes only because a successor starts or an explicit close
/// says it will not.
pub fn abandon_continuation(child_session_id: &str) -> bool {
    abandon_continuation_matching(child_session_id, None)
}

pub fn abandon_continuation_for_turn(child_session_id: &str, superseded_turn_id: &str) -> bool {
    abandon_continuation_matching(child_session_id, Some(superseded_turn_id))
}

fn abandon_continuation_matching(child_session_id: &str, superseded_turn_id: Option<&str>) -> bool {
    let _decision = SUPERVISION_DECISION_GATE
        .lock()
        .expect("subagent supervision decision gate poisoned");
    let handles = HANDLES.lock().expect("subagent handle registry poisoned");
    let mut abandoned = false;
    for handle in handles
        .iter()
        .filter(|handle| handle.child_session_id == child_session_id)
    {
        let mut state = handle
            .continuation_pending
            .lock()
            .expect("subagent continuation state poisoned");
        if superseded_turn_id
            .is_some_and(|expected| state.superseded_turn_id.as_deref() != Some(expected))
        {
            continue;
        }
        if state.committed || state.reservations > 0 {
            state.generation = state.generation.wrapping_add(1);
            state.reservations = 0;
            state.committed = false;
            state.superseded_turn_id = None;
            handle
                .result_is_current
                .store(state.result_was_current, Ordering::Release);
            abandoned = true;
            drop(state);
            handle.signal_state_change();
        }
    }
    abandoned
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HandleState {
    Running,
    /// Cancellation was requested but the child has not yet unwound.
    Cancelling,
    /// The run ended; `result` carries the envelope (which has its own
    /// completed / incomplete / error status).
    Finished,
}

#[derive(Debug, Clone, Serialize)]
pub struct HandleSnapshot {
    pub handle: String,
    pub title: String,
    pub child_session_id: String,
    pub state: HandleState,
    pub elapsed_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<SubagentResult>,
}

impl HandleSnapshot {
    /// One line for the parent LLM's list view.
    pub fn to_line(&self) -> String {
        match (&self.state, &self.result) {
            (HandleState::Finished, Some(result)) => format!(
                "{}: {} ({}, {}s): {}",
                self.handle,
                self.title,
                result.status.as_str(),
                self.elapsed_seconds,
                first_line(&result.summary),
            ),
            (state, _) => format!(
                "{}: {} ({}, {}s elapsed)",
                self.handle,
                self.title,
                match state {
                    HandleState::Cancelling => "cancelling",
                    _ => "running",
                },
                self.elapsed_seconds,
            ),
        }
    }
}

fn first_line(text: &str) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let trimmed: String = line.chars().take(160).collect();
    if line.chars().count() > 160 {
        format!("{trimmed}…")
    } else {
        trimmed
    }
}

/// Every handle owned by a parent session, oldest first. Scoped by session so
/// one chat can never poll (or cancel) another chat's children.
pub fn list_for_session(parent_session_id: &str) -> Vec<Arc<BackgroundSubagent>> {
    HANDLES
        .lock()
        .expect("subagent handle registry poisoned")
        .iter()
        .filter(|h| h.parent_session_id == parent_session_id)
        .cloned()
        .collect()
}

/// Reattach supervision when a direct child follow-up starts after its fully
/// collected handle was pruned from the bounded registry.
///
/// The replacement is born as generation one with no matching terminal and a
/// collected historical generation zero. That makes the new generation live
/// immediately at admission, while [`admit_child_turn`] remains idempotent when
/// the runner observes the same start a moment later.
pub fn ensure_direct_followup_handle(
    parent_session_id: &str,
    child_session_id: &str,
    title: &str,
) -> Arc<BackgroundSubagent> {
    let _decision = SUPERVISION_DECISION_GATE
        .lock()
        .expect("subagent supervision decision gate poisoned");
    let mut handles = HANDLES.lock().expect("subagent handle registry poisoned");
    if let Some(handle) = handles.iter().find(|handle| {
        handle.parent_session_id == parent_session_id && handle.child_session_id == child_session_id
    }) {
        return Arc::clone(handle);
    }

    let parent_closing = handles
        .iter()
        .find(|handle| handle.parent_session_id == parent_session_id)
        .map(|handle| Arc::clone(&handle.parent_closing))
        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
    let historical = SubagentResult::from_error("previous child generation already collected");
    let (result, _) = watch::channel(Some(historical));
    let (terminal_generation, _) = watch::channel(None);
    let (state_version, _) = watch::channel(0);
    let handle = Arc::new(BackgroundSubagent {
        id: format!("sub_{}", NEXT_HANDLE_ID.fetch_add(1, Ordering::SeqCst)),
        parent_session_id: parent_session_id.to_string(),
        child_session_id: child_session_id.to_string(),
        title: title.to_string(),
        parent_closing,
        started: Instant::now(),
        cancel: CancellationToken::new(),
        result,
        terminal_generation,
        state_version,
        result_is_current: AtomicBool::new(false),
        initial_turn_started: AtomicBool::new(true),
        initial_run: Mutex::new(InitialRunState {
            runtime_ready: true,
            finished: true,
            ..InitialRunState::default()
        }),
        child_turn_generation: AtomicU64::new(1),
        collected_generation: Mutex::new(Some(0)),
        continuation_pending: Mutex::new(ContinuationPendingState::default()),
        stop_cause: Mutex::new(None),
    });
    handles.push(Arc::clone(&handle));
    prune_locked(&mut handles, parent_session_id);
    handle
}

/// Evaluate whether `session_id` participates in a live delegated supervision
/// boundary, then run `decide` while that answer cannot become stale in the
/// unsafe direction.
///
/// Both sides are covered: a parent owns the handles it must collect, and a
/// child is supervised by any handle retaining its exact current generation.
/// Finished, collected handles do not exempt either session. The callback must
/// not call another subagent-handle state mutation; the audience reaper uses it
/// only to renew its idle clock or cancel the turn.
pub fn with_live_delegated_supervision<R>(session_id: &str, decide: impl FnOnce(bool) -> R) -> R {
    let _decision = SUPERVISION_DECISION_GATE
        .lock()
        .expect("subagent supervision decision gate poisoned");
    let handles = HANDLES.lock().expect("subagent handle registry poisoned");
    let supervised = handles.iter().any(|handle| {
        (handle.parent_session_id == session_id || handle.child_session_id == session_id)
            && handle.has_unsettled_generation()
    });
    drop(handles);
    decide(supervised)
}

/// Look a handle up within its owning session.
pub fn get_for_session(parent_session_id: &str, id: &str) -> Option<Arc<BackgroundSubagent>> {
    HANDLES
        .lock()
        .expect("subagent handle registry poisoned")
        .iter()
        .find(|h| h.parent_session_id == parent_session_id && h.id == id)
        .cloned()
}

/// How many background subagents are still running (test/introspection helper).
pub fn running_count() -> usize {
    HANDLES
        .lock()
        .expect("subagent handle registry poisoned")
        .iter()
        .filter(|h| h.is_running())
        .count()
}

/// Drop the oldest finished handles of `parent_session_id` once it holds more
/// than [`MAX_RETAINED_FINISHED`], then enforce the process-wide ceiling.
/// Running and uncollected handles are never evicted — their results have
/// nowhere else to land, and dropping an uncollected handle would bypass the
/// parent's final-output supervision gate.
fn prune_locked(handles: &mut Vec<Arc<BackgroundSubagent>>, parent_session_id: &str) {
    drop_oldest_finished(handles, MAX_RETAINED_FINISHED, |h| {
        h.parent_session_id == parent_session_id
    });
    drop_oldest_finished(handles, MAX_RETAINED_FINISHED_TOTAL, |_| true);
}

fn drop_oldest_finished(
    handles: &mut Vec<Arc<BackgroundSubagent>>,
    keep: usize,
    in_scope: impl Fn(&BackgroundSubagent) -> bool,
) {
    let finished = handles
        .iter()
        .filter(|h| {
            in_scope(h)
                && !h.is_running()
                && !h.continuation_pending()
                && h.latest_generation_collected()
        })
        .count();
    if finished <= keep {
        return;
    }
    let mut to_drop = finished - keep;
    handles.retain(|h| {
        if to_drop > 0
            && in_scope(h)
            && !h.is_running()
            && !h.continuation_pending()
            && h.latest_generation_collected()
        {
            to_drop -= 1;
            false
        } else {
            true
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::subagent_result::SubagentStatus;
    use crate::conversation::message::Message;
    use crate::conversation::Conversation;

    fn done(text: &str) -> SubagentResult {
        SubagentResult::from_conversation(
            &Conversation::new_unvalidated(vec![Message::assistant().with_text(text)]),
            None,
            true,
        )
    }

    fn new_initializing_handle(parent: &str) -> Arc<BackgroundSubagent> {
        BackgroundSubagent::register_initializing(
            parent,
            format!("child-session-{parent}"),
            "do the thing",
            CancellationToken::new(),
        )
    }

    fn new_handle(parent: &str) -> Arc<BackgroundSubagent> {
        BackgroundSubagent::register(
            parent,
            format!("child-session-{parent}"),
            "do the thing",
            CancellationToken::new(),
        )
    }

    #[test]
    fn handle_starts_running_and_finishes_with_result() {
        let handle = new_handle("parent-running");
        assert!(handle.is_running());
        assert_eq!(handle.snapshot().state, HandleState::Running);
        assert!(handle.result().is_none());

        handle.complete(done("all done"));

        assert!(!handle.is_running());
        let snapshot = handle.snapshot();
        assert_eq!(snapshot.state, HandleState::Finished);
        let result = snapshot.result.expect("finished handle carries a result");
        assert_eq!(result.status, SubagentStatus::Completed);
        assert_eq!(result.summary, "all done");
    }

    #[test]
    fn handles_are_scoped_to_their_parent_session() {
        let mine = new_handle("parent-a");
        let _theirs = new_handle("parent-b");

        let listed = list_for_session("parent-a");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, mine.id);

        assert!(get_for_session("parent-a", &mine.id).is_some());
        // Another session cannot reach into this one's handles.
        assert!(get_for_session("parent-b", &mine.id).is_none());
        assert!(get_for_session("parent-a", "sub_does_not_exist").is_none());
    }

    #[test]
    fn audience_supervision_tracks_only_a_live_exact_generation() {
        let parent = "audience-live-parent";
        let handle = new_handle(parent);
        let child = handle.child_session_id.clone();
        let supervised =
            |session_id: &str| with_live_delegated_supervision(session_id, |supervised| supervised);

        assert!(supervised(parent));
        assert!(supervised(&child));
        assert!(!supervised("persisted-subagent-with-no-live-handle"));

        let generation = handle.child_turn_generation();
        handle.complete(done("finished but not yet collected"));
        assert!(supervised(parent));
        assert!(supervised(&child));

        assert!(handle.mark_current_result_collected_if_generation(generation));
        assert!(!supervised(parent));
        assert!(!supervised(&child));
    }

    #[test]
    fn supervision_decision_serializes_new_handle_registration() {
        let parent = "audience-atomic-parent".to_string();
        let child = "audience-atomic-child".to_string();
        let (decision_entered_tx, decision_entered_rx) = std::sync::mpsc::channel();
        let (registration_attempted_tx, registration_attempted_rx) = std::sync::mpsc::channel();
        let (registration_finished_tx, registration_finished_rx) = std::sync::mpsc::channel();

        let decision_parent = parent.clone();
        let decision = std::thread::spawn(move || {
            with_live_delegated_supervision(&decision_parent, |supervised| {
                assert!(!supervised);
                decision_entered_tx.send(()).unwrap();
                registration_attempted_rx.recv().unwrap();
                assert!(
                    registration_finished_rx
                        .recv_timeout(Duration::from_millis(100))
                        .is_err(),
                    "registration crossed a reaper decision that had already sampled no supervision"
                );
            });
        });

        decision_entered_rx.recv().unwrap();
        let registration_parent = parent.clone();
        let registration = std::thread::spawn(move || {
            registration_attempted_tx.send(()).unwrap();
            let handle = BackgroundSubagent::register(
                registration_parent,
                child,
                "new supervision",
                CancellationToken::new(),
            );
            let _ = registration_finished_tx.send(());
            handle
        });

        decision.join().unwrap();
        let handle = registration.join().unwrap();
        assert!(with_live_delegated_supervision(&parent, |live| live));

        let generation = handle.child_turn_generation();
        handle.complete(done("cleanup"));
        assert!(handle.mark_current_result_collected_if_generation(generation));
    }

    #[test]
    fn cancel_marks_the_handle_cancelling() {
        let handle = new_handle("parent-cancel");
        assert!(!handle.is_cancelled());
        handle.cancel();
        assert!(handle.is_cancelled());
        assert_eq!(handle.snapshot().state, HandleState::Cancelling);
        // Still resolves to a result once the child unwinds.
        handle.complete(done("stopped early"));
        assert_eq!(handle.snapshot().state, HandleState::Finished);
    }

    #[test]
    fn queued_user_input_cannot_claim_the_delegated_initial_turn() {
        let handle = new_initializing_handle("parent-initializing");
        let child = handle.child_session_id.clone();
        let steer = Message::user().with_text("change the requested output");

        assert_eq!(
            queue_initializing_child_input(&child, Some("client-turn-1".into()), steer.clone()),
            InitialInputDisposition::Queued
        );
        assert_eq!(
            queue_initializing_child_input(&child, Some("client-turn-1".into()), steer),
            InitialInputDisposition::Duplicate
        );
        begin_child_turn(&child);
        assert_eq!(handle.child_turn_generation(), 0);
        assert!(handle.result_is_current());

        let queued = mark_initial_runtime_ready(&child);
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].as_concat_text(), "change the requested output");
        begin_child_turn(&child);
        assert_eq!(handle.child_turn_generation(), 1);
        assert!(handle.result_is_current());
    }

    /// Exactly what `/reply` forwards for a `biorouter session send`: no id, no
    /// turn id, and a `created` stamped fresh on every attempt.
    fn reply_shaped_steer(text: &str, created: i64) -> Message {
        let mut message = Message::user().with_text(text).with_provenance(
            crate::conversation::message::MessageProvenance {
                kind: crate::conversation::message::ProvenanceKind::UserDirect,
                from_session_id: None,
                from_session_name: None,
            },
        );
        message.created = created;
        message
    }

    /// The dedup branch used to be guarded by `turn_id.is_some()`, so an
    /// id-less push was appended unconditionally. A renderer that reads
    /// `202 ACCEPTED` with an empty body as a dropped event stream re-POSTs, and
    /// every retry added another copy of the same text to the child's opening
    /// prompt.
    ///
    /// The retries carry DIFFERENT `created` stamps on purpose: that is what a
    /// real retry looks like (each body is built with `Utc::now().timestamp()`),
    /// and it is the case a timestamp-sensitive identity would miss.
    #[test]
    fn identical_id_less_retries_queue_once() {
        let handle = new_initializing_handle("parent-retry-storm");
        let child = handle.child_session_id.clone();

        assert_eq!(
            queue_initializing_child_input(&child, None, reply_shaped_steer("run it again", 100)),
            InitialInputDisposition::Queued
        );
        for retry in 1..=4 {
            assert_eq!(
                queue_initializing_child_input(
                    &child,
                    None,
                    reply_shaped_steer("run it again", 100 + retry),
                ),
                InitialInputDisposition::Duplicate,
                "retry {retry} is the same accepted turn"
            );
        }

        let queued = mark_initial_runtime_ready(&child);
        assert_eq!(queued.len(), 1, "one user turn, one queued input");
        assert_eq!(queued[0].as_concat_text(), "run it again");
    }

    /// The other direction, so the fix above cannot quietly become "collapse
    /// everything": genuinely different steering still queues separately, and so
    /// does the same text under two distinct client turn ids.
    #[test]
    fn distinct_pre_start_steering_still_queues_separately() {
        let handle = new_initializing_handle("parent-distinct-steers");
        let child = handle.child_session_id.clone();

        assert_eq!(
            queue_initializing_child_input(&child, None, reply_shaped_steer("first point", 10)),
            InitialInputDisposition::Queued
        );
        assert_eq!(
            queue_initializing_child_input(&child, None, reply_shaped_steer("second point", 10)),
            InitialInputDisposition::Queued
        );
        assert_eq!(
            queue_initializing_child_input(
                &child,
                Some("turn-a".into()),
                reply_shaped_steer("same words", 10),
            ),
            InitialInputDisposition::Queued
        );
        assert_eq!(
            queue_initializing_child_input(
                &child,
                Some("turn-b".into()),
                reply_shaped_steer("same words", 10),
            ),
            InitialInputDisposition::Queued,
            "two client turn ids are two turns, whatever they say"
        );

        let queued = mark_initial_runtime_ready(&child);
        assert_eq!(queued.len(), 4);
        // Multiple pre-start inputs preserve their provenance — that is what
        // makes the delegated result report human intervention.
        assert!(queued.iter().all(|message| message
            .metadata
            .provenance
            .as_ref()
            .is_some_and(|provenance| provenance.kind
                == crate::conversation::message::ProvenanceKind::UserDirect)));
        assert!(handle.initial_run_human_intervened());
    }

    /// Identity, stated once and shared with the transcript probe in
    /// `subagent_handler`. `created` is excluded deliberately; ids decide when
    /// both sides have one.
    #[test]
    fn accepted_input_identity_ignores_the_restamped_timestamp() {
        let base = reply_shaped_steer("hold on", 1);
        let restamped = reply_shaped_steer("hold on", 2);
        assert!(is_same_accepted_input(&base, &restamped));

        let different_text = reply_shaped_steer("carry on", 1);
        assert!(!is_same_accepted_input(&base, &different_text));

        let unstamped = Message::user().with_text("hold on");
        assert!(
            !is_same_accepted_input(&base, &unstamped),
            "provenance is part of identity"
        );

        let named = base.clone().with_id("turn-1");
        let renamed = base.clone().with_id("turn-2");
        assert!(
            !is_same_accepted_input(&named, &renamed),
            "two client-supplied ids settle identity outright"
        );
        assert!(
            is_same_accepted_input(&named, &base),
            "one side unnamed falls back to the content tuple"
        );
    }

    /// The queue is bounded. Reaching the cap means a client is generating
    /// genuinely distinct messages in a loop, since retries of the same text no
    /// longer grow it at all.
    #[test]
    fn the_pre_start_queue_is_bounded() {
        let handle = new_initializing_handle("parent-queue-cap");
        let child = handle.child_session_id.clone();

        for index in 0..MAX_PENDING_INITIAL_INPUTS {
            assert_eq!(
                queue_initializing_child_input(
                    &child,
                    None,
                    reply_shaped_steer(&format!("steer {index}"), 1),
                ),
                InitialInputDisposition::Queued
            );
        }
        assert_eq!(
            queue_initializing_child_input(&child, None, reply_shaped_steer("one too many", 1)),
            InitialInputDisposition::Duplicate,
            "past the cap nothing more is queued"
        );
        assert_eq!(
            mark_initial_runtime_ready(&child).len(),
            MAX_PENDING_INITIAL_INPUTS
        );
    }

    /// OD-5. `complete` is synchronous and holds no session manager, so the
    /// drain it used to perform could only end in dropping the messages — and it
    /// threw away the run's real result on the way. It must destroy neither.
    #[test]
    fn completion_never_destroys_unsettled_steering() {
        let handle = new_initializing_handle("parent-unsettled-completion");
        let child = handle.child_session_id.clone();
        assert_eq!(
            queue_initializing_child_input(
                &child,
                Some("never-settled".into()),
                Message::user().with_text("this must survive completion"),
            ),
            InitialInputDisposition::Queued
        );

        handle.complete(done("the child did real work"));

        let result = handle.result().expect("the run still reports a result");
        assert!(
            result.summary.starts_with("the child did real work"),
            "the real summary is kept, not replaced: {}",
            result.summary
        );
        assert!(
            result.summary.contains("was not settled"),
            "and the anomaly is still reported: {}",
            result.summary
        );
        assert!(result.human_intervened);

        let recovered = handle.finish_initial_run_and_take_pending();
        assert_eq!(
            recovered.len(),
            1,
            "completion left the accepted steering recoverable"
        );
        assert_eq!(
            recovered[0].as_concat_text(),
            "this must survive completion"
        );

        // Admission is still closed, which is the half of the old behaviour that
        // was correct.
        assert_eq!(
            queue_initializing_child_input(
                &child,
                Some("too-late".into()),
                Message::user().with_text("too late"),
            ),
            InitialInputDisposition::NotInitializing
        );
    }

    #[test]
    fn completion_leaves_a_settled_result_untouched() {
        let handle = new_initializing_handle("parent-settled-completion");
        handle.complete(done("nothing was queued"));
        assert_eq!(handle.result().unwrap().summary, "nothing was queued");
    }

    #[test]
    fn completion_atomically_claims_accepted_initial_input_once() {
        let handle = new_initializing_handle("parent-initial-completion");
        let child = handle.child_session_id.clone();
        let steer = Message::user().with_text("preserve this accepted steering");

        assert_eq!(
            queue_initializing_child_input(&child, Some("client-turn-final".into()), steer),
            InitialInputDisposition::Queued
        );
        let claimed = handle.finish_initial_run_and_take_pending();
        assert_eq!(claimed.len(), 1);
        assert_eq!(
            claimed[0].as_concat_text(),
            "preserve this accepted steering"
        );
        assert!(handle.finish_initial_run_and_take_pending().is_empty());
        assert_eq!(
            queue_initializing_child_input(
                &child,
                Some("client-turn-too-late".into()),
                Message::user().with_text("too late"),
            ),
            InitialInputDisposition::NotInitializing
        );

        handle.complete(done("failed before runtime setup"));
        assert_eq!(
            handle.result().unwrap().summary,
            "failed before runtime setup"
        );
    }

    #[test]
    fn runtime_handoff_keeps_accepted_input_recoverable_until_durable_ack() {
        let handle = new_initializing_handle("parent-runtime-handoff");
        let child = handle.child_session_id.clone();
        let steer = Message::user().with_text("survive a post-ready setup failure");

        assert_eq!(
            queue_initializing_child_input(&child, Some("handoff-turn".into()), steer),
            InitialInputDisposition::Queued
        );
        let handed_off = mark_initial_runtime_ready(&child);
        assert_eq!(handed_off.len(), 1);
        assert_eq!(
            handed_off[0].as_concat_text(),
            "survive a post-ready setup failure"
        );

        let recovered = handle.finish_initial_run_and_take_pending();
        assert_eq!(recovered.len(), 1);
        assert_eq!(
            recovered[0].as_concat_text(),
            handed_off[0].as_concat_text()
        );
    }

    #[test]
    fn durable_runtime_handoff_acknowledgement_clears_recovery_copy() {
        let handle = new_initializing_handle("parent-runtime-ack");
        let child = handle.child_session_id.clone();
        let steer = Message::user().with_text("already durable");

        assert_eq!(
            queue_initializing_child_input(&child, Some("acked-turn".into()), steer),
            InitialInputDisposition::Queued
        );
        assert_eq!(mark_initial_runtime_ready(&child).len(), 1);
        settle_initial_runtime_inputs(&child);
        assert!(handle.finish_initial_run_and_take_pending().is_empty());

        handle.complete(done("finished after durable intervention"));
        assert!(handle.result().unwrap().human_intervened);
    }

    #[test]
    fn queued_child_remains_cancellable_before_it_has_a_daemon_turn() {
        let handle = new_initializing_handle("parent-queued-cancel");
        assert!(is_child_initializing(&handle.child_session_id));
        assert!(cancel_initializing_child(&handle.child_session_id));
        assert!(handle.is_cancelled());
    }

    #[tokio::test]
    async fn wait_returns_none_while_running_and_the_result_once_done() {
        let handle = new_handle("parent-wait");

        // Still running -> a short wait times out rather than failing.
        assert!(handle.wait(Duration::from_millis(20)).await.is_none());

        let waiter = handle.clone();
        let task = tokio::spawn(async move { waiter.wait(Duration::from_secs(5)).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        handle.complete(done("finished in the background"));

        let result = task.await.unwrap().expect("wait resolves once complete");
        assert_eq!(result.summary, "finished in the background");
    }

    #[tokio::test]
    async fn wait_on_an_already_finished_handle_returns_immediately() {
        let handle = new_handle("parent-wait-done");
        handle.complete(done("already done"));
        let result = handle
            .wait(Duration::from_millis(0))
            .await
            .expect("finished handle resolves without waiting");
        assert_eq!(result.summary, "already done");
    }

    #[tokio::test]
    async fn unbounded_supervision_wait_has_only_the_callers_cancellation_boundary() {
        let handle = new_handle("parent-unbounded-wait");
        let waiter = handle.clone();
        let task = tokio::spawn(async move { waiter.wait_until_complete().await });
        tokio::task::yield_now().await;
        assert!(!task.is_finished());

        handle.complete(done("finished without an internal deadline"));
        let result = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("test safety deadline")
            .unwrap();
        assert_eq!(result.summary, "finished without an internal deadline");
    }

    #[tokio::test]
    async fn lifecycle_wait_wakes_for_continuation_and_terminal_transitions() {
        let handle = new_handle("parent-lifecycle-events");
        begin_child_turn(&handle.child_session_id);
        let initial_version = handle.state_version();

        let mark = mark_continuation_pending(&handle.child_session_id);
        let after_reservation = handle.wait_for_state_change(initial_version).await;
        assert_ne!(after_reservation, initial_version);
        mark.rollback();
        let after_rollback = handle.wait_for_state_change(after_reservation).await;
        assert_ne!(after_rollback, after_reservation);

        record_child_turn_terminal(&handle.child_session_id, done("terminal result"));
        let after_terminal = handle.wait_for_state_change(after_rollback).await;
        assert_ne!(after_terminal, after_rollback);
        assert_eq!(
            handle.terminal_generation().unwrap().result.summary,
            "terminal result"
        );
    }

    #[test]
    fn finished_handles_are_pruned_but_running_ones_are_kept() {
        let parent = "parent-prune";
        let running = new_handle(parent);
        let mut finished_ids = Vec::new();
        for i in 0..(MAX_RETAINED_FINISHED + 5) {
            let handle = new_handle(parent);
            handle.complete(done(&format!("run {i}")));
            assert!(handle.mark_collected_if_generation(handle.child_turn_generation()));
            finished_ids.push(handle.id.clone());
        }
        // Registering one more triggers the prune of the excess finished ones.
        let newest = new_handle(parent);

        let listed = list_for_session(parent);
        let finished = listed.iter().filter(|h| !h.is_running()).count();
        assert_eq!(finished, MAX_RETAINED_FINISHED);
        // The running handles survived regardless of the cap.
        assert!(listed.iter().any(|h| h.id == running.id));
        assert!(listed.iter().any(|h| h.id == newest.id));
        // The *oldest* finished results are the ones that were dropped, and the
        // newest finished result is still collectable.
        assert!(get_for_session(parent, &finished_ids[0]).is_none());
        assert!(get_for_session(parent, finished_ids.last().unwrap()).is_some());
    }

    #[test]
    fn direct_followup_reattaches_a_pruned_child_generation_once() {
        let parent = "parent-pruned-direct-followup";
        let child = "child-pruned-direct-followup";
        let original =
            BackgroundSubagent::register(parent, child, "original child", CancellationToken::new());
        original.complete(done("original result"));
        assert!(original.mark_current_result_collected_if_generation(0));

        for index in 0..=MAX_RETAINED_FINISHED {
            let handle = BackgroundSubagent::register(
                parent,
                format!("replacement-child-{index}"),
                "retained child",
                CancellationToken::new(),
            );
            handle.complete(done("retained result"));
            assert!(handle.mark_current_result_collected_if_generation(0));
        }
        assert!(
            list_for_session(parent)
                .iter()
                .all(|handle| handle.child_session_id != child),
            "the setup did not prune the original collected handle"
        );

        let reattached = ensure_direct_followup_handle(parent, child, "continued child");
        assert_eq!(reattached.child_turn_generation(), 1);
        assert!(reattached.terminal_generation().is_none());
        assert!(!reattached.latest_generation_collected());
        assert!(with_live_delegated_supervision(parent, |live| live));
        assert!(admit_child_turn(child));
        assert_eq!(
            reattached.child_turn_generation(),
            1,
            "the runner advanced the already-admitted follow-up twice"
        );

        record_child_turn_terminal(child, done("direct follow-up result"));
        let terminal = reattached.terminal_generation().unwrap();
        assert_eq!(terminal.generation, 1);
        assert_eq!(terminal.result.summary, "direct follow-up result");
        assert!(reattached.mark_terminal_generation_collected_if_generation(1));
        assert!(!with_live_delegated_supervision(parent, |live| live));

        let same = ensure_direct_followup_handle(parent, child, "continued child");
        assert!(Arc::ptr_eq(&reattached, &same));
    }

    #[test]
    fn pruning_one_session_never_evicts_another_sessions_results() {
        let quiet = "parent-quiet";
        let busy = "parent-busy";
        let precious = new_handle(quiet);
        precious.complete(done("the result nobody collected yet"));

        for i in 0..(MAX_RETAINED_FINISHED + 10) {
            let handle = new_handle(busy);
            handle.complete(done(&format!("busy run {i}")));
            assert!(handle.mark_collected_if_generation(handle.child_turn_generation()));
        }

        let kept = get_for_session(quiet, &precious.id).expect("quiet session's result survives");
        assert_eq!(
            kept.result().unwrap().summary,
            "the result nobody collected yet"
        );
        // Pruning happens on registration, so the busy session settles at the cap
        // (plus at most the one handle that finished after the last register).
        assert!(list_for_session(busy).len() <= MAX_RETAINED_FINISHED + 1);
    }

    #[test]
    fn starting_a_successor_invalidates_collection_of_the_previous_generation() {
        let handle = new_handle("parent-collection-generation");
        begin_child_turn(&handle.child_session_id);
        handle.complete(done("initial result"));
        let initial_generation = handle.child_turn_generation();
        assert!(handle.mark_collected_if_generation(initial_generation));
        assert!(handle.latest_generation_collected());

        begin_child_turn(&handle.child_session_id);
        assert!(!handle.latest_generation_collected());
        assert!(
            !handle.mark_collected_if_generation(initial_generation),
            "a late collector for the initial turn cannot collect its successor"
        );
    }

    #[test]
    fn child_turn_admission_advances_once_before_terminal_settlement() {
        let handle = new_handle("parent-idempotent-admission");

        assert!(admit_child_turn(&handle.child_session_id));
        let initial_generation = handle.child_turn_generation();
        assert_eq!(initial_generation, 1);
        assert!(admit_child_turn(&handle.child_session_id));
        assert_eq!(
            handle.child_turn_generation(),
            initial_generation,
            "the Agent::reply fallback must not advance an already admitted turn"
        );

        record_child_turn_terminal(&handle.child_session_id, done("first terminal"));
        assert!(admit_child_turn(&handle.child_session_id));
        assert_eq!(handle.child_turn_generation(), initial_generation + 1);
        assert!(admit_child_turn(&handle.child_session_id));
        assert_eq!(handle.child_turn_generation(), initial_generation + 1);

        let pre_admission_failure = new_handle("parent-pre-admission-failure");
        pre_admission_failure.complete(done("generation zero terminal"));
        assert!(admit_child_turn(&pre_admission_failure.child_session_id));
        assert_eq!(pre_admission_failure.child_turn_generation(), 1);
        assert!(admit_child_turn(&pre_admission_failure.child_session_id));
        assert_eq!(
            pre_admission_failure.child_turn_generation(),
            1,
            "a retained generation-zero result still admits its successor only once"
        );
    }

    #[test]
    fn snapshot_lines_read_well_in_both_states() {
        let handle = new_handle("parent-lines");
        let running = handle.snapshot().to_line();
        assert!(running.contains("do the thing"));
        assert!(running.contains("running"));

        handle.complete(done("wrote the report\nand more detail"));
        let finished = handle.snapshot().to_line();
        assert!(finished.contains("completed"));
        assert!(finished.contains("wrote the report"));
        // Only the first line of the summary makes the list view.
        assert!(!finished.contains("and more detail"));
    }

    #[test]
    fn retained_result_is_invalidated_when_a_later_child_turn_begins() {
        let handle = new_handle("parent-generation");
        begin_child_turn(&handle.child_session_id);
        handle.complete(done("original run"));
        assert!(handle.result_is_current());

        begin_child_turn(&handle.child_session_id);
        assert!(!handle.result_is_current());
    }

    #[test]
    fn concurrent_second_child_turn_invalidates_the_running_handle() {
        let handle = new_handle("parent-concurrent-generation");
        begin_child_turn(&handle.child_session_id);
        begin_child_turn(&handle.child_session_id);
        assert!(!handle.result_is_current());
    }

    #[test]
    fn continuation_mark_invalidates_the_original_until_the_replacement_begins() {
        let handle = new_handle("parent-continuation");
        begin_child_turn(&handle.child_session_id);
        handle.complete(done("original run"));

        let mark = mark_continuation_pending(&handle.child_session_id);
        assert!(!mark.is_empty());
        assert!(handle.continuation_pending());
        assert!(!handle.result_is_current());

        begin_child_turn(&handle.child_session_id);
        assert!(!handle.continuation_pending());
        assert!(!handle.result_is_current());
    }

    #[test]
    fn initial_turn_start_after_admission_does_not_consume_the_replacement_lease() {
        let handle = new_handle("parent-fast-continuation");
        let mark = mark_continuation_pending_for_turn(
            &handle.child_session_id,
            Some("turn-original".to_string()),
        );
        mark.commit();

        begin_child_turn(&handle.child_session_id);
        assert!(handle.continuation_pending());

        handle.complete(done("original run"));
        begin_child_turn(&handle.child_session_id);
        assert!(!handle.continuation_pending());
        assert!(!handle.result_is_current());
    }

    #[test]
    fn failed_continuation_admission_restores_the_original_result() {
        let handle = new_handle("parent-continuation-rollback");
        begin_child_turn(&handle.child_session_id);
        handle.complete(done("original run"));

        let mark = mark_continuation_pending(&handle.child_session_id);
        mark.rollback();

        assert!(!handle.continuation_pending());
        assert!(handle.result_is_current());
    }

    #[test]
    fn explicit_close_abandons_a_continuation_without_a_successor_turn() {
        let handle = new_handle("parent-continuation-abandoned");
        begin_child_turn(&handle.child_session_id);
        handle.complete(done("original run"));

        let mark = mark_continuation_pending(&handle.child_session_id);
        assert!(!mark.is_empty());
        mark.commit();
        assert!(abandon_continuation(&handle.child_session_id));

        assert!(!handle.continuation_pending());
        assert!(handle.result_is_current());
    }

    #[test]
    fn one_dropped_concurrent_admission_cannot_rollback_another_committed_lease() {
        let handle = new_handle("parent-concurrent-continuations");
        begin_child_turn(&handle.child_session_id);
        handle.complete(done("original run"));

        let child_for_dropped = handle.child_session_id.clone();
        let child_for_success = handle.child_session_id.clone();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let dropped_barrier = barrier.clone();
        let dropped = std::thread::spawn(move || {
            dropped_barrier.wait();
            mark_continuation_pending_for_turn(
                &child_for_dropped,
                Some("turn-original".to_string()),
            )
        });
        let successful_barrier = barrier.clone();
        let successful = std::thread::spawn(move || {
            successful_barrier.wait();
            mark_continuation_pending_for_turn(
                &child_for_success,
                Some("turn-original".to_string()),
            )
        });
        barrier.wait();
        let dropped = dropped.join().unwrap();
        let successful = successful.join().unwrap();
        successful.commit();
        dropped.rollback();

        assert!(handle.continuation_pending());
        assert!(!handle.result_is_current());

        begin_child_turn(&handle.child_session_id);
        assert!(!handle.continuation_pending());
        assert!(!handle.result_is_current());
    }

    #[test]
    fn cloned_ticket_resolves_its_reservation_only_once() {
        let handle = new_handle("parent-cloned-continuation-ticket");
        begin_child_turn(&handle.child_session_id);
        handle.complete(done("original run"));

        let ticket = mark_continuation_pending(&handle.child_session_id);
        let clone = ticket.clone();
        ticket.rollback();
        clone.commit();

        assert!(!handle.continuation_pending());
        assert!(handle.result_is_current());
    }

    #[test]
    fn continuation_remembers_the_exact_superseded_turn_for_watchers() {
        let handle = new_handle("parent-canonical-continuation");
        begin_child_turn(&handle.child_session_id);
        handle.complete(done("original run"));

        let mark = mark_continuation_pending_for_turn(
            &handle.child_session_id,
            Some("turn-original".to_string()),
        );
        mark.commit();

        assert_eq!(
            handle.superseded_turn_id().as_deref(),
            Some("turn-original")
        );
    }

    #[test]
    fn abandoning_an_older_generation_cannot_clear_a_newer_continuation() {
        let handle = new_handle("parent-exact-continuation-abandonment");
        begin_child_turn(&handle.child_session_id);
        handle.complete(done("original run"));

        let first = mark_continuation_pending_for_turn(
            &handle.child_session_id,
            Some("turn-original".to_string()),
        );
        first.commit();
        let second = mark_continuation_pending_for_turn(
            &handle.child_session_id,
            Some("turn-successor".to_string()),
        );
        second.commit();

        assert!(!abandon_continuation_for_turn(
            &handle.child_session_id,
            "turn-original"
        ));
        assert!(handle.continuation_pending());
        assert!(abandon_continuation_for_turn(
            &handle.child_session_id,
            "turn-successor"
        ));
        assert!(!handle.continuation_pending());
    }

    #[test]
    fn stale_rollback_cannot_clear_a_later_continuation_generation() {
        let handle = new_handle("parent-continuation-generation");
        begin_child_turn(&handle.child_session_id);
        handle.complete(done("original run"));

        let first = mark_continuation_pending(&handle.child_session_id);
        begin_child_turn(&handle.child_session_id);
        let second = mark_continuation_pending(&handle.child_session_id);
        assert!(!second.is_empty());

        first.rollback();
        assert!(handle.continuation_pending());
        assert!(!handle.result_is_current());
    }

    #[test]
    fn parent_closing_refuses_continuation_without_mutating_the_handle() {
        let parent = "parent-closing-barrier";
        let handle = new_handle(parent);
        begin_child_turn(&handle.child_session_id);
        handle.complete(done("original result"));

        begin_parent_closing(parent);
        let refused = mark_continuation_pending(&handle.child_session_id);
        assert!(refused.is_empty());
        assert!(refused.refused_parent_closing());
        assert!(!handle.continuation_pending());
        assert!(handle.result_is_current());

        let late_child = format!("late-{}", handle.child_session_id);
        let late_handle = BackgroundSubagent::register(
            parent,
            &late_child,
            "registered after close",
            CancellationToken::new(),
        );
        begin_child_turn(&late_child);
        let late_refused = mark_continuation_pending(&late_child);
        assert!(late_refused.refused_parent_closing());
        assert!(!late_handle.continuation_pending());

        open_parent_continuation_admission(parent);
        let admitted = mark_continuation_pending(&handle.child_session_id);
        assert!(!admitted.is_empty());
        assert!(!admitted.refused_parent_closing());
        admitted.rollback();
    }

    #[test]
    fn terminal_collection_is_exact_to_the_generation_that_finished() {
        let handle = new_handle("parent-terminal-generation");
        begin_child_turn(&handle.child_session_id);
        let first_generation = handle.child_turn_generation();
        record_child_turn_terminal(&handle.child_session_id, done("first result"));

        begin_child_turn(&handle.child_session_id);
        let second_generation = handle.child_turn_generation();
        assert_ne!(first_generation, second_generation);
        assert!(
            !handle.mark_terminal_generation_collected_if_generation(first_generation),
            "a stale terminal cannot collect its successor"
        );

        record_child_turn_terminal(&handle.child_session_id, done("second result"));
        assert!(handle.mark_terminal_generation_collected_if_generation(second_generation));
        assert!(handle.latest_generation_collected());
    }

    // --- Stopping a stopped parent's subagents ---------------------------------

    /// A daemon stand-in that answers which children have a turn in flight,
    /// and records what was cancelled and abandoned.
    #[derive(Default)]
    struct FakeDaemon {
        active: Mutex<std::collections::HashSet<String>>,
        /// `cancel_turn` leaves the turn active, as a cancellation that has
        /// been requested but has not landed yet.
        cancellation_settles_later: bool,
        /// `cancel_turn` finds nothing: the turn ended on its own between
        /// `is_turn_active` and the cancel.
        turn_ends_before_cancel: bool,
        cancelled: Mutex<Vec<String>>,
        abandoned: Mutex<Vec<String>>,
    }

    impl FakeDaemon {
        fn with_active(children: &[&str]) -> Self {
            let daemon = Self::default();
            daemon
                .active
                .lock()
                .unwrap()
                .extend(children.iter().map(|child| child.to_string()));
            daemon
        }
        fn cancelled(&self) -> Vec<String> {
            self.cancelled.lock().unwrap().clone()
        }
        fn abandoned(&self) -> Vec<String> {
            self.abandoned.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl WorkspaceServices for FakeDaemon {
        fn gui_attached(&self) -> bool {
            false
        }
        fn layout_snapshot(&self) -> Option<serde_json::Value> {
            None
        }
        fn is_turn_active(&self, session_id: &str) -> bool {
            self.active.lock().unwrap().contains(session_id)
        }
        fn cancel_turn(&self, session_id: &str) -> Option<String> {
            if self.turn_ends_before_cancel {
                self.active.lock().unwrap().remove(session_id);
                return None;
            }
            let was_active = if self.cancellation_settles_later {
                self.active.lock().unwrap().contains(session_id)
            } else {
                self.active.lock().unwrap().remove(session_id)
            };
            if !was_active {
                return None;
            }
            self.cancelled.lock().unwrap().push(session_id.to_string());
            Some(format!("turn-of-{session_id}"))
        }
        fn abandon_pending_continuations(&self, session_id: &str) -> bool {
            self.abandoned.lock().unwrap().push(session_id.to_string());
            abandon_continuation(session_id)
        }
        fn begin_turn(
            &self,
            _session_id: &str,
            _cancel: CancellationToken,
        ) -> Result<Box<dyn crate::workspace_services::WorkspaceTurnLease>, String> {
            Err("not used here".into())
        }
        async fn stop_agent(&self, _session_id: &str) -> Result<(), String> {
            Ok(())
        }
        async fn start_detached_turn(
            &self,
            _session_id: &str,
            _message: Message,
        ) -> Result<String, String> {
            Err("not used here".into())
        }
        async fn start_session(
            &self,
            _working_dir: std::path::PathBuf,
            _extensions: Option<Vec<String>>,
            _knowledge_bases: Vec<String>,
            _primary: crate::workspace_services::KbPrimaryChoice,
        ) -> Result<String, String> {
            Err("not used here".into())
        }
        fn set_knowledge_bases(
            &self,
            _session_id: &str,
            _kbs: &[String],
            _primary: crate::workspace_services::KbPrimaryChoice,
        ) -> Result<crate::workspace_services::KbSelectionView, String> {
            Ok(Default::default())
        }
        fn knowledge_selection(
            &self,
            _session_id: &str,
        ) -> crate::workspace_services::KbSelectionView {
            Default::default()
        }
        async fn gui_command(
            &self,
            _frame: serde_json::Value,
            _wait_result: bool,
        ) -> Result<serde_json::Value, String> {
            Err("no GUI attached".into())
        }
    }

    fn child_of(parent: &str, child: &str, title: &str) -> Arc<BackgroundSubagent> {
        BackgroundSubagent::register(parent, child, title, CancellationToken::new())
    }

    fn queued_child_of(parent: &str, child: &str, title: &str) -> Arc<BackgroundSubagent> {
        BackgroundSubagent::register_initializing(parent, child, title, CancellationToken::new())
    }

    fn monitor_cause(name: &str) -> ChildStopCause {
        ChildStopCause::ParentMonitor {
            monitor: name.to_string(),
        }
    }

    /// The parent stopped, so its children stop: a running delegated run and
    /// one still queued for a slot are cancelled through their own tokens, a
    /// later turn the daemon runs is cancelled through the daemon, the parent
    /// refuses Stop-and-Send from then on, and each handle says why.
    #[test]
    #[serial_test::serial(workspace_services)]
    fn stop_delegated_children_cancels_running_and_queued_children_and_closes_the_parent() {
        let parent = "p06-stop-parent";
        let running = child_of(parent, "p06-stop-running", "count to sixty");
        let queued = queued_child_of(parent, "p06-stop-queued", "wait for a slot");
        // Its delegated run finished; the person continued it from its tab,
        // and that later turn is running in the daemon.
        let later = child_of(parent, "p06-stop-later", "continued by the person");
        later.complete(done("the delegated run"));
        let daemon = Arc::new(FakeDaemon::with_active(&[
            "p06-stop-running",
            "p06-stop-later",
        ]));
        crate::workspace_services::set_for_tests(Some(daemon.clone()));

        let stopped = stop_delegated_children(parent, monitor_cause("fanout"));
        crate::workspace_services::clear_test_override();

        assert_eq!(
            stopped,
            vec![
                StoppedChild {
                    child_session_id: "p06-stop-running".into(),
                    title: "count to sixty".into(),
                },
                StoppedChild {
                    child_session_id: "p06-stop-queued".into(),
                    title: "wait for a slot".into(),
                },
                StoppedChild {
                    child_session_id: "p06-stop-later".into(),
                    title: "continued by the person".into(),
                },
            ]
        );
        assert!(
            running.is_cancelled(),
            "a running delegated run is cancelled"
        );
        assert!(queued.is_cancelled(), "a queued delegated run is cancelled");
        assert!(
            !later.is_cancelled(),
            "a finished run's token is not the later turn's"
        );
        assert_eq!(
            daemon.cancelled(),
            vec!["p06-stop-later".to_string()],
            "only the later turn goes through the daemon: the delegated runs' own tokens are \
             the ones their daemon turns hold"
        );
        assert_eq!(
            daemon.abandoned(),
            vec![
                "p06-stop-running".to_string(),
                "p06-stop-queued".to_string(),
                "p06-stop-later".to_string(),
            ],
            "every stopped child's pending continuation is abandoned through the daemon"
        );
        for handle in [&running, &queued, &later] {
            assert_eq!(handle.stop_cause(), Some(monitor_cause("fanout")));
        }

        // The parent is closing: a Stop-and-Send on any of its children is
        // refused, so none can be restarted behind the stop.
        for handle in [&running, &queued, &later] {
            let refused = mark_continuation_pending(&handle.child_session_id);
            assert!(refused.refused_parent_closing());
            assert!(!handle.continuation_pending());
        }
    }

    /// A finished child has nothing to stop, and another chat's children are
    /// not this chat's to stop.
    #[test]
    fn it_leaves_finished_children_and_other_parents_children_alone() {
        let parent = "p06-alone-parent";
        let finished = child_of(parent, "p06-alone-finished", "already done");
        finished.complete(done("finished before the stop"));
        let other = child_of(
            "p06-alone-other-parent",
            "p06-alone-other",
            "someone else's",
        );
        let daemon = FakeDaemon::default();

        let stopped =
            stop_children_with(parent, None, &ChildStopCause::ParentStopped, Some(&daemon));

        assert!(stopped.is_empty(), "nothing was in flight: {stopped:?}");
        assert_eq!(finished.stop_cause(), None);
        assert_eq!(
            finished.result().unwrap().summary,
            "finished before the stop",
            "a finished result is not touched"
        );
        assert!(daemon.cancelled().is_empty());
        assert!(daemon.abandoned().is_empty());

        assert!(!other.is_cancelled());
        assert_eq!(other.stop_cause(), None);
        let admitted = mark_continuation_pending(&other.child_session_id);
        assert!(
            !admitted.refused_parent_closing(),
            "another chat stays open to continuations"
        );
        admitted.rollback();
    }

    /// The subagent whose own monitor stopped it is not stopped a second time
    /// by the stop it set off; its siblings are, and say whose monitor did it.
    #[test]
    #[serial_test::serial(workspace_services)]
    fn the_except_variant_spares_the_stopping_child() {
        let parent = "p06-except-parent";
        let stopper = child_of(parent, "p06-except-stopper", "deletes a file");
        let first = child_of(parent, "p06-except-first", "counts");
        let second = queued_child_of(parent, "p06-except-second", "also counts");
        let cause = ChildStopCause::SiblingMonitor {
            monitor: "rm".into(),
            sibling: "deletes a file".into(),
        };
        crate::workspace_services::set_for_tests(None);

        let stopped = stop_delegated_children_except(parent, "p06-except-stopper", cause.clone());
        crate::workspace_services::clear_test_override();

        let ids: Vec<&str> = stopped
            .iter()
            .map(|child| child.child_session_id.as_str())
            .collect();
        assert_eq!(ids, vec!["p06-except-first", "p06-except-second"]);
        assert!(first.is_cancelled() && second.is_cancelled());
        assert_eq!(first.stop_cause(), Some(cause.clone()));
        assert_eq!(second.stop_cause(), Some(cause));
        assert!(
            !stopper.is_cancelled(),
            "the stopping child ends through its own monitor's stop"
        );
        assert_eq!(stopper.stop_cause(), None);
        // The parent closes with it: the escalation stops the whole run.
        assert!(mark_continuation_pending(&stopper.child_session_id).refused_parent_closing());
    }

    /// The first reason recorded is the one the child reports, a run that was
    /// already stopping keeps its own (none), and a later turn starts clean.
    #[test]
    fn the_first_cause_wins() {
        let parent = "p06-first-parent";
        let running = child_of(parent, "p06-first-running", "running");
        let first = monitor_cause("budget");
        assert_eq!(
            stop_children_with(parent, None, &first, None).len(),
            1,
            "the running child is stopped"
        );
        let again = stop_children_with(parent, None, &ChildStopCause::ParentStopped, None);
        assert_eq!(
            again.len(),
            1,
            "a child still unwinding is still reported, so the parent waits for it"
        );
        assert_eq!(running.stop_cause(), Some(first.clone()));

        // A later turn the daemon runs, whose cancellation has not landed yet,
        // is stopped twice: the second stop's reason does not replace the
        // first.
        let later = child_of(parent, "p06-first-later", "continued");
        later.complete(done("delegated run"));
        let daemon = FakeDaemon {
            cancellation_settles_later: true,
            ..FakeDaemon::with_active(&["p06-first-later"])
        };
        let sibling = ChildStopCause::SiblingMonitor {
            monitor: "rm".into(),
            sibling: "deletes a file".into(),
        };
        stop_children_with(parent, None, &sibling, Some(&daemon));
        stop_children_with(parent, None, &ChildStopCause::ParentStopped, Some(&daemon));
        assert_eq!(later.stop_cause(), Some(sibling));
        assert_eq!(daemon.cancelled().len(), 2, "both stops asked the daemon");

        // The next turn is new work; the old reason is not its reason.
        begin_child_turn(&later.child_session_id);
        assert_eq!(later.stop_cause(), None);

        // A run something else had already asked to stop (another agent's
        // workspace_close) keeps no reason of ours.
        let closed = child_of("p06-first-closed-parent", "p06-first-closed", "closed");
        closed.cancel();
        let reported = stop_children_with(
            "p06-first-closed-parent",
            None,
            &ChildStopCause::ParentStopped,
            None,
        );
        assert_eq!(reported.len(), 1);
        assert_eq!(closed.stop_cause(), None);
    }

    /// A later turn that ended on its own between the check and the cancel was
    /// not stopped: it is not reported, and no reason is left on its handle.
    #[test]
    fn a_stop_that_finds_the_turn_already_over_records_nothing() {
        let parent = "p06-raced-parent";
        let later = child_of(parent, "p06-raced-later", "just finished");
        later.complete(done("delegated run"));
        let daemon = FakeDaemon {
            turn_ends_before_cancel: true,
            ..FakeDaemon::with_active(&["p06-raced-later"])
        };

        let stopped =
            stop_children_with(parent, None, &ChildStopCause::ParentStopped, Some(&daemon));

        assert!(stopped.is_empty(), "{stopped:?}");
        assert_eq!(later.stop_cause(), None);
    }

    /// The per-child step `workspace_close` takes: a delegated run through its
    /// handle, a later turn through the daemon, and nothing without one.
    #[test]
    fn stop_child_turn_stops_the_work_in_flight() {
        let running = child_of("p06-step-parent", "p06-step-running", "running");
        let daemon = FakeDaemon::with_active(&["p06-step-running", "p06-step-later"]);
        assert_eq!(
            stop_child_turn(Some(&*running), "p06-step-running", Some(&daemon)),
            ChildTurnStop::DelegatedRun
        );
        assert!(running.is_cancelled());
        assert!(daemon.cancelled().is_empty());

        assert_eq!(
            stop_child_turn(None, "p06-step-later", Some(&daemon)),
            ChildTurnStop::Turn("turn-of-p06-step-later".into())
        );
        assert_eq!(
            stop_child_turn(None, "p06-step-later", Some(&daemon)),
            ChildTurnStop::Idle
        );
        assert_eq!(
            daemon.abandoned(),
            vec![
                "p06-step-running".to_string(),
                "p06-step-later".to_string(),
                "p06-step-later".to_string(),
            ]
        );
        assert_eq!(
            stop_child_turn(None, "p06-step-later", None),
            ChildTurnStop::NoDaemon
        );

        // Without a daemon a waiting Stop-and-Send continuation is still
        // abandoned, in process.
        let resumed = child_of("p06-step-parent-2", "p06-step-resumed", "resumed");
        begin_child_turn(&resumed.child_session_id);
        resumed.complete(done("original"));
        mark_continuation_pending(&resumed.child_session_id).commit();
        assert!(resumed.continuation_pending());
        assert_eq!(
            stop_child_turn(Some(&*resumed), "p06-step-resumed", None),
            ChildTurnStop::NoDaemon
        );
        assert!(!resumed.continuation_pending());
    }

    #[test]
    fn each_cause_says_why_in_one_line() {
        assert_eq!(
            monitor_cause("fan\nout").sentence(),
            "Stopped because monitor 'fan out' stopped the chat that started this subagent."
        );
        assert_eq!(
            ChildStopCause::ParentStopped.sentence(),
            "Stopped because the person stopped the chat that started this subagent."
        );
        assert_eq!(
            ChildStopCause::SiblingMonitor {
                monitor: "rm".into(),
                sibling: "delete\tthe file".into(),
            }
            .sentence(),
            "Stopped because monitor 'rm', set by the person to watch every subagent of that \
             chat, stopped subagent 'delete the file'."
        );
    }
}
