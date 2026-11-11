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
//!
//! # The end of a card (WI-17)
//!
//! A card a surface drew must stop looking answerable the moment nothing waits on it, on every
//! surface that drew it, including one that is not the one that answered. So every way an
//! ephemeral card leaves the registry (answered, dismissed, timed out, or withdrawn by the call
//! that raised it, the turn's Stop or the chat closing) announces a [`CardEnded`] on
//! [`PendingUserActions::subscribe_card_endings`], once for its home and once for each chat it
//! was escalated into. It carries the card's ID and a [`CardEnding`] and nothing else: never a
//! value, a choice, or even whether an approval was allowed. The daemon's stream code turns it
//! into a frame for the renderers, and a late observer is handed only the cards still parked
//! ([`PendingUserActions::pending_cards_for_session`]).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{broadcast, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::debug;
use uuid::Uuid;

use rmcp::model::JsonObject;

use crate::conversation::message::{
    CrewStepRequest, Message, MessageContent, SecretDestination, SecretKeyRequest,
};
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

#[derive(Debug, Clone)]
pub struct FolderAccessRequest {
    pub action_id: String,
    pub tool_name: String,
    pub folder: String,
    pub access: String,
    pub once_descendants: bool,
    pub files: Vec<String>,
}

impl FolderAccessRequest {
    pub fn prompt(&self) -> String {
        format!(
            "External folder access\nAllow {} access to {} for {}? {} A session grant includes descendants. Revoking the grant or deleting this chat ends it.\nFiles:\n{}",
            self.access, self.folder, self.tool_name,
            if self.once_descendants { "Allow once includes read-only analysis of this folder and its descendants for this operation." } else { "Allow once covers only the listed files in this operation." },
            self.files.join("\n")
        )
    }
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
    FolderAccess(FolderAccessRequest),
    Elicitation(ElicitationRequest),
    Secrets(SecretsRequest),
    /// A Crew step only the person can take (the owner's D1 ruling on AO-12): the card opens
    /// the person's own Sign in window, vault prompt or host-key help, and the parked call
    /// learns the result from the daemon's state, never from the card.
    ///
    /// ⚠ **Nothing can resolve this as a success.** [`Self::accepts`] takes only Cancelled,
    /// TimedOut and Failed, so a forged `Approved`, `Provided` or `SecretsConfigured` is
    /// `Rejected` and `is_allowed()` is never true for it. The host that parked it decides
    /// what happened by reading the daemon (the job left `needs_sign_in`, the vault is
    /// unlocked, the key is in `known_hosts`) and withdraws the card itself.
    CrewStep(CrewStepRequest),
}

impl UserActionRequest {
    fn requires_user_proof(&self) -> bool {
        match self {
            Self::FolderAccess(_) | Self::CrewStep(_) => true,
            Self::ToolApproval(request) => request.requires_user_proof,
            Self::Elicitation(_) | Self::Secrets(_) => false,
        }
    }

    /// A short label for logs and refusal text. Never includes arguments, a
    /// schema, or anything from a secrets request beyond its key names.
    pub fn describe(&self) -> String {
        match self {
            Self::ToolApproval(r) => format!("approval for `{}`", r.tool_name),
            Self::FolderAccess(_) => "external folder access".to_string(),
            Self::Elicitation(_) => "input".to_string(),
            Self::Secrets(r) => format!("{} credential(s)", r.keys.len()),
            // Never the server, the login or a fingerprint.
            Self::CrewStep(step) => match step {
                CrewStepRequest::SignIn { .. } => "a Crew sign-in",
                CrewStepRequest::Vault {
                    action: crate::conversation::message::CrewVaultAction::Unlock,
                } => "unlocking Crew's vault",
                CrewStepRequest::Vault {
                    action: crate::conversation::message::CrewVaultAction::Init,
                } => "setting up Crew's vault",
                CrewStepRequest::HostKey { .. } => "a server's host key",
            }
            .to_string(),
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
    ///
    /// [`UserActionRequest::CrewStep`] has no arm of its own on purpose: it accepts
    /// only the three endings every request accepts, so no answer can mean "done".
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
                Self::ToolApproval(_) | Self::FolderAccess(_),
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

/// How a card stopped waiting, as every surface that drew it is told (WI-17).
///
/// ⚠ Deliberately coarse. It never says what was answered: an approval allowed and one refused
/// are both [`Self::Answered`], so the signal can go to every surface that showed the card
/// without telling any of them more than that the card is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardEnding {
    /// A person answered it.
    Answered,
    /// A person dismissed it (a Crew step's Not now, a card's Cancel).
    Dismissed,
    /// Nobody answered before its time ran out.
    TimedOut,
    /// The call that raised it stopped waiting: the call withdrew it (a Crew step the daemon
    /// saw done), the turn was stopped, or the chat closed.
    Withdrawn,
}

impl CardEnding {
    /// The ending a delivered outcome means.
    fn of(outcome: &UserActionOutcome) -> Self {
        match outcome {
            UserActionOutcome::Approved { .. }
            | UserActionOutcome::Denied { .. }
            | UserActionOutcome::Provided { .. }
            | UserActionOutcome::SecretsConfigured { .. } => Self::Answered,
            UserActionOutcome::Cancelled => Self::Dismissed,
            UserActionOutcome::TimedOut => Self::TimedOut,
            UserActionOutcome::Failed { .. } => Self::Withdrawn,
        }
    }
}

/// One card that stopped waiting, for one chat that drew it (WI-17). The card's ID and how it
/// ended, and nothing a person typed, chose or configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardEnded {
    /// A chat the card was shown in: its home, or one it was escalated into.
    pub session_id: String,
    /// The card's ID, as its published message carries it.
    pub id: String,
    pub ending: CardEnding,
}

/// How many endings a slow subscriber may fall behind by before it is told it lagged. Endings
/// are rare (one per card), so this is far beyond any real burst.
const CARD_ENDINGS_CAPACITY: usize = 256;

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
pub struct DecisionAuthority(Authority, Option<Uuid>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Authority {
    /// `auth::user_action_proof` returned `Proven` for THIS request.
    Proven,
    /// A person answered at a surface this process owns and a model cannot
    /// author — the terminal prompt, the TUI modal.
    LocalHuman,
    FolderAccess,
    /// Everything else, and the default reading of an unknown door: page
    /// JavaScript an agent wrote, an agent deciding for an agent, a transport
    /// with no proof channel at all.
    Unproven,
}

impl DecisionAuthority {
    /// Mint only after the folder-scoped browser approval key was verified.
    /// This authority cannot approve any other protected request kind.
    pub fn from_folder_access_proof() -> Self {
        Self(Authority::FolderAccess, None)
    }
    pub fn from_native_folder_access_proof(client: &crate::native_client::NativeClient) -> Self {
        Self(Authority::FolderAccess, Some(client.id()))
    }

    /// Mint ONLY after `auth::user_action_proof` returned `Proven` for the
    /// request being answered.
    pub fn from_user_action_proof() -> Self {
        Self(Authority::Proven, None)
    }

    /// A person acted at a surface this process owns.
    pub fn from_local_human_surface() -> Self {
        Self(Authority::LocalHuman, None)
    }

    /// No preconditions. The honest answer for every door that cannot tell a
    /// person from a model.
    pub fn unproven() -> Self {
        Self(Authority::Unproven, None)
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
        Self(Authority::Proven, None)
    }

    fn may_grant_proof_backed(self, request: &UserActionRequest) -> bool {
        matches!(self.0, Authority::Proven | Authority::LocalHuman)
            || (self.0 == Authority::FolderAccess
                && matches!(request, UserActionRequest::FolderAccess(_)))
    }
}

struct Entry {
    session_id: Option<String>,
    owner: Option<String>,
    request: UserActionRequest,
    tx: Option<oneshot::Sender<UserActionOutcome>>,
    native_client: Option<crate::native_client::NativeClient>,
    /// Other sessions this same card was published into, and from which a
    /// decision is therefore accepted (D10).
    ///
    /// One entry today: the conversation that delegated the work, for a card
    /// raised inside a sub-agent. `session_id` is still the card's **home** —
    /// the session that owns the parked call and whose tab always shows it —
    /// and this list only widens *who may answer*, never who may raise.
    ///
    /// ⚠ Recording and publishing happen in one operation
    /// ([`PendingUserAction::also_surface_in_for`]) because each half alone is a bug
    /// with its own symptom: a recorded surface the card was never published to
    /// is an approval nobody can see, and a published card that was never
    /// recorded is a card the user clicks and watches do nothing —
    /// [`Self::answerable_in`] answers `Unknown` for every session but this one.
    ///
    /// What that pairing does NOT promise is an audience. `session_events::publish`
    /// is best-effort by design: a session nobody is observing drops the frame.
    /// So a surface recorded here may still show nothing — for a chat whose turn
    /// has ended and which is therefore neither streaming `/reply` nor observing
    /// its own bus. That is no worse than not escalating, and it grants nothing:
    /// a decision still has to carry the request id, which only a rendered card
    /// supplies.
    escalated_to: Vec<String>,
    /// The name the escalated copies give the asker (R2-1): the subagent's
    /// role, else its chat's name. Only those copies carry it; the card's home
    /// is the asker's own chat.
    requested_by: Option<String>,
}

impl Entry {
    /// The card as `session_id` shows it: an escalated copy names who asks.
    fn card_for(&self, id: &str, session_id: &str) -> Message {
        let requested_by = if self.session_id.as_deref() == Some(session_id) {
            None
        } else {
            self.requested_by.as_deref()
        };
        request_message(id, &self.request, requested_by)
    }

    /// May a decision posted from `session_id` release this call?
    ///
    /// The card's home, plus any session it was escalated into. Deliberately
    /// NOT "any session": #40's rule is that an authorization belongs to the
    /// exact surfaces that showed it, and a card another conversation could
    /// answer merely by knowing the id is a cross-session approval leak.
    fn answerable_in(&self, session_id: &str) -> bool {
        self.session_id.as_deref() == Some(session_id)
            || self
                .escalated_to
                .iter()
                .any(|surface| surface == session_id)
    }
}

/// The process-global registry of parked user actions.
///
/// Process-global for the same reason [`crate::action_required_manager`] is:
/// the surface that answers runs on a different task from the caller that
/// parked, often in a different subsystem, and on the bridged path there is no
/// `Agent` for either of them to share.
pub struct PendingUserActions {
    entries: Mutex<HashMap<String, Entry>>,
    /// Where every ephemeral card's end is announced ([`Self::subscribe_card_endings`]).
    endings: broadcast::Sender<CardEnded>,
    /// The cards that ended most recently, oldest first ([`Self::has_ended`]).
    ended: Mutex<RecentlyEnded>,
}

/// The IDs of the last [`ENDED_MEMORY`] cards that ended.
#[derive(Default)]
struct RecentlyEnded {
    order: std::collections::VecDeque<String>,
    ids: std::collections::HashSet<String>,
}

/// How many ended cards [`PendingUserActions::has_ended`] remembers: far more than are ever
/// drawn between a card's end and its chat's loop drawing it, which is the window it covers.
const ENDED_MEMORY: usize = 1024;

impl Default for PendingUserActions {
    fn default() -> Self {
        Self {
            entries: Mutex::default(),
            endings: broadcast::channel(CARD_ENDINGS_CAPACITY).0,
            ended: Mutex::default(),
        }
    }
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

    /// Every ephemeral card's end, from now on, for every chat (WI-17): a stream filters for its
    /// own chat. Subscribe before reading [`Self::pending_cards_for_session`], so a card that
    /// ends between the two is still heard ending.
    pub fn subscribe_card_endings(&self) -> broadcast::Receiver<CardEnded> {
        self.endings.subscribe()
    }

    /// Whether the card `id` was parked here and has ended (one of the last [`ENDED_MEMORY`]).
    ///
    /// The card a call parks is queued for its chat's loop to draw, and the queue keeps it after
    /// the card ends, so the loop can draw a card that waits for nothing: answered from the chat
    /// it was escalated into, withdrawn by a call that saw the step done at once, dropped by a
    /// call that failed. A stream asks this before it hands such a card to its client. A card the
    /// agent's own loop raised was never parked here, so this is `false` for it.
    pub fn has_ended(&self, id: &str) -> bool {
        self.ended
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .ids
            .contains(id)
    }

    /// Announce that the card `id`, which `entry` held, stopped waiting: once for its home and
    /// once for each chat it was escalated into. Nothing for a card that is a record rather
    /// than a question (an elicitation, a changed host key's notice), whose stored row stays.
    /// Called after the entry left the map, never under the lock.
    fn announce_end(&self, id: &str, entry: &Entry, ending: CardEnding) {
        if !request_is_ephemeral(&entry.request) {
            return;
        }
        {
            let mut ended = self.ended.lock().unwrap_or_else(PoisonError::into_inner);
            if ended.ids.insert(id.to_string()) {
                ended.order.push_back(id.to_string());
            }
            while ended.order.len() > ENDED_MEMORY {
                if let Some(oldest) = ended.order.pop_front() {
                    ended.ids.remove(&oldest);
                }
            }
        }
        for session_id in entry.session_id.iter().chain(&entry.escalated_to) {
            // No subscriber is an ordinary state: nobody is watching that chat.
            let _ = self.endings.send(CardEnded {
                session_id: session_id.clone(),
                id: id.to_string(),
                ending,
            });
        }
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
                native_client: if matches!(request, UserActionRequest::FolderAccess(_)) {
                    crate::native_client::current()
                } else {
                    None
                },
                escalated_to: Vec::new(),
                requested_by: None,
            },
        );

        crate::action_required_manager::ActionRequiredManager::global()
            .publish(session_id, request_message(&id, &request, None));

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

    /// Claim only an ordinary elicitation at its origin. Escalation surfaces
    /// cannot cause ordinary answer data to be persisted into another session.
    pub(crate) fn claim_elicitation_in_session(
        &self,
        session_id: &str,
        id: &str,
    ) -> Option<crate::action_required_manager::OrdinaryElicitationClaim> {
        let mut entries = self.lock();
        let entry = entries.get_mut(id)?;
        if entry.session_id.as_deref() != Some(session_id)
            || !matches!(&entry.request, UserActionRequest::Elicitation(_))
        {
            return None;
        }
        let sender = entry.tx.take()?;
        entries.remove(id);
        Some(
            crate::action_required_manager::OrdinaryElicitationClaim::pending(
                session_id.into(),
                sender,
            ),
        )
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

    /// Dismiss a Crew step card from a session that shows it: the one answer a
    /// [`UserActionRequest::CrewStep`] takes from a surface, and harmless by construction,
    /// since it releases the parked call as Cancelled and grants nothing.
    ///
    /// Only a Crew step this session may answer is touched. Any other id, a card of another
    /// kind or a foreign session is [`ResolveOutcome::Unknown`], so this door can never
    /// cancel an approval or a credential card.
    pub fn dismiss_crew_step(&self, session_id: &str, id: &str) -> ResolveOutcome {
        self.resolve_matching(
            id,
            UserActionOutcome::Cancelled,
            DecisionAuthority::unproven(),
            |entry| {
                entry.answerable_in(session_id)
                    && matches!(entry.request, UserActionRequest::CrewStep(_))
            },
        )
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
            && !authority.may_grant_proof_backed(&entry.request)
            && entry.request.requires_user_proof()
        {
            return ResolveOutcome::Unproven;
        }
        if outcome.is_allowed() && matches!(entry.request, UserActionRequest::FolderAccess(_)) {
            let expected = entry
                .native_client
                .as_ref()
                .map(crate::native_client::NativeClient::id);
            if expected != authority.1
                || entry
                    .native_client
                    .as_ref()
                    .is_some_and(|client| !client.is_live())
            {
                return ResolveOutcome::Unproven;
            }
        }
        let Some(tx) = entry.tx.take() else {
            return ResolveOutcome::Unknown;
        };
        let removed = entries.remove(id);
        drop(entries);
        // Before the waiter is released, so the end is announced ahead of anything the call
        // does next.
        if let Some(removed) = removed {
            self.announce_end(id, &removed, CardEnding::of(&outcome));
        }
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
            .map(|(id, entry)| entry.card_for(id, session_id))
            .filter(is_ephemeral_card)
            .collect()
    }

    /// Whether any card answerable in `session_id` is still waiting (D17).
    pub fn has_pending_in_session(&self, session_id: &str) -> bool {
        self.lock()
            .values()
            .any(|entry| entry.answerable_in(session_id))
    }

    /// Whether this exact session-scoped approval requires proof of a human
    /// action. A foreign session learns nothing and cannot satisfy the check.
    pub fn requires_user_proof_in_session(&self, session_id: &str, id: &str) -> bool {
        self.lock().get(id).is_some_and(|entry| {
            entry.answerable_in(session_id) && entry.request.requires_user_proof()
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
        let removed: Vec<(String, Entry)> = {
            let mut entries = self.lock();
            let ids: Vec<String> = entries
                .iter()
                .filter(|(_, entry)| predicate(entry))
                .map(|(id, _)| id.clone())
                .collect();
            ids.into_iter()
                .filter_map(|id| entries.remove(&id).map(|entry| (id, entry)))
                .collect()
        };
        let mut senders = Vec::with_capacity(removed.len());
        for (id, mut entry) in removed {
            self.announce_end(&id, &entry, CardEnding::Withdrawn);
            senders.extend(entry.tx.take());
        }
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
    fn record_escalation(&self, id: &str, session_id: &str, requested_by: Option<&str>) -> bool {
        let mut entries = self.lock();
        let Some(entry) = entries.get_mut(id) else {
            return false;
        };
        if entry.session_id.as_deref() == Some(session_id) {
            return false;
        }
        if let Some(name) = requested_by.map(str::trim).filter(|name| !name.is_empty()) {
            entry.requested_by = Some(name.to_string());
        }
        if !entry
            .escalated_to
            .iter()
            .any(|surface| surface == session_id)
        {
            entry.escalated_to.push(session_id.to_string());
        }
        true
    }

    /// Drop the entry for `id` without a decision, announcing that it ended as `ending`.
    /// Idempotent: an entry already gone (answered, cancelled) announces nothing again.
    fn forget(&self, id: &str, ending: CardEnding) {
        let removed = self.lock().remove(id);
        if let Some(entry) = removed {
            self.announce_end(id, &entry, ending);
        }
    }

    /// Remove `id` if it is still waiting for a decision, announce that it
    /// timed out, and say where it was showing. `None` when a decision took the
    /// entry first, and announced its own end.
    fn take_unanswered(&self, id: &str) -> Option<CardSurfaces> {
        let entry = self.lock().remove(id)?;
        entry.tx.as_ref()?;
        // The entry leaves the map here, so the `forget` that follows in `wait`
        // finds nothing: this is the one place the timeout's end is announced.
        self.announce_end(id, &entry, CardEnding::TimedOut);
        Some(CardSurfaces {
            home: entry.session_id,
            escalated_to: entry.escalated_to,
        })
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

    /// Test shorthand for [`Self::also_surface_in_for`] with nobody named.
    #[cfg(test)]
    pub(crate) fn also_surface_in(&self, session_id: &str) -> bool {
        self.also_surface_in_for(session_id, None)
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
    /// the original with, so the two surfaces carry the same card — and
    /// therefore the same request id, which is what makes them one decision
    /// rather than two questions. The one difference is `requested_by` (R2-1):
    /// a folder card's copy names the subagent that asks (its role, else its
    /// chat's name), where the home copy is that chat's own agent asking.
    /// `user_only`, so the watching agent never reads a question it must not
    /// answer.
    ///
    /// `false` when nothing was registered (`park` declined because no person
    /// could be asked), when the call has already been answered, or when
    /// `session_id` is this card's own home. Nothing is published in any of
    /// those cases.
    ///
    /// `pub(crate)` on purpose. Widening where a card may be answered is a
    /// decision about the delegation tree, so it belongs to the one function
    /// that knows the tree — `approval_relay::surface_where_a_person_is_watching`,
    /// which is itself crate-private. A door outside this crate that needs the
    /// behaviour should go through that, not invent a second destination.
    pub(crate) fn also_surface_in_for(&self, session_id: &str, requested_by: Option<&str>) -> bool {
        if self.declined || self.rx.is_none() {
            return false;
        }
        if !self
            .registry
            .record_escalation(&self.id, session_id, requested_by)
        {
            return false;
        }
        let Some(card) = self
            .registry
            .lock()
            .get(&self.id)
            .map(|entry| entry.card_for(&self.id, session_id))
        else {
            return false;
        };
        crate::session_events::publish(
            session_id,
            crate::session_events::SessionBusEvent::Agent(crate::agents::AgentEvent::Message(card)),
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

        let mut rx = rx;
        let answered = tokio::select! {
            biased;
            () = async {
                match cancel {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending::<()>().await,
                }
            } => None,
            answered = tokio::time::timeout(ttl, &mut rx) => Some(answered),
        };
        let outcome = match answered {
            None => UserActionOutcome::Cancelled,
            Some(Ok(Ok(outcome))) => outcome,
            // The registry entry was dropped without a decision.
            Some(Ok(Err(_))) => UserActionOutcome::Cancelled,
            Some(Err(_)) => self.expire(&mut rx).await,
        };

        // On every path but a delivered decision the entry is still registered;
        // clearing it here is what stops a late click resolving a receiver that
        // has already gone. A delivered decision already took the entry, and
        // announced its end, as did an expiry (`expire`), so this announces
        // nothing twice.
        let ending = match outcome {
            UserActionOutcome::TimedOut => CardEnding::TimedOut,
            _ => CardEnding::Withdrawn,
        };
        self.registry.forget(&self.id, ending);
        outcome
    }
}

impl PendingUserAction {
    /// The window closed. Take the card out of the registry, so no surface can
    /// answer it from here on, and tell every surface that showed it.
    ///
    /// The take and the timeout are not one instant: a click can resolve the
    /// entry between the two. Whoever took the entry holds its sender and is
    /// about to send on it (or drop it), so the decision is awaited rather than
    /// lost: honoring it beats reporting "expired" to a call whose person did
    /// answer, while the clicking surface was told it was delivered.
    async fn expire(&self, rx: &mut oneshot::Receiver<UserActionOutcome>) -> UserActionOutcome {
        match self.registry.take_unanswered(&self.id) {
            Some(surfaces) => {
                announce_card_expiry(&self.id, &self.request, surfaces);
                UserActionOutcome::TimedOut
            }
            None => rx.await.unwrap_or(UserActionOutcome::TimedOut),
        }
    }
}

/// Where an expired card was showing: its home, and the sessions it was
/// escalated into ([`Entry::escalated_to`]).
struct CardSurfaces {
    home: Option<String>,
    escalated_to: Vec<String>,
}

/// Publish [`card_expiry_notice`] wherever card `id` was published, by the
/// same channels: the home session's request queue (the agent loop drains it
/// into the chat's stream, where the card arrived) and the session bus of each
/// escalated surface. Only approval cards carry a notice; an elicitation, a
/// credential request or a Crew step keeps the shape it has.
///
/// ⚠ Never to the unscoped queue. A card parked without a home was queued
/// there, and the first chat whose loop drains it claims it as its home
/// ([`PendingUserActions::claim_unscoped_for_session`]), so a card that still
/// has none at expiry never reached any chat. That queue is drained by
/// whichever loop runs next, so a notice published there would land in a chat
/// that never drew the card, as a report on a request nobody there saw.
fn announce_card_expiry(id: &str, request: &UserActionRequest, surfaces: CardSurfaces) {
    let Some(subject) = expiry_subject(request) else {
        return;
    };
    if surfaces.home.is_none() && surfaces.escalated_to.is_empty() {
        return;
    }
    let notice = card_expiry_notice(id, &subject);
    for surface in &surfaces.escalated_to {
        crate::session_events::publish(
            surface,
            crate::session_events::SessionBusEvent::Agent(crate::agents::AgentEvent::Message(
                notice.clone(),
            )),
        );
    }
    if let Some(home) = surfaces.home.as_deref() {
        crate::action_required_manager::ActionRequiredManager::global().publish(Some(home), notice);
    }
}

/// What an expiry notice names, for the cards that carry one.
fn expiry_subject(request: &UserActionRequest) -> Option<String> {
    match request {
        UserActionRequest::ToolApproval(r) => {
            Some(format!("The permission prompt for {}", r.tool_name))
        }
        UserActionRequest::FolderAccess(r) => {
            Some(format!("The folder access request for {}", r.folder))
        }
        // A Crew step is not an approval: its end reaches a surface as `ActionRequiredEnded`.
        UserActionRequest::Elicitation(_)
        | UserActionRequest::Secrets(_)
        | UserActionRequest::CrewStep(_) => None,
    }
}

/// The notice that approval card `id` closed before anyone answered it.
///
/// One constructor for both places a card can expire: a call parked here
/// ([`PendingUserAction::wait`]) and the agent's own permission prompt
/// (`handle_approval_tool_requests`). Its text is the record a terminal prints;
/// its `expired_action_id` is what lets a surface that shows the card draw it
/// as expired, buttons gone, rather than leave three live buttons on a call
/// that has already given up (a late click answers `unknown`).
pub fn card_expiry_notice(id: &str, subject: &str) -> Message {
    Message::assistant()
        .with_content(MessageContent::card_expiry_notification(
            id,
            format!(
                "{subject} expired before it was answered. The tool was not run. Ask again to retry."
            ),
        ))
        .with_metadata(crate::conversation::message::MessageMetadata::user_only())
}

/// The approval card a [`card_expiry_notice`] reports as expired.
pub fn expired_card_id(message: &Message) -> Option<&str> {
    message.content.iter().find_map(|content| match content {
        MessageContent::SystemNotification(note) => note.expired_action_id.as_deref(),
        _ => None,
    })
}

/// Whether `message` is a [`card_expiry_notice`].
///
/// Live-only, like the card it reports on: a drain yields it into the stream
/// (and so into the turn's replay log, after the card) but never stores it.
pub fn is_card_expiry_notice(message: &Message) -> bool {
    expired_card_id(message).is_some()
}

impl Drop for PendingUserAction {
    fn drop(&mut self) {
        // wait() moves the receiver before polling; cancellation must still remove its entry.
        self.registry.forget(&self.id, CardEnding::Withdrawn);
    }
}

/// The card a surface renders for `request`.
///
/// Each variant maps onto the `ActionRequired` shape the desktop already
/// understands, so an approval raised from the bridge draws the *same* dialog as
/// one raised by the agent's own loop rather than a second, subtly different
/// one.
fn request_message(id: &str, request: &UserActionRequest, requested_by: Option<&str>) -> Message {
    let content = match request {
        UserActionRequest::FolderAccess(r) => MessageContent::action_required_with_context(
            id,
            "filesystem__folder_access".to_string(),
            serde_json::json!({"folder": r.folder, "access": r.access, "files": r.files, "actionId": r.action_id})
                .as_object().cloned().unwrap_or_default(),
            Some(r.prompt()),
            None,
            Some(ToolPreview::FolderAccess {
                folder: r.folder.clone(),
                access: r.access.clone(),
                files: r.files.clone(),
                descendants: true,
                once_descendants: r.once_descendants,
                action_id: r.action_id.clone(),
                requested_by: requested_by.map(str::to_string),
            }),
        ),
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
        // The daemon's own fields only; what the person types never comes back on it.
        UserActionRequest::CrewStep(step) => {
            MessageContent::action_required_crew_step(id, step.clone())
        }
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
///
/// Nor is a **changed host key's** Crew step ([`crew_step_is_a_notice`]): it waits for nothing
/// and offers nothing, so it is kept, user-only, as a notice a reloaded chat still shows.
pub fn is_ephemeral_card(message: &Message) -> bool {
    ephemeral_card_id(message).is_some()
}

/// The ID of the decision prompt `message` carries, when it carries one ([`is_ephemeral_card`]):
/// the ID its [`CardEnded`] names, which lets a stream match a card frame to its end.
pub fn ephemeral_card_id(message: &Message) -> Option<&str> {
    use crate::conversation::message::ActionRequiredData;
    message.content.iter().find_map(|content| match content {
        MessageContent::ActionRequired(action) => match &action.data {
            ActionRequiredData::ToolConfirmation { id, .. }
            | ActionRequiredData::SecretRequest { id, .. } => Some(id.as_str()),
            // A Crew step card exists only while the step waits; a reopened chat must never
            // offer to sign in, unlock or trust from history.
            ActionRequiredData::CrewStep { id, step } if !crew_step_is_a_notice(step) => {
                Some(id.as_str())
            }
            _ => None,
        },
        _ => None,
    })
}

/// Whether a Crew step is a notice rather than a question: a server's CHANGED host key, which
/// nothing in a chat can fix and nothing waits on (D2: a changed key is always refused). The
/// call that raised it releases it at once, so as an ephemeral card it would end before any
/// surface drew it, and a reload or a late observer would never see it. As a notice it is
/// stored when the chat's loop takes it from the queue, user-only like every card, so the
/// model never reads it; its fields are the host, the login and the two fingerprints, which
/// the call's own answer to the model already carries.
pub fn crew_step_is_a_notice(step: &CrewStepRequest) -> bool {
    matches!(step, CrewStepRequest::HostKey { changed: true, .. })
}

/// [`is_ephemeral_card`] for a request, before its card is drawn.
fn request_is_ephemeral(request: &UserActionRequest) -> bool {
    match request {
        UserActionRequest::Elicitation(_) => false,
        UserActionRequest::CrewStep(step) => !crew_step_is_a_notice(step),
        UserActionRequest::ToolApproval(_)
        | UserActionRequest::FolderAccess(_)
        | UserActionRequest::Secrets(_) => true,
    }
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
        let card = request_message("req-1", &secrets(), None);
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

    fn crew_sign_in() -> UserActionRequest {
        use crate::conversation::message::CrewSignInTarget;
        UserActionRequest::CrewStep(CrewStepRequest::SignIn {
            target: CrewSignInTarget::Setup {
                job_id: "job-1".to_string(),
            },
            server: "hpc.example.edu".to_string(),
            login: "you@hpc.example.edu".to_string(),
        })
    }

    /// The owner's D1 ruling: a Crew step card can never be answered as a success, from any
    /// surface, proven or not. Its parked call learns what happened from the daemon only.
    #[tokio::test]
    async fn a_crew_step_refuses_every_answer_that_could_mean_done() {
        let registry = Arc::new(PendingUserActions::default());
        let parked = registry.park(Some("sess-1"), None, crew_sign_in());
        let id = parked.id().to_string();
        let forged = [
            UserActionOutcome::Approved {
                permission: Permission::AllowOnce,
            },
            UserActionOutcome::Denied {
                permission: Permission::DenyOnce,
            },
            UserActionOutcome::Provided {
                data: serde_json::json!({"password": "hunter2"}),
            },
            UserActionOutcome::SecretsConfigured {
                configured_keys: vec!["passphrase".to_string()],
            },
        ];
        for outcome in forged {
            for authority in [
                DecisionAuthority::unproven(),
                DecisionAuthority::for_test_proven(),
            ] {
                assert_eq!(
                    registry.resolve_in_session("sess-1", &id, outcome.clone(), authority),
                    ResolveOutcome::Rejected,
                    "{outcome:?}"
                );
                assert!(registry.is_pending(&id), "the caller must stay parked");
            }
        }
        assert_eq!(
            registry.resolve_in_session(
                "sess-1",
                &id,
                UserActionOutcome::Cancelled,
                DecisionAuthority::unproven()
            ),
            ResolveOutcome::Delivered
        );
        let outcome = parked.wait(Duration::from_secs(5), None).await;
        assert_eq!(outcome, UserActionOutcome::Cancelled);
        assert!(!outcome.is_allowed());
    }

    #[test]
    fn a_crew_step_ends_only_without_an_answer() {
        let request = crew_sign_in();
        for ending in [
            UserActionOutcome::Cancelled,
            UserActionOutcome::TimedOut,
            UserActionOutcome::Failed {
                reason: "no surface".to_string(),
            },
        ] {
            assert!(request.accepts(&ending), "{ending:?}");
            assert!(!ending.is_allowed());
        }
        assert!(request.requires_user_proof());
        assert_eq!(request.describe(), "a Crew sign-in");
        assert!(!request.describe().contains("example.edu"));
    }

    /// The card is the person's alone: the model never reads it, and it is never stored.
    #[test]
    fn a_crew_step_card_is_user_only_and_ephemeral() {
        let card = request_message("req-1", &crew_sign_in());
        assert!(card.is_user_visible());
        assert!(!card.is_agent_visible());
        assert!(is_ephemeral_card(&card));
    }

    /// Dismissing is the one answer, and only for a Crew step this session shows: it cannot
    /// reach an approval card, another kind of card, or another session's step.
    #[tokio::test]
    async fn only_a_crew_step_in_its_own_session_can_be_dismissed() {
        let registry = Arc::new(PendingUserActions::default());
        let approval_card = registry.park(Some("sess-1"), None, approval("developer__shell"));
        assert_eq!(
            registry.dismiss_crew_step("sess-1", approval_card.id()),
            ResolveOutcome::Unknown
        );
        assert!(registry.is_pending(approval_card.id()));

        let step = registry.park(Some("sess-1"), None, crew_sign_in());
        let id = step.id().to_string();
        assert_eq!(
            registry.dismiss_crew_step("sess-other", &id),
            ResolveOutcome::Unknown
        );
        assert_eq!(
            registry.dismiss_crew_step("sess-1", "no-such-card"),
            ResolveOutcome::Unknown
        );
        assert_eq!(
            registry.dismiss_crew_step("sess-1", &id),
            ResolveOutcome::Delivered
        );
        assert_eq!(
            step.wait(Duration::from_secs(5), None).await,
            UserActionOutcome::Cancelled
        );
    }

    /// Approval and credential cards must not be written into history;
    /// elicitations must, because their answer row references them.
    #[test]
    fn only_decision_prompts_are_ephemeral() {
        assert!(is_ephemeral_card(&request_message(
            "1",
            &approval("developer__shell"),
            None
        )));
        assert!(is_ephemeral_card(&request_message("2", &secrets(), None)));
        assert!(!is_ephemeral_card(&request_message(
            "3",
            &UserActionRequest::Elicitation(ElicitationRequest {
                message: "Which cohort?".to_string(),
                requested_schema: serde_json::json!({}),
            }),
            None
        )));
        assert!(!is_ephemeral_card(&Message::assistant().with_text("hi")));
    }

    // ------------------------------------------------------------------ WI-17: a card's end

    fn host_key(changed: bool) -> UserActionRequest {
        UserActionRequest::CrewStep(CrewStepRequest::HostKey {
            host: "hpc.example.edu".to_string(),
            port: 22,
            login: "you".to_string(),
            offered: Some(format!("SHA256:{}", "A".repeat(43))),
            known: None,
            changed,
        })
    }

    /// Every end heard so far on `endings`.
    fn heard(endings: &mut broadcast::Receiver<CardEnded>) -> Vec<CardEnded> {
        std::iter::from_fn(|| endings.try_recv().ok()).collect()
    }

    fn ended(session: &str, id: &str, ending: CardEnding) -> CardEnded {
        CardEnded {
            session_id: session.to_string(),
            id: id.to_string(),
            ending,
        }
    }

    /// A chat ID no other test uses: the queue a park publishes to is process-global.
    fn chat(label: &str) -> String {
        format!("wi17-{label}-{}", Uuid::new_v4())
    }

    /// Every way an ephemeral card leaves the registry announces its end once, with how it
    /// ended and nothing else: answered, dismissed, out of time, withdrawn by the call (its
    /// cancel token, or dropping the handle), and the chat or the turn closing. A delivered
    /// decision announces once, not again when the waiter clears up after it.
    #[tokio::test]
    async fn every_way_a_card_ends_is_announced_once() {
        let registry = Arc::new(PendingUserActions::default());
        let mut endings = registry.subscribe_card_endings();
        let s = chat("ends");

        let answered = registry.park(Some(&s), None, approval("developer__shell"));
        let id = answered.id().to_string();
        let allow = UserActionOutcome::Approved {
            permission: Permission::AllowOnce,
        };
        assert_eq!(
            registry.resolve_in_session(&s, &id, allow, DecisionAuthority::for_test_proven()),
            ResolveOutcome::Delivered
        );
        assert!(answered
            .wait(Duration::from_secs(5), None)
            .await
            .is_allowed());
        assert_eq!(heard(&mut endings), [ended(&s, &id, CardEnding::Answered)]);
        // Remembered, so a stream can leave out a copy its chat's loop draws later; a card that
        // was never parked here (the agent's own loop raises some) is not.
        assert!(registry.has_ended(&id));
        assert!(!registry.has_ended("a-card-the-agent-loop-raised"));

        let dismissed = registry.park(Some(&s), None, crew_sign_in());
        let id = dismissed.id().to_string();
        assert_eq!(
            registry.dismiss_crew_step(&s, &id),
            ResolveOutcome::Delivered
        );
        dismissed.wait(Duration::from_secs(5), None).await;
        assert_eq!(heard(&mut endings), [ended(&s, &id, CardEnding::Dismissed)]);

        let late = registry.park(Some(&s), None, crew_sign_in());
        let id = late.id().to_string();
        assert_eq!(
            late.wait(Duration::from_millis(5), None).await,
            UserActionOutcome::TimedOut
        );
        assert_eq!(heard(&mut endings), [ended(&s, &id, CardEnding::TimedOut)]);

        let withdraw = CancellationToken::new();
        let withdrawn = registry.park(Some(&s), None, crew_sign_in());
        let id = withdrawn.id().to_string();
        withdraw.cancel();
        withdrawn
            .wait(Duration::from_secs(5), Some(&withdraw))
            .await;
        assert_eq!(heard(&mut endings), [ended(&s, &id, CardEnding::Withdrawn)]);

        let dropped = registry.park(Some(&s), None, secrets());
        let id = dropped.id().to_string();
        drop(dropped);
        assert_eq!(heard(&mut endings), [ended(&s, &id, CardEnding::Withdrawn)]);

        let stopped = registry.park(Some(&s), None, approval("developer__shell"));
        let id = stopped.id().to_string();
        assert_eq!(registry.cancel_session(&s), 1);
        assert_eq!(heard(&mut endings), [ended(&s, &id, CardEnding::Withdrawn)]);
        drop(stopped);
        assert!(heard(&mut endings).is_empty(), "a card ends once");
    }

    /// The end goes to every chat that drew the card (its home and the chat it was escalated
    /// into) and to no other; a card that is a record rather than a question (an elicitation,
    /// a changed host key's notice) announces nothing, since its stored row stays.
    #[tokio::test]
    async fn an_end_reaches_exactly_the_chats_that_drew_the_card() {
        let registry = Arc::new(PendingUserActions::default());
        let mut endings = registry.subscribe_card_endings();
        let (child_chat, parent_chat) = (chat("child"), chat("parent"));

        let child = registry.park(Some(&child_chat), None, approval("developer__shell"));
        let id = child.id().to_string();
        assert!(child.also_surface_in(&parent_chat));
        drop(child);
        assert_eq!(
            heard(&mut endings),
            [
                ended(&child_chat, &id, CardEnding::Withdrawn),
                ended(&parent_chat, &id, CardEnding::Withdrawn),
            ]
        );

        let s = chat("records");
        let question = registry.park(
            Some(&s),
            None,
            UserActionRequest::Elicitation(ElicitationRequest {
                message: "Which cohort?".to_string(),
                requested_schema: serde_json::json!({}),
            }),
        );
        drop(question);
        let notice = registry.park(Some(&s), None, host_key(true));
        let notice_id = notice.id().to_string();
        drop(notice);
        assert!(heard(&mut endings).is_empty());
        assert!(
            !registry.has_ended(&notice_id),
            "a notice is drawn, not left out"
        );
    }

    /// A changed host key's card is a notice: nothing waits on it and the call releases it at
    /// once, so it is not ephemeral and the chat's loop stores it, user-only, when it drains it
    /// (WI-17). An unknown key's card is a question and stays ephemeral. Either way the model
    /// never reads it.
    #[test]
    fn a_changed_host_key_is_a_stored_notice_and_an_unknown_one_is_a_question() {
        let notice = request_message("n-1", &host_key(true));
        assert!(!is_ephemeral_card(&notice));
        assert_eq!(ephemeral_card_id(&notice), None);
        assert!(notice.is_user_visible());
        assert!(!notice.is_agent_visible());

        let question = request_message("q-1", &host_key(false));
        assert!(is_ephemeral_card(&question));
        assert_eq!(ephemeral_card_id(&question), Some("q-1"));
        assert_eq!(
            ephemeral_card_id(&request_message("a-1", &approval("developer__shell"))),
            Some("a-1")
        );
        assert_eq!(
            ephemeral_card_id(&request_message("c-1", &secrets())),
            Some("c-1")
        );
    }

    /// A changed key's notice, released at once, is still in the chat's queue for its loop to
    /// draw, and a late observer is never handed it as a parked card (it waits for nothing).
    #[tokio::test]
    async fn a_released_notice_is_still_drawn_but_never_replayed_as_parked() {
        use crate::action_required_manager::ActionRequiredManager;
        let session = format!("notice-{}", Uuid::new_v4());
        let registry = Arc::clone(PendingUserActions::global());
        drop(registry.park(Some(&session), None, host_key(true)));
        assert!(registry.pending_cards_for_session(&session).is_empty());
        let drawn = ActionRequiredManager::global().drain_requests(&session);
        assert_eq!(drawn.len(), 1);
        assert!(!is_ephemeral_card(&drawn[0]), "the loop stores it");
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

    fn folder_card(folder: &str) -> UserActionRequest {
        UserActionRequest::FolderAccess(FolderAccessRequest {
            action_id: "synthetic-action".to_string(),
            tool_name: "developer__text_editor".to_string(),
            folder: folder.to_string(),
            access: "read-only".to_string(),
            once_descendants: false,
            files: vec![format!("{folder}/notes.txt")],
        })
    }

    fn allow_once() -> UserActionOutcome {
        UserActionOutcome::Approved {
            permission: Permission::AllowOnce,
        }
    }

    fn card_id(message: &Message) -> Option<String> {
        message.content.iter().find_map(|content| match content {
            MessageContent::ActionRequired(action) => match &action.data {
                crate::conversation::message::ActionRequiredData::ToolConfirmation {
                    id, ..
                } => Some(id.clone()),
                _ => None,
            },
            _ => None,
        })
    }

    /// The bug the decisions lane found live: a folder card ("Allow / Allow
    /// for this session / Deny") kept its buttons after its 300 s window
    /// closed, and a late click got `unknown` and collapsed to "no longer
    /// available". The window closing must reach every surface that drew the
    /// card, by the channel the card took, naming the card, so the surface can
    /// drop its buttons; and from then on no surface may answer it.
    ///
    /// `start_paused` lets the real 300 s window elapse at once.
    #[tokio::test(start_paused = true)]
    async fn an_unanswered_folder_card_announces_its_expiry_wherever_it_was_shown() {
        let home = format!("expiry-home-{}", Uuid::new_v4());
        let watcher = format!("expiry-watcher-{}", Uuid::new_v4());
        let mut watching = crate::session_events::subscribe(&watcher);
        let registry = Arc::new(PendingUserActions::default());
        let parked = registry.park(Some(&home), None, folder_card("/synthetic/results"));
        let id = parked.id().to_string();
        assert!(parked.also_surface_in(&watcher));

        assert_eq!(
            parked.wait(Duration::from_secs(300), None).await,
            UserActionOutcome::TimedOut
        );

        // Home: the request queue the agent loop drains into the chat's
        // stream, the card first and its expiry after it.
        let drained =
            crate::action_required_manager::ActionRequiredManager::global().drain_requests(&home);
        let card_at = drained
            .iter()
            .position(|message| card_id(message).as_deref() == Some(id.as_str()))
            .expect("the card was published to its home");
        let notice_at = drained
            .iter()
            .position(is_card_expiry_notice)
            .expect("the expiry must be published where the card was");
        assert!(card_at < notice_at, "the expiry follows its card");
        let notice = &drained[notice_at];
        assert_eq!(expired_card_id(notice), Some(id.as_str()));
        assert!(notice.metadata.user_visible && !notice.metadata.agent_visible);
        assert!(
            matches!(
                notice.content.as_slice(),
                [MessageContent::SystemNotification(note)]
                    if note.notification_type
                        == crate::conversation::message::SystemNotificationType::InlineMessage
                        && note.msg.contains("/synthetic/results")
            ),
            "the notice is one inline notice naming what expired: {:?}",
            notice.content
        );

        // The escalated surface hears it on the bus the card reached it by.
        let mut heard = Vec::new();
        while let Ok(crate::session_events::SessionBusEvent::Agent(
            crate::agents::AgentEvent::Message(message),
        )) = watching.try_recv()
        {
            heard.push(message);
        }
        assert_eq!(heard.len(), 2, "{heard:?}");
        assert_eq!(card_id(&heard[0]).as_deref(), Some(id.as_str()));
        assert_eq!(expired_card_id(&heard[1]), Some(id.as_str()));

        // And a late click answers nothing, from either surface.
        for surface in [&home, &watcher] {
            assert_eq!(
                registry.resolve_in_session(
                    surface,
                    &id,
                    allow_once(),
                    DecisionAuthority::unproven()
                ),
                ResolveOutcome::Unknown
            );
        }
    }

    /// An answered card announces nothing, and neither does a cancelled one:
    /// only a window that closed unanswered is an expiry.
    #[tokio::test]
    async fn only_an_unanswered_window_is_announced_as_expired() {
        let home = format!("expiry-answered-{}", Uuid::new_v4());
        let registry = Arc::new(PendingUserActions::default());

        let answered = registry.park(Some(&home), None, approval("developer__shell"));
        let id = answered.id().to_string();
        assert_eq!(
            registry.resolve_in_session(&home, &id, allow_once(), DecisionAuthority::unproven()),
            ResolveOutcome::Delivered
        );
        assert_eq!(
            answered.wait(Duration::from_millis(30), None).await,
            allow_once()
        );

        let token = CancellationToken::new();
        let cancelled = registry.park(Some(&home), None, approval("developer__shell"));
        token.cancel();
        assert_eq!(
            cancelled.wait(Duration::from_secs(5), Some(&token)).await,
            UserActionOutcome::Cancelled
        );

        let drained =
            crate::action_required_manager::ActionRequiredManager::global().drain_requests(&home);
        assert!(!drained.iter().any(is_card_expiry_notice), "{drained:?}");
    }

    /// An elicitation or a credential request is not an approval card; its
    /// surfaces keep the shape they have, so nothing is announced for it.
    #[tokio::test]
    async fn a_credential_request_that_times_out_announces_nothing() {
        let home = format!("expiry-secrets-{}", Uuid::new_v4());
        let registry = Arc::new(PendingUserActions::default());
        let parked = registry.park(Some(&home), None, secrets());
        assert_eq!(
            parked.wait(Duration::from_millis(20), None).await,
            UserActionOutcome::TimedOut
        );
        let drained =
            crate::action_required_manager::ActionRequiredManager::global().drain_requests(&home);
        assert!(!drained.iter().any(is_card_expiry_notice), "{drained:?}");
    }

    /// A card parked with no home is queued unscoped, and no chat claimed it
    /// before its window closed, so no chat drew it. Its expiry notice must not
    /// go to the unscoped queue, where whichever loop drains next would show it
    /// in a chat that never saw the card. It goes only to a chat the card was
    /// escalated into, and with none, nowhere.
    ///
    /// The unscoped queue is shared by every test in this binary, so this test
    /// takes out only its own card and its own notice, by ID.
    #[tokio::test(start_paused = true)]
    async fn a_card_no_chat_drew_announces_its_expiry_only_where_it_was_escalated() {
        use crate::action_required_manager::ActionRequiredManager;
        let manager = ActionRequiredManager::global();
        let registry = Arc::new(PendingUserActions::default());

        // Shown nowhere: no home, no escalation.
        let nowhere = registry.park(None, None, approval("developer__shell"));
        let nowhere_id = nowhere.id().to_string();
        manager.take_unscoped(|message| card_id(message).as_deref() == Some(nowhere_id.as_str()));
        assert_eq!(
            nowhere.wait(Duration::from_secs(300), None).await,
            UserActionOutcome::TimedOut
        );
        let leaked =
            manager.take_unscoped(|message| expired_card_id(message) == Some(nowhere_id.as_str()));
        assert!(
            leaked.is_empty(),
            "an expiry no chat can place must not be queued for any chat: {leaked:?}"
        );

        // No home, but escalated into a watching chat: that chat hears it, and
        // the unscoped queue still gets nothing.
        let watcher = format!("expiry-unscoped-watcher-{}", Uuid::new_v4());
        let mut watching = crate::session_events::subscribe(&watcher);
        let escalated = registry.park(None, None, folder_card("/synthetic/unscoped"));
        let escalated_id = escalated.id().to_string();
        manager.take_unscoped(|message| card_id(message).as_deref() == Some(escalated_id.as_str()));
        assert!(escalated.also_surface_in(&watcher));
        assert_eq!(
            escalated.wait(Duration::from_secs(300), None).await,
            UserActionOutcome::TimedOut
        );
        let leaked = manager
            .take_unscoped(|message| expired_card_id(message) == Some(escalated_id.as_str()));
        assert!(leaked.is_empty(), "{leaked:?}");
        let mut heard = Vec::new();
        while let Ok(crate::session_events::SessionBusEvent::Agent(
            crate::agents::AgentEvent::Message(message),
        )) = watching.try_recv()
        {
            heard.push(message);
        }
        assert_eq!(
            heard.iter().filter_map(expired_card_id).collect::<Vec<_>>(),
            [escalated_id.as_str()],
            "{heard:?}"
        );
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

    #[tokio::test]
    async fn folder_scoped_authority_cannot_approve_other_protected_actions() {
        let registry = Arc::new(PendingUserActions::default());
        let protected = registry.park(Some("chat"), None, a_proof_backed_approval());
        assert_eq!(
            registry.resolve_in_session(
                "chat",
                protected.id(),
                allow(),
                DecisionAuthority::from_folder_access_proof()
            ),
            ResolveOutcome::Unproven
        );
        let folder = registry.park(
            Some("chat"),
            None,
            UserActionRequest::FolderAccess(FolderAccessRequest {
                action_id: "operation".into(),
                tool_name: "developer__text_editor".into(),
                folder: "/synthetic/results".into(),
                access: "read-only".into(),
                once_descendants: false,
                files: vec!["/synthetic/results/completed.csv".into()],
            }),
        );
        assert_eq!(
            registry.resolve_in_session(
                "chat",
                folder.id(),
                allow(),
                DecisionAuthority::from_folder_access_proof()
            ),
            ResolveOutcome::Delivered
        );
    }

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

    /// R2-1 (live QA, 2026-11-10): a subagent's folder card shown in the parent
    /// read "Allow this chat to read …?", as if the parent's own agent asked.
    /// The escalated copy, live and replayed, names the subagent; the card's
    /// home, where the asker is that chat's own agent, does not.
    #[tokio::test]
    async fn an_escalated_folder_card_names_the_subagent_that_asks() {
        let registry = Arc::new(PendingUserActions::default());
        let parked = registry.park(
            Some("child"),
            None,
            UserActionRequest::FolderAccess(FolderAccessRequest {
                action_id: "operation".into(),
                tool_name: "developer__text_editor".into(),
                folder: "/synthetic/notes".into(),
                access: "read-only".into(),
                once_descendants: false,
                files: vec!["/synthetic/notes/a.txt".into()],
            }),
        );
        assert!(parked.also_surface_in_for("parent", Some(" notes-reader ")));

        let card_json = |session: &str| {
            let cards = registry.pending_cards_for_session(session);
            assert_eq!(cards.len(), 1, "one card at {session}");
            serde_json::to_string(&cards[0]).expect("serialisable")
        };
        assert!(
            card_json("parent").contains(r#""requestedBy":"notes-reader""#),
            "the parent's copy names the subagent: {}",
            card_json("parent")
        );
        assert!(
            !card_json("child").contains("requestedBy"),
            "the home copy is the asker's own chat and names nobody"
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
        let folder = concat!("from_folder_", "access_proof(");

        let mut proven_files: std::collections::BTreeMap<String, usize> = Default::default();
        let mut local_files: std::collections::BTreeMap<String, usize> = Default::default();
        let mut folder_files: std::collections::BTreeMap<String, usize> = Default::default();
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
            // Integration fixtures supply synthetic human answers; production
            // constructor locations remain exhaustively checked below.
            if rel == "crates/biorouter/src/pending_user_action.rs"
                || rel == "crates/biorouter/tests/folder_access_authorization.rs"
                || rel == "crates/biorouter/tests/folder_analysis_authorization.rs"
            {
                continue;
            }
            let body = std::fs::read_to_string(p).unwrap();
            scanned += 1;
            let p_count = body.matches(proven).count();
            let l_count = body.matches(local).count();
            let folder_count = body.matches(folder).count();
            if p_count > 0 {
                proven_files.insert(rel.clone(), p_count);
            }
            if l_count > 0 {
                local_files.insert(rel.clone(), l_count);
            }
            if folder_count > 0 {
                folder_files.insert(rel.clone(), folder_count);
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
            folder_files.keys().cloned().collect::<Vec<_>>(),
            vec!["crates/biorouter-server/src/routes/action_required.rs".to_string()],
            "folder authority must only be minted in the typed approval route"
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
