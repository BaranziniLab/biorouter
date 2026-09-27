use anyhow::Result;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::time::timeout;
use tracing::warn;
use uuid::Uuid;

use crate::conversation::message::{Message, MessageContent};

struct PendingRequest {
    /// The session allowed to answer. An unattributed request starts as `None`
    /// and is bound to the first session that drains its unscoped card.
    session_id: Option<String>,
    /// `Some(data)` = the user answered; `None` = the user (or an
    /// unattended run) cancelled the elicitation. Carried as an `Option` so
    /// a cancellation unparks the waiting tool call with a **model-visible**
    /// `ElicitationAction::Cancel` instead of leaving it to time out.
    response_tx: Option<tokio::sync::oneshot::Sender<Option<Value>>>,
}

/// One delivery scope's request queue plus its wake-up.
///
/// `notify` is signalled (`notify_one`, which stores a permit when nobody is
/// parked yet) every time a message is pushed, so a consumer can *race* "an
/// elicitation arrived" against other work without holding a queue lock
/// across an await (#40). The tool call that raises an elicitation is itself
/// parked inside its batch until the request is answered or cancelled — a
/// consumer that only drained the queue after its tool stream yielded would
/// therefore never see the request before the elicitation timeout.
#[derive(Default)]
struct ScopeState {
    queue: std::sync::Mutex<VecDeque<Message>>,
    notify: tokio::sync::Notify,
}

impl ScopeState {
    fn push(&self, message: Message) {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(message);
        // `notify_one` stores a permit when nobody is waiting yet, so the
        // wake-up cannot be lost to a race with the consumer re-entering its
        // select loop.
        self.notify.notify_one();
    }

    fn drain(&self) -> Vec<Message> {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain(..)
            .collect()
    }
}

enum ElicitationSender {
    Legacy(tokio::sync::oneshot::Sender<Option<Value>>),
    Pending(tokio::sync::oneshot::Sender<crate::pending_user_action::UserActionOutcome>),
}

/// Exclusive ownership of an exact-session ordinary input waiter. Dropping it
/// interrupts the waiter; it cannot resolve an approval or a credential ask.
pub struct OrdinaryElicitationClaim {
    session_id: String,
    sender: ElicitationSender,
}
impl OrdinaryElicitationClaim {
    pub(crate) fn pending(
        session_id: String,
        sender: tokio::sync::oneshot::Sender<crate::pending_user_action::UserActionOutcome>,
    ) -> Self {
        Self {
            session_id,
            sender: ElicitationSender::Pending(sender),
        }
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn is_closed(&self) -> bool {
        match &self.sender {
            ElicitationSender::Legacy(sender) => sender.is_closed(),
            ElicitationSender::Pending(sender) => sender.is_closed(),
        }
    }
    pub fn deliver(self, data: Option<Value>) -> crate::pending_user_action::ResolveOutcome {
        use crate::pending_user_action::{ResolveOutcome, UserActionOutcome};
        let delivered = match self.sender {
            ElicitationSender::Legacy(sender) => sender.send(data).is_ok(),
            ElicitationSender::Pending(sender) => sender
                .send(match data {
                    Some(data) => UserActionOutcome::Provided { data },
                    None => UserActionOutcome::Cancelled,
                })
                .is_ok(),
        };
        if delivered {
            ResolveOutcome::Delivered
        } else {
            ResolveOutcome::Unknown
        }
    }
}

pub struct ActionRequiredManager {
    pending: Arc<Mutex<HashMap<String, PendingRequest>>>,
    /// Request queues keyed by the originating session id. The manager is
    /// process-global, but delivery must not be: under the daemon, several
    /// sessions run their batch loops concurrently, and with a single shared
    /// queue ANOTHER session's loop could win the wake-up race, drain the
    /// request, and persist/yield the elicitation prompt under its own
    /// session id — leaking the prompt to the wrong session's UI (#40).
    /// Entries are never removed: a consumer may hold the scope's `Notify`
    /// across awaits, and replacing it would lose wake-ups. The map is
    /// bounded by the number of distinct sessions that raise or await an
    /// elicitation over the process lifetime, each entry a few hundred bytes.
    scoped: std::sync::Mutex<HashMap<String, Arc<ScopeState>>>,
    /// Requests whose originating session could not be determined (no
    /// in-flight tool call to attribute, or a shared pooled client running
    /// tool calls for several sessions at once). Deliverable by ANY session's
    /// loop — the pre-scoping behavior, kept as the fallback so such a
    /// request still surfaces somewhere instead of timing out silently.
    unscoped: ScopeState,
}

impl ActionRequiredManager {
    fn new() -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
            scoped: std::sync::Mutex::new(HashMap::new()),
            unscoped: ScopeState::default(),
        }
    }

    /// The (created-on-demand) scope for a session id.
    fn scope(&self, session_id: &str) -> Arc<ScopeState> {
        self.scoped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(session_id.to_string())
            .or_default()
            .clone()
    }

    pub fn global() -> &'static Self {
        static INSTANCE: once_cell::sync::Lazy<ActionRequiredManager> =
            once_cell::sync::Lazy::new(ActionRequiredManager::new);
        &INSTANCE
    }

    /// Queue a request card for `session_id`'s agent loop to drain, and wake it.
    ///
    /// The publication half of [`Self::request_and_wait`], split out so
    /// [`crate::pending_user_action`] can publish a card whose *resolution* it
    /// owns — a tool-permission approval, or a credential ask whose answer must
    /// never come back through the conversation transport at all.
    ///
    /// One queue, deliberately. This is the only wake seam an agent loop
    /// watches ([`Self::request_arrived`]) and the only drain that persists a
    /// request under the right session id; a second channel would make every
    /// loop race two notifies, which is the exact shape that turned #40 into a
    /// cross-session prompt leak.
    ///
    /// `None` queues it unscoped — deliverable by any session's loop — for the
    /// callers that genuinely cannot attribute the request.
    pub fn publish(&self, session_id: Option<&str>, message: Message) {
        match session_id {
            Some(session_id) => self.scope(session_id).push(message),
            None => self.unscoped.push(message),
        }
    }

    /// Park until the user answers (`Ok(Some(data))`), explicitly cancels
    /// (`Ok(None)`), the timeout elapses, or the channel dies.
    ///
    /// `session_id` is the delivery scope: the session whose agent loop may
    /// surface this request (#40). `None` queues it unscoped — deliverable by
    /// any session's loop — for the callers that genuinely cannot attribute
    /// the elicitation (see [`ActionRequiredManager::unscoped`]).
    pub async fn request_and_wait(
        &self,
        message: String,
        schema: Value,
        timeout_duration: Duration,
        session_id: Option<&str>,
    ) -> Result<Option<Value>> {
        // ⚠ The unbounded half of the same refusal `PendingUserActions::park`
        // makes. Any MCP server can reach here through `create_elicitation`, so
        // the set of calls that might ask a question is not enumerable — which
        // is precisely why the rule is about the CALLER's surface rather than a
        // list of tools. `Ok(None)` is already this function's cancellation
        // shape, so no caller learns a new outcome.
        if crate::user_surface::no_human_surface() {
            return Ok(None);
        }

        let id = Uuid::new_v4().to_string();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let pending_request = PendingRequest {
            session_id: session_id.map(str::to_string),
            response_tx: Some(tx),
        };

        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.clone(), pending_request);

        let action_required_message = Message::assistant().with_content(
            MessageContent::action_required_elicitation(id.clone(), message, schema),
        );

        self.publish(session_id, action_required_message);

        let result = match timeout(timeout_duration, rx).await {
            Ok(Ok(user_data)) => Ok(user_data),
            Ok(Err(_)) => {
                warn!("Response channel closed for request: {}", id);
                Err(anyhow::anyhow!("Response channel closed"))
            }
            Err(_) => {
                warn!("Timeout waiting for response: {}", id);
                Err(anyhow::anyhow!("Timeout waiting for user response"))
            }
        };

        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);

        result
    }

    /// Resolves as soon as an elicitation request may be waiting for
    /// `session_id` — one queued in that session's scope or in the unscoped
    /// fallback (one wake per queued request; a wake whose queue was already
    /// drained by an unconditional [`Self::drain_requests`] is a harmless
    /// no-op for the caller). This is the seam that lets an agent loop whose
    /// entire tool batch is *parked on the elicitation itself* surface the
    /// request — and lets a headless run cancel it — instead of waiting out
    /// the elicitation timeout (#40). A request scoped to a DIFFERENT
    /// session never resolves this: only that session's own loop is woken,
    /// so a concurrent session cannot drain the prompt into its own UI. Does
    /// not consume the queue: pair with [`Self::drain_requests`].
    pub async fn request_arrived(&self, session_id: &str) {
        let scope = self.scope(session_id);
        tokio::select! {
            () = scope.notify.notified() => {}
            () = self.unscoped.notify.notified() => {}
        }
    }

    /// Remove and return every request currently deliverable to `session_id`:
    /// its own scope's queue plus the unscoped fallback. Requests scoped to
    /// other sessions are untouched — their queues (and armed notifies) stay
    /// intact for their own loops.
    pub fn drain_requests(&self, session_id: &str) -> Vec<Message> {
        let mut messages = self.scope(session_id).drain();
        let unscoped = self.unscoped.drain();
        if !unscoped.is_empty() {
            let ids: Vec<String> = unscoped.iter().filter_map(action_required_id).collect();
            {
                let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
                for id in &ids {
                    if let Some(request) = pending.get_mut(id) {
                        if request.session_id.is_none() {
                            request.session_id = Some(session_id.to_string());
                        }
                    }
                }
            }
            for id in ids {
                crate::pending_user_action::PendingUserActions::global()
                    .claim_unscoped_for_session(&id, session_id);
            }
        }
        messages.extend(unscoped);
        messages
    }

    #[cfg(test)]
    pub(crate) fn has_unscoped_tool_confirmation(&self, expected_tool_name: &str) -> bool {
        self.unscoped
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|message| {
                message.content.iter().any(|content| {
                    matches!(
                        content,
                        MessageContent::ActionRequired(action)
                            if matches!(
                                &action.data,
                                crate::conversation::message::ActionRequiredData::ToolConfirmation {
                                    tool_name,
                                    ..
                                } if tool_name.as_str() == expected_tool_name
                            )
                    )
                })
            })
    }

    /// Claim exactly once before persisting an answer, without holding a mutex
    /// during persistence or acquiring the agent's active-turn lock.
    pub fn claim_ordinary_elicitation(
        &self,
        session_id: &str,
        request_id: &str,
    ) -> Option<OrdinaryElicitationClaim> {
        let sender = {
            let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            match pending.get_mut(request_id) {
                Some(request) if request.session_id.as_deref() == Some(session_id) => {
                    Some(request.response_tx.take()?)
                }
                Some(_) => return None,
                None => None,
            }
        };
        match sender {
            Some(sender) => Some(OrdinaryElicitationClaim {
                session_id: session_id.into(),
                sender: ElicitationSender::Legacy(sender),
            }),
            None => crate::pending_user_action::PendingUserActions::global()
                .claim_elicitation_in_session(session_id, request_id),
        }
    }

    pub fn resolve_ordinary_elicitation(
        &self,
        session_id: &str,
        request_id: &str,
        data: Option<Value>,
    ) -> crate::pending_user_action::ResolveOutcome {
        self.claim_ordinary_elicitation(session_id, request_id)
            .map_or(
                crate::pending_user_action::ResolveOutcome::Unknown,
                |claim| claim.deliver(data),
            )
    }

    pub async fn submit_response(
        &self,
        session_id: &str,
        request_id: String,
        user_data: Value,
    ) -> Result<()> {
        self.deliver(session_id, request_id, Some(user_data)).await
    }

    /// Cancel a pending elicitation: the parked [`Self::request_and_wait`]
    /// returns `Ok(None)`, which the MCP client maps to
    /// `ElicitationAction::Cancel` — a model-visible outcome, unlike letting
    /// the request sit until its timeout. Used by unattended runs
    /// (non-interactive / no TTY) that can never collect the input (#40).
    pub async fn submit_cancellation(&self, session_id: &str, request_id: String) -> Result<()> {
        self.deliver(session_id, request_id, None).await
    }

    async fn deliver(
        &self,
        session_id: &str,
        request_id: String,
        outcome: Option<Value>,
    ) -> Result<()> {
        let sender = {
            let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            pending.get_mut(&request_id).and_then(|request| {
                if request.session_id.as_deref() == Some(session_id) {
                    request.response_tx.take()
                } else {
                    None
                }
            })
        };
        // #107: an elicitation parked through `pending_user_action` is not in
        // this map — its rendezvous lives in that registry so a bridged call,
        // which has no `Agent` and no `McpClient`, can raise one. The existing
        // routes and the CLI answer both kinds through this one function, so
        // the fallthrough is what stops the newer mechanism needing a second
        // route that a client would have to know to choose between.
        let Some(tx) = sender else {
            use crate::pending_user_action::{
                PendingUserActions, ResolveOutcome, UserActionOutcome,
            };
            let relayed = match outcome {
                Some(data) => UserActionOutcome::Provided { data },
                None => UserActionOutcome::Cancelled,
            };
            // ⚠ `unproven()`, and the arm below is not dead code. This relay
            // only ever carries `Provided`/`Cancelled`, neither of which trips
            // the gate — but it is reachable from any user message carrying an
            // elicitation response, INCLUDING one typed into an app page, so
            // the honest word here is the one that assumes nothing.
            return match PendingUserActions::global().resolve_in_session(
                session_id,
                &request_id,
                relayed,
                crate::pending_user_action::DecisionAuthority::unproven(),
            ) {
                ResolveOutcome::Delivered => Ok(()),
                ResolveOutcome::Rejected => Err(anyhow::anyhow!(
                    "Request {} does not accept that answer",
                    request_id
                )),
                ResolveOutcome::Unproven => Err(anyhow::anyhow!(
                    "Request {request_id} needs a person's approval"
                )),
                ResolveOutcome::Unknown => Err(anyhow::anyhow!("Request not found")),
            };
        };

        if tx.send(outcome).is_err() {
            warn!("Failed to send response through oneshot channel");
        }

        Ok(())
    }
}

fn action_required_id(message: &Message) -> Option<String> {
    use crate::conversation::message::ActionRequiredData;

    message.content.iter().find_map(|content| match content {
        MessageContent::ActionRequired(action) => match &action.data {
            ActionRequiredData::ToolConfirmation { id, .. }
            | ActionRequiredData::Elicitation { id, .. }
            | ActionRequiredData::SecretRequest { id, .. } => Some(id.clone()),
            ActionRequiredData::ElicitationResponse { .. } => None,
        },
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::ActionRequiredData;

    /// Extract the elicitation id a request message carries.
    fn elicitation_id(message: &Message) -> Option<String> {
        message.content.iter().find_map(|content| match content {
            MessageContent::ActionRequired(action) => match &action.data {
                ActionRequiredData::Elicitation { id, .. } => Some(id.clone()),
                _ => None,
            },
            _ => None,
        })
    }

    /// Poll the manager until the request queued for `session_id` is
    /// drainable, and return the id it was minted with. The enqueue happens
    /// inside a spawned `request_and_wait`, so the first drain can race it.
    async fn next_request_id(manager: &ActionRequiredManager, session_id: &str) -> String {
        timeout(Duration::from_secs(5), async {
            loop {
                if let Some(id) = manager
                    .drain_requests(session_id)
                    .iter()
                    .find_map(elicitation_id)
                {
                    return id;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("an elicitation request message")
    }

    #[tokio::test]
    async fn submit_response_unparks_with_the_data() {
        let manager = std::sync::Arc::new(ActionRequiredManager::new());
        let waiter = {
            let manager = manager.clone();
            tokio::spawn(async move {
                manager
                    .request_and_wait(
                        "Need input".to_string(),
                        serde_json::json!({}),
                        Duration::from_secs(5),
                        Some("sess-1"),
                    )
                    .await
            })
        };
        let id = next_request_id(&manager, "sess-1").await;
        manager
            .submit_response("sess-1", id, serde_json::json!({"name": "ada"}))
            .await
            .unwrap();
        let outcome = waiter.await.unwrap().unwrap();
        assert_eq!(outcome, Some(serde_json::json!({"name": "ada"})));
    }

    /// #40: a cancellation unparks the waiter with `Ok(None)` — the outcome
    /// the MCP client maps to `ElicitationAction::Cancel`, so an unattended
    /// run resolves the tool call visibly instead of stalling to timeout.
    #[tokio::test]
    async fn submit_cancellation_unparks_with_none() {
        let manager = std::sync::Arc::new(ActionRequiredManager::new());
        let waiter = {
            let manager = manager.clone();
            tokio::spawn(async move {
                manager
                    .request_and_wait(
                        "Need input".to_string(),
                        serde_json::json!({}),
                        Duration::from_secs(5),
                        Some("sess-1"),
                    )
                    .await
            })
        };
        let id = next_request_id(&manager, "sess-1").await;
        manager.submit_cancellation("sess-1", id).await.unwrap();
        let outcome = waiter.await.unwrap().unwrap();
        assert_eq!(outcome, None, "cancellation must be Ok(None), not an error");
    }

    /// #40: `request_arrived` must resolve — and the cancel must land — while
    /// the requesting tool call is still parked in `request_and_wait`, i.e.
    /// WITHOUT any other event feeding the consumer. The generous
    /// `request_and_wait` timeout stands in for the production 300 s one: if
    /// the wake-up depended on the waiter finishing first, the short
    /// `timeout()`s here would fire long before it, failing the test.
    #[tokio::test]
    async fn request_arrived_wakes_and_cancel_resolves_while_the_tool_is_parked() {
        let manager = std::sync::Arc::new(ActionRequiredManager::new());
        let waiter = {
            let manager = manager.clone();
            tokio::spawn(async move {
                manager
                    .request_and_wait(
                        "Need input".to_string(),
                        serde_json::json!({}),
                        Duration::from_secs(300),
                        Some("sess-1"),
                    )
                    .await
            })
        };
        // The notify must fire from the request itself — nobody has consumed
        // the queue and no other stream item will ever arrive.
        timeout(Duration::from_secs(2), manager.request_arrived("sess-1"))
            .await
            .expect("request_arrived must wake without another stream item");
        let id = timeout(Duration::from_secs(2), next_request_id(&manager, "sess-1"))
            .await
            .expect("the queued request must be drainable after the wake");
        manager.submit_cancellation("sess-1", id).await.unwrap();
        let outcome = timeout(Duration::from_secs(2), waiter)
            .await
            .expect("cancel must unpark the waiter promptly, not at its timeout")
            .unwrap()
            .unwrap();
        assert_eq!(outcome, None, "cancellation resolves as Ok(None)");
    }

    /// #40 round 3: the manager is process-global, so with concurrent daemon
    /// sessions a request scoped to session B must NEVER wake or be drained
    /// by session A's loop — that is exactly the leak where B's elicitation
    /// prompt was persisted and yielded under A's session id.
    #[tokio::test]
    async fn scoped_request_never_wakes_or_drains_a_foreign_session() {
        let manager = std::sync::Arc::new(ActionRequiredManager::new());
        let waiter = {
            let manager = manager.clone();
            tokio::spawn(async move {
                manager
                    .request_and_wait(
                        "Need B's input".to_string(),
                        serde_json::json!({}),
                        Duration::from_secs(300),
                        Some("sess-b"),
                    )
                    .await
            })
        };

        // B's own loop is woken...
        timeout(Duration::from_secs(2), manager.request_arrived("sess-b"))
            .await
            .expect("the owning session must be woken");
        // ...while A's is not (its scope and the unscoped fallback are both
        // empty, so this must still be parked when the timeout fires)...
        assert!(
            timeout(
                Duration::from_millis(200),
                manager.request_arrived("sess-a")
            )
            .await
            .is_err(),
            "a foreign session must not be woken by B's request"
        );
        // ...and even an unconditional drain by A returns nothing.
        assert!(
            manager.drain_requests("sess-a").is_empty(),
            "a foreign session must not drain B's request"
        );

        // B's request is still intact for B, and the cancel path works.
        let id = next_request_id(&manager, "sess-b").await;
        manager.submit_cancellation("sess-b", id).await.unwrap();
        let outcome = timeout(Duration::from_secs(2), waiter)
            .await
            .expect("cancel must unpark the waiter")
            .unwrap()
            .unwrap();
        assert_eq!(outcome, None);
    }

    /// An UNSCOPED request (no attributable session) keeps the pre-scoping
    /// behavior: any session's loop is woken and may drain it, so it still
    /// surfaces somewhere instead of silently timing out.
    #[tokio::test]
    async fn unscoped_request_wakes_and_drains_for_any_session() {
        let manager = std::sync::Arc::new(ActionRequiredManager::new());
        let _waiter = {
            let manager = manager.clone();
            tokio::spawn(async move {
                manager
                    .request_and_wait(
                        "Need someone's input".to_string(),
                        serde_json::json!({}),
                        Duration::from_secs(300),
                        None,
                    )
                    .await
            })
        };

        timeout(Duration::from_secs(2), manager.request_arrived("any-sess"))
            .await
            .expect("an unscoped request must wake any session's loop");
        let id = next_request_id(&manager, "any-sess").await;
        manager.submit_cancellation("any-sess", id).await.unwrap();
    }

    #[tokio::test]
    async fn delivering_to_an_unknown_id_errors() {
        let manager = ActionRequiredManager::new();
        assert!(manager
            .submit_cancellation("sess-1", "no-such-request".to_string())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn foreign_response_is_indistinguishable_and_leaves_the_waiter_parked() {
        let manager = std::sync::Arc::new(ActionRequiredManager::new());
        let waiter = {
            let manager = Arc::clone(&manager);
            tokio::spawn(async move {
                manager
                    .request_and_wait(
                        "Need owner's input".to_string(),
                        serde_json::json!({}),
                        Duration::from_secs(5),
                        Some("sess-owner"),
                    )
                    .await
            })
        };
        let id = next_request_id(&manager, "sess-owner").await;

        let foreign = manager
            .submit_response(
                "sess-foreign",
                id.clone(),
                serde_json::json!({"wrong": true}),
            )
            .await
            .expect_err("a foreign session must not resolve the request")
            .to_string();
        let unknown = manager
            .submit_response(
                "sess-foreign",
                "no-such-request".to_string(),
                serde_json::json!({"wrong": true}),
            )
            .await
            .expect_err("an unknown id must stay unknown")
            .to_string();
        assert_eq!(foreign, unknown, "scope mismatches must not reveal the id");

        manager
            .submit_response("sess-owner", id, serde_json::json!({"right": true}))
            .await
            .expect("the owning session still resolves the parked waiter");
        assert_eq!(
            waiter.await.unwrap().unwrap(),
            Some(serde_json::json!({"right": true}))
        );
    }
}
