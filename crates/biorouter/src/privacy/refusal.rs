//! The refusals the privacy gates return (issue #56).
//!
//! One module rather than one per gate, because a refusal is the *same object*
//! at three different altitudes: a typed Rust error the caller can branch on, a
//! typed HTTP body the GUI renders a repair card from, and — for Gate C — a
//! string the model reads. Keeping them together is what lets §14.4's rule
//! ("never leak content in a refusal": name the tool and the tier, never a
//! session title or a working directory) be checked by reading one file.
//!
//! Later tasks add one item each and say which:
//!
//! | Task | Adds |
//! |---|---|
//! | 12 (this one) | [`PrivacyRefusal`] and its two accessors |
//! | 13 | `turn_refusal(&Session) -> String`, and Task 10's `CHATRECALL_LOAD_REFUSAL` moves here |
//! | 14 | `privacy_refusal(extension, extension_tier, caller_tier) -> Option<ErrorData>` |
//! | 18A | [`ASK_THE_USER_TO_SWITCH`], [`raise_needs_user_action`] and the three HTTP-channel variants |
//! | 23 | the two spawn variants and `PrivacyRefusal::spawn_upgrade` / `spawn_downgrade` |
//! | 41 | [`PrivacyRefusal::AppSessionTierFixed`], DR-21's app-runtime refusal |
//! | DR-31 | [`PrivacyRefusal::SpawnCrossesAffiliation`] and [`PrivacyRefusal::spawn_affiliation`] — the spawn gate's third axis |
//! | test drive M18 | [`private_or_absent_refusal`] — [`privacy_refusal`]'s sentence, restated for the ONE gate that reads an unknown name as Private and therefore cannot claim the extension is private |
//! | findings 4+13's seam | [`extension_enable_refusal`] — the WHOLE enable gate, tier arm above the operator pin, called by both agent enable doors — and [`tier_refuses`], the boolean under [`privacy_refusal`] that the user's HTTP enable door asks instead of re-typing |

use super::{ProviderTier, SessionClassification};
use crate::session::session_manager::Session;
use rmcp::model::{ErrorCode, ErrorData};

/// The one sentence every refusal in this feature ends on. DR-16 turns step 1 of
/// the two-ways-out message from something the model DOES into something it
/// SAYS, so the wording is shared rather than re-typed per call site —
/// including by Task 18's `check_enable_allowed` arm, which Task 18A rewrote to
/// use it. Two audiences, one vocabulary.
pub const ASK_THE_USER_TO_SWITCH: &str =
    "ask the user to switch this chat to a private model first, in the desktop app under \
     Settings > Models, or with the model chip in the composer.";

/// The substring that marks a refusal as *"this request carried no proof it came
/// from the user"*, as opposed to any other failure of the same route.
///
/// It exists for the same reason [`TURN_REFUSAL_MARKER`] does: two independent
/// readers key on this text. The model reads the whole sentence; the desktop
/// renderer (`ModelAndProviderContext.changeModel`) has to tell this refusal
/// apart from a 500 so it can explain the *cause* — a backend the app did not
/// start, and therefore one that was handed no user-action key (open question
/// 23) — instead of reporting a policy refusal as `${error}` under a
/// "provider/model failed" title.
///
/// A substring is all the renderer has: under `throwOnError` the generated
/// client throws the parsed BODY, not the response, so the 409 status never
/// reaches the catch arm. Gate A's refusal is distinguishable because its body
/// is a typed JSON object; these two are plain text, and plain text is also what
/// a 500 from the same route carries.
///
/// ⚠ Mirrored verbatim in `ui/desktop/src/utils/userAction.ts`. A reword that
/// dropped it here would put every refused switch back on the generic error
/// toast, with nothing failing.
pub const USER_ACTION_REFUSAL_MARKER: &str = "is the user's decision, not yours";

/// The frame that marks a cross-affiliation refusal as one the user can **clear
/// by accepting it**, and that carries the extension key they would be accepting
/// (issue #56, DR-26 / Task 57).
///
/// It exists because DR-26's ruling is *warn, then allow if the user insists*,
/// and until Task 57 the second half had no affordance: the refusal arrived, the
/// model was told to ask, and the person at the keyboard had no button. Their
/// only route past it was to switch the chat's model — a hard block wearing a
/// warning's clothes, which is the control-people-route-around failure DR-19
/// exists to prevent.
///
/// ⚠ **It is a frame, not a flag.** The renderer needs two things and the tool
/// result carries only text: *is this refusal grantable*, and *which extension
/// key does the grant name*. `ErrorData::data` cannot carry the second — the
/// wire form of a failed tool call is `{status, error, error_kind, retryable}`
/// (`conversation::tool_result_serde`), which keeps the message and drops the
/// structured payload. So the key rides immediately after this marker, in
/// backticks, and `only_the_refusal_a_grant_can_clear_offers_the_accept_marker`
/// pins that shape rather than merely the marker's presence.
///
/// ⚠ **Present only where a grant would actually clear the call** — see
/// [`cross_affiliation_refusal`]'s `acceptable` argument. A button on a refusal
/// no grant is consulted for would record a real acceptance and leave the retry
/// refused, which is the bug it fixes, inverted.
///
/// ⚠ Mirrored verbatim in `ui/desktop/src/utils/crossAffiliation.ts`. A reword
/// that dropped it here would put the accept control back out of reach with
/// nothing failing on this side.
pub const CROSS_AFFILIATION_ACCEPT_MARKER: &str = "The flow they would be approving is ";

/// DR-16's rule, as a predicate: raising a session's capability to Private is
/// the user's act alone, and only an **upward** bind is a raise.
///
/// Sideways (`Public → Public`, `Private → Private`) and downward
/// (`Private → Public`) binds are untouched **for every caller**, which is what
/// keeps Gate A's own path, the CLI, `restore_provider_from_session` and every
/// apps-runtime bind working exactly as before. Downward is Gate A's job, not
/// this one.
pub const fn raise_needs_user_action(current: ProviderTier, incoming: ProviderTier) -> bool {
    !current.is_private() && incoming.is_private()
}

/// A privacy boundary refused an operation.
///
/// It is an ordinary `std::error::Error` carried inside `anyhow::Error`, so
/// every existing caller keeps its `Result<()>` signature and the one place
/// that needs to tell a refusal from a database failure — the
/// `/agent/update_provider` handler, which owes the GUI a 409 rather than a
/// 500 — asks with `downcast_ref`.
///
/// ⚠ The payload is deliberately thin: a session **id** and a provider **name**.
/// Refusals reach the model's context (Gate C) and the user's screen, and §14.4
/// forbids either carrying conversation content.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrivacyRefusal {
    /// Gate A: a public model was offered to a session already classified
    /// private. The classification is a permanent ratchet, so this is not
    /// retryable — the way forward is a private model, or declassifying the
    /// session.
    #[error(
        "this chat is private, so it cannot be switched to `{provider}`, which is a public model"
    )]
    PublicModelOnPrivateSession {
        session_id: String,
        provider: String,
    },

    /// DR-16 / Task 18A: `POST /agent/update_provider` was asked to raise this
    /// session's capability to Private, and the request carried no proof it came
    /// from the person at the keyboard.
    ///
    /// Model-facing: it is rendered as the 409 body and reaches the model as a
    /// tool result. It names only the provider the caller itself named — which
    /// is what `{requested}` is, and why it is rendered rather than carried: an
    /// un-rendered field made `a_refusal_names_nothing_the_caller_did_not_ask_for`
    /// pass against any implementation that merely avoided typing one of three
    /// provider names into a constant.
    #[error(
        "Switching this chat to a private model {}. The request to switch it to '{requested}' \
         did not come from the model picker, so the chat is unchanged and still on its current \
         model. Do not retry; the same call will be refused again. If this task genuinely needs \
         a private model, stop and {}",
        USER_ACTION_REFUSAL_MARKER,
        ASK_THE_USER_TO_SWITCH
    )]
    TierRaiseNeedsUser { requested: String },

    /// DR-16 at the **model-facing tool** surface: `workspace_set_tools` was
    /// asked to bind a private provider to a conversation classified public,
    /// which raises that conversation's capability to Private.
    ///
    /// ⚠ **Refused outright, with no user-proof branch, and the difference from
    /// [`Self::TierRaiseNeedsUser`] is the point.** That sibling guards an HTTP
    /// route, where the very same request *can* arrive carrying `X-User-Action`
    /// — so it asks a question the caller has a way to answer. This one guards
    /// a tool the model calls: there is no person on the other end of a tool
    /// call, so `is_user_action` is false on every one of them and a proof
    /// check here could only ever refuse. A check that can only ever refuse is
    /// a refusal wearing a question's clothes, and SD-8's ruling — *a control
    /// that can never work here says so, before it is touched* — says to write
    /// the refusal.
    ///
    /// It names the conversation because, unlike the HTTP sibling, it is about
    /// a chat **other** than the caller's: "this chat is unchanged" would name
    /// the wrong one, and a model told the wrong conversation was refused
    /// retries against the right one. §14.4 is satisfied — an id and a provider
    /// name, no conversation content — and the caller already held both (it
    /// passed them in), so nothing is disclosed that it did not itself supply.
    #[error(
        "Conversation '{session_id}' is a public chat, and switching it to '{provider}' — a \
         private model — would raise what that conversation is allowed to reach. That {}. It is \
         unchanged and still on its current model. Do not retry; the same call will be refused \
         again, and it is refused the same way through every workspace tool. If that \
         conversation genuinely needs a private model, stop and ask the user to switch it \
         themselves, in that conversation's own model picker.",
        USER_ACTION_REFUSAL_MARKER
    )]
    ToolTierRaise {
        session_id: String,
        provider: String,
    },

    /// DR-16 / Task 18A: `POST /agent/add_extension` was asked to attach a
    /// private extension to a session running on a public model.
    ///
    /// Refused **outright**, with no user-proof branch: attaching a private
    /// extension to a public session is not a raise the user can authorize
    /// either. The way forward is to switch the model first, then attach.
    ///
    /// Same shape as Gate F1's `extensionmanager__manage_extensions` refusal, so
    /// the two channels speak with one voice.
    #[error(
        "Extension '{name}' is a private extension: the Biorouter marketplace marks it as \
         reaching data held inside the institution, so only a private model may enable or call \
         it. This session is running on a public model, so the extension was not attached. Do not \
         retry. If it is needed for this task, {}",
        ASK_THE_USER_TO_SWITCH
    )]
    PrivateExtensionOverHttp { name: String },

    /// DR-16 / Task 18A, open question 24: an HTTP config write named a key that
    /// decides what privacy capability new chats start at
    /// (`privacy::config_keys::CAPABILITY_CONFIG_KEYS`), and carried no
    /// user-proof.
    #[error(
        "'{key}' decides what privacy level new chats start at, so changing it {}. The setting \
         is unchanged. Do not retry. If *this* task needs a private model, that is a per-chat \
         change and not a default: {}",
        USER_ACTION_REFUSAL_MARKER,
        ASK_THE_USER_TO_SWITCH
    )]
    CapabilityConfigNeedsUser { key: String },

    /// R4 / §8.2: a public-capability session may never gain private reach, not
    /// even through a child it spawns. A subagent is an extension of the chat
    /// that started it, so letting one reach further would make the boundary a
    /// formality — spawn a child on a private model, have it read what the
    /// parent may not, and hand the answer back as a summary.
    ///
    /// `requested` is the CHILD's tier, never the parent's, so the message can
    /// say what was asked for without naming the parent's provider (R10's
    /// disclosure bound: a refusal must not become a classification oracle).
    #[error(
        "This chat is running on a public model, so it cannot delegate to a private one: a \
         subagent may never reach further than the chat that started it. No subagent was \
         started and this chat is unchanged. Do not retry; the same call will be refused \
         again. If this task genuinely needs a private model, stop and {}",
        ASK_THE_USER_TO_SWITCH
    )]
    PrivateChildOfPublicParent { requested: ProviderTier },

    /// DR-19: a private-capability session may not hand its task prompt to a
    /// public model the **model** chose.
    ///
    /// R4 permits a private session to have public children; it says nothing
    /// about who may ask for one. DR-19 supplies the initiator R4 never named,
    /// and here there is only ever one: a subagent spawn is a tool call, and no
    /// shipped surface lets a human spawn a subagent and pick its provider. So
    /// this is an agent-initiated send of private-origin text to a public model,
    /// which DR-19's agent half refuses outright — there being no in-process
    /// channel to escalate on (Task 18A's `X-User-Action` is an HTTP header, and
    /// the approval machinery is unlockable by hooks the same agent can author).
    ///
    /// It ends on the user's route rather than on [`ASK_THE_USER_TO_SWITCH`],
    /// and that difference is deliberate: this chat is *already* private, so
    /// "switch to a private model" would be advice it has already taken. The
    /// route out is a second chat, which costs the user two steps and not a
    /// wall. The message names neither the parent's provider (R10: a refusal
    /// must not become a classification oracle) nor the prompt — the prompt is
    /// the thing being withheld.
    #[error(
        "This chat is private, so it cannot start a subagent on a public model: the task \
         prompt is private-origin text, sending it would put that text on a model outside \
         the institution, and this request came from the assistant rather than from the \
         person at the keyboard. No subagent was started and this chat is unchanged. Do not \
         retry; the same call will be refused again, and no setting, hook or permission mode \
         changes it. If the task really belongs on that model, tell the user so: they can \
         start a new chat on it and give it the task directly."
    )]
    PublicChildOfPrivateParent { requested: ProviderTier },

    /// DR-21 / Task 41: a Biorouter **app** session was asked to move to a more
    /// private model than the one it was created on.
    ///
    /// An app session's capability tier is decided at session creation and
    /// cannot be changed afterwards — not by a manifest edit, not by a
    /// reconnect, not by a client frame. All three channels are agent-authored:
    /// `agent_drafter__declare_profiles` lets a Public model write the
    /// manifest's `model`, and `ClientFrame::ModelSelect` arrives on
    /// `GET /apps/{id}/agent`, a route exempt from secret-key auth entirely. So
    /// there is no user-proof branch here and there deliberately is not one: a
    /// per-manifest grant would be exactly the scoped permission DR-21 refused
    /// to invent.
    ///
    /// The way out is **not** [`ASK_THE_USER_TO_SWITCH`] — an app has no model
    /// picker, and switching this session is the very thing that is refused.
    /// It is a fresh app session, which is the same shape as
    /// [`Self::PublicChildOfPrivateParent`]'s "start a new chat".
    ///
    /// §14.4: it names the provider the caller itself asked for and nothing
    /// else — not the model the app is on (which would tell a public model
    /// which providers are private), not the app, not the session.
    #[error(
        "An app's model is fixed when its session is created, so this app cannot be switched to \
         '{requested}', which is a more private model than the one it started on. Nothing was \
         changed and the app is still on its current model. Do not retry; the same call will be \
         refused again, and no manifest field, frame or setting changes it. If this app really \
         needs that model, it has to be *created* on it: tell the user, so they can set the app's \
         model and start a fresh session for it."
    )]
    AppSessionTierFixed { requested: String },

    /// DR-31: a subagent spawn named a provider whose **affiliation** differs
    /// from the chat that started it — DR-26's third axis, at the one gate that
    /// had only ever compared tiers.
    ///
    /// ⚠ **Equality, both directions, and not the subset rule DR-26 uses for
    /// model-versus-extension.** Affiliation has exactly the two failure modes
    /// the tier pair already refuses, so it takes the same shape rather than
    /// inventing a third:
    ///
    /// * *Elevation.* [`super::ModelAffiliation::Local`] is the TOP of this
    ///   lattice — a local model reaches every private extension, because no
    ///   transfer occurs at all — so `Institution(x) → Local` hands the child
    ///   reach the chat that spawned it does not have. Same shape as
    ///   [`Self::PrivateChildOfPublicParent`], on the axis that gate never
    ///   looked at.
    /// * *Disclosure.* `Local → Institution(x)` sends a chat's text off a
    ///   machine it was never leaving. A subset rule would **permit** exactly
    ///   this, which is why one is not used here.
    ///
    /// And `Institution(a) → Institution(b)` is refused on both readings at
    /// once: compliance does not transfer between institutions.
    ///
    /// ⚠ **Refused, not escalated**, for the reason spelled out at
    /// [`Self::PublicChildOfPrivateParent`]: a spawn is a tool call, no shipped
    /// surface lets a human spawn one and pick its provider, and no request on
    /// that path can carry a proof of user. An approval an agent can author the
    /// approver for is not an approval.
    ///
    /// ⚠ **It says what it costs.** This genuinely narrows `settings.provider`,
    /// and a user who meets it should learn why rather than conclude the
    /// override is broken — so the message names both the move that still works
    /// (`versa_azure` ↔ `versa_bedrock`, both `ucsf`) and the one that no longer
    /// does (`llamacpp`).
    ///
    /// §14.4: the payload is two affiliations and nothing else — no session, no
    /// working directory, no prompt. The prompt is the thing being withheld.
    #[error(
        "This chat runs on a model covered by {parent}, so it cannot start a subagent on a model \
         covered by {child}. A subagent may not move the work under a different set of \
         agreements, in either direction: one direction gives the child reach this chat does not \
         have (a local model is covered by nothing because nothing leaves the machine, so it \
         reaches everything private), and the other sends this chat's text somewhere it was not \
         going. Compliance does not transfer: a model approved at one institution has no \
         permission over another's data. This narrows `settings.provider` rather than disabling \
         it: a subagent may still be moved to any model with the SAME affiliation (a UCSF chat \
         can move its child between `versa_azure` and `versa_bedrock`, which are both UCSF), but \
         not to one with a different affiliation (`llamacpp` runs on this machine and is covered \
         by no institution). No subagent was started and this chat is unchanged. Do not retry; \
         the same call will be refused again, and no setting, hook or permission mode changes it. \
         If the task really belongs on that model, tell the user: they can start a new chat on it \
         and give it the task directly."
    )]
    SpawnCrossesAffiliation { parent: String, child: String },
}

impl PrivacyRefusal {
    /// R4: the child a public parent asked for was private. Named here, and
    /// nowhere before, because Task 23's spawn gate is its only caller.
    pub fn spawn_upgrade(requested: ProviderTier) -> Self {
        Self::PrivateChildOfPublicParent { requested }
    }

    /// DR-19: the child a private parent asked for was public, and only a model
    /// can have asked. Same call site, opposite direction.
    pub fn spawn_downgrade(requested: ProviderTier) -> Self {
        Self::PublicChildOfPrivateParent { requested }
    }

    /// DR-31: the child's affiliation was not the parent's. Third call site of
    /// the same spawn gate, third axis.
    ///
    /// It takes the two affiliations **typed** and renders them here, so no
    /// caller composes its own wording for a mismatch and the prose stays in the
    /// one module §14.4 is checkable by reading
    /// ([`super::affiliation::model_affiliation_label`] is the shared renderer).
    pub fn spawn_affiliation(
        parent: Option<super::affiliation::ModelAffiliation>,
        child: Option<super::affiliation::ModelAffiliation>,
    ) -> Self {
        Self::SpawnCrossesAffiliation {
            parent: super::affiliation::model_affiliation_label(parent),
            child: super::affiliation::model_affiliation_label(child),
        }
    }

    /// The classification of the session that refused. Half of the pair the
    /// GUI's card names ("this chat is **private**, that model is **public**").
    ///
    /// `None` for the three DR-16 variants, and that is the point: they are
    /// refusals about a *channel*, not about a session whose contents collided
    /// with a model, and inventing a classification for them would put a
    /// fabricated pair on the GUI's repair card. Task 18A's handlers render
    /// those three straight from [`std::fmt::Display`] and never ask. DR-21's
    /// app refusal joins them for the same reason.
    pub fn session_classification(&self) -> Option<SessionClassification> {
        match self {
            Self::PublicModelOnPrivateSession { .. } => Some(SessionClassification::Private),
            Self::TierRaiseNeedsUser { .. }
            // Same reasoning as its HTTP sibling directly above: DR-16 is about
            // WHO may raise a tier, not about a stored transcript that collided
            // with a model, so there is no (classification, tier) pair to put on
            // a repair card — and a card offering to repair it would offer the
            // model the very act the refusal exists to keep out of its hands.
            | Self::ToolTierRaise { .. }
            | Self::PrivateExtensionOverHttp { .. }
            | Self::CapabilityConfigNeedsUser { .. }
            // A spawn refusal is about the CAPABILITY the parent has, not about
            // a session whose stored contents collided with a model. Inventing a
            // classification for it would put a fabricated pair on the GUI card.
            | Self::PrivateChildOfPublicParent { .. }
            | Self::PublicChildOfPrivateParent { .. }
            // DR-21 is about WHEN an app session's tier may be set, not about a
            // stored transcript, so it has no classification either.
            | Self::AppSessionTierFixed { .. }
            // DR-31 is about WHOSE agreements cover the two models, which is a
            // different axis from the classification a repair card names.
            | Self::SpawnCrossesAffiliation { .. } => None,
        }
    }

    /// The tier of the thing that was refused.
    pub fn provider_tier(&self) -> Option<ProviderTier> {
        match self {
            Self::PublicModelOnPrivateSession { .. } => Some(ProviderTier::Public),
            // The child's tier — what was asked for, which is what was refused.
            Self::PrivateChildOfPublicParent { requested }
            | Self::PublicChildOfPrivateParent { requested } => Some(*requested),
            Self::TierRaiseNeedsUser { .. }
            | Self::ToolTierRaise { .. }
            | Self::PrivateExtensionOverHttp { .. }
            | Self::CapabilityConfigNeedsUser { .. }
            // The refused bind was necessarily private (a raise is the only
            // thing DR-21's guard fires on), but this is a refusal about a
            // channel rather than about a session/model pair, so it reports the
            // same `None` its DR-16 siblings do rather than seeding a repair
            // card the app has no way to act on.
            | Self::AppSessionTierFixed { .. }
            // DR-31 fires only where the TIERS already agreed — the two spawn
            // arms above return first — so there is no refused tier to report,
            // and naming one would put a tier crossing on a card for a refusal
            // that is not about tiers at all.
            | Self::SpawnCrossesAffiliation { .. } => None,
        }
    }

    /// The session the refusal is about. Kept as an accessor rather than
    /// destructured at every call site so a future variant without one is a
    /// compile error here instead of at six handlers.
    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::PublicModelOnPrivateSession { session_id, .. } => Some(session_id),
            // The one DR-16 variant that HAS a session to name, because it is
            // the one about a conversation other than the caller's. Reporting it
            // seeds no card — `session_classification` above is `None`, and
            // `routes/agent.rs`'s repair needs the pair — it just answers the
            // question this accessor asks truthfully.
            Self::ToolTierRaise { session_id, .. } => Some(session_id),
            Self::TierRaiseNeedsUser { .. }
            | Self::PrivateExtensionOverHttp { .. }
            | Self::CapabilityConfigNeedsUser { .. }
            | Self::PrivateChildOfPublicParent { .. }
            | Self::PublicChildOfPrivateParent { .. }
            // §14.4: an app refusal reaches the app's own page and the model
            // reading it. It carries no session id for the same reason the
            // spawn refusals do not.
            | Self::AppSessionTierFixed { .. }
            | Self::SpawnCrossesAffiliation { .. } => None,
        }
    }
}

/// The one sentence every turn refusal contains, and the reason it is a
/// constant rather than a phrase inlined at the format site: two independent
/// readers key on it — the desktop client, which must be able to tell a refusal
/// from an ordinary assistant reply, and `agents::agent::gate_b_turn_tests`,
/// whose whole discrimination between "refused" and "ran" is this substring. A
/// reworded refusal that dropped it would leave both of them silently reading
/// every refusal as a completed turn.
pub const TURN_REFUSAL_MARKER: &str = "this turn was not sent";

/// Gate B: the turn was refused because the session is classified private and
/// the model it is bound to is public, with no private model recorded on the
/// row to repair to.
///
/// §14.4: this string is rendered straight into the transcript, so it may name
/// the tier and the provider and nothing else. It takes the whole [`Session`]
/// rather than the two strings it uses so that a future variant needing the
/// classification does not have to change every call site — but note that the
/// row carries the session's *name* and *working directory*, which are CONTENT
/// and must never reach the sentence. The unit test below is what holds that.
pub fn turn_refusal(session: &Session) -> String {
    format!(
        // ⚠ "one your institution hosts, OR one that runs on this machine."
        // The old gloss said only "hosted inside the institution", which is
        // wrong: a local model is Private too, for the other of the two reasons
        // a model is safe to point at patient data. On a machine whose only
        // private model is local, that advice sent the user looking for
        // something that does not exist there.
        "This chat is private, so only a private model may run in it: one your institution \
         hosts, or one that runs on this machine. The model this chat is set to \
         (`{provider}`) is public, so \
         {TURN_REFUSAL_MARKER}. Switch this chat to a private model (Settings > Models, or the \
         model chip in the composer) and send it again. Nothing about the chat has been changed.",
        provider = session.provider_name.as_deref().unwrap_or("no model"),
    )
}

/// Gate D: the model asked to load a chat history more private than the model
/// itself.
///
/// Moved here from `agents::chatrecall_extension`, which is where Task 10 put
/// it before this module existed. Constant on purpose: a model that sees a
/// different string on retry concludes the refusal is transient and loops. It
/// names no target — not the session, not its working directory — because
/// §11.4 classifies both as CONTENT.
pub const fn chatrecall_load_refusal() -> &'static str {
    "This chat history is private: it was created under a private model, so only a private \
     model may read it. This session is running on a public model. Ask the user \
     to switch this chat to a private model (Settings > Models, or the model chip in the \
     composer) and try again. Do not retry with a different session id or through another tool; \
     the boundary is the same everywhere."
}

/// Design §7 column C, for every Workspace Control tool that names **another
/// conversation**: the caller is running on a public model and the conversation
/// it named is private — **or there is no such conversation.**
///
/// ⚠ **ONE sentence for both answers, deliberately.** This is the same
/// discipline `biorouter_server::routes::session_reach::SESSION_OUT_OF_REACH`
/// states for the HTTP surface, and it is load-bearing here for a reason that
/// surface does not have: §7 already makes `workspace_list` **omit** private
/// rows rather than redact them, precisely because a session's existence and its
/// LLM-generated title are content. A refusal that answered "no such
/// conversation" for an absent id and "that one is private" for a private one
/// would hand the omitted rows straight back — a model could walk id space and
/// rebuild the list the omission exists to withhold.
///
/// §14.4 / R10: it names no session id, no title, no working directory and no
/// provider, so the sentence is identical whichever conversation was asked for —
/// which is what makes the indistinguishability checkable by equality rather
/// than by reading. It forecloses the retry, and it forecloses it across
/// **tools** as well as across arguments, because this feature's whole surface
/// is seven ways to ask the same question.
///
/// A `String` rather than a `const fn`, only so the shared
/// [`ASK_THE_USER_TO_SWITCH`] sentence can be composed in rather than re-typed;
/// it is still deterministic, which is the property that matters (a model that
/// sees a different string on retry concludes the refusal is transient and
/// loops).
pub fn workspace_out_of_reach() -> String {
    format!(
        "That conversation is private, or there is no conversation with that id. This chat is \
         running on a public model, and the two answers are deliberately the same so that nothing \
         about the conversation is disclosed. Nothing was read and nothing was changed. Do not \
         retry: not with another view, not with a different session id, and not through another \
         workspace tool or code execution; the boundary is the same everywhere. If this task \
         genuinely needs that conversation, {ASK_THE_USER_TO_SWITCH}"
    )
}

/// Gate D on the **third** axis (issue #56, DR-26 / Task 50 Step 3): the model
/// asked to recall a chat that reached institutions its own agreements do not
/// cover.
///
/// Not a constant, unlike [`chatrecall_load_refusal`] above, and that is DR-26
/// rather than an inconsistency: the ruling requires the warning name the
/// institutions specifically enough for the user to act on, and "this may be a
/// compliance risk" is a shrug. It still names no session, no working directory
/// and no message — only institutions, which are not content — so §11.4 holds.
///
/// The `warning` is composed by [`super::affiliation::cross_affiliation_owners`]
/// and never here: one boundary must not be described two ways depending on
/// which surface the model met it at.
pub fn chatrecall_cross_affiliation_refusal(warning: &str) -> String {
    format!(
        "{warning} This chat history was not read. Only the user can accept a \
         cross-institutional risk, and only after it has been stated to them, so do not retry: \
         not with a different session id, not through a search, and not through code execution. \
         Tell the user what you were trying to do and ask them to switch this chat to a model \
         covered by that institution's agreements."
    )
}

/// The boolean under [`privacy_refusal`], as its own name.
///
/// One rule, three renderings: the model-facing sentence [`privacy_refusal`]
/// composes, the typed HTTP body [`PrivacyRefusal::PrivateExtensionOverHttp`]
/// renders for the GUI, and the plain `if` a route needs when it holds neither.
/// Extracted so `POST /agent/add_extension` — the *user's* enable door, which
/// must keep its own typed refusal and so cannot call `privacy_refusal` — asks
/// the same predicate as the agent's doors instead of re-typing
/// `class.tier.is_private() && caller == Public` a fourth time. Two spellings of
/// one table agree until the edit nobody cross-checks.
///
/// ⚠ **The master opt-out is deliberately NOT in here.** DR-15 is read off the
/// admitted [`CallCapability`](super::CallCapability) on a dispatch path and
/// straight off the global on a route that has no capability to inherit; folding
/// it in would make this predicate lie to whichever caller reads the toggle the
/// other way. Callers gate on it themselves, visibly.
pub const fn tier_refuses(extension_tier: ProviderTier, caller_tier: ProviderTier) -> bool {
    extension_tier.is_private() && !caller_tier.is_private()
}

/// Gate C's refusal. Returns `None` when the call is permitted, so the caller
/// reads as `if let Some(err) = privacy_refusal(..) { return Err(err.into()); }`.
///
/// Pure: no config, no session, no provider, no I/O. That is what lets the
/// dispatch choke point ask it while holding nothing, and what lets its whole
/// behaviour be pinned by a table-driven unit test.
///
/// `ErrorData` directly, **not** a `ToolInspector`. `Agent::handle_denied_tools`
/// passes a real reason through for exactly three inspector names — the hook
/// inspector, `"security"` and the repetition inspector — and everything else
/// falls to `DECLINED_RESPONSE`, which the code itself calls "actively
/// misleading": it tells the model *the user* refused. An inspector-shaped Gate
/// C would also be invisible to `POST /agent/call_tool`, to the `execute_code`
/// bridge and to `Agent::call_prefetch_tool`, none of which run inspectors.
///
/// §14.4: the string reaches the model's context, so it names the extension and
/// the two tiers and nothing else — no session id, no title, no working
/// directory.
pub fn privacy_refusal(
    extension: &str,
    extension_tier: ProviderTier,
    caller_tier: ProviderTier,
) -> Option<ErrorData> {
    if !tier_refuses(extension_tier, caller_tier) {
        return None;
    }
    Some(ErrorData::new(
        ErrorCode::INVALID_REQUEST,
        format!(
            "`{extension}` is a private extension: it reaches data held inside the institution, \
             so only a private model may call it. This session is running on a public model. \
             Ask the user to switch this chat to a private model (Settings > Models, or the \
             model chip in the composer) and then try again. This is a data-protection \
             boundary set by the Biorouter marketplace, not something to work around: do not \
             retry with a different tool name, through code execution, or through a resource \
             read."
        ),
        None,
    ))
}

/// The same boundary, composed by the one gate that **cannot tell a private
/// extension from a name that is not installed** — issue #56 Gate C', the
/// resource and prompt surface.
///
/// [`ExtensionManager::assert_extension_reachable`] reads an unknown name as
/// Private. That is the single place in this feature where the unknown-name
/// default is inverted, deliberately: the alternative is permitting a reach at a
/// name the manager could not resolve at all. The inversion is also what makes
/// [`privacy_refusal`]'s sentence WRONG there — it states *"`x` is a private
/// extension"* about a name that may name nothing, and a model told that goes
/// looking for a private model to reach an extension that does not exist. The
/// 2026-09-10 test drive measured it as finding M18: `POST /agent/read_resource`
/// answered `403` *"`nonexistent_ext` is a private extension …"*.
///
/// ⚠ **The two cases must keep ONE answer, and that is the whole difficulty.**
/// Saying "no such extension" for the absent case would build an existence
/// oracle out of the repair: a public caller could walk names and learn which
/// private connectors a chat has loaded — precisely the set Gate E hides, and
/// precisely the leak finding M6 closes on the roster next door in
/// `read_resource_tool`. So this states the disjunction rather than resolving
/// it, and returns the identical string in both cases. Failing closed is
/// unchanged; only the claim about *why* is.
///
/// ⚠ **`privacy_refusal` keeps its sentence, and this does not replace it.**
/// Gate C proper (`dispatch_tool_call`) resolves the extension from an
/// installed client before it refuses, so "is a private extension" is a fact
/// there and the flat statement is the better one to give a model. Two
/// renderings of [`tier_refuses`], each true where it is used — not one
/// rendering hedged everywhere.
///
/// The actionable half survives, conditionally rather than as an assertion: a
/// caller that really did meet a private extension still learns what clears it,
/// and one that merely mistyped a name is no longer sent to the model picker to
/// fix a typo.
///
/// §14.4 as everywhere else: the extension the caller itself named, the two
/// tiers, and nothing else.
///
/// [`ExtensionManager::assert_extension_reachable`]: crate::agents::ExtensionManager
pub fn private_or_absent_refusal(
    extension: &str,
    extension_tier: ProviderTier,
    caller_tier: ProviderTier,
) -> Option<ErrorData> {
    if !tier_refuses(extension_tier, caller_tier) {
        return None;
    }
    Some(ErrorData::new(
        ErrorCode::INVALID_REQUEST,
        format!(
            "`{extension}` cannot be reached from this chat, which is running on a public \
             model. To a public model a private extension — one that reaches data held inside \
             the institution — and a name that is not installed here are the same answer, and \
             Biorouter does not say which `{extension}` is. If it is a private extension, ask \
             the user to switch this chat to a private model (Settings > Models, or the model \
             chip in the composer) and try again; if it is not installed, no model will reach \
             it. This is a data-protection boundary set by the Biorouter marketplace, not \
             something to work around: do not retry with a different tool name, through code \
             execution, or through a resource read."
        ),
        None,
    ))
}

/// The refusal a **cross-affiliation** mismatch produces at a gate that refuses
/// — Gate C (dispatch), Gate F (the extension channels) and the agent's own
/// enable path (DR-26, Task 48).
///
/// `warning` is [`CallCapability::cross_affiliation_warning`]'s output verbatim,
/// composed once in `privacy::affiliation` so every surface states the risk in
/// the same words. This wraps it in the sentence the *model* needs and nothing
/// more: what happened, that the user is the only one who can clear it, and that
/// retrying is pointless.
///
/// ⚠ **It offers the approval; it does not perform it.** DR-26 records the
/// approval as a grant scoped to (session, extension, model affiliation), and an
/// agent can never clear a mismatch — it escalates to the user or the call does
/// not happen. So this text tells the model to ask, in the same register as
/// [`ASK_THE_USER_TO_SWITCH`]: a refusal that suggested a workaround is one the
/// model will try.
///
/// §14.4: the string reaches the model's context, so it names the extension and
/// the institutions and nothing else — no session id, no title, no working
/// directory.
///
/// ⚠ **Both exits it offers are real in this build, and reachable.** "Switch
/// this chat to a model covered by the same institution's agreements" is a bind,
/// and the bind surface warns rather than refusing. "Ask them to approve this
/// specific flow" is Task 49's cross-affiliation grant, scoped to (session,
/// extension, model affiliation): it is recorded by
/// `POST /agent/cross_affiliation_grant` behind `X-User-Action`, and
/// [`super::grant::is_granted`] is consulted by Gate C
/// (`ExtensionManager::cross_affiliation_denial`) before this refusal is
/// composed at all — so a granted triple never reaches here. Task 57 gave the
/// person at the keyboard the button: [`CROSS_AFFILIATION_ACCEPT_MARKER`] is
/// what the desktop transcript keys on to render it.
///
/// ⚠ **Two of the surfaces that produce this refusal do not consult the grant,
/// and they therefore pass `None` below.** `assert_extension_reachable` (the
/// eight non-tool-call entries — resource and prompt reads) holds no session id
/// to key a grant on; the agent's own `manage_extensions` enable path refuses
/// deliberately, because a grant is the user's acceptance of a data flow through
/// a connector the chat already has and not permission for the model to attach
/// one. Both fail CLOSED — a refusal the user meets again, never a disclosure
/// they did not accept — and each carries the reason at its own site.
///
/// [`CallCapability::cross_affiliation_warning`]: super::CallCapability::cross_affiliation_warning
pub fn cross_affiliation_refusal(warning: &str, acceptable: Option<&str>) -> ErrorData {
    // ⚠ ONE text with an optional tail, never two refusals. The two spellings
    // describe the same boundary and must not drift into two accounts of it;
    // what differs is whether an acceptance exists that would clear THIS call.
    let accept = match acceptable {
        Some(extension) => {
            format!(
                " {CROSS_AFFILIATION_ACCEPT_MARKER}`{extension}`, on this chat's current model."
            )
        }
        None => String::new(),
    };
    ErrorData::new(
        ErrorCode::INVALID_REQUEST,
        format!(
            "{warning} This call was not made. Only the user can accept a cross-institutional \
             risk, and only after it has been stated to them, so do not retry: not with a \
             different tool name, not through code execution, and not through a resource read. \
             Tell the user what you were trying to do and ask them to approve this specific flow \
             or to switch this chat to a model covered by the same institution's \
             agreements.{accept}"
        ),
        None,
    )
}

/// **Gate F1, whole: the one function that decides whether an extension may be
/// ENABLED** (issue #56, findings 4 and 13 — and the seam their two fixes left
/// between them).
///
/// `Some(err)` refuses; `None` permits. Three arms, in this order, and the order
/// is the security property:
///
///  1. **The tier arm** (Gate F1 proper) — [`privacy_refusal`].
///  2. **The affiliation arm** (DR-26) —
///     [`CallCapability::cross_affiliation_warning`], wrapped by
///     [`cross_affiliation_refusal`].
///  3. **The operator pin** (#42) — a persisted `enabled: false`.
///
/// ⚠ **Both privacy arms sit above the pin, and that is finding 13 generalised.**
/// The pin's refusal is an *install-state answer*: "this machine has that
/// extension and the operator turned it off" is exactly the fact a caller who may
/// not reach the extension must not be able to read out of a refusal. Finding 13
/// established this for `manage_extensions`, where the tier arm was moved above
/// the pin **and** above the not-found branch; the same reasoning does not stop at
/// the tier axis, so the affiliation arm is above the pin too. Nothing is lost in
/// the other direction: a caller entitled to the extension reaches the pin exactly
/// as before, which is every case #42 was written for.
///
/// ⚠ **One function because two were one rule.** `check_enable_allowed`
/// (`agents/extension_manager_extension.rs`) and
/// `WorkspaceClient::refuse_gated_extension_enable` (`agents/workspace_extension.rs`)
/// were written in parallel, arm for arm, with different words and — the part
/// that mattered — different clause order: the workspace copy asked the pin
/// first, so `workspace_open {new:{extensions}}` reopened the very oracle its
/// sibling had just closed. Reordering the copy would have left two spellings
/// that agree today and diverge on the next edit, so there is now one, and both
/// doors call it. Do not add a third; give the new door this one.
///
/// # Arguments
///
/// `entry` is the on-disk config entry when the extension is installed, and
/// `None` for "not installed, or not looked up yet". The classification still
/// resolves — by name, from the compiled marketplace baseline — which is what
/// makes the tier arm answer identically in both worlds. Passing the entry can
/// only RAISE the answer: [`super::resolve_extension`] also matches a renamed
/// entry through the install directory in its arguments (Task 43 / DR-23), which
/// the name alone no longer carries.
///
/// `persisted` is #42's provenance signal — `extension_entry_is_persisted`,
/// asked by each caller rather than in here, because that helper reads the global
/// config and this function is otherwise pure. Purity is what lets the enable
/// gate be driven in both toggle positions, at every tier, with no config file
/// and no machine state; a version that looked the flag up itself could only be
/// tested on a machine that had the extension installed.
///
/// ⚠ **The not-found branch is NOT here**, and deliberately: the two doors give
/// different answers to an unknown name (`manage_extensions` says "not found",
/// `workspace_open` stays silent because `start_session` answers for it) and
/// neither answer is a privacy decision. What matters is that both doors ask
/// *this* first, so no unknown-name answer can be reached by a caller the tier or
/// affiliation arm refuses.
///
/// ⚠ **Task 49's grant is not consulted**, hence the `None` passed to
/// [`cross_affiliation_refusal`]. A grant is the user's acceptance of a data flow
/// through a connector this chat already has; it is not permission for the model
/// to attach one the chat did not have. Reading it here would let an agent turn
/// one accepted flow into the authority to open the very server that flow runs
/// over — the enable is what pulls credentials out of the keychain and starts the
/// process. The route out is a user's: enable it from Settings
/// (`POST /agent/add_extension`, which warns and proceeds), then accept the flow.
///
/// [`CallCapability::cross_affiliation_warning`]: super::CallCapability::cross_affiliation_warning
pub fn extension_enable_refusal(
    cap: super::CallCapability,
    extension: &str,
    entry: Option<&crate::config::ExtensionEntry>,
    persisted: bool,
) -> Option<ErrorData> {
    extension_enable_refusal_inner(cap, extension, entry, persisted, false, false)
}

/// Extension Manager's stricter agent-controlled door. Public models may
/// never spawn private extensions through it, even when diagnostic privacy
/// enforcement is disabled. A proof-backed grant may override only a public
/// extension's persisted operator pin.
pub fn extension_manager_enable_refusal(
    cap: super::CallCapability,
    extension: &str,
    entry: Option<&crate::config::ExtensionEntry>,
    persisted: bool,
    proof_backed_public_grant: bool,
) -> Option<ErrorData> {
    extension_enable_refusal_inner(
        cap,
        extension,
        entry,
        persisted,
        true,
        proof_backed_public_grant,
    )
}

fn extension_enable_refusal_inner(
    cap: super::CallCapability,
    extension: &str,
    entry: Option<&crate::config::ExtensionEntry>,
    persisted: bool,
    absolute_public_boundary: bool,
    proof_backed_public_grant: bool,
) -> Option<ErrorData> {
    // DR-15's master opt-out, read off the SAME sample as the tier so the two can
    // never be observed at different instants. With tiers switched off the caller
    // is treated as private, which silences the tier arm and nothing else — the
    // alternative, a second flag inside this predicate, is exactly the second read
    // `CallCapability` exists to prevent. (The affiliation arm reads the same
    // sample for itself, inside `cross_affiliation_warning`.)
    let caller = if absolute_public_boundary || cap.enforced() {
        cap.tier()
    } else {
        ProviderTier::Private
    };
    // ⚠ **ONE resolution, both axes, for the whole of this function.** Nothing
    // local may GRANT private (R11(i)), so the tier comes from the compiled-in
    // marketplace baseline. Task 48 (DR-26) first asked the registry a SECOND
    // time from a `match` guard — the "two lookups let the two axes disagree
    // about one entry" pattern `resolve_extension` exists to prevent. Resolved
    // once, here; both arms below read fields off that one value.
    let class = super::resolve_extension(extension, entry.map(|entry| &entry.config));

    // 1. The tier arm. FIRST, above every answer that would betray install state.
    //
    // Enabling is not a tool call INTO a private server; it is the call that
    // SPAWNS one — it pulls that server's secrets out of the keychain and opens
    // the session — so Gate C refusing the first tool call afterwards is already
    // too late.
    if let Some(err) = privacy_refusal(extension, class.tier, caller) {
        return Some(err);
    }

    // 2. The affiliation arm.
    //
    // ⚠ **The agent is refused, not warned, and that is not an inconsistency with
    // the bind surface.** DR-26's asymmetry: a user who insists may proceed past a
    // warning; an agent never clears one automatically — it escalates to the user
    // or the call does not happen. Every caller of this function is an agent path.
    // The user's own enable path is `POST /agent/add_extension`, which warns and
    // proceeds.
    if let Some(warning) = cap.cross_affiliation_warning(extension, &class) {
        return Some(cross_affiliation_refusal(&warning, None));
    }

    // 3. Issue #42's operator pin, LAST because it is the one arm that speaks
    // about this machine. An extension whose PERSISTED config entry carries
    // `enabled: false` was turned off by the operator, and the agent must not
    // silently re-enable it — that would defeat the pinned tool environment the
    // operator set up (benchmarking, safety).
    //
    // ⚠ **Not a privacy control, and so not silenced by the master opt-out.**
    // Turning privacy tiers off must not quietly hand the agent the power to
    // re-enable everything the operator disabled.
    if let Some(entry) = entry {
        if !entry.enabled
            && persisted
            && !(proof_backed_public_grant && class.tier == ProviderTier::Public)
        {
            return Some(ErrorData::new(
                ErrorCode::INVALID_REQUEST,
                format!(
                    "Extension '{extension}' is disabled in the Biorouter configuration \
                     (enabled: false). The operator turned it off deliberately, so do not \
                     enable it yourself, not here and not on another conversation. If it \
                     is needed for this task, ask the user to re-enable it: in the desktop \
                     app under Settings > Extensions, with `biorouter configure`, or by \
                     editing the extension's entry in config.yaml."
                ),
                None,
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gate C's refusal, exercised the way `check_enable_allowed`'s four tests
    /// exercise it (`extension_manager_extension.rs`): no global config, no
    /// session, no provider — because the function is pure. Same register, too:
    /// name the state, name the reason, foreclose the workaround, name the
    /// human action.
    #[test]
    fn the_refusal_is_pure_deterministic_and_forecloses_the_workaround() {
        use ProviderTier::{Private, Public};

        // The three permitted combinations. Only "private extension, public
        // caller" is a boundary crossing.
        assert!(privacy_refusal("ucsfomopagent", Private, Private).is_none());
        assert!(privacy_refusal("developer", Public, Public).is_none());
        assert!(privacy_refusal("developer", Public, Private).is_none());

        let e = privacy_refusal("ucsfomopagent", Private, Public).unwrap();
        let m = e.message.to_string();
        assert!(m.contains("ucsfomopagent"), "{m}");
        assert!(m.contains("private"), "{m}");
        assert!(m.contains("marketplace"), "{m}"); // names the grantor (R11)
        assert!(m.contains("Settings"), "{m}"); // names the human action
        assert!(m.contains("do not"), "{m}"); // forecloses the workaround

        // Deterministic: a model that sees a different string on retry
        // concludes the refusal is transient and loops.
        assert_eq!(
            m,
            privacy_refusal("ucsfomopagent", Private, Public)
                .unwrap()
                .message
                .to_string()
        );
    }

    /// §14.4 again, for the one refusal that reaches the MODEL's context: it may
    /// name the extension and the tier and nothing else. There is no session in
    /// the signature, so this is a statement about what the function *cannot*
    /// say rather than about what it happens not to.
    #[test]
    fn gate_c_names_the_extension_and_carries_no_session_content() {
        let m = privacy_refusal("ucsfomopagent", ProviderTier::Private, ProviderTier::Public)
            .unwrap()
            .message
            .to_string();
        for content in ["20260801_7", "Patient MRN 4471 workup", "phi/cohort-3"] {
            assert!(!m.contains(content), "{m}");
        }
    }

    /// An installed extension entry under `name`, enabled or not.
    ///
    /// The tier is resolved from the NAME the caller asked for and never from
    /// the config record, so this fixture only has to carry a name that is not
    /// itself a refusal for some other reason.
    fn entry_for(name: &str, enabled: bool) -> crate::config::ExtensionEntry {
        crate::config::ExtensionEntry {
            enabled,
            config: crate::agents::ExtensionConfig::Builtin {
                name: name.to_string(),
                display_name: None,
                description: "fixture".to_string(),
                timeout: None,
                bundled: Some(true),
                available_tools: vec![],
            },
        }
    }

    /// **The enable gate's clause order, which is a security property rather
    /// than a matter of taste** (issue #56, findings 4 + 13).
    ///
    /// #42's operator pin is the one arm that speaks about *this machine*: its
    /// refusal says the extension is installed here and the operator turned it
    /// off. A caller the tier arm or the affiliation arm refuses may not learn
    /// that, so both privacy arms answer first. The two enable doors used to
    /// carry their own copy of this order and disagreed about it; there is one
    /// copy now, and this is where it is pinned.
    ///
    /// Every case is the SAME pinned-off entry, so what varies is only who is
    /// asking — which is what makes this about the order rather than about a
    /// gate that refuses one fixture.
    #[test]
    fn both_privacy_arms_answer_before_the_operator_pin() {
        use crate::privacy::affiliation::{InstitutionId, ModelAffiliation};
        use crate::privacy::CallCapability;
        const NAME: &str = "ucsfomopagent";
        let pinned_off = entry_for(NAME, false);

        // 1. The tier arm, above the pin. A public caller may not have this
        //    connector at all and must not learn that this machine does.
        let public = CallCapability::for_test(ProviderTier::Public, true);
        let tier = extension_enable_refusal(public, NAME, Some(&pinned_off), true)
            .expect("a public caller may not enable a private extension");
        assert_eq!(
            tier.message,
            privacy_refusal(NAME, ProviderTier::Private, ProviderTier::Public)
                .unwrap()
                .message,
            "the pin answered first, or the tier arm was re-spelled locally"
        );
        assert!(!tier.message.contains("enabled: false"), "{}", tier.message);

        // 2. The affiliation arm, above the pin, for the same reason one axis
        //    down: a model covered by another institution's agreements may not
        //    have it either.
        let stanford = CallCapability::for_test_affiliated(
            ProviderTier::Private,
            true,
            Some(ModelAffiliation::institution(InstitutionId::new(
                "stanford",
            ))),
        );
        let mismatch = extension_enable_refusal(stanford, NAME, Some(&pinned_off), true)
            .expect("a Stanford-covered model may not spawn a UCSF connector");
        assert!(
            !mismatch.message.contains("enabled: false"),
            "the operator pin, an answer about this machine, reached a caller the \
             affiliation arm refuses: {}",
            mismatch.message
        );

        // 3. …and for the caller #42 was written for, the pin still fires. Without
        //    this the two assertions above are satisfied by a gate that refuses
        //    everything before it ever reaches the pin.
        let local = CallCapability::for_test_affiliated(
            ProviderTier::Private,
            true,
            Some(ModelAffiliation::Local),
        );
        let pin = extension_enable_refusal(local, NAME, Some(&pinned_off), true)
            .expect("the operator's `enabled: false` is still a pin");
        assert!(pin.message.contains("enabled: false"), "{}", pin.message);
        assert!(pin.message.contains("operator"), "{}", pin.message);

        // 4. The pin is not a privacy control, so DR-15's master opt-out must not
        //    silence it — turning tiers off would otherwise hand the agent the
        //    power to re-enable everything the operator disabled.
        let opted_out = CallCapability::for_test(ProviderTier::Public, false);
        let off = extension_enable_refusal(opted_out, NAME, Some(&pinned_off), true)
            .expect("the master opt-out silences the privacy arms, not #42");
        assert!(off.message.contains("operator"), "{}", off.message);
        // …and with tiers off the private connector itself is enableable again.
        assert!(
            extension_enable_refusal(opted_out, NAME, Some(&entry_for(NAME, true)), false)
                .is_none(),
            "with privacy tiers off nothing is refused"
        );
    }

    /// The gate permits the cases every door depends on, so "refuses correctly"
    /// is not being satisfied by a function that refuses everything.
    #[test]
    fn the_enable_gate_permits_what_it_must() {
        use crate::privacy::CallCapability;
        let public = CallCapability::for_test(ProviderTier::Public, true);
        let private = CallCapability::for_test(ProviderTier::Private, true);

        // A public extension, for a public caller, installed or not.
        assert!(extension_enable_refusal(public, "developer", None, false).is_none());
        assert!(extension_enable_refusal(
            public,
            "developer",
            Some(&entry_for("developer", true)),
            true
        )
        .is_none());
        // A private extension for a caller entitled to it.
        assert!(extension_enable_refusal(
            private,
            "ucsfomopagent",
            Some(&entry_for("ucsfomopagent", true)),
            true
        )
        .is_none());
        // #42's provenance half: a default-off PLATFORM extension is injected
        // with `enabled: false` and no operator ever wrote it, so `persisted:
        // false` must stay enableable.
        assert!(
            extension_enable_refusal(
                public,
                "chatrecall",
                Some(&entry_for("chatrecall", false)),
                false
            )
            .is_none(),
            "an injected default-off entry was treated as an operator pin"
        );
    }

    /// The refusal a caller who may not reach the extension gets is **identical**
    /// across every install state — the property the two doors' disagreement
    /// broke, stated once at the gate they now share.
    #[test]
    fn no_install_state_reaches_a_caller_the_gate_refuses() {
        use crate::privacy::CallCapability;
        const NAME: &str = "ucsfomopagent";
        let public = CallCapability::for_test(ProviderTier::Public, true);
        let absent = extension_enable_refusal(public, NAME, None, false).unwrap();
        for (state, entry, persisted) in [
            ("installed and enabled", entry_for(NAME, true), true),
            (
                "installed, not in the on-disk config",
                entry_for(NAME, false),
                false,
            ),
            ("installed and pinned off", entry_for(NAME, false), true),
        ] {
            let err = extension_enable_refusal(public, NAME, Some(&entry), persisted).unwrap();
            assert_eq!(
                (absent.code, absent.message.to_string()),
                (err.code, err.message.to_string()),
                "the refusal tells the caller the connector is {state}"
            );
        }
    }

    #[test]
    fn a_refusal_survives_a_round_trip_through_anyhow() {
        // The whole reason this is a typed error: the route handler has an
        // `anyhow::Error` and owes a 409 rather than a 500.
        let err: anyhow::Error = PrivacyRefusal::PublicModelOnPrivateSession {
            session_id: "s1".into(),
            provider: "anthropic".into(),
        }
        .into();
        let refusal = err
            .downcast_ref::<PrivacyRefusal>()
            .expect("a privacy refusal must survive anyhow");
        assert_eq!(
            refusal.session_classification(),
            Some(SessionClassification::Private)
        );
        assert_eq!(refusal.provider_tier(), Some(ProviderTier::Public));
        assert_eq!(refusal.session_id(), Some("s1"));
    }

    /// §14.4: refusals are rendered to the user and (for Gate C) to the model,
    /// so the text may name the tool and the tier and nothing else. The id is
    /// carried as data for the handler; it must not be in the sentence.
    #[test]
    fn the_message_names_the_provider_and_the_tier_and_no_session_content() {
        let refusal = PrivacyRefusal::PublicModelOnPrivateSession {
            session_id: "20260801_7".into(),
            provider: "anthropic".into(),
        };
        let message = refusal.to_string();
        assert!(message.contains("anthropic"), "{message}");
        assert!(message.contains("private"), "{message}");
        assert!(
            !message.contains("20260801_7"),
            "a refusal must not carry the session id into text the user or the model reads: \
             {message}"
        );
    }

    /// The same §14.4 rule for Gate B's turn refusal, which is the one refusal
    /// built from a whole `Session` row — so it is the one with the session's
    /// name and working directory within arm's reach of the format string.
    #[test]
    fn a_turn_refusal_names_the_model_and_no_session_content() {
        let session = Session {
            id: "20260801_7".into(),
            name: "Patient MRN 4471 workup".into(),
            working_dir: std::path::PathBuf::from("/Users/someone/phi/cohort-3"),
            provider_name: Some("anthropic".into()),
            privacy_tier: SessionClassification::Private,
            ..Default::default()
        };
        let text = turn_refusal(&session);
        assert!(text.contains("anthropic"), "{text}");
        assert!(text.contains("private"), "{text}");
        for content in ["20260801_7", "Patient MRN 4471 workup", "phi/cohort-3"] {
            assert!(
                !text.contains(content),
                "a refusal carried session content ({content}) into text the user reads: {text}"
            );
        }
    }

    /// The marker is what the desktop client and `gate_b_turn_tests` key on to
    /// tell a refusal from a completed turn. A reword that dropped it would
    /// make both of them read every refusal as a turn that ran.
    #[test]
    fn every_turn_refusal_carries_the_marker() {
        for provider in [Some("anthropic".to_string()), None] {
            let session = Session {
                provider_name: provider,
                privacy_tier: SessionClassification::Private,
                ..Default::default()
            };
            let text = turn_refusal(&session);
            assert!(text.contains(TURN_REFUSAL_MARKER), "{text}");
        }
    }

    #[test]
    fn only_an_upward_bind_needs_the_user() {
        use ProviderTier::{Private, Public};
        assert!(raise_needs_user_action(Public, Private)); // the one raise
        assert!(!raise_needs_user_action(Public, Public)); // sideways
        assert!(!raise_needs_user_action(Private, Private)); // sideways
        assert!(!raise_needs_user_action(Private, Public)); // downward — Gate A's job, not this one
    }

    #[test]
    fn a_refusal_names_nothing_the_caller_did_not_ask_for() {
        // R10's disclosure bound. A refusal that says "pick versa_azure instead"
        // tells a public model which providers are private, and a refusal that
        // says "ucsfomopagent is private, cdwagent is not" is a classification
        // oracle the model can drive one name at a time.
        let msg = PrivacyRefusal::TierRaiseNeedsUser {
            requested: "llamacpp".into(),
        }
        .to_string();
        for other in ["versa_azure", "versa_bedrock", "ollama"] {
            assert!(
                !msg.contains(other),
                "refusal leaked the classification of {other}"
            );
        }
        // …and the loop above only means something if the name IS rendered.
        // While `requested` was carried but never interpolated, every assertion
        // in that loop held against a refusal that named nothing at all.
        assert!(
            msg.contains("llamacpp"),
            "the caller's own name may be named, and must be, or the loop above is vacuous"
        );

        // The private extension set comes from the generator, not from a
        // hand-written list here: a hand-written one stops tracking it and the
        // assertion goes quietly vacuous.
        let msg = PrivacyRefusal::PrivateExtensionOverHttp {
            name: "ucsfomopagent".into(),
        }
        .to_string();
        for other in crate::privacy::private_extension_ids().filter(|id| *id != "ucsfomopagent") {
            assert!(
                !msg.contains(other),
                "refusal leaked the classification of {other}"
            );
        }
        assert!(
            msg.contains("ucsfomopagent"),
            "the caller's own name may be named"
        );
    }

    /// Task 48's refusal, under the same two rules as its siblings: it carries
    /// the warning verbatim, it names no extension the caller did not ask about,
    /// and it forecloses the workaround.
    ///
    /// ⚠ It deliberately does NOT end in [`ASK_THE_USER_TO_SWITCH`], which is
    /// why it is asserted here and not in the loop below. That constant offers
    /// exactly one way out — *switch this chat to a private model* — and this
    /// chat is already on one. Telling the model to ask for a private model
    /// would be advice it has already taken, and following it would produce a
    /// loop rather than a fix.
    #[test]
    fn the_cross_affiliation_refusal_carries_the_warning_and_forecloses_the_workaround() {
        let warning = "Cross-institutional data flow. The extension `ucsfomopagent` holds data \
                       belonging to UCSF (ucsf), but this chat is bound to a model covered by \
                       Stanford's agreements.";
        let msg = cross_affiliation_refusal(warning, None).message.to_string();

        assert!(msg.contains(warning), "the warning is the product: {msg}");
        assert!(msg.contains("do not retry"), "{msg}");
        assert!(
            msg.contains("code execution") && msg.contains("resource read"),
            "the workaround has to be named to be foreclosed: {msg}"
        );
        assert!(
            msg.contains("user"),
            "only the user can accept this risk, and the model has to be told so: {msg}"
        );
        assert!(
            !msg.contains(ASK_THE_USER_TO_SWITCH),
            "this chat is already on a private model; that sentence is a loop: {msg}"
        );

        // R10's disclosure bound: the message says nothing about any other
        // extension's classification. Taken from the generator, so it cannot go
        // quietly vacuous when the set changes.
        for other in crate::privacy::private_extension_ids().filter(|id| *id != "ucsfomopagent") {
            assert!(
                !msg.contains(other),
                "refusal leaked the classification of {other}"
            );
        }

        // Deterministic: a model that sees a different string on retry concludes
        // the refusal is transient and loops.
        assert_eq!(
            msg,
            cross_affiliation_refusal(warning, None).message.to_string()
        );
        assert_eq!(
            cross_affiliation_refusal(warning, None).code,
            ErrorCode::INVALID_REQUEST
        );
    }

    /// Task 57. The desktop transcript turns this refusal into an **accept
    /// control**, and it may only do so where pressing it actually clears the
    /// call.
    ///
    /// Two of the three sites that compose this refusal do not consult the grant
    /// — `assert_extension_reachable` (no session to key one on) and the agent's
    /// own `manage_extensions` enable path (deliberately, DR-26's "an agent never
    /// clears a mismatch automatically"). A button rendered on either would
    /// record a real acceptance and leave the retry refused, which is the same
    /// bug as having no button at all, wearing a fix's clothes. So the offer is
    /// an argument, not a property of the text.
    #[test]
    fn only_the_refusal_a_grant_can_clear_offers_the_accept_marker() {
        let warning = "Cross-institutional data flow. The extension `ucsfomopagent` holds data \
                       belonging to UCSF (ucsf), but this chat is bound to a model covered by \
                       Stanford's agreements.";

        let offered = cross_affiliation_refusal(warning, Some("ucsfomopagent"))
            .message
            .to_string();
        assert!(
            offered.contains(CROSS_AFFILIATION_ACCEPT_MARKER),
            "{offered}"
        );
        // The marker is a FRAME, not a flag: the renderer reads the extension
        // key out of what follows it, so the name has to be there, in backticks,
        // immediately after. Asserting only `contains(marker)` would pass
        // against a refusal the renderer cannot act on.
        let tail = offered
            .split(CROSS_AFFILIATION_ACCEPT_MARKER)
            .nth(1)
            .expect("the marker is present, so there is a tail");
        assert!(
            tail.starts_with("`ucsfomopagent`"),
            "the renderer parses the extension key out of the marker's tail: {tail}"
        );

        let bare = cross_affiliation_refusal(warning, None).message.to_string();
        assert!(
            !bare.contains(CROSS_AFFILIATION_ACCEPT_MARKER),
            "a refusal no grant can clear must not offer one: {bare}"
        );
        // …and it is otherwise the SAME refusal. Two divergent texts for one
        // boundary is what the one-composition rule exists to prevent.
        assert!(bare.contains(warning), "{bare}");
        assert!(offered.starts_with(&bare), "{offered}");

        // Gate D's cross-affiliation refusal is about a chat history rather than
        // an extension, and no grant is keyed on one — so it must never carry
        // the marker either.
        assert!(
            !chatrecall_cross_affiliation_refusal(warning)
                .contains(CROSS_AFFILIATION_ACCEPT_MARKER),
            "a chat-recall refusal offered an acceptance nothing records"
        );

        // Deterministic, in both spellings.
        assert_eq!(
            offered,
            cross_affiliation_refusal(warning, Some("ucsfomopagent"))
                .message
                .to_string()
        );
    }

    #[test]
    fn every_refusal_ends_in_the_same_two_ways_out_sentence() {
        // DR-16's knock-on: "switch this chat to a private model" is step 1 of
        // the two-ways-out message in EVERY refusal this feature ships, and here
        // it becomes something the model hands to the USER instead of following.
        // One constant, so the two audiences cannot drift into two vocabularies.
        for msg in [
            PrivacyRefusal::TierRaiseNeedsUser {
                requested: "llamacpp".into(),
            }
            .to_string(),
            PrivacyRefusal::PrivateExtensionOverHttp {
                name: "ucsfomopagent".into(),
            }
            .to_string(),
            PrivacyRefusal::CapabilityConfigNeedsUser {
                key: "BIOROUTER_PROVIDER".into(),
            }
            .to_string(),
            PrivacyRefusal::spawn_upgrade(ProviderTier::Private).to_string(),
        ] {
            assert!(msg.contains(ASK_THE_USER_TO_SWITCH), "{msg}");
            assert!(
                msg.contains("Do not retry"),
                "a refusal the model will retry is a loop: {msg}"
            );
        }
    }

    /// The tool-surface raise refusal: it names what it refused, marks itself as
    /// a DR-16 refusal, and deliberately does NOT end in
    /// [`ASK_THE_USER_TO_SWITCH`].
    ///
    /// That constant reads *"ask the user to switch **this chat** to a private
    /// model … with the model chip in the composer"*, and this is the one DR-16
    /// refusal about a conversation **other** than the caller's. Ending on it
    /// would send the user to the wrong composer — and, worse, send the model to
    /// the one chat whose model it does not need changed, which is a retry loop
    /// dressed as a way out. The equivalent sentence is written for the target
    /// instead.
    #[test]
    fn the_tool_raise_refusal_names_the_target_and_sends_the_user_to_it() {
        let refusal = PrivacyRefusal::ToolTierRaise {
            session_id: "s-target".into(),
            provider: "llamacpp".into(),
        };
        let msg = refusal.to_string();

        // Names both halves of what it refused — the rule this repo states as
        // "a refusal names what it refused".
        assert!(msg.contains("s-target"), "{msg}");
        assert!(msg.contains("llamacpp"), "{msg}");

        // Same disclosure bound as its siblings: it may name what the caller
        // itself passed in, and nothing else.
        for other in ["versa_azure", "versa_bedrock", "ollama"] {
            assert!(
                !msg.contains(other),
                "refusal leaked the classification of {other}"
            );
        }

        assert!(msg.contains(USER_ACTION_REFUSAL_MARKER), "{msg}");
        assert!(
            msg.contains("Do not retry"),
            "a refusal the model will retry is a loop: {msg}"
        );
        assert!(
            !msg.contains(ASK_THE_USER_TO_SWITCH),
            "that sentence points at THIS chat's composer, and the chat that needs switching is \
             a different one: {msg}"
        );
        // The way out it does carry is the target's own picker.
        assert!(
            msg.contains("ask the user to switch it themselves"),
            "a refusal with no way out is a dead end: {msg}"
        );

        // A raise is about who may act, not about a transcript that collided
        // with a model, so there is no pair for `routes/agent.rs` to build a
        // repair card from — but the session it IS about is still reportable.
        assert_eq!(refusal.session_classification(), None);
        assert_eq!(refusal.provider_tier(), None);
        assert_eq!(refusal.session_id(), Some("s-target"));
    }

    /// R4's refusal, which Task 23's spawn gate is the only caller of. §14.4:
    /// it reaches the model's context, so it may name the boundary and nothing
    /// else — not the parent's provider (which would tell a public model which
    /// providers are private), not a session id.
    #[test]
    fn the_spawn_refusal_names_the_boundary_and_no_provider() {
        let refusal = PrivacyRefusal::spawn_upgrade(ProviderTier::Private);
        let msg = refusal.to_string();
        assert!(msg.contains("public model"), "{msg}");
        assert!(msg.contains("subagent"), "{msg}");
        // It is a refusal, not a warning: nothing was started.
        assert!(msg.contains("No subagent was started"), "{msg}");
        for leak in [
            "versa_azure",
            "versa_bedrock",
            "ollama",
            "llamacpp",
            "anthropic",
            "20260801_7",
        ] {
            assert!(!msg.contains(leak), "spawn refusal leaked {leak}: {msg}");
        }
        // The child's tier is carried for the handler, and the refusal is about
        // a capability rather than about a stored session.
        assert_eq!(refusal.provider_tier(), Some(ProviderTier::Private));
        assert_eq!(refusal.session_classification(), None);
        assert_eq!(refusal.session_id(), None);
    }

    /// DR-19's refusal, the other direction. Same §14.4 bound, and one extra
    /// obligation the R4 refusal does not carry: it must name the way out.
    ///
    /// The way out is a NEW CHAT, not [`ASK_THE_USER_TO_SWITCH`] — this chat is
    /// already private, so "switch to a private model" is advice it has already
    /// taken. That is why this variant is deliberately absent from
    /// `every_refusal_ends_in_the_same_two_ways_out_sentence`, and the exclusion
    /// is asserted here rather than left as a silent omission from a list.
    #[test]
    fn the_downgrade_refusal_names_the_boundary_and_the_way_out_and_no_provider() {
        let refusal = PrivacyRefusal::spawn_downgrade(ProviderTier::Public);
        let msg = refusal.to_string();
        assert!(msg.contains("public model"), "{msg}");
        assert!(msg.contains("No subagent was started"), "{msg}");
        assert!(
            msg.contains("Do not retry"),
            "a refusal the model will retry is a loop: {msg}"
        );
        // DR-19's second half, said out loud to the model: the wall is not
        // something it can unlock by writing a hook or flipping a mode.
        assert!(
            msg.contains("no setting, hook or permission mode changes it"),
            "{msg}"
        );
        // The way out, and it is a different one.
        assert!(msg.contains("start a new chat"), "{msg}");
        assert!(
            !msg.contains(ASK_THE_USER_TO_SWITCH),
            "this chat is already private; telling the user to switch to a private model is \
             advice it has already taken: {msg}"
        );

        // R10 / §14.4: it names neither a provider nor any session content.
        // These are provider names rather than extension ids, so there is no
        // generator to draw them from — the list is the same one the sibling
        // spawn-refusal test uses, and the `start a new chat` assertion above is
        // what keeps this loop from passing against a message that says nothing.
        for leak in [
            "versa_azure",
            "versa_bedrock",
            "ollama",
            "llamacpp",
            "anthropic",
        ] {
            assert!(
                !msg.contains(leak),
                "downgrade refusal leaked {leak}: {msg}"
            );
        }
        for content in ["20260801_7", "Patient MRN 4471 workup", "phi/cohort-3"] {
            assert!(!msg.contains(content), "{msg}");
        }

        assert_eq!(refusal.provider_tier(), Some(ProviderTier::Public));
        assert_eq!(refusal.session_classification(), None);
        assert_eq!(refusal.session_id(), None);
    }

    /// DR-21's refusal. Same §14.4 bound as the spawn pair, and the same extra
    /// obligation: it must name a way out, and its way out is a *fresh app
    /// session* rather than [`ASK_THE_USER_TO_SWITCH`] — an app has no model
    /// picker, and switching this session is the very thing being refused. That
    /// exclusion is asserted here rather than left as a silent omission from
    /// `every_refusal_ends_in_the_same_two_ways_out_sentence`'s list.
    #[test]
    fn the_app_tier_refusal_names_the_boundary_and_the_way_out_and_no_other_provider() {
        let refusal = PrivacyRefusal::AppSessionTierFixed {
            requested: "llamacpp".into(),
        };
        let msg = refusal.to_string();

        // It says what the boundary IS: the tier is fixed at creation.
        assert!(msg.contains("created"), "{msg}");
        // It is a refusal, not a warning: nothing moved.
        assert!(msg.contains("Nothing was changed"), "{msg}");
        assert!(
            msg.contains("Do not retry"),
            "a refusal the model will retry is a loop: {msg}"
        );
        // The wall is not something the app can unlock by rewriting its own
        // manifest — which is the channel this refusal exists to close.
        assert!(msg.contains("no manifest field, frame or setting"), "{msg}");
        // The way out, and it is a different one.
        assert!(msg.contains("start a fresh session"), "{msg}");
        assert!(
            !msg.contains(ASK_THE_USER_TO_SWITCH),
            "an app has no model picker, and switching this session is what was refused: {msg}"
        );

        // R10's disclosure bound: the caller's own name may be named — and must
        // be, or the loop below is vacuous — but no other provider's tier may be
        // inferable from it.
        assert!(msg.contains("llamacpp"), "{msg}");
        for leak in ["versa_azure", "versa_bedrock", "ollama", "anthropic"] {
            assert!(!msg.contains(leak), "app refusal leaked {leak}: {msg}");
        }
        for content in ["20260801_7", "Patient MRN 4471 workup", "phi/cohort-3"] {
            assert!(!msg.contains(content), "{msg}");
        }

        // A channel refusal, like its DR-16 siblings: no fabricated session/model
        // pair for a repair card the app cannot act on.
        assert_eq!(refusal.provider_tier(), None);
        assert_eq!(refusal.session_classification(), None);
        assert_eq!(refusal.session_id(), None);
    }

    /// The two spawn refusals are opposite crossings and must not collapse into
    /// one sentence: a model that gets the R4 wording for a DR-19 crossing is
    /// told to ask for a private model, which is the reverse of what it should
    /// do — and the whole point of the second variant is that its way out is
    /// different.
    #[test]
    fn the_two_spawn_refusals_do_not_say_the_same_thing() {
        let upgrade = PrivacyRefusal::spawn_upgrade(ProviderTier::Private).to_string();
        let downgrade = PrivacyRefusal::spawn_downgrade(ProviderTier::Public).to_string();
        assert_ne!(upgrade, downgrade);
        assert!(upgrade.contains(ASK_THE_USER_TO_SWITCH), "{upgrade}");
        assert!(!downgrade.contains(ASK_THE_USER_TO_SWITCH), "{downgrade}");
        assert!(!upgrade.contains("start a new chat"), "{upgrade}");
        assert!(downgrade.contains("start a new chat"), "{downgrade}");
    }

    /// The two refusals the desktop renderer can receive from the model picker
    /// both carry the marker it keys on, and the one it cannot receive is not
    /// required to.
    ///
    /// `changeModel` calls `/agent/update_provider` and then
    /// `/config/set_provider`; a DR-16 refusal from either is the same fact —
    /// this backend was handed no user-action key — and gets the same user-facing
    /// explanation. `/agent/add_extension` is not on that path, and its refusal
    /// is about the extension rather than about the proof, so it is deliberately
    /// outside the marker.
    #[test]
    fn the_two_refusals_the_picker_can_receive_carry_the_marker_the_renderer_keys_on() {
        for msg in [
            PrivacyRefusal::TierRaiseNeedsUser {
                requested: "llamacpp".into(),
            }
            .to_string(),
            PrivacyRefusal::CapabilityConfigNeedsUser {
                key: "BIOROUTER_PROVIDER".into(),
            }
            .to_string(),
        ] {
            assert!(
                msg.contains(USER_ACTION_REFUSAL_MARKER),
                "the renderer cannot tell this refusal from a 500 and will report it as a \
                 provider failure: {msg}"
            );
        }
        assert!(
            !PrivacyRefusal::PrivateExtensionOverHttp {
                name: "ucsfomopagent".into(),
            }
            .to_string()
            .contains(USER_ACTION_REFUSAL_MARKER),
            "the extension refusal is not a missing-user-proof refusal and must not claim to be"
        );
    }

    /// Gate D's refusal is a constant for a reason (a model that sees a
    /// different string on retry concludes the refusal is transient and loops),
    /// and it is subject to the same content rule.
    #[test]
    fn the_chatrecall_refusal_is_stable_and_names_no_target() {
        assert_eq!(chatrecall_load_refusal(), chatrecall_load_refusal());
        assert!(chatrecall_load_refusal().contains("private"));
        assert!(!chatrecall_load_refusal().contains("session id\": "));
    }

    /// §7 column C's refusal, on all three obligations at once: it is stable, it
    /// forecloses the retry, and — the one that is specific to this surface — it
    /// carries **no argument at all**, so it cannot say anything about the
    /// conversation that was asked for.
    ///
    /// The signature is what makes the last claim structural rather than
    /// observed. `turn_refusal(&Session)` needs a unit test to prove it does not
    /// print the row's name or working directory; this takes nothing, so there is
    /// no id, title or directory in scope to leak. The assertions below are
    /// therefore about the *sentence*: that it states the boundary, offers the
    /// one way out, and shuts the door on the six sibling tools.
    #[test]
    fn the_workspace_refusal_is_stable_forecloses_the_retry_and_takes_no_target() {
        let msg = workspace_out_of_reach();
        assert_eq!(
            msg,
            workspace_out_of_reach(),
            "a refusal that varies is one the model retries"
        );
        assert!(msg.contains("private"), "{msg}");
        // Both answers, in one sentence: this is the anti-oracle claim, and it
        // is the reason the whole string exists rather than two shorter ones.
        assert!(
            msg.contains("or there is no conversation with that id"),
            "{msg}"
        );
        assert!(msg.contains("Nothing was read"), "{msg}");
        assert!(msg.contains("Do not retry"), "{msg}");
        // Foreclosed ACROSS TOOLS, not merely across arguments: `workspace_open`,
        // `workspace_send_prompt` and `workspace_read_conversation` are three
        // ways to ask the same question, and a model told only "not with a
        // different session id" will try the next tool.
        assert!(msg.contains("another workspace tool"), "{msg}");
        assert!(msg.contains(ASK_THE_USER_TO_SWITCH), "{msg}");
        // …and it must not claim to be one of the other refusals. The model
        // picker's marker sends the renderer to a model-switch card; the turn
        // marker tells the user their message was not sent. Neither happened.
        assert!(!msg.contains(USER_ACTION_REFUSAL_MARKER), "{msg}");
        assert!(!msg.contains(TURN_REFUSAL_MARKER), "{msg}");
    }
}
