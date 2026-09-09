//! BR-71 §4.2: the read-only observer stream. Lets a subagent tab, a second
//! window, or a parent-watching-child render a turn none of them started.
//! Frames reuse the `/reply` wire enum (`MessageEvent`) so the generated TS
//! client and `chatStreamStore.tsx` parse them unchanged.
//!
//! `map_bus_event` below is `pub(crate)` because after Task 8 it has TWO
//! callers: this route and `/reply` itself. That is what makes "an observer
//! sees exactly what the client sees" structural rather than a property two
//! hand-written loops have to keep agreeing on.
//!
//! **The inner `match ev` is EXHAUSTIVE and must stay that way — never add a
//! `_ => None` arm.** `AgentEvent` has eight variants today
//! (`agents/agent.rs`); an exhaustive match is what makes the NINTH fail the
//! build instead of silently vanishing from every wire. The repo makes the same
//! choice, for the same reason, at `MessageContent::is_pin_eligible`
//! (`conversation/message.rs`). If a future variant genuinely has no wire form,
//! give it a named arm returning `None` with the reason written next to it.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use biorouter::agents::AgentEvent;
use biorouter::session_events::{self, SessionBusEvent};
use tokio::sync::mpsc;

use crate::routes::reply::{get_token_state, MessageEvent, SseResponse, TurnErrorScope};
use crate::state::AppState;

/// Map one bus event to a wire frame. `None` means "nothing to send" (token
/// updates fold into the cached state that stamps subsequent frames, exactly as
/// the `/reply` loop does — BR-52).
pub(crate) fn map_bus_event(
    event: SessionBusEvent,
    token_state: &mut biorouter::conversation::message::TokenState,
) -> Option<MessageEvent> {
    map_bus_event_for_turn(event, token_state, None)
}

fn map_bus_event_for_turn(
    event: SessionBusEvent,
    token_state: &mut biorouter::conversation::message::TokenState,
    terminal_turn_id: Option<&str>,
) -> Option<MessageEvent> {
    match event {
        SessionBusEvent::TurnStarted { turn_id } => Some(MessageEvent::TurnStarted { turn_id }),
        SessionBusEvent::TurnFinished {
            reason,
            token_state: final_state,
        } => {
            // BR-52: prefer the runner's authoritative end-of-turn read over the
            // running total this consumer accumulated from TokenUsage events.
            if let Some(final_state) = final_state {
                *token_state = final_state;
            }
            Some(MessageEvent::Finish {
                turn_id: terminal_turn_id.map(str::to_string),
                reason,
                token_state: token_state.clone(),
            })
        }
        // Reconciliation #9: a terminal error carries the four fields
        // `AgentEvent::TurnAborted` cannot express, so `/reply`'s envelope
        // survives the round trip through the bus byte-for-byte.
        SessionBusEvent::TurnError {
            message,
            code,
            scope,
            retryable,
            provider_kind,
        } => Some(MessageEvent::Error {
            turn_id: terminal_turn_id.map(str::to_string),
            error: message,
            code,
            scope: TurnErrorScope::from_wire_value(&scope),
            retryable,
            provider_kind,
        }),
        SessionBusEvent::Agent(ev) => match ev {
            AgentEvent::Message(message) => Some(MessageEvent::Message {
                message,
                token_state: token_state.clone(),
            }),
            AgentEvent::TokenUsage(new_state) => {
                *token_state = new_state;
                None
            }
            AgentEvent::HistoryReplaced(conversation) => Some(MessageEvent::UpdateConversation {
                conversation,
                token_state: token_state.clone(),
            }),
            AgentEvent::ModelChange { model, mode } => {
                Some(MessageEvent::ModelChange { model, mode })
            }
            AgentEvent::McpNotification((request_id, message)) => {
                Some(MessageEvent::Notification {
                    request_id,
                    message,
                })
            }
            AgentEvent::ToolCallPending(p) => Some(MessageEvent::ToolCallPending {
                id: p.id,
                name: p.name,
                partial_args: p.partial_args,
            }),
            // Reconciliation #22 / #59: the accounting frame naming the ids the
            // turn's rows were actually stored under.
            //
            // A pass-through — `MessageEvent` already has the
            // identically-shaped variant carrying the same
            // `Vec<PersistedMessage>` — but it is NOT optional, and it is NOT
            // `None`. Without it a client that watches a whole turn ends it
            // knowing none of the ids the store holds, so `expectedMessageIds`
            // on `POST /sessions/{id}/edit_message` is unsatisfiable and the
            // in-place edit it guards answers 409 on an untouched session.
            //
            // ORDERING (`agents/agent.rs`, and its `messages_then_persisted`
            // test): no `MessagesPersisted` may precede a `Message` frame
            // carrying one of the ids it publishes. This mapper preserves
            // whatever order it is fed — it is the CONSUMERS that must flush
            // before forwarding this frame (this route sends immediately, so it
            // has nothing buffered; Task 8's `/reply` loop holds a
            // `DeltaCoalescer` and flushes first — see its `!matches!(…
            // TokenUsage)` guard, which is load-bearing for exactly this).
            AgentEvent::MessagesPersisted(messages) => {
                Some(MessageEvent::MessagesPersisted { messages })
            }
            // Issue #56 Gate B's repair arm, said out loud. A pass-through: the
            // frame is advisory display state and the wire enum carries the
            // identically-shaped variant.
            AgentEvent::PrivacyProviderPinned { provider, model } => {
                Some(MessageEvent::PrivacyProviderPinned { provider, model })
            }
            // FALLBACK ONLY. The turn runner never publishes a raw
            // `TurnAborted` — it classifies it and publishes `TurnError`
            // instead, precisely so no consumer renders two terminal Error
            // frames for one abort (Task 6). This arm exists for publishers
            // that only tee raw agent events onto the bus (Task 34's subagent
            // runs), and it reuses the SAME classifier so those frames carry the
            // provider envelope too.
            AgentEvent::TurnAborted { code, message } => {
                let (scope, retryable, provider_kind) =
                    crate::workspace::turn::classify_abort(&code);
                Some(MessageEvent::Error {
                    turn_id: terminal_turn_id.map(str::to_string),
                    error: message,
                    code: code.wire_code().to_string(),
                    scope,
                    retryable,
                    provider_kind,
                })
            }
        },
    }
}

fn is_terminal_bus_event(event: &SessionBusEvent) -> bool {
    matches!(
        event,
        SessionBusEvent::TurnFinished { .. }
            | SessionBusEvent::TurnError { .. }
            | SessionBusEvent::Agent(AgentEvent::TurnAborted { .. })
    )
}

fn stable_active_turn_id(before: &Option<String>, after: &Option<String>) -> Option<String> {
    (before == after).then(|| after.clone()).flatten()
}

/// The conversation snapshot a lagged consumer sends instead of silently
/// skipping frames (§8.4). `pub(crate)` because `/reply` shares this storage
/// repair; the observer adds its authoritative lifecycle snapshot alongside it.
pub(crate) async fn bus_lag_resync_frame(
    state: &AppState,
    session_id: &str,
    token_state: &biorouter::conversation::message::TokenState,
) -> Option<MessageEvent> {
    let fresh = state
        .session_manager()
        .get_session(session_id, true)
        .await
        .ok()?;
    Some(MessageEvent::UpdateConversation {
        conversation: fresh.conversation.unwrap_or_default(),
        token_state: token_state.clone(),
    })
}

async fn observer_lag_resync_frames(
    state: &AppState,
    session_id: &str,
    token_state: &biorouter::conversation::message::TokenState,
) -> Option<(MessageEvent, MessageEvent, Option<String>)> {
    let turn_id_before_resync = state.active_turn_id(session_id);
    let conversation = bus_lag_resync_frame(state, session_id, token_state).await?;
    let active_turn_id = state.active_turn_id(session_id);
    let event_turn_id = stable_active_turn_id(&turn_id_before_resync, &active_turn_id);
    Some((
        conversation,
        MessageEvent::TurnState { active_turn_id },
        event_turn_id,
    ))
}

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/events",
    // EXPLICIT tag, not utoipa's default module path: Task 42b's CLI-parity gate
    // selects the workspace-control route surface by this tag.
    tag = "workspace",
    params(("session_id" = String, Path, description = "Session to observe")),
    responses(
        (status = 200, description = "Read-only observer stream of the session's live events",
         body = MessageEvent, content_type = "text/event-stream"),
        (status = 403, description = "Out of reach - a private or unreadable session named without the user-action proof"),
        (status = 404, description = "No such session"),
        (status = 401, description = "Unauthorized - invalid secret key")
    )
)]
pub async fn observe_session_events(
    Path(session_id): Path<String>,
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    // Issue #56. This route is `GET /sessions/{session_id}` plus a live tail: the
    // first frame it sends is the whole stored conversation. That sibling has
    // been gated by `session_reach` since Task 58 and this one was not, which is
    // the "unguarded sibling" defect exactly. The gate goes FIRST — ahead of the
    // bus subscription as well as the store read, because a subscription that
    // outlives a refusal is a side channel of its own.
    if let Err(refusal) =
        crate::routes::session_reach::session_reach(state.session_manager(), &session_id, &headers)
            .await
    {
        return refusal.into_response();
    }

    // Subscribe BEFORE the snapshot so no event falls in the gap between them.
    let mut rx = session_events::subscribe(&session_id);
    // Read immediately after subscribing, before the first await. If the same
    // turn is still active at the later snapshot, terminals already queued on
    // this receiver can be attributed to it. If the identities differ, a turn
    // boundary crossed the snapshot window and those queued terminals remain
    // deliberately unclaimed rather than being mislabeled as the successor.
    let turn_id_after_subscribe = state.active_turn_id(&session_id);

    let session = match state.session_manager().get_session(&session_id, true).await {
        Ok(s) => s,
        Err(_) => return axum::http::StatusCode::NOT_FOUND.into_response(),
    };

    // The subscription above and this authoritative registry read close both
    // sides of the join race. A turn that started before the subscription is
    // named here; one that starts after it is queued on `rx`. A turn crossing
    // the read may be named twice, which is deliberately harmless and safer
    // than leaving a quiet observer permanently idle.
    let active_turn_id = state.active_turn_id(&session_id);
    let initial_event_turn_id = stable_active_turn_id(&turn_id_after_subscribe, &active_turn_id);
    let mut token_state = get_token_state(state.session_manager(), &session_id).await;
    let (tx, rx_out) = mpsc::channel::<String>(64);

    // Claimed HERE, in the handler, rather than inside the task below: the slot
    // has to be taken before this response is handed back, or a burst of tabs
    // re-attaching together would each be spawned, each find the count still
    // under budget, and all be admitted.
    let slot = state.try_admit_observer_stream();

    let manager_session_id = session_id.clone();
    let state_for_task = state.clone();
    tokio::spawn(async move {
        let mut event_turn_id = initial_event_turn_id;
        let send = |tx: &mpsc::Sender<String>, ev: &MessageEvent| {
            let frame = format!(
                "data: {}\n\n",
                serde_json::to_string(ev).unwrap_or_default()
            );
            let tx = tx.clone();
            async move { tx.send(frame).await.is_ok() }
        };

        // Join-mid-turn snapshot: the observer starts from the full stored
        // conversation, then applies live events (BR-71 §4.2).
        let snapshot = MessageEvent::UpdateConversation {
            conversation: session.conversation.unwrap_or_default(),
            token_state: token_state.clone(),
        };
        if !send(&tx, &snapshot).await {
            return;
        }
        // Unlike a positive-only `TurnStarted`, this snapshot also carries an
        // authoritative idle edge. An observer that missed Finish while its
        // socket was down can therefore retire stale running state on reconnect.
        if !send(&tx, &MessageEvent::TurnState { active_turn_id }).await {
            return;
        }

        // Over budget: answer with the snapshot and END, instead of parking a
        // connection the client cannot spare (see `MAX_LIVE_OBSERVER_STREAMS`).
        // Returning here drops `tx`, which completes the response body — the
        // client sees a stream that finished, which is already its reconnect
        // trigger, and it comes back on its own backoff.
        //
        // The snapshot is deliberately sent FIRST, before this check. A refused
        // observer is not a failed one: it has been handed the entire stored
        // conversation, which is all a tab nobody is looking at actually needs.
        // Refusing before the snapshot would leave the tab blank and turn a
        // capacity limit into missing content.
        let Some(slot) = slot else {
            tracing::debug!(
                session_id = %manager_session_id,
                "observer over budget; answered with the snapshot and closed",
            );
            return;
        };
        // Held for exactly as long as this stream follows the tail, and released
        // by `Drop` on every way out of this task — including the client hanging
        // up mid-`send` and the bus closing underneath it.
        let _slot = slot;

        let mut heartbeat = tokio::time::interval(Duration::from_millis(500));
        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    if !send(&tx, &MessageEvent::Ping).await { return; }
                }
                received = rx.recv() => match received {
                    Ok(event) => {
                        if let SessionBusEvent::TurnStarted { turn_id } = &event {
                            event_turn_id = Some(turn_id.clone());
                        }
                        let terminal = is_terminal_bus_event(&event);
                        if let Some(mapped) = map_bus_event_for_turn(
                            event,
                            &mut token_state,
                            event_turn_id.as_deref(),
                        ) {
                            if !send(&tx, &mapped).await { return; }
                        }
                        if terminal {
                            event_turn_id = None;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // §8.4: resync from storage instead of dropping frames
                        // silently. Shared with /reply (Task 8).
                        if let Some((resync, lifecycle, next_event_turn_id)) =
                            observer_lag_resync_frames(
                            &state_for_task,
                            &manager_session_id,
                            &token_state,
                        )
                        .await
                        {
                            if !send(&tx, &resync).await { return; }
                            if !send(&tx, &lifecycle).await { return; }
                            event_turn_id = next_event_turn_id;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                },
            }
        }
    });

    SseResponse::from_rx(rx_out).into_response()
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/sessions/{session_id}/events", get(observe_session_events))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use biorouter::agents::AgentEvent;
    use biorouter::conversation::message::Message;
    use biorouter::session_events::SessionBusEvent;

    #[test]
    fn maps_lifecycle_and_messages_and_swallows_token_updates() {
        let mut token_state = Default::default();

        let started = map_bus_event(
            SessionBusEvent::TurnStarted {
                turn_id: "turn-1".into(),
            },
            &mut token_state,
        )
        .expect("turn lifecycle maps");
        assert_eq!(
            serde_json::to_value(started).unwrap(),
            serde_json::json!({ "type": "TurnStarted", "turn_id": "turn-1" })
        );

        let mapped = map_bus_event(
            SessionBusEvent::Agent(AgentEvent::Message(Message::user().with_text("hello"))),
            &mut token_state,
        )
        .expect("message maps");
        assert!(serde_json::to_string(&mapped)
            .unwrap()
            .contains("\"type\":\"Message\""));

        let fin = map_bus_event(
            SessionBusEvent::TurnFinished {
                reason: "stop".into(),
                token_state: None,
            },
            &mut token_state,
        )
        .expect("finish maps");
        assert!(serde_json::to_string(&fin)
            .unwrap()
            .contains("\"type\":\"Finish\""));
    }

    #[test]
    fn observer_terminals_name_the_turn_the_bus_receiver_was_following() {
        let mut token_state = Default::default();
        let finish = map_bus_event_for_turn(
            SessionBusEvent::TurnFinished {
                reason: "done".into(),
                token_state: None,
            },
            &mut token_state,
            Some("turn-before-snapshot"),
        )
        .expect("terminal maps");

        assert_eq!(
            serde_json::to_value(finish).unwrap()["turn_id"],
            "turn-before-snapshot"
        );
    }

    #[test]
    fn terminal_queued_before_a_successor_snapshot_remains_unclaimed() {
        let before = Some("turn-before-snapshot".to_string());
        let successor = Some("turn-successor".to_string());
        let terminal_turn_id = stable_active_turn_id(&before, &successor);
        let mut token_state = Default::default();
        let finish = map_bus_event_for_turn(
            SessionBusEvent::TurnFinished {
                reason: "done".into(),
                token_state: None,
            },
            &mut token_state,
            terminal_turn_id.as_deref(),
        )
        .expect("terminal maps");

        assert!(serde_json::to_value(finish)
            .unwrap()
            .get("turn_id")
            .is_none());
    }

    #[tokio::test]
    async fn observer_lag_resync_repairs_conversation_and_authoritative_turn_state() {
        let state = crate::state::AppState::new().await.unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let session = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "lagged observer".to_string(),
                biorouter::session::session_manager::SessionType::SubAgent,
            )
            .await
            .unwrap();
        let guard = state
            .try_begin_turn_idempotent(
                &session.id,
                tokio_util::sync::CancellationToken::new(),
                Some("lagged-turn".into()),
            )
            .expect("turn starts");
        let _writer = guard
            .stream()
            .claim_writer()
            .expect("turn owns a live writer");
        let turn_id = guard.turn_id().to_string();

        let token_state = Default::default();
        let (conversation, lifecycle, event_turn_id) =
            observer_lag_resync_frames(&state, &session.id, &token_state)
                .await
                .expect("session can be resynced");
        assert!(matches!(
            conversation,
            MessageEvent::UpdateConversation { .. }
        ));
        assert!(matches!(
            lifecycle,
            MessageEvent::TurnState {
                active_turn_id: Some(ref active)
            } if active == &turn_id
        ));
        assert_eq!(event_turn_id, Some(turn_id));
        drop(guard);
    }

    /// Reconciliation #22 / #59: `MessagesPersisted` reaches the wire through
    /// the mapper, and it maps to a frame — NOT to `None`.
    ///
    /// This is the one arm whose *plausible wrong answer compiles*. Dropping it
    /// (or reaching for a `_ => None` wildcard to silence `E0004`) is invisible
    /// to every other test in this plan: the desktop store deliberately does not
    /// consume the frame yet, and `reply.rs`'s own
    /// `persisted_message_ids_reach_the_wire_with_their_visibility` tests the
    /// *enum's serialization*, not the handler — so it stays green while no
    /// handler emits the frame at all. After Task 8 that silence means no
    /// `/reply` client ever learns a stored id again, `expectedMessageIds`
    /// becomes unsatisfiable, and `POST /sessions/{id}/edit_message` answers 409
    /// on sessions nobody touched: exactly the regression `0312dff4` +
    /// `936f5a33` were written to close.
    #[test]
    fn a_persisted_batch_maps_to_a_wire_frame_and_never_to_none() {
        use biorouter::agents::PersistedMessage;
        let mut token_state = Default::default();

        let mapped = map_bus_event(
            SessionBusEvent::Agent(AgentEvent::MessagesPersisted(vec![
                PersistedMessage {
                    id: "m-1".into(),
                    user_visible: true,
                },
                PersistedMessage {
                    id: "m-2".into(),
                    user_visible: false,
                },
            ])),
            &mut token_state,
        )
        .expect("MessagesPersisted maps to a frame, not to None");

        let json = serde_json::to_value(&mapped).unwrap();
        assert_eq!(json["type"], "MessagesPersisted");
        assert_eq!(json["messages"][0]["id"], "m-1");
        // `userVisible` is camelCase on the wire and BOTH values survive: a
        // client must be able to NAME a hidden row without drawing it.
        assert_eq!(json["messages"][0]["userVisible"], true);
        assert_eq!(json["messages"][1]["userVisible"], false);
    }

    #[test]
    fn output_recovery_exhaustion_maps_to_a_typed_non_retryable_sse_error() {
        use biorouter::agents::TurnAbortCode;

        let mut token_state = Default::default();
        let mapped = map_bus_event(
            SessionBusEvent::Agent(AgentEvent::TurnAborted {
                code: TurnAbortCode::OutputRecoveryExhausted {
                    continuations: 12,
                    zero_progress: false,
                },
                message: "Automatic continuation stopped after 12 attempts.".into(),
            }),
            &mut token_state,
        )
        .expect("terminal abort maps to an SSE error frame");

        let json = serde_json::to_value(mapped).unwrap();
        assert_eq!(json["type"], "Error");
        assert_eq!(json["code"], "output_recovery_exhausted");
        assert_eq!(json["scope"], "inference");
        assert_eq!(json["retryable"], false);
        assert_eq!(
            json["error"],
            "Automatic continuation stopped after 12 attempts."
        );
        assert!(json.get("provider_kind").is_none());
    }

    /// Reconciliation #9: every `TurnErrorScope` variant survives the string
    /// round trip through the bus, and an unknown one degrades instead of
    /// panicking. All FOUR variants — `Provider` is the one the desktop's
    /// retry/rate-limit recovery keys off.
    #[test]
    fn turn_error_scopes_round_trip_through_their_wire_values() {
        use crate::routes::reply::TurnErrorScope;
        for scope in [
            TurnErrorScope::Provider,
            TurnErrorScope::Session,
            TurnErrorScope::Inference,
            TurnErrorScope::Internal,
        ] {
            assert_eq!(TurnErrorScope::from_wire_value(scope.wire_value()), scope);
            // …and the wire value is what serde emits, so the enum and the bus
            // can never drift apart.
            assert_eq!(
                serde_json::to_value(&scope).unwrap(),
                serde_json::Value::String(scope.wire_value().to_string())
            );
        }
        assert_eq!(
            TurnErrorScope::from_wire_value("a_scope_from_a_newer_runner"),
            TurnErrorScope::Internal
        );
    }

    #[tokio::test]
    async fn observer_gets_snapshot_then_live_events() {
        use tower::ServiceExt;
        let state = crate::state::AppState::new().await.unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let session = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "obs".to_string(),
                biorouter::session::session_manager::SessionType::User,
            )
            .await
            .unwrap();

        let app = routes(state.clone());
        let response = app
            .oneshot(
                axum::http::Request::get(format!("/sessions/{}/events", session.id))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        // Publish one live event, then read the body: it must contain the
        // snapshot (UpdateConversation) and then the Finish frame.
        session_events::publish(
            &session.id,
            SessionBusEvent::TurnFinished {
                reason: "stop".into(),
                token_state: None,
            },
        );
        let bytes =
            tokio::time::timeout(Duration::from_secs(5), collect_prefix(response.into_body()))
                .await
                .expect("body bytes in time");
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("\"type\":\"UpdateConversation\""));
        assert!(text.contains("\"type\":\"Finish\""));
    }

    #[tokio::test]
    async fn observer_snapshot_names_an_already_active_quiet_turn() {
        use tower::ServiceExt;
        let state = crate::state::AppState::new().await.unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let session = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "quiet child".to_string(),
                biorouter::session::session_manager::SessionType::SubAgent,
            )
            .await
            .unwrap();
        let guard = state
            .try_begin_turn_idempotent(
                &session.id,
                tokio_util::sync::CancellationToken::new(),
                Some("delegated-turn".into()),
            )
            .expect("delegated turn starts");
        let _writer = guard
            .stream()
            .claim_writer()
            .expect("delegated turn owns an attachable stream writer");
        let turn_id = guard.turn_id().to_string();

        let response = routes(state.clone())
            .oneshot(
                axum::http::Request::get(format!("/sessions/{}/events", session.id))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        session_events::publish(
            &session.id,
            SessionBusEvent::TurnFinished {
                reason: "done".into(),
                token_state: None,
            },
        );

        let bytes =
            tokio::time::timeout(Duration::from_secs(5), collect_prefix(response.into_body()))
                .await
                .expect("observer lifecycle in time");
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("\"type\":\"UpdateConversation\""));
        assert!(text.contains(&format!(
            "\"type\":\"TurnState\",\"active_turn_id\":\"{turn_id}\""
        )));
        assert!(text.contains("\"type\":\"Finish\""));
        drop(guard);
    }

    #[tokio::test]
    async fn reconnect_after_a_missed_finish_receives_authoritative_idle_state() {
        use tower::ServiceExt;
        let state = crate::state::AppState::new().await.unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let session = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "reconnecting child".to_string(),
                biorouter::session::session_manager::SessionType::SubAgent,
            )
            .await
            .unwrap();
        let guard = state
            .try_begin_turn_idempotent(
                &session.id,
                tokio_util::sync::CancellationToken::new(),
                Some("delegated-turn".into()),
            )
            .expect("delegated turn starts");
        let writer = guard
            .stream()
            .claim_writer()
            .expect("delegated turn owns its stream writer");
        let turn_id = guard.turn_id().to_string();
        let app = routes(state.clone());

        let first = app
            .clone()
            .oneshot(
                axum::http::Request::get(format!("/sessions/{}/events", session.id))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let active_marker = format!("\"active_turn_id\":\"{turn_id}\"");
        let first_text = tokio::time::timeout(
            Duration::from_secs(5),
            collect_until(first.into_body(), &active_marker),
        )
        .await
        .expect("initial observer state in time");
        assert!(String::from_utf8_lossy(&first_text).contains("\"type\":\"TurnState\""));

        // Wait until the observer task has noticed the dropped response. The
        // terminal below must land wholly inside the disconnect gap.
        for _ in 0..40 {
            if state.live_observer_streams() == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(state.live_observer_streams(), 0);
        session_events::publish(
            &session.id,
            SessionBusEvent::TurnFinished {
                reason: "finished while disconnected".into(),
                token_state: None,
            },
        );
        drop(writer);
        assert!(state.is_turn_active(&session.id));
        assert_eq!(state.active_turn_id(&session.id), None);

        let reconnected = app
            .oneshot(
                axum::http::Request::get(format!("/sessions/{}/events", session.id))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let reconnected_text = tokio::time::timeout(
            Duration::from_secs(5),
            collect_until(reconnected.into_body(), "\"active_turn_id\":null"),
        )
        .await
        .expect("reconnected observer idle state in time");
        let reconnected_text = String::from_utf8_lossy(&reconnected_text);
        assert!(reconnected_text.contains("\"type\":\"UpdateConversation\""));
        assert!(reconnected_text.contains("\"type\":\"TurnState\""));
        assert!(reconnected_text.contains("\"active_turn_id\":null"));
        drop(guard);
    }

    /// Read chunks until both expected markers have arrived, then stop.
    ///
    /// `axum::body::to_bytes` — which every other body-reading test in this
    /// crate uses (`routes/session.rs`, `routes/reply.rs`) — CANNOT be used
    /// here: this body is an observer stream that never ends, so `to_bytes`
    /// would hang until the test's 5 s timeout every time.
    ///
    /// Nor can it use `http_body_util::BodyExt::frame()`: `http-body-util` is
    /// not a dependency of `biorouter-server` at any level (the only manifest in
    /// the workspace that has it is `crates/biorouter-headless/Cargo.toml`),
    /// and Rust does not let a crate import a transitive dependency — that is
    /// E0432, which fails the WHOLE crate's test build, so every test in this
    /// module would stop running. `Body::into_data_stream()` (axum-core 0.5) is
    /// the same thing over `futures::StreamExt`, which IS a direct dependency
    /// (`crates/biorouter-server/Cargo.toml`).
    async fn collect_prefix(body: axum::body::Body) -> Vec<u8> {
        use futures::StreamExt;
        let mut stream = body.into_data_stream();
        let mut collected = Vec::new();
        while let Some(Ok(chunk)) = stream.next().await {
            collected.extend_from_slice(&chunk);
            let text = String::from_utf8_lossy(&collected);
            if text.contains("UpdateConversation") && text.contains("Finish") {
                break;
            }
        }
        collected
    }

    async fn collect_until(body: axum::body::Body, marker: &str) -> Vec<u8> {
        use futures::StreamExt;
        let mut stream = body.into_data_stream();
        let mut collected = Vec::new();
        while let Some(Ok(chunk)) = stream.next().await {
            collected.extend_from_slice(&chunk);
            if String::from_utf8_lossy(&collected).contains(marker) {
                break;
            }
        }
        collected
    }

    /// Read a response body to its END, or give up.
    ///
    /// `Some(text)` means the body COMPLETED inside `budget`; `None` means it
    /// was still open when the budget ran out. That distinction is the whole
    /// assertion in the two budget tests below, and it cannot be made with
    /// `collect_prefix` above — that one stops on a marker, so it returns
    /// happily for a stream that is still holding its connection, which is
    /// exactly the failure being tested for.
    async fn drain_to_end(body: axum::body::Body, budget: Duration) -> Option<String> {
        use futures::StreamExt;
        let read = async {
            let mut stream = body.into_data_stream();
            let mut collected = Vec::new();
            while let Some(Ok(chunk)) = stream.next().await {
                collected.extend_from_slice(&chunk);
            }
            String::from_utf8_lossy(&collected).into_owned()
        };
        tokio::time::timeout(budget, read).await.ok()
    }

    async fn observer_session(state: &Arc<crate::state::AppState>) -> String {
        let temp = tempfile::TempDir::new().unwrap();
        state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "budget".to_string(),
                biorouter::session::session_manager::SessionType::User,
            )
            .await
            .unwrap()
            .id
    }

    async fn open_observer(app: &axum::Router, session_id: &str) -> axum::response::Response {
        use tower::ServiceExt;
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::get(format!("/sessions/{session_id}/events"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        response
    }

    /// The wedge this budget exists for: an observer never ends on its own, so
    /// enough of them park the client's whole per-host connection budget and
    /// the renderer can no longer send **anything** to the daemon — including
    /// the `POST /reply` the user is waiting on. Measured on the real app:
    /// six live observers, 348 bytes/s of heartbeat out and 0 in, a `GET
    /// /status` from inside the renderer unanswered after 8 s while the same
    /// call from a shell returned in 0.9 ms.
    ///
    /// So past the budget the answer is the snapshot and then a **finished
    /// body**. Both halves are asserted, and they fail for different reasons:
    /// without the budget the extra body never completes (the wedge itself),
    /// and refusing before the snapshot leaves the tab blank.
    #[tokio::test]
    async fn an_observer_past_the_budget_is_answered_and_closed_not_parked() {
        let state = crate::state::AppState::new().await.unwrap();
        let session_id = observer_session(&state).await;
        let app = routes(state.clone());

        // Fill the budget and HOLD the bodies, the way open tabs do.
        let mut held = Vec::new();
        for _ in 0..crate::state::MAX_LIVE_OBSERVER_STREAMS {
            held.push(open_observer(&app, &session_id).await.into_body());
        }
        assert_eq!(
            state.live_observer_streams(),
            crate::state::MAX_LIVE_OBSERVER_STREAMS
        );

        let extra = open_observer(&app, &session_id).await;
        let text = drain_to_end(extra.into_body(), Duration::from_secs(5))
            .await
            .expect(
                "an over-budget observer must END its body; still open means it is \
                 holding one of the client's few connections for a tail it was not \
                 admitted to follow",
            );
        assert!(
            text.contains("\"type\":\"UpdateConversation\""),
            "a refused observer is still owed the stored conversation, else the tab \
             renders empty: {text}"
        );
        assert_eq!(
            state.live_observer_streams(),
            crate::state::MAX_LIVE_OBSERVER_STREAMS,
            "a refused observer must not consume a slot"
        );

        // The control: an ADMITTED observer does the opposite — it keeps the
        // connection and follows the tail. Without this, a budget of zero (or a
        // slot that is never handed out) would pass every assertion above.
        let admitted = held.pop().expect("one held stream");
        assert!(
            drain_to_end(admitted, Duration::from_secs(2))
                .await
                .is_none(),
            "an admitted observer must keep following the tail, not end"
        );
    }

    /// The slot comes back when the stream does, so a long-lived daemon does not
    /// ratchet its way down to refusing everything.
    ///
    /// This is the half a counter that only ever increments still passes the
    /// test above for: closing a tab has to make room for the next one, and the
    /// release has to happen on the path a real client takes — hanging up — not
    /// on a tidy shutdown the code controls.
    #[tokio::test]
    async fn closing_an_observer_returns_its_slot_to_the_budget() {
        let state = crate::state::AppState::new().await.unwrap();
        let session_id = observer_session(&state).await;
        let app = routes(state.clone());

        let mut held = Vec::new();
        for _ in 0..crate::state::MAX_LIVE_OBSERVER_STREAMS {
            held.push(open_observer(&app, &session_id).await.into_body());
        }

        // Hang up on one, the way a closed tab does.
        drop(held.pop());

        // The task learns of it on its next heartbeat send, so poll rather than
        // sleeping a guessed interval.
        let mut released = false;
        for _ in 0..40 {
            if state.live_observer_streams() < crate::state::MAX_LIVE_OBSERVER_STREAMS {
                released = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            released,
            "a hung-up observer never gave its slot back; the budget only shrinks"
        );

        // And the freed slot is genuinely usable: the next observer is admitted
        // and follows the tail instead of being answered and closed.
        let fresh = open_observer(&app, &session_id).await;
        assert!(
            drain_to_end(fresh.into_body(), Duration::from_secs(2))
                .await
                .is_none(),
            "the observer taking the freed slot must be admitted, not refused"
        );
    }

    #[tokio::test]
    async fn closing_a_child_observer_does_not_cancel_its_turn_or_parent_monitor() {
        let state = crate::state::AppState::new().await.unwrap();
        let temp = tempfile::TempDir::new().unwrap();
        let child = state
            .session_manager()
            .create_session(
                temp.path().to_path_buf(),
                "observed child".to_string(),
                biorouter::session::session_manager::SessionType::SubAgent,
            )
            .await
            .unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let guard = state
            .try_begin_turn_idempotent(&child.id, cancel.clone(), Some("delegated-turn".into()))
            .expect("child turn starts");
        let turn_id = guard.turn_id().to_string();
        let mut parent_monitor = session_events::subscribe(&child.id);
        let body = open_observer(&routes(state.clone()), &child.id)
            .await
            .into_body();

        session_events::publish(
            &child.id,
            SessionBusEvent::TurnStarted {
                turn_id: turn_id.clone(),
            },
        );
        assert!(matches!(
            parent_monitor.recv().await.unwrap(),
            SessionBusEvent::TurnStarted { .. }
        ));

        // Closing the child tab drops this read-only response body. It must not
        // touch the daemon-owned turn or the parent's independent watch.
        drop(body);
        let mut detached = false;
        for _ in 0..40 {
            if state.live_observer_streams() == 0 {
                detached = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(detached, "closed child observer did not release its stream");
        assert!(state.is_turn_active(&child.id));
        assert!(state.knows_turn(&child.id, &turn_id));
        assert!(!cancel.is_cancelled());

        session_events::publish(
            &child.id,
            SessionBusEvent::TurnFinished {
                reason: "delegated result collected".into(),
                token_state: None,
            },
        );
        let terminal = tokio::time::timeout(Duration::from_secs(1), parent_monitor.recv())
            .await
            .expect("parent monitor stayed connected")
            .unwrap();
        assert!(matches!(terminal, SessionBusEvent::TurnFinished { .. }));
        assert!(!cancel.is_cancelled());
        drop(guard);
    }

    /// A watch on a session that does not exist is refused, not an empty stream
    /// — and since this route joined the `session_reach` list it is refused as
    /// **403, not 404**, for an unproven caller.
    ///
    /// ⚠ **That change is the control working, not a regression.** `Unreadable`
    /// is refused identically to `Private` by design (`routes::session_reach`,
    /// "the blast radius is wider than private chats"): a 404 here would answer
    /// "no such chat" for ids that do not exist and 403 for ids that do, which is
    /// precisely the per-id existence oracle the refusal is worded to close. A
    /// caller holding the user-action proof still gets the honest 404, and
    /// `session_reach`'s own tests hold that half.
    ///
    /// Note the ordering this is NOT allowed to change: the gate runs before
    /// `observe_session_events` subscribes to the bus, and the subscription still
    /// precedes `get_session` so no event falls between snapshot and stream.
    #[tokio::test]
    async fn observing_an_unknown_session_is_refused() {
        use tower::ServiceExt;
        let state = crate::state::AppState::new().await.unwrap();
        let response = routes(state.clone())
            .oneshot(
                axum::http::Request::get("/sessions/does-not-exist/events")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    }
}
