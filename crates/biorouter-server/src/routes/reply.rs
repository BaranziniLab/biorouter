use crate::state::{
    AppState, CancelTurnAttempt, CancelledTurn, ContinuationAdmission, ContinuationCancelAttempt,
    ContinuationLeaseAbandonment, ContinuationLeaseFailure, ContinuationRecovery, TurnBeginFailure,
};
use crate::turn_stream::TurnStream;
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{self, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use biorouter::agents::{InterruptRefused, PersistedMessage, ReasoningEffort};
use biorouter::conversation::message::{Message, MessageContent, TokenState};
use biorouter::conversation::Conversation;
use biorouter::privacy::SessionClassification;
use biorouter::session::session_manager::ReplaceOutcome;
use biorouter::session::SessionManager;
// Through the LIB path, as every route names the user-action digest: `src/routes/`
// is compiled into the `biorouterd` binary too, where `crate::auth` does not
// exist, and the digest must be the lib's one static.
use biorouter_server::auth::{user_action_proof, UserActionProof};
use bytes::Bytes;
use futures::Stream;
use rmcp::model::ServerNotification;
use serde::{Deserialize, Serialize};
use std::{
    convert::Infallible,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;

pub(crate) fn track_tool_telemetry(content: &MessageContent, all_messages: &[Message]) {
    match content {
        MessageContent::ToolRequest(tool_request) => {
            if let Ok(tool_call) = &tool_request.tool_call {
                tracing::info!(monotonic_counter.biorouter.tool_calls = 1,
                    tool_name = %tool_call.name,
                    "Tool call started"
                );
            }
        }
        MessageContent::ToolResponse(tool_response) => {
            let tool_name = all_messages
                .iter()
                .rev()
                .find_map(|msg| {
                    msg.content.iter().find_map(|c| {
                        if let MessageContent::ToolRequest(req) = c {
                            if req.id == tool_response.id {
                                if let Ok(tool_call) = &req.tool_call {
                                    Some(tool_call.name.clone())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or_else(|| "unknown".to_string().into());

            let success = tool_response.tool_result.is_ok();
            let result_status = if success { "success" } else { "error" };

            tracing::info!(
                counter.biorouter.tool_completions = 1,
                tool_name = %tool_name,
                result = %result_status,
                "Tool call completed"
            );
        }
        _ => {}
    }
}

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct ChatRequest {
    user_message: Message,
    /// The client's own copy of the session's history, which REPLACES what the
    /// server has stored.
    ///
    /// DEPRECATED as an authoritative overwrite (#51 W5). It is now conditional:
    /// the copy must already contain every message the server holds, and the
    /// request is refused with 409 if it does not. See
    /// [`apply_client_writeback`]. The desktop app has never sent this field.
    #[serde(default)]
    conversation_so_far: Option<Vec<Message>>,
    session_id: String,
    workflow_name: Option<String>,
    workflow_version: Option<String>,
    /// BR-63: how hard to think on this turn (`quick` / `normal` / `deep`), as
    /// picked in the composer. Omitted (the default) leaves the session's own
    /// `/effort` setting — and, failing that, the model's default depth — alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<ReasoningEffort>,
    /// Client-generated idempotency key naming *this* turn (BR-62). Optional, but
    /// a client that retries `/reply` — an SSE reconnect, a fetch retry on a
    /// flaky network — should send the same key it sent the first time. The retry
    /// then comes back as a 409 with `duplicate: true`, meaning "that turn is
    /// still running", instead of being mistaken for a genuine second turn.
    ///
    /// Since the live-turn-stream work this is also the ATTACH pointer: posting
    /// a `turn_id` that names a turn already in flight is answered 200 with that
    /// turn's stream instead of 409. Either name works — the key the client
    /// chose, or the server-assigned `turn-N` it read off a frame's `turn_id` /
    /// `POST /agent/resume`'s `active_turn`.
    #[serde(default)]
    turn_id: Option<String>,
    /// Opaque admission minted by an exact-generation Stop-and-Send. A live
    /// lease is required while that superseded child generation remains under
    /// parent supervision, and is consumed atomically with this successor's
    /// turn lock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    continuation_lease: Option<String>,
    /// Attach only from this per-turn sequence number, when `turn_id` names a
    /// turn already in flight.
    ///
    /// A pure OPTIMISATION and deliberately so: a client that already rendered
    /// frames `0..N` skips re-receiving them, but one that omits the field gets
    /// the whole turn replayed and its own sequence gate makes that idempotent.
    /// Nothing about correctness depends on the server honouring it.
    ///
    /// It lives in the BODY rather than in `?from_seq=`, because `/reply` is
    /// generated with `query?: never` in `api/types.gen.ts` — a query parameter
    /// could only be smuggled past the typed client, while a `ChatRequest` field
    /// appears properly the next time the OpenAPI spec is regenerated.
    #[serde(default)]
    from_seq: Option<u64>,
}

/// Why a client-supplied `conversation_so_far` was refused (#51 W5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritebackConflict {
    /// Ids of messages the server holds that the client's copy does not
    /// contain. Storing that copy would delete them.
    pub missing: Vec<String>,
    /// How many messages the server holds.
    pub stored_message_count: usize,
}

/// Ids of messages `stored` holds that `client` does not, in stored order.
///
/// Message ids are durable and server-assigned (BR-45/#41), so a client's copy
/// naming a message is proof it has seen it. Anything stored that the copy does
/// not name would be DELETED by storing that copy — either an append the client
/// never saw or one it deliberately dropped, and the server cannot tell the two
/// apart. A stored row without an id cannot be reasoned about and is not
/// reported (the read path always synthesizes one, so this is defensive only).
fn unacknowledged_stored_ids(stored: &[Message], client: &[Message]) -> Vec<String> {
    let acknowledged: std::collections::HashSet<&str> =
        client.iter().filter_map(|m| m.id.as_deref()).collect();
    stored
        .iter()
        .filter_map(|m| m.id.as_deref())
        .filter(|id| !acknowledged.contains(id))
        .map(str::to_string)
        .collect()
}

/// The 409 a refused write-back answers with. Machine-readable: `code` names
/// the condition and `missing_message_ids` names exactly what the client's copy
/// would have deleted, so it can re-read the session and retry rather than
/// guess.
fn writeback_conflict_response(conflict: &WritebackConflict) -> axum::response::Response {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({
            "type": "Error",
            "error": "conversation_so_far is missing messages this session already holds; \
                      nothing was written. Re-read the session and retry.",
            "code": "conversation_out_of_date",
            "missing_message_ids": conflict.missing,
            "stored_message_count": conflict.stored_message_count,
        })),
    )
        .into_response()
}

/// Store the client's copy of the history, IF it is safe to (#51 W5).
///
/// This used to be an unconditional [`SessionManager::replace_conversation`] —
/// the NAMED EXCEPTION, meant for a caller that owns the whole history — driven
/// by an HTTP client's copy of arbitrary staleness. `/reply`'s per-session turn
/// lock does not help: it is process-local and only `/reply` takes it, so the
/// CLI, `biorouter term log`, an apps agent socket and a second daemon on the
/// same `sessions.db` all append underneath it. Every one of those appends was
/// acknowledged before this write deleted it.
///
/// It is now conditional on the client's copy already containing everything the
/// server holds, and the write itself goes through the guarded rewrite so a
/// message landing between the check and the write is carried over rather than
/// destroyed. On [`WritebackConflict`] NOTHING is written and `/reply` answers
/// 409; the client re-reads the session and retries.
///
/// This is a deliberate DEPRECATION of the field's authoritative-overwrite
/// semantics. Nothing in this repository sends it — the desktop client posts
/// `session_id` + `user_message` only — so the compatibility cost falls
/// entirely on out-of-tree API clients, which now get a loud 409 instead of
/// silently deleting a user's messages.
async fn apply_client_writeback(
    session_manager: &SessionManager,
    session_id: &str,
    history: Vec<Message>,
) -> Result<Conversation, WritebackConflict> {
    let client = Conversation::new_unvalidated(history);

    // Read the revision BEFORE the conversation, so a message landing between
    // the two reads is inside the snapshot rather than looking foreign.
    let (session, basis) = match session_manager.snapshot_for_rewrite(session_id).await {
        Ok(snapshot) => snapshot,
        Err(e) => {
            // No session to overwrite, or the store is unreadable: there is no
            // precondition to evaluate, so nothing is written. The turn task's
            // own `get_session` surfaces the real failure to the client.
            tracing::warn!("Cannot check the client history for {session_id}: {e}");
            return Ok(client);
        }
    };
    let stored = session.conversation.unwrap_or_default();

    let missing = unacknowledged_stored_ids(stored.messages(), client.messages());
    if !missing.is_empty() {
        return Err(WritebackConflict {
            missing,
            stored_message_count: stored.messages().len(),
        });
    }

    // `known` is the CLIENT's copy rather than the snapshot's conversation. The
    // precondition that matters — `basis` must come from a real
    // `snapshot_for_rewrite` of this session — holds; and the check above
    // already proved the client's copy names every message the snapshot saw, so
    // it is a superset of the snapshot's uid set. What that buys: a row landing
    // above the watermark is foreign, and preserved, unless the client already
    // has it.
    match session_manager
        .replace_conversation_preserving_tail(session_id, &client, basis, &client)
        .await
    {
        // Stale: the store moved between the check above and the write, which
        // the guard caught inside its own transaction. Same answer as a stale
        // client copy — refuse, write nothing.
        Ok((ReplaceOutcome::Stale, _)) => Err(WritebackConflict {
            missing: Vec::new(),
            stored_message_count: stored.messages().len(),
        }),
        // The session vanished under us; let the turn task report it.
        Ok((ReplaceOutcome::SessionNotFound, _)) => Ok(client),
        Ok((_, stored_now)) => Ok(stored_now),
        Err(e) => {
            tracing::warn!("Failed to replace session conversation for {session_id}: {e}");
            Ok(client)
        }
    }
}

pub struct SseResponse {
    rx: ReceiverStream<String>,
}

impl SseResponse {
    fn new(rx: ReceiverStream<String>) -> Self {
        Self { rx }
    }

    /// Construct an `SseResponse` from a raw `mpsc::Receiver<String>`.
    /// Each string pushed to the sender is forwarded verbatim as SSE bytes.
    pub fn from_rx(rx: mpsc::Receiver<String>) -> Self {
        Self {
            rx: ReceiverStream::new(rx),
        }
    }
}

impl Stream for SseResponse {
    type Item = Result<Bytes, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.rx)
            .poll_next(cx)
            .map(|opt| opt.map(|s| Ok(Bytes::from(s))))
    }
}

impl IntoResponse for SseResponse {
    fn into_response(self) -> axum::response::Response {
        let stream = self;
        let body = axum::body::Body::from_stream(stream);

        http::Response::builder()
            .header("Content-Type", "text/event-stream")
            .header("Cache-Control", "no-cache")
            .header("Connection", "keep-alive")
            .body(body)
            .unwrap()
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[serde(tag = "type")]
pub enum MessageEvent {
    /// Authoritative lifecycle edge for a turn visible on the read-only session
    /// observer. It lets a quiet turn render as active without inferring work
    /// from model output or polling `/agent/resume` across initialization.
    TurnStarted {
        turn_id: String,
    },
    /// Authoritative observer snapshot of the session's current lifecycle.
    /// `None` is meaningful: it retires a turn whose terminal edge was missed
    /// while the observer was disconnected.
    TurnState {
        #[schema(required)]
        active_turn_id: Option<String>,
    },
    Message {
        message: Message,
        token_state: TokenState,
    },
    Error {
        #[serde(skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        error: String,
        code: String,
        scope: TurnErrorScope,
        retryable: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider_kind: Option<String>,
    },
    Finish {
        #[serde(skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        reason: String,
        token_state: TokenState,
    },
    ModelChange {
        model: String,
        mode: String,
    },
    Notification {
        request_id: String,
        #[schema(value_type = Object)]
        message: ServerNotification,
    },
    /// A tool call the model has begun emitting, announced as soon as its name
    /// is known — before its arguments finish generating. Advisory only: the
    /// client draws a skeleton tool card keyed by `id` and later merges the
    /// authoritative `Message` tool request (same `id`) into it. This is NOT a
    /// tool request and must never be dispatched, gated, or persisted; it does
    /// not enter `all_messages` or the coalescer.
    ToolCallPending {
        id: String,
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        partial_args: Option<String>,
    },
    UpdateConversation {
        conversation: Conversation,
        token_state: TokenState,
    },
    /// #59: the ids this turn's messages were actually persisted under.
    ///
    /// A client that watches a whole turn go by cannot otherwise learn them: a
    /// message is streamed *before* it is stored, one streamed assistant reply
    /// can become several stored rows, and the model-only rows (BR-47 post-edit
    /// diagnostics, loop-guard / stall / budget nudges, hook context) are never
    /// streamed at all. Without this frame `expectedMessageIds` on
    /// `POST /sessions/{session_id}/edit_message` — the ids of every message the
    /// client's view holds — is unsatisfiable, and the in-place edit it guards
    /// answers 409 on a session nobody else has touched.
    ///
    /// Ids only: this frame never re-sends message bodies. Each entry carries
    /// `userVisible`, which is what separates a row the client is deliberately
    /// not shown from one it was simply never told about — a client draws
    /// nothing for a `userVisible: false` row but must still name it.
    ///
    /// The converse does NOT hold: `userVisible: true` is not an instruction to
    /// draw, it is the absence of the hidden flag, and the content usually
    /// arrived already inside a `Message` frame (one streamed reply is stored as
    /// several rows). This frame is for accounting; the transcript comes from
    /// `Message` frames alone. See `PersistedMessage::user_visible`.
    MessagesPersisted {
        messages: Vec<PersistedMessage>,
    },
    /// Issue #56 Gate B: what this turn is running on — the provider and model
    /// the agent holds at the seam, plus the chat's classification after the
    /// turn ratchet has committed.
    ///
    /// Advisory display state, exactly like `ToolCallPending`: never persisted,
    /// never replayed, never shown to the model, and it changes nothing about
    /// what the privacy gates permit or refuse. `privacy_tier` here is a REPORT
    /// of a classification the daemon has already written; reading it is not a
    /// second write and no gate consults this frame.
    ///
    /// ⚠ **Sent on every turn that reaches a provider**, not only on a repaired
    /// bind. It carries the two fields a client cannot compute and that change
    /// underneath it: the ratcheted `privacy_tier` (measured by #196 as the one
    /// field that lagged, which is why a chat a turn had just made private
    /// showed neither the note nor the chip override until a reload) and the
    /// binding `restore_provider_from_session` took from a row another window,
    /// the CLI or a schedule may have rewritten.
    ///
    /// ⚠ It does NOT carry the selection it may have displaced, so a client must
    /// not treat receiving it as "the user's choice was overridden" — on the
    /// overwhelmingly common turn it names exactly what the client is already
    /// showing. Compare it against what you display, and say nothing when they
    /// agree. Widening it from the repair arm is safe precisely because that
    /// comparison, not the frame's arrival, is what earns a word on screen.
    PrivacyProviderPinned {
        provider: String,
        model: String,
        /// The chat's classification at this turn's seam, after the ratchet.
        privacy_tier: SessionClassification,
        /// That classification's provenance on the row (`turn:versa_azure`,
        /// `mcp:…`, `backfill:…`), or `None` for a row never raised.
        privacy_reason: Option<String>,
    },
    Ping,
}

#[derive(Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TurnErrorScope {
    Provider,
    Session,
    Inference,
    Internal,
}

impl TurnErrorScope {
    /// The exact strings the `#[serde(rename_all = "snake_case")]` impl emits.
    /// Written out rather than derived through `serde_json`, so the wire values
    /// are greppable and a renamed variant is a compile error here first.
    pub(crate) fn wire_value(&self) -> &'static str {
        match self {
            TurnErrorScope::Provider => "provider",
            TurnErrorScope::Session => "session",
            TurnErrorScope::Inference => "inference",
            TurnErrorScope::Internal => "internal",
        }
    }

    /// The inverse. An unrecognized string degrades to `Internal` — a frame from
    /// a newer runner must never panic a consumer.
    ///
    /// Landed with its forward direction (Task 6) so the pair is written and
    /// reviewed together; its consumer is
    /// [`crate::routes::session_events::map_bus_event`], which turns a bus
    /// `TurnError.scope` string back into this enum.
    pub(crate) fn from_wire_value(value: &str) -> Self {
        match value {
            "provider" => TurnErrorScope::Provider,
            "session" => TurnErrorScope::Session,
            "inference" => TurnErrorScope::Inference,
            _ => TurnErrorScope::Internal,
        }
    }
}

impl MessageEvent {
    pub(crate) fn error(
        error: impl Into<String>,
        code: impl Into<String>,
        scope: TurnErrorScope,
        retryable: bool,
        provider_kind: Option<String>,
    ) -> Self {
        Self::Error {
            turn_id: None,
            error: error.into(),
            code: code.into(),
            scope,
            retryable,
            provider_kind,
        }
    }
}

/// Read the session's token counters straight from the store.
///
/// BR-52: this used to run on *every* streamed event — one SQLite query per
/// token — even though the counters only ever move at a turn/compaction
/// boundary, so every mid-stream read returned the value the previous one had
/// already returned. The stream now carries the agent's own
/// [`AgentEvent::TokenUsage`] snapshot and this remains only for the two places
/// a DB read is genuinely needed: seeding the state when the stream opens (a
/// resumed session already has counters) and the authoritative reconciliation on
/// `Finish`.
pub(crate) async fn get_token_state(
    session_manager: &SessionManager,
    session_id: &str,
) -> TokenState {
    // Fetch only the token counters — not a full session row plus a
    // `COUNT(*)` over the messages table — since only the token fields are used.
    session_manager
        .get_token_counts(session_id)
        .await
        .map(TokenState::from)
        .inspect_err(|e| {
            tracing::warn!(
                "Failed to fetch session token state for {}: {}",
                session_id,
                e
            );
        })
        .unwrap_or_default()
}

/// Log one frame on the turn's stream, where every observer — and every
/// observer that has not attached yet — can read it.
///
/// **This function cannot fail and cannot cancel anything, and that is the
/// point.** It replaces a `tx.send(...)` into the single HTTP response body
/// whose failure called `cancel_token.cancel()`, which made "nobody is
/// listening" and "stop working" the same event: closing the window, moving the
/// tab, or reloading the renderer killed the turn mid-flight. A turn with zero
/// observers is now an ordinary state — see [`crate::turn_stream`].
fn stream_event(event: MessageEvent, stream: &TurnStream) {
    stream.publish(&event);
}

/// BR-53a: how long consecutive streamed text deltas are coalesced into a
/// single SSE frame, read once per `/reply` from `BIOROUTER_SSE_COALESCE_MS`.
///
/// Default (unset, empty, `0`, or unparseable) is `Duration::ZERO`, which
/// disables coalescing and keeps the byte-for-byte legacy behaviour of one SSE
/// frame per provider chunk. A non-zero value (e.g. `50`) batches same-id text
/// deltas on that millisecond window — the flush is bounded to the window and
/// happens immediately at any real boundary (see [`DeltaCoalescer`]).
pub(crate) fn sse_coalesce_window() -> Duration {
    std::env::var("BIOROUTER_SSE_COALESCE_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::ZERO)
}

/// The delta text if `msg` is the shape the provider streams for a running
/// assistant answer — exactly one `Text` content plus a stable id. Tool
/// requests, thinking, redacted-thinking, multi-content, and id-less messages
/// are never coalesced (they always flush immediately), so nothing that carries
/// structure or ordering guarantees is ever merged.
fn coalescable_delta_text(msg: &Message) -> Option<&str> {
    if msg.id.is_none() || msg.content.len() != 1 {
        return None;
    }
    msg.content[0].as_text()
}

/// BR-53a: coalesces the provider's token-by-token text deltas so the stream
/// emits at most one SSE frame per configured window instead of one per token.
///
/// A run of same-id `Text` deltas is buffered in memory; the buffer is flushed
/// (as one `Message` carrying the concatenated text, with the run's stable id)
/// when the window elapses, when a delta with a different id arrives, when a
/// non-text message (tool request, thinking, …) or any non-`Message` event
/// arrives, or when the stream ends / is cancelled. The concatenation is exact
/// (`MessageContent::text` does not re-sanitize), so the client's append-based
/// accumulation reconstructs identical text to the un-coalesced path.
struct DeltaCoalescer {
    window: Duration,
    pending: Option<Message>,
    deadline: Option<tokio::time::Instant>,
}

impl DeltaCoalescer {
    fn new(window: Duration) -> Self {
        Self {
            window,
            pending: None,
            deadline: None,
        }
    }

    fn enabled(&self) -> bool {
        !self.window.is_zero()
    }

    /// When the buffered run must be flushed by, if anything is buffered.
    fn deadline(&self) -> Option<tokio::time::Instant> {
        self.deadline
    }

    /// Offer a streamed `Message`. Returns the messages to emit to the client
    /// *now*, in order (a flushed run and/or a pass-through); anything not
    /// returned is buffered for a later [`Self::drain`].
    fn push(&mut self, msg: Message) -> Vec<Message> {
        if !self.enabled() {
            return vec![msg];
        }
        if coalescable_delta_text(&msg).is_none() {
            // Not a coalescable text delta: flush the buffered run first so
            // ordering is preserved, then pass this message straight through.
            let mut out = Vec::new();
            out.extend(self.pending.take());
            self.deadline = None;
            out.push(msg);
            return out;
        }
        let continues_run = self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.id == msg.id);
        if continues_run {
            let delta = msg
                .content
                .into_iter()
                .next()
                .and_then(|c| c.as_text().map(str::to_owned))
                .unwrap_or_default();
            if let Some(pending) = self.pending.as_mut() {
                let current = pending
                    .content
                    .first()
                    .and_then(|c| c.as_text())
                    .unwrap_or("")
                    .to_owned();
                pending.content = vec![MessageContent::text(format!("{current}{delta}"))];
            }
            // Deadline stays anchored to the run's first delta, so latency is
            // bounded by the window regardless of how many deltas land in it.
            Vec::new()
        } else {
            // First delta of a run (or the id changed mid-stream): flush any
            // previous run, then start buffering this one.
            let flushed = self.pending.take();
            self.pending = Some(msg);
            self.deadline = Some(tokio::time::Instant::now() + self.window);
            flushed.into_iter().collect()
        }
    }

    /// Take the buffered run (if any), clearing the deadline. Called on the
    /// flush timer, at end-of-stream, and on cancellation.
    fn drain(&mut self) -> Option<Message> {
        self.deadline = None;
        self.pending.take()
    }
}

/// Flush any buffered coalesced run to the turn's stream as one `Message` frame.
fn flush_coalesced(coalescer: &mut DeltaCoalescer, stream: &TurnStream, token_state: &TokenState) {
    if let Some(message) = coalescer.drain() {
        stream_event(
            MessageEvent::Message {
                message,
                token_state: token_state.clone(),
            },
            stream,
        );
    }
}

/// What a `/reply` SSE consumer does when it falls behind the session bus.
///
/// Before BR-71 this could not happen: the agent stream was throttled by the
/// `mpsc::channel(100)` into the SSE response, so a slow client slowed the
/// turn. With one runner publishing to a broadcast bus (design §4.2), the
/// publisher never blocks — which is the point, since an observer must never be
/// able to stall a turn — and a stalled renderer can miss frames instead.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BusLagAction {
    /// Re-send the whole conversation from storage. Costs one session read;
    /// leaves the client correct rather than subtly short a tool result.
    ResyncFromStorage,
}

pub(crate) fn on_bus_lag_action() -> BusLagAction {
    BusLagAction::ResyncFromStorage
}

/// How long the supervisor lets the SSE task drain after the runner returns,
/// before releasing it. Only reached when the SSE task has NOT already ended on
/// a terminal frame, i.e. when the runner died without publishing one.
const RUNNER_EXIT_DRAIN_GRACE: Duration = Duration::from_millis(250);

/// How long the pump keeps draining the bus AFTER its cancellation token trips,
/// so a cancelled turn's real terminal frame still lands in the replay buffer.
///
/// Cancellation is asynchronous: the runner unwinds at its next loop boundary
/// and only then does `finish_turn` publish `TurnFinished { reason:
/// "cancelled" }`. Breaking the instant the token trips would close the log
/// before that frame arrived, and every client attaching afterwards would read
/// the synthesized "ended without a result" terminal instead of the truth. Two
/// seconds covers the runner's end-of-turn store read under load; past it,
/// `TurnStream::close` synthesizes a terminal so nothing waits forever.
const CANCEL_DRAIN_GRACE: Duration = Duration::from_secs(2);

/// How long a re-POST of an already-finished turn waits for that turn's writer
/// to log its ending before answering without one.
///
/// It is the sum of the two graces above, because those bound exactly this: the
/// runner has returned (that is what makes the turn "finished"), the supervisor
/// gives the pump [`RUNNER_EXIT_DRAIN_GRACE`] to consume the terminal it
/// published and then cancels it, and a cancelled pump drains for at most
/// [`CANCEL_DRAIN_GRACE`] before its `TurnWriter` drops and closes the log. Past
/// that sum the writer is wedged, and this reader answers from what it has
/// rather than waiting forever — WITHOUT closing the log, which is the act that
/// cost healthy turns their real `Finish`. In practice the wait is microseconds:
/// the frame is usually already there.
const TERMINAL_WAIT_BUDGET: Duration =
    Duration::from_millis(RUNNER_EXIT_DRAIN_GRACE.as_millis() as u64 + 2_000);

/// Own the turn's end-of-life, for BOTH failure modes.
///
/// 1. **The runner panicked.** Send the one internal-error frame (unchanged
///    behaviour).
/// 2. **The pump is still waiting when the runner is gone.** Release it.
///    This is load-bearing: pre-refactor the turn task owned `task_tx`, so its
///    return — panic included — dropped the sender and closed the body. Now the
///    pump only breaks on a terminal bus event, `RecvError::Closed`, or
///    `cancel_token`. A panicking runner publishes no terminal event; `Closed`
///    is unreachable because this consumer's own `Receiver` keeps the session's
///    `broadcast::Sender` alive; and `TurnGuard::drop` does not trip the token
///    (`state.rs`, `TurnGuard`'s `Drop` impl). Without this the turn's log never
///    closes and every attached response stays open forever after the error
///    frame.
///
/// The grace period is what keeps case 2 from truncating a HEALTHY turn: the
/// runner publishes `TurnFinished` and returns, and the pump may not have
/// consumed it yet. Waiting on the pump's own handle first means a normal turn
/// is never cancelled behind its back.
///
/// Note what is NOT here: nothing about HTTP responses. The supervisor owns the
/// turn's end-of-life, and the turn no longer has a response — it has a stream
/// that any number of responses read.
async fn supervise_turn(
    runner: tokio::task::JoinHandle<()>,
    pump: tokio::task::JoinHandle<()>,
    stream: Arc<TurnStream>,
    cancel: CancellationToken,
) {
    if let Err(join_error) = runner.await {
        tracing::error!("Reply task terminated unexpectedly: {join_error}");
        stream_event(
            MessageEvent::error(
                "The model turn ended unexpectedly. Please retry.",
                "internal_error",
                TurnErrorScope::Internal,
                true,
                None,
            ),
            &stream,
        );
    }
    // A PANICKED pump is not a clean exit. `is_err()` on the timeout is true
    // only for `Elapsed`; a pump that panicked completes as `Ok(Err(JoinError))`
    // and used to read as "the pump finished, nothing to do" — while the panic
    // meant it had reached none of its own exit code. The turn's log is closed
    // regardless now (the pump holds a `TurnWriter`, whose `Drop` runs during
    // the unwind), but the runner-side token must still be tripped: nothing else
    // is going to stop a turn whose pump is gone.
    match tokio::time::timeout(RUNNER_EXIT_DRAIN_GRACE, pump).await {
        Ok(Ok(())) => {}
        Ok(Err(join_error)) => {
            tracing::error!(
                counter.biorouter.reply_sse_released_by_supervisor = 1,
                "the turn's stream pump terminated abnormally ({join_error}); releasing the turn"
            );
            cancel.cancel();
        }
        Err(_elapsed) => {
            tracing::warn!(
                counter.biorouter.reply_sse_released_by_supervisor = 1,
                "turn ended without a terminal frame; releasing the turn stream"
            );
            cancel.cancel();
        }
    }
}

/// The turn's PUMP: one bus consumer, one sequence-numbered frame log, 0..N
/// HTTP responses reading it.
///
/// Named rather than inlined into the handler for one reason — every branch
/// below is a property the refactor must not lose, and an inline `tokio::spawn`
/// block inside a route handler is reachable by no test at all. The three that
/// matter are the ones a plausible wrong implementation would still ship green:
/// the flush that carries #59's ordering invariant across the coalescer, the
/// `Lagged` resync, and breaking on the terminal frame. Each is pinned by a test
/// that drives THIS function against the real bus (`mod tests`), so removing one
/// turns a suite red instead of only being described in a comment.
///
/// **It is spawned ONCE per turn, by the `/reply` that starts it, and it does
/// not belong to that request.** The request's response is just the first
/// reader of the log this writes ([`drain_stream_to_client`]); when that reader
/// goes away the pump keeps running, which is the whole fix. Coalescing
/// therefore moved here too: with several observers, one shared numbering is
/// the only way two clients can agree on what frame 7 is.
///
/// `coalesce_window` is a parameter rather than a call to
/// [`sse_coalesce_window`] so a test can run the loop with coalescing ON.
/// `BIOROUTER_SSE_COALESCE_MS` is unset in tests, and with a zero window the
/// coalescer passes every delta straight through — so the flush ordering this
/// function exists to guarantee would never execute under test.
///
/// The heartbeat is NOT here any more: a `Ping` is a per-connection liveness
/// probe, it carries no turn content, and numbering it would fill the replay
/// buffer with keepalives a re-attaching client would then have to skip. It
/// lives in [`drain_stream_to_client`], unnumbered.
pub(crate) async fn pump_bus_into_stream(
    state: Arc<AppState>,
    session_id: String,
    mut bus: biorouter::session_events::Subscription,
    writer: crate::turn_stream::TurnWriter,
    cancel: CancellationToken,
    coalesce_window: Duration,
) {
    // The log's one owner for the whole of this function. Every exit path —
    // including a panic unwinding this task — closes it exactly once through
    // `TurnWriter::drop`, so there is no `stream.close()` at the bottom to be
    // skipped and no second closer to race.
    let stream = Arc::clone(writer.stream());
    let mut token_state = get_token_state(state.session_manager(), &session_id).await;
    // BR-53a: batch the provider's token-by-token text deltas into one SSE
    // frame per window (`BIOROUTER_SSE_COALESCE_MS`; disabled by default).
    let mut coalescer = DeltaCoalescer::new(coalesce_window);
    // Set when the token trips: the pump keeps draining for CANCEL_DRAIN_GRACE
    // so the runner's real terminal frame still reaches the log.
    let mut cancel_deadline: Option<tokio::time::Instant> = None;
    loop {
        let flush_deadline = coalescer.deadline();
        tokio::select! {
            () = cancel.cancelled(), if cancel_deadline.is_none() => {
                flush_coalesced(&mut coalescer, &stream, &token_state);
                cancel_deadline = Some(tokio::time::Instant::now() + CANCEL_DRAIN_GRACE);
            }
            () = tokio::time::sleep_until(
                    cancel_deadline.unwrap_or_else(tokio::time::Instant::now)),
                if cancel_deadline.is_some() =>
            {
                break;
            }
            () = tokio::time::sleep_until(
                    flush_deadline.unwrap_or_else(tokio::time::Instant::now)),
                if flush_deadline.is_some() =>
            {
                flush_coalesced(&mut coalescer, &stream, &token_state);
            }
            received = bus.recv() => match received {
                Ok(biorouter::session_events::SessionBusEvent::Agent(
                    biorouter::agents::AgentEvent::Message(message),
                )) => {
                    // Coalescing is applied here, once per turn, so every
                    // observer sees the same frames under the same numbering.
                    for message in coalescer.push(message) {
                        stream_event(
                            MessageEvent::Message { message, token_state: token_state.clone() },
                            &stream,
                        );
                    }
                }
                Ok(event) => {
                    // A retired pump keeps draining for CANCEL_DRAIN_GRACE, and
                    // the bus is per SESSION, not per turn — so a SECOND turn
                    // starting inside that window would have its events mapped
                    // into the FIRST turn's log, under the first turn's
                    // numbering. `TurnStarted` naming a turn that is not ours is
                    // the unambiguous signal that our turn is over and somebody
                    // else owns the session now.
                    if let biorouter::session_events::SessionBusEvent::TurnStarted { turn_id } =
                        &event
                    {
                        if turn_id != stream.turn_id() {
                            flush_coalesced(&mut coalescer, &stream, &token_state);
                            break;
                        }
                    }
                    let terminal = matches!(
                        event,
                        biorouter::session_events::SessionBusEvent::TurnFinished { .. }
                            | biorouter::session_events::SessionBusEvent::TurnError { .. }
                    );
                    if !matches!(
                        event,
                        biorouter::session_events::SessionBusEvent::Agent(
                            biorouter::agents::AgentEvent::TokenUsage(_)
                        )
                    ) {
                        // Anything that is not pure token bookkeeping ends a
                        // coalescing run, so cards appear after the prose
                        // that precedes them.
                        //
                        // ⚠ DO NOT narrow this to "flush only on terminal
                        // frames". It is what carries #59's ordering
                        // invariant across the coalescer: *no
                        // `MessagesPersisted` may precede a `Message` frame
                        // carrying one of the ids it publishes*
                        // (`agent.rs`). The coalescer can be holding the
                        // very delta whose stored row the next
                        // `MessagesPersisted` names, and the pre-refactor
                        // handler said exactly this in its own
                        // `MessagesPersisted` arm. Emitted backwards, a
                        // client that reads the id and then loses the stream
                        // claims every stored row while holding none of the
                        // bodies, passes the `expectedMessageIds` guard on a
                        // short transcript, and has the server truncate rows
                        // still on its screen.
                        //
                        // Pinned by `the_sse_loop_flushes_buffered_text_before_a_persisted_frame`
                        // and `token_usage_alone_does_not_break_a_coalescing_run`,
                        // which drive this loop with a non-zero window; narrowing
                        // the condition either way turns one of them red.
                        flush_coalesced(&mut coalescer, &stream, &token_state);
                    }
                    if let Some(frame) = crate::routes::session_events::map_bus_event(
                        event,
                        &mut token_state,
                    ) {
                        stream_event(frame, &stream);
                    }
                    if terminal {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    // BR-71 §8.4 + reconciliation #9: the publisher no longer
                    // blocks on this consumer, so a stalled renderer can fall
                    // behind. Resync from storage rather than silently
                    // dropping frames.
                    //
                    // #59 INTERACTION, stated because it is a real behaviour
                    // difference a user can hit. On `Lagged` this consumer can
                    // skip a `Message` frame and still receive the later
                    // `MessagesPersisted` naming its id — the exact
                    // "claims every stored row while holding none of the
                    // bodies" failure `agent.rs` describes. The resync below
                    // is what makes that safe, and it is safe for a specific
                    // reason: `bus_lag_resync_frame` reads
                    // `get_session(id, true)`, which INCLUDES hidden rows, so
                    // the client's view is restored complete rather than
                    // partial. The secondary effect is that the desktop's
                    // `viewNamesEveryStoredRow` gate clears on any wholesale
                    // `UpdateConversation` (`936f5a33`), so a client that lags
                    // once omits `expectedMessageIds` on its next in-place
                    // edit until it re-reads the session. Omission is the safe
                    // direction (the guard checks `stored ∖ client`), but it
                    // is a capability difference between the lag and non-lag
                    // paths and must not be discovered by a user.
                    //
                    // Pinned by `a_lagged_sse_loop_resyncs_the_client_from_storage`,
                    // which overruns the real ring before this loop's first
                    // `recv` — deleting the resync leaves the client with no
                    // frame at all and the test times out.
                    tracing::warn!(
                        counter.biorouter.reply_bus_lagged = 1,
                        skipped,
                        "reply SSE consumer lagged; resyncing from storage"
                    );
                    debug_assert!(matches!(on_bus_lag_action(), BusLagAction::ResyncFromStorage));
                    flush_coalesced(&mut coalescer, &stream, &token_state);
                    if let Some(resync) = crate::routes::session_events::bus_lag_resync_frame(
                        &state,
                        &session_id,
                        &token_state,
                    )
                    .await
                    {
                        stream_event(resync, &stream);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
        }
    }
    // No `stream.close()` here, deliberately. `writer` is this log's one owner
    // and its `Drop` — which runs on THIS path, on cancellation dropping the
    // future mid-`await`, and during a panic's unwind — ends the log with a
    // guaranteed terminal frame. A close statement at the bottom of a function
    // covers only the exit paths that reach the bottom, and the ones that do not
    // are exactly the ones that left clients parked forever.
    drop(writer);
}

/// The per-RESPONSE half: replay this turn's backlog immediately, then follow
/// the live tail, for as long as this particular HTTP client is there.
///
/// **A send failure here ends this response and nothing else.** That single
/// property is the bug fix: the turn keeps running, its pump keeps logging, and
/// the next client to attach gets everything from seq 0. Only the orphan reaper
/// ([`crate::turn_stream::TurnStream::spawn_orphan_reaper`]) can end a turn
/// because of its audience, and only after minutes of nobody watching.
///
/// R2's "delivered IMMEDIATELY on attach": the backlog is written to the socket
/// as fast as it will take it, with no pacing — a caught-up client renders the
/// whole thing at once and then continues live.
async fn drain_stream_to_client(
    state: Arc<AppState>,
    session_id: String,
    stream: Arc<TurnStream>,
    from_seq: u64,
    terminal_only: bool,
    tx: mpsc::Sender<String>,
) {
    // A turn that had ALREADY FINISHED when this request arrived is answered
    // with its terminal frame and nothing else.
    //
    // This is not a shortcut, it is a correctness requirement, and the sequence
    // numbers cannot cover it. The sequence: a window dies mid-turn, the turn
    // completes with nobody attached, the window comes back, reads the session
    // from the store — so its transcript ALREADY CONTAINS the finished turn —
    // and re-POSTs its stale turn pointer with a high-water mark of -1, because
    // this renderer never saw a frame. Replaying the backlog there re-renders
    // the whole turn as duplicates: exactly the visible failure R2 exists to
    // prevent, arriving through the one door the client's gate cannot watch.
    // The store is the authority for a completed turn; the stream's only
    // remaining job is to say that it ended.
    if terminal_only {
        send_terminal_only(&stream, &tx).await;
        return;
    }

    // A log with no writer will never carry a frame and will never be closed,
    // because nothing owns it (`TurnStream::claim_writer`): the turn lock is
    // also taken as a plain mutex by an in-place edit and a working-directory
    // change, and by turn runners that have no `/reply` pump. Following it is
    // waiting for something that cannot happen — the response never ends, the
    // client sits in `chatState: Streaming`, and the composer is dead until the
    // window is reloaded.
    //
    // Answer from what is there and end. This does NOT close the log: "no
    // writer" is a question with its own answer now, and a reader that mutates
    // the log to make its own answer true is the mistake that cost healthy
    // turns their terminal frame three separate times (see `turn_stream`'s
    // "Who may end a turn's log").
    if !stream.has_writer() {
        tracing::debug!(
            session_id = %session_id,
            turn_id = %stream.turn_id(),
            "attach to a turn whose log has no writer; answering without following it"
        );
        send_terminal_only(&stream, &tx).await;
        return;
    }

    let mut reader = stream.attach(from_seq);
    let mut heartbeat = tokio::time::interval(Duration::from_millis(500));

    loop {
        // R1's fallback. The backlog this reader needed has been evicted (only
        // reachable on a turn that overran REPLAY_BYTE_BUDGET), so the prefix is
        // recovered from the session store — where, by construction, it is: the
        // evicted frames are the OLDEST, which are the ones already persisted,
        // while the un-persisted tail is exactly what the buffer retains.
        if reader.take_gap() {
            tracing::warn!(
                counter.biorouter.turn_stream_replay_gap = 1,
                session_id = %session_id,
                turn_id = %reader.turn_id(),
                "turn replay buffer overran; recovering the prefix from storage"
            );
            let resync = crate::routes::session_events::bus_lag_resync_frame(
                &state,
                &session_id,
                &TokenState::default(),
            )
            .await;
            if let Some(resync) = resync {
                if send_unnumbered(&tx, &resync, true).await.is_err() {
                    return;
                }
            }
        }

        tokio::select! {
            _ = heartbeat.tick() => {
                // Unnumbered: a keepalive is not part of the turn.
                if send_watching(&tx, &mut reader, ping_sse()).await.is_err() {
                    return;
                }
            }
            event = reader.recv() => match event {
                crate::turn_stream::ReaderEvent::Frame(frame, replay) => {
                    let sse = if replay { frame.replay_sse() } else { frame.live_sse() };
                    if send_watching(&tx, &mut reader, sse).await.is_err() {
                        // The client hung up. That is all it is.
                        tracing::debug!(
                            session_id = %session_id,
                            "a turn stream observer disconnected; the turn continues"
                        );
                        return;
                    }
                }
                crate::turn_stream::ReaderEvent::Gap => continue,
                crate::turn_stream::ReaderEvent::Closed => return,
            },
        }
    }
}

/// The `Ping` heartbeat as SSE bytes. Unnumbered — a keepalive has no place in
/// the turn's ordering.
fn ping_sse() -> String {
    serde_json::to_value(&MessageEvent::Ping)
        .map(|value| format!("data: {value}\n\n"))
        .unwrap_or_else(|_| "data: {\"type\":\"Ping\"}\n\n".to_string())
}

/// Hand one frame to this client, and keep the turn's observer count honest
/// while doing it.
///
/// `observers` used to count ATTACHMENTS. A client that is connected but has
/// stopped draining — a frozen renderer, a suspended VM, a half-open TCP
/// connection — never drops its receiver, so `tx.send` never fails; the 100-slot
/// channel fills, this parks inside `send` forever, `observers` stays at one and
/// `idle_since` is never set. The orphan reaper's `observers > 0` check then
/// loops forever on a turn nobody is watching, which is the exact state the
/// reaper exists to end.
///
/// A `tx.closed()` arm in the `select!` would not have helped: it completes only
/// when the RECEIVER IS DROPPED, which is the case the failing `send` already
/// covers. The thing that distinguishes a frozen client from a healthy one is
/// that it stops ACCEPTING, so the timeout is on the accept. And it does not
/// disconnect anyone — the send is simply retried without a deadline, and the
/// reader re-counts itself the moment the frame lands — so a legitimately busy
/// renderer loses nothing but the reaper's clock is allowed to start.
async fn send_watching(
    tx: &mpsc::Sender<String>,
    reader: &mut crate::turn_stream::StreamReader,
    sse: String,
) -> Result<(), ()> {
    // `reserve` rather than `send`, so the frame is not moved into a future the
    // timeout is about to drop.
    let permit =
        match tokio::time::timeout(crate::turn_stream::OBSERVER_STALL_GRACE, tx.reserve()).await {
            Ok(permit) => permit,
            Err(_elapsed) => {
                reader.mark_stalled();
                tx.reserve().await
            }
        };
    match permit {
        Ok(permit) => {
            permit.send(sse);
            reader.mark_active();
            Ok(())
        }
        Err(_) => Err(()),
    }
}

/// Answer a reader that wants only this turn's ENDING: the re-POST of a turn
/// that has already finished, and the attach to a log nobody writes.
///
/// The distinction that used to be missing lives here. "This turn has no
/// terminal frame yet" has two completely different causes:
///
///  - **its writer has not finished draining.** `TurnGuard::drop` retires the
///    registry entry the instant the runner returns, but the runner's last act
///    was to publish `TurnFinished` on the bus and the pump has not read it yet.
///    A re-POST landing in that beat — an SSE retry, a reload at end of turn —
///    used to CLOSE the log here, which synthesized "the stream for this turn
///    ended without a result" and made `publish` refuse the runner's real
///    `Finish`. The cost landed on every observer of a healthy turn, not on the
///    late one. So: wait for the writer, which always produces a terminal on
///    every one of its exit paths.
///  - **there is no writer at all**, and no terminal is ever coming. Then this
///    connection is told so directly, with a frame that belongs to the
///    connection and not to the turn — the log is not mutated, because a reader
///    that edits the log to make its own answer true is precisely the bug above.
async fn send_terminal_only(stream: &Arc<TurnStream>, tx: &mpsc::Sender<String>) {
    if stream.terminal_frame().is_none() && stream.has_writer() {
        // Follow the tail (attach clamps to the newest frame) purely to be woken
        // when the writer logs the ending; the frames themselves are not wanted.
        let wait = async {
            let mut reader = stream.attach(u64::MAX);
            loop {
                match reader.recv().await {
                    crate::turn_stream::ReaderEvent::Frame(..) => {
                        if stream.terminal_frame().is_some() {
                            return;
                        }
                    }
                    crate::turn_stream::ReaderEvent::Gap => continue,
                    crate::turn_stream::ReaderEvent::Closed => return,
                }
            }
        };
        let _ = tokio::time::timeout(TERMINAL_WAIT_BUDGET, wait).await;
    }
    if let Some(terminal) = stream.terminal_frame() {
        let _ = tx.send(terminal.replay_sse()).await;
        return;
    }
    // Either no writer at all, or a writer that did not produce an ending
    // within its own budget. Say so on THIS CONNECTION — an unnumbered frame,
    // which belongs to the connection and not to the turn's ordering. The log
    // is left exactly as it was found.
    let (message, code) = if stream.has_writer() {
        (
            "The stream for this turn ended without a result. Please retry.",
            "stream_ended_without_terminal",
        )
    } else {
        (
            "This turn produces no stream to follow.",
            "turn_has_no_stream",
        )
    };
    let _ = send_unnumbered(
        tx,
        &MessageEvent::error(message, code, TurnErrorScope::Internal, true, None),
        true,
    )
    .await;
}

/// Send a frame that belongs to this CONNECTION rather than to the turn: the
/// heartbeat, and the storage resync that repairs an evicted backlog. Such a
/// frame carries no `seq`, because it has no place in the turn's ordering.
async fn send_unnumbered(
    tx: &mpsc::Sender<String>,
    event: &MessageEvent,
    replay: bool,
) -> Result<(), ()> {
    let mut value = serde_json::to_value(event).map_err(|_| ())?;
    if replay {
        if let Some(object) = value.as_object_mut() {
            object.insert("replay".to_string(), serde_json::json!(true));
        }
    }
    tx.send(format!("data: {value}\n\n")).await.map_err(|_| ())
}

/// Open an SSE response that follows `stream` from `from_seq`.
///
/// The same function serves BOTH the request that starts a turn and one that
/// re-attaches to a turn already in flight — which is the shape the fix asks
/// for: the starter has no privileged relationship with the turn, it is just
/// the first observer.
///
/// `terminal_only` distinguishes the third case: a re-POST naming a turn that
/// has already finished. See [`drain_stream_to_client`] for why that one must
/// NOT be replayed.
fn attach_response(
    state: Arc<AppState>,
    session_id: String,
    stream: Arc<TurnStream>,
    from_seq: u64,
    terminal_only: bool,
) -> axum::response::Response {
    let (tx, rx) = mpsc::channel(100);
    tokio::spawn(drain_stream_to_client(
        state,
        session_id,
        stream,
        from_seq,
        terminal_only,
        tx,
    ));
    SseResponse::new(ReceiverStream::new(rx)).into_response()
}

/// Is this request an ATTACH naming a turn this daemon does not hold?
///
/// `/reply` had no way to say "attach only", and outcome 1 of the wire contract
/// is "a `turn_id` naming no known turn starts a new turn" — while the client's
/// attach body is obliged to carry a `user_message` (the schema requires one,
/// and the transcript's trailing user message is the closest truthful value). So
/// the moment the turn being re-attached to left the registry — the daemon
/// restarted, which is the commonest reason a driving stream ends without a
/// terminal frame, or `FINISHED_TURN_RETENTION` elapsed — the re-attach silently
/// RE-SUBMITTED the user's prompt: the answer generated a second time, rendered
/// under the half already on screen (the new turn's id resets the client's
/// sequence gate), and the tokens spent twice. The contract's own words are
/// "nothing is charged twice".
///
/// `from_seq` is the explicit spelling, and it needs no new field: it means "I
/// already hold frames 0..N of a turn", which only an attach can say. A first
/// POST never sends it (see [`ChatRequest::from_seq`] and the client's
/// `buildAttachRequest`), so a request carrying it is an attach and is answered
/// as one. Including when the answer is "that turn is gone".
///
/// Asked BEFORE the turn lock, deliberately: taking the lock to find out would
/// mint a turn entry for a turn that does not exist, and inserting it evicts
/// whatever finished turn that session was still holding for replay.
fn attach_names_a_missing_turn(
    state: &Arc<AppState>,
    session_id: &str,
    request: &ChatRequest,
) -> bool {
    if request.from_seq.is_none() {
        return false;
    }
    // A continuation lease is stronger evidence than the attach heuristic: it
    // proves this exact successor has not been admitted yet. If the first POST
    // was lost before reaching the daemon, its retry must consume the lease and
    // start the successor; if it did reach the daemon, the consumed lease and
    // idempotency key below can only reattach to that same turn.
    if request.continuation_lease.is_some() {
        return false;
    }
    let known = request
        .turn_id
        .as_deref()
        .is_some_and(|turn_id| state.knows_turn(session_id, turn_id));
    if !known {
        tracing::info!(
            session_id = %session_id,
            turn_id = ?request.turn_id,
            "attach names a turn this daemon does not hold; answering without starting one"
        );
    }
    !known
}

/// The answer to an ATTACH naming a turn this daemon does not hold.
///
/// 200 with one `Error` frame and an immediate end, rather than a status code,
/// because an attach is answered on the stream it asked for: the client is
/// already reading an SSE body on every other outcome, and a `turn_not_found`
/// frame lands in the same pipeline as any other terminal instead of needing a
/// second error path. It is `retryable`, and it says the truth — the turn this
/// window was following is gone (most often because the daemon restarted under
/// it) — which is strictly better than the alternative this replaces: silently
/// re-running the prompt and billing it twice.
///
/// The frame is deliberately UNNUMBERED. It belongs to this connection, not to
/// any turn's ordering — there is no turn.
fn attach_missed_response() -> axum::response::Response {
    let (tx, rx) = mpsc::channel(1);
    let event = MessageEvent::error(
        "The turn this window was following is no longer available.",
        "turn_not_found",
        TurnErrorScope::Internal,
        true,
        None,
    );
    if let Ok(value) = serde_json::to_value(&event) {
        let _ = tx.try_send(format!("data: {value}\n\n"));
    }
    drop(tx);
    SseResponse::new(ReceiverStream::new(rx)).into_response()
}

/// The 409 body a refused `/reply` answers with, kept byte-identical to what the
/// handler has always produced.
///
/// `duplicate` is the whole point of the distinction: a client that re-POSTs the
/// same `turn_id` (an SSE reconnect) is told "this turn is already in progress",
/// which it treats as "re-attach", while a genuinely concurrent caller is told
/// "a turn is already in progress" and backs off. Same status, different copy —
/// so both strings live here together where they cannot drift apart.
fn turn_conflict_response(
    session_id: &str,
    conflict: &crate::state::TurnConflict,
) -> axum::response::Response {
    tracing::warn!(
        "Rejected concurrent /reply for session {}: turn {} already in flight (duplicate={})",
        session_id,
        conflict.running_turn_id,
        conflict.duplicate
    );
    debug_assert!(
        !conflict.duplicate,
        "a duplicate turn_id is now ATTACHED to (200 + the turn's stream), not refused"
    );
    let error = if conflict.duplicate {
        "This turn is already in progress for this session."
    } else {
        "A turn is already in progress for this session."
    };
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({
            "type": "Error",
            "error": error,
            "running_turn_id": conflict.running_turn_id,
            "duplicate": conflict.duplicate,
        })),
    )
        .into_response()
}

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct ContinuationLeaseErrorResponse {
    pub error: String,
}

fn continuation_lease_failure_response(
    failure: ContinuationLeaseFailure,
) -> axum::response::Response {
    let error = match failure {
        ContinuationLeaseFailure::Required => "continuation_lease_required",
        ContinuationLeaseFailure::Invalid => "invalid_continuation_lease",
        ContinuationLeaseFailure::CrossSession => "continuation_lease_session_mismatch",
        ContinuationLeaseFailure::Replayed => "continuation_lease_already_resolved",
        ContinuationLeaseFailure::MissingSuccessorTurnId => "continuation_lease_requires_turn_id",
        ContinuationLeaseFailure::OwnedByAnother => "continuation_owned_by_another_client",
        ContinuationLeaseFailure::AdmissionInProgress => "continuation_admission_in_progress",
        ContinuationLeaseFailure::ParentClosing => "parent_turn_closing",
    };
    (
        StatusCode::CONFLICT,
        Json(ContinuationLeaseErrorResponse {
            error: error.to_string(),
        }),
    )
        .into_response()
}

/// One `workflow_runs` counter per workflow *run*, not per turn: the
/// `mark_workflow_run_if_absent` gate is what keeps a ten-turn workflow from
/// reporting ten runs, and it is a session-scoped latch, so this must stay a
/// single call site.
async fn record_workflow_run(state: &Arc<AppState>, session_id: &str, request: &ChatRequest) {
    let Some(workflow_name) = request.workflow_name.clone() else {
        return;
    };
    if !state.mark_workflow_run_if_absent(session_id).await {
        return;
    }
    let workflow_version = request
        .workflow_version
        .clone()
        .unwrap_or_else(|| "unknown".to_string());

    tracing::info!(
        counter.biorouter.workflow_runs = 1,
        workflow_name = %workflow_name,
        workflow_version = %workflow_version,
        session_type = "app",
        interface = "ui",
        "Workflow execution started"
    );
}

#[utoipa::path(
    post,
    path = "/reply",
    // EXPLICIT tag, not utoipa's default module path. Task 42b's CLI-parity gate
    // selects the workspace-control route surface by this tag; a BR-71 route that
    // is not tagged is invisible to it, which is how a capability ships
    // GUI-or-daemon-only.
    tag = "workspace",
    request_body = ChatRequest,
    responses(
        (status = 200, description = "Streaming response initiated: either a NEW turn, or an \
                                      attachment to the turn this `turn_id` already named, \
                                      replayed from `from_seq` and then followed live",
         body = MessageEvent,
         content_type = "text/event-stream"),
        (status = 202, description = "A proven user reply was retained as steering for a delegated child that is still initializing"),
        (status = 403, description = "Refused by a privacy boundary (issue #56 Task 58 / #47): \
                                      the named chat is private (or absent, and an unproven caller \
                                      is told the same thing for both) and the request carried no \
                                      proof it came from the user (body = plain text)"),
        (status = 409, description = "A DIFFERENT turn is already in flight for this session, or \
                                      the supplied `conversation_so_far` is missing messages the \
                                      server holds (nothing was written; re-read the session \
                                      and retry)"),
        (status = 424, description = "Agent not initialized"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn reply(
    State(state): State<Arc<AppState>>,
    // Before `Json`, which consumes the body and must be last.
    headers: axum::http::HeaderMap,
    Json(mut request): Json<ChatRequest>,
) -> axum::response::Response {
    // Issue #56 Task 58 / #47. FIRST — before the turn lock, before the
    // telemetry, before anything at all reads or writes this session.
    //
    // `session_id` is a request parameter, not a credential, and `check_token`
    // compares one machine-wide bearer that AR-11 measured to be recoverable by
    // the agent. This route runs an agent turn, WITH TOOLS, in whatever session
    // the body names, so it strictly dominates every other session-addressing
    // route: a caller who can run a turn in a session can already do anything
    // that session can do. See `routes::session_reach`.
    //
    // Before the turn lock specifically, because the lock's own 409 tells an
    // unproven caller whether the chat it named is busy — which is exactly the
    // kind of disclosure the refusal is worded to withhold.
    if let Err(refusal) = crate::routes::session_reach::session_reach(
        state.session_manager(),
        &request.session_id,
        &headers,
    )
    .await
    {
        return refusal.into_response();
    }

    // A provenance-less reply to a subagent is treated as a human typing in
    // that child's tab. The daemon bearer is model-recoverable, so it cannot
    // establish that fact. Refuse before the turn lock or any write unless the
    // desktop's user-action proof is present; public user sessions retain their
    // existing machine-client behavior.
    let user_action = biorouter_server::auth::is_user_action(&headers);
    let target_session = state
        .session_manager()
        .get_session(&request.session_id, false)
        .await
        .ok();
    let is_subagent = target_session.as_ref().is_some_and(|session| {
        session.session_type == biorouter::session::session_manager::SessionType::SubAgent
    });
    let initializing_child =
        biorouter::agents::subagent_handle::is_child_initializing(&request.session_id);
    if !user_action && (is_subagent || initializing_child) {
        return StatusCode::FORBIDDEN.into_response();
    }

    // A background child exists before it receives a concurrency permit. A
    // user reply in that interval is steering for the delegated initial turn;
    // starting a generic interactive turn here would steal that turn and make
    // the parent's retained result historical. Buffer the exact message under
    // the handle's initialization lock instead. Once the atomic runtime profile
    // is installed, the delegated runner drains it into its initial prompt.
    if user_action && request.continuation_lease.is_none() {
        let queued_message = crate::workspace::turn::stamp_user_direct_if_subagent(
            request.user_message.clone(),
            biorouter::session::session_manager::SessionType::SubAgent,
        );
        match biorouter::agents::subagent_handle::queue_initializing_child_input(
            &request.session_id,
            request.turn_id.clone(),
            queued_message,
        ) {
            biorouter::agents::subagent_handle::InitialInputDisposition::Queued
            | biorouter::agents::subagent_handle::InitialInputDisposition::Duplicate => {
                return StatusCode::ACCEPTED.into_response();
            }
            biorouter::agents::subagent_handle::InitialInputDisposition::NotInitializing => {}
        }
    }

    // Turn *duration* is the runner's (`emit_completion_metrics`); this handler
    // only reports that a session was started, at request entry.
    tracing::info!(
        counter.biorouter.session_starts = 1,
        session_type = "app",
        interface = "ui",
        "Session started"
    );

    let session_id = request.session_id.clone();

    // An ATTACH whose turn is gone must not become a NEW TURN — see
    // `attach_names_a_missing_turn`.
    if attach_names_a_missing_turn(&state, &session_id, &request) {
        return attach_missed_response();
    }

    // Created before the turn lock so the token can be registered *with* the
    // turn: that is what lets `/agent/cancel` (and `/agent/stop`) reach into a
    // running turn and trip it (BR-62).
    let cancel_token = CancellationToken::new();

    // Server-enforced single-turn-per-session lock (BR-33; also serializes the
    // per-session check-compact-persist path of BR-16). Two concurrent `/reply`
    // calls for one session would share one `Arc<Agent>`, confirmation channel,
    // and soft-interrupt queue, interleaving/duplicating output and doubling
    // token spend. Reject the duplicate with 409 instead of corrupting state;
    // the guard is released when the reply task ends (drops below).
    //
    // BR-62: a client that re-POSTs the same `turn_id` (an SSE reconnect) gets
    // `duplicate: true` back, so it can tell "my turn is still running" apart
    // from "someone else's turn is in the way" — and in neither case does a
    // second turn start.
    //
    // …and since the live-turn-stream work, `duplicate: true` is answered with
    // **200 and the turn's stream** rather than a 409 with no way back in. That
    // is the difference between "your turn is still running, sorry" and "here it
    // is, from the beginning". A 409 now means only what it says: a genuinely
    // DIFFERENT turn is in the way.
    let turn_guard = match state.try_begin_turn_idempotent_with_continuation(
        &session_id,
        cancel_token.clone(),
        request.turn_id.clone(),
        request.continuation_lease.as_deref(),
    ) {
        Ok(guard) => guard,
        Err(TurnBeginFailure::Conflict(conflict)) if conflict.duplicate => {
            tracing::info!(
                session_id = %session_id,
                turn_id = %conflict.running_turn_id,
                finished = conflict.finished,
                from_seq = request.from_seq.unwrap_or(0),
                "re-POST of a known turn_id: attaching to its stream"
            );
            // `request.user_message` is DELIBERATELY dropped here. A client
            // attaching to a turn it did not start does not know the prompt that
            // began it — it sends its transcript's trailing user message, or an
            // empty one — and honouring that would inject a phantom prompt into
            // a running turn. An attach is a read.
            return attach_response(
                state.clone(),
                session_id,
                conflict.stream,
                request.from_seq.unwrap_or(0),
                conflict.finished,
            );
        }
        Err(TurnBeginFailure::Conflict(conflict)) => {
            return turn_conflict_response(&session_id, &conflict);
        }
        Err(TurnBeginFailure::ContinuationLease(failure)) => {
            return continuation_lease_failure_response(failure);
        }
    };

    record_workflow_run(&state, &session_id, &request).await;

    // #51 W5: the client's copy of the history is stored HERE, before the turn
    // task is spawned, because it can now be REFUSED — once the SSE response has
    // been returned there is no status code left to say so with. Refusing drops
    // `turn_guard`, so the session is free again.
    let client_conversation = match request.conversation_so_far.take() {
        Some(history) => {
            match apply_client_writeback(state.session_manager(), &session_id, history).await {
                Ok(conversation) => Some(conversation),
                Err(conflict) => {
                    tracing::warn!(
                        "Rejected a stale conversation_so_far for session {}: {} stored \
                         message(s) it does not contain",
                        session_id,
                        conflict.missing.len()
                    );
                    return writeback_conflict_response(&conflict);
                }
            }
        }
        None => None,
    };

    // The turn's frame log, created with the turn lock itself (`state.rs`) so it
    // exists before anything can publish into it and outlives every response
    // that reads it.
    let turn_stream = turn_guard.stream();

    // Take ownership of the log NOW — synchronously, before the first `await`
    // below and before anything is spawned. `has_writer()` is what every attach
    // consults to decide whether following this stream can ever produce
    // anything, so the promise must be on the log before a concurrent re-POST
    // can observe it. The token is moved into the pump; if this handler returned
    // before spawning it, dropping the token would close the log rather than
    // leave it open with nobody to end it.
    // Unreachable `else`: the log was minted by the turn lock this task took.
    let Some(writer) = turn_stream.claim_writer() else {
        tracing::error!(session_id = %session_id, "a new turn's log already had a writer");
        return attach_missed_response();
    };

    // BR-71: subscribe BEFORE the turn task is spawned, so no event can fall
    // into the gap between "turn started" and "we are listening".
    let bus = biorouter::session_events::subscribe(&session_id);

    if user_action && is_subagent && request.continuation_lease.is_none() {
        if let Some(session) = target_session.as_ref() {
            if let Some(parent_session_id) = session.parent_session_id.as_deref() {
                biorouter::agents::subagent_handle::ensure_direct_followup_handle(
                    parent_session_id,
                    &session_id,
                    &session.name,
                );
            }
        }
    }

    // The supervisor outlives both the turn task and the pump, so it needs its
    // own stream handle and token clone.
    let supervisor_stream = Arc::clone(&turn_stream);
    let supervisor_cancel = cancel_token.clone();

    let turn_request = crate::workspace::turn::TurnRequest {
        session_id: session_id.clone(),
        user_message: request.user_message,
        extras: crate::workspace::turn::TurnExtras {
            // The lock is already held under this key; the runner receives the
            // guard rather than re-acquiring, so the key is informational here.
            idempotency_key: request.turn_id.clone(),
            // #51 W5: the ALREADY-STORED conversation, produced by
            // `apply_client_writeback` above this range, and deliberately so: it
            // is a precondition that can answer 409, which a detached task
            // cannot. `None` when the client sent no copy. The runner treats
            // this as a seed and performs no storage write of its own (see
            // `TurnExtras.conversation_so_far`).
            conversation_so_far: client_conversation,
            reasoning_effort: request.reasoning_effort,
            // An interactive turn is already visible as a turn; only injected
            // turns register in active_work.
            register_active_work: false,
            // Supplies the `session_type = "app"` telemetry label the three
            // completion counters have always reported.
            kind: crate::workspace::turn::TurnKind::Interactive,
        },
    };

    let runner_state = state.clone();
    let runner_cancel = cancel_token.clone();
    let handle = tokio::spawn(crate::workspace::turn::run_turn(
        runner_state,
        turn_request,
        turn_guard,
        runner_cancel,
    ));

    // The pump — see `pump_bus_into_stream`, which is a named function precisely
    // so its branches are reachable by a test. It belongs to the TURN, not to
    // this request: it keeps logging with zero observers attached.
    let pump_handle = tokio::spawn(pump_bus_into_stream(
        state.clone(),
        session_id.clone(),
        bus,
        writer,
        cancel_token.clone(),
        sse_coalesce_window(),
    ));

    // The only thing that ends a turn because of its AUDIENCE, and only after
    // minutes of nobody watching. The reaper consults the live exact-generation
    // handle registry atomically with cancellation, so a supervised parent or
    // child is protected while a merely persisted SubAgent session is not.
    turn_stream.spawn_orphan_reaper(cancel_token.clone(), crate::turn_stream::orphan_timeout());

    tokio::spawn(supervise_turn(
        handle,
        pump_handle,
        supervisor_stream,
        supervisor_cancel,
    ));

    // This request is simply the turn's FIRST observer. It has no privileged
    // relationship with the turn and its departure means nothing to it.
    attach_response(state, session_id, turn_stream, 0, false)
}

/// Request body for the soft-interrupt route.
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct InterruptRequest {
    pub session_id: String,
    pub text: String,
    /// Client idempotency key for a steer submitted while a delegated child is
    /// still waiting for its initial runtime. Ordinary active-turn interrupts
    /// do not require it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
}

/// Response body for an accepted soft interrupt (#69).
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct InterruptAccepted {
    /// The agent-loop turn that took the message and will inject it. Identifies
    /// the reply loop, and is deliberately *not* the `turn_id` `/agent/cancel`
    /// reports (that one names the server's turn lock); the two id spaces are
    /// shaped differently so they cannot be confused.
    pub turn_id: String,
}

/// The refusal `POST /interrupt` gives on a daemon that holds no user-action key.
///
/// In the voice of `session_reach::SESSION_REACH_NO_KEY` and `routes::agent`'s
/// `SUBAGENT_CONTROL_NO_KEY`, and for their reason (SD-8): it is this daemon's
/// situation, not the caller's, and a person at a `biorouter serve` page must
/// not be sent hunting for a permission no one can be granted here.
///
/// ⚠ **Never empty, and that is load-bearing.** `biorouter session attach` tells
/// a daemon that wants the proof apart from one that cannot check it by whether
/// a turn-control 403 carries a body (`session_watch::key_verdict`): an empty
/// body means "this daemon holds a key and wants the proof", and the terminal
/// prompts for one. A keyless daemon has no key to be typed, so its steer
/// refusal says so in words instead of being mistaken for a missing credential.
pub const STEER_NO_KEY: &str =
    "This daemon was started without a user-action key, so it cannot verify that a request came \
     from the person at the keyboard, and steering a turn that is already running requires that \
     proof: it changes what the model is doing, in place, without the person watching it having \
     asked. Nothing was queued and the turn was not touched. Stop the turn and send the message \
     instead, or use the desktop app.";

/// May this request stop or settle a turn in the chat it names?
///
/// The gate of `POST /agent/cancel`, `POST /agent/continuation/abandon` and
/// `POST /agent/continuation/recover`, asked before any of them touches the
/// turn. **`POST /interrupt` is deliberately not one of them** — see
/// [`steer_refusal`], which says why the argument below does not reach it.
///
/// * **A daemon that holds a user-action key** — the desktop application's —
///   takes the proof and nothing else, exactly as before; `Unproven` is the
///   empty 403 these routes have always answered.
/// * **A daemon that holds none** — the one `biorouter serve` starts (SD-7), or
///   a `biorouterd` started by hand — asks what `/agent/stop` asks there:
///   `routes::agent::authorize_agent_control`, the reach gate and then the
///   subagent rule. The same call, not a copy of it, so the two routes can never
///   disagree about who may stop a turn.
///
/// ⚠ **Why the keyless arm no longer refuses** (serve decision SD-11). On such a
/// daemon the proof can only refuse everyone, the person at the browser
/// included: measured 2026-09-11, the Stop button answered 403 on every chat of
/// a `serve` host. And the refusal protected nothing. The proof is here so that
/// a model holding the daemon secret, which AR-11 found recoverable, cannot stop
/// another chat's turn; on a keyless daemon the same caller already stops that
/// turn through `/agent/stop` — literally this function's own keyless arm — or,
/// as a model, through `workspace_close { scope: "turn" }`. `/agent/stop` is
/// also strictly the more destructive of the two: it cancels the in-flight turn
/// **and** evicts the agent. Same authority, same scope, weaker effect, so
/// admitting these three gives no caller a capability it lacked.
///
/// ⚠ **Why a daemon that holds a key is not relaxed with it.** There the proof
/// costs the person nothing — the renderer attaches it to every request — and it
/// is what licenses the `UserDirect` stamp a steer carries.
///
/// ⚠ **The two refusals' SHAPES are read by the terminal.** `biorouter session`
/// cannot ask a daemon whether it holds a key, so it sends a stop or a steer
/// without the proof and reads the answer (`commands/session_watch.rs`,
/// `key_verdict`): the `Unproven` arm's EMPTY 403 is the only thing that makes it
/// ask the person for the key, and a refusal carrying a sentence — every one the
/// keyless arm can give — is shown instead, because no key would change it. So
/// a sentence added to `Unproven` would stop the terminal asking for the key on
/// the desktop's daemon, and an empty refusal on the keyless arm would make it
/// ask a `serve` user for a key that does not exist. Both are pinned: the keyed
/// side here in `integration_tests`, the keyless side in
/// `tests/turn_control_no_user_key.rs`.
async fn authorize_turn_control(
    state: &AppState,
    session_id: &str,
    headers: &HeaderMap,
) -> Result<(), axum::response::Response> {
    match user_action_proof(headers) {
        UserActionProof::Proven => Ok(()),
        UserActionProof::Unproven => Err(StatusCode::FORBIDDEN.into_response()),
        UserActionProof::NoKeyInstalled => {
            crate::routes::agent::authorize_agent_control(state, session_id, headers)
                .await
                .map(|_| ())
                .map_err(IntoResponse::into_response)
        }
    }
}

/// May this request put text into a turn that is **already running**?
///
/// The gate of `POST /interrupt` alone, and the one place the four turn-control
/// routes part company. It asks for the user-action proof on every daemon, which
/// is what `main` asked before SD-11 and what this route keeps asking.
///
/// ⚠ **Why the steer does not move with the Stop.** [`authorize_turn_control`]'s
/// keyless arm rests on a dominance argument: the caller already stops that turn
/// through `/agent/stop`, and already puts text in front of that chat's model
/// through `/reply`. The second half is false here. `/reply` takes the BR-33
/// single-turn lock (`try_begin_turn_idempotent_with_continuation`) and answers
/// `409 CONFLICT` when a *different* turn is already running in that chat;
/// `/interrupt` answers `409` when none is. The two preconditions are disjoint,
/// so in the exact state where a steer lands, the route said to dominate it is
/// refused. What admitting it would add is therefore genuinely new, and worth
/// naming precisely: attacker-chosen text injected into a turn already in
/// flight, **without cancelling it**, which the person watching sees as their own
/// turn changing direction. Cancel-then-reply — the nearest thing a caller
/// holding only the daemon secret already has — kills the turn first, and is
/// therefore visible. This is a capability asymmetry, not a tier crossing:
/// `session_reach` still refuses a private chat to a public caller and
/// `refuse_subagent_unless_user` still refuses every subagent's chat.
///
/// ⚠ **Why the keyless refusal is [`STEER_NO_KEY`] rather than an empty 403.**
/// `biorouter session attach` tells the two kinds of daemon apart by whether a
/// turn-control 403 carries a body (`session_watch::key_verdict`): an empty one
/// means "this daemon holds a key and wants the proof", and the terminal prompts
/// for it. A keyless daemon has no key to be typed, so refusing here with an
/// empty body would send a `biorouter serve` user hunting for a credential that
/// does not exist and then tell them it was the wrong one. The sentence keeps
/// the empty 403 meaning exactly what that reading needs it to mean.
///
/// It takes no session id and asks nothing about the chat, so the refusal is
/// byte-for-byte the same for every chat — which keeps a route no proof can ever
/// satisfy from becoming a per-id oracle.
///
/// Returns the refusal rather than a `Result<(), Response>`: there is no success
/// value to carry, and a `Response` is 128 bytes, which `clippy::result_large_err`
/// refuses in a synchronous function. [`authorize_turn_control`] keeps the
/// `Result` shape only because it is `async`, so the lint sees a future rather
/// than the `Result`.
fn steer_refusal(headers: &HeaderMap) -> Option<axum::response::Response> {
    match user_action_proof(headers) {
        UserActionProof::Proven => None,
        UserActionProof::Unproven => Some(StatusCode::FORBIDDEN.into_response()),
        UserActionProof::NoKeyInstalled => Some(
            (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({ "message": STEER_NO_KEY })),
            )
                .into_response(),
        ),
    }
}

/// Soft interrupt: queue a user message to be injected into the session's
/// running turn at the next safe loop boundary, instead of cancelling the turn
/// and re-sending the whole context. Returns 202 Accepted; the message surfaces
/// as a normal user message in the active reply stream on the next loop step.
///
/// BR-61: rejected with 409 when the session has no turn in flight — with no
/// running loop to drain the queue the text would sit on the agent until some
/// unrelated later turn injected it. Clients treat 409 as "just send it as a
/// normal message".
///
/// #69: the 409 is now decided by the *agent's own queue*, not by the turn-lock
/// check above it. Checking the lock and then queueing are two steps against
/// state the reply loop changes underneath them: a steer could be accepted (202)
/// after the loop had already performed its final empty-queue check, and the text
/// then surfaced in an unrelated later turn. `try_queue_soft_interrupt` decides
/// acceptance and enqueues in one critical section, and reports back *which* turn
/// took the message — so a 202 names something real.
#[utoipa::path(
    post,
    path = "/interrupt",
    // See `/reply` above: the tag is what Task 42b's parity gate selects on.
    tag = "workspace",
    request_body = InterruptRequest,
    responses(
        (status = 202, description = "Message queued for injection into the running turn", body = InterruptAccepted),
        (status = 400, description = "Empty message text"),
        (status = 403, description = "The request was not proven to come from the user; on a daemon \
                                      that holds no user-action key, steering is unavailable and the \
                                      refusal says so (SD-11)"),
        (status = 409, description = "No turn is accepting interrupts for this session"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn interrupt(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<InterruptRequest>,
) -> Result<(StatusCode, Json<InterruptAccepted>), axum::response::Response> {
    if let Some(refusal) = steer_refusal(&headers) {
        return Err(refusal);
    }
    if req.text.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST.into_response());
    }
    if let Some(turn_id) = req.turn_id.clone() {
        let queued_message = crate::workspace::turn::stamp_user_direct_if_subagent(
            Message::user()
                .with_id(turn_id.clone())
                .with_text(req.text.clone()),
            biorouter::session::session_manager::SessionType::SubAgent,
        );
        match biorouter::agents::subagent_handle::queue_initializing_child_input(
            &req.session_id,
            Some(turn_id.clone()),
            queued_message,
        ) {
            biorouter::agents::subagent_handle::InitialInputDisposition::Queued
            | biorouter::agents::subagent_handle::InitialInputDisposition::Duplicate => {
                return Ok((StatusCode::ACCEPTED, Json(InterruptAccepted { turn_id })));
            }
            biorouter::agents::subagent_handle::InitialInputDisposition::NotInitializing => {}
        }
    }
    // Cheap early-out only: it avoids constructing an agent for an idle session.
    // It is no longer the guard — see `try_queue_soft_interrupt` below.
    if !state.is_turn_active(&req.session_id) {
        return Err(StatusCode::CONFLICT.into_response());
    }
    // `steer_refusal` above is the authority for this attribution, and it
    // refuses everything but `Proven` — which is why the stamp is unconditional
    // here on every daemon. Keep it independent of the session store: a live agent can
    // legitimately outlast or race its durable row, but an accepted human steer
    // must never lose its provenance because that auxiliary lookup failed.
    let provenance = Some(biorouter::conversation::message::MessageProvenance {
        kind: biorouter::conversation::message::ProvenanceKind::UserDirect,
        from_session_id: None,
        from_session_name: None,
    });
    let agent = state
        .get_agent_for_route(req.session_id)
        .await
        .map_err(IntoResponse::into_response)?;
    match agent.try_queue_soft_interrupt(req.text, provenance) {
        Ok(turn_id) => Ok((
            StatusCode::ACCEPTED,
            Json(InterruptAccepted {
                turn_id: turn_id.into(),
            }),
        )),
        // #69: the turn the caller addressed has ended. Refusing is the honest
        // answer — queueing for whatever runs next is the bug this replaces.
        Err(InterruptRefused::TurnEnded) => Err(StatusCode::CONFLICT.into_response()),
    }
}

/// Request body for the addressable cancel route.
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct CancelTurnRequest {
    pub session_id: String,
    /// Cancel only this turn generation. A stale Stop request must never cancel
    /// a successor that started in the same session after the UI queued it.
    #[serde(default)]
    pub expected_turn_id: Option<String>,
    /// Wait until the exact cancelled turn has released the session lock before
    /// acknowledging. Stop-and-Send callers use this as their submission
    /// barrier; ordinary Stop callers retain the immediate acknowledgement.
    #[serde(default)]
    pub wait_for_idle: bool,
    /// The caller will submit a replacement turn after cancellation settles.
    /// This is distinct from ordinary Stop because a delegated child's parent
    /// must keep supervising across the gap between the two turns.
    #[serde(default)]
    pub continuation_pending: bool,
    /// Stable, per-window identifier for recovering a Stop-and-Send admission
    /// after a renderer reload. It is an ownership label, not a credential;
    /// user-action proof and the opaque lease remain the authorities.
    #[serde(default)]
    pub continuation_owner_id: Option<String>,
}

/// Response body for the addressable cancel route.
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct CancelTurnResponse {
    /// True when a running turn was found and its cancellation token tripped.
    /// False means there was nothing to cancel — which is a success, not an error.
    pub cancelled: bool,
    /// The id of the turn that was cancelled, when there was one.
    pub turn_id: Option<String>,
    /// True only after the cancelled turn's guard has retired. An idle session
    /// is already settled. An immediate cancellation acknowledgement may be
    /// `false` while the agent loop is still unwinding.
    pub settled: bool,
    /// Opaque exact-generation admission for the replacement `/reply`.
    /// Present only for a settled Stop-and-Send request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation_lease: Option<String>,
}

/// A generation-conditional cancel addressed a different active turn.
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct CancelTurnConflictResponse {
    pub mismatch: bool,
    pub expected_turn_id: String,
    pub active_turn_id: String,
}

/// The two fail-closed 409 shapes returned by `/agent/cancel`.
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(untagged)]
pub enum CancelTurnConflict {
    TurnMismatch(CancelTurnConflictResponse),
    Continuation(ContinuationLeaseErrorResponse),
}

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct AbandonContinuationLeaseRequest {
    pub session_id: String,
    pub continuation_lease: String,
}

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecoverContinuationAction {
    TakeOver,
    Abandon,
}

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct RecoverContinuationRequest {
    pub session_id: String,
    /// Exact retired generation shown by `ResumeAgentResponse.pending_continuation`.
    pub superseded_turn_id: String,
    /// Stable per-window owner returned only to that same window on resume.
    pub continuation_owner_id: String,
    pub action: RecoverContinuationAction,
}

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct RecoverContinuationResponse {
    pub resolution: String,
    pub superseded_turn_id: String,
    /// Present only after this caller explicitly takes ownership. Group
    /// abandonment never returns another window's opaque lease.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation_lease: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct AbandonContinuationLeaseResponse {
    pub resolution: String,
}

#[derive(Debug)]
enum CancelTurnFailure {
    TurnMismatch {
        expected_turn_id: String,
        active_turn_id: String,
    },
    ContinuationRequiresExpectedTurn,
    ContinuationRequiresOwner,
    ContinuationOwnerConflict,
    ContinuationAdmissionInProgress,
    ParentClosing,
    SettlementTimeout,
}

struct ContinuationAdmissionRollback<'a> {
    state: &'a AppState,
    admission: Option<ContinuationAdmission>,
}

impl<'a> ContinuationAdmissionRollback<'a> {
    fn new(state: &'a AppState, admission: Option<ContinuationAdmission>) -> Self {
        Self { state, admission }
    }

    fn commit(&mut self) -> Option<String> {
        let admission = self.admission.take()?;
        if let Some(mark) = admission.mark() {
            mark.commit();
        }
        self.state
            .commit_continuation_lease(admission.token())
            .then(|| admission.token().to_string())
    }
}

impl Drop for ContinuationAdmissionRollback<'_> {
    fn drop(&mut self) {
        if let Some(admission) = self.admission.take() {
            if let Some(mark) = admission.mark() {
                mark.rollback();
            }
            self.state.rollback_continuation_lease(admission.token());
        }
    }
}

impl IntoResponse for CancelTurnFailure {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::TurnMismatch {
                expected_turn_id,
                active_turn_id,
            } => (
                StatusCode::CONFLICT,
                Json(CancelTurnConflictResponse {
                    mismatch: true,
                    expected_turn_id,
                    active_turn_id,
                }),
            )
                .into_response(),
            Self::ContinuationRequiresExpectedTurn => StatusCode::BAD_REQUEST.into_response(),
            Self::ContinuationRequiresOwner => StatusCode::BAD_REQUEST.into_response(),
            Self::ContinuationOwnerConflict => {
                continuation_lease_failure_response(ContinuationLeaseFailure::OwnedByAnother)
            }
            Self::ContinuationAdmissionInProgress => {
                continuation_lease_failure_response(ContinuationLeaseFailure::AdmissionInProgress)
            }
            Self::ParentClosing => {
                continuation_lease_failure_response(ContinuationLeaseFailure::ParentClosing)
            }
            Self::SettlementTimeout => StatusCode::GATEWAY_TIMEOUT.into_response(),
        }
    }
}

/// Safety bound for a cancellation completion barrier.
///
/// Cancellation should normally retire the guard immediately, including from a
/// tool-permission wait. A broken provider or tool must not park the HTTP request
/// forever, though: after this bound the route returns 504, never a 200 that
/// could make Stop-and-Send submit a replacement while the old turn still owns
/// the session lock.
const CANCEL_SETTLEMENT_TIMEOUT: Duration = Duration::from_secs(30);

/// Hard cancel: trip the cancellation token of the turn in flight for this
/// session (BR-62).
///
/// Before this route, cancelling meant dropping the SSE socket — the only thing
/// that closed `tx` and tripped the token — so a turn could not be stopped by a
/// second client, the CLI, or a script, and `/agent/stop` merely evicted the
/// agent from the LRU while the in-flight reply task ran on happily against its
/// own `Arc<Agent>`. Tripping the token unwinds the agent loop at its next
/// boundary and unblocks a tool-permission prompt it may be parked on.
///
/// Deliberately **idempotent**: cancelling a session with no turn in flight (a
/// double-clicked Stop button, a cancel that raced the turn's own completion) is
/// a 200 with `cancelled: false`, never an error. A cancel that reports failure
/// because the thing was already stopped is exactly the unreliability this BR
/// exists to remove. When `expected_turn_id` names an older generation and a
/// successor is active, the route instead fails closed with 409 and leaves that
/// successor untouched.
#[utoipa::path(
    post,
    path = "/agent/cancel",
    // See `/reply` above: the tag is what Task 42b's parity gate selects on.
    tag = "workspace",
    request_body = CancelTurnRequest,
    responses(
        (status = 200, description = "Cancel processed; `cancelled` reports whether a turn was running", body = CancelTurnResponse),
        (status = 400, description = "Stop-and-Send requires an exact turn id and a valid stable continuation owner id"),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 403, description = "The request was not proven to come from the user; on a daemon \
                                      that holds no user-action key, the chat is out of the caller's \
                                      reach or is a subagent's (SD-11)"),
        (status = 409, description = "A different turn generation is active, another client owns the continuation, or its admission is still settling", body = CancelTurnConflict),
        (status = 504, description = "The cancelled turn did not release the session lock before the safety bound"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn cancel_turn(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CancelTurnRequest>,
) -> axum::response::Response {
    if let Err(refusal) = authorize_turn_control(&state, &req.session_id, &headers).await {
        return refusal;
    }
    match cancel_turn_bounded(&state, &req, CANCEL_SETTLEMENT_TIMEOUT).await {
        Ok(response) => Json(response).into_response(),
        Err(failure) => failure.into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/agent/continuation/abandon",
    tag = "workspace",
    request_body = AbandonContinuationLeaseRequest,
    responses(
        (status = 200, description = "The continuation lease is abandoned or was already resolved", body = AbandonContinuationLeaseResponse),
        (status = 403, description = "The request was not proven to come from the user; on a daemon \
                                      that holds no user-action key, the chat is out of the caller's \
                                      reach or is a subagent's (SD-11)"),
        (status = 409, description = "The lease is invalid or belongs to another session", body = ContinuationLeaseErrorResponse)
    )
)]
pub async fn abandon_continuation_lease(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<AbandonContinuationLeaseRequest>,
) -> axum::response::Response {
    if let Err(refusal) = authorize_turn_control(&state, &req.session_id, &headers).await {
        return refusal;
    }
    match state.abandon_continuation_lease(&req.session_id, &req.continuation_lease) {
        Ok(resolution) => {
            let resolution = match resolution {
                ContinuationLeaseAbandonment::Abandoned => "abandoned",
                ContinuationLeaseAbandonment::AlreadyAbandoned => "already_abandoned",
                ContinuationLeaseAbandonment::AlreadyConsumed => "already_consumed",
            };
            Json(AbandonContinuationLeaseResponse {
                resolution: resolution.to_string(),
            })
            .into_response()
        }
        Err(failure) => continuation_lease_failure_response(failure),
    }
}

fn valid_continuation_owner_id(owner_id: &str) -> bool {
    !owner_id.is_empty()
        && owner_id.len() <= 128
        && owner_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.'))
}

#[utoipa::path(
    post,
    path = "/agent/continuation/recover",
    tag = "workspace",
    request_body = RecoverContinuationRequest,
    responses(
        (status = 200, description = "The exact pending continuation was taken over or its whole claim group was abandoned", body = RecoverContinuationResponse),
        (status = 400, description = "The continuation owner id is missing or invalid"),
        (status = 403, description = "The session is out of reach or the request was not proven to \
                                      come from the user; on a daemon that holds no user-action key, \
                                      the chat is out of the caller's reach or is a subagent's (SD-11)"),
        (status = 409, description = "The exact continuation generation was already resolved", body = ContinuationLeaseErrorResponse)
    )
)]
pub async fn recover_continuation(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<RecoverContinuationRequest>,
) -> axum::response::Response {
    if let Err(refusal) = authorize_turn_control(&state, &req.session_id, &headers).await {
        return refusal;
    }
    if !valid_continuation_owner_id(&req.continuation_owner_id) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    // Asked again on a keyless daemon, where `authorize_turn_control` has already
    // asked it: this is the route's own reach gate on EVERY daemon, and a second
    // answer to the same question is the same answer. It stays where the
    // ordering census (`session_reach::every_gated_route_resolves_…`) pins it.
    if let Err(refusal) = crate::routes::session_reach::session_reach(
        state.session_manager(),
        &req.session_id,
        &headers,
    )
    .await
    {
        return refusal.into_response();
    }
    let recovered = match req.action {
        RecoverContinuationAction::TakeOver => state.recover_continuation_for_owner(
            &req.session_id,
            &req.superseded_turn_id,
            &req.continuation_owner_id,
        ),
        RecoverContinuationAction::Abandon => {
            state.abandon_continuation_group(&req.session_id, &req.superseded_turn_id)
        }
    };
    match recovered {
        Ok(ContinuationRecovery::Recovered {
            continuation_lease,
            superseded_turn_id,
        }) => Json(RecoverContinuationResponse {
            resolution: "taken_over".to_string(),
            superseded_turn_id,
            continuation_lease: Some(continuation_lease),
        })
        .into_response(),
        Ok(ContinuationRecovery::Abandoned { superseded_turn_id }) => {
            Json(RecoverContinuationResponse {
                resolution: "abandoned".to_string(),
                superseded_turn_id,
                continuation_lease: None,
            })
            .into_response()
        }
        Err(failure) => continuation_lease_failure_response(failure),
    }
}

enum CancelAdmission {
    Active {
        turn: CancelledTurn,
        continuation: Option<ContinuationAdmission>,
    },
    Complete(CancelTurnResponse),
}

fn idle_cancel_response() -> CancelTurnResponse {
    CancelTurnResponse {
        cancelled: false,
        turn_id: None,
        settled: true,
        continuation_lease: None,
    }
}

fn turn_mismatch_failure(req: &CancelTurnRequest, active_turn_id: String) -> CancelTurnFailure {
    CancelTurnFailure::TurnMismatch {
        expected_turn_id: req
            .expected_turn_id
            .clone()
            .expect("a turn mismatch requires an expected turn id"),
        active_turn_id,
    }
}

fn admit_continuation_cancel(
    state: &AppState,
    req: &CancelTurnRequest,
) -> Result<CancelAdmission, CancelTurnFailure> {
    let expected_turn_id = req
        .expected_turn_id
        .as_deref()
        .ok_or(CancelTurnFailure::ContinuationRequiresExpectedTurn)?;
    let owner_id = req
        .continuation_owner_id
        .as_deref()
        .filter(|owner_id| valid_continuation_owner_id(owner_id))
        .ok_or(CancelTurnFailure::ContinuationRequiresOwner)?;
    match state.cancel_turn_for_continuation_owned(&req.session_id, expected_turn_id, owner_id) {
        ContinuationCancelAttempt::Cancelled { turn, admission } => Ok(CancelAdmission::Active {
            turn,
            continuation: Some(admission),
        }),
        ContinuationCancelAttempt::Retired { turn_id, admission } => {
            let mut admission = ContinuationAdmissionRollback::new(state, Some(admission));
            tracing::debug!(
                session_id = req.session_id,
                turn_id,
                "Continuation admitted for an already-retired turn"
            );
            Ok(CancelAdmission::Complete(CancelTurnResponse {
                cancelled: false,
                turn_id: Some(turn_id),
                settled: true,
                continuation_lease: admission.commit(),
            }))
        }
        ContinuationCancelAttempt::Idle => {
            tracing::debug!(
                "Cancel for session {} found no addressable turn generation",
                req.session_id
            );
            Ok(CancelAdmission::Complete(idle_cancel_response()))
        }
        ContinuationCancelAttempt::TurnMismatch { active_turn_id } => {
            Err(turn_mismatch_failure(req, active_turn_id))
        }
        ContinuationCancelAttempt::OwnerConflict => {
            Err(CancelTurnFailure::ContinuationOwnerConflict)
        }
        ContinuationCancelAttempt::AdmissionInProgress => {
            Err(CancelTurnFailure::ContinuationAdmissionInProgress)
        }
        ContinuationCancelAttempt::ParentClosing => Err(CancelTurnFailure::ParentClosing),
    }
}

fn admit_ordinary_cancel(
    state: &AppState,
    req: &CancelTurnRequest,
) -> Result<CancelAdmission, CancelTurnFailure> {
    match state.cancel_turn_waitable(&req.session_id, req.expected_turn_id.as_deref()) {
        CancelTurnAttempt::Cancelled(turn) => Ok(CancelAdmission::Active {
            turn,
            continuation: None,
        }),
        CancelTurnAttempt::Idle => {
            if req.expected_turn_id.is_none()
                && biorouter::agents::subagent_handle::cancel_initializing_child(&req.session_id)
            {
                tracing::info!(
                    session_id = req.session_id,
                    "Cancelled delegated child before its initial runtime became ready"
                );
                return Ok(CancelAdmission::Complete(CancelTurnResponse {
                    cancelled: true,
                    turn_id: None,
                    settled: true,
                    continuation_lease: None,
                }));
            }
            tracing::debug!(
                "Cancel for session {} found no turn in flight",
                req.session_id
            );
            Ok(CancelAdmission::Complete(idle_cancel_response()))
        }
        CancelTurnAttempt::TurnMismatch { active_turn_id } => {
            Err(turn_mismatch_failure(req, active_turn_id))
        }
    }
}

fn admit_cancel(
    state: &AppState,
    req: &CancelTurnRequest,
) -> Result<CancelAdmission, CancelTurnFailure> {
    if req.continuation_pending {
        admit_continuation_cancel(state, req)
    } else {
        admit_ordinary_cancel(state, req)
    }
}

async fn await_cancel_settlement(
    turn: &CancelledTurn,
    req: &CancelTurnRequest,
    settlement_timeout: Duration,
) -> Result<(), CancelTurnFailure> {
    if (req.wait_for_idle || req.continuation_pending)
        && tokio::time::timeout(settlement_timeout, turn.wait_until_settled())
            .await
            .is_err()
        && !turn.is_settled()
    {
        tracing::warn!(
            session_id = req.session_id,
            turn_id = turn.turn_id(),
            timeout_secs = settlement_timeout.as_secs_f64(),
            "cancelled turn did not release its session lock before the safety bound"
        );
        return Err(CancelTurnFailure::SettlementTimeout);
    }
    Ok(())
}

async fn cancel_turn_bounded(
    state: &AppState,
    req: &CancelTurnRequest,
    settlement_timeout: Duration,
) -> Result<CancelTurnResponse, CancelTurnFailure> {
    let (turn, continuation_admission) = match admit_cancel(state, req)? {
        CancelAdmission::Active { turn, continuation } => (turn, continuation),
        CancelAdmission::Complete(response) => return Ok(response),
    };
    let mut continuation_admission =
        ContinuationAdmissionRollback::new(state, continuation_admission);

    tracing::info!(
        "Cancelled turn {} for session {}",
        turn.turn_id(),
        req.session_id
    );

    await_cancel_settlement(&turn, req, settlement_timeout).await?;

    let continuation_lease = continuation_admission.commit();

    Ok(CancelTurnResponse {
        cancelled: true,
        turn_id: Some(turn.turn_id().to_string()),
        settled: turn.is_settled(),
        continuation_lease,
    })
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/reply",
            post(reply).layer(DefaultBodyLimit::max(50 * 1024 * 1024)),
        )
        .route("/interrupt", post(interrupt))
        .route("/agent/cancel", post(cancel_turn))
        .route(
            "/agent/continuation/abandon",
            post(abandon_continuation_lease),
        )
        .route("/agent/continuation/recover", post(recover_continuation))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_events_preserve_machine_readable_metadata() {
        let event = MessageEvent::error(
            "quota exhausted",
            "provider_failure",
            TurnErrorScope::Provider,
            false,
            Some("quota".to_string()),
        );

        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["type"], "Error");
        assert_eq!(value["error"], "quota exhausted");
        assert_eq!(value["code"], "provider_failure");
        assert_eq!(value["scope"], "provider");
        assert_eq!(value["retryable"], false);
        assert_eq!(value["provider_kind"], "quota");
    }

    /// #59: the frame that makes `expectedMessageIds` satisfiable has to reach
    /// the client as *data*, not as a re-send of the messages. A client reads
    /// `messages[].id` into the set it will hand back to
    /// `POST /sessions/{id}/edit_message`, and `userVisible` tells it which of
    /// those it must draw — the difference between a row it is deliberately not
    /// shown and one it was never told about.
    #[test]
    fn persisted_message_ids_reach_the_wire_with_their_visibility() {
        let event = MessageEvent::MessagesPersisted {
            messages: vec![
                PersistedMessage {
                    id: "019fa8-visible".to_string(),
                    user_visible: true,
                },
                PersistedMessage {
                    id: "019fa8-model-only".to_string(),
                    user_visible: false,
                },
            ],
        };

        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["type"], "MessagesPersisted");
        assert_eq!(value["messages"][0]["id"], "019fa8-visible");
        assert_eq!(value["messages"][0]["userVisible"], true);
        assert_eq!(value["messages"][1]["id"], "019fa8-model-only");
        assert_eq!(value["messages"][1]["userVisible"], false);
        // Ids only — a client must never have to diff message bodies to learn
        // what the store holds, and re-sending them per turn would be a second
        // copy of the whole transcript on the hot path.
        assert!(value["messages"][0].get("content").is_none());
    }

    /// **[#59]** The accounting frame still reaches the WIRE through the real
    /// bus pump and reply-stream consumer after the refactor.
    ///
    /// This is the only test in this plan that can catch the failure mode the
    /// 7 → 8 ordering exists to prevent: `map_bus_event` repaired with
    /// `_ => None`, or a bus loop that filters the frame out. Everything else
    /// stays green — `reply.rs`'s own
    /// `persisted_message_ids_reach_the_wire_with_their_visibility` tests the
    /// enum, not the handler, and the desktop store deliberately does not
    /// consume the frame yet. The consequence of missing it is a 409 on
    /// `POST /sessions/{id}/edit_message` for every session, in the shipped app.
    ///
    /// It publishes onto the bus directly rather than driving a real turn: a
    /// provider-less turn persists nothing, so the frame has to be injected to
    /// be observed. That is exactly the right scope here — the property under
    /// test is `bus → wire`, which is `/reply`'s two production halves; the agent's half
    /// (`persist → bus`) is held by
    /// `conversation_writeback_freshness::a_client_that_watched_the_turn_knows_every_stored_message_id`.
    #[tokio::test]
    async fn a_persisted_batch_on_the_bus_reaches_the_reply_client() {
        use biorouter::agents::{AgentEvent, PersistedMessage};
        use biorouter::session_events::{self, SessionBusEvent};

        let state = AppState::new().await.unwrap();
        let session_id = "reply-persisted-frame".to_string();
        let bus = session_events::subscribe(&session_id);
        let stream = TurnStream::new(&session_id, "turn-persisted-frame");
        let pump = tokio::spawn(pump_bus_into_stream(
            state.clone(),
            session_id.clone(),
            bus,
            stream.claim_writer().expect("the test owns this log"),
            CancellationToken::new(),
            Duration::ZERO,
        ));
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
        let client = tokio::spawn(drain_stream_to_client(
            state,
            session_id.clone(),
            Arc::clone(&stream),
            0,
            false,
            tx,
        ));

        session_events::publish(
            &session_id,
            SessionBusEvent::Agent(AgentEvent::MessagesPersisted(vec![PersistedMessage {
                id: "probe-1".into(),
                user_visible: true,
            }])),
        );
        session_events::publish(
            &session_id,
            SessionBusEvent::TurnFinished {
                reason: "stop".into(),
                token_state: None,
            },
        );

        tokio::time::timeout(Duration::from_secs(10), pump)
            .await
            .expect("the terminal event must stop the bus pump")
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), client)
            .await
            .expect("the closed turn stream must stop the reply client")
            .unwrap();
        let mut text = String::new();
        while let Some(frame) = rx.recv().await {
            text.push_str(&frame);
        }
        assert!(
            text.contains("\"type\":\"MessagesPersisted\""),
            "the #59 accounting frame never reached the wire. If `map_bus_event` \
             was repaired with `_ => None`, this is that. Body: {text}"
        );
        assert!(text.contains("probe-1"), "…and it carries the ids: {text}");
    }

    /// The wire contract: a turn's frames reach the /reply client exactly as
    /// before, and a concurrent observer sees the same ones.
    #[tokio::test]
    async fn reply_streams_the_turn_and_an_observer_sees_the_same_frames() {
        use tower::ServiceExt;
        let state = AppState::new().await.unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let session = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "reply-refactor".to_string(),
                biorouter::session::session_manager::SessionType::User,
            )
            .await
            .unwrap();

        let mut observer = biorouter::session_events::subscribe(&session.id);

        // No provider is configured on this fresh agent, so the turn starts and
        // fails fast — the lifecycle bracket and the error envelope are what we
        // assert, and both must survive the refactor.
        let body = serde_json::json!({
            "user_message": serde_json::to_value(
                biorouter::conversation::message::Message::user().with_text("hi")
            ).unwrap(),
            "session_id": session.id,
        });
        let app = routes(state.clone());
        let response = app
            .oneshot(
                axum::http::Request::post("/reply")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&bytes);
        // The client still receives a terminal frame — Finish or Error, never
        // a stream that just stops.
        assert!(
            text.contains("\"type\":\"Finish\"") || text.contains("\"type\":\"Error\""),
            "no terminal frame in /reply body: {text}"
        );

        // What the /reply CLIENT actually received, heartbeats dropped and the
        // stream envelope (`seq` / `turn_id` / `replay`) stripped. `turn_id` is
        // also the semantic payload of TurnStarted, so that opening frame keeps
        // it while every other frame drops the stream-bookkeeping copy.
        let client: Vec<serde_json::Value> = text
            .lines()
            .filter_map(sse_frame)
            .map(|mut frame| {
                if let Some(object) = frame.as_object_mut() {
                    object.remove("seq");
                    if object.get("type").and_then(serde_json::Value::as_str) != Some("TurnStarted")
                    {
                        object.remove("turn_id");
                    }
                    object.remove("replay");
                }
                frame
            })
            .collect();

        // What an observer of the same turn would render: the raw bus events it
        // saw, through the one shared mapper. Both consumers subscribed before
        // the turn could publish, so the two sequences are comparable frame for
        // frame — which is the whole point of §4.2 and the reason the mapper is
        // `pub(crate)` rather than duplicated per route.
        let mut observer_token_state = TokenState::default();
        let mut saw_started = false;
        let mut observed: Vec<serde_json::Value> = Vec::new();
        while let Ok(ev) = observer.try_recv() {
            if matches!(
                ev,
                biorouter::session_events::SessionBusEvent::TurnStarted { .. }
            ) {
                saw_started = true;
            }
            if let Some(frame) =
                crate::routes::session_events::map_bus_event(ev, &mut observer_token_state)
            {
                observed.push(serde_json::to_value(&frame).unwrap());
            }
        }
        assert!(saw_started, "the observer saw the turn's opening bracket");
        assert_eq!(
            client, observed,
            "the /reply client and a concurrent observer must receive the SAME \
             frames, not merely both receive something"
        );
        assert_eq!(
            frame_types(&client),
            vec!["TurnStarted", "Error"],
            "a provider-less turn has one opening and one terminal frame: {client:?}"
        );
    }

    /// BR-62's duplicate detection must survive — but its ANSWER has changed. A
    /// re-POST of the same `turn_id` is still not a second turn; it is now
    /// answered **200 with that turn's stream, replayed from seq 0**, instead of
    /// a 409 with no way back into a turn that is still spending its tokens.
    ///
    /// That is the contract's item 2, and it is the only thing standing between
    /// a user who reloaded their window and a turn they can neither see nor stop.
    #[tokio::test]
    async fn a_reposted_turn_id_attaches_to_the_running_turn_from_seq_zero() {
        use tower::ServiceExt;
        let state = AppState::new().await.unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let session = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "idem".to_string(),
                biorouter::session::session_manager::SessionType::User,
            )
            .await
            .unwrap();
        // Hold the lock under a known key and log two frames on its stream, as a
        // running turn would have.
        let guard = state
            .try_begin_turn_idempotent(
                &session.id,
                tokio_util::sync::CancellationToken::new(),
                Some("client-turn-1".to_string()),
            )
            .unwrap();
        let stream = guard.stream();
        let _writer = stream
            .claim_writer()
            .expect("this test writes the log itself, standing in for the pump");
        stream.publish(&MessageEvent::Message {
            message: Message::assistant().with_id("m-1").with_text("half an "),
            token_state: TokenState::default(),
        });
        stream.publish(&MessageEvent::Message {
            message: Message::assistant().with_id("m-1").with_text("answer"),
            token_state: TokenState::default(),
        });

        let body = serde_json::json!({
            "user_message": serde_json::to_value(
                biorouter::conversation::message::Message::user().with_text("hi")
            ).unwrap(),
            "session_id": session.id,
            "turn_id": "client-turn-1",
        });
        let response = routes(state.clone())
            .oneshot(
                axum::http::Request::post("/reply")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::OK,
            "a re-POST of a live turn must be ATTACHED, not refused"
        );

        // End the turn so the response terminates, then read what it carried.
        // In production the PUMP closes the log on its way out (the guard
        // deliberately does not — see `TurnGuard::drop`); this test has no pump,
        // so it stands in for one.
        stream.publish(&MessageEvent::Finish {
            turn_id: None,
            reason: "stop".to_string(),
            token_state: TokenState::default(),
        });
        stream.close();
        drop(guard);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&bytes);
        let frames: Vec<serde_json::Value> = text.lines().filter_map(sse_frame).collect();

        assert_eq!(
            frames[0]["seq"],
            serde_json::json!(0),
            "the attach replays the turn FROM ITS START, not from where it joined: {frames:?}"
        );
        assert_eq!(
            frames[0]["replay"],
            serde_json::json!(true),
            "…and says so, so the client can apply it idempotently: {frames:?}"
        );
        assert!(
            text.contains("half an ") && text.contains("answer"),
            "no progress may be lost across an attach: {text}"
        );
        assert!(
            frames
                .last()
                .is_some_and(|f| f["type"] == "Error" || f["type"] == "Finish"),
            "an attached response still ends on a terminal frame: {frames:?}"
        );
    }

    /// `from_seq` is the cheap path: a client that already rendered frames
    /// `0..N` asks only for the rest, so an SSE reconnect that lost nothing costs
    /// nothing and cannot double-render.
    ///
    /// It rides in the BODY. `/reply` is generated with `query?: never`, so a
    /// query parameter is unreachable from the typed client — this test posts it
    /// the way the client does, and would fail if the field went back to the
    /// query string.
    #[tokio::test]
    async fn from_seq_asks_only_for_the_frames_the_client_is_missing() {
        use tower::ServiceExt;
        let state = AppState::new().await.unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let session = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "from-seq-session".to_string(),
                biorouter::session::session_manager::SessionType::User,
            )
            .await
            .unwrap();
        let guard = state
            .try_begin_turn_idempotent(
                &session.id,
                tokio_util::sync::CancellationToken::new(),
                Some("client-turn-1".to_string()),
            )
            .unwrap();
        let stream = guard.stream();
        let _writer = stream
            .claim_writer()
            .expect("this test writes the log itself, standing in for the pump");
        for chunk in ["one", "two", "three"] {
            stream.publish(&MessageEvent::Message {
                message: Message::assistant().with_id("m-1").with_text(chunk),
                token_state: TokenState::default(),
            });
        }

        let body = serde_json::json!({
            "user_message": serde_json::to_value(Message::user().with_text("hi")).unwrap(),
            "session_id": session.id,
            "turn_id": "client-turn-1",
            "from_seq": 2,
        });
        let response = routes(state.clone())
            .oneshot(
                axum::http::Request::post("/reply")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        stream.close();
        drop(guard);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("one") && !text.contains("two"),
            "frames below from_seq must not be re-sent: {text}"
        );
        assert!(
            text.contains("three"),
            "…and the ones above it must be: {text}"
        );
    }

    /// The boundary that makes `from_seq` safe to read as "this is an attach":
    /// a FIRST POST carries a `turn_id` (its idempotency key, BR-62) and no
    /// `from_seq`, and must still start a turn.
    ///
    /// Without this pinned, the attach-miss guard would be one careless client
    /// change away from refusing every ordinary submit with `turn_not_found` —
    /// a chat that answers nothing at all. The renderer's `buildAttachRequest`
    /// is the only place that sets the field; `submitPreparedMessage` builds its
    /// body without it.
    #[tokio::test]
    async fn a_first_post_carrying_an_idempotency_key_still_starts_a_turn() {
        use tower::ServiceExt;
        let state = AppState::new().await.unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let session_id = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "first-post-with-key".to_string(),
                biorouter::session::session_manager::SessionType::User,
            )
            .await
            .unwrap()
            .id;
        let body = serde_json::json!({
            "user_message": serde_json::to_value(Message::user().with_text("hi")).unwrap(),
            "session_id": session_id,
            "turn_id": "a-key-the-client-minted",
        });
        let response = routes(state.clone())
            .oneshot(
                axum::http::Request::post("/reply")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert!(
            state.knows_turn(&session_id, "a-key-the-client-minted"),
            "a first POST with an idempotency key must START a turn, not be read \
             as an attach to one that does not exist"
        );
        state.cancel_turn(&session_id);
        let _ = tokio::time::timeout(
            Duration::from_secs(10),
            axum::body::to_bytes(response.into_body(), usize::MAX),
        )
        .await;
    }

    /// **The bug, as a test.** The last observer leaves mid-turn; the turn must
    /// CONTINUE. Before this work, the failed `tx.send` into the departed
    /// response called `cancel_token.cancel()` and the turn died with the window.
    #[tokio::test]
    async fn the_last_observer_leaving_does_not_stop_the_turn() {
        let state = AppState::new().await.unwrap();
        let session_id = "observer-leaves".to_string();
        let cancel = CancellationToken::new();
        let guard = state
            .try_begin_turn_idempotent(&session_id, cancel.clone(), Some("t-1".into()))
            .unwrap();
        let stream = guard.stream();
        let _writer = stream
            .claim_writer()
            .expect("this test writes the log itself, standing in for the pump");

        // One observer, attached to a real drain task over a real channel...
        let (tx, rx) = mpsc::channel::<String>(4);
        let drain = tokio::spawn(drain_stream_to_client(
            state.clone(),
            session_id.clone(),
            Arc::clone(&stream),
            0,
            false,
            tx,
        ));
        stream.publish(&MessageEvent::Message {
            message: Message::assistant().with_id("m-1").with_text("before"),
            token_state: TokenState::default(),
        });

        // ...which now hangs up, exactly as a closed window does.
        drop(rx);
        tokio::time::timeout(Duration::from_secs(10), drain)
            .await
            .expect("the drain must notice the hang-up and end")
            .unwrap();

        assert!(
            !cancel.is_cancelled(),
            "a departing observer must NEVER cancel the turn: this is the bug"
        );
        assert!(
            state.is_turn_active(&session_id),
            "the turn is still running"
        );

        // And the turn keeps producing, into a log the next client can read.
        stream.publish(&MessageEvent::Message {
            message: Message::assistant().with_id("m-1").with_text("after"),
            token_state: TokenState::default(),
        });
        stream.publish(&MessageEvent::Finish {
            turn_id: None,
            reason: "stop".to_string(),
            token_state: TokenState::default(),
        });
        stream.close();
        drop(guard);

        let replayed = collect_sse(&stream).await.join("");
        assert!(
            replayed.contains("before") && replayed.contains("after"),
            "the whole turn, across the gap with no observers, must be replayable: {replayed}"
        );
    }

    /// Two observers of one turn receive every frame, in identical order. This
    /// is what makes attach-before-detach legal: during a tab handoff BOTH
    /// windows are attached, so there is no instant with nobody watching.
    #[tokio::test]
    async fn two_simultaneous_observers_receive_identical_frames() {
        let state = AppState::new().await.unwrap();
        let session_id = "two-observers".to_string();
        let guard = state
            .try_begin_turn_idempotent(&session_id, CancellationToken::new(), Some("t-1".into()))
            .unwrap();
        let stream = guard.stream();
        let _writer = stream
            .claim_writer()
            .expect("this test writes the log itself, standing in for the pump");

        let observe = |from_seq: u64| {
            let (tx, mut rx) = mpsc::channel::<String>(64);
            let task = tokio::spawn(drain_stream_to_client(
                state.clone(),
                session_id.clone(),
                Arc::clone(&stream),
                from_seq,
                false,
                tx,
            ));
            async move {
                let mut frames = Vec::new();
                while let Some(raw) = rx.recv().await {
                    frames.extend(sse_frame(&raw));
                }
                task.await.unwrap();
                frames
            }
        };
        let old_window = observe(0);
        let new_window = observe(0);

        for chunk in ["alpha", "beta", "gamma"] {
            stream.publish(&MessageEvent::Message {
                message: Message::assistant().with_id("m-1").with_text(chunk),
                token_state: TokenState::default(),
            });
        }
        stream.publish(&MessageEvent::Finish {
            turn_id: None,
            reason: "stop".to_string(),
            token_state: TokenState::default(),
        });
        stream.close();
        drop(guard);

        let (a, b) = tokio::time::timeout(
            Duration::from_secs(10),
            futures::future::join(old_window, new_window),
        )
        .await
        .expect("both observers must terminate on the turn's terminal frame");

        assert_eq!(
            frame_types(&a),
            vec!["Message", "Message", "Message", "Finish"]
        );
        assert_eq!(a, b, "two observers of one turn must see identical frames");
        assert_eq!(
            a.iter()
                .map(|f| f["seq"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3],
            "…under one shared numbering, or they cannot agree on what frame 7 is"
        );
    }

    /// Attaching AFTER the turn completed answers with the TERMINAL FRAME AND
    /// NOTHING ELSE. Not a hang, not a second turn, and — the part that is easy
    /// to get wrong — not the backlog.
    ///
    /// The sequence numbers cannot save a replay here, which is why this is a
    /// rule rather than a preference. A window dies mid-turn; the turn completes
    /// with nobody attached; the window comes back, reads the session from the
    /// store — so its transcript ALREADY CONTAINS the finished turn — and
    /// re-POSTs its stale pointer with a high-water mark of -1, because this
    /// renderer never saw a frame. Every replayed frame is then above the gate
    /// and the whole turn is rendered a second time. The store is the authority
    /// for a completed turn; the stream's only remaining job is to say it ended.
    #[tokio::test]
    async fn attaching_after_the_turn_completed_sends_only_its_terminal_frame() {
        use tower::ServiceExt;
        let state = AppState::new().await.unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let session_id = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "attach-after-finish".to_string(),
                biorouter::session::session_manager::SessionType::User,
            )
            .await
            .unwrap()
            .id;
        let guard = state
            .try_begin_turn_idempotent(
                &session_id,
                CancellationToken::new(),
                Some("client-turn-1".into()),
            )
            .unwrap();
        let stream = guard.stream();
        let _writer = stream
            .claim_writer()
            .expect("this test writes the log itself, standing in for the pump");
        stream.publish(&MessageEvent::Message {
            message: Message::assistant().with_id("m-1").with_text("the answer"),
            token_state: TokenState::default(),
        });
        stream.publish(&MessageEvent::Finish {
            turn_id: None,
            reason: "stop".to_string(),
            token_state: TokenState::default(),
        });
        stream.close();
        drop(guard); // the turn ends, and its entry is RETIRED, not deleted
        assert!(!state.is_turn_active(&session_id), "the turn is over");

        let body = serde_json::json!({
            "user_message": serde_json::to_value(Message::user().with_text("hi")).unwrap(),
            "session_id": session_id,
            "turn_id": "client-turn-1",
        });
        let response = tokio::time::timeout(
            Duration::from_secs(10),
            routes(state.clone()).oneshot(
                axum::http::Request::post("/reply")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            ),
        )
        .await
        .expect("a late attach must answer, never hang")
        .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let bytes = tokio::time::timeout(
            Duration::from_secs(10),
            axum::body::to_bytes(response.into_body(), usize::MAX),
        )
        .await
        .expect("…and must END, never hang")
        .unwrap();
        let text = String::from_utf8_lossy(&bytes);
        let frames: Vec<serde_json::Value> = text.lines().filter_map(sse_frame).collect();

        assert_eq!(
            frame_types(&frames),
            vec!["Finish"],
            "a completed turn answers with its terminal frame ALONE: {text}"
        );
        assert!(
            !text.contains("the answer"),
            "replaying the backlog of a persisted turn re-renders it as duplicates: {text}"
        );
        assert!(
            !state.is_turn_active(&session_id),
            "…and starts no second turn: the tokens are spent once"
        );
    }

    /// The turn's OWN terminal frame must reach the client — not the synthesized
    /// stand-in that closing the log too early would produce.
    ///
    /// This pins a race the obvious implementation loses. `TurnGuard::drop` runs
    /// the instant the RUNNER returns, but the runner's last act was to
    /// *publish* its terminal onto the session bus — the pump has not
    /// necessarily read it yet. Close the log in `Drop` (which is what an
    /// earlier revision of this work did) and `publish` refuses the real frame
    /// as post-terminal, so every healthy turn ends in
    /// `stream_ended_without_terminal` instead of its true `Finish`/`Error`.
    /// Both `the_reply_body_carries_exactly_one_terminal_frame` and
    /// `reply_streams_the_turn_and_an_observer_sees_the_same_frames` count
    /// terminals rather than reading them, so neither catches it; this does.
    #[tokio::test]
    async fn the_runners_own_terminal_reaches_the_client_not_a_synthesized_one() {
        use tower::ServiceExt;
        let state = AppState::new().await.unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let session = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "real-terminal".to_string(),
                biorouter::session::session_manager::SessionType::User,
            )
            .await
            .unwrap();
        let body = serde_json::json!({
            "user_message": serde_json::to_value(Message::user().with_text("hi")).unwrap(),
            "session_id": session.id,
        });
        let response = routes(state.clone())
            .oneshot(
                axum::http::Request::post("/reply")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("stream_ended_without_terminal"),
            "the log was closed before the pump read the runner's terminal: {text}"
        );
        // A provider-less turn's real terminal is the runner's classified error.
        assert!(
            text.contains("\"type\":\"Error\""),
            "no terminal frame at all: {text}"
        );
    }

    /// A retired turn whose log was never closed — an injected workspace turn
    /// has no `/reply` pump to close it — still answers a late attach instead of
    /// handing it an empty stream.
    #[tokio::test]
    async fn a_retired_turn_with_no_pump_still_answers_a_late_attach() {
        let state = AppState::new().await.unwrap();
        let session_id = "retired-no-pump".to_string();
        let guard = state
            .try_begin_turn_idempotent(&session_id, CancellationToken::new(), Some("t-1".into()))
            .unwrap();
        let stream = guard.stream();
        drop(guard); // retired with an OPEN log and no terminal
        assert!(!stream.is_closed());

        let (tx, mut rx) = mpsc::channel::<String>(8);
        tokio::time::timeout(
            Duration::from_secs(10),
            drain_stream_to_client(state, session_id, Arc::clone(&stream), 0, true, tx),
        )
        .await
        .expect("a late attach must answer, never hang");

        let frame = rx.try_recv().expect("one terminal frame");
        assert!(
            frame.contains("\"type\":\"Error\"") && frame.contains("turn_has_no_stream"),
            "got: {frame}"
        );
        assert!(rx.try_recv().is_err(), "and nothing else");
        // …and it answered WITHOUT closing the log. This is the half the earlier
        // version got wrong: it closed here, which is indistinguishable from the
        // "pump one scheduler tick behind" case and stole a healthy turn's real
        // terminal. A reader answers from what is there; it never edits the log
        // to make its own answer true.
        assert!(
            !stream.is_closed(),
            "a reader must not close a log it does not own"
        );
    }

    /// Cancel still cancels, promptly, and from a caller that is not watching.
    ///
    /// This is the property most at risk from decoupling the turn from its
    /// listeners: once "everyone left" no longer stops a turn, an explicit stop
    /// is the ONLY prompt way to end one, and it has to reach both the runner
    /// (via the token) and every attached response (via the closed log). A
    /// cancel that leaves a response hanging open looks exactly like the freeze
    /// this work exists to remove.
    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_ends_the_turn_and_every_attached_response() {
        let state = AppState::new().await.unwrap();
        let session_id = "cancel-with-observers".to_string();
        let cancel = CancellationToken::new();
        let guard = state
            .try_begin_turn_idempotent(&session_id, cancel.clone(), Some("t-1".into()))
            .unwrap();
        let stream = guard.stream();
        let bus = biorouter::session_events::subscribe(&session_id);
        let pump = tokio::spawn(pump_bus_into_stream(
            state.clone(),
            session_id.clone(),
            bus,
            stream.claim_writer().expect("the test owns this log"),
            cancel.clone(),
            Duration::ZERO,
        ));

        // Two watchers, neither of which issues the cancel.
        let watch = || {
            let (tx, mut rx) = mpsc::channel::<String>(64);
            let task = tokio::spawn(drain_stream_to_client(
                state.clone(),
                session_id.clone(),
                Arc::clone(&stream),
                0,
                false,
                tx,
            ));
            async move {
                let mut frames = Vec::new();
                while let Some(raw) = rx.recv().await {
                    frames.extend(sse_frame(&raw));
                }
                task.await.unwrap();
                frames
            }
        };
        let (a, b) = (watch(), watch());

        // A third party stops the turn — the CLI, a script, another window.
        assert_eq!(
            state.cancel_turn(&session_id).as_deref(),
            Some(guard.turn_id())
        );
        assert!(
            cancel.is_cancelled(),
            "the runner's token is tripped at once"
        );

        let (frames_a, frames_b) =
            tokio::time::timeout(Duration::from_secs(10), futures::future::join(a, b))
                .await
                .expect("a cancel must end every attached response, not leave them open");
        for frames in [&frames_a, &frames_b] {
            assert!(
                frames
                    .last()
                    .is_some_and(|f| f["type"] == "Finish" || f["type"] == "Error"),
                "every observer is told the turn ended: {frames:?}"
            );
        }
        pump.await.unwrap();
        drop(guard);
    }

    /// A client attaching to a turn it did not start does not know the prompt
    /// that began it — it sends its transcript's trailing user message, or an
    /// empty one. Honouring that would inject a phantom prompt into a running
    /// turn, so the attach path drops `user_message` entirely.
    #[tokio::test]
    async fn an_attach_ignores_the_user_message_it_carries() {
        use tower::ServiceExt;
        let state = AppState::new().await.unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let session_id = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "attach-ignores-prompt".to_string(),
                biorouter::session::session_manager::SessionType::User,
            )
            .await
            .unwrap()
            .id;
        let guard = state
            .try_begin_turn_idempotent(&session_id, CancellationToken::new(), Some("t-1".into()))
            .unwrap();
        let stream = guard.stream();
        let _writer = stream
            .claim_writer()
            .expect("this test writes the log itself, standing in for the pump");

        let body = serde_json::json!({
            "user_message": serde_json::to_value(
                Message::user().with_text("PHANTOM PROMPT")
            ).unwrap(),
            "session_id": session_id,
            "turn_id": "t-1",
        });
        let response = routes(Arc::clone(&state))
            .oneshot(
                axum::http::Request::post("/reply")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        stream.publish(&MessageEvent::Finish {
            turn_id: None,
            reason: "stop".to_string(),
            token_state: TokenState::default(),
        });
        stream.close();
        drop(guard);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(
            !String::from_utf8_lossy(&bytes).contains("PHANTOM PROMPT"),
            "an attach is a READ; its user_message must never enter the turn"
        );
    }

    /// The attach pointer works under EITHER name. A window that reloaded did
    /// not keep the idempotency key it chose — what it has is the `turn_id`
    /// stamped on the last frame it rendered, or the one `/agent/resume` handed
    /// it, and both of those are the server's `turn-N`. Matching only the
    /// client's key would 409 every reload-then-reattach.
    #[tokio::test]
    async fn the_server_assigned_turn_id_is_also_a_valid_attach_pointer() {
        use tower::ServiceExt;
        let state = AppState::new().await.unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let session_id = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "attach-by-server-id".to_string(),
                biorouter::session::session_manager::SessionType::User,
            )
            .await
            .unwrap()
            .id;
        let guard = state
            .try_begin_turn_idempotent(
                &session_id,
                CancellationToken::new(),
                Some("a-key-the-client-has-since-lost".into()),
            )
            .unwrap();
        let _writer = guard
            .stream()
            .claim_writer()
            .expect("this test writes the log itself, standing in for the pump");
        let server_turn_id = state
            .active_turn_id(&session_id)
            .expect("/agent/resume hands this to a reloading window");
        assert_eq!(server_turn_id, guard.turn_id());

        let body = serde_json::json!({
            "user_message": serde_json::to_value(Message::user().with_text("hi")).unwrap(),
            "session_id": session_id,
            "turn_id": server_turn_id,
        });
        let response = routes(Arc::clone(&state))
            .oneshot(
                axum::http::Request::post("/reply")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::OK,
            "the id the client actually holds must attach, not 409"
        );
        assert_eq!(
            state.active_turn_session_ids(),
            vec![session_id],
            "and no second turn started"
        );
        drop(guard);
    }

    /// **[F]** The new backpressure semantics, tested against the REAL broadcast
    /// channel instead of a constant function: a consumer that falls behind
    /// genuinely receives `Lagged` (so the branch is not dead code), and the
    /// branch's action is a storage resync frame, not a silent skip.
    #[tokio::test]
    async fn a_lagged_consumer_gets_a_storage_resync_frame() {
        use biorouter::session_events::{self, SessionBusEvent, BUS_CAPACITY};
        let state = AppState::new().await.unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let session = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "lagged".to_string(),
                biorouter::session::session_manager::SessionType::User,
            )
            .await
            .unwrap();

        let mut rx = session_events::subscribe(&session.id);
        // Overrun the ring without reading. BUS_CAPACITY + 1 is the smallest
        // overflow; if this ever stops producing Lagged the resync branch is
        // unreachable and the test is the thing that says so.
        for i in 0..(BUS_CAPACITY + 1) {
            session_events::publish(
                &session.id,
                SessionBusEvent::TurnStarted {
                    turn_id: format!("turn-{i}"),
                },
            );
        }
        assert!(
            matches!(
                rx.recv().await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_))
            ),
            "the ring must actually overflow, or /reply's resync branch is dead code"
        );

        assert_eq!(on_bus_lag_action(), BusLagAction::ResyncFromStorage);
        let frame = crate::routes::session_events::bus_lag_resync_frame(
            &state,
            &session.id,
            &TokenState::default(),
        )
        .await
        .expect("a resync frame is produced from storage");
        assert!(matches!(frame, MessageEvent::UpdateConversation { .. }));
    }

    /// The error envelope's four fields survive the round trip through the bus.
    /// `provider` is the scope under test on purpose: it is the one the
    /// desktop's rate-limit / retry / compaction recovery keys off, and the one
    /// a three-variant `wire_value` would have silently mismapped.
    #[test]
    fn turn_error_bus_event_maps_back_to_the_exact_error_frame() {
        use crate::routes::session_events::map_bus_event;
        let mut token_state = Default::default();
        let mapped = map_bus_event(
            biorouter::session_events::SessionBusEvent::TurnError {
                message: "rate limited".into(),
                code: "provider_failure".into(),
                scope: "provider".into(),
                retryable: true,
                provider_kind: Some("rate_limit".into()),
            },
            &mut token_state,
        )
        .expect("maps");
        let json = serde_json::to_value(&mapped).unwrap();
        assert_eq!(json["type"], "Error");
        assert_eq!(json["code"], "provider_failure");
        assert_eq!(json["retryable"], serde_json::Value::Bool(true));
        assert_eq!(json["provider_kind"], "rate_limit");
        // The scope must round-trip to the ENUM, not to a string field.
        assert_eq!(
            serde_json::to_value(TurnErrorScope::Provider).unwrap(),
            json["scope"]
        );
    }

    /// **[F]** Exactly ONE terminal frame per turn, asserted on the bytes the
    /// client actually receives rather than on the bus. A runner that published
    /// both the raw `AgentEvent::TurnAborted` and the classified `TurnError`
    /// would emit two `Error` frames for one abort (`map_bus_event` maps both),
    /// and the desktop would render the turn as failing twice.
    #[tokio::test]
    async fn the_reply_body_carries_exactly_one_terminal_frame() {
        use tower::ServiceExt;
        let state = AppState::new().await.unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let session = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "one-terminal".to_string(),
                biorouter::session::session_manager::SessionType::User,
            )
            .await
            .unwrap();
        let body = serde_json::json!({
            "user_message": serde_json::to_value(
                biorouter::conversation::message::Message::user().with_text("hi")
            ).unwrap(),
            "session_id": session.id,
        });
        let response = routes(state.clone())
            .oneshot(
                axum::http::Request::post("/reply")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&bytes);
        let terminals = text
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter_map(|json| serde_json::from_str::<serde_json::Value>(json).ok())
            .filter(|frame| frame["type"] == "Error" || frame["type"] == "Finish")
            .count();
        assert_eq!(
            terminals, 1,
            "expected exactly one terminal frame in: {text}"
        );
    }

    /// **[F]** Frame ORDER under coalescing: the `/reply` consumer (which merges
    /// same-id text deltas) and an observer (which does not) must agree on the
    /// ORDER of frame types, and the coalescer's flush MUST land before the
    /// terminal frame. Step 4 used to say "pay particular attention to order";
    /// this asserts it.
    ///
    /// **Driving this through a real turn cannot test it, which is why this test
    /// does not.** A provider-less turn produces exactly ONE frame: `Agent::reply`
    /// reaches `check_if_compaction_needed(self.provider().await?…)` and
    /// `provider()` is `Err(anyhow!("Provider not set"))`, so the runner
    /// publishes one `TurnError` and returns. And `BIOROUTER_SSE_COALESCE_MS` is
    /// unset in tests, so `sse_coalesce_window()` returns `Duration::ZERO` and
    /// `DeltaCoalescer::enabled()` is false and the flush placement is never
    /// executed. An end-to-end version of this test compares two one-element
    /// vectors derived from the same bus event and certifies nothing.
    ///
    /// `DeltaCoalescer` is private but in-file, and these tests live in
    /// `reply.rs`'s test module, so it is reachable.
    #[tokio::test]
    async fn coalesced_deltas_flush_before_the_terminal_frame() {
        use biorouter::agents::AgentEvent;
        use biorouter::conversation::message::Message;
        use biorouter::session_events::SessionBusEvent;

        let delta = |id: &str, text: &str| Message::assistant().with_id(id).with_text(text);
        let events = vec![
            SessionBusEvent::TurnStarted {
                turn_id: "t".into(),
            },
            SessionBusEvent::Agent(AgentEvent::Message(delta("a", "he"))),
            SessionBusEvent::Agent(AgentEvent::Message(delta("a", "llo"))),
            SessionBusEvent::TurnFinished {
                reason: "stop".into(),
                token_state: None,
            },
        ];

        // Observer: every event through `map_bus_event`, no coalescing.
        let mut ts = TokenState::default();
        let observer: Vec<String> = events
            .iter()
            .cloned()
            .filter_map(|e| crate::routes::session_events::map_bus_event(e, &mut ts))
            .filter_map(|f| {
                serde_json::to_value(&f).unwrap()["type"]
                    .as_str()
                    .map(str::to_string)
            })
            .collect();
        assert_eq!(
            observer,
            vec!["TurnStarted", "Message", "Message", "Finish"]
        );

        // /reply: a 50 ms window merges the two same-id deltas into ONE frame,
        // which must still precede the terminal frame.
        let mut coalescer = DeltaCoalescer::new(Duration::from_millis(50));
        let mut ts = TokenState::default();
        let mut client: Vec<String> = Vec::new();
        for event in events {
            match event {
                SessionBusEvent::Agent(AgentEvent::Message(m)) => {
                    for _ in coalescer.push(m) {
                        client.push("Message".to_string());
                    }
                }
                other => {
                    if coalescer.drain().is_some() {
                        client.push("Message".to_string());
                    }
                    if let Some(f) = crate::routes::session_events::map_bus_event(other, &mut ts) {
                        if let Some(kind) = serde_json::to_value(&f).unwrap()["type"].as_str() {
                            client.push(kind.to_string());
                        }
                    }
                }
            }
        }
        assert_eq!(
            client,
            vec!["TurnStarted", "Message", "Finish"],
            "the coalescer must flush before the terminal frame"
        );
    }

    /// **[F]** The turn's log must be ENDED when the runner dies without
    /// publishing a terminal event, or every attached client hangs forever after
    /// one error frame.
    ///
    /// Before the BR-71 refactor the turn task owned `task_tx`; when it returned
    /// — including through a panic unwind — the sender dropped and the body
    /// ended. In the subscription shape the pump only breaks on a terminal bus
    /// event, `RecvError::Closed`, or its cancel token. A panicking runner
    /// publishes no terminal event; `Closed` cannot be relied on either, because
    /// this consumer's own `Receiver` holds the channel open and
    /// `session_events::release_if_idle` only reclaims a sender once
    /// `receiver_count() == 0` — which by construction it is not, here; and
    /// `TurnGuard::drop` retires the `ActiveTurn` entry **without** tripping the
    /// token (`state.rs`). The supervisor is therefore the only thing that can
    /// release the pump.
    #[tokio::test]
    async fn the_supervisor_ends_the_stream_even_when_the_runner_panics() {
        let stream = TurnStream::new("sup-panic", "turn-1");

        // A runner that panics, with a pump that would never end on its own.
        let cancel = CancellationToken::new();
        let runner = tokio::spawn(async { panic!("runner exploded") });
        let pump = tokio::spawn({
            let cancel = cancel.clone();
            async move { cancel.cancelled().await }
        });
        supervise_turn(runner, pump, Arc::clone(&stream), cancel.clone()).await;
        let mut reader = stream.attach(0);
        let frame = match reader.recv().await {
            crate::turn_stream::ReaderEvent::Frame(frame, _) => frame.live_sse(),
            other => panic!("the supervisor must log one error frame, got {other:?}"),
        };
        assert!(
            frame.contains("\"code\":\"internal_error\""),
            "got: {frame}"
        );
        assert!(
            cancel.is_cancelled(),
            "the pump must be released, or the log never closes"
        );

        // A runner that returns cleanly while its pump has ALREADY ended on the
        // terminal frame: no error frame, and no premature cancellation that
        // could truncate the tail of a healthy turn.
        let clean = TurnStream::new("sup-clean", "turn-2");
        let cancel = CancellationToken::new();
        let runner = tokio::spawn(async {});
        let pump = tokio::spawn(async {});
        supervise_turn(runner, pump, Arc::clone(&clean), cancel.clone()).await;
        assert_eq!(
            clean.next_seq(),
            0,
            "a clean runner exit logs no error frame"
        );
        assert!(
            !cancel.is_cancelled(),
            "a stream that ended on its own must not be cancelled behind its back"
        );
    }

    /// A PUMP that panicked is not a clean exit either — and the supervisor's
    /// condition could not tell the difference.
    ///
    /// `timeout(grace, pump).await.is_err()` is true only for `Elapsed`; a
    /// panicked task completes as `Ok(Err(JoinError))`, so the supervisor read
    /// it as "the pump finished, nothing to release" and never tripped the
    /// token. Nothing else stops a turn whose pump is gone: the reaper's token
    /// has no other consumer, and the runner keeps spending.
    ///
    /// The existing panic test above covers a panicking RUNNER with a
    /// well-behaved pump, so it exercised the shape without reaching this
    /// branch. This asserts the outcome, not the count: the token is tripped.
    #[tokio::test]
    async fn the_supervisor_releases_the_turn_when_its_pump_panics() {
        let stream = TurnStream::new("sup-pump-panic", "turn-1");
        let cancel = CancellationToken::new();
        let runner = tokio::spawn(async {});
        let pump = tokio::spawn(async { panic!("pump exploded") });
        supervise_turn(runner, pump, Arc::clone(&stream), cancel.clone()).await;
        assert!(
            cancel.is_cancelled(),
            "a pump that panicked left the turn running with nothing consuming its \
             events and nothing able to stop it"
        );
    }

    /// **[F]** The coalescer must not swallow the last text delta when the
    /// terminal frame lands in the same window. `BIOROUTER_SSE_COALESCE_MS` is
    /// off by default, so this is the configuration most likely to be
    /// under-tested — and the plan itself names the flush placement as the
    /// likely cause of any order failure, which makes it the thing to pin.
    #[tokio::test]
    async fn a_terminal_frame_flushes_pending_coalesced_text_first() {
        use biorouter::conversation::message::Message;
        let stream = TurnStream::new("flush-order", "turn-1");
        let mut coalescer = DeltaCoalescer::new(Duration::from_millis(50));
        assert!(coalescer
            .push(Message::assistant().with_id("a").with_text("hel"))
            .is_empty());
        assert!(coalescer
            .push(Message::assistant().with_id("a").with_text("lo"))
            .is_empty());

        // Exactly what the new terminal branch does, in the order it does it.
        flush_coalesced(&mut coalescer, &stream, &TokenState::default());
        stream_event(
            MessageEvent::Finish {
                turn_id: None,
                reason: "stop".to_string(),
                token_state: TokenState::default(),
            },
            &stream,
        );
        stream.close();

        let logged = collect_sse(&stream).await;
        assert_eq!(logged.len(), 2, "got: {logged:?}");
        assert!(
            logged[0].contains("\"type\":\"Message\"") && logged[0].contains("hello"),
            "the buffered run is flushed first: {:?}",
            logged[0]
        );
        assert!(
            logged[1].contains("\"type\":\"Finish\""),
            "then the terminal frame: {:?}",
            logged[1]
        );
    }

    /// Every frame a stream logged, as SSE text, in order. Used by the tests
    /// that drive the pump: the log is what the pump writes, and what every
    /// present and future observer of the turn reads.
    async fn collect_sse(stream: &Arc<TurnStream>) -> Vec<String> {
        let mut reader = stream.attach(0);
        let mut out = Vec::new();
        loop {
            match reader.recv().await {
                crate::turn_stream::ReaderEvent::Frame(frame, _) => out.push(frame.live_sse()),
                crate::turn_stream::ReaderEvent::Gap => {}
                crate::turn_stream::ReaderEvent::Closed => return out,
            }
        }
    }

    // The four `[M]` tests below drive the REAL pump — `pump_bus_into_stream`,
    // the function `/reply` spawns — over the REAL session bus, and each one
    // fails if a specific branch of that loop is deleted or moved.
    //
    // They exist because a review found the eight tests above collectively green
    // against a broken hot path: `a_lagged_consumer_gets_a_storage_resync_frame`
    // overflows its own receiver and calls `bus_lag_resync_frame` itself,
    // `coalesced_deltas_flush_before_the_terminal_frame` and
    // `a_terminal_frame_flushes_pending_coalesced_text_first` perform the flush
    // themselves, and `the_supervisor_ends_the_stream_even_when_the_runner_panics`
    // hands `supervise_turn` a stand-in pump task. Every one of them describes
    // the loop rather than running it, so deleting the loop's resync branch, its
    // flush, or its `cancel` response leaves all three green.
    //
    // Verified by mutation: narrowing the flush guard to `if terminal`, widening
    // it to `if true`, deleting the `Lagged` resync, dropping `if terminal
    // { break }`, removing the supervisor's `cancel.cancel()`, giving
    // `map_bus_event` a `MessagesPersisted => None`, and forwarding only terminal
    // frames each turn at least one test below red.

    /// Parse one SSE line into its frame, dropping heartbeats — they are timing
    /// noise, not part of any ordering contract.
    fn sse_frame(raw: &str) -> Option<serde_json::Value> {
        let value: serde_json::Value = serde_json::from_str(raw.strip_prefix("data: ")?.trim_end())
            .expect("the loop must only ever write valid JSON frames");
        (value["type"] != "Ping").then_some(value)
    }

    /// Every frame the pump logged, in order, once its task has ended.
    ///
    /// The timeout on the join is what turns "a mutation left the loop running
    /// forever" into a failed assertion instead of a hung test binary. Reading
    /// the LOG rather than a socket is the point: the log is what survives an
    /// observer leaving, so a test written against it also tests the fix.
    async fn drain_pump(
        pump: tokio::task::JoinHandle<()>,
        stream: &Arc<TurnStream>,
    ) -> Vec<serde_json::Value> {
        tokio::time::timeout(Duration::from_secs(10), pump)
            .await
            .expect("the terminal frame must end the pump")
            .unwrap();
        collect_sse(stream)
            .await
            .iter()
            .filter_map(|raw| sse_frame(raw))
            .collect()
    }

    fn frame_types(frames: &[serde_json::Value]) -> Vec<String> {
        frames
            .iter()
            .map(|f| f["type"].as_str().unwrap_or("?").to_string())
            .collect()
    }

    /// **[M]** #59's ordering invariant, through the real loop with coalescing
    /// ON: a buffered run of text deltas is flushed BEFORE the
    /// `MessagesPersisted` frame that names the id those deltas belong to.
    ///
    /// This is the test the `!matches!(… TokenUsage)` guard exists for. Narrow
    /// that guard to `if terminal` — the mutation the comment beside it warns
    /// against, and the one every other test in this task survives — and the
    /// buffered run is only flushed when the terminal frame arrives, so the
    /// client is told the id of a row whose body it has not been sent yet.
    ///
    /// Coalescing must be ON or nothing is buffered and the flush is a no-op,
    /// which is why the window is a parameter of `pump_bus_into_stream` rather
    /// than a read of `BIOROUTER_SSE_COALESCE_MS` (unset under test).
    #[tokio::test]
    async fn the_sse_loop_flushes_buffered_text_before_a_persisted_frame() {
        use biorouter::agents::{AgentEvent, PersistedMessage};
        use biorouter::session_events::{self, SessionBusEvent};

        let state = AppState::new().await.unwrap();
        let session_id = "br71-reply-loop-flush-order".to_string();
        let bus = session_events::subscribe(&session_id);
        let stream = TurnStream::new(&session_id, "turn-flush-order");
        let cancel = CancellationToken::new();
        // A window far longer than the test's runtime: the only thing that can
        // emit the buffered run is an explicit flush, never the deadline.
        let loop_task = tokio::spawn(pump_bus_into_stream(
            state.clone(),
            session_id.clone(),
            bus,
            stream.claim_writer().expect("the test owns this log"),
            cancel.clone(),
            Duration::from_secs(30),
        ));

        for chunk in ["he", "llo"] {
            session_events::publish(
                &session_id,
                SessionBusEvent::Agent(AgentEvent::Message(
                    Message::assistant().with_id("m-1").with_text(chunk),
                )),
            );
        }
        session_events::publish(
            &session_id,
            SessionBusEvent::Agent(AgentEvent::MessagesPersisted(vec![PersistedMessage {
                id: "m-1".into(),
                user_visible: true,
            }])),
        );
        session_events::publish(
            &session_id,
            SessionBusEvent::TurnFinished {
                reason: "stop".into(),
                token_state: None,
            },
        );

        // Bounded: the loop must END on the terminal frame. An unbounded await
        // here turns "the `if terminal { break }` was removed" into a hung test
        // binary instead of a failed assertion.
        let frames = drain_pump(loop_task, &stream).await;
        assert_eq!(
            frame_types(&frames),
            vec!["Message", "MessagesPersisted", "Finish"],
            "the buffered deltas must reach the client BEFORE the frame naming \
             their stored id: {frames:?}"
        );
        assert!(
            frames[0].to_string().contains("hello"),
            "one coalesced run, not one frame per delta: {frames:?}"
        );
    }

    /// **[M]** …and the same guard must not be widened either: a `TokenUsage`
    /// event is pure bookkeeping and must NOT end a coalescing run.
    ///
    /// The provider interleaves token accounting with text deltas continuously,
    /// so flushing on it would emit one frame per delta and silently undo BR-53a
    /// for every turn — with `coalesce_tests` still green, because they never
    /// feed the coalescer a bus event.
    #[tokio::test]
    async fn token_usage_alone_does_not_break_a_coalescing_run() {
        use biorouter::agents::AgentEvent;
        use biorouter::session_events::{self, SessionBusEvent};

        let state = AppState::new().await.unwrap();
        let session_id = "br71-reply-loop-token-usage".to_string();
        let bus = session_events::subscribe(&session_id);
        let stream = TurnStream::new(&session_id, "turn-token-usage");
        let cancel = CancellationToken::new();
        let loop_task = tokio::spawn(pump_bus_into_stream(
            state.clone(),
            session_id.clone(),
            bus,
            stream.claim_writer().expect("the test owns this log"),
            cancel.clone(),
            Duration::from_secs(30),
        ));

        session_events::publish(
            &session_id,
            SessionBusEvent::Agent(AgentEvent::Message(
                Message::assistant().with_id("m-1").with_text("he"),
            )),
        );
        session_events::publish(
            &session_id,
            SessionBusEvent::Agent(AgentEvent::TokenUsage(TokenState::default())),
        );
        session_events::publish(
            &session_id,
            SessionBusEvent::Agent(AgentEvent::Message(
                Message::assistant().with_id("m-1").with_text("llo"),
            )),
        );
        session_events::publish(
            &session_id,
            SessionBusEvent::TurnFinished {
                reason: "stop".into(),
                token_state: None,
            },
        );

        // Bounded: the loop must END on the terminal frame. An unbounded await
        // here turns "the `if terminal { break }` was removed" into a hung test
        // binary instead of a failed assertion.
        let frames = drain_pump(loop_task, &stream).await;
        assert_eq!(
            frame_types(&frames),
            vec!["Message", "Finish"],
            "token bookkeeping must not split a coalescing run: {frames:?}"
        );
        assert!(
            frames[0].to_string().contains("hello"),
            "the run must still merge across the TokenUsage: {frames:?}"
        );
    }

    /// **[M]** The `Lagged` branch of the real loop, exercised by the real
    /// broadcast channel: a consumer that has fallen off the ring is sent the
    /// whole conversation from STORAGE, not left silently short of frames.
    ///
    /// The ring is overrun before the loop's first `recv`, so `Lagged` is
    /// deterministic rather than raced. Delete the resync (or replace it with a
    /// bare `continue`) and no frame is ever written — the drain below times out
    /// and says so.
    #[tokio::test]
    async fn a_lagged_sse_loop_resyncs_the_client_from_storage() {
        use biorouter::session_events::{self, SessionBusEvent, BUS_CAPACITY};

        let state = AppState::new().await.unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let session = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "lagged-loop".to_string(),
                biorouter::session::session_manager::SessionType::User,
            )
            .await
            .unwrap();
        state
            .session_manager()
            .add_message(
                &session.id,
                &Message::user().with_text("stored before the lag"),
            )
            .await
            .unwrap();

        let bus = session_events::subscribe(&session.id);
        // Overrun the ring while nothing is reading it, so the loop's very first
        // `recv` returns `Lagged`.
        for i in 0..(BUS_CAPACITY + 1) {
            session_events::publish(
                &session.id,
                SessionBusEvent::TurnStarted {
                    turn_id: format!("turn-{i}"),
                },
            );
        }

        let stream = TurnStream::new(&session.id, "turn-lagged");
        let cancel = CancellationToken::new();
        let loop_task = tokio::spawn(pump_bus_into_stream(
            state.clone(),
            session.id.clone(),
            bus,
            stream.claim_writer().expect("the test owns this log"),
            cancel.clone(),
            Duration::ZERO,
        ));

        let mut reader = stream.attach(0);
        let resync = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match reader.recv().await {
                    crate::turn_stream::ReaderEvent::Frame(frame, _) => {
                        if let Some(value) = sse_frame(&frame.live_sse()) {
                            return Some(value);
                        }
                    }
                    crate::turn_stream::ReaderEvent::Gap => {}
                    crate::turn_stream::ReaderEvent::Closed => return None,
                }
            }
        })
        .await
        .expect("a lagged consumer must be resynced, not silently skipped")
        .expect("the stream closed without logging anything");

        assert_eq!(
            resync["type"], "UpdateConversation",
            "the resync is a whole-conversation frame: {resync}"
        );
        assert!(
            resync.to_string().contains("stored before the lag"),
            "…read from storage, not synthesised empty: {resync}"
        );

        cancel.cancel();
        // Bounded for the same reason as above: a loop that ignores its cancel
        // token must fail this test, not hang it. `CANCEL_DRAIN_GRACE` is what
        // it waits out — the window in which a cancelled runner's real terminal
        // frame can still arrive.
        tokio::time::timeout(Duration::from_secs(10), loop_task)
            .await
            .expect("cancellation must end the pump")
            .unwrap();
    }

    /// **[M]** The supervisor's release path against the REAL loop.
    ///
    /// `the_supervisor_ends_the_stream_even_when_the_runner_panics` hands
    /// `supervise_turn` a stand-in task that does nothing but await the token,
    /// so it proves the token is tripped and nothing about the loop. This one
    /// composes the two production functions: a runner that returns without ever
    /// publishing a terminal event leaves the real loop parked on `bus.recv()`
    /// forever, and only the supervisor's `cancel` can end it.
    ///
    /// Remove that `cancel.cancel()` and the drain below never completes.
    #[tokio::test]
    async fn the_supervisor_releases_a_real_sse_loop_that_never_got_a_terminal() {
        let state = AppState::new().await.unwrap();
        let session_id = "br71-reply-supervisor-releases-loop".to_string();
        let bus = biorouter::session_events::subscribe(&session_id);
        let stream = TurnStream::new(&session_id, "turn-supervisor");
        let cancel = CancellationToken::new();
        let pump = tokio::spawn(pump_bus_into_stream(
            state.clone(),
            session_id,
            bus,
            stream.claim_writer().expect("the test owns this log"),
            cancel.clone(),
            Duration::ZERO,
        ));

        // A runner that ends without publishing a terminal event — the shape a
        // turn leaves behind when it is aborted, or when its own supervisor
        // swallowed the panic after the bus entry was gone.
        let runner = tokio::spawn(async {});
        supervise_turn(runner, pump, Arc::clone(&stream), cancel.clone()).await;
        assert!(
            cancel.is_cancelled(),
            "the supervisor must release a loop that never saw a terminal frame"
        );

        // …and the released pump must CLOSE the log, so a client attached to it
        // (or attaching later) gets a terminal instead of an open socket.
        let ended = tokio::time::timeout(Duration::from_secs(10), async {
            while !stream.is_closed() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            collect_sse(&stream).await
        })
        .await
        .expect("the turn's log must close once the supervisor releases the pump");
        assert!(
            ended
                .last()
                .is_some_and(|frame| frame.contains("\"type\":\"Error\"")),
            "a pump released without a terminal must still leave one behind: {ended:?}"
        );
    }

    /// #51 W5: `conversation_so_far` overwriting a live session.
    ///
    /// Driven against an ISOLATED `SessionManager` over a temp dir rather than
    /// the process-global store, so these can seed real sessions without
    /// touching the developer's own `sessions.db`.
    mod writeback_tests {
        use super::*;
        use biorouter::session::session_manager::SessionType;
        use std::path::PathBuf;
        use tempfile::TempDir;

        async fn seeded() -> (TempDir, SessionManager, String) {
            let temp_dir = TempDir::new().unwrap();
            let sm = SessionManager::new(temp_dir.path().to_path_buf());
            let id = sm
                .create_session(PathBuf::from("/tmp/wb"), "wb".into(), SessionType::User)
                .await
                .unwrap()
                .id;
            (temp_dir, sm, id)
        }

        async fn stored_texts(sm: &SessionManager, id: &str) -> Vec<String> {
            sm.get_session(id, true)
                .await
                .unwrap()
                .conversation
                .unwrap()
                .messages()
                .iter()
                .map(|m| m.as_concat_text())
                .collect()
        }

        /// THE defect. A client whose copy of the history predates a message
        /// another writer appended — `biorouter term log`, a second socket, a
        /// note tool, the CLI on the same `sessions.db` — used to have that
        /// copy stored verbatim, deleting a message whose writer had already
        /// been told the append succeeded. The `/reply` turn lock does not
        /// cover any of those writers.
        #[tokio::test]
        async fn a_stale_client_copy_cannot_delete_an_acknowledged_message() {
            let (_temp, sm, id) = seeded().await;
            sm.add_message(&id, &Message::user().with_text("one"))
                .await
                .unwrap();
            let client = sm
                .get_session(&id, true)
                .await
                .unwrap()
                .conversation
                .unwrap();

            // ...and only now does somebody else append.
            sm.add_message(&id, &Message::user().with_text("NOTE from elsewhere"))
                .await
                .unwrap();

            let conflict = apply_client_writeback(&sm, &id, client.messages().to_vec())
                .await
                .expect_err("a stale client copy must be refused");

            assert_eq!(conflict.stored_message_count, 2);
            assert_eq!(conflict.missing.len(), 1);
            assert_eq!(
                stored_texts(&sm, &id).await,
                vec!["one".to_string(), "NOTE from elsewhere".to_string()],
                "a refused write-back must leave the store untouched"
            );
        }

        /// The compatible path: a client that is up to date still gets its copy
        /// stored, so an API client using the field keeps working.
        #[tokio::test]
        async fn an_up_to_date_client_copy_is_still_stored() {
            let (_temp, sm, id) = seeded().await;
            sm.add_message(&id, &Message::user().with_text("one"))
                .await
                .unwrap();
            let mut client = sm
                .get_session(&id, true)
                .await
                .unwrap()
                .conversation
                .unwrap()
                .messages()
                .to_vec();
            client.push(Message::assistant().with_text("client-side answer"));

            let stored = apply_client_writeback(&sm, &id, client).await.unwrap();

            assert_eq!(
                stored
                    .messages()
                    .iter()
                    .map(|m| m.as_concat_text())
                    .collect::<Vec<_>>(),
                vec!["one".to_string(), "client-side answer".to_string()]
            );
            assert_eq!(
                stored_texts(&sm, &id).await,
                vec!["one".to_string(), "client-side answer".to_string()]
            );
        }

        /// A client that deliberately drops a stored message is refused too:
        /// from the server there is no way to tell "I edited this away" apart
        /// from "I never saw it". Message edits have their own endpoint
        /// (`POST /sessions/{id}/edit_message`), which is where that intent
        /// belongs.
        #[tokio::test]
        async fn a_client_copy_that_drops_a_stored_message_is_refused() {
            let (_temp, sm, id) = seeded().await;
            sm.add_message(&id, &Message::user().with_text("one"))
                .await
                .unwrap();
            sm.add_message(&id, &Message::user().with_text("two"))
                .await
                .unwrap();
            let full = sm
                .get_session(&id, true)
                .await
                .unwrap()
                .conversation
                .unwrap();
            let trimmed = vec![full.messages()[0].clone()];

            apply_client_writeback(&sm, &id, trimmed)
                .await
                .expect_err("dropping a stored message must be refused");
            assert_eq!(
                stored_texts(&sm, &id).await,
                vec!["one".to_string(), "two".to_string()]
            );
        }

        /// An empty client copy would wipe the session outright.
        #[tokio::test]
        async fn an_empty_client_copy_cannot_wipe_a_session() {
            let (_temp, sm, id) = seeded().await;
            sm.add_message(&id, &Message::user().with_text("one"))
                .await
                .unwrap();

            apply_client_writeback(&sm, &id, Vec::new())
                .await
                .expect_err("an empty copy must be refused");
            assert_eq!(stored_texts(&sm, &id).await, vec!["one".to_string()]);
        }

        /// A session that does not exist has no history to protect, and the
        /// turn task reports the missing session itself — so the write-back is
        /// a silent no-op rather than a 409 the client cannot act on.
        #[tokio::test]
        async fn an_unknown_session_is_not_reported_as_a_conflict() {
            let (_temp, sm, _id) = seeded().await;
            let conv = apply_client_writeback(
                &sm,
                "no-such-session",
                vec![Message::user().with_text("hi")],
            )
            .await
            .expect("a missing session is the turn task's error to report");
            assert_eq!(conv.messages().len(), 1);
        }

        /// The refusal has to be actionable: a status the client can branch on
        /// and the exact ids its copy would have destroyed.
        #[tokio::test]
        async fn the_conflict_response_names_what_would_have_been_destroyed() {
            let conflict = WritebackConflict {
                missing: vec!["msg-a".into(), "msg-b".into()],
                stored_message_count: 7,
            };
            let response = writeback_conflict_response(&conflict);
            assert_eq!(response.status(), StatusCode::CONFLICT);

            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body["code"], "conversation_out_of_date");
            assert_eq!(
                body["missing_message_ids"],
                serde_json::json!(["msg-a", "msg-b"])
            );
            assert_eq!(body["stored_message_count"], serde_json::json!(7));
        }
    }

    /// BR-53a: the pure state machine that batches streamed text deltas.
    mod coalesce_tests {
        use super::*;
        use biorouter::conversation::message::Message;

        fn delta(id: &str, text: &str) -> Message {
            Message::assistant().with_id(id).with_text(text)
        }

        #[test]
        fn disabled_window_passes_each_delta_straight_through() {
            let mut c = DeltaCoalescer::new(Duration::ZERO);
            assert!(!c.enabled());
            let out = c.push(delta("a", "hello "));
            assert_eq!(out.len(), 1);
            assert_eq!(out[0].as_concat_text(), "hello ");
            // Nothing is ever buffered when disabled.
            assert!(c.deadline().is_none());
            assert!(c.drain().is_none());
        }

        #[test]
        fn same_id_text_deltas_merge_until_drained() {
            let mut c = DeltaCoalescer::new(Duration::from_millis(50));
            assert!(c.push(delta("a", "he")).is_empty());
            assert!(c.push(delta("a", "ll")).is_empty());
            assert!(c.push(delta("a", "o")).is_empty());
            assert!(c.deadline().is_some());

            let flushed = c.drain().expect("a buffered run");
            assert_eq!(flushed.as_concat_text(), "hello");
            assert_eq!(flushed.id.as_deref(), Some("a"));
            assert!(c.deadline().is_none());
        }

        #[test]
        fn a_new_message_id_flushes_the_previous_run() {
            let mut c = DeltaCoalescer::new(Duration::from_millis(50));
            assert!(c.push(delta("a", "aaa")).is_empty());

            let out = c.push(delta("b", "bbb"));
            assert_eq!(out.len(), 1);
            assert_eq!(out[0].as_concat_text(), "aaa");
            assert_eq!(out[0].id.as_deref(), Some("a"));

            // The new id is now buffered.
            let buffered = c.drain().expect("second run");
            assert_eq!(buffered.as_concat_text(), "bbb");
            assert_eq!(buffered.id.as_deref(), Some("b"));
        }

        #[test]
        fn non_text_message_flushes_run_then_passes_through_in_order() {
            let mut c = DeltaCoalescer::new(Duration::from_millis(50));
            assert!(c.push(delta("a", "partial ")).is_empty());

            // Two text contents => not a single-text delta => not coalescable.
            let multi = Message::assistant()
                .with_id("m")
                .with_text("x")
                .with_text("y");
            let out = c.push(multi);
            assert_eq!(out.len(), 2);
            assert_eq!(out[0].as_concat_text(), "partial "); // flushed run first
            assert_eq!(out[0].id.as_deref(), Some("a"));
            assert_eq!(out[1].id.as_deref(), Some("m")); // then the pass-through
            assert!(c.deadline().is_none());
            assert!(c.drain().is_none());
        }

        #[test]
        fn id_less_message_is_never_coalesced() {
            let mut c = DeltaCoalescer::new(Duration::from_millis(50));
            let out = c.push(Message::assistant().with_text("no id"));
            assert_eq!(out.len(), 1);
            assert_eq!(out[0].as_concat_text(), "no id");
            assert!(c.deadline().is_none());
        }

        #[test]
        fn coalesced_concatenation_is_byte_exact() {
            // No spacing or re-sanitising is introduced by merging.
            let mut c = DeltaCoalescer::new(Duration::from_millis(50));
            for chunk in ["The ", "quick", " brown\n", "fox"] {
                assert!(c.push(delta("x", chunk)).is_empty());
            }
            let merged = c.drain().unwrap();
            assert_eq!(merged.as_concat_text(), "The quick brown\nfox");
        }
    }

    mod integration_tests {
        use super::*;
        use axum::{body::Body, http::Request};
        use biorouter::conversation::message::Message;
        use tower::ServiceExt;

        /// Issue #56 Task 58 / #47. Why every `/reply` request below now carries
        /// a proof-of-user header.
        ///
        /// `/reply` resolves the named session's tier before it does anything
        /// else, and a session the store has never heard of is answered exactly
        /// as a private one is — that indistinguishability is the whole point of
        /// the gate (`routes::session_reach`), because a refusal that told an
        /// unproven caller "no such chat" would enumerate the machine's private
        /// chats one id at a time.
        ///
        /// Every test in this module names a session that was never created
        /// (`test-session`, `busy-session`, `retry-session`, …), so each one is
        /// an `Unreadable` target and each one is refused without the header.
        /// Carrying it puts them back to asserting exactly what they were
        /// written to assert — the BR-33 turn lock and BR-62's idempotency
        /// semantics — rather than the barrier, which has its own tests. This is
        /// the same key `routes::session::diverge_tests` installs, deliberately:
        /// the digest lives in a process-global `OnceLock`, so a second key here
        /// would turn whichever module ran second into a wall of 403s.
        use crate::routes::session::diverge_tests::{
            install_test_user_action_key, TEST_USER_ACTION_KEY,
        };

        /// Begin a turn with a throwaway token and no idempotency key.
        fn begin_turn(
            state: &AppState,
            session_id: &str,
        ) -> Result<crate::state::TurnGuard, crate::state::TurnConflict> {
            state.try_begin_turn_idempotent(session_id, CancellationToken::new(), None)
        }

        #[tokio::test]
        async fn parent_closing_continuation_conflict_has_a_stable_typed_code() {
            let response =
                continuation_lease_failure_response(ContinuationLeaseFailure::ParentClosing);
            assert_eq!(response.status(), StatusCode::CONFLICT);
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let body: ContinuationLeaseErrorResponse = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body.error, "parent_turn_closing");
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn test_reply_endpoint() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();

            let app = routes(state);

            let request = Request::builder()
                .uri("/reply")
                .method("POST")
                .header("content-type", "application/json")
                .header("x-secret-key", "test-secret")
                .header("X-User-Action", TEST_USER_ACTION_KEY)
                .body(Body::from(
                    serde_json::to_string(&ChatRequest {
                        user_message: Message::user().with_text("test message"),
                        conversation_so_far: None,
                        session_id: "test-session".to_string(),
                        workflow_name: None,
                        workflow_version: None,
                        reasoning_effort: None,
                        turn_id: None,
                        continuation_lease: None,
                        from_seq: None,
                    })
                    .unwrap(),
                ))
                .unwrap();

            let response = app.oneshot(request).await.unwrap();

            assert_eq!(response.status(), StatusCode::OK);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn test_reply_rejects_concurrent_turn() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();

            // Simulate a turn already in flight for this session. The guard owns
            // its own Arc into the shared active-turns map, so it stays valid
            // after `state` is moved into `routes`.
            let _guard = begin_turn(&state, "busy-session").expect("first turn acquires the lock");

            let app = routes(state);

            let request = Request::builder()
                .uri("/reply")
                .method("POST")
                .header("content-type", "application/json")
                .header("x-secret-key", "test-secret")
                .header("X-User-Action", TEST_USER_ACTION_KEY)
                .body(Body::from(
                    serde_json::to_string(&ChatRequest {
                        user_message: Message::user().with_text("second message"),
                        conversation_so_far: None,
                        session_id: "busy-session".to_string(),
                        workflow_name: None,
                        workflow_version: None,
                        reasoning_effort: None,
                        turn_id: None,
                        continuation_lease: None,
                        from_seq: None,
                    })
                    .unwrap(),
                ))
                .unwrap();

            let response = app.oneshot(request).await.unwrap();

            assert_eq!(response.status(), StatusCode::CONFLICT);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn test_reply_allows_new_turn_after_guard_dropped() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();

            // A turn that has ended (guard dropped) must not block the next one.
            {
                let _guard = begin_turn(&state, "recycled-session").unwrap();
            }

            let app = routes(state);

            let request = Request::builder()
                .uri("/reply")
                .method("POST")
                .header("content-type", "application/json")
                .header("x-secret-key", "test-secret")
                .header("X-User-Action", TEST_USER_ACTION_KEY)
                .body(Body::from(
                    serde_json::to_string(&ChatRequest {
                        user_message: Message::user().with_text("fresh message"),
                        conversation_so_far: None,
                        session_id: "recycled-session".to_string(),
                        workflow_name: None,
                        workflow_version: None,
                        reasoning_effort: None,
                        turn_id: None,
                        continuation_lease: None,
                        from_seq: None,
                    })
                    .unwrap(),
                ))
                .unwrap();

            let response = app.oneshot(request).await.unwrap();

            assert_eq!(response.status(), StatusCode::OK);
        }

        fn interrupt_request(session_id: &str, text: &str) -> Request<Body> {
            interrupt_request_with_turn(session_id, text, None)
        }

        fn interrupt_request_with_turn(
            session_id: &str,
            text: &str,
            turn_id: Option<&str>,
        ) -> Request<Body> {
            Request::builder()
                .uri("/interrupt")
                .method("POST")
                .header("content-type", "application/json")
                .header("x-secret-key", "test-secret")
                .header("X-User-Action", TEST_USER_ACTION_KEY)
                .body(Body::from(
                    serde_json::to_string(&InterruptRequest {
                        session_id: session_id.to_string(),
                        text: text.to_string(),
                        turn_id: turn_id.map(str::to_string),
                    })
                    .unwrap(),
                ))
                .unwrap()
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn interrupt_without_a_stored_row_drains_exact_user_direct_provenance() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let _guard = begin_turn(&state, "steering-session").expect("turn lock acquired");
            assert!(
                state
                    .session_manager()
                    .get_session("steering-session", false)
                    .await
                    .is_err(),
                "this regression test requires a live turn with no durable session row"
            );
            // #69: acceptance is the *agent loop's* to give, so the session's
            // agent has to be in the accepting state a running loop puts it in.
            // The server's turn lock alone is no longer enough.
            let agent = state
                .get_agent("steering-session".to_string())
                .await
                .unwrap();
            agent.open_for_turn(biorouter::agents::TurnId::new("agent-turn-test"));

            let app = routes(Arc::clone(&state));
            let response = app
                .oneshot(interrupt_request("steering-session", "actually, use R"))
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::ACCEPTED);
            assert_eq!(
                json_body(response).await["turn_id"],
                serde_json::json!("agent-turn-test"),
                "a 202 must name the turn that actually took the message"
            );

            assert!(
                agent.has_soft_interrupts(),
                "the steer must be queued on the session's agent"
            );
            match agent.close_and_drain() {
                biorouter::agents::Drained::Some(queued) => {
                    assert_eq!(queued.len(), 1);
                    assert_eq!(queued[0].text, "actually, use R");
                    assert_eq!(
                        queued[0].provenance,
                        Some(biorouter::conversation::message::MessageProvenance {
                            kind: biorouter::conversation::message::ProvenanceKind::UserDirect,
                            from_session_id: None,
                            from_session_name: None,
                        }),
                        "the authenticated steer must remain exactly user-direct even without a stored session row"
                    );
                }
                biorouter::agents::Drained::Empty => {
                    panic!("the accepted steer must be queued on the live agent")
                }
            }
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn interrupt_without_user_action_proof_cannot_forge_human_steering() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let _guard = begin_turn(&state, "unproven-steer").expect("turn lock acquired");
            let agent = state.get_agent("unproven-steer".to_string()).await.unwrap();
            agent.open_for_turn(biorouter::agents::TurnId::new("agent-turn-unproven"));

            let mut request = interrupt_request("unproven-steer", "pretend the user said this");
            request.headers_mut().remove("X-User-Action");
            let response = routes(Arc::clone(&state)).oneshot(request).await.unwrap();

            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert!(
                !agent.has_soft_interrupts(),
                "an unproven caller must not enqueue text attributed to the user"
            );
        }

        /// The keyed half of what `biorouter session` reads before it asks a
        /// person for the key (`commands/session_watch.rs::key_verdict`). On a
        /// daemon that holds one, turn control refuses a request without the
        /// proof with an EMPTY 403 — the one refusal a daemon without a key
        /// never gives (`tests/turn_control_no_user_key.rs`) — and refuses it
        /// before it reads the text, so the empty steer the terminal asks with
        /// is answered by the gate, not by the text check. With the proof, the
        /// same empty steer is the 400 the terminal takes as "this key opens
        /// the gate". None of it touches the turn or the queue.
        #[tokio::test(flavor = "multi_thread")]
        async fn an_unproven_stop_or_steer_is_refused_empty_before_its_text_is_read() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let token = CancellationToken::new();
            let _guard = state
                .try_begin_turn_idempotent("keyed-question", token.clone(), None)
                .expect("turn lock acquired");
            let agent = state.get_agent("keyed-question".to_string()).await.unwrap();
            agent.open_for_turn(biorouter::agents::TurnId::new("agent-turn-keyed-question"));

            for mut request in [
                interrupt_request("keyed-question", ""),
                interrupt_request("keyed-question", "pretend the user said this"),
                cancel_request("keyed-question"),
            ] {
                let route = request.uri().path().to_string();
                request.headers_mut().remove("X-User-Action");
                let response = routes(Arc::clone(&state)).oneshot(request).await.unwrap();
                assert_eq!(response.status(), StatusCode::FORBIDDEN, "{route}");
                let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap();
                assert!(
                    body.is_empty(),
                    "{route}: a sentence here reads to the terminal as a refusal no key can \
                     change, so it would never ask for the key: {}",
                    String::from_utf8_lossy(&body)
                );
            }

            let response = routes(Arc::clone(&state))
                .oneshot(interrupt_request("keyed-question", ""))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert!(!token.is_cancelled(), "a refused Stop reached the turn");
            assert!(
                !agent.has_soft_interrupts(),
                "the terminal's question reached the agent's queue"
            );
        }

        /// #69: the turn lock is still held — the reply task has not unwound yet —
        /// but the loop has performed its final drain and committed to exiting.
        /// The old route read only the lock and returned 202, and the text then
        /// surfaced in an unrelated later turn.
        #[tokio::test(flavor = "multi_thread")]
        async fn test_interrupt_refuses_once_the_agent_loop_has_closed() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let _guard = begin_turn(&state, "closing-session").expect("turn lock acquired");
            let agent = state
                .get_agent("closing-session".to_string())
                .await
                .unwrap();
            agent.open_for_turn(biorouter::agents::TurnId::new("agent-turn-closing"));
            // What the loop does at its exit when nothing is queued.
            assert!(matches!(
                agent.close_and_drain(),
                biorouter::agents::Drained::Empty
            ));

            let app = routes(Arc::clone(&state));
            let response = app
                .oneshot(interrupt_request("closing-session", "too late"))
                .await
                .unwrap();

            assert_eq!(
                response.status(),
                StatusCode::CONFLICT,
                "a steer that misses its turn must be refused, not deferred"
            );
            assert!(
                !agent.has_soft_interrupts(),
                "the refused steer must not be sitting on the agent for a later turn"
            );
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn test_interrupt_rejects_when_no_turn_in_flight() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();

            let app = routes(state);
            let response = app
                .oneshot(interrupt_request("idle-session", "steer me"))
                .await
                .unwrap();

            // Nothing to steer: the client should send this as a normal message
            // rather than let it sit on the agent's queue.
            assert_eq!(response.status(), StatusCode::CONFLICT);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn test_interrupt_rejects_empty_text() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let _guard = begin_turn(&state, "blank-session").unwrap();

            let app = routes(state);
            let response = app
                .oneshot(interrupt_request("blank-session", "   "))
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }

        fn cancel_request(session_id: &str) -> Request<Body> {
            cancel_request_for_turn(session_id, None, false)
        }

        fn cancel_request_for_turn(
            session_id: &str,
            expected_turn_id: Option<&str>,
            wait_for_idle: bool,
        ) -> Request<Body> {
            cancel_request_for_turn_with_continuation(
                session_id,
                expected_turn_id,
                wait_for_idle,
                false,
            )
        }

        fn cancel_request_for_turn_with_continuation(
            session_id: &str,
            expected_turn_id: Option<&str>,
            wait_for_idle: bool,
            continuation_pending: bool,
        ) -> Request<Body> {
            Request::builder()
                .uri("/agent/cancel")
                .method("POST")
                .header("content-type", "application/json")
                .header("x-secret-key", "test-secret")
                .header("X-User-Action", TEST_USER_ACTION_KEY)
                .body(Body::from(
                    serde_json::to_string(&CancelTurnRequest {
                        session_id: session_id.to_string(),
                        expected_turn_id: expected_turn_id.map(str::to_string),
                        wait_for_idle,
                        continuation_pending,
                        continuation_owner_id: continuation_pending
                            .then(|| "test-window-owner".to_string()),
                    })
                    .unwrap(),
                ))
                .unwrap()
        }

        async fn json_body(response: axum::response::Response) -> serde_json::Value {
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            serde_json::from_slice(&bytes).unwrap()
        }

        fn reply_request(session_id: &str, turn_id: Option<&str>) -> Request<Body> {
            Request::builder()
                .uri("/reply")
                .method("POST")
                .header("content-type", "application/json")
                .header("x-secret-key", "test-secret")
                .header("X-User-Action", TEST_USER_ACTION_KEY)
                .body(Body::from(
                    serde_json::to_string(&ChatRequest {
                        user_message: Message::user().with_text("hello"),
                        conversation_so_far: None,
                        session_id: session_id.to_string(),
                        workflow_name: None,
                        workflow_version: None,
                        turn_id: turn_id.map(str::to_string),
                        continuation_lease: None,
                        reasoning_effort: None,
                        from_seq: None,
                    })
                    .unwrap(),
                ))
                .unwrap()
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn unproven_reply_cannot_impersonate_a_human_in_a_subagent_tab() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let temp = tempfile::TempDir::new().unwrap();
            let session = state
                .session_manager()
                .create_session(
                    temp.path().to_path_buf(),
                    "public-child".into(),
                    biorouter::session::session_manager::SessionType::SubAgent,
                )
                .await
                .unwrap();
            let mut request = reply_request(&session.id, None);
            request.headers_mut().remove("X-User-Action");

            let response = routes(Arc::clone(&state)).oneshot(request).await.unwrap();

            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert!(!state.is_turn_active(&session.id));
            let stored = state
                .session_manager()
                .get_session(&session.id, true)
                .await
                .unwrap();
            assert!(
                stored
                    .conversation
                    .as_ref()
                    .is_none_or(|conversation| conversation.messages().is_empty()),
                "the refused request must not persist an attributed user message"
            );
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn interrupt_queues_an_idempotent_steer_before_a_child_turn_exists() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let child = "queued-child-interrupt";
            let handle =
                biorouter::agents::subagent_handle::BackgroundSubagent::register_initializing(
                    "queued-parent-interrupt",
                    child,
                    "delegated work",
                    CancellationToken::new(),
                );

            for _ in 0..2 {
                let response = routes(Arc::clone(&state))
                    .oneshot(interrupt_request_with_turn(
                        child,
                        "change the delegated plan",
                        Some("queued-steer-1"),
                    ))
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::ACCEPTED);
                assert_eq!(
                    json_body(response).await["turn_id"],
                    serde_json::json!("queued-steer-1")
                );
            }

            assert!(!state.is_turn_active(child));
            let pending = biorouter::agents::subagent_handle::mark_initial_runtime_ready(child);
            assert_eq!(
                pending.len(),
                1,
                "the idempotency key must deduplicate retries"
            );
            assert_eq!(pending[0].as_concat_text(), "change the delegated plan");
            assert_eq!(
                pending[0].metadata.provenance.as_ref().unwrap().kind,
                biorouter::conversation::message::ProvenanceKind::UserDirect
            );
            handle.complete(
                biorouter::agents::subagent_result::SubagentResult::from_error("test cleanup"),
            );
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn reply_to_a_queued_child_is_buffered_without_claiming_its_initial_turn() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let temp = tempfile::TempDir::new().unwrap();
            let session = state
                .session_manager()
                .create_session(
                    temp.path().to_path_buf(),
                    "queued-child".into(),
                    biorouter::session::session_manager::SessionType::SubAgent,
                )
                .await
                .unwrap();
            let handle =
                biorouter::agents::subagent_handle::BackgroundSubagent::register_initializing(
                    "queued-parent",
                    session.id.clone(),
                    "delegated work",
                    CancellationToken::new(),
                );

            let response = routes(Arc::clone(&state))
                .oneshot(reply_request(&session.id, Some("queued-user-turn")))
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::ACCEPTED);
            let retry = routes(Arc::clone(&state))
                .oneshot(reply_request(&session.id, Some("queued-user-turn")))
                .await
                .unwrap();
            assert_eq!(retry.status(), StatusCode::ACCEPTED);
            assert!(!state.is_turn_active(&session.id));
            biorouter::agents::subagent_handle::begin_child_turn(&session.id);
            assert_eq!(handle.child_turn_generation(), 0);
            assert!(handle.result_is_current());

            let pending =
                biorouter::agents::subagent_handle::mark_initial_runtime_ready(&session.id);
            assert_eq!(pending.len(), 1);
            assert_eq!(pending[0].as_concat_text(), "hello");
            assert_eq!(
                pending[0].metadata.provenance.as_ref().unwrap().kind,
                biorouter::conversation::message::ProvenanceKind::UserDirect
            );
            biorouter::agents::subagent_handle::begin_child_turn(&session.id);
            assert_eq!(handle.child_turn_generation(), 1);
            assert!(handle.result_is_current());
            handle.complete(
                biorouter::agents::subagent_result::SubagentResult::from_error("test cleanup"),
            );
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn cancel_reaches_a_queued_child_that_has_no_daemon_turn() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let child = "queued-child-cancel-route";
            let handle =
                biorouter::agents::subagent_handle::BackgroundSubagent::register_initializing(
                    "queued-parent-cancel-route",
                    child,
                    "delegated work",
                    CancellationToken::new(),
                );

            let response = routes(Arc::clone(&state))
                .oneshot(cancel_request(child))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = json_body(response).await;
            assert_eq!(body["cancelled"], serde_json::json!(true));
            assert!(body["turn_id"].is_null());
            assert_eq!(body["settled"], serde_json::json!(true));
            assert!(handle.is_cancelled());
            handle.complete(
                biorouter::agents::subagent_result::SubagentResult::from_error("test cleanup"),
            );
        }

        /// BR-62: a turn can now be stopped by session id, without the caller
        /// holding the SSE socket.
        #[tokio::test(flavor = "multi_thread")]
        async fn test_cancel_trips_the_running_turn() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let token = CancellationToken::new();
            let _guard = state
                .try_begin_turn_idempotent("busy-session", token.clone(), None)
                .expect("turn lock acquired");

            let app = routes(Arc::clone(&state));
            let response = app.oneshot(cancel_request("busy-session")).await.unwrap();

            assert_eq!(response.status(), StatusCode::OK);
            let body = json_body(response).await;
            assert_eq!(body["cancelled"], serde_json::json!(true));
            assert!(body["turn_id"].is_string());
            assert!(
                token.is_cancelled(),
                "the running turn's token must be tripped"
            );
            assert_eq!(body["settled"], serde_json::json!(false));
        }

        /// Stop-and-Send must not receive its 200 until the exact cancelled
        /// guard has released the session lock. Tripping the token is only the
        /// start of cancellation, not proof that a replacement can begin.
        #[tokio::test(flavor = "multi_thread")]
        async fn cancel_waits_for_the_exact_turn_guard_to_retire() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let token = CancellationToken::new();
            let guard = state
                .try_begin_turn_idempotent("wait-cancel", token.clone(), None)
                .expect("turn lock acquired");
            let turn_id = guard.turn_id().to_string();

            let response_task =
                tokio::spawn(routes(Arc::clone(&state)).oneshot(cancel_request_for_turn(
                    "wait-cancel",
                    Some(&turn_id),
                    true,
                )));

            tokio::time::timeout(Duration::from_secs(2), async {
                while !token.is_cancelled() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("the request must trip the token before it waits");
            assert!(
                !response_task.is_finished(),
                "the route acknowledged before the guard released the lock"
            );

            drop(guard);
            let response = tokio::time::timeout(Duration::from_secs(2), response_task)
                .await
                .expect("the exact guard's drop must release the waiter")
                .unwrap()
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = json_body(response).await;
            assert_eq!(body["cancelled"], serde_json::json!(true));
            assert_eq!(body["turn_id"], serde_json::json!(turn_id));
            assert_eq!(body["settled"], serde_json::json!(true));
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn stop_and_send_marks_the_exact_child_before_cancellation_settles() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let token = CancellationToken::new();
            let guard = state
                .try_begin_turn_idempotent("continuation-child", token.clone(), None)
                .unwrap();
            let turn_id = guard.turn_id().to_string();
            let handle = biorouter::agents::subagent_handle::BackgroundSubagent::register(
                "continuation-parent",
                "continuation-child",
                "original delegated turn",
                CancellationToken::new(),
            );
            biorouter::agents::subagent_handle::begin_child_turn("continuation-child");
            handle.complete(
                biorouter::agents::subagent_result::SubagentResult::from_error(
                    "original completion",
                ),
            );

            let response_task = tokio::spawn(routes(Arc::clone(&state)).oneshot(
                cancel_request_for_turn_with_continuation(
                    "continuation-child",
                    Some(&turn_id),
                    true,
                    true,
                ),
            ));
            tokio::time::timeout(Duration::from_secs(2), async {
                while !token.is_cancelled() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();

            assert!(handle.continuation_pending());
            assert!(!handle.result_is_current());
            assert!(!response_task.is_finished());

            drop(guard);
            let response = response_task.await.unwrap().unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = json_body(response).await;
            let lease = body["continuation_lease"]
                .as_str()
                .expect("Stop-and-Send must return its exact-generation lease");
            assert!(handle.continuation_pending());

            let successor = state
                .try_begin_turn_idempotent_with_continuation(
                    "continuation-child",
                    CancellationToken::new(),
                    Some("replacement-turn".into()),
                    Some(lease),
                )
                .expect("the exact lease admits its successor");
            biorouter::agents::subagent_handle::begin_child_turn("continuation-child");
            assert!(!handle.continuation_pending());
            drop(successor);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn dropped_cancel_wait_rolls_back_the_continuation_mark() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let token = CancellationToken::new();
            let guard = state
                .try_begin_turn_idempotent("dropped-continuation-child", token.clone(), None)
                .unwrap();
            let turn_id = guard.turn_id().to_string();
            let handle = biorouter::agents::subagent_handle::BackgroundSubagent::register(
                "dropped-continuation-parent",
                "dropped-continuation-child",
                "delegated turn",
                CancellationToken::new(),
            );
            biorouter::agents::subagent_handle::begin_child_turn("dropped-continuation-child");
            handle.complete(
                biorouter::agents::subagent_result::SubagentResult::from_error("original result"),
            );

            let response_task = tokio::spawn(routes(Arc::clone(&state)).oneshot(
                cancel_request_for_turn_with_continuation(
                    "dropped-continuation-child",
                    Some(&turn_id),
                    true,
                    true,
                ),
            ));
            tokio::time::timeout(Duration::from_secs(2), async {
                while !token.is_cancelled() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(handle.continuation_pending());

            response_task.abort();
            let _ = response_task.await;
            assert!(!handle.continuation_pending());
            assert!(handle.result_is_current());
            drop(guard);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn ordinary_stop_and_generation_mismatch_never_mark_a_continuation() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let first_token = CancellationToken::new();
            let first = state
                .try_begin_turn_idempotent("ordinary-stop-child", first_token, None)
                .unwrap();
            let first_id = first.turn_id().to_string();
            let handle = biorouter::agents::subagent_handle::BackgroundSubagent::register(
                "ordinary-stop-parent",
                "ordinary-stop-child",
                "delegated turn",
                CancellationToken::new(),
            );
            biorouter::agents::subagent_handle::begin_child_turn("ordinary-stop-child");

            let response = routes(Arc::clone(&state))
                .oneshot(cancel_request_for_turn(
                    "ordinary-stop-child",
                    Some(&first_id),
                    false,
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert!(!handle.continuation_pending());
            assert!(handle.result_is_current());
            drop(first);

            let successor = state
                .try_begin_turn_idempotent("ordinary-stop-child", CancellationToken::new(), None)
                .unwrap();
            let response = routes(Arc::clone(&state))
                .oneshot(cancel_request_for_turn_with_continuation(
                    "ordinary-stop-child",
                    Some(&first_id),
                    true,
                    true,
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::CONFLICT);
            assert!(!handle.continuation_pending());
            assert!(handle.result_is_current());
            drop(successor);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn exact_retired_turn_keeps_continuation_pending_across_the_start_gap() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let retired = begin_turn(&state, "retired-continuation-child").unwrap();
            let retired_id = retired.turn_id().to_string();
            drop(retired);
            let handle = biorouter::agents::subagent_handle::BackgroundSubagent::register(
                "retired-continuation-parent",
                "retired-continuation-child",
                "delegated turn",
                CancellationToken::new(),
            );
            biorouter::agents::subagent_handle::begin_child_turn("retired-continuation-child");
            handle.complete(
                biorouter::agents::subagent_result::SubagentResult::from_error("original result"),
            );

            let response = routes(Arc::clone(&state))
                .oneshot(cancel_request_for_turn_with_continuation(
                    "retired-continuation-child",
                    Some(&retired_id),
                    true,
                    true,
                ))
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK);
            let body = json_body(response).await;
            assert_eq!(body["cancelled"], serde_json::json!(false));
            assert_eq!(body["settled"], serde_json::json!(true));
            let lease = body["continuation_lease"]
                .as_str()
                .expect("the retained exact generation must mint a lease");
            assert!(handle.continuation_pending());
            assert!(!handle.result_is_current());
            assert_eq!(
                state
                    .abandon_continuation_lease("retired-continuation-child", lease)
                    .unwrap(),
                ContinuationLeaseAbandonment::Abandoned
            );
            assert!(!handle.continuation_pending());
        }

        /// Cancelling an idle session is a success no-op, not an error — a Stop
        /// button that reports failure because the turn already finished is
        /// exactly the unreliability BR-62 removes.
        #[tokio::test(flavor = "multi_thread")]
        async fn test_cancel_is_a_noop_when_no_turn_is_running() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();

            let app = routes(state);
            let response = app.oneshot(cancel_request("idle-session")).await.unwrap();

            assert_eq!(response.status(), StatusCode::OK);
            let body = json_body(response).await;
            assert_eq!(body["cancelled"], serde_json::json!(false));
            assert_eq!(body["turn_id"], serde_json::Value::Null);
            assert_eq!(body["settled"], serde_json::json!(true));
        }

        /// Waiting is idempotent when the addressed generation already retired:
        /// there is no guard left to await and no successor to touch.
        #[tokio::test(flavor = "multi_thread")]
        async fn cancel_wait_is_immediately_settled_when_the_session_is_idle() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let response = routes(state)
                .oneshot(cancel_request_for_turn(
                    "already-idle",
                    Some("turn-that-ended"),
                    true,
                ))
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK);
            let body = json_body(response).await;
            assert_eq!(body["cancelled"], serde_json::json!(false));
            assert_eq!(body["settled"], serde_json::json!(true));
        }

        /// A delayed Stop request for turn A must never kill turn B after A has
        /// already retired. The generation check and token lookup are one atomic
        /// registry operation, so the mismatch is fail-closed.
        #[tokio::test(flavor = "multi_thread")]
        async fn stale_cancel_cannot_kill_a_successor() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let first = begin_turn(&state, "successor-session").unwrap();
            let first_turn_id = first.turn_id().to_string();
            drop(first);

            let successor_token = CancellationToken::new();
            let successor = state
                .try_begin_turn_idempotent("successor-session", successor_token.clone(), None)
                .unwrap();
            let successor_turn_id = successor.turn_id().to_string();

            let response = routes(Arc::clone(&state))
                .oneshot(cancel_request_for_turn(
                    "successor-session",
                    Some(&first_turn_id),
                    true,
                ))
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::CONFLICT);
            let body = json_body(response).await;
            assert_eq!(body["mismatch"], serde_json::json!(true));
            assert_eq!(body["expected_turn_id"], first_turn_id);
            assert_eq!(body["active_turn_id"], successor_turn_id);
            assert!(
                !successor_token.is_cancelled(),
                "a stale generation-conditional cancel tripped the successor"
            );
            assert!(state.is_turn_active("successor-session"));
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn expected_generation_mismatches_a_different_retained_generation() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let first = begin_turn(&state, "retired-mismatch-session").unwrap();
            let first_turn_id = first.turn_id().to_string();
            drop(first);
            let second = state
                .try_begin_turn_idempotent(
                    "retired-mismatch-session",
                    CancellationToken::new(),
                    Some("second-client-turn".into()),
                )
                .unwrap();
            let second_turn_id = second.turn_id().to_string();
            drop(second);

            let response = routes(Arc::clone(&state))
                .oneshot(cancel_request_for_turn_with_continuation(
                    "retired-mismatch-session",
                    Some(&first_turn_id),
                    true,
                    true,
                ))
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::CONFLICT);
            let body = json_body(response).await;
            assert_eq!(body["mismatch"], serde_json::json!(true));
            assert_eq!(body["expected_turn_id"], first_turn_id);
            assert_eq!(body["active_turn_id"], second_turn_id);

            let response = routes(Arc::clone(&state))
                .oneshot(cancel_request_for_turn(
                    "retired-mismatch-session",
                    Some(&first_turn_id),
                    true,
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::CONFLICT);
        }

        /// The safety bound is an error barrier, never an optimistic 200. This
        /// test uses the same core path with a short bound so the suite does not
        /// spend the production thirty seconds proving it exists.
        #[tokio::test(flavor = "multi_thread")]
        async fn a_cancel_wait_timeout_is_gateway_timeout_not_false_success() {
            let state = AppState::new().await.unwrap();
            let token = CancellationToken::new();
            let guard = state
                .try_begin_turn_idempotent("wedged-cancel", token.clone(), None)
                .unwrap();
            let handle = biorouter::agents::subagent_handle::BackgroundSubagent::register(
                "wedged-cancel-parent",
                "wedged-cancel",
                "delegated turn",
                CancellationToken::new(),
            );
            biorouter::agents::subagent_handle::begin_child_turn("wedged-cancel");
            handle.complete(
                biorouter::agents::subagent_result::SubagentResult::from_error("original result"),
            );
            let req = CancelTurnRequest {
                session_id: "wedged-cancel".to_string(),
                expected_turn_id: Some(guard.turn_id().to_string()),
                wait_for_idle: true,
                continuation_pending: true,
                continuation_owner_id: Some("wedged-test-owner".to_string()),
            };

            let failure = cancel_turn_bounded(&state, &req, Duration::from_millis(25))
                .await
                .expect_err("the held guard must reach the safety bound");
            assert_eq!(
                failure.into_response().status(),
                StatusCode::GATEWAY_TIMEOUT,
                "a safety timeout must be non-200 so the client cannot submit early"
            );
            assert!(token.is_cancelled(), "the cancellation still starts");
            assert!(
                state.is_turn_active("wedged-cancel"),
                "the held guard proves a 200 would have been false"
            );
            assert!(!handle.continuation_pending());
            assert!(
                handle.result_is_current(),
                "a failed cancellation barrier must restore the collectable original result"
            );
            drop(guard);
        }

        #[test]
        fn legacy_cancel_body_defaults_to_immediate_unconditional_ack() {
            let req: CancelTurnRequest =
                serde_json::from_value(serde_json::json!({"session_id": "legacy"})).unwrap();
            assert_eq!(req.session_id, "legacy");
            assert!(req.expected_turn_id.is_none());
            assert!(!req.wait_for_idle);
            assert!(!req.continuation_pending);
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn cancel_without_user_action_proof_cannot_stop_another_turn() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let token = CancellationToken::new();
            let _guard = state
                .try_begin_turn_idempotent("unproven-cancel", token.clone(), None)
                .expect("turn lock acquired");

            let mut request = cancel_request("unproven-cancel");
            request.headers_mut().remove("X-User-Action");
            let response = routes(Arc::clone(&state)).oneshot(request).await.unwrap();

            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert!(
                !token.is_cancelled(),
                "the unproven cancel reached the turn"
            );
        }

        /// A re-POST of the same turn (SSE reconnect) is ATTACHED to that turn's
        /// stream — 200, not 409 — and no second turn starts.
        #[tokio::test(flavor = "multi_thread")]
        async fn test_reply_attaches_a_reposted_turn_id() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let guard = state
                .try_begin_turn_idempotent(
                    "retry-session",
                    CancellationToken::new(),
                    Some("client-turn-1".to_string()),
                )
                .expect("turn lock acquired");

            let app = routes(Arc::clone(&state));
            let response = app
                .oneshot(reply_request("retry-session", Some("client-turn-1")))
                .await
                .unwrap();

            assert_eq!(
                response.status(),
                StatusCode::OK,
                "a reconnect must be given the turn, not an error about it"
            );
            assert_eq!(
                response.headers().get("Content-Type").unwrap(),
                "text/event-stream"
            );
            // Still exactly one turn: the lock was never re-acquired.
            assert_eq!(state.active_turn_session_ids(), vec!["retry-session"]);
            drop(guard);
        }

        /// A *different* turn arriving while one is in flight is a genuine
        /// conflict, not a retry.
        #[tokio::test(flavor = "multi_thread")]
        async fn test_reply_reports_a_different_turn_id_as_a_real_conflict() {
            install_test_user_action_key();
            let state = AppState::new().await.unwrap();
            let _guard = state
                .try_begin_turn_idempotent(
                    "contended-session",
                    CancellationToken::new(),
                    Some("client-turn-1".to_string()),
                )
                .expect("turn lock acquired");

            let app = routes(state);
            let response = app
                .oneshot(reply_request("contended-session", Some("client-turn-2")))
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::CONFLICT);
            let body = json_body(response).await;
            assert_eq!(body["duplicate"], serde_json::json!(false));
        }
    }
}

/// ADVERSARIAL probes — each written to FAIL against the current
/// implementation and to name, in its assertion, the user-visible symptom.
#[cfg(test)]
mod adversarial_output_correctness {
    use super::*;
    use biorouter::conversation::message::Message;

    fn sse_frame(raw: &str) -> Option<serde_json::Value> {
        let value: serde_json::Value =
            serde_json::from_str(raw.strip_prefix("data: ")?.trim_end()).ok()?;
        (value["type"] != "Ping").then_some(value)
    }

    fn frame_types(frames: &[serde_json::Value]) -> Vec<String> {
        frames
            .iter()
            .map(|f| f["type"].as_str().unwrap_or("?").to_string())
            .collect()
    }

    /// DEFECT 1 — an attach arriving in the window between the runner returning
    /// (`TurnGuard::drop`, which retires the entry) and the pump consuming the
    /// runner's `TurnFinished` **closes the turn's log out from under the pump**.
    ///
    /// `try_begin_turn_idempotent` reports `finished: true` the instant the
    /// guard drops, so `/reply` takes the `terminal_only` path;
    /// `drain_stream_to_client` finds no terminal frame yet and calls
    /// `stream.close()`, which synthesizes `stream_ended_without_terminal` and
    /// latches `closed`. Every frame the pump still had to publish — the real
    /// `Finish`, and on a CANCEL path up to `CANCEL_DRAIN_GRACE` (2 s) of
    /// genuine output — is then refused by `publish`.
    ///
    /// User-visible: a second window opening (or a reload landing) at the wrong
    /// millisecond turns a healthy turn's ending into
    /// "The stream for this turn ended without a result. Please retry." **in
    /// every window**, and eats whatever the turn had left to say. This is the
    /// same race `the_runners_own_terminal_reaches_the_client_not_a_synthesized_one`
    /// pins for `TurnGuard::drop` — reachable through the other door.
    #[tokio::test]
    async fn an_attach_in_the_guard_drop_window_must_not_close_a_live_pump() {
        use tower::ServiceExt;
        let state = AppState::new().await.unwrap();
        let session_id = "adv-guard-drop-window".to_string();
        let guard = state
            .try_begin_turn_idempotent(
                &session_id,
                CancellationToken::new(),
                Some("client-turn-1".into()),
            )
            .unwrap();
        let stream = guard.stream();
        let _writer = stream
            .claim_writer()
            .expect("this test writes the log itself, standing in for the pump");

        // The window that started the turn is watching.
        let (tx, mut rx) = mpsc::channel::<String>(256);
        let watcher = tokio::spawn(drain_stream_to_client(
            state.clone(),
            session_id.clone(),
            Arc::clone(&stream),
            0,
            false,
            tx,
        ));

        stream.publish(&MessageEvent::Message {
            message: Message::assistant().with_id("m-1").with_text("part one "),
            token_state: TokenState::default(),
        });

        // The runner has returned and its guard has dropped, but the PUMP has
        // not yet read `TurnFinished` off the bus. The entry is retired; the log
        // is still open and still being written to.
        drop(guard);

        // A second window attaches by the same key, right now.
        let body = serde_json::json!({
            "user_message": serde_json::to_value(Message::user().with_text("hi")).unwrap(),
            "session_id": session_id,
            "turn_id": "client-turn-1",
        });
        let response = tokio::time::timeout(
            Duration::from_secs(10),
            routes(state.clone()).oneshot(
                axum::http::Request::post("/reply")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            ),
        )
        .await
        .expect("the attach must answer")
        .unwrap();
        let _ = tokio::time::timeout(
            Duration::from_secs(10),
            axum::body::to_bytes(response.into_body(), usize::MAX),
        )
        .await
        .expect("…and end")
        .unwrap();

        // …and only NOW does the pump finish its work, exactly as it would in
        // production.
        stream.publish(&MessageEvent::Message {
            message: Message::assistant().with_id("m-1").with_text("part two"),
            token_state: TokenState::default(),
        });
        stream.publish(&MessageEvent::Finish {
            turn_id: None,
            reason: "stop".to_string(),
            token_state: TokenState::default(),
        });
        stream.close();

        let mut frames = Vec::new();
        while let Ok(Some(raw)) = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            frames.extend(sse_frame(&raw));
        }
        let _ = watcher.await;
        let text = frames.iter().map(|f| f.to_string()).collect::<String>();

        assert!(
            text.contains("part two"),
            "an attach must not truncate the turn for the window already watching it: {:?}",
            frame_types(&frames)
        );
        assert!(
            !text.contains("stream_ended_without_terminal"),
            "an attach must not replace a healthy turn's terminal with a synthesized error: {text}"
        );
        assert_eq!(
            frame_types(&frames).last().map(String::as_str),
            Some("Finish"),
            "the turn's OWN terminal must reach the client: {:?}",
            frame_types(&frames)
        );
    }

    /// DEFECT 2 — the eviction fallback's premise. `turn_stream.rs` claims
    /// "the evicted prefix is exactly the part that has already been persisted,
    /// and the un-persisted part (the running assistant message) is exactly the
    /// part that is retained."
    ///
    /// That holds only while the running message fits in the retained window.
    /// One assistant message longer than `REPLAY_BYTE_BUDGET` evicts its OWN
    /// earlier deltas, and the storage resync cannot restore them: the message
    /// has not been persisted — that is precisely why it is still streaming.
    /// The client is then handed the TAIL of a message whose beginning nothing
    /// on the machine still holds.
    ///
    /// User-visible: a long answer (a big code dump, a long report) re-attached
    /// to mid-flight renders with its opening silently missing.
    #[tokio::test]
    async fn a_running_message_bigger_than_the_budget_is_not_recoverable_from_storage() {
        let state = AppState::new().await.unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let session = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "adv-oversized-message".to_string(),
                biorouter::session::session_manager::SessionType::User,
            )
            .await
            .unwrap();
        let guard = state
            .try_begin_turn_idempotent(&session.id, CancellationToken::new(), Some("k".into()))
            .unwrap();
        let stream = guard.stream();
        let _writer = stream
            .claim_writer()
            .expect("this test writes the log itself, standing in for the pump");

        // ONE assistant message, streamed as deltas, larger than the replay
        // budget. Nothing here is persisted: the turn is still running.
        let chunk = "X".repeat(64 * 1024);
        let deltas = (crate::turn_stream::REPLAY_BYTE_BUDGET / chunk.len()) + 8;
        for i in 0..deltas {
            let text = if i == 0 { "OPENING-MARKER" } else { &chunk };
            stream.publish(&MessageEvent::Message {
                message: Message::assistant().with_id("m-1").with_text(text),
                token_state: TokenState::default(),
            });
        }

        let (tx, mut rx) = mpsc::channel::<String>(4096);
        let reader = tokio::spawn(drain_stream_to_client(
            state.clone(),
            session.id.clone(),
            Arc::clone(&stream),
            0,
            false,
            tx,
        ));
        stream.publish(&MessageEvent::Finish {
            turn_id: None,
            reason: "stop".to_string(),
            token_state: TokenState::default(),
        });
        stream.close();
        drop(guard);

        let mut got = String::new();
        while let Ok(Some(raw)) = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            got.push_str(&raw);
        }
        let _ = reader.await;

        assert!(
            got.contains("OPENING-MARKER"),
            "the opening of an un-persisted running message was evicted and the storage \
             resync cannot restore it. R1 says no progress is ever lost"
        );
    }

    /// DEFECT 3 (hardening) — `TurnStream::attach` does not clamp `from_seq` to
    /// the frames the turn actually has. A client that asks to start above the
    /// turn's high-water mark is answered with silence: every subsequent frame
    /// is below `next` and is dropped as "already delivered", and `next` never
    /// walks back. The whole turn is lost for that observer.
    ///
    /// `from_seq` is a client-supplied field on a public route, and the wire
    /// doc calls it "a pure optimisation… nothing about correctness depends on
    /// the server honouring it". Honouring it out of range is not optional.
    #[tokio::test]
    async fn an_out_of_range_from_seq_must_not_silence_the_whole_turn() {
        let stream = crate::turn_stream::TurnStream::new("adv-from-seq", "turn-1");
        let mut reader = stream.attach(10); // the turn has produced nothing yet
        for i in 0..4 {
            stream.publish(&MessageEvent::Message {
                message: Message::assistant()
                    .with_id("m-1")
                    .with_text(format!("chunk-{i}")),
                token_state: TokenState::default(),
            });
        }
        stream.publish(&MessageEvent::Finish {
            turn_id: None,
            reason: "stop".to_string(),
            token_state: TokenState::default(),
        });
        stream.close();

        let mut seqs = Vec::new();
        loop {
            match tokio::time::timeout(Duration::from_secs(5), reader.recv())
                .await
                .expect("the reader must terminate")
            {
                crate::turn_stream::ReaderEvent::Frame(f, _) => seqs.push(f.seq),
                crate::turn_stream::ReaderEvent::Gap => {}
                crate::turn_stream::ReaderEvent::Closed => break,
            }
        }
        assert!(
            !seqs.is_empty(),
            "an out-of-range from_seq silently discarded the entire turn"
        );
    }
}

/// ADVERSARIAL REVIEW — lifecycle attacks on the live turn stream.
///
/// Every test in this module asserts a property the shipped contract claims and
/// FAILS against the current code. They are diagnostic, not a fix.
#[cfg(test)]
mod adversarial_lifecycle {
    use super::*;
    use biorouter::conversation::message::{Message, TokenState};
    use biorouter::session_events::{self, SessionBusEvent};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    fn text(id: &str, body: &str) -> MessageEvent {
        MessageEvent::Message {
            message: Message::assistant().with_id(id).with_text(body),
            token_state: TokenState::default(),
        }
    }

    fn frames_of(raw: &[String]) -> Vec<String> {
        raw.iter()
            .filter_map(|line| {
                serde_json::from_str::<serde_json::Value>(line.strip_prefix("data: ")?.trim_end())
                    .ok()
            })
            .map(|v| v["type"].as_str().unwrap_or("?").to_string())
            .filter(|t| t != "Ping")
            .collect()
    }

    /// ATTACK 1 — the turn with no pump.
    ///
    /// Only `/reply` spawns `pump_bus_into_stream`. Every OTHER creator of a
    /// turn — `workspace::turn::start_turn` (an injected `workspace_send_prompt`
    /// turn), `routes/apps.rs::run_bounded_turn`, and the two routes that take
    /// the turn lock purely as a mutex (`session.rs::edit_in_place`,
    /// `agent.rs::update_working_dir`) — creates a `TurnStream` that nothing
    /// ever publishes into and nothing ever closes.
    ///
    /// `/agent/resume` advertised that turn's id in `active_turn`, and the
    /// renderer auto-attached to it (`noteActiveTurn` -> `resumeActiveTurn` ->
    /// `POST /reply`). The attach was a LIVE one (`finished == false`), so it
    /// took the `drain_stream_to_client` path below — which parked forever.
    ///
    /// BOTH halves are asserted, because either alone leaves the hang reachable:
    /// a turn with no writer is no longer ADVERTISED (so nothing is told to
    /// attach to it), and an attach that happens anyway — a client that kept a
    /// stale id, or guessed one — still ENDS.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_attach_to_a_live_turn_with_no_pump_must_still_end() {
        let state = AppState::new().await.unwrap();
        let session_id = "adversarial-pumpless-live".to_string();
        let guard = state
            .try_begin_turn_idempotent(&session_id, CancellationToken::new(), None)
            .expect("an injected workspace turn takes the same lock");

        // Half one: `/agent/resume` must NOT hand a reloading window a turn
        // whose log nothing writes.
        assert_eq!(
            state.active_turn_id(&session_id),
            None,
            "a turn with no writer must not be advertised as attachable"
        );

        // Half two: an attach that reaches it anyway resolves to a LIVE attach,
        // not the terminal-only path — and must still end.
        let conflict = state
            .try_begin_turn_idempotent(
                &session_id,
                CancellationToken::new(),
                Some(guard.turn_id().to_string()),
            )
            .expect_err("the re-POST is recognised as a duplicate");
        assert!(conflict.duplicate && !conflict.finished);

        let (tx, mut rx) = mpsc::channel::<String>(64);
        let drain = tokio::spawn(drain_stream_to_client(
            state.clone(),
            session_id.clone(),
            Arc::clone(&conflict.stream),
            0,
            /* terminal_only = */ false,
            tx,
        ));

        // The injected turn ends. Its guard retires the entry — and, by design,
        // does not close the log; nothing else will either, because there is no
        // pump.
        tokio::time::sleep(Duration::from_millis(200)).await;
        drop(guard);

        let ended = tokio::time::timeout(Duration::from_secs(3), drain).await;
        let mut received = Vec::new();
        while let Ok(line) = rx.try_recv() {
            received.push(line);
        }
        assert!(
            ended.is_ok(),
            "the attached response never ended: the turn is over and the client is \
             still parked on a stream nothing will ever close. It received only \
             {:?}, and the composer stays disabled (chatState = Streaming) until the \
             window is reloaded.",
            frames_of(&received)
        );
    }

    /// ATTACK 2 — a late attach steals the terminal frame from a pump that is
    /// still draining.
    ///
    /// `TurnGuard::drop` retires the entry the instant the runner returns, but
    /// the runner's last act was to PUBLISH its terminal on the bus; the pump
    /// has not read it yet. `state.rs` documents this race and refuses to close
    /// the log in `Drop` for exactly this reason — and then
    /// `drain_stream_to_client`'s `terminal_only` branch closes it anyway,
    /// guarded only by `terminal_frame().is_none()`, which cannot tell "no pump"
    /// from "pump one scheduler tick behind".
    ///
    /// The cost is not confined to the late attacher: `close()` synthesizes an
    /// Error terminal and `publish` then refuses the real `Finish`, so EVERY
    /// observer of a healthy turn — including the one that watched it from the
    /// start, is told "The stream for this turn ended without a result."
    #[tokio::test(flavor = "multi_thread")]
    async fn a_late_attach_must_not_steal_the_terminal_from_a_draining_pump() {
        let state = AppState::new().await.unwrap();
        let session_id = "adversarial-terminal-race".to_string();
        let cancel = CancellationToken::new();
        let guard = state
            .try_begin_turn_idempotent(&session_id, cancel.clone(), Some("t-1".into()))
            .unwrap();
        let stream = guard.stream();
        let bus = session_events::subscribe(&session_id);
        let pump = tokio::spawn(pump_bus_into_stream(
            state.clone(),
            session_id.clone(),
            bus,
            stream.claim_writer().expect("the test owns this log"),
            cancel.clone(),
            Duration::ZERO,
        ));
        // Some real output, so this is a healthy turn rather than an empty one.
        stream.publish(&text("m-1", "the answer"));

        // The runner returns: the guard retires the entry. `TurnFinished` is
        // already on the bus but the pump has not been scheduled yet.
        drop(guard);

        // A window reloads in exactly that beat and re-POSTs its key.
        let conflict = state
            .try_begin_turn_idempotent(&session_id, CancellationToken::new(), Some("t-1".into()))
            .expect_err("the retired turn is still addressable");
        assert!(conflict.duplicate && conflict.finished);
        let (tx, _rx) = mpsc::channel::<String>(8);
        drain_stream_to_client(
            state.clone(),
            session_id.clone(),
            Arc::clone(&conflict.stream),
            0,
            /* terminal_only = */ true,
            tx,
        )
        .await;

        // …and now the runner's own terminal lands, microseconds late.
        session_events::publish(
            &session_id,
            SessionBusEvent::TurnFinished {
                reason: "stop".into(),
                token_state: None,
            },
        );
        let _ = tokio::time::timeout(Duration::from_secs(3), pump).await;

        let mut reader = stream.attach(0);
        let mut kinds = Vec::new();
        loop {
            match tokio::time::timeout(Duration::from_secs(3), reader.recv()).await {
                Ok(crate::turn_stream::ReaderEvent::Frame(f, _)) => {
                    let json: serde_json::Value = serde_json::from_str(
                        f.live_sse().strip_prefix("data: ").unwrap().trim_end(),
                    )
                    .unwrap();
                    kinds.push(json["type"].as_str().unwrap_or("?").to_string());
                }
                Ok(crate::turn_stream::ReaderEvent::Gap) => {}
                Ok(crate::turn_stream::ReaderEvent::Closed) => break,
                Err(_) => panic!("reader hung"),
            }
        }
        assert_eq!(
            kinds.last().map(String::as_str),
            Some("Finish"),
            "a healthy turn's own terminal was replaced by the synthesized \
             `stream_ended_without_terminal` error because a late attach closed \
             the log first. Frames: {kinds:?}"
        );
    }

    /// ATTACK 3 — an observer that has stopped reading counts as present
    /// forever, so the orphan reaper never fires.
    ///
    /// `Inner::observers` counts ATTACHMENTS, not live clients. A renderer that
    /// is frozen (App Nap, a suspended VM, a paused debugger, a half-open TCP
    /// connection) stops draining its socket; hyper stops polling the response
    /// body; the 100-slot `mpsc` fills; `drain_stream_to_client` parks in
    /// `tx.send().await` and never reaches the heartbeat that would notice the
    /// disconnect. `observers` stays 1, `idle_since` is never set, and the
    /// reaper's `observers > 0` test `continue`s forever.
    ///
    /// Nobody is watching, and nothing will stop the turn.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_client_that_stopped_reading_must_not_defeat_the_orphan_reaper() {
        let state = AppState::new().await.unwrap();
        let stream = crate::turn_stream::TurnStream::new("s", "turn-frozen");
        let cancel = CancellationToken::new();

        let _writer = stream
            .claim_writer()
            .expect("this test writes the log itself, standing in for the pump");

        // The receiver stays ALIVE — the socket is open, the client is simply
        // not reading. Dropping it instead is the case the code handles.
        let (tx, _rx_open_but_never_polled) = mpsc::channel::<String>(100);
        let _drain = tokio::spawn(drain_stream_to_client(
            state,
            "s".to_string(),
            Arc::clone(&stream),
            0,
            false,
            tx,
        ));
        let reaper = stream.spawn_orphan_reaper(cancel.clone(), Duration::from_millis(200));

        // Fill the socket buffer, then keep the turn producing.
        for i in 0..400 {
            stream.publish(&text(&format!("m-{i}"), "burning tokens"));
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            stream.observer_count(),
            1,
            "the frozen client is still counted as an observer"
        );
        for i in 400..800 {
            stream.publish(&text(&format!("m-{i}"), "still burning tokens"));
        }

        let reaped = tokio::time::timeout(Duration::from_secs(3), cancel.cancelled()).await;
        reaper.abort();
        assert!(
            reaped.is_ok(),
            "the orphan reaper never fired: a client that stopped reading holds \
             the turn alive indefinitely, which is the exact state the reaper \
             exists to end"
        );
    }

    /// DEFECT 4 — an ATTACH that misses becomes a NEW TURN, carrying the
    /// prompt the client only sent as a formality.
    ///
    /// `/reply` has no way to say "attach only". Outcome 1 of the wire contract
    /// is "a `turn_id` naming no known turn starts a new turn", and the client's
    /// `buildAttachRequest` fills `user_message` with the transcript's TRAILING
    /// USER MESSAGE. So the moment the turn a client is re-attaching to is not
    /// in the registry — the daemon restarted (the commonest reason a driving
    /// stream ends without a terminal frame), or `FINISHED_TURN_RETENTION`
    /// elapsed — `reattachAfterDrop` silently re-submits the user's prompt.
    ///
    /// User-visible: the answer is generated a second time and rendered under
    /// the half-answer already on screen (the new turn's `turn-N` resets the
    /// client's sequence gate), and the tokens are spent twice. The contract's
    /// own words: "nothing is charged twice."
    #[tokio::test]
    async fn an_attach_to_a_turn_that_is_gone_must_not_re_submit_the_prompt() {
        use tower::ServiceExt;
        let state = AppState::new().await.unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let session = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "adv-attach-misses".to_string(),
                biorouter::session::session_manager::SessionType::User,
            )
            .await
            .unwrap();

        // Exactly the body `buildAttachRequest` produces: an attach pointer, a
        // high-water mark, and the trailing user message it is obliged to send.
        let body = serde_json::json!({
            "user_message": serde_json::to_value(
                Message::user().with_text("summarise the whole cohort")
            ).unwrap(),
            "session_id": session.id,
            "turn_id": "turn-that-no-longer-exists",
            "from_seq": 41,
        });
        let response = routes(state.clone())
            .oneshot(
                axum::http::Request::post("/reply")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let started_a_turn = state.active_turn_id(&session.id).is_some()
            || state
                .session_manager()
                .get_session(&session.id, true)
                .await
                .map(|s| {
                    s.conversation
                        .map(|c| {
                            c.messages().iter().any(|m: &Message| {
                                m.as_concat_text().contains("summarise the whole cohort")
                            })
                        })
                        .unwrap_or(false)
                })
                .unwrap_or(false);
        let _ = axum::body::to_bytes(response.into_body(), usize::MAX).await;

        assert!(
            !started_a_turn,
            "an attach whose turn is gone started a NEW turn and re-sent the user's \
             prompt: the answer is produced twice and billed twice"
        );
    }
}
