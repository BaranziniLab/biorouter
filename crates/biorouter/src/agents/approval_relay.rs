//! BR-71 Task 36b: approval delegation for agent-created sessions.
//!
//! **The ruling (2026-07-30).** If an agent spun up a conversation, that
//! agent owns its approvals: it decides what it can, relays what it cannot to
//! *its* layer, and only the layer with no layer above it asks the human. The
//! user may then answer either in the chat where the escalation surfaced or in
//! the sub-agent's own tab — once, in either place, for that very specific ask.
//!
//! **What this replaces.** Task 14's `target_mode_requires_approval` refuses a
//! `mode:"turn"` injection whenever the target has no live agent, because an
//! approval prompt raised in a conversation nobody is watching parks until it
//! times out. That is no longer the only option: an agent-created session's
//! prompts now route to the agent that created it, and only reach a human at
//! the root of the chain.
//!
//! # The two bounds (plan decisions 30 and 31)
//!
//! If a parent may approve for its child, a long enough chain approves anything
//! and the human is out of the loop by construction. Two limits close that, and
//! **both live in [`DelegationPolicy`] and nowhere else**, so that loosening
//! either is a single edit with a single test:
//!
//! * **Decision 30 — no amplification (structural).** An agent may grant only
//!   what it already holds. Enforced by construction rather than by policy:
//!   [`DelegationPolicy::ask_ancestor`] runs the call through the *ancestor's
//!   own* [`crate::tool_inspection::ToolInspectionManager`], in the ancestor's
//!   own [`crate::config::BioRouterMode`], and reads the answer out of the same
//!   combining function that decides whether the ancestor itself may run a tool.
//!   [`DelegationPolicy::agent_decision`] caps what an agent can produce at
//!   `AllowOnce`/`DenyOnce` — the persistent `AlwaysAllow`/`AlwaysDeny` forms
//!   are a human's to make.
//! * **Decision 31 — always-confirm still reaches a human (tunable).** An ask
//!   an always-confirm inspector raised is [`AskClass::HumanOnly`] and no agent
//!   may satisfy it at any depth, checked *before* the ancestor is consulted so
//!   no inspector configuration can reach past it. Such an ask is also never
//!   memoized.
//!
//! Both were approved with the qualifier that they may be adjusted once real
//! users exercise the feature. They are **not one dial**: decision 31 narrows a
//! set, decision 30 is the security model. See the plan's Round 3 section.
//!
//! # Identity
//!
//! Two identities, both pre-existing, neither invented here:
//!
//! * [`AskId`] — *which parked prompt does this decision resolve*: the tool
//!   request id [`crate::agents::Agent::handle_confirmation`] already routes by
//!   (BR-62), paired with the session that owns it.
//! * [`AskKey`] — *what was approved*: the permission store's exact-args key
//!   ([`crate::permission::permission_store::exact_call_key`]). "That very
//!   specific ask" is this tool with these exact arguments — the narrowest
//!   identity the permission layer has.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use crate::agents::Agent;
use crate::conversation::message::ToolRequest;
use crate::permission::permission_confirmation::PrincipalType;
use crate::permission::permission_store::exact_call_key;
use crate::permission::{Permission, PermissionConfirmation};
use crate::session::{Session, SessionManager};
use crate::tool_inspection::{InspectionAction, InspectionResult};

/// How far up a delegation chain the walk will climb. Bounds the work and, with
/// the visited set in [`ancestor_chain`], makes a corrupted `parent_session_id`
/// cycle impossible to spin on.
const MAX_DELEGATION_DEPTH: usize = 16;

/// Inspectors whose `RequireApproval` **no agent may satisfy on the user's
/// behalf**, at any depth (decision 31). Names are each inspector's own
/// `ToolInspector::name()`.
///
/// `"managed"` is deliberately absent: a managed *Ask* is an org policy that an
/// ancestor inside the same org policy is entitled to answer, and a managed
/// *Deny* already arrives as [`InspectionAction::Deny`], which
/// [`DelegationPolicy::ask_ancestor`] turns into a refusal without asking
/// anybody.
///
/// `"hooks"` is here for a reason that is really about [`ask_ancestor`]:
/// re-running the hook inspector against an ancestor would re-execute the
/// user's own PreToolUse shell commands (see the exclusion there), so the
/// ancestor is asked *without* it. The hook's authority must not be lost with
/// its side effects, and it does not have to be — by the time an ask exists the
/// hook has already spoken in the descendant's own round: a `Deny` became
/// [`InspectionAction::Deny`] and never reached this path at all
/// (`apply_inspection_results_to_permissions` moves a denied request out of
/// `needs_approval`), and an `Ask` is the very `RequireApproval` we are
/// classifying here. Calling that `HumanOnly` keeps a user-configured "confirm
/// this" answerable only by the user, which is what a hook that asks means.
///
/// ⚠ **This is the tunable half.** Narrowing it is a product decision about
/// approval fatigue. Do not narrow it by deleting the check in `ask_ancestor` —
/// that is decision 30's half and a different security model. And do not drop
/// `"hooks"` without also restoring it to the ancestor's inspection round;
/// dropping only this entry makes an agent able to answer the user's hook.
pub const HUMAN_ONLY_INSPECTORS: &[&str] = &[
    "workspace_mutation",
    "sensitive_ops",
    "security",
    crate::hooks::inspector::HOOK_INSPECTOR_NAME,
];

/// Which parked prompt a decision resolves.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AskId {
    /// The session whose agent is parked on this prompt — the **origin**.
    pub session_id: String,
    /// The tool request id, the key `Agent::pending_confirmations` uses.
    pub request_id: String,
}

/// The identity of the ask **itself** — this tool, these exact arguments.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AskKey(String);

impl AskKey {
    /// `None` for a request that carries no recoverable identity; the caller
    /// then leaves the ask undelegated, exactly as before this task.
    pub fn for_request(request: &ToolRequest) -> Option<Self> {
        exact_call_key(request).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether **any** agent may answer this ask on the user's behalf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskClass {
    /// Only a person may answer it, at any depth (decision 31).
    HumanOnly,
    /// An ancestor may answer it within its own authority (decision 30).
    Delegable,
}

/// What one ancestor concluded about a descendant's ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DelegatedVerdict {
    /// The ancestor holds this authority itself and confers it.
    Grant(String),
    /// The ancestor's own authority stops this call.
    Refuse(String),
    /// Beyond this ancestor's authority: relay to its own layer.
    Escalate(String),
}

/// **The single policy point for decisions 30 and 31.**
///
/// Every question of the form "may an ancestor answer this, and with what?"
/// goes through this type. Nothing else in the tree reads
/// [`HUMAN_ONLY_INSPECTORS`], reads an ancestor's `tool_inspection_manager`, or
/// turns an agent's verdict into a [`Permission`]. That is deliberate: the
/// bounds are expected to be revisited after real use, and a bound enforced in
/// three places is a bound that will be relaxed in two.
pub struct DelegationPolicy;

impl DelegationPolicy {
    /// Decision 31: classify an ask from the inspection round that raised it.
    pub fn class_of(request_id: &str, inspection_results: &[InspectionResult]) -> AskClass {
        let human_only = inspection_results.iter().any(|result| {
            result.tool_request_id == request_id
                && matches!(result.action, InspectionAction::RequireApproval(_))
                && HUMAN_ONLY_INSPECTORS.contains(&result.inspector_name.as_str())
        });
        if human_only {
            AskClass::HumanOnly
        } else {
            AskClass::Delegable
        }
    }

    /// Decision 31, as a predicate, so the one check has one name.
    pub fn may_decide(class: AskClass) -> bool {
        class == AskClass::Delegable
    }

    /// Decision 30's ceiling: the strongest decision an **agent** may produce.
    ///
    /// `AlwaysAllow` and `AlwaysDeny` write a machine-wide, permanent
    /// `PermissionLevel` (`agents/tool_execution.rs:409-413` / `:418-422`).
    /// Those are a human's to make; an agent grants for this call only.
    /// `None` for `Escalate` — there is nothing to deliver.
    pub fn agent_decision(verdict: &DelegatedVerdict) -> Option<Permission> {
        match verdict {
            DelegatedVerdict::Grant(_) => Some(Permission::AllowOnce),
            DelegatedVerdict::Refuse(_) => Some(Permission::DenyOnce),
            DelegatedVerdict::Escalate(_) => None,
        }
    }

    /// Ask ONE ancestor what it would decide, using only authority it holds.
    ///
    /// `call_session` is the **descendant's** session, not the ancestor's:
    /// authority comes from the ancestor, facts come from the call. BR-24
    /// directory grants are matched against the paths the call actually names
    /// (`CallFacts::for_tool_call`, resolved against `session.working_dir`), so
    /// passing the ancestor's session would let a parent's `/work/a` grant
    /// authorize a child writing somewhere else — amplification along the
    /// directory axis.
    pub async fn ask_ancestor(
        ancestor: &Agent,
        request: &ToolRequest,
        call_session: &Session,
        class: AskClass,
    ) -> DelegatedVerdict {
        // Decision 31, FIRST — before the ancestor is consulted at all, so no
        // inspector configuration and no permission mode can reach past it.
        if !Self::may_decide(class) {
            return DelegatedVerdict::Escalate(
                "this ask must be answered by a person, in every permission mode".to_string(),
            );
        }

        // Decision 30: the ancestor's authority IS its own inspection pipeline,
        // run in its own mode. Reusing it — rather than restating it as a
        // parallel policy — is what makes non-amplification a property of
        // construction instead of a rule someone has to keep true.
        //
        // Bound ONCE, here and nowhere else in the crate: this binding is the
        // single window onto an ancestor's authority, and Step 5b's gate counts
        // the field read below to keep it that way. Both calls go through it.
        let authority = &ancestor.tool_inspection_manager;

        // ⚠ EXCLUDING the hook inspector is not an optimisation — it is the
        // difference between reading an ancestor's policy and *running the
        // user's shell commands again*. `HookInspector::inspect` calls
        // `HooksManager::pre_tool_use`, which executes the user's configured
        // PreToolUse commands; a plain `inspect_tools` here would fire them once
        // more per ancestor, with the DESCENDANT's session id and working dir,
        // for a call the descendant's own round already hooked. A hook that
        // appends to a log or bumps a counter would run N+1 times for one tool
        // call, and a rewriting hook would stage a `StagedToolHook` on the
        // ancestor's manager keyed by a session that ancestor will never drain.
        // `inspect_tools_excluding` exists for exactly this hazard (BR-19).
        //
        // No authority is lost: the hook already spoke in the descendant's round
        // — a `Deny` denied the call before this path, and an `Ask` is why we are
        // here and is classified `HumanOnly` (see [`HUMAN_ONLY_INSPECTORS`]), so
        // it is rejected above before any ancestor is consulted.
        let results = match authority
            .inspect_tools_excluding(
                &[crate::hooks::inspector::HOOK_INSPECTOR_NAME],
                std::slice::from_ref(request),
                &[],
                ancestor.config.biorouter_mode,
                call_session,
            )
            .await
        {
            Ok(results) => results,
            // An inspection round we could not complete is not an approval.
            Err(e) => {
                return DelegatedVerdict::Escalate(format!(
                    "could not evaluate this call against the ancestor's policy: {e}"
                ))
            }
        };

        let Some(outcome) = authority.process_inspection_results_with_permission_inspector(
            std::slice::from_ref(request),
            &results,
        ) else {
            // No permission inspector on that agent: no authority to read.
            return DelegatedVerdict::Escalate(
                "the ancestor has no permission inspector to consult".to_string(),
            );
        };

        if outcome.denied.iter().any(|r| r.id == request.id) {
            return DelegatedVerdict::Refuse(
                "the delegating agent's own policy denies this call".to_string(),
            );
        }
        if outcome.approved.iter().any(|r| r.id == request.id) {
            return DelegatedVerdict::Grant(
                "the delegating agent holds this authority itself".to_string(),
            );
        }
        // `needs_approval`, or an id the combiner never saw. Both mean the same
        // thing: not this ancestor's to give.
        DelegatedVerdict::Escalate("beyond the delegating agent's own authority".to_string())
    }
}

// ---------------------------------------------------------------- the relay

/// Where a remembered decision applies: the delegation tree, and the directory
/// the call was judged in.
///
/// This is **scope, not identity** — the ask's identity is [`AskKey`] and this
/// task invents no second one. `root_session_id` was always here; `working_dir`
/// joins it because a memo hit replays a human's grant with **no inspection at
/// all**, and [`DelegationPolicy::ask_ancestor`] documents at length why the
/// directory may not be dropped: BR-24 grants are matched against the paths a
/// call actually names, resolved against `session.working_dir`. Without it two
/// sibling children of one root with different working dirs share a memo entry
/// for a byte-identical *relative-path* call, so approving `rm -rf build` once
/// in `/work/a` silently deletes `/work/b/build` — amplification along exactly
/// the axis `ask_ancestor` is built to avoid, arriving through the back door.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MemoScope {
    root_session_id: String,
    working_dir: String,
}

struct Pending {
    key: AskKey,
    class: AskClass,
    /// Where a decision on this ask may be replayed (A-5).
    scope: MemoScope,
    /// Sessions currently rendering a card for this ask (A-4). At most two: the
    /// origin's own tab, and wherever a human was asked.
    surfaces: Vec<String>,
    /// `Some` once anybody has decided. First writer wins.
    decision: Option<Permission>,
}

#[derive(Default)]
struct Relay {
    pending: HashMap<AskId, Pending>,
    /// A-5: decisions already made in a delegation tree, keyed by
    /// [`MemoScope`] + [`AskKey`]. In memory and for this process only —
    /// deliberately NOT `ToolPermissionStore`, whose grants are machine-wide
    /// and permanent.
    memo: HashMap<(MemoScope, AskKey), Permission>,
}

static RELAY: LazyLock<Mutex<Relay>> = LazyLock::new(|| Mutex::new(Relay::default()));

/// Poisoning cannot leave the relay logically inconsistent — every mutation is
/// a single map operation — so a panic elsewhere must not turn approval routing
/// into a process-wide outage. Same reasoning as `session_events::bus()`.
fn lock() -> std::sync::MutexGuard<'static, Relay> {
    RELAY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What [`resolve`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// This decision won. `notify` is every OTHER surface still showing the ask
    /// — they must be dismissed, or the second card stays pending forever.
    Resolved {
        decision: Permission,
        notify: Vec<String>,
    },
    /// Somebody already decided. A no-op: not a second grant, not a panic.
    AlreadyResolved(Permission),
    /// No live ask with that id.
    Unknown,
}

/// Start tracking an ask. `working_dir` is the **origin session's** working
/// directory: it scopes any memo this ask produces, so a grant is replayed only
/// where the paths it named still mean the same thing.
pub fn register(
    origin: AskId,
    key: AskKey,
    class: AskClass,
    root_session_id: String,
    working_dir: String,
) {
    lock().pending.insert(
        origin,
        Pending {
            key,
            class,
            scope: MemoScope {
                root_session_id,
                working_dir,
            },
            surfaces: Vec::new(),
            decision: None,
        },
    );
}

/// Record that `session_id` is rendering a card for this ask.
pub fn add_surface(origin: &AskId, session_id: &str) {
    if let Some(entry) = lock().pending.get_mut(origin) {
        if !entry.surfaces.iter().any(|s| s.as_str() == session_id) {
            entry.surfaces.push(session_id.to_string());
        }
    }
}

/// Find the ask a decision addresses. **Fails closed**, twice over.
///
/// 1. The posting session must itself be a surface, so a request id from
///    another tree resolves nothing.
/// 2. If the pair names **more than one** live ask, it names none. [`AskId`] is
///    (session, request id) precisely so ids from different agents cannot
///    collide — but the root is a surface for *every* child in its tree, so a
///    decision posted from the root has only the bare request id to go on. Two
///    siblings parked on the same provider-assigned id (short or index-derived
///    ids are common on toolshim and OpenAI-compatible local servers) would
///    otherwise be separated by `HashMap` iteration order: the click lands on
///    whichever came out first, granting one child's call from the other
///    child's card. Returning `None` instead costs the root click — the user
///    answers from the sub-agent's own tab, where the pair *is* unique — and
///    never grants the wrong call.
pub fn lookup(request_id: &str, from_session_id: &str) -> Option<AskId> {
    let relay = lock();
    let mut matched = relay.pending.iter().filter(|(id, entry)| {
        id.request_id == request_id && entry.surfaces.iter().any(|s| s.as_str() == from_session_id)
    });
    let (id, _) = matched.next()?;
    if matched.next().is_some() {
        tracing::warn!(
            request_id,
            from_session_id,
            "Ambiguous tool-confirmation route: more than one pending ask carries this \
             request id at this surface. Refusing to guess; answer it in the sub-agent's \
             own tab, where the session pairing is unique."
        );
        return None;
    }
    Some(id.clone())
}

/// A-5. First writer wins, in one critical section — a check-then-set would let
/// two surfaces both believe they granted.
pub fn resolve(origin: &AskId, decision: Permission, decided_by: &str) -> ResolveOutcome {
    let mut relay = lock();
    let (notify, memo_key) = {
        let Some(entry) = relay.pending.get_mut(origin) else {
            return ResolveOutcome::Unknown;
        };
        if let Some(existing) = entry.decision.clone() {
            return ResolveOutcome::AlreadyResolved(existing);
        }
        entry.decision = Some(decision.clone());
        let notify: Vec<String> = entry
            .surfaces
            .iter()
            .filter(|s| s.as_str() != decided_by)
            .cloned()
            .collect();
        // ⚠ The surfaces are deliberately RETAINED, not cleared. `lookup`
        // requires the posting session to be a surface, so clearing them here
        // would make a decided-but-not-yet-forgotten ask unroutable from
        // anywhere — and the second click would fall through to the
        // pre-Task-36b path, find no prompt parked in that session, and answer
        // `unknown`. `AlreadyResolved` would be unreachable from the route.
        //
        // That matters because `already_resolved` IS the consistency mechanism
        // while the renderer still ignores the `resolve_confirmation` dismissal
        // frame (see `dismiss_on`): the other card keeps rendering as pending,
        // and the only thing that reconciles it is the answer its own click
        // gets. `decision` being `Some` — not an empty surface list — is what
        // makes this ask closed; `forget` is what makes it gone.
        // Decision 31: an always-confirm ask is answered once per call, never
        // remembered for the tree.
        let memo_key =
            (entry.class == AskClass::Delegable).then(|| (entry.scope.clone(), entry.key.clone()));
        (notify, memo_key)
    };
    if let Some(key) = memo_key {
        relay.memo.insert(key, decision.clone());
    }
    ResolveOutcome::Resolved { decision, notify }
}

/// A-5: a decision already made for this exact ask, anywhere in this tree —
/// and in the same working directory, so that replaying it cannot mean
/// something the human was never shown. See [`MemoScope`].
pub fn remembered(root_session_id: &str, working_dir: &str, key: &AskKey) -> Option<Permission> {
    let scope = MemoScope {
        root_session_id: root_session_id.to_string(),
        working_dir: working_dir.to_string(),
    };
    lock().memo.get(&(scope, key.clone())).cloned()
}

/// Drop a finished ask. Idempotent — called once per prompt beside
/// `Agent::forget_confirmation`, whatever the outcome.
pub fn forget(origin: &AskId) {
    lock().pending.remove(origin);
}

// ------------------------------------------------------- the escalation walk

/// What [`begin_delegated_approval`] concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delegation {
    /// No parent, or no recoverable ask identity: the card the caller is about
    /// to yield is the human's, exactly as before Task 36b.
    NotDelegated,
    /// An ancestor decided within its own authority. The decision has already
    /// been delivered to the waiting prompt, so **no card is shown**.
    DecidedByAncestor(Permission),
    /// The chain was exhausted: a person is being asked, in `surfaced_in`, and
    /// the origin's own tab shows the same ask.
    AwaitingHuman { surfaced_in: String },
}

/// A-1/A-2/A-3. Called from `handle_approval_tool_requests` *after* the prompt's
/// channel is registered and *before* the card would be yielded.
pub async fn begin_delegated_approval(
    agent: &Agent,
    request: &ToolRequest,
    session: &Session,
    inspection_results: &[InspectionResult],
) -> Delegation {
    // A-1: only an agent-CREATED session delegates. A root conversation's
    // approvals were always the user's.
    if session.parent_session_id.is_none() {
        return Delegation::NotDelegated;
    }
    let Some(key) = AskKey::for_request(request) else {
        // No identity, so no grant that could be scoped to "that very specific
        // ask". Fail closed to the pre-Task-36b behaviour.
        return Delegation::NotDelegated;
    };

    let chain = ancestor_chain(&agent.config.session_manager, session).await;
    let Some(root) = chain.last().cloned() else {
        return Delegation::NotDelegated;
    };
    let class = DelegationPolicy::class_of(&request.id, inspection_results);
    let origin = AskId {
        session_id: session.id.clone(),
        request_id: request.id.clone(),
    };

    // The directory the call's paths resolve against, and therefore the scope a
    // decision about it may be replayed in.
    let working_dir = session.working_dir.to_string_lossy().to_string();

    // A-5: the same ask, already answered in this tree.
    if DelegationPolicy::may_decide(class) {
        if let Some(decision) = remembered(&root, &working_dir, &key) {
            deliver(agent, &origin, decision.clone()).await;
            return Delegation::DecidedByAncestor(decision);
        }
    }

    register(origin.clone(), key, class, root.clone(), working_dir);
    // A-4, surface one: the sub-agent's own conversation tab.
    add_surface(&origin, &session.id);

    // A-3: one level at a time. A depth-3 agent asks depth 2, not the user.
    for ancestor_id in &chain {
        let Some(ancestor) = peek_ancestor(ancestor_id).await else {
            // That layer has no live agent in this process, so it cannot be
            // asked. Climb — do NOT treat silence as an approval.
            continue;
        };
        let verdict = DelegationPolicy::ask_ancestor(&ancestor, request, session, class).await;
        let Some(decision) = DelegationPolicy::agent_decision(&verdict) else {
            continue; // Escalate: relay to its own layer.
        };
        match resolve(&origin, decision.clone(), ancestor_id) {
            ResolveOutcome::Resolved { .. } => {
                deliver(agent, &origin, decision.clone()).await;
                return Delegation::DecidedByAncestor(decision);
            }
            // Raced with a human at one of the surfaces; their decision stands
            // — and it has ALREADY been delivered to this prompt by whoever
            // resolved it (`confirm_tool_action` calls `handle_confirmation`
            // before it returns). So the ask is answered, and the caller must
            // show no card: reporting `AwaitingHuman` here would publish a
            // second card into the root for a question nobody can still answer,
            // at a surface that was never registered — `add_surface(&root)` is
            // below this loop, so `lookup` would refuse to route from it.
            ResolveOutcome::AlreadyResolved(decided) => {
                return Delegation::DecidedByAncestor(decided);
            }
            // The entry vanished under us. There is nothing for the relay to
            // route, at any surface — so fall all the way back to the
            // pre-Task-36b path, where a decision posted from the origin's own
            // tab reaches the parked prompt directly.
            ResolveOutcome::Unknown => return Delegation::NotDelegated,
        }
    }

    // Chain exhausted: ask the person, at the root of the tree. A-4, surface
    // two. Registering the surface is what authorizes a decision POSTED from
    // that session (`lookup` fails closed otherwise); the caller then publishes
    // the very same card there, so the two surfaces are one ask on the wire.
    add_surface(&origin, &root);
    Delegation::AwaitingHuman { surfaced_in: root }
}

/// **D10.** Show a card raised inside an agent-created conversation to the person
/// watching the root of its tree as well, and accept the answer from there.
///
/// This is the [`crate::pending_user_action`] counterpart of the
/// [`Delegation::AwaitingHuman`] half of [`begin_delegated_approval`], and
/// deliberately *only* that half. Same destination (the root of the delegation
/// tree — every layer between is itself an agent, and the layer with no layer
/// above it is where a person is), same "one ask, two surfaces, one request id"
/// shape, and the same reason: a card that lives only in a conversation nobody is
/// looking at is an unexplained stall.
///
/// **No ancestor agent is consulted.** [`DelegationPolicy`] is not reached from
/// here, so decisions 30 and 31 keep their single home and this path cannot
/// produce a permission at all — it widens who may *see and click*, never who
/// may decide. Proof of user is equally untouched:
/// `PendingUserActions::resolve_matching` gates an allow on a proof-backed
/// approval by the authority of the answering request, not by the session it came
/// from.
///
/// Returns the session the card was also surfaced in, for the caller's log:
/// `None` for a root conversation (which is already where the person is), for a
/// chain that reads back empty, and for a card that is no longer parked.
///
/// ⚠ Callers: this is for a card whose decision **must** reach a person —
/// everything [`crate::pending_user_action::PendingUserActions::park`] raises
/// inside a delegated child. Do not reach for it from a door that has an
/// ancestor-agent path available; that is [`begin_delegated_approval`].
pub(crate) async fn surface_where_a_person_is_watching(
    session_manager: &SessionManager,
    session: &Session,
    parked: &crate::pending_user_action::PendingUserAction,
) -> Option<String> {
    // A root conversation IS where the person is; surfacing into it would show
    // that chat the same card twice.
    session.parent_session_id.as_ref()?;
    let chain = ancestor_chain(session_manager, session).await;
    let root = chain.last()?;
    parked.also_surface_in(root).then(|| root.clone())
}

/// Hand a decision to the prompt that is parked on it. Reuses BR-62's routing
/// wholesale: `handle_confirmation` finds the `oneshot` `register_confirmation`
/// created and `await_confirmation` is already waiting on it, so the turn loop
/// needs no new plumbing for the agent-decided path.
async fn deliver(origin_agent: &Agent, origin: &AskId, permission: Permission) {
    let _ = origin_agent
        .handle_confirmation_for_session(
            &origin.session_id,
            origin.request_id.clone(),
            PermissionConfirmation {
                principal_type: PrincipalType::Tool,
                permission,
            },
            // An ancestor AGENT decided. That is a model, by definition, so
            // the honest word is `unproven()` — even though no behaviour
            // changes today, because `register_confirmation` runs before
            // `begin_delegated_approval` and this therefore reaches the
            // `pending_confirmations` map rather than the registry.
            crate::pending_user_action::DecisionAuthority::unproven(),
        )
        .await;
}

async fn peek_ancestor(session_id: &str) -> Option<std::sync::Arc<Agent>> {
    let manager = crate::execution::manager::AgentManager::instance()
        .await
        .ok()?;
    manager.peek_agent(session_id).await
}

/// The delegation chain above `session`: immediate parent first, root last.
///
/// Bounded by [`MAX_DELEGATION_DEPTH`] and by a visited set, so a corrupted
/// `parent_session_id` cycle cannot spin. A parent row we cannot read ends the
/// chain there rather than aborting the walk — the ancestors we *did* find are
/// still the right ones to ask.
async fn ancestor_chain(session_manager: &SessionManager, session: &Session) -> Vec<String> {
    let mut chain: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::from([session.id.clone()]);
    let mut next = session.parent_session_id.clone();

    while let Some(id) = next {
        if chain.len() >= MAX_DELEGATION_DEPTH || !seen.insert(id.clone()) {
            break;
        }
        // Metadata only — `false` skips loading the conversation.
        next = match session_manager.get_session(&id, false).await {
            Ok(parent) => parent.parent_session_id.clone(),
            Err(_) => None,
        };
        chain.push(id);
    }
    chain
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BioRouterMode;
    use crate::conversation::message::ToolRequest;
    use crate::session::session_manager::SessionType;
    use std::sync::Arc;

    fn request(id: &str, tool: &str, args: serde_json::Value) -> ToolRequest {
        ToolRequest {
            id: id.to_string(),
            tool_call: Ok(rmcp::model::CallToolRequestParams {
                meta: None,
                name: tool.to_string().into(),
                arguments: Some(args.as_object().unwrap().clone()),
                task: None,
            }),
            // `ToolRequest` has FOUR fields (`conversation/message.rs:65-76`);
            // omitting the last two is E0063. Same shape as Task 10's tests.
            metadata: None,
            tool_meta: None,
        }
    }

    /// A real `Agent` with a real inspection manager, in a chosen mode, plus a
    /// child session for it to be asked about. The `TempDir` is returned
    /// because the `SessionManager` outlives the call, and ONE `SessionManager`
    /// is shared rather than two opened over the same directory.
    async fn ancestor_and_child(
        mode: BioRouterMode,
    ) -> (
        tempfile::TempDir,
        Arc<crate::agents::Agent>,
        crate::session::Session,
    ) {
        let (temp, sm, agent) = sandbox(mode).await;
        let child = new_session(&sm, temp.path(), "child").await;
        (temp, agent, child)
    }

    /// A sandbox: a temp dir, ONE `SessionManager` over it, and a real `Agent`
    /// with a real inspection manager in a chosen mode **sharing that manager**.
    ///
    /// Sharing is not tidiness. `begin_delegated_approval` walks
    /// `agent.config.session_manager`, so an agent built over a second manager
    /// would climb a chain that does not contain the sessions the test just
    /// created — and every walk assertion would pass for the wrong reason. (Two
    /// managers over one directory are also two SQLite handles on one file.)
    async fn sandbox(
        mode: BioRouterMode,
    ) -> (
        tempfile::TempDir,
        Arc<crate::session::SessionManager>,
        Arc<crate::agents::Agent>,
    ) {
        let temp = tempfile::TempDir::new().unwrap();
        let sm = Arc::new(crate::session::SessionManager::new(
            temp.path().to_path_buf(),
        ));
        let agent = Arc::new(crate::agents::Agent::with_config(
            crate::agents::AgentConfig::new(
                Arc::clone(&sm),
                crate::config::permission::PermissionManager::instance(),
                None,
                mode,
            ),
        ));
        (temp, sm, agent)
    }

    async fn new_session(
        sm: &crate::session::SessionManager,
        working_dir: &std::path::Path,
        name: &str,
    ) -> crate::session::Session {
        std::fs::create_dir_all(working_dir).unwrap();
        sm.create_session(
            working_dir.to_path_buf(),
            name.to_string(),
            SessionType::SubAgent,
        )
        .await
        .unwrap()
    }

    /// Stamp the delegation edge Task 32 writes at spawn. Without it every
    /// session looks like a root and the whole walk is inert.
    async fn set_parent(sm: &crate::session::SessionManager, child: &str, parent: &str) {
        sm.update(child)
            .parent_session_id(Some(parent.to_string()))
            .apply()
            .await
            .unwrap();
    }

    fn required_by(inspector: &str, request_id: &str) -> InspectionResult {
        InspectionResult {
            tool_request_id: request_id.to_string(),
            action: InspectionAction::RequireApproval(Some("because".into())),
            reason: "because".into(),
            confidence: 1.0,
            inspector_name: inspector.to_string(),
            finding_id: None,
        }
    }

    // ------------------------------------------------- A-6: the ask's identity

    #[test]
    fn an_ask_is_identified_by_the_permission_stores_own_exact_args_key() {
        let a = request("call-1", "acme__widget", serde_json::json!({"n": 1}));
        // Same tool, same arguments, DIFFERENT tool-request id: one ask.
        let same = request("call-2", "acme__widget", serde_json::json!({"n": 1}));
        // Same tool, different arguments: a different ask.
        let other = request("call-3", "acme__widget", serde_json::json!({"n": 2}));
        // Same arguments, different tool: a different ask.
        let elsewhere = request("call-4", "other__widget", serde_json::json!({"n": 1}));

        let key = AskKey::for_request(&a).unwrap();
        assert_eq!(key, AskKey::for_request(&same).unwrap());
        assert_ne!(key, AskKey::for_request(&other).unwrap());
        assert_ne!(key, AskKey::for_request(&elsewhere).unwrap());
        // …and it IS the store's key, not a lookalike.
        assert_eq!(
            key.as_str(),
            crate::permission::permission_store::exact_call_key(&a).unwrap()
        );
    }

    // ------------------------------------- bound 30 (decision 30): structural

    /// **Bound 30.** An ancestor whose own authority would stop the call must
    /// not grant it to a descendant.
    ///
    /// ⚠ `AskClass::Delegable` is passed EXPLICITLY, and that is the point:
    /// narrowing `HUMAN_ONLY_INSPECTORS` (bound 31, the tunable half) can never
    /// make this test pass for the wrong reason, because this test never
    /// consults that set. Do not "simplify" it into deriving the class from an
    /// inspection result — the two bounds must fail independently.
    #[tokio::test]
    async fn an_ancestor_cannot_grant_what_its_own_authority_would_stop() {
        let (_temp, parent, child) = ancestor_and_child(BioRouterMode::Approve).await;
        let req = request("call-1", "acme__widget", serde_json::json!({"n": 1}));

        let verdict =
            DelegationPolicy::ask_ancestor(&parent, &req, &child, AskClass::Delegable).await;
        assert!(
            matches!(verdict, DelegatedVerdict::Escalate(_)),
            "an Approve-mode ancestor holds no silent grant to confer; got {verdict:?}"
        );
        assert_eq!(DelegationPolicy::agent_decision(&verdict), None);
    }

    /// The mirror, plus bound 30's ceiling: the strongest thing an AGENT may
    /// ever produce is `AllowOnce`. `AlwaysAllow` is a machine-wide, permanent
    /// grant (`tool_execution.rs:409-413` writes it into the permission
    /// manager); only a human may make one.
    #[tokio::test]
    async fn an_ancestor_grants_only_once_never_always() {
        let (_temp, parent, child) = ancestor_and_child(BioRouterMode::Auto).await;
        let req = request("call-1", "acme__widget", serde_json::json!({"n": 1}));

        let verdict =
            DelegationPolicy::ask_ancestor(&parent, &req, &child, AskClass::Delegable).await;
        assert!(
            matches!(verdict, DelegatedVerdict::Grant(_)),
            "an Auto-mode ancestor could have run this itself; got {verdict:?}"
        );
        assert_eq!(
            DelegationPolicy::agent_decision(&verdict),
            Some(crate::permission::Permission::AllowOnce)
        );
    }

    // --------------------------------------- bound 31 (decision 31): tunable

    /// **Bound 31.** Auto mode is where every other path grants, so it is the
    /// only mode this test is worth running in.
    #[tokio::test]
    async fn an_always_confirm_ask_is_not_delegable_even_to_an_auto_mode_ancestor() {
        let (_temp, parent, child) = ancestor_and_child(BioRouterMode::Auto).await;
        let req = request(
            "call-1",
            "workspace__workspace_set_tools",
            serde_json::json!({"session_id": "s-target", "add_extensions": ["developer"]}),
        );

        let verdict =
            DelegationPolicy::ask_ancestor(&parent, &req, &child, AskClass::HumanOnly).await;
        assert!(
            matches!(verdict, DelegatedVerdict::Escalate(_)),
            "no agent may satisfy an always-confirm inspector; got {verdict:?}"
        );
        assert_eq!(DelegationPolicy::agent_decision(&verdict), None);

        // …and again with a tool the Auto-mode ancestor genuinely WOULD approve.
        //
        // ⚠ Do not drop this half. `workspace__workspace_set_tools` above is the
        // one tool `WorkspaceMutationInspector` escalates on its own, in every
        // mode — so that case escalates whether or not `may_decide` is consulted
        // at all, and it cannot tell where the check sits. `acme__widget` is
        // approved outright by an Auto-mode ancestor, so ONLY the `may_decide`
        // check standing *before* the ancestor is consulted keeps this
        // `HumanOnly` ask away from the agent. This is the assertion that goes
        // red when the check is moved below the inspection round.
        let plain = request("call-2", "acme__widget", serde_json::json!({"n": 1}));
        let verdict =
            DelegationPolicy::ask_ancestor(&parent, &plain, &child, AskClass::HumanOnly).await;
        assert!(
            matches!(verdict, DelegatedVerdict::Escalate(_)),
            "an Auto-mode ancestor's blanket allow must not satisfy a HumanOnly ask; got {verdict:?}"
        );
        assert_eq!(DelegationPolicy::agent_decision(&verdict), None);
    }

    #[test]
    fn the_always_confirm_inspectors_classify_as_human_only() {
        // Names taken from the inspectors' own `name()` impls.
        for inspector in ["workspace_mutation", "sensitive_ops", "security"] {
            assert_eq!(
                DelegationPolicy::class_of("call-1", &[required_by(inspector, "call-1")]),
                AskClass::HumanOnly,
                "{inspector} must not be satisfiable by an agent"
            );
        }
        // The ordinary permission prompt is delegable…
        assert_eq!(
            DelegationPolicy::class_of("call-1", &[required_by("permission", "call-1")]),
            AskClass::Delegable
        );
        // …and a finding about a DIFFERENT request must not contaminate this one.
        assert_eq!(
            DelegationPolicy::class_of("call-1", &[required_by("workspace_mutation", "call-2")]),
            AskClass::Delegable
        );
    }

    /// The hook inspector is deliberately EXCLUDED from the ancestor's
    /// inspection round, because re-running it would execute the user's own
    /// PreToolUse shell commands a second time. That exclusion is only safe
    /// because the hook's verdict is preserved here instead: a hook that asks
    /// is a user-configured "a person must confirm this", so no agent may
    /// answer it at any depth.
    ///
    /// ⚠ These two must move together. Dropping this classification while
    /// keeping the exclusion hands the user's hook to an Auto-mode ancestor.
    #[test]
    fn a_hook_that_asks_is_answerable_only_by_the_user() {
        assert_eq!(
            DelegationPolicy::class_of(
                "call-1",
                &[required_by(
                    crate::hooks::inspector::HOOK_INSPECTOR_NAME,
                    "call-1"
                )]
            ),
            AskClass::HumanOnly,
            "a PreToolUse hook's Ask must not be satisfiable by an agent"
        );
    }

    // ------------------------------------ A-4 / A-5: two surfaces, one grant

    #[test]
    fn approving_at_one_surface_dismisses_the_other_and_never_grants_twice() {
        let origin = AskId {
            session_id: "child-1".into(),
            request_id: "call-1".into(),
        };
        register(
            origin.clone(),
            AskKey::for_request(&request("call-1", "acme__widget", serde_json::json!({}))).unwrap(),
            AskClass::Delegable,
            "root-1".to_string(),
            "/work/a".to_string(),
        );
        add_surface(&origin, "child-1"); // the subagent's own tab
        add_surface(&origin, "root-1"); // where the escalation surfaced

        // The user clicks Allow in the ROOT's chat.
        let first = resolve(&origin, crate::permission::Permission::AllowOnce, "root-1");
        match first {
            ResolveOutcome::Resolved { decision, notify } => {
                assert_eq!(decision, crate::permission::Permission::AllowOnce);
                // ⚠ The child's card must be named for dismissal. An
                // implementation that grants without propagating returns an
                // empty `notify` and leaves the other surface pending forever.
                assert_eq!(notify, vec!["child-1".to_string()]);
            }
            other => panic!("the first decision must resolve; got {other:?}"),
        }

        // A decided ask stays ROUTABLE until it is forgotten. `confirm_tool_action`
        // reaches `resolve` only through `lookup`, so an ask that stopped being
        // findable the moment it was decided could never answer `already_resolved`
        // — the second click would fall through to the pre-Task-36b path and get
        // `unknown` instead. While the renderer still ignores the dismissal frame,
        // that answer is the only thing that clears the other card.
        assert_eq!(lookup("call-1", "child-1"), Some(origin.clone()));
        assert_eq!(lookup("call-1", "root-1"), Some(origin.clone()));

        // The same user then clicks Allow in the child's tab (or double-clicks).
        assert_eq!(
            resolve(&origin, crate::permission::Permission::DenyOnce, "child-1"),
            ResolveOutcome::AlreadyResolved(crate::permission::Permission::AllowOnce),
            "a second decision is a no-op, not a second grant and not a panic"
        );

        // A-5: satisfied for the whole chain, by ask identity.
        let same_ask =
            AskKey::for_request(&request("call-9", "acme__widget", serde_json::json!({}))).unwrap();
        assert_eq!(
            remembered("root-1", "/work/a", &same_ask),
            Some(crate::permission::Permission::AllowOnce)
        );
        // …but NOT into a sibling's working directory. A memo hit replays the
        // grant with no inspection at all, and the very same byte-identical call
        // means a different set of files there. This is the directory-axis
        // amplification `DelegationPolicy::ask_ancestor` refuses to allow when
        // it asks an ancestor; the memo must not reintroduce it.
        assert_eq!(
            remembered("root-1", "/work/b", &same_ask),
            None,
            "a grant made in one working directory must not replay in another"
        );
        // …and not into another tree, which was already true.
        assert_eq!(remembered("root-9", "/work/a", &same_ask), None);
        forget(&origin);
    }

    /// A `HumanOnly` ask is deliberately NOT memoized: decision 31's guarantee
    /// is per-call, and a tree-wide memo would quietly turn "confirms in every
    /// mode" into "confirms once per tree per process".
    #[test]
    fn a_human_only_ask_is_never_memoized() {
        let origin = AskId {
            session_id: "child-2".into(),
            request_id: "call-2".into(),
        };
        let key =
            AskKey::for_request(&request("call-2", "acme__gadget", serde_json::json!({}))).unwrap();
        register(
            origin.clone(),
            key.clone(),
            AskClass::HumanOnly,
            "root-2".into(),
            "/work/b".into(),
        );
        add_surface(&origin, "child-2");
        assert!(matches!(
            resolve(&origin, crate::permission::Permission::AllowOnce, "root-2"),
            ResolveOutcome::Resolved { .. }
        ));
        assert_eq!(remembered("root-2", "/work/b", &key), None);
        forget(&origin);
    }

    /// Both surfaces, at once. `resolve` takes the registry lock exactly once
    /// per call, so first-writer-wins is a property of the critical section
    /// rather than of the ordering. A check-then-set implementation passes the
    /// sequential test above and fails this one.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_decisions_from_both_surfaces_resolve_exactly_once() {
        let origin = AskId {
            session_id: "child-3".into(),
            request_id: "call-3".into(),
        };
        register(
            origin.clone(),
            AskKey::for_request(&request("call-3", "acme__widget", serde_json::json!({}))).unwrap(),
            AskClass::Delegable,
            "root-3".into(),
            "/work/c".into(),
        );
        add_surface(&origin, "child-3");
        add_surface(&origin, "root-3");

        let mut handles = Vec::new();
        for surface in [
            "child-3", "root-3", "child-3", "root-3", "child-3", "root-3",
        ] {
            let origin = origin.clone();
            handles.push(tokio::spawn(async move {
                resolve(&origin, crate::permission::Permission::AllowOnce, surface)
            }));
        }
        let mut resolved = 0;
        for handle in handles {
            if matches!(handle.await.unwrap(), ResolveOutcome::Resolved { .. }) {
                resolved += 1;
            }
        }
        assert_eq!(
            resolved, 1,
            "exactly one of six concurrent decisions may win"
        );
        forget(&origin);
    }

    /// A decision may only be routed by a session that is actually showing the
    /// ask. Fail closed: a request id from another tree resolves nothing.
    #[test]
    fn a_decision_from_a_session_that_is_not_a_surface_finds_nothing() {
        let origin = AskId {
            session_id: "child-4".into(),
            request_id: "call-4".into(),
        };
        register(
            origin.clone(),
            AskKey::for_request(&request("call-4", "acme__widget", serde_json::json!({}))).unwrap(),
            AskClass::Delegable,
            "root-4".into(),
            "/work/d".into(),
        );
        add_surface(&origin, "child-4");
        assert_eq!(lookup("call-4", "child-4"), Some(origin.clone()));
        assert_eq!(lookup("call-4", "a-stranger"), None);
        assert_eq!(lookup("call-nope", "child-4"), None);
        forget(&origin);
        assert_eq!(lookup("call-4", "child-4"), None);
    }

    /// Two children of one root, parked on the SAME provider-assigned request
    /// id. The root is a surface for both, so at that surface the request id is
    /// the only discriminator left — and it does not discriminate. `lookup` must
    /// refuse to guess rather than grant one child's call from the other's card;
    /// an implementation that returns the first `HashMap` hit passes every other
    /// test here and misroutes this one nondeterministically.
    #[test]
    fn a_request_id_shared_by_two_children_never_routes_from_their_shared_root() {
        let a = AskId {
            session_id: "child-5a".into(),
            request_id: "call_1".into(),
        };
        let b = AskId {
            session_id: "child-5b".into(),
            request_id: "call_1".into(),
        };
        for (origin, child) in [(&a, "child-5a"), (&b, "child-5b")] {
            register(
                origin.clone(),
                AskKey::for_request(&request("call_1", "acme__widget", serde_json::json!({})))
                    .unwrap(),
                AskClass::Delegable,
                "root-5".into(),
                "/work/e".into(),
            );
            add_surface(origin, child);
            add_surface(origin, "root-5");
        }

        assert_eq!(
            lookup("call_1", "root-5"),
            None,
            "an ambiguous id at the shared root must resolve nothing, not the first map hit"
        );
        // Each child's OWN tab still pairs uniquely, so the ask stays answerable.
        assert_eq!(lookup("call_1", "child-5a"), Some(a.clone()));
        assert_eq!(lookup("call_1", "child-5b"), Some(b.clone()));

        // And once one is gone the root is unambiguous again.
        forget(&a);
        assert_eq!(lookup("call_1", "root-5"), Some(b.clone()));
        forget(&b);
    }

    // ------------------------------------ A-1/A-3: the escalation walk itself

    /// A-3 depends entirely on this order: the loop asks `chain[0]` first, so a
    /// root-first chain would hand a depth-3 child's ask straight to the user's
    /// own conversation, skipping the two agents between. Nothing else in the
    /// module can catch that — both orders are non-empty and both terminate.
    #[tokio::test]
    async fn the_ancestor_chain_runs_immediate_parent_first_and_root_last() {
        let (temp, sm, _agent) = sandbox(BioRouterMode::Approve).await;
        let root = new_session(&sm, temp.path(), "root").await;
        let mid = new_session(&sm, temp.path(), "mid").await;
        let leaf = new_session(&sm, temp.path(), "leaf").await;
        set_parent(&sm, &mid.id, &root.id).await;
        set_parent(&sm, &leaf.id, &mid.id).await;
        let leaf = sm.get_session(&leaf.id, false).await.unwrap();

        assert_eq!(
            ancestor_chain(&sm, &leaf).await,
            vec![mid.id.clone(), root.id.clone()],
            "the walk must climb one hop at a time, so the immediate parent is asked first"
        );

        // A root has no chain at all, which is what makes `begin_delegated_approval`
        // return `NotDelegated` for a conversation the user started.
        let root = sm.get_session(&root.id, false).await.unwrap();
        assert!(ancestor_chain(&sm, &root).await.is_empty());
    }

    /// The two bounds on the walk, both defending against a corrupted
    /// `parent_session_id`. A session that is its own parent must yield an empty
    /// chain (not an infinite loop, and not a chain containing itself), and a
    /// cycle between two sessions must terminate.
    #[tokio::test]
    async fn the_ancestor_chain_is_cycle_safe_and_depth_bounded() {
        let (temp, sm, _agent) = sandbox(BioRouterMode::Approve).await;

        let solo = new_session(&sm, temp.path(), "solo").await;
        set_parent(&sm, &solo.id, &solo.id).await;
        let solo = sm.get_session(&solo.id, false).await.unwrap();
        assert!(
            ancestor_chain(&sm, &solo).await.is_empty(),
            "a self-referential parent is not an ancestor, so there is nobody to delegate to"
        );

        let a = new_session(&sm, temp.path(), "a").await;
        let b = new_session(&sm, temp.path(), "b").await;
        set_parent(&sm, &a.id, &b.id).await;
        set_parent(&sm, &b.id, &a.id).await;
        let a = sm.get_session(&a.id, false).await.unwrap();
        assert_eq!(
            ancestor_chain(&sm, &a).await,
            vec![b.id.clone()],
            "the visited set must stop the walk before it revisits the origin"
        );

        // A long legitimate chain is truncated, never followed forever.
        let mut previous = new_session(&sm, temp.path(), "d0").await;
        let mut deepest = previous.clone();
        for depth in 1..=(MAX_DELEGATION_DEPTH + 4) {
            let next = new_session(&sm, temp.path(), &format!("d{depth}")).await;
            set_parent(&sm, &next.id, &previous.id).await;
            previous = next.clone();
            deepest = next;
        }
        let deepest = sm.get_session(&deepest.id, false).await.unwrap();
        assert_eq!(
            ancestor_chain(&sm, &deepest).await.len(),
            MAX_DELEGATION_DEPTH,
            "the climb is bounded"
        );
    }

    /// A-1. A conversation the user started has always owned its own approvals,
    /// and nothing about this task may change that: no relay entry, and no card
    /// published anywhere else.
    #[tokio::test]
    async fn a_root_conversation_never_delegates() {
        let (_temp, agent, child) = ancestor_and_child(BioRouterMode::Approve).await;
        let req = request("call-r1", "acme__widget", serde_json::json!({"n": 1}));
        assert_eq!(child.parent_session_id, None);

        assert_eq!(
            begin_delegated_approval(&agent, &req, &child, &[]).await,
            Delegation::NotDelegated
        );
        assert_eq!(
            lookup("call-r1", &child.id),
            None,
            "a root's ask must not enter the relay at all"
        );
    }

    /// Fail closed on a request with no recoverable identity: there is no key to
    /// scope a grant to, so there is no grant to make.
    #[tokio::test]
    async fn a_malformed_request_is_never_delegated() {
        let (temp, sm, agent) = sandbox(BioRouterMode::Approve).await;
        let parent = new_session(&sm, temp.path(), "parent").await;
        let child = new_session(&sm, temp.path(), "child").await;
        set_parent(&sm, &child.id, &parent.id).await;
        let child = sm.get_session(&child.id, false).await.unwrap();

        let malformed = ToolRequest {
            id: "call-m1".to_string(),
            tool_call: Err(rmcp::model::ErrorData::new(
                rmcp::model::ErrorCode::INTERNAL_ERROR,
                "poisoned tool_call payload".to_string(),
                None,
            )),
            metadata: None,
            tool_meta: None,
        };
        assert_eq!(
            begin_delegated_approval(&agent, &malformed, &child, &[]).await,
            Delegation::NotDelegated
        );
        assert_eq!(lookup("call-m1", &child.id), None);
    }

    /// **The security-critical branch of the walk.** No ancestor in this chain
    /// has a live agent in this process, and the answer must be "ask the person"
    /// — never "nobody objected, so allow". It also pins A-4: after the walk,
    /// BOTH the origin's own tab and the root are registered surfaces, which is
    /// what makes a decision posted from either one routable.
    #[tokio::test]
    async fn an_absent_ancestor_is_never_silence_that_approves() {
        let (temp, sm, agent) = sandbox(BioRouterMode::Auto).await;
        let root = new_session(&sm, temp.path(), "root").await;
        let mid = new_session(&sm, temp.path(), "mid").await;
        let leaf = new_session(&sm, temp.path(), "leaf").await;
        set_parent(&sm, &mid.id, &root.id).await;
        set_parent(&sm, &leaf.id, &mid.id).await;
        let leaf = sm.get_session(&leaf.id, false).await.unwrap();

        let req = request("call-w1", "acme__widget", serde_json::json!({"n": 1}));
        assert_eq!(
            begin_delegated_approval(&agent, &req, &leaf, &[]).await,
            Delegation::AwaitingHuman {
                surfaced_in: root.id.clone()
            },
            "with no live ancestor the chain is exhausted and a person is asked, at the root"
        );

        let origin = AskId {
            session_id: leaf.id.clone(),
            request_id: "call-w1".to_string(),
        };
        assert_eq!(lookup("call-w1", &leaf.id), Some(origin.clone()));
        assert_eq!(lookup("call-w1", &root.id), Some(origin.clone()));
        assert_eq!(
            lookup("call-w1", &mid.id),
            None,
            "an intermediate layer the ask only passed through is not a surface"
        );
        // Nothing was granted, and nothing was memoized.
        assert_eq!(
            remembered(
                &root.id,
                &leaf.working_dir.to_string_lossy(),
                &AskKey::for_request(&req).unwrap()
            ),
            None
        );
        forget(&origin);
    }

    /// A-5 end to end: a human's grant, remembered for the tree, short-circuits
    /// the walk for a byte-identical later ask — and is delivered rather than
    /// re-asked. The second half is the guard: the SAME ask from a sibling in a
    /// DIFFERENT working directory does not hit the memo.
    #[tokio::test]
    async fn a_remembered_grant_short_circuits_the_walk_within_its_own_directory() {
        let (temp, sm, agent) = sandbox(BioRouterMode::Auto).await;
        let root = new_session(&sm, temp.path(), "root").await;
        let near = new_session(&sm, &temp.path().join("here"), "near").await;
        let far = new_session(&sm, &temp.path().join("elsewhere"), "far").await;
        set_parent(&sm, &near.id, &root.id).await;
        set_parent(&sm, &far.id, &root.id).await;
        let near = sm.get_session(&near.id, false).await.unwrap();
        let far = sm.get_session(&far.id, false).await.unwrap();

        // A human answered this exact ask once, in `near`'s directory.
        let first = request("call-m0", "acme__widget", serde_json::json!({"n": 7}));
        let seed = AskId {
            session_id: near.id.clone(),
            request_id: "call-m0".to_string(),
        };
        register(
            seed.clone(),
            AskKey::for_request(&first).unwrap(),
            AskClass::Delegable,
            root.id.clone(),
            near.working_dir.to_string_lossy().to_string(),
        );
        add_surface(&seed, &near.id);
        assert!(matches!(
            resolve(&seed, crate::permission::Permission::AllowOnce, &root.id),
            ResolveOutcome::Resolved { .. }
        ));
        forget(&seed);

        // The same ask again, same directory: decided from the memo, no card.
        let again = request("call-m1", "acme__widget", serde_json::json!({"n": 7}));
        assert_eq!(
            begin_delegated_approval(&agent, &again, &near, &[]).await,
            Delegation::DecidedByAncestor(crate::permission::Permission::AllowOnce)
        );
        assert_eq!(lookup("call-m1", &near.id), None, "no card, so no surface");

        // The same ask from a sibling in a DIFFERENT directory: the human is
        // asked again, because those bytes name different files there.
        let sibling = request("call-m2", "acme__widget", serde_json::json!({"n": 7}));
        assert_eq!(
            begin_delegated_approval(&agent, &sibling, &far, &[]).await,
            Delegation::AwaitingHuman {
                surfaced_in: root.id.clone()
            },
            "a grant made in one working directory must not silently cover another"
        );
        forget(&AskId {
            session_id: far.id.clone(),
            request_id: "call-m2".to_string(),
        });
    }
}
