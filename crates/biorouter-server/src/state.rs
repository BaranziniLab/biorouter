use axum::http::StatusCode;
use biorouter::execution::manager::AgentManager;
use biorouter::scheduler_trait::SchedulerTrait;
use biorouter::session::SessionManager;
use biorouter_mcp::knowledge::service::KnowledgeService;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::tunnel::TunnelManager;
use crate::turn_stream::TurnStream;
use biorouter::agents::ExtensionLoadResult;

type ExtensionLoadingTasks =
    Arc<Mutex<HashMap<String, Arc<Mutex<Option<JoinHandle<Vec<ExtensionLoadResult>>>>>>>>;

/// Process-wide monotonic turn id source. Ids are only used to identify the
/// in-flight turn a rejected concurrent `/reply` collided with.
static TURN_SEQ: AtomicU64 = AtomicU64::new(1);

/// How long a FINISHED turn's entry is kept so a re-POST of its idempotency key
/// attaches to its replay instead of starting a second turn.
///
/// The window that matters is the same one the orphan reaper is sized for: a
/// renderer that reloads (~4.6 s) while the turn is completing must re-POST into
/// the finished turn's replay, not into a fresh turn that spends the tokens
/// again. Five minutes matches
/// [`crate::turn_stream::DEFAULT_ORPHAN_TIMEOUT`], so "the turn is still
/// addressable by its key" has ONE duration rather than two that can disagree.
/// The retained log is trimmed on close (see `CLOSED_REPLAY_BYTE_BUDGET`), so
/// the memory cost of the window is bounded per session.
const FINISHED_TURN_RETENTION: Duration = Duration::from_secs(300);

/// The turn currently in flight for a session — or, briefly, the one that just
/// ended (see [`FINISHED_TURN_RETENTION`]).
#[derive(Debug, Clone)]
struct ActiveTurn {
    /// Server-assigned id, surfaced to a client whose `/reply` was rejected.
    turn_id: String,
    /// The client's idempotency key for this turn, if it sent one (BR-62). Lets
    /// a re-POST of the *same* turn — an SSE reconnect, a retried fetch — be
    /// recognized as a duplicate rather than counted as a second turn.
    idempotency_key: Option<String>,
    /// Trips the running turn's agent loop and its SSE task. This is what makes
    /// cancel *addressable*: before BR-62 the token lived only inside the
    /// `/reply` task, so the only way to cancel was to tear down the SSE socket
    /// and `/agent/stop` merely evicted the agent from the LRU while the turn
    /// kept running on its own `Arc<Agent>`.
    cancel: CancellationToken,
    /// This turn's sequence-numbered, replayable frame log. Created WITH the
    /// turn rather than by the request that happens to start it, because the
    /// stream belongs to the turn: every observer, including the POST that
    /// began it, is just a reader of this.
    stream: Arc<TurnStream>,
    /// `Some` once the turn's guard has dropped — i.e. the turn is OVER. The
    /// entry survives its turn only so a re-POST of the same idempotency key
    /// can be answered from the replay above instead of starting a second turn;
    /// a finished entry blocks nothing and is swept after
    /// [`FINISHED_TURN_RETENTION`].
    finished_at: Option<Instant>,
    /// Exact-turn retirement signal for a cancel caller that must not submit a
    /// replacement until this guard has released the session lock.
    retirement: Arc<TurnRetirement>,
}

#[derive(Debug, Default)]
struct TurnRegistry {
    turns: HashMap<String, ActiveTurn>,
    continuation_leases: HashMap<String, ContinuationLeaseRecord>,
    stopping_sessions: HashMap<String, usize>,
    /// D18: this daemon's lease time-to-live, when a test set one. Per
    /// registry rather than process-wide so a test cannot shorten the leases of
    /// every other test running beside it.
    lease_ttl: Option<std::time::Duration>,
}

impl TurnRegistry {
    fn lease_ttl(&self) -> std::time::Duration {
        self.lease_ttl.unwrap_or_else(continuation_lease_ttl)
    }
}

#[derive(Debug, Clone)]
struct ContinuationLeaseRecord {
    group_id: String,
    owner_id: String,
    session_id: String,
    superseded_turn_id: String,
    state: ContinuationLeaseState,
}

#[derive(Debug, Clone)]
enum ContinuationLeaseState {
    Reserved {
        mark: biorouter::agents::subagent_handle::ContinuationPendingMark,
    },
    Live,
    Consumed {
        successor_idempotency_key: String,
        resolved_at: Instant,
    },
    Lost {
        resolved_at: Instant,
    },
    Abandoned {
        resolved_at: Instant,
    },
}

#[derive(Debug)]
enum ContinuationLeaseUse {
    Unclaimed,
    Live { token: String, group_id: String },
    ConsumedRetry,
}

/// Event-driven completion signal owned by one turn, never by a session slot.
///
/// Pairing the notification with an atomic state bit makes the wait safe when
/// the guard drops just before the waiter registers. Keeping it per turn means
/// a successor cannot satisfy, consume, or prolong its predecessor's wait.
#[derive(Debug)]
struct TurnRetirement {
    retired: AtomicBool,
    notify: Notify,
    /// Item 7 — the rows a Stop of this turn wrote to the transcript, left here
    /// by the runner BEFORE its guard drops, so a cancel that waited for
    /// retirement reads a complete record. It lives on the retirement because
    /// that is the one object the runner's guard and the cancel's handle already
    /// share, and it has exactly the lifetime this needs: one turn.
    stopped: StdMutex<Vec<biorouter::conversation::message::Message>>,
}

impl TurnRetirement {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            retired: AtomicBool::new(false),
            notify: Notify::new(),
            stopped: StdMutex::new(Vec::new()),
        })
    }

    fn is_retired(&self) -> bool {
        self.retired.load(Ordering::Acquire)
    }

    fn retire(&self) {
        self.retired.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    async fn wait(&self) {
        loop {
            if self.is_retired() {
                return;
            }

            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            if self.is_retired() {
                return;
            }
            notified.await;
        }
    }
}

/// The exact turn whose cancellation token was tripped.
///
/// A caller may wait on this handle without looking the session up again. That
/// distinction is what prevents a fast successor from being mistaken for the
/// cancelled turn that the caller is waiting to retire.
#[derive(Debug, Clone)]
pub struct CancelledTurn {
    turn_id: String,
    retirement: Arc<TurnRetirement>,
}

impl CancelledTurn {
    pub fn turn_id(&self) -> &str {
        &self.turn_id
    }

    pub fn is_settled(&self) -> bool {
        self.retirement.is_retired()
    }

    pub async fn wait_until_settled(&self) {
        self.retirement.wait().await;
    }

    /// The rows this turn's Stop wrote to the transcript, in stored order
    /// (item 7). Complete only once [`Self::is_settled`]: the runner records them
    /// before its guard retires. Empty for a turn that ended any other way.
    pub fn stop_messages(&self) -> Vec<biorouter::conversation::message::Message> {
        self.retirement
            .stopped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// Where a turn's runner leaves the rows its Stop wrote, for the cancel that is
/// waiting on the turn to retire (item 7). See [`CancelledTurn::stop_messages`].
#[derive(Debug, Clone)]
pub struct TurnStopRecord(Arc<TurnRetirement>);

impl TurnStopRecord {
    pub fn record(&self, rows: Vec<biorouter::conversation::message::Message>) {
        if rows.is_empty() {
            return;
        }
        self.0
            .stopped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend(rows);
    }
}

/// Outcome of an atomic, optionally generation-conditional cancel attempt.
#[derive(Debug, Clone)]
pub enum CancelTurnAttempt {
    Cancelled(CancelledTurn),
    Idle,
    TurnMismatch { active_turn_id: String },
}

/// Outcome of a Stop-and-Send cancellation whose exact generation is also the
/// admission point for a replacement child turn.
#[derive(Debug, Clone)]
pub enum ContinuationCancelAttempt {
    Cancelled {
        turn: CancelledTurn,
        admission: ContinuationAdmission,
    },
    Retired {
        turn_id: String,
        admission: ContinuationAdmission,
    },
    Idle,
    TurnMismatch {
        active_turn_id: String,
    },
    OwnerConflict,
    AdmissionInProgress,
    ParentClosing,
}

#[derive(Debug, Clone)]
pub struct ContinuationAdmission {
    token: String,
    mark: Option<biorouter::agents::subagent_handle::ContinuationPendingMark>,
}

impl ContinuationAdmission {
    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn mark(&self) -> Option<&biorouter::agents::subagent_handle::ContinuationPendingMark> {
        self.mark.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinuationLeaseFailure {
    Required,
    Invalid,
    CrossSession,
    Replayed,
    MissingSuccessorTurnId,
    OwnedByAnother,
    AdmissionInProgress,
    ParentClosing,
}

#[derive(Debug, Clone)]
pub enum TurnBeginFailure {
    Conflict(TurnConflict),
    ContinuationLease(ContinuationLeaseFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinuationLeaseAbandonment {
    Abandoned,
    AlreadyAbandoned,
    AlreadyConsumed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingContinuationStatus {
    pub superseded_turn_id: String,
    pub continuation_lease: Option<String>,
    pub ownership: PendingContinuationOwnership,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingContinuationOwnership {
    Owned,
    Foreign,
    Settling,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContinuationRecovery {
    Recovered {
        continuation_lease: String,
        superseded_turn_id: String,
    },
    Abandoned {
        superseded_turn_id: String,
    },
}

/// Why `AppState::try_begin_turn_idempotent` refused to start a turn.
#[derive(Debug, Clone)]
pub struct TurnConflict {
    /// The id of the turn already in flight for this session.
    pub running_turn_id: String,
    /// True when the caller's idempotency key matches the in-flight turn's: this
    /// POST is a *re-delivery of that same turn*, not a second one. Clients treat
    /// it as "your turn is still running, re-attach" instead of an error.
    pub duplicate: bool,
    /// The colliding turn's frame log, so a `duplicate` caller can be ATTACHED
    /// to it — answered 200 with the whole turn replayed from seq 0 and then the
    /// live tail — rather than told 409 and left with no way back into a turn
    /// that is still spending its tokens.
    pub stream: Arc<TurnStream>,
    /// True when the colliding turn has already ENDED and is only being held for
    /// its replay. Attaching to it yields the complete turn and a terminal
    /// frame; nothing is still running.
    pub finished: bool,
}

/// RAII guard marking that a session has an interactive turn in flight. Held by
/// the `/reply` task and removed from the active-turns map when dropped (turn
/// completes, errors, or is cancelled), so the next `/reply` for that session
/// can proceed. See `AppState::try_begin_turn`.
#[derive(Debug)]
pub struct TurnGuard {
    session_id: String,
    /// The turn this guard owns. Checked on drop so a guard can only ever retire
    /// *its own* entry, never a successor's.
    turn_id: String,
    /// This turn's frame log, so the runner can publish into it without a second
    /// registry lookup — and so `Drop` can close it.
    stream: Arc<TurnStream>,
    retirement: Arc<TurnRetirement>,
    continuation_lease_token: Option<String>,
    active_turns: Arc<StdMutex<TurnRegistry>>,
}

/// Admission barrier held while an agent is being stopped and evicted.
///
/// The refcount lives under the same lock as turn admission. A successor can
/// therefore observe neither the retired predecessor nor an absent barrier in
/// the gap where the stop path awaits agent eviction.
#[derive(Debug)]
pub struct AgentStopGuard {
    session_id: String,
    cancelled_turn_id: Option<String>,
    active_turns: Arc<StdMutex<TurnRegistry>>,
}

impl AgentStopGuard {
    pub fn cancelled_turn_id(&self) -> Option<&str> {
        self.cancelled_turn_id.as_deref()
    }
}

impl Drop for AgentStopGuard {
    fn drop(&mut self) {
        let mut registry = self
            .active_turns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(stops) = registry.stopping_sessions.get_mut(&self.session_id) else {
            return;
        };
        *stops = stops.saturating_sub(1);
        if *stops == 0 {
            registry.stopping_sessions.remove(&self.session_id);
        }
    }
}

impl TurnGuard {
    /// The server-assigned id of the turn this guard owns (BR-71: published as
    /// `SessionBusEvent::TurnStarted` so consumers can correlate lifecycles).
    pub fn turn_id(&self) -> &str {
        &self.turn_id
    }

    /// The session this guard locked.
    ///
    /// BR-71's `run_turn` takes the request and the guard as separate
    /// arguments, and both Task 8 and Task 14 acquire the guard themselves
    /// before calling it — so without this there is nothing stopping
    /// `run_turn(state, request_for_B, guard_for_A)` from compiling, running an
    /// unguarded turn on B while holding A's lock. The runner `debug_assert`s
    /// on it.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// This turn's frame log. The runner's SSE pump publishes into it and every
    /// HTTP response that watches the turn reads from it.
    pub fn stream(&self) -> Arc<TurnStream> {
        Arc::clone(&self.stream)
    }

    /// Where the runner records what a Stop of this turn wrote (item 7). Taken
    /// before the guard moves into supervision, and written before it drops.
    pub fn stop_record(&self) -> TurnStopRecord {
        TurnStopRecord(Arc::clone(&self.retirement))
    }
}

impl Drop for TurnGuard {
    fn drop(&mut self) {
        // ⚠ This does NOT close the turn's stream, and must not.
        //
        // The guard drops the instant the RUNNER returns, but the runner's last
        // act was to *publish* `TurnFinished` onto the session bus — the pump
        // has not necessarily consumed it yet. Closing here would race that
        // read, `publish` would refuse the frame as post-terminal, and every
        // healthy turn would end in the synthesized "stream ended without a
        // result" error instead of a clean `Finish`. The pump owns the log's
        // lifetime and closes it on every one of its exit paths
        // (`pump_bus_into_stream`); a turn with no pump is only ever attached to
        // through the retired path, which closes on demand.
        let retired = {
            let mut turns = self
                .active_turns
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // Only touch the slot if it is still ours.
            let retired = if let Some(turn) = turns.turns.get_mut(&self.session_id) {
                if turn.turn_id == self.turn_id {
                    // NOT `remove`. The entry is retired, not deleted: a re-POST of
                    // this turn's idempotency key inside FINISHED_TURN_RETENTION
                    // must attach to the replay above, not start a second turn and
                    // spend the tokens twice. `finished_at` is what every "is a turn
                    // running?" reader below filters on, so a retired entry blocks
                    // nothing.
                    turn.finished_at = Some(Instant::now());
                    true
                } else {
                    false
                }
            } else {
                false
            };
            prune_finished_turns(&mut turns);
            retired
        };

        // Publish retirement only after `finished_at` is visible. A waiter that
        // wakes is therefore guaranteed that the exact guard no longer blocks a
        // replacement turn; it never observes an optimistic acknowledgement.
        if retired {
            self.retirement.retire();
        }

        let Some(token) = self.continuation_lease_token.as_deref() else {
            return;
        };
        let superseded_turn_id = {
            let registry = self
                .active_turns
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            registry.continuation_leases.get(token).and_then(|lease| {
                matches!(&lease.state, ContinuationLeaseState::Consumed { .. })
                    .then(|| lease.superseded_turn_id.clone())
            })
        };
        let Some(superseded_turn_id) = superseded_turn_id else {
            return;
        };
        if !biorouter::agents::subagent_handle::abandon_continuation_for_turn(
            &self.session_id,
            &superseded_turn_id,
        ) {
            return;
        }
        let mut registry = self
            .active_turns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(lease) = registry.continuation_leases.get_mut(token) else {
            return;
        };
        if matches!(&lease.state, ContinuationLeaseState::Consumed { .. }) {
            lease.state = ContinuationLeaseState::Abandoned {
                resolved_at: Instant::now(),
            };
        }
    }
}

/// How long a Stop-and-Send lease may stay unclaimed (D18). 120 s, far past any
/// real Stop-and-Send (the successor is sent the moment the cancel settles);
/// `BIOROUTER_CONTINUATION_LEASE_TTL_MS` overrides it for tests.
fn continuation_lease_ttl() -> std::time::Duration {
    std::env::var("BIOROUTER_CONTINUATION_LEASE_TTL_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(std::time::Duration::from_millis)
        .unwrap_or(std::time::Duration::from_secs(120))
}

/// Abandon `token` if it is still pending. See
/// `AppState::schedule_continuation_lease_expiry`.
fn expire_continuation_lease(registry: &Arc<StdMutex<TurnRegistry>>, token: &str) {
    let expired = {
        let mut registry = registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(lease) = registry.continuation_leases.get_mut(token) else {
            return;
        };
        let pending = match &lease.state {
            ContinuationLeaseState::Reserved { mark } => {
                mark.rollback();
                true
            }
            ContinuationLeaseState::Live => true,
            ContinuationLeaseState::Consumed { .. }
            | ContinuationLeaseState::Lost { .. }
            | ContinuationLeaseState::Abandoned { .. } => false,
        };
        if !pending {
            return;
        }
        lease.state = ContinuationLeaseState::Abandoned {
            resolved_at: Instant::now(),
        };
        (lease.session_id.clone(), lease.superseded_turn_id.clone())
    };
    tracing::warn!(
        session_id = %expired.0,
        superseded_turn_id = %expired.1,
        "a Stop-and-Send continuation lease was never claimed; abandoning it"
    );
    // Outside the registry lock, like every other abandonment: the handle
    // registry has its own locks.
    biorouter::agents::subagent_handle::abandon_continuation_for_turn(&expired.0, &expired.1);
}

/// Drop retired entries once nothing can still be addressing them by key.
///
/// Called from the two places that already hold the registry lock — a guard
/// dropping and a turn beginning — so there is no sweeper task to leak, and the
/// map stays bounded by "sessions that ran a turn in the last five minutes"
/// rather than by every session id the process has ever seen.
fn prune_finished_turns(registry: &mut TurnRegistry) {
    registry
        .continuation_leases
        .retain(|_, lease| match &lease.state {
            ContinuationLeaseState::Consumed { resolved_at, .. }
            | ContinuationLeaseState::Lost { resolved_at }
            | ContinuationLeaseState::Abandoned { resolved_at } => {
                resolved_at.elapsed() < FINISHED_TURN_RETENTION
            }
            ContinuationLeaseState::Reserved { .. } | ContinuationLeaseState::Live => true,
        });
    let continuation_leases = &registry.continuation_leases;
    registry.turns.retain(|session_id, turn| {
        turn.finished_at.is_none_or(|at| {
            at.elapsed() < FINISHED_TURN_RETENTION
                || continuation_leases.values().any(|lease| {
                    lease.session_id == *session_id
                        && lease.superseded_turn_id == turn.turn_id
                        && matches!(
                            &lease.state,
                            ContinuationLeaseState::Reserved { .. } | ContinuationLeaseState::Live
                        )
                })
        })
    });
}

fn continuation_lease_use(
    registry: &TurnRegistry,
    session_id: &str,
    successor_key: Option<&str>,
    continuation_lease: Option<&str>,
) -> Result<ContinuationLeaseUse, TurnBeginFailure> {
    let Some(token) = continuation_lease else {
        let claim_required = registry.continuation_leases.values().any(|lease| {
            lease.session_id == session_id
                && matches!(
                    &lease.state,
                    ContinuationLeaseState::Reserved { .. } | ContinuationLeaseState::Live
                )
        });
        return if claim_required {
            Err(TurnBeginFailure::ContinuationLease(
                ContinuationLeaseFailure::Required,
            ))
        } else {
            Ok(ContinuationLeaseUse::Unclaimed)
        };
    };

    let lease =
        registry
            .continuation_leases
            .get(token)
            .ok_or(TurnBeginFailure::ContinuationLease(
                ContinuationLeaseFailure::Invalid,
            ))?;
    if lease.session_id != session_id {
        return Err(TurnBeginFailure::ContinuationLease(
            ContinuationLeaseFailure::CrossSession,
        ));
    }
    match &lease.state {
        ContinuationLeaseState::Reserved { .. } => Err(TurnBeginFailure::ContinuationLease(
            ContinuationLeaseFailure::Invalid,
        )),
        ContinuationLeaseState::Live => {
            let successor_key = successor_key.ok_or(TurnBeginFailure::ContinuationLease(
                ContinuationLeaseFailure::MissingSuccessorTurnId,
            ))?;
            let exact_retired_generation = registry.turns.get(session_id).is_some_and(|turn| {
                turn.finished_at.is_some() && turn.turn_id == lease.superseded_turn_id
            });
            if !exact_retired_generation || successor_key.is_empty() {
                return Err(TurnBeginFailure::ContinuationLease(
                    ContinuationLeaseFailure::Replayed,
                ));
            }
            Ok(ContinuationLeaseUse::Live {
                token: token.to_string(),
                group_id: lease.group_id.clone(),
            })
        }
        ContinuationLeaseState::Consumed {
            successor_idempotency_key,
            ..
        } if successor_key == Some(successor_idempotency_key.as_str()) => {
            Ok(ContinuationLeaseUse::ConsumedRetry)
        }
        ContinuationLeaseState::Consumed { .. }
        | ContinuationLeaseState::Lost { .. }
        | ContinuationLeaseState::Abandoned { .. } => Err(TurnBeginFailure::ContinuationLease(
            ContinuationLeaseFailure::Replayed,
        )),
    }
}

fn active_continuation_group(
    registry: &TurnRegistry,
    session_id: &str,
    superseded_turn_id: &str,
) -> Option<String> {
    registry
        .continuation_leases
        .values()
        .find(|lease| {
            lease.session_id == session_id
                && lease.superseded_turn_id == superseded_turn_id
                && matches!(
                    &lease.state,
                    ContinuationLeaseState::Reserved { .. } | ContinuationLeaseState::Live
                )
        })
        .map(|lease| lease.group_id.clone())
}

fn existing_owner_admission(
    registry: &TurnRegistry,
    group_id: &str,
    owner_id: &str,
    turn: &ActiveTurn,
) -> Option<ContinuationCancelAttempt> {
    let (token, lease) = registry.continuation_leases.iter().find(|(_, lease)| {
        lease.group_id == group_id
            && lease.owner_id == owner_id
            && matches!(
                &lease.state,
                ContinuationLeaseState::Reserved { .. } | ContinuationLeaseState::Live
            )
    })?;
    Some(match &lease.state {
        ContinuationLeaseState::Live if turn.finished_at.is_some() => {
            ContinuationCancelAttempt::Retired {
                turn_id: turn.turn_id.clone(),
                admission: ContinuationAdmission {
                    token: token.clone(),
                    mark: None,
                },
            }
        }
        ContinuationLeaseState::Live | ContinuationLeaseState::Reserved { .. } => {
            ContinuationCancelAttempt::AdmissionInProgress
        }
        ContinuationLeaseState::Consumed { .. }
        | ContinuationLeaseState::Lost { .. }
        | ContinuationLeaseState::Abandoned { .. } => unreachable!(),
    })
}

fn reserve_continuation_admission(
    registry: &mut TurnRegistry,
    session_id: &str,
    turn: &ActiveTurn,
    group_id: String,
    owner_id: &str,
) -> Result<ContinuationAdmission, ContinuationLeaseFailure> {
    if registry.stopping_sessions.contains_key(session_id) {
        return Err(ContinuationLeaseFailure::ParentClosing);
    }
    let mark = biorouter::agents::subagent_handle::mark_continuation_pending_for_turn(
        session_id,
        Some(turn.turn_id.clone()),
    );
    if mark.refused_parent_closing() {
        return Err(ContinuationLeaseFailure::ParentClosing);
    }
    let token = format!("{:032x}", rand::random::<u128>());
    registry.continuation_leases.insert(
        token.clone(),
        ContinuationLeaseRecord {
            group_id,
            owner_id: owner_id.to_string(),
            session_id: session_id.to_string(),
            superseded_turn_id: turn.turn_id.clone(),
            state: ContinuationLeaseState::Reserved { mark: mark.clone() },
        },
    );
    Ok(ContinuationAdmission {
        token,
        mark: Some(mark),
    })
}

fn conflicting_turn(
    registry: &TurnRegistry,
    session_id: &str,
    idempotency_key: Option<&str>,
) -> Option<TurnConflict> {
    let running = registry.turns.get(session_id)?;
    // Either the client key or the server-assigned turn id identifies a
    // re-delivered turn. A keyless request is always a distinct turn.
    let duplicate = idempotency_key.is_some()
        && (idempotency_key == running.idempotency_key.as_deref()
            || idempotency_key == Some(running.turn_id.as_str()));
    let finished = running.finished_at.is_some();
    // Running turns always conflict. Finished turns conflict only when the
    // caller is re-posting that exact turn into its retained replay.
    (!finished || duplicate).then(|| TurnConflict {
        running_turn_id: running.turn_id.clone(),
        duplicate,
        stream: Arc::clone(&running.stream),
        finished,
    })
}

fn stopping_conflict(registry: &TurnRegistry, session_id: &str) -> TurnConflict {
    if let Some(turn) = registry.turns.get(session_id) {
        return TurnConflict {
            running_turn_id: turn.turn_id.clone(),
            duplicate: false,
            stream: Arc::clone(&turn.stream),
            finished: turn.finished_at.is_some(),
        };
    }

    let stream = TurnStream::new(session_id, "agent-stop");
    TurnConflict {
        running_turn_id: "agent-stop".to_string(),
        duplicate: false,
        stream,
        finished: false,
    }
}

fn consume_continuation_claim_group(
    registry: &mut TurnRegistry,
    token: &str,
    group_id: &str,
    successor_idempotency_key: &str,
) {
    let resolved_at = Instant::now();
    for (claim_token, lease) in &mut registry.continuation_leases {
        if lease.group_id != group_id {
            continue;
        }
        if claim_token == token {
            lease.state = ContinuationLeaseState::Consumed {
                successor_idempotency_key: successor_idempotency_key.to_string(),
                resolved_at,
            };
            continue;
        }
        let sibling_lost = match &lease.state {
            ContinuationLeaseState::Reserved { mark } => {
                mark.rollback();
                true
            }
            ContinuationLeaseState::Live => true,
            ContinuationLeaseState::Consumed { .. }
            | ContinuationLeaseState::Lost { .. }
            | ContinuationLeaseState::Abandoned { .. } => false,
        };
        if sibling_lost {
            lease.state = ContinuationLeaseState::Lost { resolved_at };
        }
    }
}

/// How many session-observer streams (`GET /sessions/{id}/events`) this daemon
/// will follow the live tail for at the same time.
///
/// ⚠ **This is a limit on the CLIENT's connection budget, not on the server's
/// resources.** An observer stream costs the daemon almost nothing — a bus
/// subscription and a 500 ms heartbeat — so the obvious reading, "three is
/// plenty of capacity", is the wrong frame entirely. The cost is on the other
/// end of the socket. An observer never finishes on its own; it ends only when
/// the client hangs up. Every one of them therefore *parks a TCP connection for
/// as long as the tab exists*, and a browser will not open more than a handful
/// per host (six in Chromium, shared by every window of the app, because they
/// are one origin behind one network process). Once the daemon is holding that
/// many, the renderer cannot dispatch **any** further request to it: not
/// `/config/read`, not `/agent/tools`, and not `POST /reply`. The turn the user
/// just asked for is never sent, so the composer spins forever while the daemon
/// sits idle, healthy, and answering `curl` in under a millisecond — which is
/// exactly what makes the failure so hard to read from either side alone.
///
/// Measured on the wedge this constant was introduced for: six live observers,
/// 348 bytes/s of pure heartbeat out and 0 bytes in, a fresh `GET /status`
/// issued from inside the renderer still unanswered after 8 s while the same
/// request from a shell returned in 0.9 ms, and the whole app recovering the
/// instant enough tabs were closed to drop the observer count. The turns
/// themselves had all completed server-side minutes earlier.
///
/// ⚠ **Count every long-lived connection the renderer holds, not only these.**
/// This constant used to be three, documented as "leaving half the pool free",
/// and the arithmetic left out what else parks a socket: the renderer also
/// holds two ~25 s long-polls for its whole life (`GET /catalog/changes` and
/// `GET /sessions/changes`) and the running turn's
/// `/reply` stream. Three observers plus those three is six — the entire pool,
/// with nothing left for the steer, the Stop, or the `/reply` preflight, which
/// then sat 3–25 s in the browser's queue over an idle daemon (measured live,
/// 2026-09-13, before the renderer leak beside it was also fixed). Two leaves
/// `2 + 2 + 1 = 5`, one slot always free for request/response traffic;
/// [`tests::the_observer_budget_leaves_a_connection_for_a_steer_or_a_stop`]
/// pins that sum. `/reply` is still never refused: a turn stream is bounded by
/// its turn rather than by a tab, and refusing one would break the thing the
/// app exists to do.
///
/// Err low rather than high. The two mistakes are not comparable: set too high
/// and the wedge above is still reachable and takes the whole app with it; set
/// too low and a background tab does some extra polling.
///
/// ⚠ **Know what a refused observer really does next, because it is not what
/// the desktop client's backoff appears to do.** That loop resets its retry
/// delay to 1 s when the stream OPENS, not after one that lasted
/// (`ui/desktop/src/hooks/chatStreamStore.tsx:1659`), so a stream answered 200
/// and ended immediately is retried at about 1 Hz forever and never climbs
/// toward the 15 s ceiling the code looks like it provides. So an over-budget
/// tab degrades from streaming to polling the conversation roughly once a
/// second. It stays correct and current, which is why this is a tolerable
/// degraded mode rather than a second bug — but it is much more traffic than
/// reading that loop suggests. Repairing the reset is the client-side
/// follow-up; do not raise this budget to paper over it.
pub const MAX_LIVE_OBSERVER_STREAMS: usize = 2;

/// Permission to hold one session-observer stream open, released on drop.
///
/// RAII rather than a paired increment/decrement because the release has to
/// survive paths that do not run to the bottom of the observer task: the client
/// hanging up mid-send, the bus closing, a panic unwinding the task. A missed
/// decrement is not a leak that shows up as memory — it permanently shrinks the
/// client's usable connection budget, which is the same wedge one slot smaller.
#[derive(Debug)]
pub struct ObserverSlot(Arc<AtomicUsize>);

impl Drop for ObserverSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone)]
pub struct AppState {
    pub(crate) agent_manager: Arc<AgentManager>,
    /// Tracks sessions that have already emitted workflow telemetry to prevent double counting.
    workflow_session_tracker: Arc<Mutex<HashSet<String>>>,
    /// Sessions with an interactive turn in flight, mapped to that turn's state.
    /// Enforces one turn per session at the server so a second `/reply` can't
    /// race a shared `Arc<Agent>` (confirmation channel, soft-interrupt queue,
    /// check-compact-persist) with the running turn (BR-33), and holds each
    /// running turn's cancellation token so cancel is addressable by session id
    /// rather than only by dropping the SSE socket (BR-62).
    active_turns: Arc<StdMutex<TurnRegistry>>,
    /// Session-observer SSE responses this daemon is holding open right now.
    /// See [`MAX_LIVE_OBSERVER_STREAMS`] for why the count is bounded at all.
    observer_streams: Arc<AtomicUsize>,
    pub tunnel_manager: Arc<TunnelManager>,
    pub extension_loading_tasks: ExtensionLoadingTasks,
    // Used by knowledge route handlers (Task 5+).
    pub knowledge_service: Arc<KnowledgeService>,
}

impl AppState {
    pub async fn new() -> anyhow::Result<Arc<AppState>> {
        let agent_manager = AgentManager::instance().await?;
        let tunnel_manager = Arc::new(TunnelManager::new());
        let knowledge_service = Arc::new(KnowledgeService::new_default()?);

        Ok(Arc::new(Self {
            agent_manager,
            workflow_session_tracker: Arc::new(Mutex::new(HashSet::new())),
            active_turns: Arc::new(StdMutex::new(TurnRegistry::default())),
            observer_streams: Arc::new(AtomicUsize::new(0)),
            tunnel_manager,
            extension_loading_tasks: Arc::new(Mutex::new(HashMap::new())),
            knowledge_service,
        }))
    }

    /// [`AppState::new`] with its `KnowledgeService` rooted at `knowledge_root`
    /// instead of the developer's real knowledge tree.
    ///
    /// Test-only, and it exists because a test of the knowledge-selection seam
    /// has to CREATE knowledge bases and move a session's write target around.
    /// Against `new_default()` that would write into
    /// `~/.config/biorouter/knowledge` — inventing bases in the developer's own
    /// sidebar and repointing their primary. The session database behind
    /// `AgentManager::instance()` still cannot be relocated from inside a
    /// running test binary (`BIOROUTER_PATH_ROOT` is read before the process
    /// starts), so this isolates the part that can be isolated; see the
    /// preamble in `workspace/services.rs`'s tests.
    #[cfg(test)]
    pub(crate) async fn new_with_knowledge_root(
        knowledge_root: std::path::PathBuf,
    ) -> anyhow::Result<Arc<AppState>> {
        let agent_manager = AgentManager::instance().await?;
        let tunnel_manager = Arc::new(TunnelManager::new());
        let knowledge_service = Arc::new(KnowledgeService::new(knowledge_root));

        Ok(Arc::new(Self {
            agent_manager,
            workflow_session_tracker: Arc::new(Mutex::new(HashSet::new())),
            active_turns: Arc::new(StdMutex::new(TurnRegistry::default())),
            observer_streams: Arc::new(AtomicUsize::new(0)),
            tunnel_manager,
            extension_loading_tasks: Arc::new(Mutex::new(HashMap::new())),
            knowledge_service,
        }))
    }

    /// Claim one of the [`MAX_LIVE_OBSERVER_STREAMS`] slots for following a
    /// session's live tail, or `None` when the daemon is already holding that
    /// many open.
    ///
    /// The check and the increment are one `fetch_update`, not a load followed
    /// by a store: several tabs re-attach in the same instant after a reload,
    /// and a read-then-write would let all of them see the same under-budget
    /// count and admit every one of them — which is the state being bounded.
    ///
    /// Refusal is not an error, and callers must not report it as one. The
    /// observer's first frame is the whole stored conversation, so a caller
    /// that is turned away has already been told everything the session
    /// currently says; all it loses is the live tail, and its own reconnect
    /// backoff will ask again.
    pub fn try_admit_observer_stream(&self) -> Option<ObserverSlot> {
        let counter = Arc::clone(&self.observer_streams);
        counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |live| {
                (live < MAX_LIVE_OBSERVER_STREAMS).then_some(live + 1)
            })
            .ok()
            .map(|_| ObserverSlot(counter))
    }

    /// Session-observer streams currently held open. Test-facing.
    #[cfg(test)]
    pub(crate) fn live_observer_streams(&self) -> usize {
        self.observer_streams.load(Ordering::Acquire)
    }

    /// Begin an interactive turn for `session_id`. Returns a `TurnGuard` that
    /// keeps the session marked busy until dropped, or a [`TurnConflict`] if a
    /// turn is already in flight — the caller must reject the duplicate `/reply`
    /// rather than start a second turn on the shared agent (BR-33).
    ///
    /// `cancel` is registered as the token that [`AppState::cancel_turn`] trips,
    /// and `idempotency_key` as the client's name for this turn (BR-62).
    ///
    /// The key is what makes `/reply` safe to retry. With `sseMaxRetryAttempts`,
    /// a dropped stream makes the client re-POST the *same* turn; without a key
    /// the server cannot distinguish that from a genuine second turn, so the
    /// retry either starts a duplicate turn (double token spend, interleaved
    /// output) or is rejected as a hard error. With one, the conflict comes back
    /// flagged `duplicate` — carrying the turn's [`TurnStream`], so `/reply` can
    /// ATTACH the caller to the running turn (200 + its replay) instead of
    /// answering 409 and leaving it with no way back into a turn that is still
    /// spending its tokens.
    pub fn try_begin_turn_idempotent(
        &self,
        session_id: &str,
        cancel: CancellationToken,
        idempotency_key: Option<String>,
    ) -> Result<TurnGuard, TurnConflict> {
        match self.try_begin_turn_idempotent_with_continuation(
            session_id,
            cancel,
            idempotency_key,
            None,
        ) {
            Ok(guard) => Ok(guard),
            Err(TurnBeginFailure::Conflict(conflict)) => Err(conflict),
            Err(TurnBeginFailure::ContinuationLease(_)) => {
                let turns = self
                    .active_turns
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let turn = turns
                    .turns
                    .get(session_id)
                    .expect("a live continuation lease pins its retired turn");
                Err(TurnConflict {
                    running_turn_id: turn.turn_id.clone(),
                    duplicate: false,
                    stream: Arc::clone(&turn.stream),
                    finished: turn.finished_at.is_some(),
                })
            }
        }
    }

    pub fn try_begin_turn_idempotent_with_continuation(
        &self,
        session_id: &str,
        cancel: CancellationToken,
        idempotency_key: Option<String>,
        continuation_lease: Option<&str>,
    ) -> Result<TurnGuard, TurnBeginFailure> {
        let mut registry = self
            .active_turns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        prune_finished_turns(&mut registry);

        if registry.stopping_sessions.contains_key(session_id) {
            return Err(TurnBeginFailure::Conflict(stopping_conflict(
                &registry, session_id,
            )));
        }
        let lease_use = continuation_lease_use(
            &registry,
            session_id,
            idempotency_key.as_deref(),
            continuation_lease,
        )?;
        if let Some(conflict) = conflicting_turn(&registry, session_id, idempotency_key.as_deref())
        {
            return Err(TurnBeginFailure::Conflict(conflict));
        }
        if matches!(lease_use, ContinuationLeaseUse::ConsumedRetry) {
            return Err(TurnBeginFailure::ContinuationLease(
                ContinuationLeaseFailure::Replayed,
            ));
        }

        let turn_id = format!("turn-{}", TURN_SEQ.fetch_add(1, Ordering::Relaxed));
        let stream = TurnStream::new(session_id, &turn_id);
        let retirement = TurnRetirement::new();
        let successor_idempotency_key = idempotency_key.clone();
        registry.turns.insert(
            session_id.to_string(),
            ActiveTurn {
                turn_id: turn_id.clone(),
                idempotency_key,
                cancel,
                stream: Arc::clone(&stream),
                finished_at: None,
                retirement: Arc::clone(&retirement),
            },
        );
        let continuation_lease_token =
            if let ContinuationLeaseUse::Live { token, group_id } = lease_use {
                consume_continuation_claim_group(
                    &mut registry,
                    &token,
                    &group_id,
                    successor_idempotency_key
                        .as_deref()
                        .expect("a consumed continuation lease requires an idempotency key"),
                );
                Some(token)
            } else {
                None
            };
        Ok(TurnGuard {
            session_id: session_id.to_string(),
            turn_id,
            stream,
            retirement,
            continuation_lease_token,
            active_turns: Arc::clone(&self.active_turns),
        })
    }

    /// Block successor admission while a stop operation cancels and evicts the
    /// current agent. The barrier and active-turn snapshot are taken under the
    /// same registry lock used by [`Self::try_begin_turn_idempotent`].
    pub fn begin_agent_stop(&self, session_id: &str) -> AgentStopGuard {
        let (cancel, cancelled_turn_id) = {
            let mut registry = self
                .active_turns
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            prune_finished_turns(&mut registry);
            *registry
                .stopping_sessions
                .entry(session_id.to_string())
                .or_default() += 1;
            registry
                .turns
                .get(session_id)
                .filter(|turn| turn.finished_at.is_none())
                .map_or((None, None), |turn| {
                    (Some(turn.cancel.clone()), Some(turn.turn_id.clone()))
                })
        };
        if let Some(cancel) = cancel {
            cancel.cancel();
        }
        AgentStopGuard {
            session_id: session_id.to_string(),
            cancelled_turn_id,
            active_turns: Arc::clone(&self.active_turns),
        }
    }

    /// Cancel the turn in flight for `session_id`, returning its id. `None` when
    /// no turn is running.
    ///
    /// This is the addressable cancel BR-62 adds: tripping the token unwinds the
    /// agent loop at its next boundary, unblocks any tool-permission prompt it is
    /// parked on, and ends the SSE task — all without the client having to drop
    /// its socket, so a second client, the CLI, or a script can stop a runaway
    /// turn. The `TurnGuard` clears the slot as the turn task unwinds; we
    /// deliberately do **not** remove the entry here, so `cancel_turn` stays
    /// idempotent (a second call finds the same token, already tripped) rather
    /// than racing the guard.
    pub fn cancel_turn(&self, session_id: &str) -> Option<String> {
        match self.cancel_turn_waitable(session_id, None) {
            CancelTurnAttempt::Cancelled(turn) => Some(turn.turn_id),
            CancelTurnAttempt::Idle | CancelTurnAttempt::TurnMismatch { .. } => None,
        }
    }

    /// Cancel the turn in flight and return an exact-turn retirement handle.
    ///
    /// Unlike re-reading `active_turns` by session id, this handle cannot become
    /// confused with a successor that starts after the cancelled guard drops.
    pub fn cancel_turn_waitable(
        &self,
        session_id: &str,
        expected_turn_id: Option<&str>,
    ) -> CancelTurnAttempt {
        let turn = {
            let turns = self
                .active_turns
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(turn) = turns.turns.get(session_id) else {
                return CancelTurnAttempt::Idle;
            };

            if expected_turn_id.is_some_and(|expected| {
                expected != turn.turn_id && turn.idempotency_key.as_deref() != Some(expected)
            }) {
                return CancelTurnAttempt::TurnMismatch {
                    active_turn_id: turn.turn_id.clone(),
                };
            }
            // A retired matching generation is not a turn: cancelling a turn
            // that already ended stays the 200 no-op it has always been. A
            // DIFFERENT retained generation was handled as a mismatch above.
            if turn.finished_at.is_some() {
                return CancelTurnAttempt::Idle;
            }
            turn.clone()
        };
        turn.cancel.cancel();
        CancelTurnAttempt::Cancelled(CancelledTurn {
            turn_id: turn.turn_id,
            retirement: turn.retirement,
        })
    }

    /// Mark an exact child-turn generation as awaiting a replacement, then
    /// cancel it if it is still active.
    ///
    /// The generation match, continuation mark, and active-turn snapshot happen
    /// while holding the same registry lock. A successor therefore cannot slip
    /// between the match and the mark. An exactly retained retired generation
    /// is safe to mark too: its guard is already settled, but the parent still
    /// needs the pending state during the gap before the replacement starts.
    pub fn cancel_turn_for_continuation_owned(
        &self,
        session_id: &str,
        expected_turn_id: &str,
        owner_id: &str,
    ) -> ContinuationCancelAttempt {
        let (turn, admission, ttl) = {
            let mut registry = self
                .active_turns
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            prune_finished_turns(&mut registry);
            if registry.stopping_sessions.contains_key(session_id) {
                return ContinuationCancelAttempt::ParentClosing;
            }
            let Some(turn) = registry.turns.get(session_id).cloned() else {
                return ContinuationCancelAttempt::Idle;
            };
            let matches = turn.turn_id == expected_turn_id
                || turn.idempotency_key.as_deref() == Some(expected_turn_id);
            if !matches {
                return ContinuationCancelAttempt::TurnMismatch {
                    active_turn_id: turn.turn_id.clone(),
                };
            }

            let existing_group_id = active_continuation_group(&registry, session_id, &turn.turn_id);
            if let Some(group_id) = existing_group_id.as_deref() {
                if let Some(attempt) =
                    existing_owner_admission(&registry, group_id, owner_id, &turn)
                {
                    return attempt;
                }
                return ContinuationCancelAttempt::OwnerConflict;
            }

            let group_id = format!("{:032x}", rand::random::<u128>());
            let admission = match reserve_continuation_admission(
                &mut registry,
                session_id,
                &turn,
                group_id,
                owner_id,
            ) {
                Ok(admission) => admission,
                Err(ContinuationLeaseFailure::ParentClosing) => {
                    return ContinuationCancelAttempt::ParentClosing
                }
                Err(_) => unreachable!("reservation only refuses a closing parent"),
            };
            let ttl = registry.lease_ttl();
            if turn.finished_at.is_some() {
                // Only spawns; it takes no lock until the lease's time is up.
                self.schedule_continuation_lease_expiry(&admission.token, ttl);
                return ContinuationCancelAttempt::Retired {
                    turn_id: turn.turn_id.clone(),
                    admission,
                };
            }
            (turn, admission, ttl)
        };

        self.schedule_continuation_lease_expiry(&admission.token, ttl);
        turn.cancel.cancel();
        ContinuationCancelAttempt::Cancelled {
            turn: CancelledTurn {
                turn_id: turn.turn_id,
                retirement: turn.retirement,
            },
            admission,
        }
    }

    /// D18: a Stop-and-Send lease that is never claimed must not wedge the chat.
    ///
    /// Reserved and Live leases used to have no expiry. A renderer that lost its
    /// lease — reloaded, crashed, its successor `/reply` never sent — left every
    /// lease-less `/reply` answering 409 `continuation_lease_required` and every
    /// steer answering 409 (no turn runs), and a parent supervising the child
    /// waited on `continuation_pending` forever. After
    /// [`continuation_lease_ttl`] a lease still pending is abandoned exactly as
    /// `POST /agent/continuation/abandon` would, and the child's handle is told,
    /// so supervision wakes. A lease already claimed, lost or abandoned is left
    /// alone.
    fn schedule_continuation_lease_expiry(&self, token: &str, ttl: std::time::Duration) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let registry = Arc::clone(&self.active_turns);
        let token = token.to_string();
        handle.spawn(async move {
            tokio::time::sleep(ttl).await;
            expire_continuation_lease(&registry, &token);
        });
    }

    pub fn commit_continuation_lease(&self, token: &str) -> bool {
        let mut registry = self
            .active_turns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(lease) = registry.continuation_leases.get_mut(token) else {
            return false;
        };
        if matches!(&lease.state, ContinuationLeaseState::Reserved { .. }) {
            lease.state = ContinuationLeaseState::Live;
            return true;
        }
        matches!(&lease.state, ContinuationLeaseState::Live)
    }

    pub fn rollback_continuation_lease(&self, token: &str) {
        let release_supervision = {
            let mut registry = self
                .active_turns
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(lease) = registry.continuation_leases.get(token) else {
                return;
            };
            let (group_id, session_id, superseded_turn_id) =
                if let ContinuationLeaseState::Reserved { mark } = &lease.state {
                    mark.rollback();
                    (
                        lease.group_id.clone(),
                        lease.session_id.clone(),
                        lease.superseded_turn_id.clone(),
                    )
                } else {
                    return;
                };
            registry.continuation_leases.remove(token);
            let release_supervision = !registry.continuation_leases.values().any(|lease| {
                lease.group_id == group_id
                    && matches!(
                        &lease.state,
                        ContinuationLeaseState::Reserved { .. } | ContinuationLeaseState::Live
                    )
            });
            release_supervision.then_some((session_id, superseded_turn_id))
        };
        if let Some((session_id, superseded_turn_id)) = release_supervision {
            biorouter::agents::subagent_handle::abandon_continuation_for_turn(
                &session_id,
                &superseded_turn_id,
            );
        }
    }

    pub fn pending_continuation_for_owner(
        &self,
        session_id: &str,
        owner_id: Option<&str>,
    ) -> Option<PendingContinuationStatus> {
        let mut registry = self
            .active_turns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        prune_finished_turns(&mut registry);
        let owned = owner_id.and_then(|owner_id| {
            registry
                .continuation_leases
                .iter()
                .find(|(_, lease)| {
                    lease.session_id == session_id
                        && lease.owner_id == owner_id
                        && matches!(
                            &lease.state,
                            ContinuationLeaseState::Reserved { .. } | ContinuationLeaseState::Live
                        )
                })
                .map(|(token, lease)| (token.clone(), lease.clone()))
        });
        if let Some((token, lease)) = owned {
            let (ownership, continuation_lease) = match lease.state {
                ContinuationLeaseState::Live => (PendingContinuationOwnership::Owned, Some(token)),
                ContinuationLeaseState::Reserved { .. } => {
                    (PendingContinuationOwnership::Settling, None)
                }
                ContinuationLeaseState::Consumed { .. }
                | ContinuationLeaseState::Lost { .. }
                | ContinuationLeaseState::Abandoned { .. } => unreachable!(),
            };
            return Some(PendingContinuationStatus {
                superseded_turn_id: lease.superseded_turn_id,
                continuation_lease,
                ownership,
            });
        }
        registry
            .continuation_leases
            .values()
            .find(|lease| {
                lease.session_id == session_id
                    && matches!(
                        &lease.state,
                        ContinuationLeaseState::Reserved { .. } | ContinuationLeaseState::Live
                    )
            })
            .map(|lease| PendingContinuationStatus {
                superseded_turn_id: lease.superseded_turn_id.clone(),
                continuation_lease: None,
                ownership: PendingContinuationOwnership::Foreign,
            })
    }

    pub fn recover_continuation_for_owner(
        &self,
        session_id: &str,
        superseded_turn_id: &str,
        owner_id: &str,
    ) -> Result<ContinuationRecovery, ContinuationLeaseFailure> {
        let mut registry = self
            .active_turns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        prune_finished_turns(&mut registry);
        if registry.stopping_sessions.contains_key(session_id) {
            return Err(ContinuationLeaseFailure::ParentClosing);
        }
        let exact_retired_generation = registry
            .turns
            .get(session_id)
            .is_some_and(|turn| turn.finished_at.is_some() && turn.turn_id == superseded_turn_id);
        if !exact_retired_generation {
            return Err(ContinuationLeaseFailure::Replayed);
        }
        let group_id = active_continuation_group(&registry, session_id, superseded_turn_id)
            .ok_or(ContinuationLeaseFailure::Replayed)?;
        if let Some((token, _)) = registry.continuation_leases.iter().find(|(_, lease)| {
            lease.group_id == group_id
                && lease.owner_id == owner_id
                && matches!(&lease.state, ContinuationLeaseState::Live)
        }) {
            return Ok(ContinuationRecovery::Recovered {
                continuation_lease: token.clone(),
                superseded_turn_id: superseded_turn_id.to_string(),
            });
        }

        let mark = biorouter::agents::subagent_handle::mark_continuation_pending_for_turn(
            session_id,
            Some(superseded_turn_id.to_string()),
        );
        if mark.refused_parent_closing() {
            return Err(ContinuationLeaseFailure::ParentClosing);
        }

        let resolved_at = Instant::now();
        for lease in registry.continuation_leases.values_mut() {
            if lease.group_id != group_id {
                continue;
            }
            let was_pending = match &lease.state {
                ContinuationLeaseState::Reserved { mark } => {
                    mark.rollback();
                    true
                }
                ContinuationLeaseState::Live => true,
                ContinuationLeaseState::Consumed { .. }
                | ContinuationLeaseState::Lost { .. }
                | ContinuationLeaseState::Abandoned { .. } => false,
            };
            if was_pending {
                lease.state = ContinuationLeaseState::Lost { resolved_at };
            }
        }

        mark.commit();
        let token = format!("{:032x}", rand::random::<u128>());
        registry.continuation_leases.insert(
            token.clone(),
            ContinuationLeaseRecord {
                group_id,
                owner_id: owner_id.to_string(),
                session_id: session_id.to_string(),
                superseded_turn_id: superseded_turn_id.to_string(),
                state: ContinuationLeaseState::Live,
            },
        );
        let ttl = registry.lease_ttl();
        drop(registry);
        self.schedule_continuation_lease_expiry(&token, ttl);
        Ok(ContinuationRecovery::Recovered {
            continuation_lease: token,
            superseded_turn_id: superseded_turn_id.to_string(),
        })
    }

    pub fn abandon_continuation_group(
        &self,
        session_id: &str,
        superseded_turn_id: &str,
    ) -> Result<ContinuationRecovery, ContinuationLeaseFailure> {
        {
            let mut registry = self
                .active_turns
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            prune_finished_turns(&mut registry);
            let Some(group_id) =
                active_continuation_group(&registry, session_id, superseded_turn_id)
            else {
                let already_abandoned = registry.continuation_leases.values().any(|lease| {
                    lease.session_id == session_id
                        && lease.superseded_turn_id == superseded_turn_id
                        && matches!(&lease.state, ContinuationLeaseState::Abandoned { .. })
                });
                return if already_abandoned {
                    Ok(ContinuationRecovery::Abandoned {
                        superseded_turn_id: superseded_turn_id.to_string(),
                    })
                } else {
                    Err(ContinuationLeaseFailure::Replayed)
                };
            };
            let resolved_at = Instant::now();
            for lease in registry.continuation_leases.values_mut() {
                if lease.group_id != group_id {
                    continue;
                }
                let was_pending = match &lease.state {
                    ContinuationLeaseState::Reserved { mark } => {
                        mark.rollback();
                        true
                    }
                    ContinuationLeaseState::Live => true,
                    ContinuationLeaseState::Consumed { .. }
                    | ContinuationLeaseState::Lost { .. }
                    | ContinuationLeaseState::Abandoned { .. } => false,
                };
                if was_pending {
                    lease.state = ContinuationLeaseState::Abandoned { resolved_at };
                }
            }
        }
        biorouter::agents::subagent_handle::abandon_continuation_for_turn(
            session_id,
            superseded_turn_id,
        );
        Ok(ContinuationRecovery::Abandoned {
            superseded_turn_id: superseded_turn_id.to_string(),
        })
    }

    /// Resolve every Stop-and-Send admission for a session that is being
    /// stopped. The route calls this before it cancels the turn or evicts the
    /// agent, so neither a reserved mark nor a live lease can outlast the child.
    pub fn abandon_pending_continuations_for_session(&self, session_id: &str) -> bool {
        let lease_abandoned = {
            let mut registry = self
                .active_turns
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            prune_finished_turns(&mut registry);
            let resolved_at = Instant::now();
            let mut abandoned = false;
            for lease in registry
                .continuation_leases
                .values_mut()
                .filter(|lease| lease.session_id == session_id)
            {
                let was_pending = match &lease.state {
                    ContinuationLeaseState::Reserved { mark } => {
                        mark.rollback();
                        true
                    }
                    ContinuationLeaseState::Live => true,
                    ContinuationLeaseState::Consumed { .. }
                    | ContinuationLeaseState::Lost { .. }
                    | ContinuationLeaseState::Abandoned { .. } => false,
                };
                if was_pending {
                    lease.state = ContinuationLeaseState::Abandoned { resolved_at };
                    abandoned = true;
                }
            }
            abandoned
        };
        biorouter::agents::subagent_handle::abandon_continuation(session_id) || lease_abandoned
    }

    pub fn abandon_continuation_lease(
        &self,
        session_id: &str,
        token: &str,
    ) -> Result<ContinuationLeaseAbandonment, ContinuationLeaseFailure> {
        let (superseded_turn_id, release_supervision) = {
            let mut registry = self
                .active_turns
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            prune_finished_turns(&mut registry);
            let lease = registry
                .continuation_leases
                .get_mut(token)
                .ok_or(ContinuationLeaseFailure::Invalid)?;
            if lease.session_id != session_id {
                return Err(ContinuationLeaseFailure::CrossSession);
            }
            let (group_id, superseded_turn_id) = match &lease.state {
                ContinuationLeaseState::Reserved { mark } => {
                    mark.rollback();
                    let group_id = lease.group_id.clone();
                    let superseded_turn_id = lease.superseded_turn_id.clone();
                    lease.state = ContinuationLeaseState::Abandoned {
                        resolved_at: Instant::now(),
                    };
                    (group_id, superseded_turn_id)
                }
                ContinuationLeaseState::Live => {
                    let group_id = lease.group_id.clone();
                    let superseded_turn_id = lease.superseded_turn_id.clone();
                    lease.state = ContinuationLeaseState::Abandoned {
                        resolved_at: Instant::now(),
                    };
                    (group_id, superseded_turn_id)
                }
                ContinuationLeaseState::Abandoned { .. } => {
                    return Ok(ContinuationLeaseAbandonment::AlreadyAbandoned);
                }
                ContinuationLeaseState::Consumed { .. } | ContinuationLeaseState::Lost { .. } => {
                    return Ok(ContinuationLeaseAbandonment::AlreadyConsumed);
                }
            };
            let release_supervision = !registry.continuation_leases.values().any(|lease| {
                lease.group_id == group_id
                    && matches!(
                        &lease.state,
                        ContinuationLeaseState::Reserved { .. } | ContinuationLeaseState::Live
                    )
            });
            (superseded_turn_id, release_supervision)
        };
        if release_supervision {
            biorouter::agents::subagent_handle::abandon_continuation_for_turn(
                session_id,
                &superseded_turn_id,
            );
        }
        Ok(ContinuationLeaseAbandonment::Abandoned)
    }

    /// True while an interactive turn is in flight for `session_id` (the BR-33
    /// turn lock is held). BR-61 uses it to reject a soft interrupt that has no
    /// running turn to steer — queueing it on the agent would otherwise strand
    /// the text until some later turn injected it out of nowhere.
    ///
    /// A retired entry (a turn that has ended, kept only so its key can be
    /// re-POSTed into its replay) is NOT active. Every reader below filters the
    /// same way, so "the map has an entry" and "a turn is running" never drift.
    pub fn is_turn_active(&self, session_id: &str) -> bool {
        self.active_turns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .turns
            .get(session_id)
            .is_some_and(|turn| turn.finished_at.is_none())
    }

    /// Every session with a turn in flight right now (BR-71 CLI parity).
    ///
    /// `is_turn_active` answers for one session; a listing needs the set, and
    /// N round-trips for an N-row page is both slower and racier — the rows
    /// would be read at N different instants.
    pub fn active_turn_session_ids(&self) -> Vec<String> {
        self.active_turns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .turns
            .iter()
            .filter(|(_, turn)| turn.finished_at.is_none())
            .map(|(session_id, _)| session_id.clone())
            .collect()
    }

    /// The id of the turn in flight for `session_id`, if there is one.
    ///
    /// Exists so `POST /agent/resume` can hand a reloading window the turn it
    /// should re-attach to. Without it the client has to publish its own turn
    /// pointer through `localStorage` and survive a reload with it — and a
    /// pointer the client keeps is a pointer that can go stale, which makes
    /// "attached to a turn that no longer exists" a category of bug rather than
    /// an impossible state. The server always knows; asking it removes the
    /// class. Returns `None` for a retired (already finished) turn: there is
    /// nothing live to attach to.
    ///
    /// It also returns `None` for a turn whose stream has no LIVE writer, and
    /// that filter is not an optimisation. Several callers take this same turn
    /// lock for reasons that have nothing to do with streaming — an in-place edit
    /// and a working-directory change hold it as a plain mutex, and some app turn
    /// runners hold it without a `/reply` pump. A claimed writer is not enough
    /// either: that ownership latch stays set after the writer drops, while the
    /// closed log can no longer deliver a terminal to a new observer. Advertising
    /// either shape as attachable parks the window in `Streaming` forever.
    pub fn active_turn_id(&self, session_id: &str) -> Option<String> {
        self.active_turns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .turns
            .get(session_id)
            .filter(|turn| turn.finished_at.is_none() && turn.stream.has_live_writer())
            .map(|turn| turn.turn_id.clone())
    }

    /// D18: shorten this daemon's continuation-lease time-to-live. Test-only.
    #[cfg(test)]
    pub(crate) fn set_continuation_lease_ttl_for_test(&self, ttl: std::time::Duration) {
        self.active_turns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .lease_ttl = Some(ttl);
    }

    /// The live frame log of the turn running in `session_id`, if one is.
    pub fn active_turn_stream(&self, session_id: &str) -> Option<Arc<TurnStream>> {
        self.active_turns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .turns
            .get(session_id)
            .filter(|turn| turn.finished_at.is_none())
            .map(|turn| Arc::clone(&turn.stream))
    }

    /// Does this session hold a turn — running or retained for replay — that
    /// `turn_id` names?
    ///
    /// Read-only, and that is the point: `/reply` uses it to tell an ATTACH
    /// whose turn is gone from a first POST, and it must be able to ask without
    /// minting a turn entry to find out. Matches EITHER name, exactly as
    /// [`Self::try_begin_turn_idempotent`] does — the client's own idempotency
    /// key, or the server-assigned `turn-N` a reloaded window read off a frame.
    pub fn knows_turn(&self, session_id: &str, turn_id: &str) -> bool {
        self.active_turns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .turns
            .get(session_id)
            .is_some_and(|turn| {
                turn.turn_id == turn_id || turn.idempotency_key.as_deref() == Some(turn_id)
            })
    }

    pub fn has_active_turns(&self) -> bool {
        self.active_turns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .turns
            .values()
            .any(|turn| turn.finished_at.is_none())
    }

    pub async fn clear_cached_agents(&self) -> usize {
        let tasks = {
            let mut tasks = self.extension_loading_tasks.lock().await;
            tasks.drain().map(|(_, task)| task).collect::<Vec<_>>()
        };
        for task in tasks {
            if let Some(handle) = task.lock().await.take() {
                handle.abort();
            }
        }
        self.workflow_session_tracker.lock().await.clear();
        self.agent_manager.clear_sessions().await
    }

    pub async fn set_extension_loading_task(
        &self,
        session_id: String,
        task: JoinHandle<Vec<ExtensionLoadResult>>,
    ) {
        let mut tasks = self.extension_loading_tasks.lock().await;
        tasks.insert(session_id, Arc::new(Mutex::new(Some(task))));
    }

    /// How long a turn will wait for a session's extensions before going ahead
    /// without them.
    ///
    /// Generous, because spawning stdio MCP servers is genuinely slow and the
    /// normal wait is ~300 ms. It is a deadlock bound, not a performance tuning
    /// knob: what it rules out is one wedged extension parking every turn in the
    /// session forever. `cdwagent` and `medcp` fail to load on a machine without
    /// UCSF credentials, so this is the ordinary case, not the exotic one.
    const EXTENSION_LOAD_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

    pub async fn take_extension_loading_task(
        &self,
        session_id: &str,
    ) -> Option<Vec<ExtensionLoadResult>> {
        self.take_extension_loading_task_bounded(session_id, Self::EXTENSION_LOAD_WAIT)
            .await
    }

    /// The same wait with an explicit bound, so a test can prove the bound
    /// EXISTS without sitting through the production one. A 30 second unit test
    /// is a test somebody eventually deletes.
    async fn take_extension_loading_task_bounded(
        &self,
        session_id: &str,
        wait: std::time::Duration,
    ) -> Option<Vec<ExtensionLoadResult>> {
        let task_holder = {
            let tasks = self.extension_loading_tasks.lock().await;
            tasks.get(session_id).cloned()
        };

        if let Some(holder) = task_holder {
            let task = holder.lock().await.take();
            if let Some(handle) = task {
                // ⚠ BOUNDED. An unbounded `handle.await` here turns a slow
                // extension into a HUNG PRODUCT, and that is not hypothetical:
                // it wedged a live app for hours. `/reply` runs this while
                // holding the session's turn guard, so one extension that never
                // finishes loading parks the turn task forever and every later
                // turn then blocks on the lock. Symptom set, all of which point
                // away from the cause: no outbound network, ~0% CPU, every
                // tokio worker in `kevent`, `/active_work` empty, HTTP reads
                // still 200 in 2 ms, and NO turn frame in `sample` (a parked
                // task has no thread stack). The GUI shows "Thinking" forever
                // over a daemon that holds no turn at all.
                //
                // Timing out is the pre-D2 behaviour and it is safe: the point
                // of the wait was only to stop a turn racing ahead of a load
                // that is about to finish in ~300 ms. Waiting an unbounded time
                // for one that never will buys nothing and costs everything.
                //
                // Dropping the handle DETACHES the task, it does not abort it,
                // so a genuinely slow extension still finishes loading in the
                // background and the next turn sees it.
                match tokio::time::timeout(wait, handle).await {
                    Ok(Ok(results)) => return Some(results),
                    Ok(Err(e)) => {
                        tracing::warn!("Background extension loading task failed: {}", e);
                    }
                    Err(_) => {
                        tracing::warn!(
                            session_id,
                            timeout_secs = wait.as_secs(),
                            "extensions still loading after the wait; continuing \
                             with whatever has loaded so far. A turn may see a \
                             partial toolset."
                        );
                    }
                }
            }
        }
        None
    }

    pub async fn remove_extension_loading_task(&self, session_id: &str) {
        let mut tasks = self.extension_loading_tasks.lock().await;
        tasks.remove(session_id);
    }

    pub fn scheduler(&self) -> Arc<dyn SchedulerTrait> {
        self.agent_manager.scheduler()
    }

    pub fn session_manager(&self) -> &SessionManager {
        self.agent_manager.session_manager()
    }

    pub async fn mark_workflow_run_if_absent(&self, session_id: &str) -> bool {
        let mut sessions = self.workflow_session_tracker.lock().await;
        if sessions.contains(session_id) {
            false
        } else {
            sessions.insert(session_id.to_string());
            true
        }
    }

    pub async fn get_agent(
        &self,
        session_id: String,
    ) -> anyhow::Result<Arc<biorouter::agents::Agent>> {
        self.agent_manager.get_or_create_agent(session_id).await
    }

    /// Look up a live agent for `session_id` **without creating one**.
    ///
    /// ⚠ **The route that INSPECTS a chat wants this, never [`Self::get_agent`].**
    /// That one is `AgentManager::get_or_create_agent`, and its miss path mints a
    /// bare agent bound to the process default provider with no extensions, then
    /// caches it under the session id — so an inspection reads today's global
    /// config instead of the chat's, gets an empty answer, and leaves a
    /// placeholder behind for whoever asks next. `AgentManager::peek_agent`'s own
    /// doc records that hazard; this is the accessor that makes it avoidable from
    /// a route.
    ///
    /// `None` is a real answer — a daemon restart or an LRU eviction — and a
    /// caller must say so rather than reporting it as an empty chat.
    pub async fn peek_agent(&self, session_id: &str) -> Option<Arc<biorouter::agents::Agent>> {
        self.agent_manager.peek_agent(session_id).await
    }

    /// The agent for a route that is about to DO something with it.
    ///
    /// ⚠ **Waits for the session's extensions first, and that wait is the
    /// point.** `/agent/start` kicks extension loading off in the background
    /// and returns 200 immediately, so for roughly 300 ms a session reports a
    /// couple of tools before settling to its full set. Measured 4/4, and under
    /// concurrent starts a session became "ready" holding 10 of 116 tools.
    ///
    /// A turn running inside that window silently gets a degraded toolset with
    /// no `subagent`, so the model answers "I cannot delegate" — which is
    /// **indistinguishable from the legitimate condition-5 refusal** (no
    /// non-injected extensions). A live stress pass corrupted one of its own
    /// batches on this before it added a client-side readiness wait, and no
    /// client should have to.
    ///
    /// `/agent/resume` already awaited the same handle; this puts the wait at
    /// the choke point every turn-running route passes through instead, so a
    /// new route cannot forget it.
    pub async fn get_agent_for_route(
        &self,
        session_id: String,
    ) -> Result<Arc<biorouter::agents::Agent>, StatusCode> {
        // Best-effort: a load that failed is reported by the route that started
        // it, and a turn on a partially-loaded session is still better than a
        // 500 here. What matters is that we do not run BEFORE it settles.
        self.take_extension_loading_task(&session_id).await;
        self.remove_extension_loading_task(&session_id).await;

        self.get_agent(session_id).await.map_err(|e| {
            tracing::error!("Failed to get agent: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
    }
}

#[cfg(test)]
mod tests {
    /// The long-polls the desktop renderer keeps parked for its whole life, each
    /// on its own connection: `GET /catalog/changes` (`ConfigContext`) and
    /// `GET /sessions/changes` (`utils/sessionRowSync`). A third long-poll added
    /// to the renderer must raise this and lower `MAX_LIVE_OBSERVER_STREAMS`.
    const RENDERER_LONG_POLLS: usize = 2;

    /// Chromium's per-host connection limit, shared by every window of the app
    /// (one origin, one network process).
    const BROWSER_CONNECTIONS_PER_HOST: usize = 6;

    /// D1: observers, the renderer's long-polls and one running `/reply` must
    /// leave at least one of the browser's connections free, or a steer and a
    /// Stop queue in the browser behind sockets that never finish.
    #[test]
    fn the_observer_budget_leaves_a_connection_for_a_steer_or_a_stop() {
        let parked = std::hint::black_box(super::MAX_LIVE_OBSERVER_STREAMS)
            + std::hint::black_box(RENDERER_LONG_POLLS)
            + 1;
        assert!(
            parked < BROWSER_CONNECTIONS_PER_HOST,
            "{parked} parked connections leave no slot for request/response traffic \
             out of {}",
            BROWSER_CONNECTIONS_PER_HOST
        );
    }

    use super::*;

    /// Begin a turn with a throwaway token and no idempotency key.
    fn begin(state: &AppState, session_id: &str) -> Result<TurnGuard, TurnConflict> {
        state.try_begin_turn_idempotent(session_id, CancellationToken::new(), None)
    }

    /// D2. A route that is about to USE an agent waits for that session's
    /// extensions to finish loading.
    ///
    /// ⚠ This asserts a *wait*, which is why it uses a flag flipped by the task
    /// rather than a duration: a timing assertion would pass on a slow machine
    /// with the wait deleted. Delete either line in `get_agent_for_route` and
    /// the flag is still false when the assertion runs.
    ///
    /// The scenario it stands for: `/agent/start` returns 200 while extensions
    /// are still loading, the client replies immediately, and the turn runs
    /// with a couple of tools instead of its full set — so the model answers
    /// "I cannot delegate", which reads exactly like a real refusal.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_route_waits_for_the_sessions_extensions_before_using_the_agent() {
        let state = AppState::new().await.unwrap();
        let loaded = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let flag = loaded.clone();
        state
            .set_extension_loading_task(
                "s-wait".to_string(),
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                    flag.store(true, std::sync::atomic::Ordering::SeqCst);
                    Vec::new()
                }),
            )
            .await;

        // The agent itself may or may not build here; what is under test is the
        // ordering, so the result is deliberately ignored.
        let _ = state.get_agent_for_route("s-wait".to_string()).await;

        assert!(
            loaded.load(std::sync::atomic::Ordering::SeqCst),
            "returned before the session's extensions had finished loading"
        );
    }

    /// A load that never finishes must NOT park the turn forever.
    ///
    /// ⚠ This is the guard for a real outage: an unbounded `handle.await`
    /// wedged a live daemon for hours. `/reply` waits while holding the
    /// session's turn guard, so one extension that never settles parks the turn
    /// task and every later turn blocks behind the lock. It presents as a
    /// daemon with no network, no CPU and no turn at all, under a GUI that says
    /// "Thinking" indefinitely.
    ///
    /// The test drives a handle that never completes, so it hangs forever if
    /// the timeout is removed rather than failing an assertion. That is the
    /// point: nothing weaker distinguishes "bounded" from "fast enough today".
    #[tokio::test(flavor = "multi_thread")]
    async fn a_wedged_extension_load_does_not_park_the_turn_forever() {
        let state = AppState::new().await.unwrap();
        state
            .set_extension_loading_task(
                "s-wedged".to_string(),
                // Never completes, and is never aborted: exactly a stdio MCP
                // server that spawned and then went silent.
                tokio::spawn(std::future::pending()),
            )
            .await;

        // A short bound, then an outer limit well above it: the inner one is
        // what must fire. Remove the `timeout` in the production path and the
        // outer one trips instead, failing this.
        let waited = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            state.take_extension_loading_task_bounded(
                "s-wedged",
                std::time::Duration::from_millis(150),
            ),
        )
        .await;

        assert!(
            waited.is_ok(),
            "a wedged extension load parked the turn: the wait is unbounded"
        );

        // And the production path really does pass a bound, not `Duration::MAX`.
        // Without this, the test above passes against a wait that is
        // technically finite and effectively forever.
        assert!(
            AppState::EXTENSION_LOAD_WAIT <= std::time::Duration::from_secs(60),
            "the production wait is too long to be a deadlock bound"
        );
    }

    /// The wait is not a leak: the handle is consumed once and the entry drops,
    /// so the second turn of a chat does not re-await anything.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_extension_wait_is_consumed_once() {
        let state = AppState::new().await.unwrap();
        state
            .set_extension_loading_task("s-once".to_string(), tokio::spawn(async { Vec::new() }))
            .await;

        let _ = state.get_agent_for_route("s-once".to_string()).await;
        assert!(
            state.take_extension_loading_task("s-once").await.is_none(),
            "the loading handle outlived the turn that awaited it"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_try_begin_turn_rejects_second_and_recovers_on_drop() {
        let state = AppState::new().await.unwrap();

        let guard = begin(&state, "s1").expect("first turn acquires the lock");

        // A second turn for the same session is rejected with the running id.
        let running = begin(&state, "s1").unwrap_err().running_turn_id;
        assert!(running.starts_with("turn-"), "got id {running}");

        // A different session is independent.
        let _other = begin(&state, "s2").expect("distinct session is unaffected");

        // Dropping the guard releases the session for the next turn.
        drop(guard);
        let _next = begin(&state, "s1").expect("session is free after the guard drops");
    }

    /// BR-62: cancel is addressable by session id. Before, the running turn's
    /// token lived only inside the `/reply` task, so nothing outside that socket
    /// could stop the turn.
    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_turn_trips_the_running_turns_token() {
        let state = AppState::new().await.unwrap();
        let token = CancellationToken::new();

        let _guard = state
            .try_begin_turn_idempotent("s1", token.clone(), None)
            .expect("turn starts");
        assert!(!token.is_cancelled());

        let cancelled = state.cancel_turn("s1").expect("a turn was running");
        assert!(cancelled.starts_with("turn-"), "got id {cancelled}");
        assert!(token.is_cancelled(), "the running turn's token was tripped");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn agent_stop_blocks_successor_and_continuation_admission_until_eviction_finishes() {
        let state = AppState::new().await.unwrap();
        let token = CancellationToken::new();
        let first = state
            .try_begin_turn_idempotent("stop-barrier", token.clone(), None)
            .expect("the first turn starts");
        let first_turn_id = first.turn_id().to_string();

        let stop = state.begin_agent_stop("stop-barrier");
        assert_eq!(stop.cancelled_turn_id(), Some(first_turn_id.as_str()));
        assert!(token.is_cancelled());
        assert!(matches!(
            state.cancel_turn_for_continuation_owned(
                "stop-barrier",
                &first_turn_id,
                "stop-barrier-owner"
            ),
            ContinuationCancelAttempt::ParentClosing
        ));

        drop(first);
        let conflict = state
            .try_begin_turn_idempotent("stop-barrier", CancellationToken::new(), None)
            .expect_err("a successor crossed the stop/eviction barrier");
        assert!(!conflict.duplicate);

        drop(stop);
        state
            .try_begin_turn_idempotent("stop-barrier", CancellationToken::new(), None)
            .expect("admission reopens after the stop operation ends");
    }

    /// The guard may retire between the route tripping its token and registering
    /// its async wait. The retained state bit must make that edge observable;
    /// `Notify` alone would lose a `notify_waiters` sent in this window.
    #[tokio::test(flavor = "multi_thread")]
    async fn retirement_wait_cannot_miss_a_guard_that_already_dropped() {
        let state = AppState::new().await.unwrap();
        let guard = begin(&state, "retire-before-wait").unwrap();
        let turn_id = guard.turn_id().to_string();
        let cancelled = match state.cancel_turn_waitable("retire-before-wait", Some(&turn_id)) {
            CancelTurnAttempt::Cancelled(turn) => turn,
            other => panic!("expected the running turn, got {other:?}"),
        };

        drop(guard);
        tokio::time::timeout(Duration::from_millis(100), cancelled.wait_until_settled())
            .await
            .expect("a retirement notification was lost before the waiter registered");
        assert!(cancelled.is_settled());
    }

    /// A wait handle belongs to a turn generation, not to the session slot. A
    /// successor neither prolongs the old wait nor gets its token tripped.
    #[tokio::test(flavor = "multi_thread")]
    async fn retirement_wait_is_not_confused_by_a_successor() {
        let state = AppState::new().await.unwrap();
        let first = begin(&state, "generation-wait").unwrap();
        let first_id = first.turn_id().to_string();
        let cancelled = match state.cancel_turn_waitable("generation-wait", Some(&first_id)) {
            CancelTurnAttempt::Cancelled(turn) => turn,
            other => panic!("expected the first turn, got {other:?}"),
        };
        drop(first);

        let successor_token = CancellationToken::new();
        let _successor = state
            .try_begin_turn_idempotent("generation-wait", successor_token.clone(), None)
            .unwrap();

        tokio::time::timeout(Duration::from_millis(100), cancelled.wait_until_settled())
            .await
            .expect("the successor incorrectly prolonged the predecessor's wait");
        assert!(state.is_turn_active("generation-wait"));
        assert!(!successor_token.is_cancelled());
    }

    /// Cancelling a session with no turn in flight — a double-clicked Stop, a
    /// cancel that raced the turn's own completion — is a no-op, never an error.
    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_turn_is_idempotent_and_safe_when_idle() {
        let state = AppState::new().await.unwrap();
        let token = CancellationToken::new();

        assert!(state.cancel_turn("s1").is_none(), "nothing to cancel yet");

        let guard = state
            .try_begin_turn_idempotent("s1", token.clone(), None)
            .expect("turn starts");

        // Cancelling twice is fine: the second call finds the same, already
        // tripped token rather than racing the guard's cleanup.
        assert!(state.cancel_turn("s1").is_some());
        assert!(state.cancel_turn("s1").is_some());
        assert!(token.is_cancelled());

        // Once the turn task unwinds and drops its guard, the slot is clear.
        drop(guard);
        assert!(state.cancel_turn("s1").is_none());
    }

    /// A re-POST of the same turn (an SSE reconnect) is recognizable as a
    /// duplicate, so the client can re-attach instead of treating it as a hard
    /// conflict — and, either way, no second turn starts.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_repost_of_the_same_turn_id_is_flagged_duplicate() {
        let state = AppState::new().await.unwrap();

        let _guard = state
            .try_begin_turn_idempotent("s1", CancellationToken::new(), Some("turn-abc".into()))
            .expect("turn starts");

        let conflict = state
            .try_begin_turn_idempotent("s1", CancellationToken::new(), Some("turn-abc".into()))
            .expect_err("no second turn starts");
        assert!(conflict.duplicate, "same key => same turn, re-delivered");

        // A genuinely different turn is a real conflict, not a duplicate.
        let conflict = state
            .try_begin_turn_idempotent("s1", CancellationToken::new(), Some("turn-xyz".into()))
            .expect_err("no second turn starts");
        assert!(!conflict.duplicate);
    }

    /// Two keyless turns are two turns — absence of a key must not be mistaken
    /// for a matching key, or every concurrent `/reply` would look like a retry.
    #[tokio::test(flavor = "multi_thread")]
    async fn keyless_turns_are_never_duplicates_of_each_other() {
        let state = AppState::new().await.unwrap();

        let _guard = state
            .try_begin_turn_idempotent("s1", CancellationToken::new(), None)
            .expect("turn starts");

        let conflict = state
            .try_begin_turn_idempotent("s1", CancellationToken::new(), None)
            .expect_err("no second turn starts");
        assert!(!conflict.duplicate);
    }

    #[tokio::test]
    async fn turn_guard_exposes_its_turn_id() {
        let state = AppState::new().await.unwrap();
        let guard = state
            .try_begin_turn_idempotent("tg-id-test", CancellationToken::new(), None)
            .unwrap();
        assert!(guard.turn_id().starts_with("turn-"));
    }

    /// A guard must also say WHICH session it locked.
    ///
    /// BR-71's runner takes a `TurnRequest` and a `TurnGuard` as separate
    /// arguments, and Tasks 8 and 14 both acquire the guard themselves before
    /// calling `run_turn`. Without an accessor there is nothing — not a type,
    /// not an assertion — stopping `run_turn(state, request_for_B, guard_for_A)`
    /// from compiling and running an unguarded turn on B while holding A's lock.
    #[tokio::test]
    async fn turn_guard_exposes_the_session_it_locked() {
        let state = AppState::new().await.unwrap();
        let guard = state
            .try_begin_turn_idempotent("tg-session-test", CancellationToken::new(), None)
            .unwrap();
        assert_eq!(guard.session_id(), "tg-session-test");
    }

    #[tokio::test]
    async fn a_closed_writer_is_not_advertised_as_an_active_attachable_turn() {
        let state = AppState::new().await.unwrap();
        let guard = state
            .try_begin_turn_idempotent("closed-writer", CancellationToken::new(), None)
            .unwrap();
        let stream = guard.stream();
        let writer = stream
            .claim_writer()
            .expect("the turn owns its stream writer");
        let turn_id = guard.turn_id().to_string();

        assert_eq!(state.active_turn_id("closed-writer"), Some(turn_id));
        drop(writer);

        assert!(stream.is_closed());
        assert!(
            stream.has_writer(),
            "writer ownership remains a permanent latch"
        );
        assert!(!stream.has_live_writer());
        assert!(
            state.is_turn_active("closed-writer"),
            "the guard remains live so this test isolates stream closure"
        );
        assert_eq!(state.active_turn_id("closed-writer"), None);
        drop(guard);
    }

    /// A turn that has ENDED is retired, not deleted — its entry survives so a
    /// re-POST of its idempotency key attaches to the replay instead of starting
    /// a second turn. But a retired entry must be invisible to every "is a turn
    /// running?" reader, or the session would look permanently busy: no new
    /// turn, no steer, and a Stop button reporting it stopped something.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_retired_turn_blocks_nothing_and_is_not_reported_as_running() {
        let state = AppState::new().await.unwrap();
        let token = CancellationToken::new();

        let guard = state
            .try_begin_turn_idempotent("retired", token.clone(), Some("k-1".into()))
            .expect("turn starts");
        assert!(state.is_turn_active("retired"));
        drop(guard);

        assert!(
            !state.is_turn_active("retired"),
            "a finished turn is not active"
        );
        assert!(state.active_turn_id("retired").is_none());
        assert!(!state
            .active_turn_session_ids()
            .contains(&"retired".to_string()));
        assert!(
            state.cancel_turn("retired").is_none(),
            "nothing left to stop"
        );

        // The same key still names the finished turn, so a reconnect is answered
        // from its replay rather than re-running it.
        let conflict = state
            .try_begin_turn_idempotent("retired", CancellationToken::new(), Some("k-1".into()))
            .expect_err("the retired turn is still addressable by its key");
        assert!(conflict.duplicate && conflict.finished);
        // The guard deliberately did NOT close the log — closing there would
        // race the pump's read of the runner's own terminal frame. Closing is
        // the pump's job, or (for a turn with no pump) the retired-attach path's.
        assert!(!conflict.stream.is_closed());

        // A DIFFERENT key is simply the next message, and starts a fresh turn.
        let next = state
            .try_begin_turn_idempotent("retired", CancellationToken::new(), Some("k-2".into()))
            .expect("a retired entry must never block the next turn");
        assert_ne!(next.turn_id(), conflict.running_turn_id);
        assert!(state.is_turn_active("retired"));
    }

    /// The server-assigned id is an attach pointer too — it is what a reloaded
    /// window holds, since it never kept the key it originally chose.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_turn_is_addressable_by_its_server_assigned_id() {
        let state = AppState::new().await.unwrap();
        let guard = state
            .try_begin_turn_idempotent("by-server-id", CancellationToken::new(), Some("k".into()))
            .expect("turn starts");
        let server_id = guard.turn_id().to_string();

        let conflict = state
            .try_begin_turn_idempotent(
                "by-server-id",
                CancellationToken::new(),
                Some(server_id.clone()),
            )
            .expect_err("no second turn starts");
        assert!(
            conflict.duplicate,
            "the id a reloaded window actually holds must identify the turn"
        );
        assert_eq!(conflict.stream.turn_id(), server_id);
    }

    /// A guard may only ever clear its own turn. If a guard outlived its slot and
    /// removed a successor's entry, the session would look idle while a turn ran.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stale_guard_cannot_clear_a_successors_turn() {
        let state = AppState::new().await.unwrap();

        let first = state
            .try_begin_turn_idempotent("s1", CancellationToken::new(), None)
            .expect("first turn starts");
        drop(first);

        let second_token = CancellationToken::new();
        let _second = state
            .try_begin_turn_idempotent("s1", second_token.clone(), None)
            .expect("second turn starts");

        // The second turn still owns the slot and is still cancellable.
        assert!(state.is_turn_active("s1"));
        assert!(state.cancel_turn("s1").is_some());
        assert!(second_token.is_cancelled());
    }

    /// D18: a Stop-and-Send lease its renderer never claims — reloaded, crashed,
    /// its successor `/reply` never sent — expires, so the chat is not wedged
    /// behind `continuation_lease_required` forever and a supervising parent is
    /// released.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_unclaimed_continuation_lease_expires_and_frees_the_chat() {
        let state = AppState::new().await.unwrap();
        // Long enough that the refusal below is observed before it fires; the
        // wait afterwards is a bounded condition, not a guessed sleep.
        state.set_continuation_lease_ttl_for_test(std::time::Duration::from_millis(3_000));
        let retired = begin(&state, "lease-ttl-child").unwrap();
        let retired_id = retired.turn_id().to_string();
        drop(retired);
        let admission = match state.cancel_turn_for_continuation_owned(
            "lease-ttl-child",
            &retired_id,
            "a-window-that-reloaded",
        ) {
            ContinuationCancelAttempt::Retired { admission, .. } => admission,
            other => panic!("expected exact retired admission, got {other:?}"),
        };
        admission.mark().unwrap().commit();
        assert!(state.commit_continuation_lease(admission.token()));

        assert!(
            matches!(
                state.try_begin_turn_idempotent_with_continuation(
                    "lease-ttl-child",
                    CancellationToken::new(),
                    Some("unrelated-send".into()),
                    None,
                ),
                Err(TurnBeginFailure::ContinuationLease(
                    ContinuationLeaseFailure::Required
                ))
            ),
            "while the lease is pending a lease-less send is refused"
        );

        let freed = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                if let Ok(guard) = state.try_begin_turn_idempotent_with_continuation(
                    "lease-ttl-child",
                    CancellationToken::new(),
                    Some("unrelated-send".into()),
                    None,
                ) {
                    return guard;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(freed.is_ok(), "an expired lease must stop wedging the chat");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn continuation_status_returns_the_token_only_to_its_stable_owner() {
        let state = AppState::new().await.unwrap();
        let retired = begin(&state, "owner-bound-child").unwrap();
        let retired_id = retired.turn_id().to_string();
        drop(retired);
        let admission = match state.cancel_turn_for_continuation_owned(
            "owner-bound-child",
            &retired_id,
            "window-owner-a",
        ) {
            ContinuationCancelAttempt::Retired { admission, .. } => admission,
            other => panic!("expected exact retired admission, got {other:?}"),
        };
        let settling = state
            .pending_continuation_for_owner("owner-bound-child", Some("window-owner-a"))
            .unwrap();
        assert_eq!(settling.ownership, PendingContinuationOwnership::Settling);
        assert_eq!(settling.continuation_lease, None);
        assert!(matches!(
            state.cancel_turn_for_continuation_owned(
                "owner-bound-child",
                &retired_id,
                "window-owner-a"
            ),
            ContinuationCancelAttempt::AdmissionInProgress
        ));
        admission.mark().unwrap().commit();
        assert!(state.commit_continuation_lease(admission.token()));

        let owned = state
            .pending_continuation_for_owner("owner-bound-child", Some("window-owner-a"))
            .unwrap();
        assert_eq!(owned.ownership, PendingContinuationOwnership::Owned);
        assert_eq!(owned.continuation_lease.as_deref(), Some(admission.token()));
        let repeated = match state.cancel_turn_for_continuation_owned(
            "owner-bound-child",
            &retired_id,
            "window-owner-a",
        ) {
            ContinuationCancelAttempt::Retired { admission, .. } => admission,
            other => panic!("same-owner retry must reuse its lease, got {other:?}"),
        };
        assert_eq!(repeated.token(), admission.token());
        assert!(repeated.mark().is_none());

        let foreign = state
            .pending_continuation_for_owner("owner-bound-child", Some("window-owner-b"))
            .unwrap();
        assert_eq!(foreign.ownership, PendingContinuationOwnership::Foreign);
        assert_eq!(foreign.continuation_lease, None);
        assert!(matches!(
            state.cancel_turn_for_continuation_owned(
                "owner-bound-child",
                &retired_id,
                "window-owner-b"
            ),
            ContinuationCancelAttempt::OwnerConflict
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn explicit_takeover_is_atomic_and_keeps_one_token_per_owner() {
        let state = AppState::new().await.unwrap();
        let retired = begin(&state, "takeover-child").unwrap();
        let retired_id = retired.turn_id().to_string();
        drop(retired);
        let original = match state.cancel_turn_for_continuation_owned(
            "takeover-child",
            &retired_id,
            "old-window",
        ) {
            ContinuationCancelAttempt::Retired { admission, .. } => admission,
            other => panic!("expected exact retired admission, got {other:?}"),
        };
        original.mark().unwrap().commit();
        assert!(state.commit_continuation_lease(original.token()));

        let recovered = state
            .recover_continuation_for_owner("takeover-child", &retired_id, "new-window")
            .unwrap();
        let ContinuationRecovery::Recovered {
            continuation_lease, ..
        } = recovered
        else {
            panic!("takeover must return its replacement lease")
        };
        assert_ne!(continuation_lease, original.token());
        assert!(matches!(
            state.try_begin_turn_idempotent_with_continuation(
                "takeover-child",
                CancellationToken::new(),
                Some("old-window-successor".into()),
                Some(original.token())
            ),
            Err(TurnBeginFailure::ContinuationLease(
                ContinuationLeaseFailure::Replayed
            ))
        ));

        let same_owner = state
            .recover_continuation_for_owner("takeover-child", &retired_id, "new-window")
            .unwrap();
        assert_eq!(
            same_owner,
            ContinuationRecovery::Recovered {
                continuation_lease: continuation_lease.clone(),
                superseded_turn_id: retired_id.clone(),
            }
        );
        assert_eq!(
            state
                .pending_continuation_for_owner("takeover-child", Some("new-window"))
                .unwrap()
                .continuation_lease
                .as_deref(),
            Some(continuation_lease.as_str())
        );
        let successor = state
            .try_begin_turn_idempotent_with_continuation(
                "takeover-child",
                CancellationToken::new(),
                Some("recovered-successor".into()),
                Some(&continuation_lease),
            )
            .expect("the recovered owner admits exactly one successor");
        assert!(state
            .pending_continuation_for_owner("takeover-child", Some("new-window"))
            .is_none());
        assert!(matches!(
            state.try_begin_turn_idempotent_with_continuation(
                "takeover-child",
                CancellationToken::new(),
                Some("different-successor".into()),
                Some(&continuation_lease)
            ),
            Err(TurnBeginFailure::ContinuationLease(
                ContinuationLeaseFailure::Replayed
            ))
        ));
        drop(successor);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn closing_parent_refuses_a_new_continuation_admission() {
        let state = AppState::new().await.unwrap();
        let parent = "closing-admission-parent";
        let child = "closing-admission-child";
        let _handle = biorouter::agents::subagent_handle::BackgroundSubagent::register(
            parent,
            child,
            "closing child",
            CancellationToken::new(),
        );
        let turn = begin(&state, child).unwrap();

        biorouter::agents::subagent_handle::begin_parent_closing(parent);
        assert!(matches!(
            state.cancel_turn_for_continuation_owned(child, turn.turn_id(), "window"),
            ContinuationCancelAttempt::ParentClosing
        ));
        assert!(state.is_turn_active(child));
        assert!(state
            .pending_continuation_for_owner(child, Some("window"))
            .is_none());

        biorouter::agents::subagent_handle::open_parent_continuation_admission(parent);
        drop(turn);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn closing_parent_takeover_refusal_preserves_the_existing_owner() {
        let state = AppState::new().await.unwrap();
        let parent = "closing-takeover-parent";
        let child = "closing-takeover-child";
        let _handle = biorouter::agents::subagent_handle::BackgroundSubagent::register(
            parent,
            child,
            "closing takeover child",
            CancellationToken::new(),
        );
        let retired = begin(&state, child).unwrap();
        let retired_id = retired.turn_id().to_string();
        drop(retired);
        let original =
            match state.cancel_turn_for_continuation_owned(child, &retired_id, "original-window") {
                ContinuationCancelAttempt::Retired { admission, .. } => admission,
                other => panic!("expected retired admission, got {other:?}"),
            };
        original.mark().unwrap().commit();
        assert!(state.commit_continuation_lease(original.token()));

        biorouter::agents::subagent_handle::begin_parent_closing(parent);
        assert_eq!(
            state.recover_continuation_for_owner(child, &retired_id, "new-window"),
            Err(ContinuationLeaseFailure::ParentClosing)
        );
        let pending = state
            .pending_continuation_for_owner(child, Some("original-window"))
            .expect("a refused takeover cannot destroy the existing owner's lease");
        assert_eq!(pending.ownership, PendingContinuationOwnership::Owned);
        assert_eq!(
            pending.continuation_lease.as_deref(),
            Some(original.token())
        );

        biorouter::agents::subagent_handle::open_parent_continuation_admission(parent);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn takeover_wins_atomically_against_a_delayed_cancel_admission_commit() {
        let state = AppState::new().await.unwrap();
        let retired = begin(&state, "reserved-takeover-child").unwrap();
        let retired_id = retired.turn_id().to_string();
        drop(retired);
        let delayed = match state.cancel_turn_for_continuation_owned(
            "reserved-takeover-child",
            &retired_id,
            "departed-window",
        ) {
            ContinuationCancelAttempt::Retired { admission, .. } => admission,
            other => panic!("expected exact retired admission, got {other:?}"),
        };

        let recovered = state
            .recover_continuation_for_owner(
                "reserved-takeover-child",
                &retired_id,
                "replacement-window",
            )
            .unwrap();
        delayed.mark().unwrap().commit();
        assert!(
            !state.commit_continuation_lease(delayed.token()),
            "the delayed cancel response must not resurrect or return its superseded token"
        );
        let ContinuationRecovery::Recovered {
            continuation_lease, ..
        } = recovered
        else {
            panic!("takeover must return its replacement lease")
        };
        assert_eq!(
            state
                .pending_continuation_for_owner(
                    "reserved-takeover-child",
                    Some("replacement-window")
                )
                .unwrap()
                .continuation_lease
                .as_deref(),
            Some(continuation_lease.as_str())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn explicit_group_abandon_invalidates_every_claim_for_the_exact_generation() {
        let state = AppState::new().await.unwrap();
        let retired = begin(&state, "group-abandon-child").unwrap();
        let retired_id = retired.turn_id().to_string();
        drop(retired);
        let admission = match state.cancel_turn_for_continuation_owned(
            "group-abandon-child",
            &retired_id,
            "departed-window",
        ) {
            ContinuationCancelAttempt::Retired { admission, .. } => admission,
            other => panic!("expected exact retired admission, got {other:?}"),
        };
        admission.mark().unwrap().commit();
        assert!(state.commit_continuation_lease(admission.token()));

        assert!(matches!(
            state
                .abandon_continuation_group("group-abandon-child", &retired_id)
                .unwrap(),
            ContinuationRecovery::Abandoned { .. }
        ));
        assert!(matches!(
            state
                .abandon_continuation_group("group-abandon-child", &retired_id)
                .unwrap(),
            ContinuationRecovery::Abandoned { .. }
        ));
        assert!(state
            .pending_continuation_for_owner("group-abandon-child", Some("departed-window"))
            .is_none());
        assert!(matches!(
            state.try_begin_turn_idempotent_with_continuation(
                "group-abandon-child",
                CancellationToken::new(),
                Some("late-successor".into()),
                Some(admission.token())
            ),
            Err(TurnBeginFailure::ContinuationLease(
                ContinuationLeaseFailure::Replayed
            ))
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_continuation_lease_is_exact_session_bound_and_single_successor_only() {
        let state = AppState::new().await.unwrap();
        let retired = state
            .try_begin_turn_idempotent(
                "lease-child",
                CancellationToken::new(),
                Some("superseded-client-turn".into()),
            )
            .unwrap();
        let retired_id = retired.turn_id().to_string();
        drop(retired);
        let admission = match state.cancel_turn_for_continuation_owned(
            "lease-child",
            &retired_id,
            "lease-test-owner",
        ) {
            ContinuationCancelAttempt::Retired { admission, .. } => admission,
            other => panic!("expected retained exact generation, got {other:?}"),
        };
        if let Some(mark) = admission.mark() {
            mark.commit();
        }
        state.commit_continuation_lease(admission.token());
        let lease = admission.token().to_string();

        assert!(matches!(
            state.try_begin_turn_idempotent_with_continuation(
                "lease-child",
                CancellationToken::new(),
                Some("successor-client-turn".into()),
                Some("not-a-lease")
            ),
            Err(TurnBeginFailure::ContinuationLease(
                ContinuationLeaseFailure::Invalid
            ))
        ));
        assert!(matches!(
            state.try_begin_turn_idempotent_with_continuation(
                "other-child",
                CancellationToken::new(),
                Some("successor-client-turn".into()),
                Some(&lease)
            ),
            Err(TurnBeginFailure::ContinuationLease(
                ContinuationLeaseFailure::CrossSession
            ))
        ));
        assert!(matches!(
            state.try_begin_turn_idempotent_with_continuation(
                "lease-child",
                CancellationToken::new(),
                Some("successor-client-turn".into()),
                None
            ),
            Err(TurnBeginFailure::ContinuationLease(
                ContinuationLeaseFailure::Required
            ))
        ));

        let successor = state
            .try_begin_turn_idempotent_with_continuation(
                "lease-child",
                CancellationToken::new(),
                Some("successor-client-turn".into()),
                Some(&lease),
            )
            .expect("the exact session and retired generation consume the lease");
        assert!(matches!(
            state.try_begin_turn_idempotent_with_continuation(
                "lease-child",
                CancellationToken::new(),
                Some("successor-client-turn".into()),
                Some(&lease)
            ),
            Err(TurnBeginFailure::Conflict(TurnConflict {
                duplicate: true,
                ..
            }))
        ));
        assert!(matches!(
            state.try_begin_turn_idempotent_with_continuation(
                "lease-child",
                CancellationToken::new(),
                Some("different-successor".into()),
                Some(&lease)
            ),
            Err(TurnBeginFailure::ContinuationLease(
                ContinuationLeaseFailure::Replayed
            ))
        ));
        drop(successor);
        assert_eq!(
            state
                .abandon_continuation_lease("lease-child", &lease)
                .unwrap(),
            ContinuationLeaseAbandonment::AlreadyConsumed
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn abandoning_a_live_lease_is_idempotent_and_releases_supervision() {
        let state = AppState::new().await.unwrap();
        let retired = begin(&state, "abandoned-lease-child").unwrap();
        let retired_id = retired.turn_id().to_string();
        drop(retired);
        let handle = biorouter::agents::subagent_handle::BackgroundSubagent::register(
            "abandoned-lease-parent",
            "abandoned-lease-child",
            "delegated work",
            CancellationToken::new(),
        );
        biorouter::agents::subagent_handle::begin_child_turn("abandoned-lease-child");
        handle.complete(
            biorouter::agents::subagent_result::SubagentResult::from_error("original result"),
        );
        let admission = match state.cancel_turn_for_continuation_owned(
            "abandoned-lease-child",
            &retired_id,
            "abandon-test-owner",
        ) {
            ContinuationCancelAttempt::Retired { admission, .. } => admission,
            other => panic!("expected retained exact generation, got {other:?}"),
        };
        admission.mark().unwrap().commit();
        state.commit_continuation_lease(admission.token());
        assert!(handle.continuation_pending());

        assert_eq!(
            state
                .abandon_continuation_lease("abandoned-lease-child", admission.token())
                .unwrap(),
            ContinuationLeaseAbandonment::Abandoned
        );
        assert_eq!(
            state
                .abandon_continuation_lease("abandoned-lease-child", admission.token())
                .unwrap(),
            ContinuationLeaseAbandonment::AlreadyAbandoned
        );
        assert!(!handle.continuation_pending());
        assert!(handle.result_is_current());
        let _ordinary_successor = begin(&state, "abandoned-lease-child")
            .expect("abandonment must release the replacement admission barrier");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stopping_a_child_abandons_every_pending_continuation_for_the_session() {
        let state = AppState::new().await.unwrap();
        let child = "stopped-continuation-child";
        let retired = begin(&state, child).unwrap();
        let retired_id = retired.turn_id().to_string();
        drop(retired);
        let handle = biorouter::agents::subagent_handle::BackgroundSubagent::register(
            "stopped-continuation-parent",
            child,
            "delegated work",
            CancellationToken::new(),
        );
        biorouter::agents::subagent_handle::begin_child_turn(child);
        handle.complete(
            biorouter::agents::subagent_result::SubagentResult::from_error("original result"),
        );
        let admission = match state.cancel_turn_for_continuation_owned(
            child,
            &retired_id,
            "stopped-child-window",
        ) {
            ContinuationCancelAttempt::Retired { admission, .. } => admission,
            other => panic!("expected retained exact generation, got {other:?}"),
        };
        admission.mark().unwrap().commit();
        assert!(state.commit_continuation_lease(admission.token()));
        assert!(handle.continuation_pending());

        assert!(state.abandon_pending_continuations_for_session(child));
        assert!(!handle.continuation_pending());
        assert!(handle.result_is_current());
        assert!(state
            .pending_continuation_for_owner(child, Some("stopped-child-window"))
            .is_none());
        assert!(!state.abandon_pending_continuations_for_session(child));
        assert!(matches!(
            state.try_begin_turn_idempotent_with_continuation(
                child,
                CancellationToken::new(),
                Some("stale-successor".into()),
                Some(admission.token())
            ),
            Err(TurnBeginFailure::ContinuationLease(
                ContinuationLeaseFailure::Replayed
            ))
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_successor_that_never_reaches_child_start_cannot_strand_supervision() {
        let state = AppState::new().await.unwrap();
        let retired = begin(&state, "unstarted-successor-child").unwrap();
        let retired_id = retired.turn_id().to_string();
        drop(retired);
        let handle = biorouter::agents::subagent_handle::BackgroundSubagent::register(
            "unstarted-successor-parent",
            "unstarted-successor-child",
            "delegated work",
            CancellationToken::new(),
        );
        biorouter::agents::subagent_handle::begin_child_turn("unstarted-successor-child");
        handle.complete(
            biorouter::agents::subagent_result::SubagentResult::from_error("original result"),
        );
        let admission = match state.cancel_turn_for_continuation_owned(
            "unstarted-successor-child",
            &retired_id,
            "unstarted-test-owner",
        ) {
            ContinuationCancelAttempt::Retired { admission, .. } => admission,
            other => panic!("expected retained exact generation, got {other:?}"),
        };
        admission.mark().unwrap().commit();
        state.commit_continuation_lease(admission.token());
        let successor = state
            .try_begin_turn_idempotent_with_continuation(
                "unstarted-successor-child",
                CancellationToken::new(),
                Some("replacement-client-turn".into()),
                Some(admission.token()),
            )
            .unwrap();
        assert!(handle.continuation_pending());

        drop(successor);
        assert!(!handle.continuation_pending());
        assert_eq!(
            state
                .abandon_continuation_lease("unstarted-successor-child", admission.token())
                .unwrap(),
            ContinuationLeaseAbandonment::AlreadyAbandoned
        );
    }
}
