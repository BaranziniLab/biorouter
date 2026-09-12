//! BR-71 §4.2: THE turn runner.
//!
//! Design §4.2 asks for a turn that can run "server-side with no attached HTTP
//! response", and for `/reply` to become "detached turn + a subscription".
//! This module is the first half; `routes/reply.rs` (Task 8) is the second.
//! Everything a turn owns lives here — the per-session turn lock, the
//! interactive-turn guard, `get_agent`, `agent.reply(...)`, consuming the
//! `AgentEvent` stream, tool telemetry, terminal-reason classification, the
//! best-effort session rename, session-completion metrics, and the
//! authoritative end-of-turn token read (BR-52). Everything a *request* owns
//! (SSE framing, delta coalescing, heartbeats, the JoinError envelope) stays in
//! the handler.
//!
//! Every event the turn produces is published to the session bus, so a client
//! that started the turn and an observer that did not see byte-identical
//! frames. That equality is the design's actual goal, and it is now structural
//! rather than maintained by hand across two loops.
//!
//! **With one exception, and Task 7 should know about it before it starts.**
//! The bus carries no per-frame token state. `/reply` seeds a running
//! `TokenState` from the session (`TokenState::from(&session)`), folds every
//! `AgentEvent::TokenUsage` into it, and attaches a clone to every `Message`,
//! `UpdateConversation` and `Finish` frame it emits — `MessageEvent` has no
//! `TokenUsage` variant at all, so that fold is the only way the value reaches
//! a client. This runner does not fold: it republishes `TokenUsage` raw, like
//! every other `AgentEvent`, because reordering or rewriting frames here is
//! forbidden for the reasons in [`run_turn`]'s stated non-goal 2. A bus
//! consumer therefore has to do the accumulation itself, *and* seed it from the
//! store when it attaches mid-turn. For `token_state` specifically the
//! equality above is still maintained by hand — just in one mapper instead of
//! two loops.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use biorouter::agents::{AgentEvent, SessionConfig};
use biorouter::conversation::message::Message;
use biorouter::conversation::Conversation;
use biorouter::session_events::{self, SessionBusEvent};
use futures::{FutureExt, StreamExt};
use tokio_util::sync::CancellationToken;

use crate::routes::reply::{get_token_state, track_tool_telemetry, TurnErrorScope};
use crate::state::AppState;

#[derive(Debug, thiserror::Error)]
pub enum TurnStartError {
    #[error("a turn is already in flight for this session (running turn {running_turn_id})")]
    TurnInFlight {
        running_turn_id: String,
        /// BR-62: true when the caller re-sent the SAME idempotency key — "your
        /// turn is still running", not "someone else is in the way".
        duplicate: bool,
    },
}

/// What kind of turn this is, which is also its `session_type` telemetry
/// dimension.
///
/// An enum rather than the `&'static str` an earlier revision carried. That
/// string is written verbatim into `session_type` on three counters, and under
/// `#[derive(Default)]` its default is `""` — so one forgotten field in Task 8's
/// `TurnExtras` literal would start a `session_completions{session_type=""}`
/// series while draining the `session_type="app"` one it was meant to preserve,
/// silently and at exactly the moment the whole hot path moves onto this runner.
/// A type with no meaningless value cannot express that mistake.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TurnKind {
    /// A turn driven by an interactive `/reply` request. The default,
    /// deliberately: it is what the counters already report, so a caller that
    /// omits the field lands on the existing series rather than inventing one.
    #[default]
    Interactive,
    /// A turn injected by BR-71 workspace control (`workspace_send_prompt`).
    Workspace,
}

impl TurnKind {
    /// The `session_type` label. `"app"` is the exact string `/reply` has always
    /// reported; changing it would split every existing dashboard's series.
    pub(crate) fn session_type(self) -> &'static str {
        match self {
            TurnKind::Interactive => "app",
            TurnKind::Workspace => "workspace",
        }
    }
}

/// Everything a turn needs that is not the session id or the user's message.
/// A struct rather than eight positional arguments because `/reply` supplies
/// four of these and an injected turn supplies none.
#[derive(Debug, Default)]
pub struct TurnExtras {
    /// BR-62 turn idempotency key (the client's `turn_id`). `None` for injected
    /// turns: two keyless turns are two turns, which is correct there.
    pub idempotency_key: Option<String>,
    /// The conversation this turn should START from — **already stored**.
    ///
    /// `Option<Conversation>`, NOT `Option<Vec<Message>>`. It is a *seed*, not a
    /// write to perform, and the type says so: it is the `Ok` value of
    /// `apply_client_writeback` (`reply.rs`), i.e. what the store holds
    /// after `/reply` has already validated and applied the client's copy.
    /// `None` means "read the session" — every caller except `/reply` passes
    /// `None`, because none of them has a client prefix.
    ///
    /// ⚠ **The whole-history rewrite does NOT move into `run_turn`.** An earlier
    /// revision of this plan said it did, quoting a `replace_conversation` call
    /// in the `/reply` handler. That call no longer exists: `306552fd
    /// fix(server): refuse a stale conversation_so_far instead of storing it`
    /// (#51 W5) replaced it with `apply_client_writeback`, a **pre-spawn
    /// precondition with an HTTP status code** — it reads a paired snapshot
    /// (`snapshot_for_rewrite`), computes `unacknowledged_stored_ids`, returns
    /// 409 `conversation_out_of_date` if the client's copy would delete a stored
    /// row, and otherwise writes through `replace_conversation_preserving_tail`,
    /// never through `replace_conversation`.
    ///
    /// A detached runner structurally cannot express that: it has no status code
    /// to refuse with, and `SessionManager::replace_conversation` is now
    /// documented as **the NAMED EXCEPTION**, correct only for a caller that
    /// owns the whole history — `/clear` and the import/copy/diverge paths.
    /// Calling it here, on a LIVE session, from a detached task, reintroduces
    /// exactly the tail-destroying bug #51 shipped a fix for. So the write stays
    /// in `/reply` (Task 8), above the range Task 8 replaces, and hands its
    /// result here as a seed.
    pub conversation_so_far: Option<Conversation>,
    /// `Option<ReasoningEffort>`, NOT `Option<String>`. It is copied verbatim
    /// from `ChatRequest.reasoning_effort` into `SessionConfig.reasoning_effort`,
    /// and both are `Option<biorouter::agents::ReasoningEffort>` — a fieldless
    /// enum (`Quick | Normal | Deep`) that derives `Clone, Copy`. Declaring it
    /// as a `String` is an E0308 in BOTH directions: at the `SessionConfig`
    /// literal below and at Task 8's `TurnExtras` construction.
    pub reasoning_effort: Option<biorouter::agents::ReasoningEffort>,
    /// Register in the `active_work` registry (the issue's binding table asks
    /// for this on workspace-spawned work). `/reply` passes `false` — an
    /// interactive turn is already visible as a turn.
    pub register_active_work: bool,
    /// What kind of turn this is; supplies the `session_type` telemetry label.
    pub kind: TurnKind,
}

pub struct TurnRequest {
    pub session_id: String,
    pub user_message: Message,
    pub extras: TurnExtras,
}

impl TurnRequest {
    pub fn new(session_id: String, user_message: Message) -> Self {
        Self {
            session_id,
            user_message,
            extras: TurnExtras {
                register_active_work: true,
                kind: TurnKind::Workspace,
                ..TurnExtras::default()
            },
        }
    }

    pub fn with_idempotency_key(mut self, key: Option<String>) -> Self {
        self.extras.idempotency_key = key;
        self
    }

    pub fn with_extras(mut self, extras: TurnExtras) -> Self {
        self.extras = extras;
        self
    }
}

/// A turn that has been accepted and is now running on its own task.
#[derive(Debug)]
pub struct StartedTurn {
    /// The server-assigned turn id, also carried by this turn's
    /// `SessionBusEvent::TurnStarted` so a consumer can correlate lifecycles.
    pub turn_id: String,
    /// The session's event stream, subscribed **before** the turn task was
    /// spawned — so it cannot have missed a frame.
    ///
    /// Handing the subscription back rather than letting the caller open its
    /// own is what makes the lost-event race unrepresentable; see
    /// [`start_turn`]. A caller that genuinely does not want the events just
    /// drops it, which reclaims the ring exactly as any other observer's exit
    /// would.
    pub events: session_events::Subscription,
}

/// Acquire the session's turn lock, subscribe, and spawn the turn. Returns
/// immediately with the turn's id and a subscription carrying every event it
/// will produce. The user message is stamped/persisted by the agent's own reply
/// path — this function persists nothing itself.
///
/// **The subscription is opened here, not by the caller, and that is
/// load-bearing.** `session_events::publish` is a pure lookup and a no-op when
/// the session has no ring; `subscribe` is the only thing that creates one. The
/// obvious caller shape — take the id back, then subscribe — therefore loses
/// `TurnStarted`, and on a turn that fails in microseconds loses the terminal
/// too, after which the caller blocks forever on a `recv()` for events that
/// were dropped on the floor. The daemon runs a multi-thread runtime, so the
/// spawned task really can get there first. Returning the subscription removes
/// the window instead of documenting it.
///
/// Callers that spawn [`run_turn`] themselves must subscribe before spawning,
/// not after. Panic supervision is part of `run_turn` itself.
pub async fn start_turn(
    state: Arc<AppState>,
    request: TurnRequest,
) -> Result<StartedTurn, TurnStartError> {
    let cancel_token = CancellationToken::new();
    let turn_guard = state
        .try_begin_turn_idempotent(
            &request.session_id,
            cancel_token.clone(),
            request.extras.idempotency_key.clone(),
        )
        .map_err(|conflict| TurnStartError::TurnInFlight {
            running_turn_id: conflict.running_turn_id,
            duplicate: conflict.duplicate,
        })?;
    let turn_id = turn_guard.turn_id().to_string();

    // Before the spawn, never after it.
    let events = session_events::subscribe(&request.session_id);

    tokio::spawn(run_turn(state, request, turn_guard, cancel_token));
    Ok(StartedTurn { turn_id, events })
}

const TURN_OUTCOME_NONE: u8 = 0;
const TURN_OUTCOME_STOP: u8 = 1;
const TURN_OUTCOME_CANCELLED: u8 = 2;
const TURN_TERMINAL_PUBLISHED: u8 = 3;

/// Run a turn and publish a terminal event on its behalf if it dies without one.
///
/// `run_turn_body` publishes `TurnStarted` before any fallible work. Let a panic
/// escape its task and a turn that panics anywhere afterwards publishes a start
/// and then nothing,
/// forever — "one terminal event per turn, always" becomes zero, and every
/// observer (Task 7's watcher, Task 14's `wait:"final_message"`) blocks on a
/// frame that never comes.
///
/// The turn guard is deliberately owned outside the future being caught. If the
/// guard unwound with `turn`, another turn could acquire the session and publish
/// `TurnStarted` before this function published the old turn's `TurnError`.
/// Terminal events do not carry a turn id, so that interleaving is
/// indistinguishable from the new turn failing.
///
/// Once an exact child outcome exists, a later teardown panic must preserve it:
/// the supervisor supplies only the missing clean finish frame in that case.
async fn supervise_turn<F>(
    session_id: String,
    _turn_guard: crate::state::TurnGuard,
    outcome: Arc<AtomicU8>,
    turn: F,
) where
    F: std::future::Future<Output = ()> + Send,
{
    if std::panic::AssertUnwindSafe(turn)
        .catch_unwind()
        .await
        .is_err()
    {
        tracing::error!("turn: task terminated unexpectedly");
        let recorded_outcome = outcome.load(Ordering::Acquire);
        match recorded_outcome {
            TURN_OUTCOME_STOP | TURN_OUTCOME_CANCELLED => {
                session_events::publish(
                    &session_id,
                    SessionBusEvent::TurnFinished {
                        reason: if recorded_outcome == TURN_OUTCOME_CANCELLED {
                            "cancelled".into()
                        } else {
                            "stop".into()
                        },
                        token_state: None,
                    },
                );
            }
            TURN_TERMINAL_PUBLISHED => {}
            _ => {
                biorouter::agents::subagent_handle::record_child_turn_terminal(
                    &session_id,
                    biorouter::agents::SubagentResult::from_error(
                        "Child turn terminated unexpectedly before it could publish a result",
                    ),
                );
                publish_turn_error(
                    &session_id,
                    "The model turn ended unexpectedly. Please retry.".to_string(),
                    "internal_error",
                    TurnErrorScope::Internal,
                    true,
                    None,
                );
            }
        }
    }
}

/// The turn body. Split out of `start_turn` so Task 8 can also call it with a
/// guard it acquired itself (`/reply` needs the guard's conflict detail before
/// it decides whether to open an SSE response at all).
///
/// # Stated non-goals — both are load-bearing, and both are one-line mistakes
///
/// 1. **This function performs NO whole-history rewrite.** It never calls
///    `SessionManager::replace_conversation` (the NAMED EXCEPTION) and never
///    calls `replace_conversation_preserving_tail` either. The only rewrite that
///    happens during a turn is the agent's own guarded compaction, inside
///    `Agent::reply`, which pairs its revision and its view through
///    `RewriteBasis` — a private struct with private fields and no field-wise
///    constructor, precisely so a conversation cannot be carried across an async
///    boundary, detached from its revision, and then used as an authoritative
///    rewrite. `extras.conversation_so_far` is a *seed* and must stay one;
///    making it a write here is that forbidden split, one layer up.
/// 2. **Events are published to the bus in stream order, never reordered,
///    filtered or coalesced.** #59's ordering invariant — *no
///    `MessagesPersisted` may precede a `Message` frame carrying one of the ids
///    it publishes* (`agent.rs`;
///    `docs/agent-loop/conversation-writeback-freshness.md`) — is a
///    producer-side property that the agent already satisfies, and since #66
///    satisfies BY CONSTRUCTION (the `persisted_ordering` seam: the frame cannot
///    be built except through one of three named shapes, and the yield-then-name
///    shape produces the ordering itself). Re-ordering here would make it false **at the
///    bus**, where no consumer-side flush can repair it, and the failure is
///    silent: a client that reads an id and then loses the stream claims every
///    stored row while holding none of the bodies, and the `expectedMessageIds`
///    guard truncates rows the user can still see. Coalescing is a per-client
///    concern and lives in `/reply`'s subscription (Task 8), not here.
pub async fn run_turn(
    state: Arc<AppState>,
    request: TurnRequest,
    turn_guard: crate::state::TurnGuard,
    cancel_token: CancellationToken,
) {
    let session_id = request.session_id.clone();
    if turn_guard.session_id() != session_id {
        tracing::error!(
            request_session_id = %session_id,
            guard_session_id = %turn_guard.session_id(),
            "turn: refusing to run with another session's guard"
        );
        biorouter::agents::subagent_handle::record_child_turn_terminal(
            &session_id,
            biorouter::agents::SubagentResult::from_error(
                "Child turn was refused because its runtime guard belonged to another session",
            ),
        );
        return;
    }

    let turn_id = turn_guard.turn_id().to_string();
    let outcome = Arc::new(AtomicU8::new(TURN_OUTCOME_NONE));
    let turn = run_turn_body(state, request, turn_id, cancel_token, Arc::clone(&outcome));
    supervise_turn(session_id, turn_guard, outcome, turn).await;
}

/// Belt-and-braces sweep of a session's broadcast ring, which
/// `broadcast::channel` allocates in full (1024 slots) at creation. Held by
/// [`run_turn_body`] for the whole turn, so the sweep runs on every exit path
/// including an unwind.
///
/// Read the comment this replaces carefully if you are tempted to lean on this
/// guard: it claimed the registry was an insert-and-never-remove map and that
/// this sweep was what stopped the leak. That has not been true since `publish`
/// became a pure lookup that never inserts and `Subscription::drop` took
/// ownership of the reclaim — every ring in the registry now belongs to some
/// observer, and the last observer to leave frees it.
/// `session_events::release_if_idle` is still public *for* this call and says
/// so, so the sweep stays; but it is a second line of defence, not the
/// mechanism, and a reader who believes otherwise will draw the wrong
/// conclusion about who owns a ring.
struct BusRelease(String);

impl Drop for BusRelease {
    fn drop(&mut self) {
        let session_id = std::mem::take(&mut self.0);
        // Only when a runtime is still around: the guard can also unwind
        // during runtime teardown, where there is nothing to spawn onto.
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            biorouter::session_events::release_if_idle(&session_id);
            return;
        };
        handle.spawn(async move {
            // Grace period, deliberately: at the instant `run_turn` returns,
            // the `/reply` SSE consumer is still holding its `Receiver` to
            // read the terminal frame, so an immediate call would always
            // find `receiver_count() > 0` and free nothing — and the entry
            // would then live forever for a session that never runs another
            // turn. 30 s also keeps a rapid back-and-forth from churning
            // one allocation per turn.
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            biorouter::session_events::release_if_idle(&session_id);
        });
    }
}

/// BR-71 §4.5: a human typing into a subagent's tab is an intervention the
/// parent must hear about. Sessions of other types are untouched.
///
/// **An existing stamp is never overwritten**, and that is load-bearing rather
/// than defensive. `run_turn` is the ONE turn runner (Task 6), so a
/// `workspace_send_prompt mode:"turn"` injection arrives here too — already
/// carrying `ProvenanceKind::AgentInjection`, applied by
/// `workspace_extension.rs` before it calls `start_detached_turn`. Stamping it
/// `UserDirect` because the *target* happens to be a subagent would relabel
/// another agent's injection as a human steer, and
/// `conversation_has_user_direct` would then report `human_intervened: true` to
/// the parent for a run no human ever touched — the exact false positive that
/// helper's `agent_injected` case exists to rule out, reintroduced one layer up.
/// A message that already names its origin is not a human at a composer.
pub(crate) fn stamp_user_direct_if_subagent(
    message: biorouter::conversation::message::Message,
    session_type: biorouter::session::session_manager::SessionType,
) -> biorouter::conversation::message::Message {
    use biorouter::conversation::message::{MessageProvenance, ProvenanceKind};
    if message.metadata.provenance.is_none()
        && session_type == biorouter::session::session_manager::SessionType::SubAgent
    {
        message.with_provenance(MessageProvenance {
            kind: ProvenanceKind::UserDirect,
            from_session_id: None,
            from_session_name: None,
        })
    } else {
        message
    }
}

/// Everything [`run_turn_body`] resolves out of the store before it can ask the
/// provider for a single token. Produced by [`prepare_turn`].
struct TurnSetup {
    agent: Arc<biorouter::agents::Agent>,
    session_config: SessionConfig,
    /// The turn's accumulator, already seeded and already carrying
    /// `user_message` as its last entry.
    all_messages: Conversation,
    /// The message to hand `agent.reply(...)`, **after** [`prepare_turn`] has
    /// stamped it. It must be this one and not the caller's original: the
    /// accumulator above already holds a clone of the stamped message, and two
    /// objects for one logical row that differ only in `metadata.provenance` is
    /// exactly the bug BR-71 §4.5 exists to prevent.
    user_message: Message,
    /// Index of the first message attributable to this generation. Provenance
    /// from an older child turn must not mark a later autonomous turn as human-
    /// intervened.
    turn_message_start: usize,
}

/// Drop a child whose delegated runtime profile could not be restored.
///
/// `restore_subagent_runtime_profile` installs the profile's grants one at a
/// time, so a failure part-way leaves a half-built agent cached under the
/// session id — reachable, and already holding whichever extensions landed
/// before the error. The next turn would take that agent as-is and never run
/// the check again. `POST /agent/resume` evicts on both of its refusal paths
/// for the same reason.
async fn evict_partially_restored_child(state: &Arc<AppState>, session_id: &str) {
    let _ = state.agent_manager.remove_session(session_id).await;
}

/// Put a delegated child back in the exact shape the daemon authored for it,
/// and refuse the turn if that is not possible.
///
/// A current child carries a profile for its prompt, structured response,
/// subworkflows, and exact grant set. A live agent recognizes the profile as
/// already installed and is left untouched; a cold one reconstructs it without
/// parsing the human-readable transcript.
///
/// A child with no profile has no daemon-authored grant set, and its legacy
/// `EnabledExtensionsState` snapshot is not a stand-in for one: that snapshot
/// can name `workspace` with an empty `available_tools`, which means EVERY
/// tool, and `load_extensions_from_session` would install it as an Explicit
/// entry OVER the four-tool injection delegation actually granted. The child
/// could then `workspace_open { new: { prompt } }` itself a fresh User session
/// carrying the machine's whole default extension set — an escape from the
/// delegated grant, out of a row the parent never authored. `POST /agent/resume`
/// already refuses the same row for the same reason.
///
/// An ordinary chat has no profile either, so the gate is scoped to `SubAgent`
/// rows. `false` means this turn's single terminal `TurnError` has been
/// published and the caller must return.
async fn restore_delegated_runtime_profile(
    state: &Arc<AppState>,
    session_id: &str,
    agent: &Arc<biorouter::agents::Agent>,
    session: &biorouter::session::Session,
) -> bool {
    let restored = match agent.restore_subagent_runtime_profile(session).await {
        Ok(restored) => restored,
        Err(e) => {
            tracing::error!(
                session_id,
                "turn: failed to restore subagent runtime profile: {e}"
            );
            evict_partially_restored_child(state, session_id).await;
            publish_turn_error(
                session_id,
                format!("Failed to restore subagent runtime profile: {e}"),
                "subagent_runtime_restore_failed",
                TurnErrorScope::Session,
                false,
                None,
            );
            return false;
        }
    };

    if restored || session.session_type != biorouter::session::SessionType::SubAgent {
        return true;
    }

    tracing::error!(
        session_id,
        "turn: subagent has no delegated runtime profile"
    );
    evict_partially_restored_child(state, session_id).await;
    publish_turn_error(
        session_id,
        "Biorouter could not restore the delegated runtime profile for this subagent, so it \
         cannot take a turn. Ask the parent conversation to delegate the work again."
            .to_string(),
        "subagent_runtime_profile_missing",
        TurnErrorScope::Session,
        false,
        None,
    );
    false
}

/// The setup phase: resolve the session's agent, read the session, build its
/// `SessionConfig`, and seed the accumulator.
///
/// `None` means the turn is over and **this function has already published its
/// single terminal `TurnError`** — the caller returns without publishing
/// anything else. Split out of [`run_turn_body`] to keep that function under the
/// repo's 100-line ceiling; it is a phase boundary the code already had, not a
/// line-count slice, and every publish/return pair moved across it verbatim.
/// `user_message` is taken **by value** so the stamp below can shadow it and
/// both consumers — the accumulator and `agent.reply(...)` — see the same
/// object; the returned [`TurnSetup`] carries it back out.
async fn prepare_turn(
    state: &Arc<AppState>,
    session_id: &str,
    user_message: Message,
    reasoning_effort: Option<biorouter::agents::ReasoningEffort>,
    conversation_so_far: Option<Conversation>,
) -> Option<TurnSetup> {
    // ⚠ WAIT for this session's extensions before running a turn on it.
    //
    // `/agent/start` kicks extension loading off in the background and returns
    // 200 immediately, so for roughly 300 ms a session answers with a couple of
    // tools before settling to its full set (measured 4/4; under concurrent
    // starts one session became "ready" holding 10 of 116). A turn inside that
    // window silently runs on a degraded toolset with no `subagent`, and the
    // model says "I cannot delegate" — **indistinguishable from the legitimate
    // condition-5 refusal**, which is what makes it expensive to diagnose.
    //
    // `/agent/resume` already awaited the same handle. This is the other half:
    // every interactive turn funnels through here.
    //
    // ⚠ It must NOT go inside `AppState::get_agent`, tempting as that is — the
    // background loader *itself* calls `get_agent` (`routes/agent.rs`), so a
    // wait there would have the loader waiting on itself. The wait belongs at
    // the points that consume a ready agent, not at the one that builds it.
    //
    // Best-effort by design: a load that failed is reported by the route that
    // started it, and a degraded turn still beats refusing to run. What this
    // buys is only that we do not start BEFORE the load settles. A second
    // caller blocks on the holder's mutex rather than skipping, so concurrent
    // waiters both wait.
    let _ = state.take_extension_loading_task(session_id).await;
    state.remove_extension_loading_task(session_id).await;

    let agent = match state.get_agent(session_id.to_string()).await {
        Ok(agent) => agent,
        Err(e) => {
            tracing::error!("turn: failed to get session agent: {e}");
            publish_turn_error(
                session_id,
                format!("Failed to get session agent: {e}"),
                "agent_unavailable",
                TurnErrorScope::Session,
                true,
                None,
            );
            return None;
        }
    };
    let session = match state.session_manager().get_session(session_id, true).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("turn: failed to read session: {e}");
            publish_turn_error(
                session_id,
                format!("Failed to read session: {e}"),
                "session_unavailable",
                TurnErrorScope::Session,
                true,
                None,
            );
            return None;
        }
    };

    if let Err(e) = agent.restore_persisted_provider_if_missing(&session).await {
        tracing::error!(
            session_id,
            "turn: failed to restore persisted provider: {e}"
        );
        publish_turn_error(
            session_id,
            format!("Failed to restore session provider: {e}"),
            "provider_restore_failed",
            TurnErrorScope::Provider,
            false,
            None,
        );
        return None;
    }

    if !restore_delegated_runtime_profile(state, session_id, &agent, &session).await {
        return None;
    }

    // BR-71 §4.5: a human typing into a subagent's tab (its composer posts to
    // /reply like any other tab) is an intervention the parent must hear about.
    // It has to happen HERE, above the accumulator seed below, so the row the
    // telemetry accumulator keeps and the message the agent replies to are the
    // same stamped object.
    let user_message = stamp_user_direct_if_subagent(user_message, session.session_type);

    let session_config = SessionConfig {
        id: session_id.to_string(),
        schedule_id: session.schedule_id.clone(),
        max_turns: None,
        max_tool_calls: None,
        budget: None,
        retry_config: None,
        // `ReasoningEffort` is `Copy`, so this is a plain move-out-of-a-copy;
        // the field types on both sides are `Option<ReasoningEffort>`.
        reasoning_effort,
    };

    // Seed the accumulator. NO STORAGE WRITE HAPPENS HERE — see [`run_turn`]'s
    // stated non-goal 1, and `TurnExtras.conversation_so_far`'s doc.
    //
    // `Some(_)` is the conversation `/reply` already validated and stored via
    // `apply_client_writeback` before spawning this task; `None` is every other
    // caller, which starts from the session's own history. The trailing push is
    // load-bearing and must not be dropped: without it
    // `emit_completion_metrics`' fallback `message_count` is off by one and
    // `track_tool_telemetry`'s lookup base differs by a message.
    let mut all_messages =
        conversation_so_far.unwrap_or_else(|| session.conversation.clone().unwrap_or_default());
    let turn_message_start = all_messages.len();
    all_messages.push(user_message.clone());

    Some(TurnSetup {
        agent,
        session_config,
        all_messages,
        user_message,
        turn_message_start,
    })
}

/// Best-effort LLM session rename — always spawned, unlike a tail on the lazy
/// reply stream which an early `break` in [`drive_stream`] would skip, leaving
/// the session stuck on "New chat".
fn spawn_session_rename(agent: &Arc<biorouter::agents::Agent>, session_id: &str) {
    let agent_for_rename = agent.clone();
    let session_id_for_rename = session_id.to_string();
    tokio::spawn(async move {
        agent_for_rename
            .maybe_rename_session(&session_id_for_rename)
            .await;
    });
}

/// The teardown phase: session-completion telemetry, the authoritative
/// end-of-turn token read, and — for a turn that did not already publish a
/// terminal error — the `TurnFinished` frame.
///
/// `cancel_token` rather than a precomputed flag, deliberately: the exit-type
/// label and the finish reason are two **separate** reads of the token, at the
/// two points the pre-split code read it (before and after the metrics await).
/// Collapsing them into one read would change what a turn cancelled inside
/// `emit_completion_metrics` reports.
struct TurnFinish<'a> {
    session_id: &'a str,
    session_type: &'static str,
    terminal_error: bool,
    cancel_token: &'a CancellationToken,
    turn_started: std::time::Instant,
    fallback_message_count: usize,
    outcome: &'a AtomicU8,
}

async fn finish_turn(state: &Arc<AppState>, finish: TurnFinish<'_>) {
    let TurnFinish {
        session_id,
        session_type,
        terminal_error,
        cancel_token,
        turn_started,
        fallback_message_count,
        outcome,
    } = finish;
    let exit_type = if terminal_error {
        "error"
    } else if cancel_token.is_cancelled() {
        "cancelled"
    } else {
        "normal"
    };
    emit_completion_metrics(
        state,
        session_id,
        session_type,
        exit_type,
        turn_started.elapsed(),
        fallback_message_count,
    )
    .await;

    // BR-52: one authoritative read at the end of the turn — the single point
    // where a client's token readout is reconciled with the store, so nothing
    // written outside this turn (a background eager compaction, a concurrent
    // scheduled run) can leave the UI on a stale count.
    let final_token_state = get_token_state(state.session_manager(), session_id).await;

    if !terminal_error {
        session_events::publish(
            session_id,
            SessionBusEvent::TurnFinished {
                reason: if cancel_token.is_cancelled() {
                    "cancelled".into()
                } else {
                    "stop".into()
                },
                token_state: Some(final_token_state),
            },
        );
        outcome.store(TURN_TERMINAL_PUBLISHED, Ordering::Release);
    }
}

fn register_detached_turn_work(
    enabled: bool,
    session_id: &str,
    cancel_token: &CancellationToken,
) -> Option<biorouter_mcp::active_work::ActiveWorkGuard> {
    use biorouter_mcp::active_work::{ActiveWorkGuard, ActiveWorkKind};
    enabled.then(|| {
        let token = cancel_token.clone();
        let cancel: std::sync::Arc<dyn Fn() + Send + Sync> =
            std::sync::Arc::new(move || token.cancel());
        ActiveWorkGuard::register(
            ActiveWorkKind::DetachedTurn,
            "detached workspace turn",
            Some(format!("session {session_id}")),
            Some(session_id.to_string()),
            Some(cancel),
        )
    })
}

async fn run_turn_body(
    state: Arc<AppState>,
    request: TurnRequest,
    turn_id: String,
    cancel_token: CancellationToken,
    outcome: Arc<AtomicU8>,
) {
    let TurnRequest {
        session_id,
        mut user_message,
        extras,
    } = request;
    let turn_started = std::time::Instant::now();

    // Defer scheduled background jobs while a turn is in flight.
    let _interactive_turn = biorouter::scheduler::interactive_turn_guard();

    let _bus_release = BusRelease(session_id.clone());

    let _active_work =
        register_detached_turn_work(extras.register_active_work, &session_id, &cancel_token);

    let admitted_child_turn = biorouter::agents::subagent_handle::admit_child_turn(&session_id);
    if admitted_child_turn {
        user_message =
            stamp_user_direct_if_subagent(user_message, biorouter::session::SessionType::SubAgent);
    }
    let setup_failure_messages =
        Conversation::new_unvalidated(std::iter::once(user_message.clone()));

    biorouter::agents::subagent_handle::open_parent_continuation_admission(&session_id);
    session_events::publish(&session_id, SessionBusEvent::TurnStarted { turn_id });

    // One terminal event per turn, always. Every exit path below publishes
    // exactly one `TurnError` or one `TurnFinished`, never both and never two —
    // and the one exit path that cannot publish anything, a panic, is covered
    // by `supervise_turn`, which publishes `internal_error` while this session's
    // turn guard is still held. The two phases split out of this function,
    // [`prepare_turn`] and [`finish_turn`], each own their share of that
    // guarantee and nothing else publishes a terminal on this path.
    //
    // Every terminal goes through [`publish_turn_error`], whose `provider_kind`
    // is a parameter rather than a hardcoded `None`: it is one of the three
    // fields the desktop's rate-limit/retry/compaction recovery reads, and
    // `classify_abort` is the only thing in the process that produces it.
    let Some(TurnSetup {
        agent,
        session_config,
        mut all_messages,
        user_message,
        turn_message_start,
    }) = prepare_turn(
        &state,
        &session_id,
        user_message,
        extras.reasoning_effort,
        extras.conversation_so_far,
    )
    .await
    else {
        record_setup_failure(
            &session_id,
            &cancel_token,
            &setup_failure_messages,
            &outcome,
        );
        return;
    };

    let mut stream = match agent
        .reply(user_message, session_config, Some(cancel_token.clone()))
        .await
    {
        Ok(stream) => stream,
        Err(e) => {
            tracing::error!("turn: failed to start reply stream: {e:?}");
            publish_turn_error(
                &session_id,
                e.to_string(),
                "inference_start_failed",
                TurnErrorScope::Inference,
                false,
                None,
            );
            record_child_turn_outcome(
                &session_id,
                true,
                cancel_token.is_cancelled(),
                &all_messages,
                turn_message_start,
                &outcome,
            );
            return;
        }
    };

    let terminal_error = drive_stream(
        &session_id,
        &mut stream,
        &cancel_token,
        &mut all_messages,
        Some(agent.as_ref()),
    )
    .await;

    record_child_turn_outcome(
        &session_id,
        terminal_error,
        cancel_token.is_cancelled(),
        &all_messages,
        turn_message_start,
        &outcome,
    );

    spawn_session_rename(&agent, &session_id);

    finish_turn(
        &state,
        TurnFinish {
            session_id: &session_id,
            session_type: extras.kind.session_type(),
            terminal_error,
            cancel_token: &cancel_token,
            turn_started,
            fallback_message_count: all_messages.len(),
            outcome: &outcome,
        },
    )
    .await;
}

fn record_setup_failure(
    session_id: &str,
    cancel_token: &CancellationToken,
    setup_messages: &Conversation,
    outcome: &AtomicU8,
) {
    record_child_turn_outcome(
        session_id,
        true,
        cancel_token.is_cancelled(),
        setup_messages,
        0,
        outcome,
    );
}

fn record_child_turn_outcome(
    session_id: &str,
    terminal_error: bool,
    cancelled: bool,
    all_messages: &Conversation,
    turn_message_start: usize,
    outcome: &AtomicU8,
) {
    let turn_messages = Conversation::new_unvalidated(
        all_messages
            .messages()
            .iter()
            .skip(turn_message_start)
            .cloned(),
    );
    let mut child_result = if terminal_error {
        biorouter::agents::SubagentResult::from_aborted_turn(
            &turn_messages,
            "child_turn_error",
            "Child turn ended with an error; inspect its conversation for the classified cause",
        )
    } else {
        biorouter::agents::SubagentResult::from_conversation(&turn_messages, None, true)
    };
    if cancelled {
        child_result.mark_cancelled();
    }
    child_result.human_intervened =
        biorouter::agents::subagent_result::conversation_has_user_direct(&turn_messages);
    biorouter::agents::subagent_handle::record_child_turn_terminal(session_id, child_result);
    outcome.store(
        if terminal_error {
            TURN_TERMINAL_PUBLISHED
        } else if cancelled {
            TURN_OUTCOME_CANCELLED
        } else {
            TURN_OUTCOME_STOP
        },
        Ordering::Release,
    );
}

/// Publish one `TurnError` frame for `session_id`.
///
/// A free function rather than a closure inside any one phase, because all four
/// of them — [`prepare_turn`], [`run_turn_body`], [`drive_stream`] and
/// [`supervise_turn`] — publish terminals, and they must all produce the same
/// envelope: `scope`, `retryable` and `provider_kind` are exactly the three
/// fields the desktop's rate-limit / retry / compaction recovery keys off, and
/// nothing else in the process emits them.
fn publish_turn_error(
    session_id: &str,
    message: String,
    code: &str,
    scope: TurnErrorScope,
    retryable: bool,
    provider_kind: Option<String>,
) {
    session_events::publish(
        session_id,
        SessionBusEvent::TurnError {
            message,
            code: code.to_string(),
            scope: scope.wire_value().to_string(),
            retryable,
            provider_kind,
        },
    );
}

/// Drain the agent's event stream onto the session bus, returning `true` when
/// the turn ended on a **published terminal error**.
///
/// Split out of [`run_turn`] because [`run_turn`] as a whole is not testable
/// without a provider: a provider-less agent fails inside `agent.reply` and
/// never reaches this loop, so the abort classifier's integration with the
/// stream, the "an abort is republished classified and NOT also raw" rule and
/// the cancellation escape would every one of them be uncovered. Generic over
/// the stream so a test can supply the exact events it wants — including a
/// stream that never yields at all.
///
/// Events are published in stream order, never reordered, filtered or
/// coalesced; see [`run_turn`]'s stated non-goal 2 for why that is load-bearing.
async fn settle_accepted_interrupts(
    session_id: &str,
    all_messages: &mut Conversation,
    agent: &biorouter::agents::Agent,
) -> anyhow::Result<()> {
    biorouter::agents::subagent_handle::begin_parent_closing(session_id);
    for message in agent
        .settle_carried_over_soft_interrupts(session_id)
        .await?
    {
        all_messages.push(message.clone());
        session_events::publish(
            session_id,
            SessionBusEvent::Agent(AgentEvent::Message(message)),
        );
    }
    Ok(())
}

async fn drain_delegated_work_after_forced_exit(
    session_id: &str,
    all_messages: &mut Conversation,
    agent: &biorouter::agents::Agent,
    cancel_token: &CancellationToken,
) -> anyhow::Result<()> {
    while let Some(message) = agent
        .next_native_supervision_message_after_forced_exit(session_id, Some(cancel_token.clone()))
        .await?
    {
        all_messages.push(message.clone());
        session_events::publish(
            session_id,
            SessionBusEvent::Agent(AgentEvent::Message(message)),
        );
    }
    Ok(())
}

async fn publish_stream_failure(
    session_id: &str,
    error: anyhow::Error,
    all_messages: &mut Conversation,
    agent: Option<&biorouter::agents::Agent>,
    cancel_token: &CancellationToken,
) {
    if let Some(agent) = agent {
        if let Err(settlement_error) =
            settle_accepted_interrupts(session_id, all_messages, agent).await
        {
            tracing::error!("turn: failed to settle accepted interrupts: {settlement_error}");
            publish_turn_error(
                session_id,
                settlement_error.to_string(),
                "interrupt_settlement_failed",
                TurnErrorScope::Session,
                false,
                None,
            );
            return;
        }
        if let Err(supervision_error) =
            drain_delegated_work_after_forced_exit(session_id, all_messages, agent, cancel_token)
                .await
        {
            tracing::error!("turn: failed to supervise delegated work: {supervision_error}");
            publish_turn_error(
                session_id,
                supervision_error.to_string(),
                "delegated_supervision_failed",
                TurnErrorScope::Session,
                false,
                None,
            );
            return;
        }
    }
    tracing::error!("turn: stream error: {error}");
    publish_turn_error(
        session_id,
        error.to_string(),
        "inference_error",
        TurnErrorScope::Inference,
        false,
        None,
    );
}

async fn drive_stream<S>(
    session_id: &str,
    stream: &mut S,
    cancel_token: &CancellationToken,
    all_messages: &mut Conversation,
    agent: Option<&biorouter::agents::Agent>,
) -> bool
where
    S: futures::Stream<Item = anyhow::Result<AgentEvent>> + Unpin,
{
    loop {
        // The hard cancellation escape. Without it this loop can only end when
        // the agent yields, and `AppState::cancel_turn` deliberately does not
        // remove the session's map entry — it relies on the `TurnGuard`
        // clearing the slot as the turn task unwinds. So an agent parked inside
        // a provider call that never observes the token would hold the turn
        // lock indefinitely and every later `/reply` for that session would
        // 409 until the daemon restarted; it would also make
        // `ActiveWorkKind::DetachedTurn`'s cancel closure, which does nothing
        // but trip this token, a no-op.
        //
        // Unbiased, exactly like `/reply`'s `select!`: a stream item that is
        // already ready may still be processed in the iteration the token
        // trips. That is the behaviour Task 8 replaces, so it is the behaviour
        // reproduced here.
        let item = tokio::select! {
            _ = cancel_token.cancelled() => {
                tracing::info!("turn: cancelled");
                if let Some(agent) = agent {
                    if let Err(error) =
                        settle_accepted_interrupts(session_id, all_messages, agent).await
                    {
                        tracing::error!("turn: failed to settle accepted interrupts: {error}");
                        publish_turn_error(
                            session_id,
                            error.to_string(),
                            "interrupt_settlement_failed",
                            TurnErrorScope::Session,
                            false,
                            None,
                        );
                        return true;
                    }
                }
                break;
            }
            item = stream.next() => item,
        };
        let Some(item) = item else { break };

        match item {
            Ok(event) => {
                // Bookkeeping that belongs to the TURN, not to any consumer:
                // telemetry and the accumulated conversation used by the
                // completion metrics.
                match &event {
                    AgentEvent::Message(message) => {
                        for content in &message.content {
                            track_tool_telemetry(content, all_messages.messages());
                        }
                        all_messages.push(message.clone());
                    }
                    AgentEvent::HistoryReplaced(new_messages) => {
                        *all_messages = new_messages.clone();
                    }
                    _ => {}
                }

                // An abort is CLASSIFIED and republished as this turn's single
                // terminal event — it is deliberately NOT also forwarded raw.
                // `map_bus_event` maps both `SessionBusEvent::TurnError` and
                // `AgentEvent::TurnAborted` to `MessageEvent::Error`, so
                // publishing both would give every consumer two terminal Error
                // frames for one abort. The agent has already yielded the
                // human-readable assistant message in an earlier iteration; this
                // event is what stops the desktop from rendering a provider 403
                // as a completed turn — and, with `classify_abort`, what keeps
                // it recoverable.
                if let AgentEvent::TurnAborted { code, message } = &event {
                    let (scope, retryable, provider_kind) = classify_abort(code);
                    tracing::error!(
                        abort = code.wire_code(),
                        scope = scope.wire_value(),
                        retryable,
                        provider_kind = provider_kind.as_deref().unwrap_or("none"),
                        "Turn aborted"
                    );
                    publish_turn_error(
                        session_id,
                        message.clone(),
                        code.wire_code(),
                        scope,
                        retryable,
                        provider_kind,
                    );
                    return true;
                }

                // Publish the raw AgentEvent. Consumers map it; the runner does
                // not pre-render any wire frame.
                session_events::publish(session_id, SessionBusEvent::Agent(event));
            }
            Err(e) => {
                // `"inference_error"`, the code `/reply`'s own arm for this
                // exact `Ok(Some(Err(e)))` case has always published — NOT
                // `"stream_error"`, which the desktop mints for itself when the
                // SSE socket dies (`clientTurnError(error, 'stream_error',
                // 'transport')`) and reserves in `MIDSTREAM_CODES`. Reusing it
                // here would give one code two unrelated meanings, separable
                // only by `scope`, and would silently re-bucket every log and
                // dashboard keyed on `code` the moment Task 8 lands.
                publish_stream_failure(session_id, e, all_messages, agent, cancel_token).await;
                return true;
            }
        }
    }
    false
}

/// The terminal-error classifier, moved out of `/reply`'s event loop so the ONE
/// runner still produces the full envelope. Pure, so it is unit-testable
/// without a provider (see this module's tests).
pub(crate) fn classify_abort(
    code: &biorouter::agents::TurnAbortCode,
) -> (TurnErrorScope, bool, Option<String>) {
    use biorouter::agents::TurnAbortCode;
    match code {
        TurnAbortCode::ProviderFailure { kind } => (
            TurnErrorScope::Provider,
            kind.is_transient(),
            Some(kind.wire_code().to_string()),
        ),
        // #31/#41: a session-store failure is a Session-scoped error — not the
        // provider's fault and not retryable until the local db problem is fixed.
        TurnAbortCode::SessionStore => (TurnErrorScope::Session, false, None),
        TurnAbortCode::ToolLoop { .. } => (TurnErrorScope::Inference, false, None),
        TurnAbortCode::WorkerTimeout { .. } => (TurnErrorScope::Inference, true, None),
        TurnAbortCode::OutputRecoveryExhausted { .. } => (TurnErrorScope::Inference, false, None),
        // Signed history that is already on the record and cannot be replayed
        // unchanged: there is nowhere to resume from, so offering Retry would be
        // offering a button that cannot work.
        TurnAbortCode::SignedReplayInvalidated => (TurnErrorScope::Inference, false, None),
        // A dropped connection mid-tool-call, with the partial response
        // discarded rather than persisted. The chat is back at the state the
        // turn started from, which is precisely the precondition `retryTurn`
        // needs — the last stored row is still the user's message.
        TurnAbortCode::SignedStreamTruncated => (TurnErrorScope::Inference, true, None),
    }
}

/// The session-completion telemetry, byte-for-byte the block in `/reply`'s task
/// at `275d735d` with one substitution: the three literal `session_type = "app"`
/// fields become `session_type = session_type_label`, so an injected turn is not
/// counted as an app session.
async fn emit_completion_metrics(
    state: &Arc<AppState>,
    session_id: &str,
    session_type_label: &'static str,
    exit_type: &'static str,
    duration: std::time::Duration,
    fallback_message_count: usize,
) {
    if let Ok(session) = state.session_manager().get_session(session_id, true).await {
        let total_tokens = session.total_tokens.unwrap_or(0);
        tracing::info!(
            counter.biorouter.session_completions = 1,
            session_type = session_type_label,
            interface = "ui",
            exit_type = exit_type,
            duration_ms = duration.as_millis() as u64,
            total_tokens = total_tokens,
            message_count = session.message_count,
            "Session completed"
        );

        tracing::info!(
            counter.biorouter.session_duration_ms = duration.as_millis() as u64,
            session_type = session_type_label,
            interface = "ui",
            "Session duration"
        );

        if total_tokens > 0 {
            tracing::info!(
                counter.biorouter.session_tokens = total_tokens,
                session_type = session_type_label,
                interface = "ui",
                "Session tokens"
            );
        }
    } else {
        tracing::info!(
            counter.biorouter.session_completions = 1,
            session_type = session_type_label,
            interface = "ui",
            exit_type = exit_type,
            duration_ms = duration.as_millis() as u64,
            total_tokens = 0u64,
            message_count = fallback_message_count,
            "Session completed"
        );

        tracing::info!(
            counter.biorouter.session_duration_ms = duration.as_millis() as u64,
            session_type = session_type_label,
            interface = "ui",
            "Session duration"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default)]
    struct DiagnosticCapture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for DiagnosticCapture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// D2, the turn half. Every interactive turn funnels through `setup`, and
    /// it must await this session's pending extension load before it takes the
    /// agent — otherwise a reply sent in the ~300 ms after `/agent/start`
    /// returns runs on a partial toolset and the model reports it cannot
    /// delegate, which is indistinguishable from the real refusal.
    ///
    /// ⚠ A source scan, because the behaviour needs a live `AppState` with a
    /// provider bound and the ordering is what matters, not the return value.
    /// It anchors on the CALL, not on the surrounding comment: a guard that
    /// matches its own explanatory prose passes when the code is gone, and this
    /// repo has produced that own-goal three times.
    #[test]
    fn a_turn_waits_for_the_sessions_extensions_before_taking_the_agent() {
        let src = include_str!("turn.rs");
        let wait = src
            .find("state.take_extension_loading_task(session_id).await;")
            .expect("run_turn's setup no longer waits for the session's extensions");
        let take = src
            .find("state.get_agent(session_id.to_string()).await")
            .expect("setup no longer takes the agent");
        assert!(
            wait < take,
            "the extension wait must come BEFORE the agent is taken, not after"
        );
    }
    use biorouter::conversation::message::Message;
    use biorouter::session::session_manager::SessionType;
    use biorouter::session::{EnabledExtensionsState, ExtensionState};
    use biorouter::session_events::{self, SessionBusEvent};
    use serial_test::serial;
    use tokio_util::sync::CancellationToken;

    /// NOTE — two things about every test in this module:
    ///
    /// 1. `AppState::new()` opens the **ONE shared session database** (it goes
    ///    through `AgentManager::instance()` → `SessionManager::instance()`;
    ///    `routes/session.rs` carries the same warning). Every test in the
    ///    binary creates rows in the same store, so keep session names unique and
    ///    never assert on total row counts. It is a throwaway root rather than
    ///    the developer's own history — `src/test_sandbox.rs` pins it there
    ///    before the first test runs.
    /// 2. The `TempDir` is the session's **working dir**, not a database.
    ///    `create_session`'s first parameter is `working_dir`
    ///    (`session_manager.rs:1191-1196`). An earlier draft did
    ///    `std::mem::forget(temp)` with the comment "keep the DB alive" — a false
    ///    invariant that would send the next reader looking for a database that
    ///    is not there. The guard is still returned, because deleting the
    ///    working directory while the turn runner is using it is its own bug;
    ///    the caller just has to hold it.
    ///
    /// (Contrast Task 18's helper, which builds `SessionManager::new(temp.path())`
    /// — there the TempDir really does hold the sqlite file.)
    async fn session(
        state: &std::sync::Arc<crate::state::AppState>,
        name: &str,
    ) -> (tempfile::TempDir, String) {
        let temp = tempfile::TempDir::new().unwrap();
        let s = state
            .session_manager()
            .create_session(temp.path().to_path_buf(), name.into(), SessionType::User)
            .await
            .unwrap();
        (temp, s.id)
    }

    #[tokio::test]
    async fn a_cold_subagent_turn_reports_a_typed_persisted_provider_restore_failure() {
        let state = crate::state::AppState::new().await.unwrap();
        let workdir = tempfile::TempDir::new().unwrap();
        let child = state
            .session_manager()
            .create_session(
                workdir.path().to_path_buf(),
                "br71 cold child provider restore".into(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        let missing_provider = "br71-turn-provider-not-in-the-factory";
        state
            .session_manager()
            .update(&child.id)
            .provider_name(missing_provider)
            .model_config(biorouter::model::ModelConfig::new("br71-turn-model").unwrap())
            .apply()
            .await
            .unwrap();
        let mut events = session_events::subscribe(&child.id);

        let setup = prepare_turn(
            &state,
            &child.id,
            Message::user().with_text("continue"),
            None,
            None,
        )
        .await;

        assert!(
            setup.is_none(),
            "a failed restore must stop before inference"
        );
        // Scan for this turn's terminal rather than taking whatever frame is at
        // the head of the ring. `session_events` is a process-global bus keyed
        // by session id, while `SessionManager` allocates ids as
        // `YYYYMMDD_<max+1>` *per database* — so a sibling test on a temporary
        // store restarts the ordinals and can publish onto this id. Taking the
        // first frame made this test fail on a neighbour's event; the daily
        // counter also resets at midnight, so it recurred by date.
        let (message, scope, retryable) = loop {
            let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
                .await
                .expect("restore failure must publish a terminal")
                .unwrap();
            if let SessionBusEvent::TurnError {
                message,
                code,
                scope,
                retryable,
                ..
            } = event
            {
                if code != "provider_restore_failed" {
                    continue;
                }
                break (message, scope, retryable);
            }
        };
        assert_eq!(scope, TurnErrorScope::Provider.wire_value());
        assert!(!retryable);
        assert!(
            message.contains(missing_provider),
            "unexpected error: {message}"
        );
        assert!(!message.contains("Provider not set"));
    }

    #[tokio::test]
    async fn a_direct_child_followup_claims_its_generation_before_setup_can_fail() {
        let state = crate::state::AppState::new().await.unwrap();
        let workdir = tempfile::TempDir::new().unwrap();
        let child = state
            .session_manager()
            .create_session(
                workdir.path().to_path_buf(),
                "br71 admitted child setup failure".into(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        state
            .session_manager()
            .update(&child.id)
            .provider_name("br71-followup-provider-not-in-the-factory")
            .model_config(biorouter::model::ModelConfig::new("br71-turn-model").unwrap())
            .apply()
            .await
            .unwrap();

        let handle = biorouter::agents::subagent_handle::BackgroundSubagent::register(
            "br71-admitted-child-parent",
            &child.id,
            "delegated work",
            CancellationToken::new(),
        );
        biorouter::agents::subagent_handle::begin_child_turn(&child.id);
        handle.complete(biorouter::agents::SubagentResult::from_error(
            "initial generation",
        ));
        let initial_generation = handle.child_turn_generation();
        assert!(handle.mark_terminal_generation_collected_if_generation(initial_generation));

        let cancel = CancellationToken::new();
        let guard = state
            .try_begin_turn_idempotent(&child.id, cancel.clone(), None)
            .expect("the child is idle");
        run_turn(
            state,
            TurnRequest::new(
                child.id.clone(),
                Message::user().with_text("direct follow-up"),
            ),
            guard,
            cancel,
        )
        .await;

        assert_eq!(handle.child_turn_generation(), initial_generation + 1);
        let terminal = handle
            .terminal_generation()
            .expect("setup failure must settle the admitted generation");
        assert_eq!(terminal.generation, initial_generation + 1);
        assert!(terminal.result.human_intervened);
        assert!(!handle.latest_generation_collected());
    }

    #[test]
    fn a_cancelled_direct_followup_is_incomplete_and_keeps_human_intervention() {
        use biorouter::agents::subagent_result::SubagentStatus;

        let handle = biorouter::agents::subagent_handle::BackgroundSubagent::register(
            "br71-cancelled-followup-parent",
            "br71-cancelled-followup-child",
            "delegated work",
            CancellationToken::new(),
        );
        biorouter::agents::subagent_handle::begin_child_turn(&handle.child_session_id);
        handle.complete(biorouter::agents::SubagentResult::from_error(
            "initial generation",
        ));
        assert!(biorouter::agents::subagent_handle::admit_child_turn(
            &handle.child_session_id
        ));
        let followup_generation = handle.child_turn_generation();
        let messages = Conversation::new_unvalidated([
            stamp_user_direct_if_subagent(
                Message::user().with_text("stop here"),
                SessionType::SubAgent,
            ),
            Message::assistant().with_text("partial useful work"),
        ]);

        let outcome = AtomicU8::new(TURN_OUTCOME_NONE);
        record_child_turn_outcome(
            &handle.child_session_id,
            false,
            true,
            &messages,
            0,
            &outcome,
        );

        let terminal = handle
            .terminal_generation()
            .expect("cancellation must settle the direct follow-up");
        assert_eq!(terminal.generation, followup_generation);
        assert_eq!(terminal.result.status, SubagentStatus::Incomplete);
        assert!(terminal.result.error.is_none());
        assert!(terminal.result.summary.contains("partial useful work"));
        assert!(terminal.result.human_intervened);
    }

    #[test]
    fn human_intervention_is_scoped_to_the_exact_direct_followup_generation() {
        use biorouter::conversation::message::{MessageProvenance, ProvenanceKind};

        let handle = biorouter::agents::subagent_handle::BackgroundSubagent::register(
            "br71-generation-provenance-parent",
            "br71-generation-provenance-child",
            "delegated work",
            CancellationToken::new(),
        );
        biorouter::agents::subagent_handle::begin_child_turn(&handle.child_session_id);
        handle.complete(biorouter::agents::SubagentResult::from_error(
            "initial generation",
        ));
        assert!(biorouter::agents::subagent_handle::admit_child_turn(
            &handle.child_session_id
        ));
        let messages = Conversation::new_unvalidated([
            stamp_user_direct_if_subagent(
                Message::user().with_text("old human turn"),
                SessionType::SubAgent,
            ),
            Message::assistant().with_text("old answer"),
            Message::user()
                .with_text("current parent injection")
                .with_provenance(MessageProvenance {
                    kind: ProvenanceKind::AgentInjection,
                    from_session_id: Some("br71-generation-provenance-parent".into()),
                    from_session_name: Some("Parent".into()),
                }),
            Message::assistant().with_text("current answer"),
        ]);

        let outcome = AtomicU8::new(TURN_OUTCOME_NONE);
        record_child_turn_outcome(
            &handle.child_session_id,
            false,
            false,
            &messages,
            2,
            &outcome,
        );

        let result = handle.terminal_generation().unwrap().result;
        assert_eq!(result.summary, "current answer");
        assert!(!result.human_intervened);
    }

    /// The shape a pre-runtime-profile child row carries, and the reason the
    /// snapshot can never stand in for a profile: an empty `available_tools`
    /// means EVERY tool of that extension (`extension_manager.rs`), so this one
    /// value is a full grant of workspace control — `workspace_open { new: {
    /// prompt } }` included, which mints a User session holding the machine's
    /// default extension set.
    ///
    /// The fixture is deliberately `workspace` and not some innocuous
    /// extension: with `todo` here, a "no `workspace__` tools" assertion passes
    /// against code that restores the snapshot verbatim, and that is exactly
    /// how the vulnerable behaviour was asserted as correct.
    fn broad_workspace_snapshot() -> biorouter::agents::ExtensionConfig {
        biorouter::agents::ExtensionConfig::Platform {
            name: "workspace".into(),
            description: "Legacy broad workspace snapshot".into(),
            bundled: Some(true),
            available_tools: Vec::new(),
        }
    }

    async fn seed_child_with_extension_data(
        state: &std::sync::Arc<crate::state::AppState>,
        name: &str,
        session_type: SessionType,
        seed: impl FnOnce(&mut biorouter::session::ExtensionData),
    ) -> (tempfile::TempDir, String) {
        let workdir = tempfile::TempDir::new().unwrap();
        let mut session = state
            .session_manager()
            .create_session(workdir.path().to_path_buf(), name.into(), session_type)
            .await
            .unwrap();
        seed(&mut session.extension_data);
        state
            .session_manager()
            .update(&session.id)
            .provider_name("ollama")
            .model_config(biorouter::model::ModelConfig::new("br71-local-model").unwrap())
            .extension_data(session.extension_data)
            .apply()
            .await
            .unwrap();
        (workdir, session.id)
    }

    /// Read the terminal `TurnError` a refused setup publishes, and assert its
    /// scope and retryability.
    ///
    /// ⚠ It SCANS for `code` rather than taking the first frame, because the
    /// bus is process-global and keyed by session id while `create_session`
    /// hands out `YYYYMMDD_N` **per database** — so a neighbouring test built on
    /// its own `SessionManager::new(temp)` publishes onto the same ring as a
    /// real-store session with the same ordinal. Demanding the first frame made
    /// this test fail on that traffic roughly one run in three.
    async fn refusal(events: &mut session_events::Subscription, code: &str) -> String {
        let deadline = std::time::Duration::from_secs(5);
        loop {
            let event = tokio::time::timeout(deadline, events.recv())
                .await
                .unwrap_or_else(|_| panic!("a refused setup must publish a {code} terminal"))
                .unwrap();
            if let SessionBusEvent::TurnError {
                message,
                code: published,
                scope,
                retryable,
                ..
            } = event
            {
                if published != code {
                    continue;
                }
                assert_eq!(scope, TurnErrorScope::Session.wire_value());
                assert!(!retryable, "a missing grant set is not retryable");
                return message;
            }
        }
    }

    /// Every `TurnError` code already sitting in the ring. `prepare_turn` has
    /// been awaited by the time this runs, so anything it published is here.
    fn published_error_codes(events: &mut session_events::Subscription) -> Vec<String> {
        let mut codes = Vec::new();
        while let Ok(event) = events.try_recv() {
            if let SessionBusEvent::TurnError { code, .. } = event {
                codes.push(code);
            }
        }
        codes
    }

    async fn tool_names(
        agent: &std::sync::Arc<biorouter::agents::Agent>,
        session_id: &str,
    ) -> Vec<String> {
        agent
            .list_tools(session_id, None)
            .await
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect()
    }

    /// A legacy child — one whose row predates the daemon-authored runtime
    /// profile — must be REFUSED, not hydrated from its saved extension list.
    ///
    /// The saved list is the escalation vector, not a fallback: nothing that
    /// wrote it was a delegation decision, `load_extensions_from_session`
    /// applies none of the clamps `restore_subagent_runtime_profile` applies,
    /// and it deliberately replaces an auto-injected entry with an Explicit one
    /// — so the four-tool `workspace` injection a subagent is granted becomes
    /// the whole extension. `POST /agent/resume` already refuses this row.
    #[tokio::test]
    #[serial]
    async fn a_cold_subagent_turn_without_a_runtime_profile_is_refused_not_hydrated() {
        let state = crate::state::AppState::new().await.unwrap();
        let (_workdir, child_id) = seed_child_with_extension_data(
            &state,
            "br71 cold child no profile",
            SessionType::SubAgent,
            |extension_data| {
                EnabledExtensionsState::new(vec![broad_workspace_snapshot()])
                    .to_extension_data(extension_data)
                    .unwrap();
            },
        )
        .await;
        // Held across the call so the tools can be read after the refusal has
        // evicted the agent from the manager.
        let agent = state.get_agent(child_id.clone()).await.unwrap();
        let mut events = session_events::subscribe(&child_id);

        let setup = prepare_turn(
            &state,
            &child_id,
            Message::user().with_text("continue"),
            None,
            None,
        )
        .await;

        assert!(
            setup.is_none(),
            "a child with no delegated grant set must not reach inference"
        );
        let message = refusal(&mut events, "subagent_runtime_profile_missing").await;
        assert!(
            message.contains("delegate the work again"),
            "the refusal must say what to do: {message}"
        );

        let tools = tool_names(&agent, &child_id).await;
        assert!(
            tools.iter().all(|name| !name.starts_with("workspace__")),
            "the legacy snapshot granted workspace control to a subagent: {tools:?}"
        );
        assert!(
            !state.agent_manager.has_session(&child_id).await,
            "a refused child must not stay cached for the next turn to reuse"
        );
    }

    /// The other half of the same gate: a profile that exists but cannot be
    /// read is a refusal too, and it must not fall back to the snapshot either.
    #[tokio::test]
    #[serial]
    async fn a_cold_subagent_turn_with_an_unreadable_runtime_profile_is_refused() {
        let state = crate::state::AppState::new().await.unwrap();
        let (_workdir, child_id) = seed_child_with_extension_data(
            &state,
            "br71 cold child corrupt profile",
            SessionType::SubAgent,
            |extension_data| {
                EnabledExtensionsState::new(vec![broad_workspace_snapshot()])
                    .to_extension_data(extension_data)
                    .unwrap();
                extension_data.set_extension_state(
                    "subagent_runtime_profile",
                    "v999",
                    serde_json::json!({"system_prompt": "do not install"}),
                );
            },
        )
        .await;
        let agent = state.get_agent(child_id.clone()).await.unwrap();
        let mut events = session_events::subscribe(&child_id);

        let setup = prepare_turn(
            &state,
            &child_id,
            Message::user().with_text("continue"),
            None,
            None,
        )
        .await;

        assert!(setup.is_none(), "an unreadable profile must stop the turn");
        refusal(&mut events, "subagent_runtime_restore_failed").await;
        let tools = tool_names(&agent, &child_id).await;
        assert!(
            tools.iter().all(|name| !name.starts_with("workspace__")),
            "a failed restore fell back to the legacy snapshot: {tools:?}"
        );
        assert!(
            !state.agent_manager.has_session(&child_id).await,
            "a partially restored child must not stay cached"
        );
    }

    /// The over-correction guard. The gate is scoped to `SubAgent` rows: an
    /// ordinary chat has no runtime profile either, and refusing it would take
    /// the whole application down rather than close a delegation hole.
    #[tokio::test]
    #[serial]
    async fn a_user_session_with_a_legacy_extension_snapshot_still_takes_its_turn() {
        let state = crate::state::AppState::new().await.unwrap();
        let (_workdir, session_id) = seed_child_with_extension_data(
            &state,
            "br71 user session legacy snapshot",
            SessionType::User,
            |extension_data| {
                EnabledExtensionsState::new(vec![broad_workspace_snapshot()])
                    .to_extension_data(extension_data)
                    .unwrap();
            },
        )
        .await;
        let mut events = session_events::subscribe(&session_id);

        let setup = prepare_turn(
            &state,
            &session_id,
            Message::user().with_text("continue"),
            None,
            None,
        )
        .await;

        assert!(
            setup.is_some(),
            "the subagent gate must not refuse an ordinary chat"
        );
        let codes = published_error_codes(&mut events);
        assert!(
            !codes
                .iter()
                .any(|code| code.starts_with("subagent_runtime")),
            "the subagent gate refused an ordinary chat: {codes:?}"
        );
    }

    #[tokio::test]
    async fn start_turn_refuses_when_a_turn_is_in_flight() {
        let state = crate::state::AppState::new().await.unwrap();
        let (_workdir, sid) = session(&state, "busy").await;

        let _guard = state
            .try_begin_turn_idempotent(&sid, CancellationToken::new(), None)
            .unwrap();

        let err = start_turn(
            state.clone(),
            TurnRequest::new(sid.clone(), Message::user().with_text("x")),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, TurnStartError::TurnInFlight { .. }));
    }

    #[tokio::test]
    async fn turn_publishes_lifecycle_and_releases_the_lock_even_when_it_fails() {
        let state = crate::state::AppState::new().await.unwrap();
        let (_workdir, sid) = session(&state, "detached").await;

        let mut rx = session_events::subscribe(&sid);
        // No provider on the fresh agent → the turn starts, fails fast, and
        // must still bracket itself on the bus.
        //
        // Reads through this test's OWN subscription rather than the one
        // `start_turn` returns, deliberately: the property under test is that
        // every observer of the session sees the bracket, not just the caller
        // that started the turn.
        let started = start_turn(
            state.clone(),
            TurnRequest::new(sid.clone(), Message::user().with_text("go")),
        )
        .await
        .unwrap();
        assert!(started.turn_id.starts_with("turn-"));

        let first = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("event in time")
            .unwrap();
        assert!(matches!(first, SessionBusEvent::TurnStarted { .. }));

        // Drain the WHOLE turn, not just up to the first terminal. Breaking on
        // the first one asserts "at least one" while the message claims
        // "exactly one" — and the regression this guards against is a runner
        // that publishes BOTH the raw `AgentEvent::TurnAborted` and the
        // classified `TurnError`, which `map_bus_event` renders as two `Error`
        // frames. Step 3's implementation comment forbids exactly that.
        //
        // A short timeout, not `try_recv`: the double-publish puts the two
        // terminals on the bus adjacently but asynchronously, so an immediate
        // `try_recv() == Empty` right after the first proves nothing.
        //
        // The loop exits on anything that is not a delivered event — a timeout
        // or a closed/lagged channel both mean the turn has stopped publishing.
        let mut terminals: Vec<SessionBusEvent> = Vec::new();
        while let Ok(Ok(ev)) =
            tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await
        {
            if matches!(
                ev,
                SessionBusEvent::TurnFinished { .. } | SessionBusEvent::TurnError { .. }
            ) {
                terminals.push(ev);
            }
        }
        assert_eq!(
            terminals.len(),
            1,
            "every turn must publish exactly one terminal event, got {terminals:?}"
        );

        // The turn lock must be released once the task unwinds.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while state.is_turn_active(&sid) {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("turn lock released");
    }

    #[tokio::test]
    async fn idempotency_key_is_forwarded_so_reply_reconnects_still_dedupe() {
        // Task 8 depends on this: /reply forwards the client's turn_id, and a
        // re-POST of the same id must be reported as a duplicate, not a second
        // turn (BR-62).
        let state = crate::state::AppState::new().await.unwrap();
        let (_workdir, sid) = session(&state, "idem").await;

        let request = TurnRequest::new(sid.clone(), Message::user().with_text("a"))
            .with_idempotency_key(Some("client-turn-1".to_string()));
        let _first = start_turn(state.clone(), request).await.unwrap();

        let again = TurnRequest::new(sid.clone(), Message::user().with_text("a"))
            .with_idempotency_key(Some("client-turn-1".to_string()));
        // Deliberately no catch-all arm: `TurnStartError` has exactly one
        // variant today, so an `other => panic!(…)` arm is an
        // `unreachable_patterns` warning and this repo lints with `-D warnings`.
        // An exhaustive match is also the stronger guard — a new variant breaks
        // the build here instead of silently taking a panic arm at runtime.
        match start_turn(state.clone(), again).await {
            Err(TurnStartError::TurnInFlight { duplicate, .. }) => assert!(duplicate),
            // A fast machine may have finished the (provider-less) turn already;
            // then the second call legitimately starts a new turn.
            Ok(_) => {}
        }
    }

    /// ⚠ Reconciliation #21: `run_turn` performs NO whole-history rewrite.
    ///
    /// The property, not a grep: seed a session with a stored message, then run
    /// a turn whose `conversation_so_far` **omits** it. The stored row must
    /// still be there afterwards. Under the pre-amendment plan — which had
    /// `run_turn` call `SessionManager::replace_conversation` on the client's
    /// copy — that row is DELETEd and re-INSERTed away, silently, from a
    /// detached task with no status code to refuse with. That is the #51 W5 bug
    /// `306552fd` shipped a fix for, and `grep -c replace_conversation turn.rs`
    /// cannot tell the two implementations apart.
    ///
    /// This test does not need a provider: the turn fails fast without one, and
    /// the destruction (if any) happens before `agent.reply` is ever reached.
    #[tokio::test]
    async fn a_seed_conversation_is_never_written_back_over_the_store() {
        use biorouter::conversation::Conversation;

        let state = crate::state::AppState::new().await.unwrap();
        let (_workdir, sid) = session(&state, "seed-is-not-a-write").await;

        // One durable row the client's copy will not name.
        let mut stored = Message::user().with_text("br71-seed-probe");
        state
            .session_manager()
            .add_message_adopting_uid(&sid, &mut stored)
            .await
            .unwrap();
        let probe_id = stored.id.clone().expect("the store stamps an id");

        // A seed that pretends the session is empty.
        let request = TurnRequest {
            session_id: sid.clone(),
            user_message: Message::user().with_text("go"),
            extras: TurnExtras {
                conversation_so_far: Some(Conversation::new_unvalidated(Vec::new())),
                ..TurnExtras::default()
            },
        };
        let cancel = CancellationToken::new();
        let guard = state
            .try_begin_turn_idempotent(&sid, cancel.clone(), None)
            .expect("session is idle");
        run_turn(state.clone(), request, guard, cancel).await;

        let reread = state
            .session_manager()
            .get_session(&sid, true)
            .await
            .unwrap();
        let ids: Vec<String> = reread
            .conversation
            .unwrap_or_default()
            .messages()
            .iter()
            .filter_map(|m| m.id.clone())
            .collect();
        assert!(
            ids.contains(&probe_id),
            "run_turn destroyed a stored message it was merely seeded around; \
             the seed is not a write-back (ids: {ids:?})"
        );
    }

    /// Drain the bus into a vec, stopping once nothing more arrives. A short
    /// timeout rather than `try_recv`, because a *second* terminal is published
    /// adjacently but asynchronously — an immediate `try_recv() == Empty` proves
    /// nothing about it.
    async fn drain(rx: &mut session_events::Subscription) -> Vec<SessionBusEvent> {
        let mut seen = Vec::new();
        while let Ok(Ok(ev)) =
            tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await
        {
            seen.push(ev);
        }
        seen
    }

    fn is_terminal(ev: &SessionBusEvent) -> bool {
        matches!(
            ev,
            SessionBusEvent::TurnError { .. } | SessionBusEvent::TurnFinished { .. }
        )
    }

    /// Collect events up to and including the turn's terminal frame, giving up
    /// only at `deadline`.
    ///
    /// ⚠ This is NOT [`drain`], and the difference is why it exists. `drain`
    /// stops after a 200 ms *gap*, which answers "has the bus gone quiet?" —
    /// the right question for a test asserting that nothing more is coming.
    /// Ask it of an event that IS coming and the 200 ms silently becomes a race
    /// against the machine: a turn publishes its terminal only after
    /// `prepare_turn` has awaited the session's extension load and built an
    /// agent (config reads, session I/O), which on a loaded CI runner takes
    /// longer than the gap. The test then reports zero terminals and prints an
    /// empty vector, which reads as a lost event rather than a short wait.
    ///
    /// So wait on the *condition* — the terminal frame — with a deadline no
    /// healthy run comes near. A slow machine makes this slower, never red.
    async fn events_until_terminal(
        rx: &mut session_events::Subscription,
        deadline: std::time::Duration,
    ) -> Vec<SessionBusEvent> {
        let mut seen = Vec::new();
        let collect = async {
            while let Ok(ev) = rx.recv().await {
                let terminal = is_terminal(&ev);
                seen.push(ev);
                if terminal {
                    break;
                }
            }
        };
        let _ = tokio::time::timeout(deadline, collect).await;
        seen
    }

    /// The abort path end-to-end through the stream loop, which no test could
    /// reach before `drive_stream` was split out: `run_turn` needs a provider to
    /// get this far, and a provider-less session exits via
    /// `inference_start_failed` long before.
    ///
    /// Two properties, and the second is the one the implementation comment
    /// spends a paragraph on: an abort is republished CLASSIFIED, and is **not
    /// also forwarded raw**. `map_bus_event` renders both
    /// `SessionBusEvent::TurnError` and `AgentEvent::TurnAborted` as
    /// `MessageEvent::Error`, so forwarding both gives every consumer two
    /// terminal error frames for one abort.
    #[tokio::test]
    async fn an_abort_is_republished_classified_and_never_also_raw() {
        use biorouter::agents::TurnAbortCode;
        use biorouter::providers::errors::ProviderErrorKind;

        let sid = "br71-drive-stream-abort";
        let mut rx = session_events::subscribe(sid);
        let mut all = Conversation::new_unvalidated(Vec::new());
        let mut stream = futures::stream::iter(vec![Ok(AgentEvent::TurnAborted {
            code: TurnAbortCode::ProviderFailure {
                kind: ProviderErrorKind::RateLimit,
            },
            message: "slow down".to_string(),
        })]);

        let terminal_error =
            drive_stream(sid, &mut stream, &CancellationToken::new(), &mut all, None).await;
        assert!(terminal_error, "an abort ends the turn on an error");

        let seen = drain(&mut rx).await;
        assert!(
            !seen
                .iter()
                .any(|ev| matches!(ev, SessionBusEvent::Agent(AgentEvent::TurnAborted { .. }))),
            "the raw abort must not also reach the bus: {seen:?}"
        );
        let terminals: Vec<_> = seen
            .iter()
            .filter(|ev| {
                matches!(
                    ev,
                    SessionBusEvent::TurnError { .. } | SessionBusEvent::TurnFinished { .. }
                )
            })
            .collect();
        assert_eq!(terminals.len(), 1, "exactly one terminal: {seen:?}");
        let SessionBusEvent::TurnError {
            message,
            code,
            scope,
            retryable,
            provider_kind,
        } = terminals[0]
        else {
            panic!("expected a TurnError, got {:?}", terminals[0]);
        };
        assert_eq!(message, "slow down");
        assert_eq!(
            code,
            TurnAbortCode::ProviderFailure {
                kind: ProviderErrorKind::RateLimit
            }
            .wire_code()
        );
        // The full envelope, not a collapsed one: these three fields are what
        // the desktop's rate-limit recovery keys off.
        assert_eq!(scope, "provider");
        assert!(*retryable);
        assert_eq!(
            provider_kind.as_deref(),
            Some(ProviderErrorKind::RateLimit.wire_code())
        );
    }

    #[tokio::test]
    async fn provider_abort_keeps_private_details_off_diagnostics_but_on_bus() {
        use biorouter::agents::TurnAbortCode;
        use biorouter::providers::errors::ProviderErrorKind;
        use tracing::instrument::WithSubscriber;

        const SENTINEL: &str = "PRIVATE_PROVIDER_ABORT_2f58c9";
        let sid = "br71-private-provider-abort-log";
        let mut rx = session_events::subscribe(sid);
        let mut all = Conversation::new_unvalidated(Vec::new());
        let mut stream = futures::stream::iter(vec![Ok(AgentEvent::TurnAborted {
            code: TurnAbortCode::ProviderFailure {
                kind: ProviderErrorKind::InvalidRequest,
            },
            message: format!("Bad request from private provider: {SENTINEL}"),
        })]);
        let capture = DiagnosticCapture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_target(true)
            .with_level(true)
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || writer.clone())
            .finish();

        let terminal_error =
            drive_stream(sid, &mut stream, &CancellationToken::new(), &mut all, None)
                .with_subscriber(subscriber)
                .await;
        assert!(terminal_error, "a provider abort ends the turn on an error");

        let seen = drain(&mut rx).await;
        let terminal = seen
            .iter()
            .find(|event| matches!(event, SessionBusEvent::TurnError { .. }))
            .expect("the user-facing bus must retain the classified provider error");
        let SessionBusEvent::TurnError {
            message,
            code,
            scope,
            retryable,
            provider_kind,
        } = terminal
        else {
            unreachable!();
        };
        assert!(message.contains(SENTINEL));
        assert_eq!(code, "provider_failure");
        assert_eq!(scope, "provider");
        assert!(!retryable);
        assert_eq!(provider_kind.as_deref(), Some("invalid_request"));

        let diagnostics = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(diagnostics.contains("ERROR"));
        assert!(diagnostics.contains("workspace::turn"));
        assert!(diagnostics.lines().any(|line| {
            line.contains("Turn aborted")
                && line.contains("provider_failure")
                && line.contains("provider")
                && line.contains("invalid_request")
                && line.contains("retryable=false")
        }));
        assert!(
            !diagnostics.contains(SENTINEL),
            "private provider abort detail leaked into diagnostics: {diagnostics}"
        );
    }

    #[tokio::test]
    async fn drive_stream_preserves_message_before_persisted_ids_order() {
        use biorouter::agents::PersistedMessage;

        let sid = "br71-drive-stream-persist-order";
        let message_id = "br71-persisted-message";
        let mut rx = session_events::subscribe(sid);
        let mut all = Conversation::new_unvalidated(Vec::new());
        let message = Message::assistant()
            .with_text("durable answer")
            .with_id(message_id);
        let mut stream = futures::stream::iter(vec![
            Ok(AgentEvent::Message(message)),
            Ok(AgentEvent::MessagesPersisted(vec![PersistedMessage {
                id: message_id.to_string(),
                user_visible: true,
            }])),
        ]);

        let terminal_error =
            drive_stream(sid, &mut stream, &CancellationToken::new(), &mut all, None).await;
        assert!(!terminal_error);

        let seen = drain(&mut rx).await;
        assert_eq!(seen.len(), 2, "both producer events must reach the bus");
        let SessionBusEvent::Agent(AgentEvent::Message(message)) = &seen[0] else {
            panic!("message body must be published first: {seen:?}");
        };
        assert_eq!(message.id.as_deref(), Some(message_id));
        let SessionBusEvent::Agent(AgentEvent::MessagesPersisted(persisted)) = &seen[1] else {
            panic!("persisted-id accounting must follow the body: {seen:?}");
        };
        assert_eq!(persisted[0].id, message_id);
    }

    /// A turn that dies without publishing a terminal still gets one.
    ///
    /// `run_turn` publishes `TurnStarted` before any fallible work, and
    /// `tokio::spawn` swallows a panic into a `JoinHandle`. Drop that handle
    /// and a turn that panics anywhere afterwards — in the agent stream, in
    /// `track_tool_telemetry`, in a `Conversation` op — publishes a start and
    /// then nothing, forever: the module's "one terminal event per turn,
    /// always" becomes zero, and every observer (Task 7's watcher, Task 14's
    /// `wait:"final_message"`) blocks on a frame that will never come.
    ///
    /// `/reply` has had exactly this backstop since BR-33 ("Reply task
    /// terminated unexpectedly" → `internal_error`), but a *detached* turn has
    /// no handler to supervise it, so the runner has to carry its own.
    #[tokio::test]
    async fn a_turn_that_panics_still_publishes_a_terminal_event() {
        struct AcquireSuccessorOnDrop {
            state: Arc<crate::state::AppState>,
            session_id: String,
            acquired: Arc<std::sync::atomic::AtomicBool>,
        }

        impl Drop for AcquireSuccessorOnDrop {
            fn drop(&mut self) {
                if self
                    .state
                    .try_begin_turn_idempotent(&self.session_id, CancellationToken::new(), None)
                    .is_ok()
                {
                    self.acquired
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
        }

        let state = crate::state::AppState::new().await.unwrap();
        let sid = "br71-supervised-panic".to_string();
        let mut rx = session_events::subscribe(&sid);
        let guard = state
            .try_begin_turn_idempotent(&sid, CancellationToken::new(), None)
            .expect("session is idle");
        let successor_acquired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let probe = AcquireSuccessorOnDrop {
            state: state.clone(),
            session_id: sid.clone(),
            acquired: successor_acquired.clone(),
        };

        let handle = tokio::spawn(supervise_turn(
            sid.clone(),
            guard,
            Arc::new(AtomicU8::new(TURN_OUTCOME_NONE)),
            async move {
                let _probe = probe;
                panic!("br71: deliberate panic, this backtrace is expected");
            },
        ));

        let ev = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("a panicking turn must still be bracketed")
            .unwrap();
        let SessionBusEvent::TurnError {
            code,
            scope,
            retryable,
            ..
        } = &ev
        else {
            panic!("expected a TurnError, got {ev:?}");
        };
        assert_eq!(code, "internal_error");
        assert_eq!(scope, TurnErrorScope::Internal.wire_value());
        assert!(retryable, "the client may retry a turn that fell over");
        handle.await.unwrap();
        assert!(
            !successor_acquired.load(std::sync::atomic::Ordering::SeqCst),
            "the session guard must survive the panicking body's unwind"
        );
        assert!(
            !state.is_turn_active(&sid),
            "the guard is released after the supervisor publishes the terminal"
        );
    }

    #[tokio::test]
    async fn panic_after_exact_child_outcome_preserves_result_and_finishes_cleanly() {
        let state = crate::state::AppState::new().await.unwrap();
        let sid = "br71-panic-after-child-outcome".to_string();
        let handle = biorouter::agents::subagent_handle::BackgroundSubagent::register(
            "br71-panic-after-child-outcome-parent",
            sid.clone(),
            "delegated work",
            CancellationToken::new(),
        );
        biorouter::agents::subagent_handle::begin_child_turn(&sid);
        let generation = handle.child_turn_generation();
        let mut rx = session_events::subscribe(&sid);
        let guard = state
            .try_begin_turn_idempotent(&sid, CancellationToken::new(), None)
            .expect("session is idle");
        let outcome = Arc::new(AtomicU8::new(TURN_OUTCOME_NONE));
        let turn_outcome = Arc::clone(&outcome);
        let turn_sid = sid.clone();

        supervise_turn(sid, guard, outcome, async move {
            let messages = Conversation::new_unvalidated([
                Message::user().with_text("finish the delegated work"),
                Message::assistant().with_text("PROVEN_CHILD_RESULT"),
            ]);
            record_child_turn_outcome(&turn_sid, false, false, &messages, 0, &turn_outcome);
            panic!("deliberate panic after exact child outcome");
        })
        .await;

        let terminal = handle
            .terminal_generation()
            .expect("the exact child result was lost");
        assert_eq!(terminal.generation, generation);
        assert_eq!(terminal.result.summary, "PROVEN_CHILD_RESULT");
        assert!(terminal.result.error.is_none());
        let seen = drain(&mut rx).await;
        assert_eq!(
            seen.len(),
            1,
            "the supervisor published a false error: {seen:?}"
        );
        assert!(matches!(
            &seen[0],
            SessionBusEvent::TurnFinished { reason, .. } if reason == "stop"
        ));
    }

    /// A turn that ends normally must NOT also get the supervisor's terminal —
    /// otherwise every turn would publish two.
    #[tokio::test]
    async fn a_turn_that_ends_cleanly_gets_no_extra_terminal() {
        let state = crate::state::AppState::new().await.unwrap();
        let sid = "br71-supervised-clean".to_string();
        let mut rx = session_events::subscribe(&sid);
        let guard = state
            .try_begin_turn_idempotent(&sid, CancellationToken::new(), None)
            .expect("session is idle");
        let publish_session_id = sid.clone();

        supervise_turn(
            sid,
            guard,
            Arc::new(AtomicU8::new(TURN_OUTCOME_NONE)),
            async move {
                session_events::publish(
                    &publish_session_id,
                    SessionBusEvent::TurnFinished {
                        reason: "stop".into(),
                        token_state: None,
                    },
                );
            },
        )
        .await;

        let seen = drain(&mut rx).await;
        assert_eq!(seen.len(), 1, "the supervisor must stay quiet: {seen:?}");
        assert!(matches!(seen[0], SessionBusEvent::TurnFinished { .. }));
    }

    #[tokio::test]
    async fn run_turn_refuses_a_guard_for_another_session_in_all_builds() {
        let state = crate::state::AppState::new().await.unwrap();
        let first_sid = "br71-guard-owner".to_string();
        let second_sid = "br71-guard-mismatch".to_string();
        let mut second_events = session_events::subscribe(&second_sid);
        let guard = state
            .try_begin_turn_idempotent(&first_sid, CancellationToken::new(), None)
            .expect("first session is idle");

        run_turn(
            state.clone(),
            TurnRequest::new(
                second_sid.clone(),
                Message::user().with_text("must not run"),
            ),
            guard,
            CancellationToken::new(),
        )
        .await;

        assert!(!state.is_turn_active(&first_sid));
        assert!(!state.is_turn_active(&second_sid));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), second_events.recv(),)
                .await
                .is_err(),
            "the unguarded session must not publish a turn lifecycle"
        );
    }

    #[tokio::test]
    async fn replies_into_subagent_sessions_are_stamped_user_direct() {
        // The pure stamping helper is what we assert; the full /reply path
        // exercises it via the session_type read it already performs.
        use biorouter::conversation::message::ProvenanceKind;
        let stamped =
            stamp_user_direct_if_subagent(Message::user().with_text("hi"), SessionType::SubAgent);
        assert_eq!(
            stamped.metadata.provenance.as_ref().unwrap().kind,
            ProvenanceKind::UserDirect
        );
        let untouched =
            stamp_user_direct_if_subagent(Message::user().with_text("hi"), SessionType::User);
        assert!(untouched.metadata.provenance.is_none());
    }

    /// `run_turn` is the ONE turn runner, so `workspace_send_prompt mode:"turn"`
    /// lands here as well — and its message is already stamped
    /// `AgentInjection` by the time it arrives. Relabelling it `UserDirect`
    /// because the target is a subagent would report `human_intervened: true`
    /// to the parent for a run no human touched.
    #[tokio::test]
    async fn an_agent_injection_into_a_subagent_keeps_its_own_provenance() {
        use biorouter::conversation::message::{MessageProvenance, ProvenanceKind};
        let injected = Message::user()
            .with_text("from the parent")
            .with_provenance(MessageProvenance {
                kind: ProvenanceKind::AgentInjection,
                from_session_id: Some("s-parent".into()),
                from_session_name: Some("Planning chat".into()),
            });
        let stamped = stamp_user_direct_if_subagent(injected, SessionType::SubAgent);
        let provenance = stamped.metadata.provenance.as_ref().unwrap();
        assert_eq!(provenance.kind, ProvenanceKind::AgentInjection);
        assert_eq!(provenance.from_session_id.as_deref(), Some("s-parent"));
    }

    /// `start_turn` must hand back a subscription it opened itself, not leave
    /// the caller to race the turn it just spawned.
    ///
    /// `session_events::publish` is a pure LOOKUP and a no-op when the session
    /// has no ring — `subscribe` is the only thing that creates one. So the
    /// natural `let id = start_turn(..).await?; let rx = subscribe(&sid);`
    /// misses `TurnStarted`, and on a fast-failing turn (the provider-less path
    /// takes microseconds) misses the terminal as well; the caller then blocks
    /// forever on a `recv()` for events that were dropped on the floor. The
    /// daemon is `#[tokio::main]` multi-thread, so the spawned task genuinely
    /// can run before the caller's next statement.
    ///
    /// The real guard is the signature — a caller cannot express the race any
    /// more. This test is the end-to-end check that the subscription it returns
    /// really does carry the whole lifecycle, and it runs on a multi-thread
    /// runtime because a current-thread one cannot poll the spawned task until
    /// the test awaits and would hide the problem entirely.
    #[tokio::test(flavor = "multi_thread")]
    async fn start_turn_hands_back_a_subscription_that_cannot_have_missed_anything() {
        let state = crate::state::AppState::new().await.unwrap();
        let (_workdir, sid) = session(&state, "subscription-not-a-race").await;

        // Deliberately no `subscribe` of our own: the returned subscription is
        // the only way this test can see the bus.
        let mut started = start_turn(
            state.clone(),
            TurnRequest::new(sid.clone(), Message::user().with_text("go")),
        )
        .await
        .unwrap();
        assert!(started.turn_id.starts_with("turn-"));

        let first = tokio::time::timeout(std::time::Duration::from_secs(5), started.events.recv())
            .await
            .expect("TurnStarted must not have been published before we could listen")
            .unwrap();
        assert!(matches!(first, SessionBusEvent::TurnStarted { .. }));

        // And the terminal. ⚠ Wait for the frame itself, never for the bus to
        // fall quiet for 200 ms: this path is only "microseconds" on an idle
        // machine, and `prepare_turn` awaits the session's extension load and
        // builds an agent before it can fail. Under CI contention that
        // outran a `drain`, which then reported zero terminals and an empty
        // vector — indistinguishable from the lost-event bug this test exists
        // to catch.
        let mut lifecycle =
            events_until_terminal(&mut started.events, std::time::Duration::from_secs(30)).await;
        // Only now is silence meaningful: a second terminal would be an extra
        // publish, and by here the first has already arrived.
        lifecycle.extend(drain(&mut started.events).await);

        let terminals: Vec<_> = lifecycle.into_iter().filter(is_terminal).collect();
        assert_eq!(terminals.len(), 1, "exactly one terminal: {terminals:?}");
    }

    /// Cancelling a turn must end it even when the agent is not yielding.
    ///
    /// `/reply`'s loop is a `select!` whose first arm is
    /// `_ = task_cancel.cancelled() => break`, so tripping the token ends the
    /// turn within one iteration *regardless of what the agent is doing*. A
    /// bare `while let Some(item) = stream.next().await` can only end when the
    /// agent yields, and `AppState::cancel_turn` deliberately does NOT remove
    /// the map entry — it relies on the `TurnGuard` clearing the slot as the
    /// turn task unwinds. So an agent parked inside a provider call that never
    /// observes the token would hold the session's turn lock forever and every
    /// later `/reply` would 409 until the daemon restarted. This codebase has
    /// already shipped that failure once (the Versa/Bedrock no-timeout freeze).
    /// It also neuters `ActiveWorkKind::DetachedTurn`'s cancel closure, whose
    /// only effect is to trip this same token.
    ///
    /// `stream::pending()` is the agent that never yields.
    #[tokio::test]
    async fn a_cancelled_turn_escapes_a_stream_that_never_yields() {
        let sid = "br71-drive-stream-cancel";
        let mut rx = session_events::subscribe(sid);
        let mut all = Conversation::new_unvalidated(Vec::new());
        let mut stream = futures::stream::pending::<anyhow::Result<AgentEvent>>();

        let cancel = CancellationToken::new();
        let trip = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            trip.cancel();
        });

        let terminal_error = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            drive_stream(sid, &mut stream, &cancel, &mut all, None),
        )
        .await
        .expect("a cancelled turn must escape a stalled agent stream");

        // Cancellation is not an error terminal: `/reply` breaks with
        // `terminal_error` still false and finishes with `reason: "cancelled"`,
        // and `run_turn` reads the token for the same decision.
        assert!(!terminal_error, "cancellation is not a terminal error");
        let seen = drain(&mut rx).await;
        assert!(
            seen.is_empty(),
            "the loop publishes nothing of its own on cancel: {seen:?}"
        );
    }

    #[tokio::test]
    #[serial]
    async fn cancellation_settles_an_accepted_interrupt_before_dropping_the_stream() {
        use biorouter::agents::{Agent, AgentConfig, InterruptRefused, TurnId};
        use biorouter::config::permission::PermissionManager;
        use biorouter::config::BioRouterMode;
        use biorouter::conversation::message::{MessageProvenance, ProvenanceKind};
        use biorouter::session::SessionManager;

        let data_dir = tempfile::TempDir::new().unwrap();
        let work_dir = tempfile::TempDir::new().unwrap();
        let session_manager = Arc::new(SessionManager::new(data_dir.path().to_path_buf()));
        let session = session_manager
            .create_session(
                work_dir.path().to_path_buf(),
                "cancelled accepted interrupt".into(),
                SessionType::Hidden,
            )
            .await
            .unwrap();
        let agent = Agent::with_config(AgentConfig::new(
            Arc::clone(&session_manager),
            PermissionManager::instance(),
            None,
            BioRouterMode::Auto,
        ));
        agent.open_for_turn(TurnId::new("turn-cancel-settlement"));
        let provenance = MessageProvenance {
            kind: ProvenanceKind::UserDirect,
            from_session_id: Some("parent-session".into()),
            from_session_name: Some("Parent".into()),
        };
        agent
            .try_queue_soft_interrupt("please preserve this".into(), Some(provenance.clone()))
            .expect("the running turn must accept the interrupt");

        let mut rx = session_events::subscribe(&session.id);
        let mut all = Conversation::new_unvalidated(Vec::new());
        let mut stream = futures::stream::pending::<anyhow::Result<AgentEvent>>();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let terminal_error =
            drive_stream(&session.id, &mut stream, &cancel, &mut all, Some(&agent)).await;

        assert!(!terminal_error);
        assert_eq!(all.len(), 1);
        assert_eq!(all.messages()[0].as_concat_text(), "please preserve this");
        assert_eq!(
            all.messages()[0].metadata.provenance,
            Some(provenance.clone())
        );
        let seen = drain(&mut rx).await;
        assert!(matches!(
            seen.as_slice(),
            [SessionBusEvent::Agent(AgentEvent::Message(message))]
                if message.as_concat_text() == "please preserve this"
                    && message.metadata.provenance == Some(provenance.clone())
        ));

        let stored = session_manager
            .get_session(&session.id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        assert_eq!(stored.messages().len(), 1);
        assert_eq!(stored.messages()[0].metadata.provenance, Some(provenance));
        assert!(agent
            .settle_carried_over_soft_interrupts(&session.id)
            .await
            .unwrap()
            .is_empty());
        assert!(matches!(
            agent.try_queue_soft_interrupt("too late".into(), None),
            Err(InterruptRefused::TurnEnded)
        ));
    }

    #[tokio::test]
    #[serial]
    async fn a_stream_error_settles_an_accepted_interrupt_before_the_error_terminal() {
        use biorouter::agents::{Agent, AgentConfig, InterruptRefused, TurnId};
        use biorouter::config::permission::PermissionManager;
        use biorouter::config::BioRouterMode;
        use biorouter::conversation::message::{MessageProvenance, ProvenanceKind};
        use biorouter::session::SessionManager;

        let data_dir = tempfile::TempDir::new().unwrap();
        let work_dir = tempfile::TempDir::new().unwrap();
        let session_manager = Arc::new(SessionManager::new(data_dir.path().to_path_buf()));
        let session = session_manager
            .create_session(
                work_dir.path().to_path_buf(),
                "errored accepted interrupt".into(),
                SessionType::Hidden,
            )
            .await
            .unwrap();
        let agent = Agent::with_config(AgentConfig::new(
            Arc::clone(&session_manager),
            PermissionManager::instance(),
            None,
            BioRouterMode::Auto,
        ));
        let delegated = biorouter::agents::subagent_handle::BackgroundSubagent::register(
            &session.id,
            "stream-error-child",
            "stream error child",
            CancellationToken::new(),
        );
        biorouter::agents::subagent_handle::begin_child_turn("stream-error-child");
        delegated.complete(biorouter::agents::SubagentResult::from_error(
            "terminal under stream-error supervision",
        ));
        agent.open_for_turn(TurnId::new("turn-error-settlement"));
        let provenance = MessageProvenance {
            kind: ProvenanceKind::UserDirect,
            from_session_id: Some("parent-session".into()),
            from_session_name: Some("Parent".into()),
        };
        agent
            .try_queue_soft_interrupt("preserve before error".into(), Some(provenance.clone()))
            .expect("the running turn must accept the interrupt");

        let mut rx = session_events::subscribe(&session.id);
        let mut all = Conversation::new_unvalidated(Vec::new());
        let mut stream = futures::stream::iter(vec![Err(anyhow::anyhow!("provider hung up"))]);
        let terminal_error = drive_stream(
            &session.id,
            &mut stream,
            &CancellationToken::new(),
            &mut all,
            Some(&agent),
        )
        .await;

        assert!(terminal_error);
        assert_eq!(all.len(), 2);
        assert_eq!(all.messages()[0].as_concat_text(), "preserve before error");
        assert_eq!(all.messages()[0].metadata.provenance, Some(provenance));
        assert!(all.messages()[1]
            .as_concat_text()
            .contains("terminal under stream-error supervision"));
        let seen = drain(&mut rx).await;
        assert!(
            matches!(
                seen.as_slice(),
                [
                    SessionBusEvent::Agent(AgentEvent::Message(interrupt)),
                    SessionBusEvent::Agent(AgentEvent::Message(delegated_result)),
                    SessionBusEvent::TurnError { code, .. }
                ] if interrupt.as_concat_text() == "preserve before error"
                    && delegated_result.as_concat_text().contains("terminal under stream-error supervision")
                    && code == "inference_error"
            ),
            "unexpected bus events: {seen:#?}"
        );
        let stored = session_manager
            .get_session(&session.id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        assert_eq!(stored.messages().len(), 2);
        assert!(delegated.latest_generation_collected());
        assert!(matches!(
            agent.try_queue_soft_interrupt("too late".into(), None),
            Err(InterruptRefused::TurnEnded)
        ));
    }

    /// A mid-stream inference failure keeps `/reply`'s wire code.
    ///
    /// `"stream_error"` is not a free string: the desktop mints it itself for a
    /// dead SSE socket (`chatStreamStore.tsx`, `clientTurnError(error,
    /// 'stream_error', 'transport')`) and reserves it in `ChatTurnError`'s
    /// `MIDSTREAM_CODES`. Publishing it here for a *model* failure would make
    /// one code name two unrelated things — a provider/inference failure and a
    /// dropped connection — separable only by `scope`, and would silently
    /// re-bucket every log and dashboard keyed on `code` the moment Task 8
    /// moves `/reply` onto this runner. The handler's own arm for the identical
    /// `Ok(Some(Err(e)))` case publishes `"inference_error"`; so does this one.
    #[tokio::test]
    async fn a_mid_stream_failure_keeps_the_handlers_inference_error_code() {
        let sid = "br71-drive-stream-error";
        let mut rx = session_events::subscribe(sid);
        let mut all = Conversation::new_unvalidated(Vec::new());
        let mut stream = futures::stream::iter(vec![Err(anyhow::anyhow!("provider hung up"))]);

        let terminal_error =
            drive_stream(sid, &mut stream, &CancellationToken::new(), &mut all, None).await;
        assert!(terminal_error, "a stream error ends the turn on an error");

        let seen = drain(&mut rx).await;
        assert_eq!(seen.len(), 1, "exactly one frame: {seen:?}");
        let SessionBusEvent::TurnError { code, scope, .. } = &seen[0] else {
            panic!("expected a TurnError, got {:?}", seen[0]);
        };
        assert_eq!(
            code, "inference_error",
            "`stream_error` is the desktop's own code for a dead SSE socket"
        );
        assert_eq!(scope, "inference");
    }

    /// A defaulted `TurnExtras` must not put an EMPTY string in the
    /// `session_type` telemetry dimension.
    ///
    /// The label is written verbatim into `session_type` on three counters
    /// (`session_completions`, `session_duration_ms`, `session_tokens`). Task 8
    /// builds `TurnExtras` by struct literal for `/reply`, so a field that
    /// defaults to nothing means one forgotten line makes
    /// `session_completions{session_type=""}` appear while the existing
    /// `session_type="app"` series silently loses the volume — defeating the
    /// only reason this derived copy of the metrics block exists. A type with
    /// no meaningless value cannot express that, and the default is the
    /// interactive turn `/reply` reports today.
    #[test]
    fn the_default_turn_kind_is_the_one_reply_already_reports() {
        assert_eq!(TurnKind::default(), TurnKind::Interactive);
        assert_eq!(TurnExtras::default().kind.session_type(), "app");
        assert_eq!(TurnKind::Workspace.session_type(), "workspace");
        // …and the workspace constructor still labels itself.
        let request = TurnRequest::new("s".to_string(), Message::user().with_text("x"));
        assert_eq!(request.extras.kind, TurnKind::Workspace);
    }

    /// Reconciliation #9: the terminal-error CLASSIFIER moved out of `/reply`
    /// with its fidelity intact. This is the test that stops the refactor from
    /// silently collapsing every abort to `(Inference, false, None)` — which
    /// would delete the desktop's rate-limit/retry/compaction recovery, because
    /// `scope:"provider"`, `retryable:true` and `provider_kind` are exactly what
    /// it keys off and nothing else in the process emits them.
    #[test]
    fn abort_codes_classify_exactly_as_the_pre_refactor_handler_did() {
        use biorouter::agents::TurnAbortCode;
        use biorouter::providers::errors::ProviderErrorKind;

        // A transient provider failure: scope provider, retryable, kind named.
        let (scope, retryable, kind) = classify_abort(&TurnAbortCode::ProviderFailure {
            kind: ProviderErrorKind::RateLimit,
        });
        assert_eq!(scope, TurnErrorScope::Provider);
        assert!(
            retryable,
            "a rate limit is transient: the client retries it"
        );
        assert_eq!(
            kind.as_deref(),
            Some(ProviderErrorKind::RateLimit.wire_code())
        );

        // A non-transient one: still provider-scoped, still named, not retryable.
        let (scope, retryable, kind) = classify_abort(&TurnAbortCode::ProviderFailure {
            kind: ProviderErrorKind::Auth,
        });
        assert_eq!(scope, TurnErrorScope::Provider);
        assert!(!retryable, "a bad credential never succeeds on retry");
        assert_eq!(kind.as_deref(), Some("auth"));

        // #31/#41: a local store failure is NOT the provider's fault.
        assert_eq!(
            classify_abort(&TurnAbortCode::SessionStore),
            (TurnErrorScope::Session, false, None)
        );
        assert_eq!(
            classify_abort(&TurnAbortCode::ToolLoop {
                tool: "shell".into()
            }),
            (TurnErrorScope::Inference, false, None)
        );
        assert_eq!(
            classify_abort(&TurnAbortCode::WorkerTimeout {
                agent: "reviewer".into(),
                elapsed_s: 90,
            }),
            (TurnErrorScope::Inference, true, None)
        );
    }

    /// The two signed aborts must not classify alike. A dropped connection
    /// mid-tool-call is transient and the agent rolled the turn back, so the
    /// client gets a Retry; signed history that cannot be replayed unchanged has
    /// nowhere to resume from and must not.
    #[test]
    fn only_the_rolled_back_signed_abort_is_retryable() {
        use biorouter::agents::TurnAbortCode;

        assert_eq!(
            classify_abort(&TurnAbortCode::SignedStreamTruncated),
            (TurnErrorScope::Inference, true, None)
        );
        assert_eq!(
            classify_abort(&TurnAbortCode::SignedReplayInvalidated),
            (TurnErrorScope::Inference, false, None)
        );
    }

    /// **`workspace_send_prompt mode:"turn"` reflects in an open tab live.**
    ///
    /// The injected prompt is persisted by `Agent::reply` like any other user
    /// message, and `Agent::reply` deliberately does NOT yield a `Message` frame
    /// for a user prompt — #66's rule, on the premise that "the client authored
    /// it and already holds it". That premise is false for an injection: the
    /// target's tab authored nothing, so with no frame carrying the body the
    /// message appears only after a reload. `Agent::reply` therefore publishes
    /// an agent-injected row straight onto the session bus, at the point it
    /// becomes durable.
    ///
    /// Driven through the real `run_turn` against a provider that RESTORES but
    /// cannot infer (`ollama`, naming a model that is not there). Getting past
    /// provider restore is what matters: a name the factory does not know fails
    /// in setup, before `Agent::reply` runs at all, so the prompt is never
    /// persisted and the test would pass for the wrong reason. Failing at
    /// inference instead is the point — the publish must already have happened,
    /// because that is where the row became durable.
    #[tokio::test]
    async fn an_injected_turns_prompt_reaches_the_targets_open_tab_live() {
        use biorouter::conversation::message::{MessageProvenance, ProvenanceKind};

        const INJECTED: &str = "br71-injected-turn-live-marker";

        let state = crate::state::AppState::new().await.unwrap();
        let workdir = tempfile::TempDir::new().unwrap();
        let target = state
            .session_manager()
            .create_session(
                workdir.path().to_path_buf(),
                "br71 injected turn live".into(),
                SessionType::User,
            )
            .await
            .unwrap();
        state
            .session_manager()
            .update(&target.id)
            .provider_name("ollama")
            .model_config(biorouter::model::ModelConfig::new("br71-turn-model").unwrap())
            .apply()
            .await
            .unwrap();

        // An observer, subscribed BEFORE the turn — the bus is a broadcast with
        // no replay, so a subscription opened afterwards proves nothing.
        let mut observer = biorouter::session_events::subscribe(&target.id);

        let cancel = CancellationToken::new();
        let guard = state
            .try_begin_turn_idempotent(&target.id, cancel.clone(), None)
            .expect("the target is idle");
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            run_turn(
                state,
                TurnRequest::new(
                    target.id.clone(),
                    Message::user()
                        .with_text(INJECTED)
                        .with_provenance(MessageProvenance {
                            kind: ProvenanceKind::AgentInjection,
                            from_session_id: Some("br71-injecting-caller".into()),
                            from_session_name: Some("the other chat".into()),
                        }),
                ),
                guard,
                cancel,
            ),
        )
        .await;

        let mut saw_body = false;
        while let Ok(event) = observer.try_recv() {
            if let SessionBusEvent::Agent(biorouter::agents::AgentEvent::Message(m)) = event {
                if !message_text_contains(&m, INJECTED) {
                    continue;
                }
                // The DURABLE row, not a pre-write copy: `add_message_adopting_uid`
                // stamps the minted uid, and a frame with `id: None` is one the
                // renderer cannot reconcile against the stored twin that arrives
                // with the next snapshot.
                assert!(
                    m.id.is_some(),
                    "the injected prompt was published before it was durable"
                );
                saw_body = true;
            }
        }
        assert!(
            saw_body,
            "no Message frame carried the injected prompt, so an open tab would \
             show it only after a reload"
        );
    }

    /// The control for the test above, and the thing that keeps it from being
    /// vacuous: an ORDINARY user prompt still gets no `Message` frame. #66's
    /// ordering rule depends on that — the client's own prompt is named by
    /// `MessagesPersisted` and never yielded — so a publish that fired for every
    /// user message would be a regression wearing the same green tick.
    #[tokio::test]
    async fn an_ordinary_user_prompt_is_still_never_published_as_a_message() {
        const TYPED: &str = "br71-ordinary-prompt-marker";

        let state = crate::state::AppState::new().await.unwrap();
        let workdir = tempfile::TempDir::new().unwrap();
        let target = state
            .session_manager()
            .create_session(
                workdir.path().to_path_buf(),
                "br71 ordinary prompt".into(),
                SessionType::User,
            )
            .await
            .unwrap();
        state
            .session_manager()
            .update(&target.id)
            .provider_name("ollama")
            .model_config(biorouter::model::ModelConfig::new("br71-turn-model").unwrap())
            .apply()
            .await
            .unwrap();

        let mut observer = biorouter::session_events::subscribe(&target.id);

        let cancel = CancellationToken::new();
        let guard = state
            .try_begin_turn_idempotent(&target.id, cancel.clone(), None)
            .expect("the target is idle");
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            run_turn(
                state,
                TurnRequest::new(target.id.clone(), Message::user().with_text(TYPED)),
                guard,
                cancel,
            ),
        )
        .await;

        while let Ok(event) = observer.try_recv() {
            if let SessionBusEvent::Agent(biorouter::agents::AgentEvent::Message(m)) = event {
                assert!(
                    !message_text_contains(&m, TYPED),
                    "an ordinary typed prompt was published as a Message frame; \
                     #66's ordering rule assumes it never is"
                );
            }
        }
    }

    fn message_text_contains(
        message: &biorouter::conversation::message::Message,
        needle: &str,
    ) -> bool {
        message
            .content
            .iter()
            .filter_map(|c| c.as_text())
            .any(|t| t.contains(needle))
    }
}
