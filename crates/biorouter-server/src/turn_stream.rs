//! The live turn stream: one sequence-numbered, replayable frame log per turn,
//! with 0..N simultaneous observers.
//!
//! # Why this exists
//!
//! Before this module a turn's output went to a single `mpsc::channel(100)` —
//! the body of the `POST /reply` that started it. Exactly one consumer. When
//! that consumer went away (window closed, tab moved, renderer reloaded), the
//! next `tx.send` failed and the handler called `cancel_token.cancel()`, so
//! **"nobody is listening" and "stop working" were the same event**. Nothing
//! buffered, so nothing could catch up: the turn belonged to an HTTP *request*
//! rather than to a *session*.
//!
//! A `TurnStream` is that missing session-scoped object. One pump (the task
//! `/reply` spawns) maps the session bus into frames and appends them here;
//! every HTTP response that wants to watch the turn takes a [`StreamReader`]
//! and gets the whole log from seq 0, then the live tail. Readers come and go
//! without the turn noticing.
//!
//! # The two binding requirements
//!
//! **R1 — no progress is ever lost.** Every frame is appended to a replay
//! buffer sized to hold a whole turn ([`REPLAY_BYTE_BUDGET`]). Past that size
//! the buffer evicts, and eviction is **restricted to frames the session store
//! has already absorbed** — the ones, and only the ones, that the storage
//! resync can put back. A reader whose prefix was evicted is told
//! ([`StreamReader::take_gap`]) and `/reply` prepends a whole-conversation
//! snapshot read from the store before the retained tail.
//!
//! R1 therefore holds exactly, with one stated bound rather than an assumed
//! one: a turn is only ever short of its own opening past
//! [`REPLAY_HARD_BYTE_CEILING`], reached only by a SINGLE un-persisted message
//! of ~280k streamed deltas — beyond any provider's context window. See that
//! constant for why the earlier one-level design silently violated R1 on a long
//! answer.
//!
//! **R2 — seamless in the UI.** Frames carry a monotonic per-turn `seq` so a
//! client can apply them idempotently after re-attaching, and replayed frames
//! carry `"replay": true` so the client can tell a backlog from the live tail.
//! The backlog is written to the socket as fast as it will take it — never
//! re-paced at the original speed.
//!
//! # Wire format
//!
//! Every frame is one SSE `data:` line holding the existing
//! [`crate::routes::reply::MessageEvent`] JSON object, with up to three fields
//! added at the top level:
//!
//! | field     | on                          | meaning |
//! |-----------|-----------------------------|---------|
//! | `seq`     | every logged frame          | monotonic per-turn index, from 0 |
//! | `turn_id` | every logged frame          | the server-assigned turn this belongs to |
//! | `replay`  | replayed frames only        | `true` — already-seen history, apply idempotently |
//!
//! A frame with **no `seq`** is connection-local and carries no ordering
//! guarantee: the `Ping` heartbeat, and the storage-resync `UpdateConversation`
//! emitted ahead of a gapped backlog. A client keyed on `seq` must ignore
//! ordering for those and simply apply them.
//!
//! # Concurrency
//!
//! All state lives behind one `std::sync::Mutex` held for a few instructions at
//! a time and never across an `await`. [`TurnStream::publish`] appends to the
//! replay buffer **and** broadcasts under that one lock, and
//! [`TurnStream::attach`] subscribes **and** snapshots the buffer under it too.
//! That pairing is what makes an attach atomic: a frame published after the
//! subscribe is guaranteed to arrive on the broadcast, and one published before
//! it is guaranteed to be in the snapshot. Cloning the sender out and
//! subscribing afterwards would open a window in which a frame is in neither.
//!
//! # Who may end a turn's log — ONE owner, ONE critical section
//!
//! This is the module's most expensive lesson, learned three times: a turn's
//! terminal was closed out from under its own writer in `TurnGuard::drop`, then
//! again from `drain_stream_to_client`, then again by `close`'s own
//! check-then-act. Each instance was individually defensible and each cost a
//! healthy turn its real `Finish`. The rules that replace those three patches:
//!
//! 1. **A log has at most one writer**, taken with [`TurnStream::claim_writer`]
//!    and held as a [`TurnWriter`] for the writer's whole life. Its `Drop`
//!    closes the log, so a pump that returns, breaks, is cancelled mid-`await`
//!    or PANICS still ends the turn exactly once.
//! 2. **Nobody else closes.** A reader that finds [`TurnStream::has_writer`]
//!    false knows no frame is coming and answers its client from what is there;
//!    it never mutates the log to make that true. "This turn has no writer" and
//!    "this turn's writer has not finished draining" are now different
//!    questions with different answers, instead of both being inferred from
//!    `terminal_frame().is_none()`.
//! 3. **Appending and closing share one critical section** ([`append_locked`]),
//!    so a terminal cannot be duplicated by, or lost to, a concurrent publish.
//! 4. **Nothing follows the terminal.** `publish` refuses once `terminal_seq`
//!    is set, which is what makes [`TurnStream::terminal_frame`] able to name an
//!    ending rather than hand back whatever landed last.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::routes::reply::MessageEvent;

/// How much serialized frame text one turn may retain for replay **before
/// eviction starts** — and eviction at this level may only drop frames the
/// session store has already absorbed.
///
/// A turn is bounded by the context window, so a normal one — even a long
/// tool-heavy one — is a few hundred KB of frames. 8 MiB is roughly an order of
/// magnitude above that, chosen so that R1 holds *without* the storage fallback
/// for every turn a user will actually run, while still bounding the pathology
/// (an un-coalesced 100k-token answer is one frame per provider chunk and can
/// reach tens of MB). At most one turn per session is live, so the real ceiling
/// is [`REPLAY_HARD_BYTE_CEILING`] × concurrent chats.
pub const REPLAY_BYTE_BUDGET: usize = 8 * 1024 * 1024;

/// Frame-count companion to [`REPLAY_BYTE_BUDGET`], so a turn that emits an
/// enormous number of tiny frames is bounded too.
pub const REPLAY_FRAME_BUDGET: usize = 200_000;

/// The point past which the replay buffer evicts frames that are **not** yet
/// persisted — accepting a hole and reporting it as a [`ReaderEvent::Gap`].
///
/// Why two levels rather than one. The module used to claim that "the evicted
/// prefix is exactly the part that has already been persisted, and the
/// un-persisted part (the running assistant message) is exactly the part that is
/// retained". That is false for a single assistant message whose own frames
/// exceed the budget: it evicts its OWN earlier deltas, and the storage resync
/// cannot restore them, because the message is un-persisted *precisely because
/// it is still streaming*. The client is then handed the tail of a message whose
/// opening nothing on the machine still holds — R1 failing silently.
///
/// So the soft budget above is now a *persisted-only* eviction level: it drops
/// exactly the frames whose rows storage already has, which is the only eviction
/// the storage fallback can actually repair. This ceiling is the bound on the
/// pathology, and it is set where no real turn can reach it: at ~120 bytes per
/// streamed delta, 32 MiB is ~280k deltas in ONE un-persisted message, which is
/// an order of magnitude beyond any context window a provider offers. R1 holds
/// for every turn that can physically be produced; past this the gap is reported
/// rather than hidden.
pub const REPLAY_HARD_BYTE_CEILING: usize = 32 * 1024 * 1024;

/// Frame-count companion to [`REPLAY_HARD_BYTE_CEILING`], in the same 4:1 ratio
/// as the byte levels.
pub const REPLAY_HARD_FRAME_CEILING: usize = REPLAY_FRAME_BUDGET * 4;

/// What a *finished* turn keeps, once its result is persisted.
///
/// The whole log is no longer needed: everything a completed turn produced is
/// in the session store, so a late attach can be answered from storage plus the
/// terminal frame. Keeping a smaller tail means the common case (a turn well
/// under this size) is still replayed exactly, without a five-minute retention
/// window pinning 8 MiB per recently-finished chat.
pub const CLOSED_REPLAY_BYTE_BUDGET: usize = 2 * 1024 * 1024;

/// Broadcast ring for the live tail. Deliberately modest: a reader that falls
/// behind it is repaired from the replay buffer, which is the authoritative
/// record, so this only has to absorb ordinary scheduling jitter.
const LIVE_RING_CAPACITY: usize = 512;

/// How long a turn may run with **zero** observers before it is treated as
/// abandoned and cancelled.
///
/// The number this must beat is a window reload, measured at ~4.6 s. Five
/// minutes is ~65× that, which also comfortably covers a laptop sleep/resume, a
/// network blip, a renderer crash-and-restart, and a user dragging a tab
/// between windows and getting distracted. It is short enough that a genuinely
/// abandoned turn cannot spend money indefinitely: the worst case is five more
/// minutes of tokens, and every *deliberate* stop — the Stop button,
/// `POST /agent/cancel`, session teardown — is still instant.
///
/// Overridable with `BIOROUTER_TURN_ORPHAN_TIMEOUT_MS` so tests can use a
/// sub-second value; production never sets it.
pub const DEFAULT_ORPHAN_TIMEOUT: Duration = Duration::from_secs(300);

/// The orphan reaper's poll interval, capped so a short test timeout is still
/// observed promptly.
fn reaper_tick(timeout: Duration) -> Duration {
    (timeout / 10).clamp(Duration::from_millis(20), Duration::from_secs(5))
}

/// How long a reader may be unable to hand a frame to its client before it
/// stops counting as somebody watching (see [`StreamReader::mark_stalled`]).
///
/// `observers` used to count ATTACHMENTS, which is not the same question the
/// orphan reaper asks. A client that is connected but not draining — a frozen
/// renderer, a suspended VM, a half-open TCP connection, a zero-window peer —
/// never drops its receiver, so the send never fails, the 100-slot response
/// channel fills, the drain parks inside `send` forever, and `observers` stays
/// at one. Nobody is watching and nothing can end the turn.
///
/// A second is far beyond any healthy loopback delivery (the whole backlog is
/// written in one or two reads) and far below the five-minute orphan timeout, so
/// crossing it costs a legitimately busy renderer nothing: the reader is still
/// attached, still parked in the same send, and re-counts itself the moment the
/// frame lands. All that changes is that the reaper's clock is allowed to start.
pub const OBSERVER_STALL_GRACE: Duration = Duration::from_secs(1);

/// [`DEFAULT_ORPHAN_TIMEOUT`], or the `BIOROUTER_TURN_ORPHAN_TIMEOUT_MS`
/// override. An unparseable or zero value falls back to the default rather than
/// reaping instantly — a typo in an env var must not start killing live turns.
pub fn orphan_timeout() -> Duration {
    std::env::var("BIOROUTER_TURN_ORPHAN_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_ORPHAN_TIMEOUT)
}

/// One logged frame: its per-turn sequence number and the serialized JSON
/// object (already carrying `seq` and `turn_id`).
#[derive(Clone, Debug)]
pub struct SeqFrame {
    pub seq: u64,
    json: Arc<str>,
}

impl SeqFrame {
    /// The SSE bytes for a LIVE delivery of this frame.
    pub fn live_sse(&self) -> String {
        format!("data: {}\n\n", self.json)
    }

    /// The SSE bytes for a REPLAYED delivery: the same object with
    /// `"replay":true` spliced in as its first member.
    ///
    /// String surgery rather than a re-serialize, and it is exact: `json` is
    /// always a JSON *object*, so it always starts with `{`, and member order
    /// is not significant. `serde_json` emits no whitespace after the brace, so
    /// the only case needing care is an empty object, which takes no separator.
    /// Pinned by `replay_marker_is_spliced_in_exactly`.
    pub fn replay_sse(&self) -> String {
        let Some(rest) = self.json.strip_prefix('{') else {
            // Unreachable — `encode_frame` only ever produces an object. Degrade
            // to a live delivery rather than emit a corrupt frame; a client that
            // re-renders one frame is a far smaller failure than one that
            // cannot parse the stream at all.
            tracing::error!("a logged frame was not a JSON object; replay marker skipped");
            return self.live_sse();
        };
        let separator = if rest.starts_with('}') { "" } else { "," };
        format!("data: {{\"replay\":true{separator}{rest}\n\n")
    }
}

/// What a reader saw next.
#[derive(Debug)]
pub enum ReaderEvent {
    /// A frame, and whether it came from the replay buffer rather than live.
    Frame(SeqFrame, bool),
    /// The reader fell so far behind that the frames it still needed have been
    /// evicted. The consumer must resync from storage before continuing; the
    /// reader has already skipped forward to the oldest frame still retained.
    Gap,
    /// The turn is over and every frame has been delivered.
    Closed,
}

/// The mutable half of a [`TurnStream`].
#[derive(Debug)]
struct Inner {
    next_seq: u64,
    replay: VecDeque<SeqFrame>,
    replay_bytes: usize,
    /// The lowest seq still retained. Anything below this was evicted, and a
    /// reader asking for it gets [`ReaderEvent::Gap`].
    oldest_seq: u64,
    /// The seq of this turn's terminal frame, once one has been logged. `Some`
    /// is what makes the log FINAL: [`TurnStream::publish`] refuses everything
    /// after it, so the terminal is always the last frame in the log and
    /// [`TurnStream::terminal_frame`] can name it exactly.
    terminal_seq: Option<u64>,
    /// The seq of the newest frame whose rows the session store has already
    /// absorbed (a `MessagesPersisted`). Everything at or below it is
    /// recoverable from storage, which is what makes evicting it safe; see
    /// [`REPLAY_HARD_BYTE_CEILING`].
    persisted_through: Option<u64>,
    closed: bool,
    /// True once somebody has taken responsibility for WRITING this turn's log
    /// — publishing its frames and closing it when the turn ends.
    ///
    /// The distinction the code lacked. A `TurnStream` is minted by the turn
    /// LOCK (`state.rs`), which several callers take for reasons that have
    /// nothing to do with streaming: an in-place edit and a working-directory
    /// change hold it as a plain mutex, and the workspace/app turn runners hold
    /// it without a `/reply` pump. Those turns' logs have no writer at all, and
    /// an observer attaching to one would park forever on frames that will never
    /// come and an end that will never be signalled. Inferring "no writer" from
    /// "no frames yet" or "no terminal yet" is exactly the mistake that let a
    /// late attach close a healthy turn's log out from under its pump.
    writer_claimed: bool,
    /// Live readers right now. Zero is an ordinary state, not an error.
    observers: usize,
    /// True once anything has EVER attached. The orphan reaper will not fire
    /// before this: a turn started by a caller that never intended to watch it
    /// (an injected workspace turn) has no orphan to reap.
    ever_attached: bool,
    /// When `observers` last fell to zero, if it is zero now. `None` while
    /// somebody is watching.
    idle_since: Option<Instant>,
}

/// A turn's frame log: append-only, sequence-numbered, replayable, fanned out.
#[derive(Debug)]
pub struct TurnStream {
    session_id: String,
    turn_id: String,
    tx: broadcast::Sender<SeqFrame>,
    inner: Mutex<Inner>,
    /// Mirrors `Inner::observers` for cheap, lock-free introspection in logs
    /// and tests. Authoritative count stays under the lock.
    observers: AtomicU64,
    /// D17: set by the orphan reaper just before it cancels, so the turn's
    /// terminal frame can say `orphaned` rather than read as a Stop.
    reaped: std::sync::atomic::AtomicBool,
    /// How many decisions the orphan reaper has made. A lower bound on how long
    /// it has been watching, which is what lets a test prove it did NOT reap
    /// without guessing a sleep.
    reaper_ticks: AtomicU64,
    /// D17: the agent a person steered this turn through. While its queue holds
    /// a steer the loop has not read, the turn owes that person an answer and
    /// is not an orphan. Weak: the stream must never keep an agent alive.
    steered: Mutex<SteeredAgent>,
}

/// The agent behind an accepted steer, held weakly (see `TurnStream::steered`).
#[derive(Default)]
struct SteeredAgent(Option<std::sync::Weak<biorouter::agents::Agent>>);

impl std::fmt::Debug for SteeredAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SteeredAgent")
            .field(&self.0.is_some())
            .finish()
    }
}

impl TurnStream {
    pub fn new(session_id: impl Into<String>, turn_id: impl Into<String>) -> Arc<Self> {
        let (tx, _) = broadcast::channel(LIVE_RING_CAPACITY);
        Arc::new(Self {
            session_id: session_id.into(),
            turn_id: turn_id.into(),
            tx,
            inner: Mutex::new(Inner {
                next_seq: 0,
                replay: VecDeque::new(),
                replay_bytes: 0,
                oldest_seq: 0,
                terminal_seq: None,
                persisted_through: None,
                closed: false,
                writer_claimed: false,
                observers: 0,
                ever_attached: false,
                idle_since: None,
            }),
            observers: AtomicU64::new(0),
            reaped: std::sync::atomic::AtomicBool::new(false),
            reaper_ticks: AtomicU64::new(0),
            steered: Mutex::new(SteeredAgent::default()),
        })
    }

    /// See the `reaper_ticks` field.
    pub fn reaper_ticks(&self) -> u64 {
        self.reaper_ticks.load(Ordering::Acquire)
    }

    /// Whether the orphan reaper ended this turn (D17).
    pub fn was_reaped(&self) -> bool {
        self.reaped.load(Ordering::Acquire)
    }

    /// A person steered this turn through `agent` without attaching to it (D17).
    /// Restarts the orphan clock when nobody is reading, and remembers the agent
    /// so the reaper leaves the turn alone for as long as that steer is still
    /// queued: a turn holding a person's unread words is not abandoned.
    pub fn note_user_activity(&self, agent: &Arc<biorouter::agents::Agent>) {
        *self
            .steered
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            SteeredAgent(Some(Arc::downgrade(agent)));
        let mut inner = self.lock();
        if inner.observers == 0 {
            inner.idle_since = Some(Instant::now());
        }
    }

    /// Whether a steer accepted into this turn is still waiting for the loop.
    /// Takes the agent's own queue lock, so never call it holding `inner`.
    fn steer_still_owed(&self) -> bool {
        let agent = self
            .steered
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .0
            .as_ref()
            .and_then(std::sync::Weak::upgrade);
        agent.is_some_and(|agent| agent.has_soft_interrupts())
    }

    /// Poisoning cannot leave this logically inconsistent — every critical
    /// section below is a handful of infallible field updates — so a panic
    /// elsewhere must not turn every live turn into a dead stream.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn turn_id(&self) -> &str {
        &self.turn_id
    }

    pub fn observer_count(&self) -> u64 {
        self.observers.load(Ordering::Relaxed)
    }

    pub fn is_closed(&self) -> bool {
        self.lock().closed
    }

    /// Frames logged so far (tests/introspection).
    pub fn next_seq(&self) -> u64 {
        self.lock().next_seq
    }

    /// This turn's terminal frame, once it has one.
    ///
    /// The whole answer for a client re-POSTing a turn that has already
    /// finished: its transcript was read back from the session store and already
    /// contains the turn, so all it still needs is the fact that the turn ended.
    ///
    /// Named by `terminal_seq`, never by "the last retained frame". The old
    /// version returned `replay.back()` on the premise that nothing can follow a
    /// terminal — a premise `publish` did not enforce, so a `Message` published
    /// after an `Error` (the runner panics, the supervisor logs its
    /// `internal_error`, the pump drains events queued before the panic) was
    /// handed back as this turn's ending. A client re-POSTing that id then
    /// received one `Message` and an ended response: never told the turn was
    /// over, thinking forever. `publish` now refuses post-terminal frames, and
    /// this reads the seq rather than a position, so neither half can drift.
    pub fn terminal_frame(&self) -> Option<SeqFrame> {
        let inner = self.lock();
        let seq = inner.terminal_seq?;
        inner.replay.iter().rev().find(|f| f.seq == seq).cloned()
    }

    /// Has somebody taken responsibility for writing (and ending) this log?
    ///
    /// The question every attach has to ask. `false` means no frame will ever
    /// arrive and no terminal will ever be logged, so following this stream is
    /// waiting for something that cannot happen — see [`Inner::writer_claimed`].
    pub fn has_writer(&self) -> bool {
        self.lock().writer_claimed
    }

    /// Is this log both owned by a writer and still open for live delivery?
    ///
    /// `writer_claimed` is intentionally a permanent ownership latch, so it
    /// cannot by itself distinguish a running pump from one whose writer has
    /// already dropped and closed the stream.
    pub fn has_live_writer(&self) -> bool {
        let inner = self.lock();
        inner.writer_claimed && !inner.closed
    }

    /// Take ownership of this turn's log. `None` when somebody already has it.
    ///
    /// The returned [`TurnWriter`] closes the log when it drops, which is the
    /// point: a pump cannot exit — return, break, be cancelled mid-`await`, or
    /// PANIC — without the log ending. Before this, `stream.close()` sat at the
    /// bottom of the pump's body, so a panicking pump left every attached
    /// response parked on a turn that was already dead, and the orphan reaper
    /// could not help because the only consumer of its token was the pump that
    /// was gone.
    pub fn claim_writer(self: &Arc<Self>) -> Option<TurnWriter> {
        let mut inner = self.lock();
        if inner.writer_claimed {
            return None;
        }
        inner.writer_claimed = true;
        drop(inner);
        Some(TurnWriter {
            stream: Arc::clone(self),
        })
    }

    /// Append one frame and fan it out. Returns its sequence number.
    ///
    /// Never blocks and never fails: a stream with no observers is an ordinary
    /// state, and a broadcast send with no receivers is a no-op. This is the
    /// single most important property in the module — it is what makes "nobody
    /// is listening" stop meaning "stop working".
    ///
    /// Refused once the log is CLOSED **or once its terminal has been logged**.
    /// Both halves matter: the second is what makes "the terminal is the last
    /// frame" true rather than merely intended.
    pub fn publish(&self, event: &MessageEvent) -> u64 {
        let mut inner = self.lock();
        append_locked(&mut inner, &self.tx, &self.turn_id, event)
    }

    /// End the turn's log, in ONE critical section.
    ///
    /// Guarantees a terminal frame exists: a pump that exits on cancellation
    /// without ever seeing the runner's `TurnFinished` would otherwise leave a
    /// late-attaching client waiting forever for an end that never comes. The
    /// synthesized frame is an `Error`, not a `Finish`, because a stream that
    /// ended without its runner saying why did not finish normally, and telling
    /// a client "stop, all good" when the turn's fate is unknown is the worse
    /// lie of the two.
    ///
    /// ⚠ **The lock is not released between the check and the act, and must not
    /// be.** This used to read `!closed && !terminal_logged`, drop the lock,
    /// publish the synthetic terminal, then re-take the lock to latch `closed`.
    /// Two writers exist on every turn (the pump and `supervise_turn`), so a
    /// real terminal landing in that window either produced two terminals or —
    /// when close won the race — was refused as post-terminal, and a perfectly
    /// healthy turn ended in "The stream for this turn ended without a result".
    /// A barrier test measured that at 3000 runs out of 3000.
    pub fn close(&self) {
        let mut inner = self.lock();
        if inner.closed {
            return;
        }
        if inner.terminal_seq.is_none() {
            append_locked(
                &mut inner,
                &self.tx,
                &self.turn_id,
                &MessageEvent::error(
                    "The stream for this turn ended without a result. Please retry.",
                    "stream_ended_without_terminal",
                    crate::routes::reply::TurnErrorScope::Internal,
                    true,
                    None,
                ),
            );
        }
        inner.closed = true;
        // A finished turn's output is in the store, so its retained log can be
        // trimmed hard (see CLOSED_REPLAY_BYTE_BUDGET). Still only the PERSISTED
        // prefix, on the same rule as during the turn: a turn that ended without
        // ever publishing a `MessagesPersisted` has nothing in storage to fall
        // back to, and trimming it here would lose the very frames a client
        // attaching a moment later still needs.
        evict_to_budget(
            &mut inner,
            CLOSED_REPLAY_BYTE_BUDGET,
            REPLAY_HARD_BYTE_CEILING,
            REPLAY_FRAME_BUDGET,
            REPLAY_HARD_FRAME_CEILING,
        );
        drop(inner);
        // Wake every parked reader so it observes `closed` and ends its
        // response instead of blocking on a channel nothing will send to.
        let _ = self.tx.send(SeqFrame {
            seq: u64::MAX,
            json: Arc::from("{}"),
        });
    }

    /// Attach an observer that already holds frames `0..from_seq`.
    ///
    /// Subscribes and snapshots the replay buffer under one lock, so no frame
    /// can fall between the backlog and the live tail.
    ///
    /// `from_seq` is CLAMPED to the frames the turn actually has. It is a
    /// client-supplied field on a public route, and an out-of-range value used
    /// to silence the whole turn for that observer: every later frame was below
    /// `next`, so each was dropped as "already delivered" and `next` never
    /// walked back. Honouring the hint is an optimisation; honouring it out of
    /// range is not optional.
    pub fn attach(self: &Arc<Self>, from_seq: u64) -> StreamReader {
        let mut inner = self.lock();
        let from_seq = from_seq.min(inner.next_seq);
        let rx = self.tx.subscribe();
        let gap = from_seq < inner.oldest_seq;
        let backlog: VecDeque<SeqFrame> = inner
            .replay
            .iter()
            .filter(|f| f.seq >= from_seq)
            .cloned()
            .collect();
        let next = backlog.front().map_or(from_seq, |f| f.seq);

        inner.observers += 1;
        inner.ever_attached = true;
        inner.idle_since = None;
        self.observers
            .store(inner.observers as u64, Ordering::Relaxed);
        let closed = inner.closed;
        drop(inner);

        StreamReader {
            stream: Arc::clone(self),
            rx,
            backlog,
            next,
            gap,
            closed,
            counted: true,
        }
    }

    /// Cancel the turn once it has been observerless for `timeout`.
    ///
    /// The reaper is the ONLY thing that ends a turn because of its audience,
    /// and it is deliberately generous — see [`DEFAULT_ORPHAN_TIMEOUT`]. It
    /// will not fire on a turn nothing ever attached to, and it exits as soon
    /// as the stream closes, so a completed turn never trips it.
    pub fn spawn_orphan_reaper(
        self: &Arc<Self>,
        cancel: CancellationToken,
        timeout: Duration,
    ) -> tokio::task::JoinHandle<()> {
        let stream = Arc::clone(self);
        let tick = reaper_tick(timeout);
        tokio::spawn(async move {
            let mut supervision_was_active = false;
            loop {
                tokio::select! {
                    () = cancel.cancelled() => return,
                    () = tokio::time::sleep(tick) => {}
                }
                // The decision AND the cancel happen under one lock. Reading the
                // idle time, releasing the lock and only then logging (real I/O)
                // and cancelling left a window in which an `attach()` — a user
                // reopening the window at exactly the wrong moment — was
                // answered and then had its turn killed underneath it. `attach`
                // takes this same lock, so it either lands before the re-check
                // (and the turn lives) or after the cancel (and reads a
                // cancelled turn, whose pump closes the log and answers it).
                let mut supervising_delegated_work = false;
                let mut stream_closed = false;
                // Sampled before the stream's lock: the agent's queue has its own.
                let steer_owed = stream.steer_still_owed();
                let reaped = biorouter::agents::subagent_handle::with_live_delegated_supervision(
                    &stream.session_id,
                    |supervised| {
                        supervising_delegated_work = supervised;
                        let supervision_just_settled = supervision_was_active && !supervised;
                        let mut inner = stream.lock();
                        if inner.closed {
                            stream_closed = true;
                            return false;
                        }
                        // A parent and child are supervised through an exact
                        // live handle generation, not through either turn's SSE
                        // audience. The registry decision remains locked until
                        // this cancel decision is complete, so a new generation
                        // cannot land after a stale "not supervised" sample.
                        if (supervised || supervision_just_settled) && inner.observers == 0 {
                            inner.idle_since = Some(Instant::now());
                        }
                        // D17: a turn parked on a card is waiting for a PERSON,
                        // not abandoned. Reaping it would cancel the turn out
                        // from under an approval the person may be about to give
                        // from another surface (a second window, the CLI). The
                        // card's own time-to-live still bounds the wait.
                        if inner.observers == 0
                            && biorouter::pending_user_action::PendingUserActions::global()
                                .has_pending_in_session(&stream.session_id)
                        {
                            inner.idle_since = Some(Instant::now());
                        }
                        // The same for a steer the loop has not read yet: the
                        // person is owed an answer, and the clock restarts from
                        // the moment the loop takes it.
                        if steer_owed && inner.observers == 0 {
                            inner.idle_since = Some(Instant::now());
                        }
                        let idle_long_enough = inner.ever_attached
                            && inner.observers == 0
                            && inner
                                .idle_since
                                .is_some_and(|since| since.elapsed() >= timeout);
                        if idle_long_enough {
                            stream.reaped.store(true, Ordering::Release);
                            cancel.cancel();
                        }
                        idle_long_enough
                    },
                );
                stream.reaper_ticks.fetch_add(1, Ordering::AcqRel);
                if stream_closed {
                    return;
                }
                if reaped {
                    tracing::warn!(
                        counter.biorouter.turn_orphan_reaped = 1,
                        session_id = %stream.session_id,
                        turn_id = %stream.turn_id,
                        timeout_ms = timeout.as_millis() as u64,
                        "no client has observed this turn for the orphan timeout; cancelling"
                    );
                    return;
                }
                supervision_was_active = supervising_delegated_work;
            }
        })
    }

    /// Stop counting one observer, recording when the audience fell to zero.
    fn drop_observer(&self) {
        let mut inner = self.lock();
        inner.observers = inner.observers.saturating_sub(1);
        if inner.observers == 0 {
            inner.idle_since = Some(Instant::now());
        }
        self.observers
            .store(inner.observers as u64, Ordering::Relaxed);
    }

    /// Count one observer again (a stalled reader whose client started reading).
    fn add_observer(&self) {
        let mut inner = self.lock();
        inner.observers += 1;
        inner.ever_attached = true;
        inner.idle_since = None;
        self.observers
            .store(inner.observers as u64, Ordering::Relaxed);
    }
}

/// Ownership of a turn's log: the right to write it, and the duty to end it.
///
/// Held by the turn's pump for the pump's whole life. `Drop` closes the log, so
/// EVERY way out — a clean return, a `break`, cancellation dropping the future
/// mid-`await`, or a panic unwinding the task — ends the turn's log exactly
/// once. Terminal-and-close has one owner; this is it.
#[derive(Debug)]
pub struct TurnWriter {
    stream: Arc<TurnStream>,
}

impl TurnWriter {
    /// The log this writer owns.
    pub fn stream(&self) -> &Arc<TurnStream> {
        &self.stream
    }
}

impl Drop for TurnWriter {
    fn drop(&mut self) {
        self.stream.close();
    }
}

/// Append one frame under a lock the caller already holds, and fan it out.
///
/// The ONE place a frame enters the log — `publish` and `close`'s synthetic
/// terminal both come through here — so "nothing follows the terminal" is a
/// property of a single function rather than an agreement between two.
fn append_locked(
    inner: &mut Inner,
    tx: &broadcast::Sender<SeqFrame>,
    turn_id: &str,
    event: &MessageEvent,
) -> u64 {
    if inner.closed || inner.terminal_seq.is_some() {
        // The turn is over; nothing may be appended after its terminal frame or
        // a late attach would read past the end of the turn — and
        // `terminal_frame()` would hand back something that is not an ending.
        return inner.next_seq;
    }
    let seq = inner.next_seq;
    let json: Arc<str> = Arc::from(encode_frame(event, seq, turn_id));
    let frame = SeqFrame { seq, json };

    inner.next_seq += 1;
    if matches!(
        event,
        MessageEvent::Finish { .. } | MessageEvent::Error { .. }
    ) {
        inner.terminal_seq = Some(seq);
    }
    if matches!(event, MessageEvent::MessagesPersisted { .. }) {
        // Everything logged up to here is now recoverable from the session
        // store, which is what makes evicting it safe.
        inner.persisted_through = Some(seq);
    }
    inner.replay_bytes += frame.json.len();
    inner.replay.push_back(frame.clone());
    evict_to_budget(
        inner,
        REPLAY_BYTE_BUDGET,
        REPLAY_HARD_BYTE_CEILING,
        REPLAY_FRAME_BUDGET,
        REPLAY_HARD_FRAME_CEILING,
    );

    // Under the same lock as the append — see the module docs.
    let _ = tx.send(frame);
    seq
}

/// Drop frames from the front until the buffer fits, remembering how far the
/// eviction reached so a reader asking for an evicted seq gets a
/// [`ReaderEvent::Gap`] rather than a silently short stream.
///
/// Two levels, and the difference between them is R1 (see
/// [`REPLAY_HARD_BYTE_CEILING`]):
///
///  - at the SOFT budget only frames at or below `persisted_through` are
///    dropped, because those — and only those — are the ones the storage resync
///    can actually put back;
///  - at the HARD ceiling anything goes, and the resulting hole is reported as a
///    gap rather than hidden.
fn evict_to_budget(
    inner: &mut Inner,
    soft_bytes: usize,
    hard_bytes: usize,
    soft_frames: usize,
    hard_frames: usize,
) {
    let persisted_through = inner.persisted_through;
    // Soft pass: recoverable frames only.
    while (inner.replay_bytes > soft_bytes || inner.replay.len() > soft_frames)
        && inner.replay.len() > 1
        && inner
            .replay
            .front()
            .is_some_and(|f| persisted_through.is_some_and(|p| f.seq <= p))
    {
        if let Some(dropped) = inner.replay.pop_front() {
            inner.replay_bytes -= dropped.json.len();
            inner.oldest_seq = dropped.seq + 1;
        }
    }
    // Hard pass: the bound on the pathology. Unreachable for any turn a
    // provider can actually produce, and honest — the reader is told.
    while (inner.replay_bytes > hard_bytes || inner.replay.len() > hard_frames)
        && inner.replay.len() > 1
    {
        if let Some(dropped) = inner.replay.pop_front() {
            inner.replay_bytes -= dropped.json.len();
            inner.oldest_seq = dropped.seq + 1;
        }
    }
}

/// Serialize one frame and stamp it with its sequence number and turn id.
///
/// The `seq`/`turn_id` fields are ADDED to the existing `MessageEvent` object
/// rather than wrapping it in an envelope, so every client that parses today's
/// frames keeps parsing them unchanged.
fn encode_frame(event: &MessageEvent, seq: u64, turn_id: &str) -> String {
    let mut value = serde_json::to_value(event).unwrap_or_else(|e| {
        serde_json::json!({
            "type": "Error",
            "error": format!("Failed to serialize stream event: {e}"),
            "code": "stream_serialization_failed",
            "scope": "internal",
            "retryable": false,
        })
    });
    if let Some(object) = value.as_object_mut() {
        object.insert("seq".to_string(), serde_json::json!(seq));
        object.insert("turn_id".to_string(), serde_json::json!(turn_id));
    }
    value.to_string()
}

/// One observer of a [`TurnStream`]. Reclaims its slot on drop, which is what
/// the orphan reaper counts.
#[derive(Debug)]
pub struct StreamReader {
    stream: Arc<TurnStream>,
    rx: broadcast::Receiver<SeqFrame>,
    backlog: VecDeque<SeqFrame>,
    /// The next sequence number this reader still needs. Frames below it are
    /// dropped, which is what makes a re-attach idempotent.
    next: u64,
    gap: bool,
    closed: bool,
    /// Whether this reader is currently counted in `Inner::observers`. A reader
    /// whose client has stopped draining stops counting (see
    /// [`Self::mark_stalled`]) without detaching, so `Drop` must know which.
    counted: bool,
}

impl StreamReader {
    /// True when the frames this reader asked for had already been evicted, so
    /// the consumer must recover the prefix from storage (R1's fallback).
    /// Cleared once reported.
    pub fn take_gap(&mut self) -> bool {
        std::mem::take(&mut self.gap)
    }

    /// This reader's client has not accepted a frame for
    /// [`OBSERVER_STALL_GRACE`]: stop counting it as somebody watching.
    ///
    /// Not a detach — the reader stays attached and keeps trying, so nothing is
    /// dropped and no client is disconnected. All that changes is that the
    /// orphan reaper's clock is allowed to start on a turn whose only audience
    /// is a socket nobody is reading.
    pub fn mark_stalled(&mut self) {
        if !self.counted {
            return;
        }
        self.counted = false;
        self.stream.drop_observer();
    }

    /// This reader's client accepted a frame: it is watching again.
    pub fn mark_active(&mut self) {
        if self.counted {
            return;
        }
        self.counted = true;
        self.stream.add_observer();
    }

    pub fn turn_id(&self) -> &str {
        self.stream.turn_id()
    }

    /// The next frame, from the backlog first and then live.
    ///
    /// Returns [`ReaderEvent::Closed`] once the turn is over and the backlog is
    /// drained — never a hang, which is what makes attaching to an
    /// already-finished turn safe.
    pub async fn recv(&mut self) -> ReaderEvent {
        loop {
            if let Some(frame) = self.backlog.pop_front() {
                self.next = frame.seq + 1;
                return ReaderEvent::Frame(frame, true);
            }
            if self.closed {
                // A gap discovered on the way to the end is still a gap. Report
                // it before ending, or a client whose prefix was evicted is left
                // silently short of it with no resync — the one repair R1's
                // fallback exists to trigger.
                if self.gap {
                    return ReaderEvent::Gap;
                }
                return ReaderEvent::Closed;
            }
            match self.rx.recv().await {
                Ok(frame) => {
                    if frame.seq == u64::MAX {
                        // The close wake-up. Re-snapshot: frames may have been
                        // appended (the synthesized terminal) after our last read.
                        self.refill();
                        continue;
                    }
                    if frame.seq < self.next {
                        continue; // already delivered — idempotent by construction
                    }
                    if frame.seq > self.next {
                        // A frame went missing between the ring and us; repair
                        // from the replay buffer rather than skipping it.
                        self.refill();
                        continue;
                    }
                    self.next = frame.seq + 1;
                    return ReaderEvent::Frame(frame, false);
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // The replay buffer is the authoritative record; re-read
                    // from it instead of resyncing the whole conversation.
                    self.refill();
                    if self.gap {
                        return ReaderEvent::Gap;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => {
                    self.refill();
                    if self.backlog.is_empty() {
                        return ReaderEvent::Closed;
                    }
                }
            }
        }
    }

    /// Re-snapshot the replay buffer from `next`, recording whether the frames
    /// we needed have been evicted.
    fn refill(&mut self) {
        let inner = self.stream.lock();
        if self.next < inner.oldest_seq {
            self.gap = true;
        }
        self.backlog = inner
            .replay
            .iter()
            .filter(|f| f.seq >= self.next)
            .cloned()
            .collect();
        if let Some(front) = self.backlog.front() {
            self.next = front.seq;
        }
        self.closed = inner.closed;
    }
}

impl Drop for StreamReader {
    fn drop(&mut self) {
        if self.counted {
            self.stream.drop_observer();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use biorouter::conversation::message::{Message, TokenState};

    fn text_frame(text: &str) -> MessageEvent {
        MessageEvent::Message {
            message: Message::assistant().with_id("m-1").with_text(text),
            token_state: TokenState::default(),
        }
    }

    fn finish() -> MessageEvent {
        MessageEvent::Finish {
            turn_id: None,
            reason: "stop".to_string(),
            token_state: TokenState::default(),
        }
    }

    fn seq_of(sse: &str) -> u64 {
        let json = sse.strip_prefix("data: ").unwrap().trim_end();
        serde_json::from_str::<serde_json::Value>(json).unwrap()["seq"]
            .as_u64()
            .unwrap()
    }

    /// Frames are numbered from 0, monotonically, and the number reaches the
    /// wire — without which a client cannot dedupe and R2 is unachievable.
    #[tokio::test]
    async fn frames_are_numbered_from_zero_and_the_number_reaches_the_wire() {
        let stream = TurnStream::new("s", "turn-1");
        assert_eq!(stream.publish(&text_frame("a")), 0);
        assert_eq!(stream.publish(&text_frame("b")), 1);
        assert_eq!(stream.publish(&finish()), 2);
        // Without this the reader below would correctly park on the live tail
        // forever: a stream that has logged a terminal is not the same thing as
        // a stream that has been CLOSED, and only the pump closes one.
        stream.close();

        let mut reader = stream.attach(0);
        let mut seqs = Vec::new();
        while let ReaderEvent::Frame(frame, _) = reader.recv().await {
            assert_eq!(seq_of(&frame.live_sse()), frame.seq);
            seqs.push(frame.seq);
        }
        assert_eq!(seqs, vec![0, 1, 2]);
    }

    /// The replay marker is spliced in without disturbing the object, so a
    /// client parses the same frame either way.
    #[test]
    fn replay_marker_is_spliced_in_exactly() {
        let stream = TurnStream::new("s", "turn-1");
        stream.publish(&text_frame("hello"));
        let frame = stream.lock().replay[0].clone();

        let live: serde_json::Value =
            serde_json::from_str(frame.live_sse().strip_prefix("data: ").unwrap().trim_end())
                .unwrap();
        let replayed: serde_json::Value = serde_json::from_str(
            frame
                .replay_sse()
                .strip_prefix("data: ")
                .unwrap()
                .trim_end(),
        )
        .unwrap();

        assert!(live.get("replay").is_none(), "a live frame is not replay");
        assert_eq!(replayed["replay"], serde_json::json!(true));
        assert_eq!(replayed["type"], "Message");
        assert_eq!(replayed["seq"], serde_json::json!(0));
        assert_eq!(replayed["turn_id"], "turn-1");
        // Everything else is byte-identical.
        for (key, value) in live.as_object().unwrap() {
            assert_eq!(&replayed[key], value, "field {key} changed under replay");
        }

        // The two degenerate shapes the splice has to survive: an empty object
        // (a stray comma would make it invalid JSON) and a non-object (which
        // must degrade, not corrupt).
        let empty = SeqFrame {
            seq: 0,
            json: Arc::from("{}"),
        };
        assert_eq!(empty.replay_sse(), "data: {\"replay\":true}\n\n");
        let not_an_object = SeqFrame {
            seq: 0,
            json: Arc::from("\"scalar\""),
        };
        assert_eq!(not_an_object.replay_sse(), not_an_object.live_sse());
    }

    /// Publishing with nobody attached is an ordinary no-op — the property the
    /// whole fix rests on.
    #[tokio::test]
    async fn publishing_with_no_observers_is_ordinary_and_replayable_later() {
        let stream = TurnStream::new("s", "turn-1");
        assert_eq!(stream.observer_count(), 0);
        for i in 0..10 {
            stream.publish(&text_frame(&format!("chunk-{i}")));
        }
        let mut reader = stream.attach(0);
        let mut seen = 0;
        while let ReaderEvent::Frame(_, replay) = reader.recv().await {
            assert!(replay, "everything published before the attach is replay");
            seen += 1;
            if seen == 10 {
                break;
            }
        }
        assert_eq!(seen, 10);
    }

    /// Two readers of one turn see every frame, in identical order.
    #[tokio::test]
    async fn two_simultaneous_readers_see_identical_frames() {
        let stream = TurnStream::new("s", "turn-1");
        let a = stream.attach(0);
        let b = stream.attach(0);
        assert_eq!(stream.observer_count(), 2);

        for i in 0..5 {
            stream.publish(&text_frame(&format!("t{i}")));
        }
        stream.publish(&finish());
        stream.close();

        async fn drain(mut r: StreamReader) -> Vec<u64> {
            let mut out = Vec::new();
            while let ReaderEvent::Frame(f, _) = r.recv().await {
                out.push(f.seq);
            }
            out
        }
        let (from_a, from_b) = tokio::join!(drain(a), drain(b));
        assert_eq!(from_a, vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(
            from_a, from_b,
            "two observers of one turn must see identical frames in identical order"
        );
    }

    /// `from_seq` lets a client that already holds a prefix ask only for the
    /// rest — the cheap path for a reconnect that lost nothing.
    #[tokio::test]
    async fn from_seq_skips_frames_the_client_already_holds() {
        let stream = TurnStream::new("s", "turn-1");
        for i in 0..4 {
            stream.publish(&text_frame(&format!("t{i}")));
        }
        stream.close();
        let mut reader = stream.attach(2);
        let mut seqs = Vec::new();
        while let ReaderEvent::Frame(f, _) = reader.recv().await {
            seqs.push(f.seq);
        }
        // 4 is the synthesized terminal `close` appended (no Finish was logged).
        assert_eq!(seqs, vec![2, 3, 4]);
    }

    /// Attaching to a turn that is already over ENDS, promptly. A hang here is
    /// the failure the contract calls out by name.
    #[tokio::test]
    async fn attaching_after_the_turn_ended_never_hangs() {
        let stream = TurnStream::new("s", "turn-1");
        stream.publish(&text_frame("all of it"));
        stream.publish(&finish());
        stream.close();

        let drained = tokio::time::timeout(Duration::from_secs(5), async {
            let mut reader = stream.attach(0);
            let mut out = Vec::new();
            loop {
                match reader.recv().await {
                    ReaderEvent::Frame(f, replay) => out.push((f.seq, replay)),
                    ReaderEvent::Gap => {}
                    ReaderEvent::Closed => break,
                }
            }
            out
        })
        .await
        .expect("a late attach must terminate, not block on a turn that is over");
        assert_eq!(drained, vec![(0, true), (1, true)]);
    }

    /// `close` never leaves a stream without a terminal frame, or a late
    /// attacher waits forever for an end that already happened.
    #[tokio::test]
    async fn close_synthesizes_a_terminal_when_the_pump_died_without_one() {
        let stream = TurnStream::new("s", "turn-1");
        stream.publish(&text_frame("half an answer"));
        stream.close();

        let mut reader = stream.attach(0);
        let mut kinds = Vec::new();
        while let ReaderEvent::Frame(f, _) = reader.recv().await {
            let json: serde_json::Value =
                serde_json::from_str(f.live_sse().strip_prefix("data: ").unwrap().trim_end())
                    .unwrap();
            kinds.push(json["type"].as_str().unwrap().to_string());
        }
        assert_eq!(kinds, vec!["Message", "Error"]);
    }

    /// Nothing may be appended after the terminal, or a late attach would read
    /// past the end of the turn.
    #[tokio::test]
    async fn a_closed_stream_refuses_further_frames() {
        let stream = TurnStream::new("s", "turn-1");
        stream.publish(&finish());
        stream.close();
        let before = stream.next_seq();
        stream.publish(&text_frame("too late"));
        assert_eq!(stream.next_seq(), before);
    }

    /// Eviction drops the OLDEST frames and reports a gap, so the consumer
    /// knows to recover the prefix from storage rather than shipping a hole.
    #[tokio::test]
    async fn overrunning_the_budget_evicts_the_prefix_and_reports_a_gap() {
        let stream = TurnStream::new("s", "turn-1");
        for i in 0..64 {
            stream.publish(&text_frame(&format!("chunk-{i}")));
        }
        {
            let mut inner = stream.lock();
            inner.persisted_through = Some(u64::MAX);
            evict_to_budget(&mut inner, 0, 0, usize::MAX, usize::MAX);
        }
        let mut reader = stream.attach(0);
        assert!(reader.take_gap(), "an evicted prefix must be reported");
        assert!(
            !reader.take_gap(),
            "…and reported once, so it is not resynced on every frame"
        );
        match reader.recv().await {
            ReaderEvent::Frame(f, _) => assert_eq!(
                f.seq, 63,
                "the newest frame, the un-persisted tail, is what survives"
            ),
            other => panic!("expected the retained tail, got {other:?}"),
        }
    }

    /// The reaper fires on a turn nobody is watching…
    #[tokio::test(flavor = "multi_thread")]
    async fn the_orphan_reaper_cancels_a_turn_with_no_observers() {
        let stream = TurnStream::new("s", "turn-1");
        let cancel = CancellationToken::new();
        let reader = stream.attach(0);
        let reaper = stream.spawn_orphan_reaper(cancel.clone(), Duration::from_millis(120));

        drop(reader); // the last window closes
        tokio::time::timeout(Duration::from_secs(5), cancel.cancelled())
            .await
            .expect("an abandoned turn must be reaped");
        reaper.abort();
    }

    /// …and does NOT fire across a gap the length of a window reload, which is
    /// the whole reason the timeout is minutes rather than seconds.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_orphan_reaper_survives_a_reload_length_gap() {
        let stream = TurnStream::new("s", "turn-1");
        let cancel = CancellationToken::new();
        // 5 s stands in for the production 5 min; the gap below stands in for a
        // measured ~4.6 s reload scaled by the same factor.
        let reaper = stream.spawn_orphan_reaper(cancel.clone(), Duration::from_millis(2_000));

        let reader = stream.attach(0);
        drop(reader);
        tokio::time::sleep(Duration::from_millis(300)).await;
        let _reattached = stream.attach(0);
        tokio::time::sleep(Duration::from_millis(400)).await;

        assert!(
            !cancel.is_cancelled(),
            "a reload-length gap must not reap a live turn"
        );
        reaper.abort();
    }

    /// D17: a turn parked on an approval card is waiting for a PERSON, and is
    /// not reaped for having no reader. The person may answer from a second
    /// window or the CLI; the card's own time-to-live bounds the wait.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_turn_parked_on_a_card_is_not_reaped() {
        use biorouter::pending_user_action::{
            PendingUserActions, ToolApprovalRequest, UserActionRequest,
        };
        let session_id = "d17-parked-card";
        let stream = TurnStream::new(session_id, "turn-card");
        let cancel = CancellationToken::new();
        let timeout = Duration::from_millis(60);
        let reaper = stream.spawn_orphan_reaper(cancel.clone(), timeout);
        drop(stream.attach(0)); // the only reader leaves
        let _card = PendingUserActions::global().park(
            Some(session_id),
            None,
            UserActionRequest::ToolApproval(ToolApprovalRequest {
                tool_name: "developer__shell".into(),
                arguments: serde_json::Map::new(),
                prompt: None,
                risk: None,
                preview: None,
                requires_user_proof: false,
            }),
        );

        // Ten decisions at a 20 ms tick is well past the 60 ms timeout. Counted,
        // not slept: each tick is a decision the reaper actually made.
        tokio::time::timeout(Duration::from_secs(20), async {
            while stream.reaper_ticks() < 10 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the reaper keeps deciding");
        assert!(
            !cancel.is_cancelled(),
            "a turn waiting on a person's answer must not be reaped"
        );
        assert!(!stream.was_reaped());
        reaper.abort();
    }

    /// D17: an accepted steer restarts the orphan clock, so the turn the person
    /// just steered from a surface that is not reading it is not reaped for
    /// having no reader.
    #[tokio::test(flavor = "multi_thread")]
    async fn user_activity_restarts_the_orphan_clock_and_a_reap_is_recorded() {
        let stream = TurnStream::new("d17-activity", "turn-activity");
        let cancel = CancellationToken::new();
        let timeout = Duration::from_millis(1_000);
        let reaper = stream.spawn_orphan_reaper(cancel.clone(), timeout);
        drop(stream.attach(0));
        // An agent with no turn open: its queue is empty, so only the clock
        // restart can keep this turn alive.
        let agent = Arc::new(biorouter::agents::Agent::new());
        // Touch the turn before each of 25 reaper decisions (a 100 ms tick, so
        // well past the timeout), synchronised on the decisions themselves.
        for _ in 0..25 {
            stream.note_user_activity(&agent);
            let seen = stream.reaper_ticks();
            tokio::time::timeout(Duration::from_secs(20), async {
                while stream.reaper_ticks() == seen {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("the reaper keeps deciding");
        }
        assert!(!cancel.is_cancelled(), "activity must keep the turn alive");
        // Then leave it alone: it is reaped, and says so.
        tokio::time::timeout(Duration::from_secs(20), cancel.cancelled())
            .await
            .expect("an untouched orphan is still reaped");
        assert!(
            stream.was_reaped(),
            "the reap is recorded for the terminal frame"
        );
        reaper.abort();
    }

    /// D17: a steer accepted into a turn nobody is reading is a person's words
    /// still owed an answer. The turn is not reaped while the loop has not read
    /// them, however long that takes, and the clock only starts once it has.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_turn_holding_an_unread_steer_is_not_reaped() {
        let stream = TurnStream::new("d17-owed", "turn-owed");
        let cancel = CancellationToken::new();
        let timeout = Duration::from_millis(1_000);
        let reaper = stream.spawn_orphan_reaper(cancel.clone(), timeout);
        drop(stream.attach(0));
        let agent = Arc::new(biorouter::agents::Agent::new());
        agent.prepare_soft_interrupt_turn();
        agent
            .try_queue_soft_interrupt("also check the controls".into(), None)
            .expect("the prepared turn accepts a steer");
        stream.note_user_activity(&agent);
        // Forty reaper decisions at a 100 ms tick: four timeouts' worth, with the
        // steer still queued and nobody reading.
        // A reap ends the reaper, so stop waiting on either outcome.
        let start = stream.reaper_ticks();
        tokio::time::timeout(Duration::from_secs(30), async {
            while stream.reaper_ticks() < start + 40 && !cancel.is_cancelled() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the reaper keeps deciding");
        assert!(
            !cancel.is_cancelled(),
            "a turn holding an unread steer must not be reaped"
        );
        // The loop takes the steer (here: the next turn clears the queue); from
        // then on an unread turn is an orphan again.
        agent.prepare_soft_interrupt_turn();
        tokio::time::timeout(Duration::from_secs(20), cancel.cancelled())
            .await
            .expect("once the steer is read, an unread turn is still reaped");
        assert!(stream.was_reaped());
        reaper.abort();
    }

    /// A turn nothing ever attached to is not an orphan — an injected workspace
    /// turn has no window to lose.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_never_attached_turn_is_not_reaped() {
        let stream = TurnStream::new("s", "turn-1");
        let cancel = CancellationToken::new();
        let reaper = stream.spawn_orphan_reaper(cancel.clone(), Duration::from_millis(50));
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(!cancel.is_cancelled());
        reaper.abort();
    }

    /// Attach-before-detach: during a tab handoff both windows are attached at
    /// once, so there is no instant with zero observers and nothing to catch up
    /// on.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_handoff_with_overlapping_readers_never_goes_idle() {
        let stream = TurnStream::new("s", "turn-1");
        let cancel = CancellationToken::new();
        let reaper = stream.spawn_orphan_reaper(cancel.clone(), Duration::from_millis(80));

        let old_window = stream.attach(0);
        let new_window = stream.attach(0); // attaches BEFORE the old one leaves
        drop(old_window);
        assert_eq!(stream.observer_count(), 1);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !cancel.is_cancelled(),
            "an overlapping handoff must never be seen as abandonment"
        );
        drop(new_window);
        reaper.abort();
    }

    #[test]
    fn a_persisted_subagent_without_a_live_handle_has_no_audience_exemption() {
        let supervised = biorouter::agents::subagent_handle::with_live_delegated_supervision(
            "persisted-subagent-without-live-handle",
            |supervised| supervised,
        );
        assert!(!supervised);
    }

    #[test]
    fn unsettled_child_generations_keep_both_turns_supervised() {
        let parent = "turn-stream-supervised-parent";
        let child = "turn-stream-supervised-child";
        let handle = biorouter::agents::subagent_handle::BackgroundSubagent::register(
            parent,
            child,
            "supervised work",
            CancellationToken::new(),
        );
        let supervised = |session_id: &str| {
            biorouter::agents::subagent_handle::with_live_delegated_supervision(
                session_id,
                |supervised| supervised,
            )
        };
        assert!(supervised(parent));
        assert!(supervised(child));

        let generation = handle.child_turn_generation();
        handle.complete(
            biorouter::agents::subagent_result::SubagentResult::from_error("test terminal"),
        );
        assert!(
            supervised(parent) && supervised(child),
            "a finished but uncollected result still belongs to the parent's supervision gate"
        );
        assert!(handle.mark_current_result_collected_if_generation(generation));
        assert!(!supervised(parent));
        assert!(!supervised(child));
    }

    /// A reader that falls off the live ring is repaired from the replay
    /// buffer, in order and with no hole — the buffer, not the ring, is the
    /// record.
    #[tokio::test]
    async fn a_reader_that_falls_off_the_ring_is_repaired_from_the_buffer() {
        let stream = TurnStream::new("s", "turn-1");
        let mut reader = stream.attach(0);
        for i in 0..(LIVE_RING_CAPACITY + 32) {
            stream.publish(&text_frame(&format!("t{i}")));
        }
        stream.close();

        let mut seqs = Vec::new();
        loop {
            match reader.recv().await {
                ReaderEvent::Frame(f, _) => seqs.push(f.seq),
                ReaderEvent::Gap => panic!("the replay buffer was big enough; no gap expected"),
                ReaderEvent::Closed => break,
            }
        }
        let expected: Vec<u64> = (0..(LIVE_RING_CAPACITY as u64 + 33)).collect();
        assert_eq!(seqs, expected, "no frame may be skipped on a ring overrun");
    }

    #[test]
    fn a_bad_orphan_timeout_env_falls_back_rather_than_reaping_instantly() {
        // Not a `set_var` test (process-global, races the suite): the parse is
        // exercised directly through the same predicate the reader uses.
        let parse = |raw: &str| {
            raw.trim()
                .parse::<u64>()
                .ok()
                .filter(|ms| *ms > 0)
                .map(Duration::from_millis)
                .unwrap_or(DEFAULT_ORPHAN_TIMEOUT)
        };
        assert_eq!(parse("0"), DEFAULT_ORPHAN_TIMEOUT);
        assert_eq!(parse("banana"), DEFAULT_ORPHAN_TIMEOUT);
        assert_eq!(parse("250"), Duration::from_millis(250));
        assert_eq!(orphan_timeout(), DEFAULT_ORPHAN_TIMEOUT);
    }
}
