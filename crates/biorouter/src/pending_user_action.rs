//! One parked call, one human decision — the rendezvous a tool call uses when
//! it cannot proceed without a person (#107).
//!
//! # Why this exists
//!
//! Biorouter already had two park/resume mechanics and they did not compose:
//!
//! * **Tool permission** — [`crate::agents::Agent::register_confirmation`] hands
//!   the reply loop a `oneshot`, the loop yields a confirmation card, and
//!   `POST /action-required/tool-confirmation` resolves it. It lives on the
//!   `Agent`, so only code holding an `&Agent` can raise one.
//! * **MCP elicitation** — [`crate::action_required_manager`] queues a request
//!   message per session and parks the caller. It is process-global, so anyone
//!   can raise one, but it can only ask for *data*, never for a permission
//!   decision, and it has no way to collect a value without that value passing
//!   through the conversation transport.
//!
//! A bridged coding-agent tool call needs both halves and can reach neither: it
//! runs on an axum task inside `POST /tool_bridge/{nonce}`, with a
//! [`crate::providers::coding_agent::bridge::BridgeGrant`] snapshot and no
//! `Agent` handle at all. Until this module existed the bridge answered
//! `needs_approval` with a refusal, and the model dutifully asked the user in
//! prose — a question nothing could answer, because no request id had ever been
//! minted.
//!
//! So this is the one registry both mechanics now route through, and the one
//! type a new surface should reach for rather than inventing a third.
//!
//! # The shape
//!
//! ```text
//! park(session, owner, request)  ->  PendingUserAction   (publishes the card)
//!        .wait(ttl, cancel)      ->  UserActionOutcome    (approve/deny/data/
//!                                                          secrets/cancel/timeout)
//! resolve_in_session(session, id, outcome, DecisionAuthority::unproven()) <- the session surface that showed it
//! ```
//!
//! Publication deliberately reuses [`crate::action_required_manager`]'s
//! session-scoped queue rather than adding a second one. That queue already has
//! the only wake seam an agent loop watches (`request_arrived`) and the only
//! drain that persists a request under the right session id, and a second
//! channel would mean a loop had to race two notifies — the exact shape that
//! made #40 a cross-session prompt leak.
//!
//! # Resolution is keyed per request, and that is a safety property
//!
//! Every park mints its own uuid and its own `oneshot`. A decision for an id
//! nobody is waiting on is dropped and reported as [`ResolveOutcome::Unknown`],
//! never applied to whichever call happens to be parked now. Two bridged calls
//! from the same child, in flight at the same time (both CLIs issue parallel
//! `tools/call`), therefore cannot resolve each other — the property BR-62
//! established for the agent's own path, extended to this one.
//!
//! # Secret-safe resolution
//!
//! [`UserActionRequest::Secrets`] asks a *trusted surface* to collect values and
//! write them straight to the keyring. The parked caller learns only which keys
//! were configured, because that is the only thing
//! [`UserActionOutcome::SecretsConfigured`] can carry — there is no field for a
//! value, the published card carries only key names and labels, and
//! [`PendingUserActions::resolve`] **refuses** a data-bearing outcome for a
//! secrets request rather than letting a mis-wired route smuggle one into the
//! transcript. A secret that never enters the conversation transport cannot be
//! persisted into a session row, replayed into a later prompt, or flattened into
//! a child agent's transcript.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::debug;
use uuid::Uuid;

use rmcp::model::JsonObject;

use crate::conversation::message::{Message, MessageContent, SecretDestination, SecretKeyRequest};
use crate::conversation::tool_preview::ToolPreview;
use crate::permission::tool_risk::ToolRisk;
use crate::permission::Permission;

/// A tool call the permission inspector routed to `needs_approval`.
#[derive(Debug, Clone)]
pub struct ToolApprovalRequest {
    /// The tool as the user will see it named on the card. The bridge strips no
    /// prefix here: a bridged call's name is the one the child asked for, and a
    /// card naming something else would be a card about a different call.
    pub tool_name: String,
    pub arguments: JsonObject,
    /// Why approval is being asked for, when the inspector said.
    pub prompt: Option<String>,
    /// BR-63's risk grade, so the card can say *how* dangerous the call is.
    pub risk: Option<ToolRisk>,
    /// BR-63's preview — the resolved command, the diff — so the decision is
    /// informed rather than a name and a shrug.
    pub preview: Option<ToolPreview>,
    /// The answering HTTP request must carry the desktop's proof that a person
    /// clicked the card. Use this for install/delete and other authorization
    /// decisions a model must never be able to approve through daemon HTTP.
    pub requires_user_proof: bool,
}

/// An ordinary MCP elicitation: free-form data described by a JSON schema.
#[derive(Debug, Clone)]
pub struct ElicitationRequest {
    pub message: String,
    pub requested_schema: Value,
}

/// Values that must never enter the conversation transport.
///
/// The surface that answers this writes the values straight to their
/// destination (the OS credential store, an extension's env) and resolves with
/// [`UserActionOutcome::SecretsConfigured`], which carries key *names* only.
#[derive(Debug, Clone)]
pub struct SecretsRequest {
    /// What the values are for, in the user's terms.
    pub prompt: String,
    /// The keys to collect. Order is the order the card should show them in.
    pub keys: Vec<SecretKeyRequest>,
    /// Where the trusted surface must put them.
    pub destination: SecretDestination,
}

/// What a parked call is asking a human for.
#[derive(Debug, Clone)]
pub enum UserActionRequest {
    ToolApproval(ToolApprovalRequest),
    Elicitation(ElicitationRequest),
    Secrets(SecretsRequest),
}

impl UserActionRequest {
    /// A short label for logs and refusal text. Never includes arguments, a
    /// schema, or anything from a secrets request beyond its key names.
    pub fn describe(&self) -> String {
        match self {
            Self::ToolApproval(r) => format!("approval for `{}`", r.tool_name),
            Self::Elicitation(_) => "input".to_string(),
            Self::Secrets(r) => format!("{} credential(s)", r.keys.len()),
        }
    }

    /// Whether an outcome is one this request could legitimately produce.
    ///
    /// The load-bearing case is [`UserActionRequest::Secrets`] refusing
    /// [`UserActionOutcome::Provided`]: `Provided` carries a `Value`, and a
    /// route that answered a secrets card with one would put the credential on
    /// the same path every other tool result takes — persisted to the session
    /// row, replayed into the next prompt, flattened into a child agent's
    /// transcript. Refusing it here means the guarantee holds for surfaces this
    /// module has never heard of.
    fn accepts(&self, outcome: &UserActionOutcome) -> bool {
        matches!(
            (self, outcome),
            // Every request can end without an answer.
            (
                _,
                UserActionOutcome::Cancelled
                    | UserActionOutcome::TimedOut
                    | UserActionOutcome::Failed { .. }
            ) | (
                Self::ToolApproval(_),
                UserActionOutcome::Approved { .. } | UserActionOutcome::Denied { .. }
            ) | (Self::Elicitation(_), UserActionOutcome::Provided { .. })
                | (
                    Self::Secrets(_),
                    UserActionOutcome::SecretsConfigured { .. }
                )
        )
    }
}

/// How a parked call was released.
///
/// Every variant is a *decision*, including the three that are not the user's:
/// a caller matches on this and always has something to tell the model, which is
/// the difference between this and the timeout error it replaced.
#[derive(Debug, Clone, PartialEq)]
pub enum UserActionOutcome {
    /// The user allowed the call. `permission` distinguishes a one-off from an
    /// `AlwaysAllow` the caller may want to record.
    Approved { permission: Permission },
    /// The user refused. Also carries the flavour (`DenyOnce` / `AlwaysDeny`).
    Denied { permission: Permission },
    /// An elicitation was answered.
    Provided { data: Value },
    /// A secrets request was satisfied. **Names only, by construction** — see
    /// the module header.
    SecretsConfigured { configured_keys: Vec<String> },
    /// The user dismissed it, the turn was cancelled, or an unattended run
    /// could never have collected an answer.
    Cancelled,
    /// The time-to-live elapsed with nobody answering.
    TimedOut,
    /// The request could not be put to a human at all.
    Failed { reason: String },
}

impl UserActionOutcome {
    /// Whether the parked call may now proceed.
    pub fn is_allowed(&self) -> bool {
        matches!(
            self,
            Self::Approved { .. } | Self::Provided { .. } | Self::SecretsConfigured { .. }
        )
    }

    /// A sentence for the caller to hand back to whatever asked, safe to show a
    /// model. Deliberately never suggests that a plain chat message can approve
    /// anything: on the bridged path the request id is gone by the time this is
    /// read, so an instruction to "ask the user to approve" would be asking for
    /// an answer with nowhere to land (#107).
    pub fn refusal_detail(&self) -> &'static str {
        match self {
            Self::Approved { .. } | Self::Provided { .. } | Self::SecretsConfigured { .. } => {
                "was allowed"
            }
            Self::Denied { .. } => "was refused by the user",
            Self::Cancelled => "was cancelled before anyone answered it",
            Self::TimedOut => "expired before anyone answered it",
            Self::Failed { .. } => "could not be put to a person",
        }
    }
}

/// What [`PendingUserActions::resolve`] did with a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// A live parked caller took it.
    Delivered,
    /// Nothing is waiting on that id: a double-click, a card answered after the
    /// turn ended, a stale client. Dropped, never re-aimed at another call.
    Unknown,
    /// The id exists but this outcome is not one that request can produce —
    /// today, a data-bearing answer to a secrets card. The caller stays parked;
    /// the surface has a bug.
    Rejected,
    /// The id exists and the outcome is one it accepts, but the answering
    /// surface cannot prove a PERSON decided, and this approval requires that.
    /// The caller stays parked.
    ///
    /// ⚠ Distinct from [`Self::Unknown`] on purpose. A door that cannot tell
    /// "nobody is waiting" from "you may not answer this" cannot follow up
    /// correctly — and the follow-up matters: an approval left parked after a
    /// refusal sits for its full time-to-live with no card anywhere.
    Unproven,
}

/// Who is answering, as far as the process can tell.
///
/// ⚠ **A newtype with a private field, not an enum.** The property this type
/// exists to have is that the privileged answers can only be minted at a small,
/// auditable set of call sites — and an enum variant is a literal any crate can
/// write, which would make that property unenforceable. A foreign crate must go
/// through a named constructor here, the same shape `UserKbTierChange` uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecisionAuthority(Authority);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Authority {
    /// `auth::user_action_proof` returned `Proven` for THIS request.
    Proven,
    /// A person answered at a surface this process owns and a model cannot
    /// author — the terminal prompt, the TUI modal.
    LocalHuman,
    /// Everything else, and the default reading of an unknown door: page
    /// JavaScript an agent wrote, an agent deciding for an agent, a transport
    /// with no proof channel at all.
    Unproven,
}

impl DecisionAuthority {
    /// Mint ONLY after `auth::user_action_proof` returned `Proven` for the
    /// request being answered.
    pub fn from_user_action_proof() -> Self {
        Self(Authority::Proven)
    }

    /// A person acted at a surface this process owns.
    pub fn from_local_human_surface() -> Self {
        Self(Authority::LocalHuman)
    }

    /// No preconditions. The honest answer for every door that cannot tell a
    /// person from a model.
    pub fn unproven() -> Self {
        Self(Authority::Unproven)
    }

    /// Stand in for a proven surface, in a test that is exercising something
    /// else and simply needs the desktop dialog's answer to land.
    ///
    /// ⚠ `#[cfg(test)]` is what keeps this honest: the compiler, not a
    /// convention, is what stops it appearing in production code — so the audit
    /// of the two privileged constructors stays a statement about the real
    /// doors. A test that is exercising the GATE ITSELF must use the production
    /// constructors instead, and the tests in this module do.
    #[cfg(test)]
    pub(crate) fn for_test_proven() -> Self {
        Self(Authority::Proven)
    }

    fn may_grant_proof_backed(self) -> bool {
        matches!(self.0, Authority::Proven | Authority::LocalHuman)
    }
}

struct Entry {
    session_id: Option<String>,
    owner: Option<String>,
    request: UserActionRequest,
    tx: Option<oneshot::Sender<UserActionOutcome>>,
    /// Other sessions this same card was published into, and from which a
    /// decision is therefore accepted (D10).
    ///
    /// One entry today: the conversation that delegated the work, for a card
    /// raised inside a sub-agent. `session_id` is still the card's **home** —
    /// the session that owns the parked call and whose tab always shows it —
    /// and this list only widens *who may answer*, never who may raise.
    ///
    /// ⚠ Publishing and recording are one operation
    /// ([`PendingUserAction::also_surface_in`]) because each half alone is a
    /// bug with its own symptom: a recorded surface that was never published is
    /// an approval nobody can see, and a published card that was never recorded
    /// is a card the user clicks and watches do nothing —
    /// [`Self::answerable_in`] answers `Unknown` for every session but this
    /// one.
    escalated_to: Vec<String>,
}

impl Entry {
    /// May a decision posted from `session_id` release this call?
    ///
    /// The card's home, plus any session it was escalated into. Deliberately
    /// NOT "any session": #40's rule is that an authorization belongs to the
    /// exact surfaces that showed it, and a card another conversation could
    /// answer merely by knowing the id is a cross-session approval leak.
    fn answerable_in(&self, session_id: &str) -> bool {
        self.session_id.as_deref() == Some(session_id)
            || self.escalated_to.iter().any(|surface| surface == session_id)
    }
}

/// The process-global registry of parked user actions.
///
/// Process-global for the same reason [`crate::action_required_manager`] is:
/// the surface that answers runs on a different task from the caller that
/// parked, often in a different subsystem, and on the bridged path there is no
/// `Agent` for either of them to share.
#[derive(Default)]
pub struct PendingUserActions {
    entries: Mutex<HashMap<String, Entry>>,
}

/// Can THIS process obtain a person's proof at all?
///
/// The daemon is handed a proof-of-user key on stdin at startup and every
/// approval that sets `requires_user_proof` is answered against it. `biorouter
/// serve` spawns its daemon with `Stdio::null()` DELIBERATELY — that is the same
/// property that stops a browser session changing the model — so it holds no
/// key, and every such approval refuses forever.
///
/// A tool whose approval can never be granted must not be OFFERED. Reporting the
/// refusal honestly is necessary but not sufficient: a model that is offered an
/// installer will propose an install, and the user then meets a card whose three
/// buttons cannot work.
///
/// Defaults to `true`, and the daemon sets it from the key it actually received.
/// The direction is deliberate: an embedder that never calls this keeps today's
/// behaviour, and the cost of being wrong is a tool that is advertised and then
/// refused — never an approval that is skipped.
static USER_PROOF_AVAILABLE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

/// Published once at daemon startup, beside `install_user_action_digest`.
pub fn set_user_proof_available(available: bool) {
    USER_PROOF_AVAILABLE.store(available, std::sync::atomic::Ordering::Relaxed);
}

/// Whether a proof-backed approval can be granted in this process. Tools that
/// require one ask this before advertising themselves.
pub fn user_proof_available() -> bool {
    USER_PROOF_AVAILABLE.load(std::sync::atomic::Ordering::Relaxed)
}

impl PendingUserActions {
    /// The registry every production caller uses.
    ///
    /// An `Arc` rather than a bare `&'static` because [`PendingUserAction`]
    /// holds its registry: `park`, `wait` and `Drop` must all act on the *same*
    /// one, and a handle that assumed the global would silently split a test's
    /// standalone registry in half — parking in one and deregistering from the
    /// other.
    pub fn global() -> &'static Arc<Self> {
        static INSTANCE: once_cell::sync::Lazy<Arc<PendingUserActions>> =
            once_cell::sync::Lazy::new(|| Arc::new(PendingUserActions::default()));
        &INSTANCE
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Register `request`, publish its card to `session_id`'s queue, and return
    /// the handle to park on.
    ///
    /// Registration happens **before** publication, deliberately: a surface fast
    /// enough to answer the card before this function returns must still find a
    /// live sender. That is BR-62's ordering, and the reason it exists is that
    /// the opposite order loses exactly the decisions made by a user who was
    /// already looking at the screen.
    ///
    /// `session_id` is the delivery scope — the session whose loop may surface
    /// this. `None` queues it unscoped, deliverable by any loop, for callers
    /// that genuinely cannot attribute it.
    ///
    /// `owner` groups actions that must die together: a bridge lease passes its
    /// nonce, so [`Self::cancel_owner`] releases every call parked under a turn
    /// that has ended. `None` means only the session scope and the TTL bound it.
    pub fn park(
        self: &Arc<Self>,
        session_id: Option<&str>,
        owner: Option<&str>,
        request: UserActionRequest,
    ) -> PendingUserAction {
        let id = Uuid::new_v4().to_string();

        // ⚠ Refuse to park where there is nobody to answer. A card raised
        // beneath `POST /agent/call_tool` has no admitted turn and no stream to
        // draw on, so it would sit in a queue only the agent loop drains — for
        // its whole time-to-live, unanswerable — and the NEXT chat turn would
        // then surface it, as a question about something that happened minutes
        // ago. Registering nothing is what makes that resurrection impossible.
        //
        // The returned handle answers `Cancelled` immediately, which every
        // caller already handles: it is the same outcome a dismissal produces.
        if crate::user_surface::no_human_surface() {
            return PendingUserAction {
                id,
                request,
                rx: None,
                declined: true,
                registry: Arc::clone(self),
            };
        }

        let (tx, rx) = oneshot::channel();
        self.lock().insert(
            id.clone(),
            Entry {
                session_id: session_id.map(str::to_string),
                owner: owner.map(str::to_string),
                request: request.clone(),
                tx: Some(tx),
                escalated_to: Vec::new(),
            },
        );

        crate::action_required_manager::ActionRequiredManager::global()
            .publish(session_id, request_message(&id, &request));

        PendingUserAction {
            id,
            request,
            rx: Some(rx),
            declined: false,
            registry: Arc::clone(self),
        }
    }

    /// Deliver a decision from the exact session that owns the parked call.
    ///
    /// The scope comparison and sender take happen in one critical section. A
    /// caller that knows an id but posts it from another session therefore sees
    /// the same [`ResolveOutcome::Unknown`] as a stale id, and the real waiter
    /// remains parked for its own surface.
    pub fn resolve_in_session(
        &self,
        session_id: &str,
        id: &str,
        outcome: UserActionOutcome,
        authority: DecisionAuthority,
    ) -> ResolveOutcome {
        self.resolve_matching(id, outcome, authority, |entry| {
            entry.answerable_in(session_id)
        })
    }

    /// Resolve a credential card from the trusted, proof-of-user surface that
    /// predates session-bearing credential submissions.
    ///
    /// This intentionally cannot resolve approvals or elicitations. Keep it
    /// crate-private: an HTTP/model-facing caller must use
    /// [`Self::resolve_in_session`] instead.
    pub(crate) fn resolve_trusted_sessionless_secret(
        &self,
        id: &str,
        outcome: UserActionOutcome,
    ) -> ResolveOutcome {
        // The gate below fires only on a `ToolApproval`, so the value is inert
        // here — and this function's own door checks proof unconditionally,
        // before any branch. Naming `unproven()` keeps the privileged
        // constructors out of this file, so the audit that counts their call
        // sites stays honest.
        self.resolve_matching(id, outcome, DecisionAuthority::unproven(), |entry| {
            matches!(&entry.request, UserActionRequest::Secrets(_))
        })
    }

    fn resolve_matching(
        &self,
        id: &str,
        outcome: UserActionOutcome,
        authority: DecisionAuthority,
        scope_matches: impl FnOnce(&Entry) -> bool,
    ) -> ResolveOutcome {
        let mut entries = self.lock();
        let Some(entry) = entries.get_mut(id) else {
            debug!("No parked user action is waiting on {id}; dropping the decision");
            return ResolveOutcome::Unknown;
        };
        if !scope_matches(entry) {
            debug!("No parked user action is waiting on {id}; dropping the decision");
            return ResolveOutcome::Unknown;
        }
        if !entry.request.accepts(&outcome) {
            debug!(
                "Refusing a {outcome:?} for {id}, which asked for {}",
                entry.request.describe()
            );
            return ResolveOutcome::Rejected;
        }
        // ⚠ The gate sits HERE, above `tx.take()` and `entries.remove(id)`, and
        // inside the same critical section as the scope check. Refusing after
        // either would destroy the park while refusing the poster: the tool
        // would fail as cancelled, and the door's deny follow-up would have
        // nothing left to deny.
        //
        // Keyed on `is_allowed()` and the request's own flag, never on the
        // variant. Denials, cancellations and every approval that does not
        // require proof must pass from any surface — a gate on
        // `ToolApproval(_)` would kill every bridged coding-agent approval on
        // the desktop, and one that also caught denials would strand every
        // Reject and every socket close for the full time-to-live.
        if outcome.is_allowed()
            && !authority.may_grant_proof_backed()
            && matches!(
                &entry.request,
                UserActionRequest::ToolApproval(r) if r.requires_user_proof
            )
        {
            return ResolveOutcome::Unproven;
        }
        let Some(tx) = entry.tx.take() else {
            return ResolveOutcome::Unknown;
        };
        entries.remove(id);
        drop(entries);
        match tx.send(outcome) {
            Ok(()) => ResolveOutcome::Delivered,
            // The waiter went away between the lookup and the send — the turn
            // ended. Nothing to do and nothing to blame.
            Err(_) => ResolveOutcome::Unknown,
        }
    }

    /// Bind an unscoped card to the first session that drains it.
    ///
    /// The unscoped queue itself is single-consumer, and this claim happens
    /// before the drained message is returned to that consumer. Once claimed,
    /// every response must pass [`Self::resolve_in_session`].
    pub(crate) fn claim_unscoped_for_session(&self, id: &str, session_id: &str) {
        let mut entries = self.lock();
        let Some(entry) = entries.get_mut(id) else {
            return;
        };
        if entry.session_id.is_none() {
            entry.session_id = Some(session_id.to_string());
        }
    }

    /// Whether anything is parked on `id`. Lets a route answer a duplicate POST
    /// honestly instead of pretending it resolved something.
    pub fn is_pending(&self, id: &str) -> bool {
        self.lock().contains_key(id)
    }

    /// The ephemeral cards this session still has parked, as the messages they
    /// were published as.
    ///
    /// For a late-joining observer on the session event stream. An ephemeral
    /// card is deliberately never stored — a resolved approval must not
    /// reappear in a transcript — so a reader that attaches after the card was
    /// published has no other way to learn of it, and sees a turn that is
    /// running and apparently stuck.
    ///
    /// ⚠ Rendered through `request_message`, the same function that published
    /// the original, so the replay is byte-identical BY CONSTRUCTION rather
    /// than by a second hand-written renderer that could drift. And it reads
    /// the live map, which is what makes an already-answered card
    /// unrepresentable here: resolution removes the entry.
    ///
    /// Elicitations are excluded by `is_ephemeral_card` and that is correct —
    /// they are persisted, so an observer's conversation snapshot already
    /// carries them.
    pub fn pending_cards_for_session(&self, session_id: &str) -> Vec<Message> {
        self.lock()
            .iter()
            .filter(|(_, entry)| entry.answerable_in(session_id))
            .map(|(id, entry)| request_message(id, &entry.request))
            .filter(is_ephemeral_card)
            .collect()
    }

    /// Whether this exact session-scoped approval requires proof of a human
    /// action. A foreign session learns nothing and cannot satisfy the check.
    pub fn requires_user_proof_in_session(&self, session_id: &str, id: &str) -> bool {
        self.lock().get(id).is_some_and(|entry| {
            entry.answerable_in(session_id)
                && matches!(
                    &entry.request,
                    UserActionRequest::ToolApproval(request) if request.requires_user_proof
                )
        })
    }

    /// The request parked under `id`, if any. A surface uses this to decide
    /// *which* dialog to draw before it answers.
    pub fn peek(&self, id: &str) -> Option<UserActionRequest> {
        self.lock().get(id).map(|e| e.request.clone())
    }

    /// Cancel every action parked under `owner`, returning how many were
    /// released. Called when a bridge lease drops, so a turn that ended — normally,
    /// by panic, or by an early return — cannot leave a child blocked on an HTTP
    /// response nobody will ever answer.
    pub fn cancel_owner(&self, owner: &str) -> usize {
        self.cancel_matching(|entry| entry.owner.as_deref() == Some(owner))
    }

    /// Cancel every action parked for `session_id`. Called on session stop.
    pub fn cancel_session(&self, session_id: &str) -> usize {
        self.cancel_matching(|entry| entry.session_id.as_deref() == Some(session_id))
    }

    fn cancel_matching(&self, predicate: impl Fn(&Entry) -> bool) -> usize {
        let senders: Vec<oneshot::Sender<UserActionOutcome>> = {
            let mut entries = self.lock();
            let ids: Vec<String> = entries
                .iter()
                .filter(|(_, entry)| predicate(entry))
                .map(|(id, _)| id.clone())
                .collect();
            ids.into_iter()
                .filter_map(|id| entries.remove(&id).and_then(|mut e| e.tx.take()))
                .collect()
        };
        let mut released = 0;
        for tx in senders {
            if tx.send(UserActionOutcome::Cancelled).is_ok() {
                released += 1;
            }
        }
        released
    }

    /// Also accept a decision for `id` posted from `session_id`.
    ///
    /// `false` when nothing is parked on `id` (already answered, already gone)
    /// or when `session_id` is the card's own home, so the caller can decline
    /// to publish a second copy rather than showing a chat the same card twice.
    /// Idempotent in the session: two calls add one surface.
    fn record_escalation(&self, id: &str, session_id: &str) -> bool {
        let mut entries = self.lock();
        let Some(entry) = entries.get_mut(id) else {
            return false;
        };
        if entry.session_id.as_deref() == Some(session_id) {
            return false;
        }
        if !entry.escalated_to.iter().any(|surface| surface == session_id) {
            entry.escalated_to.push(session_id.to_string());
        }
        true
    }

    /// Drop the entry for `id` without a decision. Idempotent.
    fn forget(&self, id: &str) {
        self.lock().remove(id);
    }
}

/// A published request, and the caller's end of its rendezvous.
///
/// Dropping this without [`Self::wait`] deregisters the request, so a caller
/// that bails cannot leave a decision routable to a receiver nobody holds.
pub struct PendingUserAction {
    id: String,
    request: UserActionRequest,
    rx: Option<oneshot::Receiver<UserActionOutcome>>,
    /// Nothing was registered: `park` refused because no person could answer.
    ///
    /// Distinct from `rx: None` alone, which also means "already awaited" —
    /// and the two must produce different outcomes. A declined park is
    /// `Cancelled`, the fail-safe every caller handles; an already-awaited
    /// handle is `Failed`, which names a bug in the caller.
    declined: bool,
    registry: Arc<PendingUserActions>,
}

impl PendingUserAction {
    /// The id the answering surface posts back. Also the id on the published
    /// card.
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn request(&self) -> &UserActionRequest {
        &self.request
    }

    /// Show this same card in `session_id` as well, and accept the answer from
    /// there (D10).
    ///
    /// The card a sub-agent raises is published to the sub-agent's own session
    /// and nowhere else, so a person watching the conversation that *delegated*
    /// the work sees a tool call that has silently stopped — and a child running
    /// without a tab at all (`visible: false`, a fan-out past the four-tab cap,
    /// a terminal session) has no surface anywhere. This is how a caller hands
    /// the decision to the conversation a person is actually looking at.
    ///
    /// **What this does not do.** It does not ask the parent *agent* anything —
    /// no [`crate::agents::approval_relay`] consultation, no model-produced
    /// permission — and it does not touch proof of user: `resolve_matching`
    /// still refuses an *allow* on a proof-backed approval from a surface that
    /// cannot prove a person acted, whichever session it was posted from. The
    /// only thing that widens is which conversation's card a **person** may
    /// click.
    ///
    /// Published with [`request_message`], the same function `park` published
    /// the original with, so the two surfaces carry a byte-identical card — and
    /// therefore the same request id, which is what makes them one decision
    /// rather than two questions. `user_only`, so the watching agent never reads
    /// a question it must not answer.
    ///
    /// `false` when nothing was registered (`park` declined because no person
    /// could be asked), when the call has already been answered, or when
    /// `session_id` is this card's own home. Nothing is published in any of
    /// those cases.
    pub fn also_surface_in(&self, session_id: &str) -> bool {
        if self.declined || self.rx.is_none() {
            return false;
        }
        if !self.registry.record_escalation(&self.id, session_id) {
            return false;
        }
        crate::session_events::publish(
            session_id,
            crate::session_events::SessionBusEvent::Agent(crate::agents::AgentEvent::Message(
                request_message(&self.id, &self.request),
            )),
        );
        true
    }

    /// Park until a human answers, `ttl` elapses, or `cancel` trips.
    ///
    /// `cancel` is the **turn's** token, not one made here: every cancellation
    /// mechanism Biorouter has — Stop, `AppState::cancel_turn`, the websocket
    /// `TurnGuard` — reaches a running call through that one token, so a token
    /// minted at this call site would leave the user's Stop pulling on nothing
    /// while the child sat on an HTTP response for the full TTL.
    ///
    /// Always returns an outcome; there is no error path. A caller that cannot
    /// tell "denied" from "the machinery broke" would have to invent a policy
    /// for the difference, and the fail-safe policy is the same either way.
    pub async fn wait(
        mut self,
        ttl: Duration,
        cancel: Option<&CancellationToken>,
    ) -> UserActionOutcome {
        if self.declined {
            return UserActionOutcome::Cancelled;
        }
        let Some(rx) = self.rx.take() else {
            return UserActionOutcome::Failed {
                reason: "this request was already awaited".to_string(),
            };
        };

        let outcome = tokio::select! {
            biased;
            () = async {
                match cancel {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending::<()>().await,
                }
            } => UserActionOutcome::Cancelled,
            answered = tokio::time::timeout(ttl, rx) => match answered {
                Ok(Ok(outcome)) => outcome,
                // The registry entry was dropped without a decision.
                Ok(Err(_)) => UserActionOutcome::Cancelled,
                Err(_) => UserActionOutcome::TimedOut,
            },
        };

        // On every path but a delivered decision the entry is still registered;
        // clearing it here is what stops a late click resolving a receiver that
        // has already gone.
        self.registry.forget(&self.id);
        outcome
    }
}

impl Drop for PendingUserAction {
    fn drop(&mut self) {
        if self.rx.is_some() {
            self.registry.forget(&self.id);
        }
    }
}

/// The card a surface renders for `request`.
///
/// Each variant maps onto the `ActionRequired` shape the desktop already
/// understands, so an approval raised from the bridge draws the *same* dialog as
/// one raised by the agent's own loop rather than a second, subtly different
/// one.
fn request_message(id: &str, request: &UserActionRequest) -> Message {
    let content = match request {
        UserActionRequest::ToolApproval(r) => MessageContent::action_required_with_context(
            id,
            r.tool_name.clone(),
            r.arguments.clone(),
            r.prompt.clone(),
            r.risk,
            r.preview.clone(),
        ),
        UserActionRequest::Elicitation(r) => MessageContent::action_required_elicitation(
            id,
            r.message.clone(),
            r.requested_schema.clone(),
        ),
        // Key names and labels only. Whatever the user types goes from the
        // trusted surface to the keyring; this card is the ask, never the answer.
        UserActionRequest::Secrets(r) => MessageContent::action_required_secrets(
            id,
            r.prompt.clone(),
            r.keys.clone(),
            r.destination.clone(),
        ),
    };
    // `user_only` for the same reason `handle_approval_tool_requests` marks its
    // card: a decision prompt is for the person, and a model that read one would
    // be reading a question it cannot answer. It is belt-and-braces here — the
    // drain does not persist an ephemeral card and never pushes one into the
    // conversation — but the flag is what keeps that true if either changes.
    match request {
        UserActionRequest::Elicitation(_) => Message::assistant().with_content(content),
        // An elicitation is deliberately NOT user-only: its answer is part of
        // the conversation, and the model that raised it has to see both.
        _ => Message::assistant().with_content(content).user_only(),
    }
}

/// Whether `message` is a decision prompt rather than a record.
///
/// An approval or credential card exists only while somebody has to answer it.
/// Persisting one means reopening the session shows a live-looking dialog for a
/// call that finished long ago — and clicking it posts a decision to a request
/// id that no longer exists. `handle_approval_tool_requests` has always yielded
/// its card without persisting; this is the same rule, applied where the drain
/// can see it.
///
/// An **elicitation** is deliberately not ephemeral: its answer is part of the
/// conversation, and the `ElicitationResponse` row references this message's id.
pub fn is_ephemeral_card(message: &Message) -> bool {
    use crate::conversation::message::ActionRequiredData;
    message.content.iter().any(|content| {
        matches!(
            content,
            MessageContent::ActionRequired(action)
                if matches!(
                    action.data,
                    ActionRequiredData::ToolConfirmation { .. }
                        | ActionRequiredData::SecretRequest { .. }
                )
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approval(tool: &str) -> UserActionRequest {
        UserActionRequest::ToolApproval(ToolApprovalRequest {
            tool_name: tool.to_string(),
            arguments: serde_json::Map::new(),
            prompt: None,
            risk: None,
            preview: None,
            requires_user_proof: false,
        })
    }

    fn secrets() -> UserActionRequest {
        UserActionRequest::Secrets(SecretsRequest {
            prompt: "SPOKEAgent needs its passcode".to_string(),
            keys: vec![SecretKeyRequest {
                key: "SPOKEAGENT_PASSCODE".to_string(),
                label: "Passcode".to_string(),
                description: None,
                required: true,
            }],
            destination: SecretDestination::Keyring,
        })
    }

    #[tokio::test]
    async fn an_approval_unparks_with_the_decision() {
        let registry = Arc::new(PendingUserActions::default());
        let parked = registry.park(Some("sess-1"), None, approval("developer__shell"));
        let id = parked.id().to_string();
        assert_eq!(
            registry.resolve_in_session(
                "sess-1",
                &id,
                UserActionOutcome::Approved {
                    permission: Permission::AllowOnce
                },
                DecisionAuthority::unproven(),
            ),
            ResolveOutcome::Delivered
        );
        assert_eq!(
            parked.wait(Duration::from_secs(5), None).await,
            UserActionOutcome::Approved {
                permission: Permission::AllowOnce
            }
        );
    }

    #[tokio::test]
    async fn a_foreign_session_is_unknown_and_leaves_the_waiter_parked() {
        let registry = Arc::new(PendingUserActions::default());
        let parked = registry.park(Some("sess-owner"), None, approval("developer__shell"));
        let id = parked.id().to_string();

        assert_eq!(
            registry.resolve_in_session(
                "sess-foreign",
                &id,
                UserActionOutcome::Approved {
                    permission: Permission::AllowOnce,
                },
                DecisionAuthority::unproven(),
            ),
            ResolveOutcome::Unknown
        );
        assert!(
            registry.is_pending(&id),
            "the real waiter must remain parked"
        );

        assert_eq!(
            registry.resolve_in_session(
                "sess-owner",
                &id,
                UserActionOutcome::Denied {
                    permission: Permission::DenyOnce,
                },
                DecisionAuthority::unproven(),
            ),
            ResolveOutcome::Delivered
        );
        assert_eq!(
            parked.wait(Duration::from_secs(5), None).await,
            UserActionOutcome::Denied {
                permission: Permission::DenyOnce,
            }
        );
    }

    #[tokio::test]
    async fn the_trusted_sessionless_resolver_refuses_non_secret_requests() {
        let registry = Arc::new(PendingUserActions::default());
        let parked = registry.park(Some("sess-owner"), None, approval("developer__shell"));
        let id = parked.id().to_string();

        assert_eq!(
            registry.resolve_trusted_sessionless_secret(
                &id,
                UserActionOutcome::Approved {
                    permission: Permission::AllowOnce,
                },
            ),
            ResolveOutcome::Unknown
        );
        assert!(registry.is_pending(&id), "the approval must remain parked");

        assert_eq!(
            registry.resolve_in_session(
                "sess-owner",
                &id,
                UserActionOutcome::Denied {
                    permission: Permission::DenyOnce,
                },
                DecisionAuthority::unproven(),
            ),
            ResolveOutcome::Delivered
        );
        let _ = parked.wait(Duration::from_secs(5), None).await;
    }

    /// The #107 property that made the bug possible: two bridged calls are in
    /// flight at once (both CLIs issue parallel `tools/call`), and a decision
    /// for one must never release the other.
    #[tokio::test]
    async fn concurrent_requests_cannot_resolve_each_other() {
        let registry = Arc::new(PendingUserActions::default());
        let a = registry.park(Some("sess-1"), None, approval("developer__shell"));
        let b = registry.park(Some("sess-1"), None, approval("developer__text_editor"));
        let b_id = b.id().to_string();
        assert_ne!(a.id(), b.id());

        registry.resolve_in_session(
            "sess-1",
            &b_id,
            UserActionOutcome::Denied {
                permission: Permission::DenyOnce,
            },
            DecisionAuthority::unproven(),
        );

        assert_eq!(
            b.wait(Duration::from_secs(5), None).await,
            UserActionOutcome::Denied {
                permission: Permission::DenyOnce
            }
        );
        // A is untouched and still parked: only its own TTL releases it.
        assert!(registry.is_pending(a.id()));
        assert_eq!(
            a.wait(Duration::from_millis(50), None).await,
            UserActionOutcome::TimedOut
        );
    }

    #[tokio::test]
    async fn a_timeout_is_an_outcome_not_an_error() {
        let registry = Arc::new(PendingUserActions::default());
        let parked = registry.park(Some("sess-1"), None, approval("developer__shell"));
        assert_eq!(
            parked.wait(Duration::from_millis(30), None).await,
            UserActionOutcome::TimedOut
        );
    }

    #[tokio::test]
    async fn the_turns_cancel_token_releases_the_park() {
        let registry = Arc::new(PendingUserActions::default());
        let parked = registry.park(Some("sess-1"), None, approval("developer__shell"));
        let token = CancellationToken::new();
        token.cancel();
        assert_eq!(
            // A TTL far longer than the test could tolerate: if the token were
            // not honoured this would hang rather than fail.
            parked.wait(Duration::from_secs(600), Some(&token)).await,
            UserActionOutcome::Cancelled
        );
    }

    /// A bridge lease drop must release every call parked under that turn —
    /// otherwise a panicking turn leaves the child blocked on an HTTP response
    /// for the full TTL.
    #[tokio::test]
    async fn cancel_owner_releases_only_that_turns_parks() {
        let registry = Arc::new(PendingUserActions::default());
        let mine = registry.park(Some("s"), Some("nonce-a"), approval("t1"));
        let theirs = registry.park(Some("s"), Some("nonce-b"), approval("t2"));
        assert_eq!(registry.cancel_owner("nonce-a"), 1);
        assert_eq!(
            mine.wait(Duration::from_secs(5), None).await,
            UserActionOutcome::Cancelled
        );
        assert!(registry.is_pending(theirs.id()));
    }

    #[tokio::test]
    async fn cancel_session_releases_only_that_sessions_parks() {
        let registry = Arc::new(PendingUserActions::default());
        let a = registry.park(Some("sess-a"), None, approval("t1"));
        let b = registry.park(Some("sess-b"), None, approval("t2"));
        assert_eq!(registry.cancel_session("sess-a"), 1);
        assert_eq!(
            a.wait(Duration::from_secs(5), None).await,
            UserActionOutcome::Cancelled
        );
        assert!(registry.is_pending(b.id()));
    }

    /// The secret-safety guarantee, enforced rather than documented: a route
    /// that answered a credential card with the value would put it on the same
    /// path every tool result takes. `Provided` is refused, the caller stays
    /// parked, and the surface's bug is visible instead of silent.
    #[tokio::test]
    async fn a_secrets_request_refuses_a_value_bearing_outcome() {
        let registry = Arc::new(PendingUserActions::default());
        let parked = registry.park(Some("sess-1"), None, secrets());
        let id = parked.id().to_string();
        assert_eq!(
            registry.resolve_in_session(
                "sess-1",
                &id,
                UserActionOutcome::Provided {
                    data: serde_json::json!({ "SPOKEAGENT_PASSCODE": "hunter2" })
                },
                DecisionAuthority::unproven(),
            ),
            ResolveOutcome::Rejected
        );
        assert!(registry.is_pending(&id), "the caller must stay parked");

        assert_eq!(
            registry.resolve_in_session(
                "sess-1",
                &id,
                UserActionOutcome::SecretsConfigured {
                    configured_keys: vec!["SPOKEAGENT_PASSCODE".to_string()]
                },
                DecisionAuthority::unproven(),
            ),
            ResolveOutcome::Delivered
        );
        assert_eq!(
            parked.wait(Duration::from_secs(5), None).await,
            UserActionOutcome::SecretsConfigured {
                configured_keys: vec!["SPOKEAGENT_PASSCODE".to_string()]
            }
        );
    }

    /// A secrets card must be publishable without the value existing yet, and
    /// the card itself must carry no field a value could sit in.
    #[test]
    fn a_secrets_card_carries_names_only() {
        let card = request_message("req-1", &secrets());
        let json = serde_json::to_string(&card).expect("serialisable");
        assert!(
            json.contains("SPOKEAGENT_PASSCODE"),
            "the key name is shown"
        );
        assert!(
            !json.contains("hunter2"),
            "nothing in the card can carry a value"
        );
    }

    #[tokio::test]
    async fn an_approval_refuses_an_elicitation_answer() {
        let registry = Arc::new(PendingUserActions::default());
        let parked = registry.park(Some("sess-1"), None, approval("developer__shell"));
        assert_eq!(
            registry.resolve_in_session(
                "sess-1",
                parked.id(),
                UserActionOutcome::Provided {
                    data: serde_json::json!({})
                },
                DecisionAuthority::unproven(),
            ),
            ResolveOutcome::Rejected
        );
    }

    /// Approval and credential cards must not be written into history;
    /// elicitations must, because their answer row references them.
    #[test]
    fn only_decision_prompts_are_ephemeral() {
        assert!(is_ephemeral_card(&request_message(
            "1",
            &approval("developer__shell")
        )));
        assert!(is_ephemeral_card(&request_message("2", &secrets())));
        assert!(!is_ephemeral_card(&request_message(
            "3",
            &UserActionRequest::Elicitation(ElicitationRequest {
                message: "Which cohort?".to_string(),
                requested_schema: serde_json::json!({}),
            })
        )));
        assert!(!is_ephemeral_card(&Message::assistant().with_text("hi")));
    }

    /// A chat-less run's card must be UNSCOPED, not queued under a session
    /// named "". Every such run in the process would otherwise share one queue,
    /// which is #40's cross-session leak with a different key.
    #[tokio::test]
    async fn an_unscoped_park_is_deliverable_to_any_loop() {
        use crate::action_required_manager::ActionRequiredManager;
        let registry = Arc::clone(PendingUserActions::global());
        let parked = registry.park(None, None, approval("developer__shell"));
        let id = parked.id().to_string();
        // Any session's loop can drain it, which is what "unscoped" means.
        let drained = ActionRequiredManager::global().drain_requests("some-other-session");
        assert!(
            drained.iter().any(|m| m.content.iter().any(|c| {
                matches!(
                    c,
                    MessageContent::ActionRequired(a)
                        if matches!(
                            &a.data,
                            crate::conversation::message::ActionRequiredData::ToolConfirmation {
                                id: card_id, ..
                            } if *card_id == id
                        )
                )
            })),
            "an unscoped card must reach a loop that is not its own session's"
        );
        assert_eq!(
            registry.resolve_in_session(
                "foreign-session",
                &id,
                UserActionOutcome::Approved {
                    permission: Permission::AllowOnce,
                },
                DecisionAuthority::unproven(),
            ),
            ResolveOutcome::Unknown
        );
        assert!(registry.is_pending(&id));
        assert_eq!(
            registry.resolve_in_session(
                "some-other-session",
                &id,
                UserActionOutcome::Approved {
                    permission: Permission::AllowOnce,
                },
                DecisionAuthority::unproven(),
            ),
            ResolveOutcome::Delivered
        );
        let _ = parked.wait(Duration::from_secs(5), None).await;
    }

    #[tokio::test]
    async fn a_decision_for_an_unknown_id_is_dropped() {
        let registry = Arc::new(PendingUserActions::default());
        assert_eq!(
            registry.resolve_in_session(
                "s",
                "no-such-request",
                UserActionOutcome::Cancelled,
                DecisionAuthority::unproven()
            ),
            ResolveOutcome::Unknown
        );
    }

    /// A double-click is a no-op, not a second decision aimed at whatever is
    /// parked now.
    #[tokio::test]
    async fn a_duplicate_decision_is_unknown() {
        let registry = Arc::new(PendingUserActions::default());
        let parked = registry.park(Some("s"), None, approval("t"));
        let id = parked.id().to_string();
        assert_eq!(
            registry.resolve_in_session(
                "s",
                &id,
                UserActionOutcome::Approved {
                    permission: Permission::AllowOnce
                },
                DecisionAuthority::unproven(),
            ),
            ResolveOutcome::Delivered
        );
        assert_eq!(
            registry.resolve_in_session(
                "s",
                &id,
                UserActionOutcome::Approved {
                    permission: Permission::AlwaysAllow
                },
                DecisionAuthority::unproven(),
            ),
            ResolveOutcome::Unknown
        );
    }

    /// Dropping the handle without awaiting deregisters the request: a decision
    /// arriving afterwards has nowhere to land and says so.
    #[tokio::test]
    async fn dropping_the_handle_deregisters_the_request() {
        let registry = Arc::new(PendingUserActions::default());
        let id = {
            let parked = registry.park(Some("s"), None, approval("t"));
            assert!(registry.is_pending(parked.id()));
            parked.id().to_string()
        };
        assert!(!registry.is_pending(&id));
    }

    /// The refusal text must never tell a model that a chat message can approve
    /// something — on the bridged path the request id is gone by the time the
    /// model reads it (#107).
    #[test]
    fn no_outcome_claims_a_chat_message_can_approve() {
        for outcome in [
            UserActionOutcome::Denied {
                permission: Permission::DenyOnce,
            },
            UserActionOutcome::Cancelled,
            UserActionOutcome::TimedOut,
            UserActionOutcome::Failed {
                reason: "no surface".to_string(),
            },
        ] {
            let detail = outcome.refusal_detail();
            assert!(
                !detail.contains("approve it") && !detail.contains("let them"),
                "{detail:?} invites an answer that cannot land"
            );
        }
    }
}

/// F-15 Gap A: a decision raised where nobody can answer it must be refused,
/// not parked. The failure it prevents is not a hang — it is a card that waits
/// out its whole TTL unanswerable and is then resurrected by the NEXT chat
/// turn, as a question about something that happened minutes ago.
#[cfg(test)]
mod no_human_surface_tests {
    use super::*;

    fn an_approval() -> UserActionRequest {
        UserActionRequest::ToolApproval(ToolApprovalRequest {
            tool_name: "developer__shell".to_string(),
            arguments: serde_json::Map::new(),
            prompt: None,
            risk: None,
            preview: None,
            requires_user_proof: false,
        })
    }

    #[tokio::test]
    async fn a_park_with_no_human_surface_registers_nothing_and_cancels() {
        let registry = Arc::new(PendingUserActions::default());
        let (id, outcome) = crate::user_surface::without_human_surface(async {
            let parked = registry.park(Some("call-tool-session"), None, an_approval());
            let id = parked.id().to_string();
            // Must not block: there is nobody to answer, so the outcome is
            // available immediately.
            let outcome = parked.wait(Duration::from_secs(30), None).await;
            (id, outcome)
        })
        .await;

        assert!(
            matches!(outcome, UserActionOutcome::Cancelled),
            "{outcome:?}"
        );
        // ⚠ The registration is the half that matters. A refusal that still
        // inserted would leave the entry for the next turn to surface, which is
        // the exact bug — and `wait` returning promptly would hide it.
        assert!(
            !registry.is_pending(&id),
            "a refused park must leave nothing behind for a later turn to find"
        );
        assert!(registry
            .pending_cards_for_session("call-tool-session")
            .is_empty());
    }

    #[tokio::test]
    async fn an_ordinary_park_is_untouched() {
        // Catches a guard whose default is inverted, which would silently
        // cancel every approval card in every normal turn.
        let registry = Arc::new(PendingUserActions::default());
        let parked = registry.park(Some("chat-session"), None, an_approval());
        let id = parked.id().to_string();
        assert!(registry.is_pending(&id));
        assert_eq!(registry.pending_cards_for_session("chat-session").len(), 1);
        assert!(registry
            .pending_cards_for_session("another-session")
            .is_empty());

        assert_eq!(
            registry.resolve_in_session(
                "chat-session",
                &id,
                UserActionOutcome::Approved {
                    permission: crate::permission::Permission::AllowOnce,
                },
                DecisionAuthority::unproven(),
            ),
            ResolveOutcome::Delivered
        );
        // A resolved card is unrepresentable: resolution removed the entry, so
        // a late observer can never be shown one that was already answered.
        assert!(registry
            .pending_cards_for_session("chat-session")
            .is_empty());
        let _ = parked.wait(Duration::from_secs(5), None).await;
    }

    #[tokio::test]
    async fn an_already_awaited_handle_still_reports_a_caller_bug() {
        // The `declined` flag must not collapse into `rx: None`: a declined
        // park is the fail-safe `Cancelled`, while awaiting twice names a bug.
        let registry = Arc::new(PendingUserActions::default());
        let parked = registry.park(Some("chat-session"), None, an_approval());
        let id = parked.id().to_string();
        registry.cancel_session(&id);
        let mut parked = parked;
        parked.rx = None;
        assert!(matches!(
            parked.wait(Duration::from_millis(50), None).await,
            UserActionOutcome::Failed { .. }
        ));
    }
}

/// F-12: an Agent Drafter app page resolved a proof-backed approval with no
/// proof at all. The page's JavaScript is written by the agent, so the app
/// socket's `Approve` frame is a model approving its own request — and the only
/// proof check lived in one HTTP route the socket does not pass through.
#[cfg(test)]
mod decision_authority_tests {
    use super::*;

    fn a_proof_backed_approval() -> UserActionRequest {
        UserActionRequest::ToolApproval(ToolApprovalRequest {
            tool_name: "skills__removeSkillPackage".to_string(),
            arguments: serde_json::Map::new(),
            prompt: None,
            risk: None,
            preview: None,
            requires_user_proof: true,
        })
    }

    fn an_ordinary_approval() -> UserActionRequest {
        UserActionRequest::ToolApproval(ToolApprovalRequest {
            tool_name: "developer__shell".to_string(),
            arguments: serde_json::Map::new(),
            prompt: None,
            risk: None,
            preview: None,
            requires_user_proof: false,
        })
    }

    fn allow() -> UserActionOutcome {
        UserActionOutcome::Approved {
            permission: crate::permission::Permission::AllowOnce,
        }
    }

    #[tokio::test]
    async fn a_refusal_parks_the_caller_rather_than_cancelling_it() {
        // ⚠ Catches a gate written after `entry.tx.take()` or
        // `entries.remove(id)`. That version refuses the poster while
        // DESTROYING the park: the tool then fails as cancelled, and the
        // refusing door's deny follow-up has nothing left to deny.
        let registry = Arc::new(PendingUserActions::default());
        let parked = registry.park(Some("app"), None, a_proof_backed_approval());
        let id = parked.id().to_string();

        assert_eq!(
            registry.resolve_in_session("app", &id, allow(), DecisionAuthority::unproven()),
            ResolveOutcome::Unproven
        );
        assert!(
            registry.is_pending(&id),
            "a refused approval must stay parked for a surface that could answer it"
        );
        drop(parked);
    }

    #[tokio::test]
    async fn a_denial_lands_from_any_surface() {
        // Catches the over-correction "no proof, no resolution", which would
        // strand every app-page Reject and every socket close for the full
        // time-to-live — and would break the deny follow-up that makes the
        // refusal fast instead of a 570-second hang.
        for outcome in [
            UserActionOutcome::Denied {
                permission: crate::permission::Permission::DenyOnce,
            },
            UserActionOutcome::Cancelled,
        ] {
            let registry = Arc::new(PendingUserActions::default());
            let parked = registry.park(Some("app"), None, a_proof_backed_approval());
            let id = parked.id().to_string();
            assert_eq!(
                registry.resolve_in_session(
                    "app",
                    &id,
                    outcome.clone(),
                    DecisionAuthority::unproven()
                ),
                ResolveOutcome::Delivered,
                "{outcome:?} must land from an unproven surface"
            );
        }
    }

    /// **D10, and the line that must not move.** A card escalated into the
    /// conversation a person is watching widens who may SEE and CLICK it. It must
    /// not widen what a click without proof may grant.
    ///
    /// The gate keys on the authority of the answering request, never on the
    /// session it came from, so the escalation surface is refused exactly as the
    /// card's own home is — and the caller stays parked for a surface that can
    /// prove a person.
    #[tokio::test]
    async fn an_escalated_card_refuses_an_unproven_allow_exactly_as_its_home_does() {
        let registry = Arc::new(PendingUserActions::default());
        let parked = registry.park(Some("child"), None, a_proof_backed_approval());
        let id = parked.id().to_string();
        assert!(parked.also_surface_in("parent"));

        assert_eq!(
            registry.resolve_in_session("parent", &id, allow(), DecisionAuthority::unproven()),
            ResolveOutcome::Unproven,
            "escalating a card must not create a door that grants without proof"
        );
        assert!(registry.is_pending(&id));
        assert_eq!(
            registry.resolve_in_session(
                "parent",
                &id,
                allow(),
                DecisionAuthority::for_test_proven()
            ),
            ResolveOutcome::Delivered,
            "a proven person at the escalation surface may still answer"
        );
        drop(parked);
    }

    /// The route asks `requires_user_proof_in_session` BEFORE it resolves, to
    /// choose between a 403 that says "the user decides this" and one that says
    /// "this control is unavailable here". Answered for the card's home only, an
    /// escalated card would be reported as needing no proof, the route would skip
    /// its check, and the person would get a bare `refused` from the gate below
    /// with no sentence attached.
    #[tokio::test]
    async fn the_proof_requirement_is_reported_at_the_escalation_surface_too() {
        let registry = Arc::new(PendingUserActions::default());
        let parked = registry.park(Some("child"), None, a_proof_backed_approval());
        let id = parked.id().to_string();
        assert!(parked.also_surface_in("parent"));

        assert!(registry.requires_user_proof_in_session("child", &id));
        assert!(registry.requires_user_proof_in_session("parent", &id));
        assert!(
            !registry.requires_user_proof_in_session("bystander", &id),
            "a session the card was never shown in must learn nothing about it"
        );
        drop(parked);
    }

    /// Exactly one session is added, and only a session that was asked for. An
    /// escalation is not a licence for any conversation that knows the id (#40).
    #[tokio::test]
    async fn escalating_widens_the_answering_scope_by_exactly_one_session() {
        let registry = Arc::new(PendingUserActions::default());
        let parked = registry.park(Some("child"), None, an_ordinary_approval());
        let id = parked.id().to_string();
        assert!(parked.also_surface_in("parent"));

        assert_eq!(
            registry.resolve_in_session("bystander", &id, allow(), DecisionAuthority::unproven()),
            ResolveOutcome::Unknown,
            "a bystander conversation must not be able to grant this call"
        );
        assert!(registry.is_pending(&id));
        assert_eq!(
            registry.resolve_in_session("parent", &id, allow(), DecisionAuthority::unproven()),
            ResolveOutcome::Delivered
        );
        let _ = parked.wait(Duration::from_secs(5), None).await;
    }

    /// Two calls add one surface, and a card is never escalated into its own
    /// home — otherwise the chat that raised it renders the same ask twice.
    #[tokio::test]
    async fn escalation_is_idempotent_and_never_targets_the_cards_own_home() {
        let registry = Arc::new(PendingUserActions::default());
        let parked = registry.park(Some("child"), None, an_ordinary_approval());

        assert!(
            !parked.also_surface_in("child"),
            "the card's home already shows it"
        );
        assert!(parked.also_surface_in("parent"));
        assert!(
            parked.also_surface_in("parent"),
            "a second call is a no-op, not a second surface"
        );
        assert_eq!(
            registry.pending_cards_for_session("parent").len(),
            1,
            "one ask, one card at that surface"
        );
        drop(parked);
    }

    /// A park that registered nothing — `park` declined because no person could
    /// be asked — has nothing to escalate, and must publish nothing. Without the
    /// guard an unattended run would broadcast a card for a call it is about to
    /// refuse anyway.
    #[tokio::test]
    async fn a_park_nobody_could_answer_is_not_escalated() {
        let registry = Arc::new(PendingUserActions::default());
        let parked = crate::user_surface::without_human_surface(async {
            registry.park(Some("child"), None, an_ordinary_approval())
        })
        .await;
        assert!(!parked.also_surface_in("parent"));
        assert!(
            registry.pending_cards_for_session("parent").is_empty(),
            "a declined park must leave no card anywhere"
        );
    }

    #[tokio::test]
    async fn an_ordinary_approval_is_untouched_by_the_gate() {
        // ⚠ Catches a gate keyed on `ToolApproval(_)` rather than on the flag.
        // That version kills every bridged Claude Code / Codex approval on the
        // desktop, which sets `requires_user_proof: false` in production.
        let registry = Arc::new(PendingUserActions::default());
        let parked = registry.park(Some("chat"), None, an_ordinary_approval());
        let id = parked.id().to_string();
        assert_eq!(
            registry.resolve_in_session("chat", &id, allow(), DecisionAuthority::unproven()),
            ResolveOutcome::Delivered
        );
        let _ = parked.wait(Duration::from_secs(5), None).await;
    }

    #[tokio::test]
    async fn the_two_privileged_authorities_grant_it() {
        for authority in [
            DecisionAuthority::from_user_action_proof(),
            DecisionAuthority::from_local_human_surface(),
        ] {
            let registry = Arc::new(PendingUserActions::default());
            let parked = registry.park(Some("desktop"), None, a_proof_backed_approval());
            let id = parked.id().to_string();
            assert_eq!(
                registry.resolve_in_session("desktop", &id, allow(), authority),
                ResolveOutcome::Delivered,
                "{authority:?} must still be able to approve, or the desktop is broken"
            );
        }
    }

    /// The property the newtype exists to have: the privileged words can only
    /// be spoken in a small, named set of places.
    ///
    /// ⚠ Three mechanics, and without all three this is theatre. (a) The
    /// needles are COMPOSED, so this file does not match its own audit. (b) The
    /// defining file is skipped, since it names all three constructors. (c) A
    /// negative control asserts the app socket says `unproven` and neither
    /// privileged word — without it, a needle that stopped matching would make
    /// every assertion pass vacuously.
    #[test]
    fn the_privileged_authorities_are_minted_in_exactly_the_expected_places() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let crates = root.join("crates");
        assert!(
            crates.is_dir(),
            "the audit walks {}; if that path is wrong every assertion below \
             passes for the wrong reason",
            crates.display()
        );

        let proven = concat!("from_user_", "action_proof(");
        let local = concat!("from_local_", "human_surface(");
        let unproven = concat!("Decision", "Authority::unproven(");

        let mut proven_files: std::collections::BTreeMap<String, usize> = Default::default();
        let mut local_files: std::collections::BTreeMap<String, usize> = Default::default();
        let mut app_socket = None;
        let mut scanned = 0usize;
        for entry in walkdir::WalkDir::new(&crates) {
            let entry = entry.expect("the audit must not silently skip an unreadable directory");
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let rel = p
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            // This file defines all three constructors and exercises them.
            if rel == "crates/biorouter/src/pending_user_action.rs" {
                continue;
            }
            let body = std::fs::read_to_string(p).unwrap();
            scanned += 1;
            let p_count = body.matches(proven).count();
            let l_count = body.matches(local).count();
            if p_count > 0 {
                proven_files.insert(rel.clone(), p_count);
            }
            if l_count > 0 {
                local_files.insert(rel.clone(), l_count);
            }
            if rel == "crates/biorouter-server/src/routes/apps.rs" {
                app_socket = Some((body.matches(unproven).count(), p_count, l_count));
            }
        }
        assert!(scanned > 100, "the walk found only {scanned} files");

        assert_eq!(
            proven_files.keys().cloned().collect::<Vec<_>>(),
            vec!["crates/biorouter-server/src/routes/action_required.rs".to_string()],
            "a new door is claiming a user's own proof: {proven_files:?}"
        );
        assert_eq!(
            local_files.keys().cloned().collect::<Vec<_>>(),
            vec![
                "crates/biorouter-cli/src/session/mod.rs".to_string(),
                "crates/biorouter-cli/src/session/tui/mod.rs".to_string(),
            ],
            "a new door is claiming a local human surface: {local_files:?}"
        );

        // The negative control. If the needles ever stop matching, this fails
        // rather than letting the two assertions above pass on empty maps.
        let (unproven_hits, proven_hits, local_hits) =
            app_socket.expect("the app socket file must exist");
        assert!(
            unproven_hits > 0,
            "the app socket must name `unproven` — if it does not, the needle is wrong \
             and this whole audit is vacuous"
        );
        assert_eq!((proven_hits, local_hits), (0, 0));
    }
}
