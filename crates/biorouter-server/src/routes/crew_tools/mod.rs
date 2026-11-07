//! The daemon's side of a chat's two Crew person tools, `crew__read` and `crew__change`
//! (account ownership AO-12, "A chat can do what the Crew view does").
//!
//! The `crew` extension in the `biorouter` crate decides which chats are offered the tools and
//! hands each call to the [`CrewToolHost`] the daemon installs at start ([`install`]). This is
//! that host. Every action calls the same functions the Crew routes and the `biorouter crew`
//! commands call: [`CrewManager`]'s methods, the setup service, the name resolver in
//! `routes::crew::names`, the grant and consent records. Where the CLI drives a flow of several
//! steps (open, join, host), the action takes the same steps, calling the daemon's functions
//! directly.
//!
//! What this module adds around them, shared by every action ([`Ctx`]):
//!
//! * **Parameters**, checked against the action's table entry ([`catalog`]) before it runs, so
//!   what `crew__read describe` says is what a call is held to.
//! * **One approval per change** ([`Ctx::approve`]): a proof-backed card with Allow and Deny,
//!   written from targets resolved before it is parked, showing the payload whole. The action
//!   then acts on exactly those IDs, and the chat is checked again for a grant first.
//! * **Read consent** ([`Ctx::read_consent`]): the first read of a connection's workspace
//!   content in a chat asks the person once, per chat and connection. A decline is said as one,
//!   and the read is not asked again until the person writes in the chat.
//! * **Privacy** ([`Ctx::content_scope`], [`Ctx::release`]): the call's capability, sampled
//!   once at dispatch, decides what content a read may return, and the chat is raised Private
//!   before content returns.
//! * **Long work** ([`Ctx::wait_for_job`], [`Ctx::follow`]): an action that starts a job waits at
//!   most 20 seconds, or until Stop, and then answers where the job stands.
//! * **Coding agents** ([`within_call`]): a call a Claude Code or Codex child makes over the
//!   tool bridge (owner ruling D3) answers before the bridge stops waiting for it, every card
//!   and job wait in it included, and keys its changes on the child's own id for the call.
//! * **Refusals** ([`tool_error`]): a sentence with its code and, where a step needs the person
//!   (signing in, a host key, a locked key store), that step. Never a secret, and never a name
//!   from a workspace the call may not read.
//!
//! ⚠ **Person authority; needs human review.** Everything here acts as the person.

mod catalog;
mod channels;
mod hostkey;
mod messages;
mod more;
mod places;
mod setup;
mod signin;
mod tasks;
mod vault;
mod workspace;

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use biorouter::conversation::message::CrewStepRequest;
use biorouter::conversation::tool_preview::{ToolPreview, ToolPreviewLine, ToolPreviewLineKind};
use biorouter::crew::refusal::{self as crew_refusal};
use biorouter::crew::tool_host::{CrewToolHost, PersonToolCall};
use biorouter::crew::{
    manager, ClusterMode, Connection, CrewManager, CrewRefusal, SetupRefused, SetupState,
    SetupStatus, SetupWorkspacesUnlisted,
};
use biorouter::pending_user_action::{
    user_proof_available, PendingUserAction, PendingUserActions, ToolApprovalRequest,
    UserActionOutcome, UserActionRequest,
};
use biorouter::permission::tool_risk::ToolRisk;
use biorouter::privacy::affiliation::InstitutionId;
use biorouter::privacy::visibility::appears_in_list;
use biorouter::privacy::{ProviderTier, SessionClassification};
use futures::future::BoxFuture;
use serde_json::{json, Map, Value};
use tokio_util::sync::CancellationToken;

use crate::state::AppState;

pub(crate) use catalog::Action;

/// An action: given the call, its answer.
pub(crate) type Run = for<'a> fn(&'a Ctx) -> BoxFuture<'a, Result<Value>>;

/// How long a change's card, or the read consent, waits for the person. A call bridged from a
/// coding agent waits less ([`within_call`]).
const APPROVAL_TTL: Duration = Duration::from_secs(600);
/// How long an action that starts a job waits before answering where it stands (AO-12). A call
/// bridged from a coding agent waits less ([`within_call`]).
pub(crate) const JOB_WAIT: Duration = Duration::from_secs(20);
/// A Crew call bridged from a coding agent stops waiting this long before the bridge's budget
/// (`bridged_call_budget`) runs out, so it answers in time (owner ruling D3). It is the margin
/// the bridge's own approval window leaves: 570 seconds of the bridge's 600.
const BRIDGE_MARGIN: Duration = Duration::from_secs(30);
/// How often a waited-on job is asked where it stands.
const JOB_POLL: Duration = Duration::from_millis(250);
/// The largest payload a card shows: anything a parameter can carry.
const CARD_PAYLOAD_CHARS: usize = 300 * 1024;

/// Said when a change cannot be approved in this process: no person's proof can be obtained.
pub(crate) const NO_PROOF: &str = "This needs your approval, which this Biorouter can't ask you for here. Do it in the Crew view of the desktop app or with biorouter crew in a terminal.";
/// Said when a read of workspace content would need the consent card on a daemon that can
/// never show it.
pub(crate) const NO_PROOF_FOR_READS: &str = "Reading a workspace as you needs your permission once per chat, which this Biorouter can't ask you for here. Read it in the Crew view of the desktop app or with biorouter crew in a terminal.";
/// Said to a public model asked to read content that is not all public.
pub(crate) const PUBLIC_MODEL_READ: &str = "This chat uses a public model, so it can't read this workspace as you. Its connection and the workspace both have to be Public. Choose a private model to read it.";
/// Said when a private workspace has no institution, so no chat may read it.
pub(crate) const UNLABELLED_PRIVATE: &str = "This workspace has private or restricted content and no institution, so a chat can't read it. Its host can set one with privacy_set_workspace, or read it in the Crew view.";
/// Said when a private connection has no institution.
pub(crate) const CONNECTION_WITHOUT_INSTITUTION: &str = "This connection is Private and has no institution set, so a chat can't read through it. Set one with privacy_set_personal.";
/// Said when the connection and the workspace name different institutions.
pub(crate) const INSTITUTIONS_DIFFER: &str = "This connection and its workspace are set to different institutions, so a chat can't read it. Set the connection's institution to the workspace's with privacy_set_personal.";
/// Said when the chat could not be raised Private, so nothing is returned.
pub(crate) const RAISE_FAILED: &str = "Biorouter couldn't mark this chat Private, so it returned nothing from the workspace. Try again.";
/// Said when the chat already holds content another institution owns, or any institution's
/// content and the workspace has none: a grant into the chat would be refused the same way.
pub(crate) const ANOTHER_INSTITUTIONS_CONTENT: &str = "This chat already holds private content that belongs to another institution, so it can't read this workspace too. Start a new chat to read it.";
/// Said when the institutions the chat's content belongs to can't be read, so nothing is read.
pub(crate) const AFFILIATIONS_UNREADABLE: &str = "Biorouter couldn't read which institutions this chat's content belongs to, so it read nothing from the workspace. Try again.";
/// Said when the connection a call names, or one read again after a card, is not saved here.
pub(crate) const NO_CONNECTION: &str = "This computer has no saved Crew connection with that ID.";
/// Said when the person allowed a change and then pressed Stop before it ran.
pub(crate) const STOPPED_BEFORE_IT_RAN: &str =
    "Nothing was changed: the chat was stopped before it ran.";
/// Said for anything a core function refused without a sentence of its own.
pub(crate) const FAILED: &str =
    "Crew couldn't do that. Try again. If it keeps failing, the Biorouter log has the details.";

/// The daemon's [`CrewToolHost`].
pub struct ChatCrewHost {
    state: Arc<AppState>,
}

/// Install the daemon's host, so the `crew` extension offers the person tools (AO-12). First
/// install wins. A test binary, which is a process of its own, may call it too.
pub fn install(state: Arc<AppState>) {
    biorouter::crew::tool_host::install(Arc::new(ChatCrewHost { state }));
}

#[async_trait::async_trait]
impl CrewToolHost for ChatCrewHost {
    async fn call(&self, call: PersonToolCall) -> Result<Value> {
        let Some(action) = catalog::find(call.tool, &call.action) else {
            anyhow::bail!(
                "{} has no action {}. Its actions: {}.",
                call.tool.name(),
                call.action,
                call.tool.actions().join(", ")
            );
        };
        catalog::check_params(action, &call.params).map_err(anyhow::Error::msg)?;
        let Some(run) = action.run else {
            anyhow::bail!(catalog::NOT_YET);
        };
        let ctx = Ctx {
            call,
            state: Arc::clone(&self.state),
            action,
        };
        // A call bridged from a coding agent answers before the bridge stops waiting (D3).
        let answered = within_the_bridge(CALL_APPROVAL.scope(RefCell::default(), async {
            // A refusal by Crew's locked vault before any approval may be answered with the
            // vault card first, and the action run once more.
            match vault::run_unlocked(&ctx, run).await {
                Ok(answer) => Ok(answer),
                // A refusal on a server's host key may be answered with the host-key card
                // first, and the action tried again on the approval already given.
                Err(error) => retrying_after_a_host_key(hostkey::on_refusal(&ctx, error)).await,
            }
        }))
        .await;
        match answered {
            Ok(answer) => Ok(answer),
            Err(error) => {
                // A refusal that names things from a workspace returns them only as a read of
                // it would: the chat is raised first, and if it can't be, nothing is named.
                let names_from = error
                    .chain()
                    .find_map(|cause| cause.downcast_ref::<Said>())
                    .and_then(|said| said.names_from.clone());
                if let Some(scope) = names_from {
                    if let Err(failed) = ctx.release(&scope, Value::Null).await {
                        return Err(tool_error(failed, ctx.named_connection()).await);
                    }
                }
                Err(tool_error(error, ctx.named_connection()).await)
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------------------------

/// A refusal an action words itself: an optional code and one sentence.
#[derive(Debug)]
pub(crate) struct Said {
    pub(crate) code: Option<&'static str>,
    pub(crate) sentence: String,
    /// The read whose names the sentence lists, when it lists any ([`naming`]): the chat is
    /// raised as that read raises it before the refusal returns.
    pub(crate) names_from: Option<ContentScope>,
}

impl std::fmt::Display for Said {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.sentence)
    }
}

impl std::error::Error for Said {}

/// A refusal with `sentence` and, when given, `code`.
pub(crate) fn said(code: Option<&'static str>, sentence: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Said {
        code,
        sentence: sentence.into(),
        names_from: None,
    })
}

/// `error`, a refusal that lists names from a workspace this call may read under `scope`
/// (resolver candidates, a channel's team): the chat is raised as a read under `scope` raises
/// it before the refusal returns (AO-12, "Privacy"), so a name reaches the model only the way
/// content does. Any other error is returned as it is.
pub(crate) fn naming(error: anyhow::Error, scope: &ContentScope) -> anyhow::Error {
    match error.downcast::<Said>() {
        Ok(mut said) => {
            said.names_from = Some(scope.clone());
            anyhow::Error::new(said)
        }
        Err(error) => error,
    }
}

/// Why a change's card let nothing run, for any outcome but an allow the action may act on:
/// declined, allowed and then stopped, or never answered.
pub(crate) fn not_approved(outcome: &UserActionOutcome) -> anyhow::Error {
    match outcome {
        UserActionOutcome::Denied { .. } => said(None, "The person declined. Nothing was changed."),
        // An allow reaches here only when Stop was pressed before the change ran.
        UserActionOutcome::Approved { .. } => said(None, STOPPED_BEFORE_IT_RAN),
        other => said(
            None,
            format!(
                "Nothing was changed: the approval {}.",
                other.refusal_detail()
            ),
        ),
    }
}

/// The step a refusal's code asks the person to take, when there is one (AO-12, "Signing in
/// stays the person's", as the owner's D1 ruling amends it). `connection` is the saved
/// connection's name, when the call named one. Signing in, a host key and the vault each word
/// their own step ([`signin::next_step`], [`hostkey::next_step`], [`vault::next_step`]).
pub(crate) fn next_step(code: &str, connection: Option<&str>) -> Option<String> {
    if let Some(step) = signin::next_step(code, connection)
        .or_else(|| hostkey::next_step(code, connection))
        .or_else(|| vault::next_step(code, connection))
    {
        return Some(step);
    }
    Some(match code {
        "crew_device_key_missing" | "crew_device_not_recognized" => "Add this computer again with crew__change action rejoin, or with Add this computer again in Crew.".to_owned(),
        "crew_not_connected" => "Connect it first with crew__change action connect.".to_owned(),
        "crew_read_consent_required" => "Ask the person to allow it on the card, then read again.".to_owned(),
        _ => return None,
    })
}

/// `sentence`, its code and its step, as the model reads a refusal.
pub(crate) fn refusal_text(code: Option<&str>, sentence: &str, connection: Option<&str>) -> String {
    let mut text = sentence.trim().to_owned();
    if let Some(code) = code {
        text.push_str(&format!(" (code: {code})"));
        if let Some(step) = next_step(code, connection) {
            text.push(' ');
            text.push_str(&step);
        }
    }
    text
}

/// An action's error as the model is told it: the action's own words ([`Said`]), a setup
/// refusal's, or a core refusal classified as the admission routes classify it
/// (`routes::crew_authentication::core_refusal`), which never passes a workspace's own words
/// on. Anything else is [`FAILED`], and its cause goes to the log.
pub(crate) async fn tool_error(error: anyhow::Error, connection: Option<String>) -> anyhow::Error {
    let named = connection.as_deref();
    if let Some(said) = error.chain().find_map(|cause| cause.downcast_ref::<Said>()) {
        return anyhow::anyhow!(refusal_text(said.code, &said.sentence, named));
    }
    if let Some(refused) = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<SetupRefused>())
    {
        let text = refusal_text(Some(refused.code), &refused.message, named);
        return anyhow::anyhow!(hostkey::enrich(text, Some(refused.code), &[]));
    }
    // Crew's SSH policy refusing the route says what to change, in the daemon's own words.
    if let Some(refused) = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<biorouter::crew::SshPolicyRefused>())
    {
        let code = biorouter::crew::SshPolicyRefused::CODE;
        return anyhow::anyhow!(refusal_text(Some(code), refused.sentence(), named));
    }
    // A workspace's own refusal is passed on as the broker's envelope, which the `crew`
    // extension words for the model (`agent_error_text`), as it words `crew__request`'s. A
    // technical text the daemon has a sentence for is said in that sentence instead, as the
    // CLI says it ("Only the workspace host can do this."): fixed text, never the broker's.
    let broker = error
        .chain()
        .map(ToString::to_string)
        .find(|text| text.starts_with(BROKER_REFUSAL_PREFIX));
    let known = biorouter::crew::known_refusal_sentence(&error);
    let Ok(crew) = manager() else {
        return anyhow::anyhow!(FAILED);
    };
    let refused = super::crew_authentication::core_refusal(&crew, None, error, FAILED).await;
    let untyped =
        refused.code == super::crew_authentication::REQUEST_REFUSED_CODE && refused.error == FAILED;
    if let (true, Some(broker)) = (untyped, broker) {
        return match known {
            Some(sentence) => anyhow::anyhow!(sentence),
            None => anyhow::anyhow!(broker),
        };
    }
    let code = (refused.code != super::crew_authentication::REQUEST_REFUSED_CODE)
        .then_some(refused.code.as_str());
    // The daemon's own words beside the code (a host key's fingerprints among them) were
    // dropped here before; the host-key seam now sees them.
    let text = refusal_text(code, &refused.error, named);
    anyhow::anyhow!(hostkey::enrich(text, code, &refused.fields))
}

/// How a workspace's refusal begins, as the daemon's transport writes it.
const BROKER_REFUSAL_PREFIX: &str = "Crew broker refused request: ";

// ---------------------------------------------------------------------------------------------
// The call
// ---------------------------------------------------------------------------------------------

/// One person-tool call, with what every action needs.
pub(crate) struct Ctx {
    pub(crate) call: PersonToolCall,
    pub(crate) state: Arc<AppState>,
    pub(crate) action: &'static Action,
}

/// A card for one change ([`Ctx::approve`]).
#[derive(Default)]
pub(crate) struct Card {
    /// The sentence at the top: what will happen, to what, named as resolved.
    pub(crate) summary: String,
    /// What else matters: who it affects, what cannot be undone, a privacy crossing.
    pub(crate) lines: Vec<String>,
    /// What the card shows whole under the sentence.
    pub(crate) payload: Map<String, Value>,
    /// A preview in place of the payload's, such as the context document's diff.
    pub(crate) preview: Option<ToolPreview>,
}

tokio::task_local! {
    /// The approval the person gave in this person-tool call ([`Ctx::approve`]), for the one
    /// time the host-key hooks try the call's step again once the person added the key
    /// ([`retrying_after_a_host_key`]). Scoped to one call in [`ChatCrewHost::call`]; work a
    /// call runs apart from it ([`Ctx::follow`]) does not see it, and asks as before.
    static CALL_APPROVAL: RefCell<CallApproval>;
}

/// [`CALL_APPROVAL`]: the card the person allowed, and whether a host-key retry may take it.
#[derive(Default)]
struct CallApproval {
    /// The card as it was shown (summary, payload and preview), and when it was allowed.
    allowed: Option<(String, Instant)>,
    /// Set while a host-key hook, which tries the step again at most once, runs.
    retrying: bool,
}

/// Run `hook`, a host-key hook that may try the call's step again once
/// ([`hostkey::on_refusal`], [`hostkey::on_status`]), with the call's approval open to that
/// retry. The retry runs the very action the person allowed, as the model called it, so the
/// identical card would ask them the question they already answered; any other card is asked.
async fn retrying_after_a_host_key<T>(hook: impl std::future::Future<Output = T>) -> T {
    let before =
        CALL_APPROVAL.try_with(|memo| std::mem::replace(&mut memo.borrow_mut().retrying, true));
    let answer = hook.await;
    if let Ok(before) = before {
        let _ = CALL_APPROVAL.try_with(|memo| memo.borrow_mut().retrying = before);
    }
    answer
}

/// Remember that the person allowed the card `shown` at `at`, in this call.
fn remember_allowed(shown: String, at: Instant) {
    let _ = CALL_APPROVAL.try_with(|memo| memo.borrow_mut().allowed = Some((shown, at)));
}

/// Whether the card `shown` may be taken as allowed without asking: only in a host-key retry,
/// only the very card the person allowed earlier in this call, only once, and only while an
/// approval card would still have been answerable ([`APPROVAL_TTL`]). The remembered approval
/// is spent by any card a retry asks for, so a retry that asks for another card asks for it,
/// and nothing after that can take it.
fn allowed_for_this_retry(shown: &str, now: Instant) -> bool {
    CALL_APPROVAL
        .try_with(|memo| {
            let mut memo = memo.borrow_mut();
            if !memo.retrying {
                return false;
            }
            memo.allowed.take().is_some_and(|(allowed, at)| {
                allowed == shown && now.saturating_duration_since(at) <= APPROVAL_TTL
            })
        })
        .unwrap_or(false)
}

tokio::task_local! {
    /// When this person-tool call must have answered (owner ruling D3): for a call a coding
    /// agent's child made over the tool bridge, the bridge's budget less [`BRIDGE_MARGIN`] from
    /// when the call began; `None` for any other call. Scoped to one call in
    /// [`ChatCrewHost::call`] ([`within_the_bridge`]) and carried into the work [`Ctx::follow`]
    /// runs apart from it, so a card that work parks ends in time as well.
    static CALL_DEADLINE: Option<Instant>;
}

/// Run `call`, a person-tool call beginning now, with its deadline ([`CALL_DEADLINE`]): set
/// when the call came over a coding agent's tool bridge, none otherwise.
async fn within_the_bridge<T>(call: impl std::future::Future<Output = T>) -> T {
    let deadline = biorouter::providers::coding_agent::bridge::bridged_call_budget()
        .map(|budget| Instant::now() + budget.saturating_sub(BRIDGE_MARGIN));
    CALL_DEADLINE.scope(deadline, call).await
}

/// This call's deadline, when it has one.
fn call_deadline() -> Option<Instant> {
    CALL_DEADLINE.try_with(|deadline| *deadline).ok().flatten()
}

/// `wait`, cut to what is left of this call's time when it has a deadline (D3), and zero once
/// that is up. Every card and every job wait in a call bridged from a coding agent is cut so,
/// all together and not one by one: a call that asks for the vault and then for its approval
/// still answers before the bridge stops waiting for it, so a card the person could still
/// answer never turns into a transport error the child reads as a broken server.
pub(crate) fn within_call(wait: Duration) -> Duration {
    match call_deadline() {
        Some(deadline) => wait.min(deadline.saturating_duration_since(Instant::now())),
        None => wait,
    }
}

/// `at`, or this call's deadline when that comes first.
fn no_later_than_the_call(at: Instant) -> Instant {
    call_deadline().map_or(at, |deadline| at.min(deadline))
}

/// A Crew step card parked in a chat ([`Ctx::park_step`], [`park_step_in`]). Dropping it
/// without [`Self::end`] takes the card down.
pub(crate) struct PendingStep {
    parked: PendingUserAction,
    ttl: Duration,
    /// A child of the call's token: cancelling it takes this card down and nothing else.
    withdraw: CancellationToken,
    /// The call's own token: Stop.
    stop: CancellationToken,
}

/// Park a Crew step card in the chat `session_id`, in `approvals`, for the call whose Stop is
/// `stop`: what [`Ctx::park_step`] does, for work that runs apart from its call (a spawned task
/// [`Ctx::follow`] waits on, which does not inherit the call's task-local scope, so the caller
/// asks `no_human_surface` where the call runs). `None` where no person's proof can be obtained
/// (`biorouter serve`, SD-8), the call was already stopped, or a call bridged from a coding
/// agent has no time left to show it ([`within_call`]). The card waits `ttl`, never more than
/// [`APPROVAL_TTL`] nor past a bridged call's deadline.
pub(crate) fn park_step_in(
    approvals: &Arc<PendingUserActions>,
    session_id: &str,
    stop: &CancellationToken,
    step: CrewStepRequest,
    ttl: Duration,
) -> Option<PendingStep> {
    let ttl = within_call(ttl.min(APPROVAL_TTL));
    if !user_proof_available() || stop.is_cancelled() || ttl.is_zero() {
        return None;
    }
    let withdraw = stop.child_token();
    let parked = approvals.park(Some(session_id), None, UserActionRequest::CrewStep(step));
    Some(PendingStep {
        parked,
        ttl,
        withdraw,
        stop: stop.clone(),
    })
}

/// How a [`PendingStep`] ended. None of these means the step is done: the caller reads the
/// daemon for that, because a card can never be answered as a success
/// (`UserActionRequest::CrewStep` accepts no such outcome).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StepEnd {
    /// The caller withdrew it ([`PendingStep::withdrawer`]), as it does once the daemon sees
    /// the step done.
    Withdrawn,
    /// The person dismissed it.
    Dismissed,
    /// The chat was stopped.
    Stopped,
    /// Nobody acted on it before its time ran out.
    TimedOut,
    /// It could not be put to a person.
    NotShown,
}

impl PendingStep {
    /// The card's ID, as the published message carries it.
    // Only tests read it so far: the steps watch the daemon, never the card's ID.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn id(&self) -> &str {
        self.parked.id()
    }

    /// A handle that takes the card down when cancelled, for the task that watches the
    /// daemon while [`Self::end`] waits.
    pub(crate) fn withdrawer(&self) -> CancellationToken {
        self.withdraw.clone()
    }

    /// Wait for the card to end, and say how.
    pub(crate) async fn end(self) -> StepEnd {
        let outcome = self.parked.wait(self.ttl, Some(&self.withdraw)).await;
        match outcome {
            UserActionOutcome::TimedOut => StepEnd::TimedOut,
            UserActionOutcome::Failed { .. } => StepEnd::NotShown,
            UserActionOutcome::Cancelled if self.stop.is_cancelled() => StepEnd::Stopped,
            UserActionOutcome::Cancelled if self.withdraw.is_cancelled() => StepEnd::Withdrawn,
            // A Crew step accepts no other outcome; were one ever delivered, it would still
            // mean nothing was done.
            _ => StepEnd::Dismissed,
        }
    }
}

/// Where a change goes, as every change card names it (AO-12, "Approvals"): a workspace and the
/// server it is on, `lab on lab.example.edu`. A saved connection's own name stands for its
/// workspace where the card is about the connection.
pub(crate) fn on_server(workspace: &str, server: &str) -> String {
    format!("{workspace} on {server}")
}

impl Card {
    pub(crate) fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            ..Self::default()
        }
    }

    /// A card for a change to `workspace` on `server`, the one way every change card says where
    /// it goes: `summary` is written around the two named together ([`on_server`]), so a person
    /// can tell two workspaces with a `#general` apart, and the payload shows both.
    pub(crate) fn at(workspace: &str, server: &str, summary: impl FnOnce(&str) -> String) -> Self {
        let here = on_server(workspace, server);
        let summary = summary(&here);
        debug_assert!(
            summary.contains(&here),
            "a card names where it goes: {summary}"
        );
        Self::new(summary)
            .show("workspace", workspace)
            .show("server", server)
    }

    pub(crate) fn line(mut self, line: impl Into<String>) -> Self {
        self.lines.push(line.into());
        self
    }

    pub(crate) fn show(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.payload.insert(key.to_owned(), value.into());
        self
    }
}

/// What a content read may return and what it does to the chat ([`Ctx::content_scope`]).
#[derive(Clone, Debug, Default)]
pub(crate) struct ContentScope {
    /// Only public-safe content: a public model's call. Everything else is withheld and
    /// counted.
    pub(crate) public_only: bool,
    /// Raise the chat Private before the answer returns.
    pub(crate) raise: bool,
    /// The institutions the content belongs to, recorded on the chat with the raise.
    pub(crate) institutions: BTreeSet<String>,
    /// The workspace's protected channels: never public-safe.
    pub(crate) protected_channels: BTreeSet<String>,
    /// Every channel is protected: the snapshot did not say which are.
    pub(crate) all_protected: bool,
    /// For a public call, the channels the snapshot lists that it may be shown: the person's
    /// own and the public-safe ones they may join ([`listed_channels`]). The workspace says
    /// which channels are protected only among those, so any other, such as one beyond a list
    /// cut short, is withheld from a public call.
    pub(crate) listed_channels: BTreeSet<String>,
}

impl ContentScope {
    /// Whether a channel's content may be returned to this call: to a public call, only a
    /// channel the snapshot lists that is not protected.
    pub(crate) fn channel_allowed(&self, channel_id: &str) -> bool {
        !self.public_only
            || (self.unprotected(channel_id) && self.listed_channels.contains(channel_id))
    }

    /// Whether a channel listed in the snapshot is not protected.
    fn unprotected(&self, channel_id: &str) -> bool {
        !self.all_protected && !self.protected_channels.contains(channel_id)
    }
}

/// The channels `snapshot` lists that a public call may be shown, before the protected ones
/// are taken out: the person's own, and of those they may join the public-safe ones.
fn listed_channels(snapshot: &Value) -> BTreeSet<String> {
    let joinable = snapshot["joinable_channels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|channel| channel["classification"] == "public_safe");
    snapshot["channels"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(joinable)
        .filter_map(|channel| channel["id"].as_str().map(str::to_owned))
        .collect()
}

/// A setup job this host started, and the target it started it on, so `open` can save a
/// workspace it found with the same login, route and sign-in.
static JOB_TARGETS: LazyLock<Mutex<HashMap<String, Value>>> = LazyLock::new(Mutex::default);
/// How many job targets are remembered.
const JOB_TARGETS_KEPT: usize = 64;

/// Remember the target `job_id` was started on.
pub(crate) fn remember_job_target(job_id: &str, target: Value) {
    let Ok(mut targets) = JOB_TARGETS.lock() else {
        return;
    };
    if targets.len() >= JOB_TARGETS_KEPT {
        targets.clear();
    }
    targets.insert(job_id.to_owned(), target);
}

/// The target a job this host started was started on.
pub(crate) fn job_target(job_id: &str) -> Option<Value> {
    JOB_TARGETS.lock().ok()?.get(job_id).cloned()
}

/// Read consents the person declined, by chat and connection, each with how many messages the
/// person had written in the chat when they declined ([`person_messages`]). Until they write
/// again, a read of that workspace in that chat is said as declined and parks no new card
/// ([`Ctx::read_consent`]).
static DECLINED_READS: LazyLock<Mutex<HashMap<(String, String), usize>>> =
    LazyLock::new(Mutex::default);
/// How many declined reads are remembered.
const DECLINED_READS_KEPT: usize = 256;

/// Said when the person declines a chat's read of a workspace on its consent card.
pub(crate) const READ_DECLINED: &str = "The person declined. Nothing was read.";

/// Forget a declined read (`key`, chat and connection): the person wrote again, or allowed it.
fn forget_decline(key: &(String, String)) {
    if let Ok(mut declined) = DECLINED_READS.lock() {
        declined.remove(key);
    }
}

/// How many messages the person has written in `session`: user messages with text that both
/// the person and the model see, as their own message and a steer are. A tool's answer and a
/// note the agent loop adds for one side only are not. `None` when the chat can't be read.
async fn person_messages(state: &AppState, session: &str) -> Option<usize> {
    let session = state
        .session_manager()
        .get_session(session, true)
        .await
        .ok()?;
    let conversation = session.conversation?;
    Some(
        conversation
            .messages()
            .iter()
            .filter(|message| {
                message.role == rmcp::model::Role::User
                    && message.is_user_visible()
                    && message.is_agent_visible()
                    && !message.is_tool_response()
                    && !message.as_concat_text().trim().is_empty()
            })
            .count(),
    )
}

/// The read consent card's words (AO-12, "Read consent"): a heading, then two sentences, what
/// the chat may then read and that changes still ask, and where the person takes it back.
pub(crate) fn consent_prompt(workspace: &str, server: &str, session: &str) -> String {
    format!(
        "Let this chat read {workspace} as you\nIt can then read what you see in {workspace} on {server} (messages, files, names and the context document), and every change still asks you first. Take this back in this chat's Crew bar, or with biorouter crew grants revoke {session} --reads."
    )
}

/// A long piece of work's stage, which [`Ctx::follow`] reports while it runs, and whether it
/// waits for the person on a card ([`Self::awaiting_person`]), which [`Ctx::follow`] does not
/// count against [`JOB_WAIT`].
#[derive(Clone, Default)]
pub(crate) struct Progress {
    stage: Arc<Mutex<Value>>,
    person: Arc<Mutex<PersonWait>>,
}

/// How long a piece of work has waited for the person ([`Progress::awaiting_person`]).
#[derive(Default)]
struct PersonWait {
    /// How many waits are open now: a card inside a card's wait counts once.
    open: usize,
    /// Since when it waits, while any wait is open.
    since: Option<Instant>,
    /// How long the waits that closed took.
    waited: Duration,
}

/// Held while a piece of work waits for the person ([`Progress::awaiting_person`]).
pub(crate) struct AwaitingPerson(Progress);

impl Drop for AwaitingPerson {
    fn drop(&mut self) {
        let mut person = self.0.person();
        person.open = person.open.saturating_sub(1);
        if person.open == 0 {
            if let Some(since) = person.since.take() {
                person.waited += since.elapsed();
            }
        }
    }
}

impl Progress {
    /// Record where the work stands.
    pub(crate) fn set(&self, key: &str, value: impl Into<Value>) {
        if let Ok(mut state) = self.stage.lock() {
            if !state.is_object() {
                *state = json!({});
            }
            state[key] = value.into();
        }
    }

    fn snapshot(&self) -> Value {
        self.stage
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default()
    }

    fn person(&self) -> std::sync::MutexGuard<'_, PersonWait> {
        self.person
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The work waits for the person (a sign-in card is up) until the guard drops. Meanwhile
    /// [`Ctx::follow`] does not answer that the work is still running: that would end the turn,
    /// whose token takes the card down while the person is signing in.
    pub(crate) fn awaiting_person(&self) -> AwaitingPerson {
        let mut person = self.person();
        person.open += 1;
        person.since.get_or_insert_with(Instant::now);
        drop(person);
        AwaitingPerson(self.clone())
    }

    /// Whether the work waits for the person now.
    fn awaiting(&self) -> bool {
        self.person().open > 0
    }

    /// How long the work has waited for the person, all told.
    fn person_time(&self) -> Duration {
        let person = self.person();
        person.waited + person.since.map_or(Duration::ZERO, |since| since.elapsed())
    }
}

/// Wait for `task` for at most `budget`, not counting the time `progress` says it waited for
/// the person, or until `stop`. `None` when the budget ran out or the call was stopped first.
/// A call bridged from a coding agent stops at its deadline as well, the person's time
/// included (D3): a card the work parked ends by then too ([`park_step_in`]).
async fn finished_within<T>(
    task: &mut tokio::task::JoinHandle<T>,
    progress: &Progress,
    stop: &CancellationToken,
    budget: Duration,
) -> Option<Result<T, tokio::task::JoinError>> {
    let started = Instant::now();
    let out_of_time = || call_deadline().is_some_and(|deadline| Instant::now() >= deadline);
    loop {
        let awaiting = progress.awaiting();
        let left = (budget + progress.person_time()).saturating_sub(started.elapsed());
        if (left.is_zero() && !awaiting) || out_of_time() {
            return None;
        }
        // While the person is at a card the budget stands still; look again soon.
        let nap = within_call(if awaiting { JOB_POLL } else { left });
        tokio::select! {
            joined = &mut *task => return Some(joined),
            () = tokio::time::sleep(nap) => {}
            () = stop.cancelled() => return None,
        }
    }
}

impl Ctx {
    // ----------------------------------------------------------------------------- parameters

    fn param(&self, name: &str) -> Option<&Value> {
        self.call.params.get(name).filter(|value| !value.is_null())
    }

    /// Whether the call gave the parameter at all (not `null`).
    pub(crate) fn param_given(&self, name: &str) -> bool {
        self.param(name).is_some()
    }

    /// A text parameter, trimmed; `None` when absent or blank.
    pub(crate) fn text(&self, name: &str) -> Option<String> {
        self.param(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    }

    /// A text parameter as given, untrimmed (a message, a document).
    pub(crate) fn raw_text(&self, name: &str) -> Option<String> {
        self.param(name).and_then(Value::as_str).map(str::to_owned)
    }

    /// A required text parameter.
    pub(crate) fn required(&self, name: &str) -> Result<String> {
        self.text(name)
            .ok_or_else(|| said(None, format!("{} needs {name}.", self.action.name)))
    }

    pub(crate) fn flag(&self, name: &str) -> Option<bool> {
        self.param(name).and_then(Value::as_bool)
    }

    pub(crate) fn number(&self, name: &str) -> Option<u64> {
        self.param(name).and_then(Value::as_u64)
    }

    pub(crate) fn texts(&self, name: &str) -> Vec<String> {
        self.param(name)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The connection text the call named, for a refusal's step.
    fn named_connection(&self) -> Option<String> {
        self.text("connection")
    }

    // ------------------------------------------------------------------------------ the chat

    pub(crate) fn crew(&self) -> Result<Arc<CrewManager>> {
        manager()
    }

    /// The chat's own classification, as stored now. Unreadable reads as Private, the side a
    /// crossing is said on.
    pub(crate) async fn chat_classification(&self) -> SessionClassification {
        self.state
            .session_manager()
            .get_session(&self.call.session_id, false)
            .await
            .map_or(SessionClassification::Private, |session| {
                session.privacy_tier
            })
    }

    /// Whether this call may be shown the chat `session` in a listing or by name (AO-12): the
    /// rule a model's own listing of chats follows (`appears_in_list`, which `workspace_list`
    /// asks), on the call's one sampled capability, against the chat's stored classification.
    /// A chat that cannot be read counts as Private. With privacy tiers switched off, every chat
    /// is listed. The one filter for every action that names another chat: the grants a
    /// connection holds (`grants_list`, `grants_revoke`) and the tasks this computer runs, each
    /// a chat of its own (`tasks_list`, `tasks_watch`).
    pub(crate) async fn lists_chat(&self, session: &str) -> bool {
        let capability = self.call.capability;
        if !capability.enforced() {
            return true;
        }
        let classification = self
            .state
            .session_manager()
            .get_session(session, false)
            .await
            .map_or(SessionClassification::Private, |found| found.privacy_tier);
        appears_in_list(capability.tier(), classification)
    }

    // ----------------------------------------------------------------------------- resolution

    /// The saved connection the call names (`connection`: its name, SSH login or ID), or the
    /// only one saved, resolved as `POST /crew/resolve` resolves it. Connection names are not
    /// workspace content: the connection list goes to any chat.
    pub(crate) async fn connection(&self) -> Result<Connection> {
        let crew = self.crew()?;
        let connections = crew.list().await;
        let named = self.text("connection");
        let (_, id) = super::crew::names::choose_connection(&connections, named.as_deref(), true)
            .map_err(connection_refusal)?;
        let id =
            id.ok_or_else(|| said(Some("crew_connection_required"), "Name the connection."))?;
        saved_connection(&crew, &id).await
    }

    /// The person's own snapshot of `connection`'s workspace, as `biorouter crew workspace
    /// show` reads it. Nothing in it reaches the model until [`Self::content_scope`],
    /// [`Self::read_consent`] and [`Self::release`] say so.
    pub(crate) async fn snapshot(&self, connection: &Connection) -> Result<Value> {
        self.crew()?
            .human_request(&connection.id, "workspace.snapshot", json!({}), None)
            .await
    }

    /// The workspace's name, as its snapshot signs it, else the connection's.
    pub(crate) fn workspace_label(connection: &Connection, snapshot: Option<&Value>) -> String {
        snapshot
            .and_then(|snapshot| snapshot["workspace"]["name"].as_str())
            .filter(|name| biorouter_crew::workspace_name_valid(name))
            .map_or_else(|| connection.name.clone(), str::to_owned)
    }

    /// The server a connection reaches, as a person names it.
    pub(crate) async fn server_label(connection: &Connection) -> String {
        biorouter::crew::server_label(&connection.ssh_target, connection.port).await
    }

    /// One name (`@bob`, `#methods`, `analysis-lab`) resolved in `snapshot` as
    /// `POST /crew/resolve` resolves it: its ID, label and username. An unknown or ambiguous
    /// name is refused with the resolver's own words, which list only what the person can see
    /// there; a caller passes such a refusal on only under a scope that may name things from
    /// the workspace, through [`naming`], and words its own otherwise, so no name from a
    /// withheld workspace reaches a refusal and none returns before the chat is raised.
    pub(crate) fn resolve(
        snapshot: &Value,
        kind: super::crew::names::SelectorKind,
        text: &str,
    ) -> Result<Resolved> {
        use super::crew::names::{resolve_all, Resolution, SelectorInput};
        let selector = SelectorInput {
            kind: Some(kind),
            text: text.to_owned(),
        };
        match resolve_all(Some(snapshot), &[], &[selector]).pop() {
            Some(Resolution::Resolved {
                id,
                label,
                username,
                ..
            }) => Ok(Resolved {
                label: label.clone().unwrap_or_else(|| text.to_owned()),
                id,
                username,
            }),
            Some(Resolution::AmbiguousName { candidates, .. }) => Err(said(
                Some("ambiguous_name"),
                format!(
                    "More than one matches {text}: {}. Name it exactly.",
                    candidates.join("; ")
                ),
            )),
            Some(Resolution::UnknownName { did_you_mean, .. }) => Err(said(
                Some("unknown_name"),
                match did_you_mean {
                    Some(name) => {
                        format!("Nothing you can see is named {text}. Did you mean {name}?")
                    }
                    None => format!("Nothing you can see is named {text}."),
                },
            )),
            None => Err(said(
                Some("unknown_name"),
                format!("Nothing you can see is named {text}."),
            )),
        }
    }

    // -------------------------------------------------------------------------------- privacy

    /// What a read of `connection`'s workspace content may return to this call (AO-12,
    /// "Privacy"), from the call's capability, sampled once at dispatch, and the workspace as
    /// its snapshot describes it. A refusal here names nothing from the workspace.
    ///
    /// * A public model gets only what a public-provider run gets: the connection and the
    ///   workspace both Public, and of them only public-safe channels (never a protected
    ///   one). Everything else is withheld.
    /// * A private model passes the institution checks a grant passes
    ///   (`institution::admission`): one institution between connection and workspace, and,
    ///   when what it reads is protected (a Private connection or workspace, or restricted
    ///   channels), a labelled workspace, a labelled Private connection, and a model whose
    ///   affiliation covers their institutions. Protected content raises the chat Private.
    /// * Either way, a chat whose content already belongs to an institution reads only a
    ///   workspace of that institution ([`Self::origin_admits`]), as a grant into it would be
    ///   admitted, so two institutions' private Crew content never meet in one chat.
    pub(crate) async fn content_scope(
        &self,
        connection: &Connection,
        snapshot: &Value,
    ) -> Result<ContentScope> {
        let workspace = &snapshot["workspace"];
        // A workspace that does not say its mode is treated as Private: fail closed.
        let workspace_mode = serde_json::from_value::<ClusterMode>(workspace["mode"].clone())
            .unwrap_or(ClusterMode::Private);
        let protected: Option<BTreeSet<String>> =
            serde_json::from_value(snapshot["protected_channel_ids"].clone()).ok();
        let mut scope = ContentScope {
            all_protected: protected.is_none(),
            protected_channels: protected.unwrap_or_default(),
            ..ContentScope::default()
        };
        let institution = |value: Option<&str>| {
            value
                .map(|id| id.trim().to_ascii_lowercase())
                .filter(|id| !id.is_empty())
        };
        let workspace_institution = institution(workspace["institution_id"].as_str());
        if self.call.capability.tier() == ProviderTier::Public {
            if connection.mode != ClusterMode::Public || workspace_mode != ClusterMode::Public {
                return Err(said(
                    Some(crew_refusal::PUBLIC_MODEL_REFUSED),
                    PUBLIC_MODEL_READ,
                ));
            }
            self.origin_admits(workspace_institution.as_deref()).await?;
            scope.public_only = true;
            scope.listed_channels = listed_channels(snapshot);
            return Ok(scope);
        }
        let connection_institution = institution(connection.institution_id.as_deref());
        if let (Some(ours), Some(theirs)) = (&connection_institution, &workspace_institution) {
            if ours != theirs {
                return Err(said(
                    Some(crew_refusal::INSTITUTION_MISMATCH),
                    INSTITUTIONS_DIFFER,
                ));
            }
        }
        // Where a grant's admission checks it: after the connection and the workspace agree,
        // before anything about the content.
        self.origin_admits(workspace_institution.as_deref()).await?;
        let protected = connection.mode == ClusterMode::Private
            || workspace_mode == ClusterMode::Private
            || scope.all_protected
            || !scope.protected_channels.is_empty();
        if !protected {
            // All of it is public-safe: nothing to label, nothing to raise.
            return Ok(scope);
        }
        if workspace_institution.is_none() {
            return Err(said(None, UNLABELLED_PRIVATE));
        }
        if connection.mode == ClusterMode::Private && connection_institution.is_none() {
            return Err(said(None, CONNECTION_WITHOUT_INSTITUTION));
        }
        scope.institutions = connection_institution
            .into_iter()
            .chain(workspace_institution)
            .collect();
        // The check a grant passes (`institution::check_provider`), on this call's model.
        self.call
            .capability
            .check_crew_institutions(&scope.institutions)?;
        scope.raise = true;
        Ok(scope)
    }

    /// The chat's recorded institutions against the workspace's (`workspace_institution`,
    /// normalized), by a grant's own rule (`CallCapability::check_crew_origin`): a chat that
    /// already holds content an institution owns reads no workspace of another institution, nor
    /// one with none. Affiliations that can't be read refuse, so an unknown answer never reads
    /// as none (fail closed). With privacy tiers switched off nothing is recorded on the chat
    /// ([`Self::release`]), so nothing is checked here either (AR-7). A refusal names nothing
    /// from the workspace.
    async fn origin_admits(&self, workspace_institution: Option<&str>) -> Result<()> {
        let capability = self.call.capability;
        if !capability.enforced() {
            return Ok(());
        }
        let recorded = self
            .state
            .session_manager()
            .session_affiliations(&self.call.session_id)
            .await
            .map_err(|_| said(None, AFFILIATIONS_UNREADABLE))?;
        let origin: BTreeSet<String> = recorded
            .iter()
            .map(|institution| institution.as_str().to_owned())
            .collect();
        capability
            .check_crew_origin(&origin, workspace_institution)
            .map_err(|_| {
                said(
                    Some(crew_refusal::INSTITUTION_MISMATCH),
                    ANOTHER_INSTITUTIONS_CONTENT,
                )
            })
    }

    /// The person's consent for this chat to read `connection`'s workspace as them (AO-12,
    /// "Read consent"): already given in this chat, or asked for now on one proof-backed card.
    /// Declined, the read says the person declined, and until they write in the chat again a
    /// retry parks no second card ([`Self::still_declined`]); a card that expired or was
    /// stopped answers `crew_read_consent_required`. On a daemon that can never
    /// show the card (`biorouter serve`), a sentence says so instead of parking one. Answers
    /// whether a card was asked and allowed now, after which the caller judges the workspace
    /// again ([`Self::consented`]): the person may have taken minutes to answer.
    pub(crate) async fn read_consent(
        &self,
        connection: &Connection,
        workspace: &str,
    ) -> Result<bool> {
        let crew = self.crew()?;
        if crew
            .has_person_read(&self.call.session_id, &connection.id)
            .await
        {
            return Ok(false);
        }
        if !user_proof_available() {
            return Err(said(
                Some(crew_refusal::READ_CONSENT_REQUIRED),
                NO_PROOF_FOR_READS,
            ));
        }
        let declined = (self.call.session_id.clone(), connection.id.clone());
        if self.still_declined(&declined).await {
            return Err(said(
                None,
                format!(
                    "The person declined to let this chat read {workspace} as them, so nothing was read. Biorouter asks them again only after they write in this chat."
                ),
            ));
        }
        let server = Self::server_label(connection).await;
        let mut arguments = Map::new();
        arguments.insert("action".into(), json!("read as you"));
        arguments.insert("workspace".into(), json!(workspace));
        arguments.insert("server".into(), json!(server));
        arguments.insert("connection".into(), json!(connection.name));
        let prompt = consent_prompt(workspace, &server, &self.call.session_id);
        let outcome = self
            .park(
                ToolRisk::Low,
                prompt,
                arguments.clone(),
                ToolPreview::arguments_within(&arguments, 4096),
            )
            .await;
        if let UserActionOutcome::Denied { .. } = outcome {
            self.remember_decline(declined).await;
            return Err(said(None, READ_DECLINED));
        }
        if !outcome.is_allowed() || self.call.cancel.is_cancelled() {
            return Err(CrewRefusal::read_consent_required(workspace).into());
        }
        self.call.ensure_still_ungranted().await?;
        crew.grant_person_read(&self.call.session_id, &connection.id)
            .await?;
        forget_decline(&declined);
        Ok(true)
    }

    /// Whether the person declined this chat's read of a connection's workspace (`key`, chat
    /// and connection) and has not written in the chat since: within the rest of that turn the
    /// read is not asked again. Once they have written, the decline is forgotten and the next
    /// read asks. A chat whose messages can't be read keeps the decline, so a retry never
    /// parks a card the person already answered.
    async fn still_declined(&self, key: &(String, String)) -> bool {
        let Some(then) = DECLINED_READS
            .lock()
            .ok()
            .and_then(|declined| declined.get(key).copied())
        else {
            return false;
        };
        match person_messages(&self.state, &self.call.session_id).await {
            Some(now) if now > then => {
                forget_decline(key);
                false
            }
            _ => true,
        }
    }

    /// Remember that the person declined the read `key`, with how many messages they had
    /// written in the chat then. Kept in memory only, at most [`DECLINED_READS_KEPT`] at once.
    async fn remember_decline(&self, key: (String, String)) {
        let written = person_messages(&self.state, &self.call.session_id)
            .await
            .unwrap_or(0);
        if let Ok(mut declined) = DECLINED_READS.lock() {
            if declined.len() >= DECLINED_READS_KEPT {
                declined.clear();
            }
            declined.insert(key, written);
        }
    }

    /// A content read's connection, its snapshot and the read's privacy scope, with the
    /// person's consent ([`Self::read_consent`]). When the consent card had to be asked, the
    /// workspace or the connection may have changed while it waited (made Private, a channel
    /// protected), so both are read again and the scope judged anew: what returns is judged on
    /// them as they are when it is fetched, and a scope that now refuses refuses the read.
    pub(crate) async fn consented(
        &self,
        connection: Connection,
    ) -> Result<(Connection, Value, ContentScope)> {
        let snapshot = self.snapshot(&connection).await?;
        let scope = self.content_scope(&connection, &snapshot).await?;
        let workspace = Self::workspace_label(&connection, Some(&snapshot));
        if !self.read_consent(&connection, &workspace).await? {
            return Ok((connection, snapshot, scope));
        }
        self.rescoped(&connection.id).await
    }

    /// The saved connection `id`, its snapshot and a read's scope, all read again.
    pub(crate) async fn rescoped(&self, id: &str) -> Result<(Connection, Value, ContentScope)> {
        let crew = self.crew()?;
        let connection = saved_connection(&crew, id).await?;
        let snapshot = self.snapshot(&connection).await?;
        let scope = self.content_scope(&connection, &snapshot).await?;
        Ok((connection, snapshot, scope))
    }

    /// Return `answer`, raising the chat Private first when `scope` says its content is not
    /// all public (AO-12): the raise is a permanent ratchet, and if it fails nothing returns.
    /// With privacy tiers switched off, nothing is raised (AR-7).
    pub(crate) async fn release(&self, scope: &ContentScope, answer: Value) -> Result<Value> {
        if scope.raise && self.call.capability.enforced() {
            let sessions = self.state.session_manager();
            let raised = sessions
                .update(&self.call.session_id)
                .raise_privacy(SessionClassification::Private, "mcp:crew")
                .apply()
                .await;
            if raised.is_err() {
                return Err(said(None, RAISE_FAILED));
            }
            for institution in &scope.institutions {
                if sessions
                    .record_required_session_affiliation(
                        &self.call.session_id,
                        InstitutionId::new(institution),
                    )
                    .await
                    .is_err()
                {
                    return Err(said(None, RAISE_FAILED));
                }
            }
        }
        Ok(answer)
    }

    /// A content read, whole: the snapshot, the privacy scope, the consent, then `pick` on what
    /// the call may see, and the raise. `pick` gets the snapshot already narrowed to the
    /// scope ([`public_snapshot`]) and the scope itself; a refusal of its own that lists names
    /// from the snapshot passes through [`naming`].
    pub(crate) async fn read_content(
        &self,
        pick: impl FnOnce(&Value, &ContentScope) -> Result<Value>,
    ) -> Result<Value> {
        let connection = self.connection().await?;
        let (_, snapshot, scope) = self.consented(connection).await?;
        let visible = public_snapshot(&snapshot, &scope);
        let answer = pick(&visible, &scope)?;
        self.release(&scope, answer).await
    }

    // ------------------------------------------------------------------------------ approvals

    /// Park one proof-backed card in this chat, named for the tool the call came through, and
    /// wait for the person, [`APPROVAL_TTL`] at most, or until Stop. A call bridged from a
    /// coding agent waits only until its deadline ([`within_call`]), and parks nothing once that
    /// has passed.
    async fn park(
        &self,
        risk: ToolRisk,
        prompt: String,
        arguments: Map<String, Value>,
        preview: ToolPreview,
    ) -> UserActionOutcome {
        let ttl = within_call(APPROVAL_TTL);
        if ttl.is_zero() {
            return UserActionOutcome::TimedOut;
        }
        let request = UserActionRequest::ToolApproval(ToolApprovalRequest {
            tool_name: self.call.tool.name().to_owned(),
            arguments,
            // A card with a prompt offers Allow once and Deny, never Always allow.
            prompt: Some(prompt),
            risk: Some(risk),
            preview: Some(preview),
            requires_user_proof: true,
        });
        self.call
            .approvals
            .park(Some(&self.call.session_id), None, request)
            .wait(ttl, Some(&self.call.cancel))
            .await
    }

    /// Ask the person to allow this change, on one proof-backed card (AO-12, "Approvals"):
    /// Allow once or Deny, its summary written from targets resolved before it was parked, its
    /// payload shown whole. Answers only once they allowed it and the chat still has no Crew
    /// grant; every other outcome is a refusal saying nothing was changed.
    pub(crate) async fn approve(&self, card: Card) -> Result<()> {
        // A change that needs a locked vault asks for it first, so its approval is asked once.
        vault::before_approval(self).await?;
        if !user_proof_available() {
            return Err(said(None, NO_PROOF));
        }
        let mut arguments = Map::new();
        arguments.insert("action".into(), json!(self.action.name));
        arguments.extend(card.payload);
        let mut prompt = card.summary;
        for line in &card.lines {
            prompt.push('\n');
            prompt.push_str(line);
        }
        let preview = card
            .preview
            .unwrap_or_else(|| ToolPreview::arguments_within(&arguments, CARD_PAYLOAD_CHARS));
        let shown =
            json!({"prompt": prompt, "arguments": arguments, "preview": preview}).to_string();
        // The one retry after a host-key card, of the very card the person allowed in this call:
        // not asked twice, but every check after an approval is made again.
        if allowed_for_this_retry(&shown, Instant::now()) {
            if self.call.cancel.is_cancelled() {
                return Err(not_approved(&UserActionOutcome::Cancelled));
            }
            return self.call.ensure_still_ungranted().await;
        }
        let outcome = self.park(ToolRisk::High, prompt, arguments, preview).await;
        match outcome {
            UserActionOutcome::Approved { .. } if !self.call.cancel.is_cancelled() => {
                self.call.ensure_still_ungranted().await?;
                remember_allowed(shown, Instant::now());
                Ok(())
            }
            other => Err(not_approved(&other)),
        }
    }

    /// Park a Crew step card in this chat (the owner's D1 ruling): a sign-in, the vault or a
    /// host key, written from the daemon's own records by the caller. Waits for nothing:
    /// [`PendingStep::end`] waits, at most `ttl` (never more than [`APPROVAL_TTL`], nor past a
    /// bridged call's deadline), for the person to dismiss it, the chat to stop, or the caller
    /// to withdraw it once the daemon sees the step done.
    ///
    /// `None` where no card can be answered: no person's proof can be obtained in this process
    /// (`biorouter serve`, SD-8), the call has no surface a person watches
    /// (`POST /agent/call_tool`), it was stopped, or a bridged call has no time left. The
    /// caller then says the step in words, as before.
    pub(crate) fn park_step(&self, step: CrewStepRequest, ttl: Duration) -> Option<PendingStep> {
        if biorouter::user_surface::no_human_surface() {
            return None;
        }
        park_step_in(
            &self.call.approvals,
            &self.call.session_id,
            &self.call.cancel,
            step,
            ttl,
        )
    }

    // ------------------------------------------------------------------------------ long work

    /// Wait for the setup job `job_id` for at most [`JOB_WAIT`], or until Stop, then answer
    /// where it stands ([`job_answer`]). The job keeps running either way, visible in Crew.
    ///
    /// Each status read goes first to the sign-in and host-key hooks ([`signin::on_status`],
    /// [`hostkey::on_status`]), which may park a card for the person and answer instead. A hook
    /// that waited returns `None`, and the job is read again before it is judged. A call
    /// bridged from a coding agent answers by its deadline, whatever the person's card took
    /// ([`no_later_than_the_call`]).
    pub(crate) async fn wait_for_job(&self, job_id: &str) -> Value {
        let mut deadline = Instant::now() + JOB_WAIT;
        let gone = || {
            json!({"job_id": job_id, "state": "unknown",
                "note": "That job is not on this computer any more."})
        };
        loop {
            let Some(status) = biorouter::crew::setup_status(job_id) else {
                return gone();
            };
            let asked = Instant::now();
            if let Some(answer) = signin::on_status(self, &status).await {
                return answer;
            }
            if let Some(answer) = retrying_after_a_host_key(hostkey::on_status(self, &status)).await
            {
                return answer;
            }
            // The time the person spent on a card a hook parked is not counted against
            // JOB_WAIT: answering "running" then would end the turn while they sign in.
            deadline += asked.elapsed();
            let Some(status) = biorouter::crew::setup_status(job_id) else {
                return gone();
            };
            let settled = !matches!(status.state, SetupState::Running);
            if settled
                || Instant::now() >= no_later_than_the_call(deadline)
                || self.call.cancel.is_cancelled()
            {
                return job_answer(&status, !settled);
            }
            tokio::select! {
                () = self.call.cancel.cancelled() => {}
                () = tokio::time::sleep(JOB_POLL) => {}
            }
        }
    }

    /// Run `work` (a flow of several steps, such as open or join) for at most [`JOB_WAIT`], or
    /// until Stop. Done in time, its answer; otherwise it keeps running and the answer says
    /// where it stands, from `progress`. Time `work` spends waiting for the person on a card
    /// ([`Progress::awaiting_person`]) is not counted, except against a bridged call's deadline,
    /// which `work` carries too, so a card it parks ends by then ([`CALL_DEADLINE`]).
    pub(crate) async fn follow<F>(&self, progress: Progress, work: F) -> Result<Value>
    where
        F: std::future::Future<Output = Result<Value>> + Send + 'static,
    {
        let mut task = tokio::spawn(CALL_DEADLINE.scope(call_deadline(), work));
        let finished = finished_within(&mut task, &progress, &self.call.cancel, JOB_WAIT).await;
        match finished {
            Some(Ok(answer)) => answer,
            Some(Err(_)) => Err(said(None, FAILED)),
            None => {
                let mut answer = progress.snapshot();
                if !answer.is_object() {
                    answer = json!({});
                }
                answer["state"] = json!("running");
                answer["note"] = json!("Still going; it keeps running in Crew. Check it with crew__read action status.");
                Ok(answer)
            }
        }
    }
}

/// A name [`Ctx::resolve`] resolved.
pub(crate) struct Resolved {
    pub(crate) id: String,
    /// How a person reads it: `Bob Lee (@bob)`, `#methods`.
    pub(crate) label: String,
    /// A person's canonical username, to send as `expected_username`.
    pub(crate) username: Option<String>,
}

/// The saved connection `id`, as it is saved now.
pub(crate) async fn saved_connection(crew: &CrewManager, id: &str) -> Result<Connection> {
    crew.connection(id).await.map_err(|_| {
        said(
            Some(super::crew_authentication::CONNECTION_NOT_FOUND_CODE),
            NO_CONNECTION,
        )
    })
}

fn connection_refusal(refusal: super::crew::names::ResolveRefusal) -> anyhow::Error {
    use super::crew::names::{Resolution, ResolveRefusal};
    match refusal {
        ResolveRefusal::Invalid(message) => said(Some("crew_invalid_selector"), message),
        ResolveRefusal::ConnectionRequired(message) => {
            said(Some("crew_connection_required"), message)
        }
        ResolveRefusal::Connection(Resolution::AmbiguousName {
            text, candidates, ..
        }) => said(
            Some("ambiguous_name"),
            format!(
                "More than one saved connection matches {text}: {}. Use its full name or its server.",
                candidates.join("; ")
            ),
        ),
        ResolveRefusal::Connection(
            Resolution::UnknownName { text, .. } | Resolution::Resolved { text, .. },
        ) => said(
            Some("unknown_name"),
            format!("No saved Crew connection is named {text}. List them with crew__read action status."),
        ),
    }
}

/// `snapshot` as `scope` lets the call see it. For a public model (AO-12, "Privacy"): only the
/// person's channels that are not protected, of the channels they may join only the public-safe
/// ones (the snapshot names no protected channel the person is not in, so a joinable channel's
/// own classification is what says), invitations to teams and to the channels shown, and
/// references the workspace does not restrict in channels shown. Each list's count in `totals`
/// is lowered by what was withheld from it, so the name resolver reads the narrowed view as
/// whole exactly when the person's own list was whole, and `withheld` says how many: always
/// `channels`, and `joinable_channels`, `invitations` and `references` when the snapshot lists
/// them. This is the one narrowing every content read takes ([`Ctx::read_content`] and
/// `places::reading`).
pub(crate) fn public_snapshot(snapshot: &Value, scope: &ContentScope) -> Value {
    let mut visible = snapshot.clone();
    if !scope.public_only {
        return visible;
    }
    // Entries of the snapshot's own lists: listed by definition, so only protection decides.
    let allowed = |id: &Value| id.as_str().is_some_and(|id| scope.unprotected(id));
    let mut withheld = Map::new();
    let channels = narrow(&mut visible, "channels", |channel| allowed(&channel["id"]));
    withheld.insert("channels".into(), json!(channels.unwrap_or(0)));
    let joinable = narrow(&mut visible, "joinable_channels", |channel| {
        channel["classification"] == "public_safe" && allowed(&channel["id"])
    });
    let shown: BTreeSet<String> = ["channels", "joinable_channels"]
        .iter()
        .filter_map(|key| visible[*key].as_array())
        .flatten()
        .filter_map(|channel| channel["id"].as_str().map(str::to_owned))
        .collect();
    let shown_channel = |id: &Value| id.as_str().is_some_and(|id| shown.contains(id));
    let invitations = narrow(&mut visible, "invitations", |invitation| {
        invitation["kind"] == "team" || shown_channel(&invitation["target_id"])
    });
    let references = narrow(&mut visible, "references", |reference| {
        reference["restricted"] == false && shown_channel(&reference["channel_id"])
    });
    for (key, count) in [
        ("joinable_channels", joinable),
        ("invitations", invitations),
        ("references", references),
    ] {
        if let Some(count) = count {
            withheld.insert(key.into(), json!(count));
        }
    }
    if let Some(object) = visible.as_object_mut() {
        object.remove("protected_channel_ids");
        object.insert("withheld".into(), Value::Object(withheld));
    }
    visible
}

/// Keep the entries of `visible`'s list `key` that `keep` allows, and lower its count in
/// `totals` by how many went. `None` when the snapshot has no such list.
fn narrow(visible: &mut Value, key: &str, keep: impl Fn(&Value) -> bool) -> Option<u64> {
    let list = visible.get_mut(key)?.as_array_mut()?;
    let before = list.len();
    list.retain(|entry| keep(entry));
    let gone = u64::try_from(before - list.len()).unwrap_or(u64::MAX);
    if let Some(total) = visible["totals"].get(key).and_then(Value::as_u64) {
        visible["totals"][key] = json!(total.saturating_sub(gone));
    }
    Some(gone)
}

/// What a setup job's waiting action answers: the job as `GET /crew/setup/{job_id}` reads it
/// (`job_id`, `state`, `stage` first), with the step the person takes when it waits for them.
pub(crate) fn job_answer(status: &SetupStatus, still_running: bool) -> Value {
    let mut answer = serde_json::to_value(status).unwrap_or_else(|_| json!({}));
    chat_unlisted_note(status, &mut answer);
    let job = status.job_id.as_str();
    if still_running {
        answer["note"] = json!(
            "Still running; it keeps running in Crew. Follow it with crew__read action setup_status."
        );
    }
    match status.state {
        SetupState::NeedsSignIn => {
            answer["code"] = json!(signin::AUTH_REQUIRED);
            answer["next_step"] = json!(signin::job_step(job));
        }
        SetupState::Failed => {
            if let Some(error) = status.error.as_ref() {
                if let Some(step) = next_step(&error.code, None) {
                    answer["next_step"] = json!(hostkey::enrich(
                        step,
                        Some(&error.code),
                        &setup_error_fields(error)
                    ));
                }
            }
        }
        _ => {}
    }
    answer
}

/// A failed setup job's own words beside its code, as [`hostkey::enrich`] takes an admission
/// refusal's fields: `detail` (OpenSSH's or the server's bounded words) and `host` (the SSH hop
/// a connect failure concerns), each only when the job has it.
pub(crate) fn setup_error_fields(error: &biorouter::crew::SetupError) -> Vec<(String, Value)> {
    [("detail", &error.detail), ("host", &error.host)]
        .into_iter()
        .filter_map(|(key, value)| value.as_ref().map(|value| (key.to_owned(), json!(value))))
        .collect()
}

/// What a chat does where a Crew too old to list the workspaces asks for an update: the
/// daemon's note names the command line's command, which a chat cannot run.
pub(crate) const UPDATE_TO_LIST: &str =
    "Update it with crew__change action setup, which then lists them.";

/// A job whose Crew is too old to list the workspaces (`workspaces_unlisted` `too_old`) says
/// so in a note that ends with the command line's command (`biorouter crew setup --server …`).
/// The chat is told its own action instead, as its other refusals name one, in the note and in
/// the `workspaces` stage that repeats it. A note that names no command is left as it is.
fn chat_unlisted_note(status: &SetupStatus, answer: &mut Value) {
    if status.workspaces_unlisted != Some(SetupWorkspacesUnlisted::TooOld) {
        return;
    }
    let Some(note) = status.workspaces_note.as_deref() else {
        return;
    };
    let Some((before, _)) = note.split_once("biorouter crew ") else {
        return;
    };
    // The command's whole sentence goes, from the end of the sentence before it.
    let said = match before.rsplit_once(". ") {
        Some((kept, _)) => format!("{kept}. {UPDATE_TO_LIST}"),
        None => UPDATE_TO_LIST.to_owned(),
    };
    answer["workspaces_note"] = json!(said);
    if let Some(stages) = answer["stages"].as_array_mut() {
        for stage in stages {
            if stage["detail"].as_str() == Some(note) {
                stage["detail"] = json!(said);
            }
        }
    }
}

/// The lines of `old` and `new` as a diff preview, for a document a change replaces. Lines
/// common to both are kept as context; a document too long to compare line by line is shown
/// as removed and added whole.
pub(crate) fn document_diff(path: &str, old: &str, new: &str) -> ToolPreview {
    const MAX_COMPARED: usize = 2000;
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let line = |kind, text: &str| ToolPreviewLine {
        kind,
        text: text.to_owned(),
    };
    let mut lines = Vec::new();
    if old_lines.len() > MAX_COMPARED || new_lines.len() > MAX_COMPARED {
        lines.extend(
            old_lines
                .iter()
                .map(|text| line(ToolPreviewLineKind::Removed, text)),
        );
        lines.extend(
            new_lines
                .iter()
                .map(|text| line(ToolPreviewLineKind::Added, text)),
        );
    } else {
        // The longest common subsequence, then a walk that emits it as context.
        let (n, m) = (old_lines.len(), new_lines.len());
        let mut table = vec![vec![0u32; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                table[i][j] = if old_lines[i] == new_lines[j] {
                    table[i + 1][j + 1] + 1
                } else {
                    table[i + 1][j].max(table[i][j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && old_lines[i] == new_lines[j] {
                lines.push(line(ToolPreviewLineKind::Context, old_lines[i]));
                i += 1;
                j += 1;
            } else if j < m && (i == n || table[i][j + 1] >= table[i + 1][j]) {
                lines.push(line(ToolPreviewLineKind::Added, new_lines[j]));
                j += 1;
            } else {
                lines.push(line(ToolPreviewLineKind::Removed, old_lines[i]));
                i += 1;
            }
        }
    }
    let added = lines
        .iter()
        .filter(|line| line.kind == ToolPreviewLineKind::Added)
        .count();
    let removed = lines
        .iter()
        .filter(|line| line.kind == ToolPreviewLineKind::Removed)
        .count();
    ToolPreview::FileEdit {
        path: path.to_owned(),
        lines,
        added,
        removed,
        truncated: false,
    }
}

/// The idempotency key for one change, from the tool call's ID when the call has one, so a
/// retried call is applied at most once; a fresh one otherwise. The ID is the agent loop's for
/// its own calls, and the child's own for a call a coding agent made over the tool bridge
/// (`crew_extension::with_bridged_call_id`, D3), which the child sends again when it retries.
///
/// The ID is not ours (a vendor's CLI names it), so the key also carries a digest of what the
/// call asks for: an ID sent again for a different change never matches the first. The ID's
/// part is cut to [`KEY_ID_CHARS`], so the step that ends the key is never cut off.
pub(crate) fn idempotency_key(ctx: &Ctx, step: usize) -> String {
    let Some(id) = ctx.call.tool_call_id.as_deref() else {
        return format!("chat-{}-{step}", uuid::Uuid::new_v4());
    };
    let id: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
        .take(KEY_ID_CHARS)
        .collect();
    format!("chat-{id}-{}-{step}", request_digest(&ctx.call))
}

/// The most of a call's ID an idempotency key keeps.
const KEY_ID_CHARS: usize = 64;

/// 16 hex digits of SHA-256 over the call's tool, action and params (object keys sorted).
fn request_digest(call: &PersonToolCall) -> String {
    use sha2::Digest as _;
    let asked = serde_json::json!([call.tool.name(), call.action, sorted_keys(&call.params)]);
    let digest = sha2::Sha256::digest(asked.to_string().as_bytes());
    hex::encode(&digest[..8])
}

/// `value` with every object's keys in order, so the same request always reads the same.
fn sorted_keys(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.clone(), sorted_keys(value)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted_keys).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests;

/// What the action files' own tests share: a call as the host makes one, without a daemon.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use biorouter::crew::tool_host::PersonTool;
    use biorouter::pending_user_action::PendingUserActions;
    use biorouter::privacy::CallCapability;

    /// A call of `tool`'s `action` with `params`, in a chat of its own, with its own approval
    /// registry and a fresh turn token. `params` is not checked, so a test can drive an action
    /// past what `check_params` would refuse.
    pub(crate) async fn test_ctx(tool: PersonTool, action: &str, params: Value) -> Ctx {
        let action = catalog::find(tool, action).expect("a listed action");
        Ctx {
            call: PersonToolCall {
                tool,
                action: action.name.to_owned(),
                params,
                session_id: format!("crew-tools-test-{}", uuid::Uuid::new_v4()),
                tool_call_id: None,
                capability: CallCapability::public_enforced(),
                cancel: CancellationToken::new(),
                approvals: Arc::new(PendingUserActions::default()),
            },
            state: AppState::new().await.expect("the app state"),
            action,
        }
    }
}

#[cfg(test)]
mod step_tests {
    use super::test_support::test_ctx;
    use super::*;
    use biorouter::conversation::message::CrewVaultAction;
    use biorouter::crew::tool_host::PersonTool;
    use biorouter::pending_user_action::{DecisionAuthority, ResolveOutcome};
    use biorouter::permission::Permission;

    fn unlock() -> CrewStepRequest {
        CrewStepRequest::Vault {
            action: CrewVaultAction::Unlock,
        }
    }

    async fn ctx() -> Ctx {
        test_ctx(PersonTool::Change, "credentials_unlock", json!({})).await
    }

    /// A step card is parked in the calling chat, a forged success leaves it parked, and only
    /// the caller's withdrawal (or the person, Stop, or time) takes it down. Withdrawing it
    /// never stops the chat.
    #[tokio::test]
    async fn a_parked_step_ends_only_when_withdrawn_dismissed_stopped_or_late() {
        let ctx = ctx().await;
        let step = ctx.park_step(unlock(), Duration::from_secs(60)).unwrap();
        let id = step.id().to_owned();
        assert!(ctx.call.approvals.is_pending(&id));
        assert_eq!(
            ctx.call.approvals.resolve_in_session(
                &ctx.call.session_id,
                &id,
                UserActionOutcome::Approved {
                    permission: Permission::AllowOnce
                },
                DecisionAuthority::unproven(),
            ),
            ResolveOutcome::Rejected
        );
        step.withdrawer().cancel();
        assert_eq!(step.end().await, StepEnd::Withdrawn);
        assert!(!ctx.call.approvals.is_pending(&id));
        assert!(!ctx.call.cancel.is_cancelled());

        let step = ctx.park_step(unlock(), Duration::from_secs(60)).unwrap();
        let id = step.id().to_owned();
        assert_eq!(
            ctx.call
                .approvals
                .dismiss_crew_step(&ctx.call.session_id, &id),
            ResolveOutcome::Delivered
        );
        assert_eq!(step.end().await, StepEnd::Dismissed);

        let step = ctx.park_step(unlock(), Duration::from_millis(20)).unwrap();
        assert_eq!(step.end().await, StepEnd::TimedOut);

        let step = ctx.park_step(unlock(), Duration::from_secs(60)).unwrap();
        ctx.call.cancel.cancel();
        assert_eq!(step.end().await, StepEnd::Stopped);
    }

    /// Work that waits for the person on a card is not cut off when the call's wait runs out:
    /// only the time it spends on its own counts. Without the person, the wait runs out and the
    /// work keeps running; Stop ends the wait at once.
    #[tokio::test]
    async fn time_the_person_spends_on_a_card_is_not_counted() {
        let stop = CancellationToken::new();
        let progress = Progress::default();
        let watched = progress.clone();
        let mut signing_in = tokio::spawn(async move {
            let _person = watched.awaiting_person();
            tokio::time::sleep(Duration::from_millis(600)).await;
            "signed in"
        });
        let budget = Duration::from_millis(100);
        let finished = finished_within(&mut signing_in, &progress, &stop, budget).await;
        assert_eq!(finished.unwrap().unwrap(), "signed in");
        assert!(progress.person_time() >= Duration::from_millis(400));
        assert!(!progress.awaiting());

        let mut slow = tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(600)).await;
        });
        let alone = Progress::default();
        assert!(finished_within(&mut slow, &alone, &stop, budget)
            .await
            .is_none());
        assert!(!slow.is_finished(), "the work keeps running");

        stop.cancel();
        let started = Instant::now();
        assert!(
            finished_within(&mut slow, &alone, &stop, Duration::from_secs(60))
                .await
                .is_none()
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        slow.abort();
    }

    /// The approval a call was given is taken again only by the one host-key retry, only for the
    /// identical card, only once, and only within an approval card's time. Outside a call's
    /// scope nothing is remembered.
    #[tokio::test]
    async fn only_a_host_key_retry_of_the_same_card_takes_the_approval_again() {
        let now = Instant::now();
        let in_a_call =
            |test: fn(Instant)| CALL_APPROVAL.scope(RefCell::default(), async move { test(now) });

        // Not in a retry: the card is asked again, and the approval is kept for the retry.
        in_a_call(|now| {
            remember_allowed("card".into(), now);
            assert!(!allowed_for_this_retry("card", now));
            CALL_APPROVAL.with(|memo| assert!(memo.borrow().allowed.is_some()));
        })
        .await;
        // In the retry, the same card once; a second ask of it asks.
        CALL_APPROVAL
            .scope(RefCell::default(), async move {
                remember_allowed("card".into(), now);
                retrying_after_a_host_key(async {
                    assert!(allowed_for_this_retry("card", now));
                    assert!(!allowed_for_this_retry("card", now));
                })
                .await;
                CALL_APPROVAL.with(|memo| assert!(!memo.borrow().retrying));
            })
            .await;
        // Another card in the retry is asked, and spends the approval.
        CALL_APPROVAL
            .scope(RefCell::default(), async move {
                remember_allowed("card".into(), now);
                retrying_after_a_host_key(async {
                    assert!(!allowed_for_this_retry("another card", now));
                    assert!(!allowed_for_this_retry("card", now));
                })
                .await;
            })
            .await;
        // An approval older than an approval card's time is asked again.
        CALL_APPROVAL
            .scope(RefCell::default(), async move {
                remember_allowed("card".into(), now);
                retrying_after_a_host_key(async {
                    let late = now + APPROVAL_TTL + Duration::from_secs(1);
                    assert!(!allowed_for_this_retry("card", late));
                })
                .await;
            })
            .await;
        // Outside a call, nothing is remembered and nothing is taken.
        remember_allowed("card".into(), now);
        assert!(!retrying_after_a_host_key(async { allowed_for_this_retry("card", now) }).await);
    }

    /// A step parked for work apart from its call is refused once the call was stopped.
    #[tokio::test]
    async fn no_step_is_parked_for_a_stopped_call() {
        let ctx = ctx().await;
        ctx.call.cancel.cancel();
        assert!(park_step_in(
            &ctx.call.approvals,
            &ctx.call.session_id,
            &ctx.call.cancel,
            unlock(),
            Duration::from_secs(60)
        )
        .is_none());
        assert!(!ctx
            .call
            .approvals
            .has_pending_in_session(&ctx.call.session_id));
    }

    /// No card where no person could answer it: the call then says the step in words.
    #[tokio::test]
    async fn no_step_is_parked_where_no_person_can_answer() {
        let ctx = ctx().await;
        let parked = biorouter::user_surface::without_human_surface(async {
            ctx.park_step(unlock(), Duration::from_secs(60)).is_some()
        })
        .await;
        assert!(!parked);
    }
}

/// Owner ruling D3: a call a coding agent's child makes over the tool bridge answers before the
/// bridge stops waiting for it, and keys its changes on its own id.
#[cfg(test)]
mod bridged_call_tests {
    use super::test_support::test_ctx;
    use super::*;
    use biorouter::conversation::message::CrewVaultAction;
    use biorouter::crew::tool_host::PersonTool;
    use biorouter::providers::coding_agent::bridge::with_call_budget_for_test;

    fn unlock() -> CrewStepRequest {
        CrewStepRequest::Vault {
            action: CrewVaultAction::Unlock,
        }
    }

    async fn ctx() -> Ctx {
        test_ctx(PersonTool::Change, "credentials_unlock", json!({})).await
    }

    /// Run `f` as a call bridged with `budget` left runs: inside the bridge's budget, with the
    /// host's own deadline.
    async fn bridged<T>(budget: Duration, f: impl std::future::Future<Output = T>) -> T {
        with_call_budget_for_test(budget, within_the_bridge(f)).await
    }

    /// A step card in a bridged call waits only until the call's deadline, the bridge's budget
    /// less its margin, so it ends before the bridge's 570-second approval window does. Outside
    /// a bridged call it waits as asked, and with no time left no card is parked at all.
    #[tokio::test]
    async fn a_bridged_call_s_step_card_ends_before_the_bridge_does() {
        let ctx = ctx().await;
        let step = within_the_bridge(async { ctx.park_step(unlock(), APPROVAL_TTL) })
            .await
            .expect("a card outside the bridge");
        assert_eq!(step.ttl, APPROVAL_TTL);
        drop(step);

        let budget = biorouter::providers::coding_agent::bridge::child_tool_call_budget();
        let step = bridged(budget, async { ctx.park_step(unlock(), APPROVAL_TTL) })
            .await
            .expect("a card inside the bridge's budget");
        assert!(step.ttl <= Duration::from_secs(570), "{:?}", step.ttl);
        assert!(step.ttl > Duration::from_secs(560), "{:?}", step.ttl);
        drop(step);

        let parked = bridged(BRIDGE_MARGIN, async {
            ctx.park_step(unlock(), Duration::from_secs(60)).is_some()
        })
        .await;
        assert!(!parked, "no time left, so no card");
        assert!(!ctx
            .call
            .approvals
            .has_pending_in_session(&ctx.call.session_id));
    }

    /// Every card of one bridged call shares its deadline: an approval asked after another used
    /// the call's time is not parked, and the call answers in time instead of at 600 seconds.
    #[tokio::test]
    async fn a_bridged_call_s_approvals_share_one_deadline() {
        let ctx = ctx().await;
        let card = || {
            let arguments = Map::new();
            let preview = ToolPreview::arguments_within(&arguments, 64);
            ctx.park(
                ToolRisk::High,
                "Post hi in #general".into(),
                arguments,
                preview,
            )
        };
        let started = Instant::now();
        let (first, second) = bridged(BRIDGE_MARGIN + Duration::from_millis(300), async {
            (card().await, card().await)
        })
        .await;
        assert!(matches!(first, UserActionOutcome::TimedOut), "{first:?}");
        assert!(matches!(second, UserActionOutcome::TimedOut), "{second:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(!ctx
            .call
            .approvals
            .has_pending_in_session(&ctx.call.session_id));
    }

    /// Work a bridged call runs apart from it carries the call's deadline, so a card that work
    /// parks ends in time, and the call answers by its deadline even while the person is still
    /// at that card. A call that is not bridged has no deadline there either.
    #[tokio::test]
    async fn work_a_bridged_call_runs_apart_carries_its_deadline() {
        let ctx = ctx().await;
        let carried = |progress| {
            ctx.follow(progress, async {
                Ok(json!({"deadline": call_deadline().is_some()}))
            })
        };
        let inside = bridged(Duration::from_secs(600), carried(Progress::default()))
            .await
            .unwrap();
        assert_eq!(inside["deadline"], true);
        let outside = within_the_bridge(carried(Progress::default()))
            .await
            .unwrap();
        assert_eq!(outside["deadline"], false);

        let progress = Progress::default();
        let watched = progress.clone();
        let started = Instant::now();
        let answer = bridged(BRIDGE_MARGIN + Duration::from_millis(300), async {
            ctx.follow(progress, async move {
                let _person = watched.awaiting_person();
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(json!({"signed_in": true}))
            })
            .await
        })
        .await
        .unwrap();
        assert_eq!(answer["state"], "running", "{answer}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// A job wait in a bridged call ends by the call's deadline, whatever it was extended by.
    #[tokio::test]
    async fn a_job_wait_ends_by_the_call_s_deadline() {
        let later = Instant::now() + Duration::from_secs(3600);
        assert_eq!(
            within_the_bridge(async { no_later_than_the_call(later) }).await,
            later
        );
        let (deadline, waited) = bridged(Duration::from_secs(600), async {
            (call_deadline().unwrap(), within_call(JOB_WAIT))
        })
        .await;
        assert!(deadline < later);
        assert_eq!(waited, JOB_WAIT);
        let cut = bridged(BRIDGE_MARGIN + Duration::from_secs(5), async {
            (no_later_than_the_call(later), within_call(JOB_WAIT))
        })
        .await;
        assert!(cut.0 < Instant::now() + Duration::from_secs(6));
        assert!(cut.1 <= Duration::from_secs(5), "{:?}", cut.1);
    }

    /// A change's idempotency key is its call's own id, which for a bridged call is the child's:
    /// the child's retry of the same call carries the same key, and each change of one call its
    /// own. A call with no id gets a fresh key every time.
    #[tokio::test]
    async fn the_idempotency_key_is_the_call_s_own_id() {
        let mut ctx = ctx().await;
        ctx.call.tool_call_id = Some("toolu_01AbC".into());
        let key = idempotency_key(&ctx, 0);
        assert!(key.starts_with("chat-toolu_01AbC-"), "{key}");
        assert!(key.ends_with("-0"), "{key}");
        assert_eq!(idempotency_key(&ctx, 0), key);
        assert_ne!(idempotency_key(&ctx, 0), idempotency_key(&ctx, 1));
        ctx.call.tool_call_id = None;
        assert_ne!(idempotency_key(&ctx, 0), idempotency_key(&ctx, 0));
    }

    /// The child names the ID, so the same ID sent with a different change is another key, and
    /// the same change with its params in another order is the same key. A long ID never cuts
    /// off the step, so two steps of one call never share a key.
    #[tokio::test]
    async fn an_id_reused_for_another_change_is_another_key() {
        let mut ctx = ctx().await;
        ctx.call.tool_call_id = Some("toolu_01AbC".into());
        ctx.call.params = json!({"channel": "#general", "text": "hi"});
        let first = idempotency_key(&ctx, 0);
        ctx.call.params = json!({"text": "hi", "channel": "#general"});
        assert_eq!(idempotency_key(&ctx, 0), first, "the same change");
        ctx.call.params = json!({"channel": "#general", "text": "bye"});
        assert_ne!(idempotency_key(&ctx, 0), first, "other params");
        ctx.call.params = json!({"channel": "#general", "text": "hi"});
        ctx.call.action = "something_else".into();
        assert_ne!(idempotency_key(&ctx, 0), first, "another action");

        ctx.call.tool_call_id = Some("x".repeat(400));
        let (zero, one) = (idempotency_key(&ctx, 0), idempotency_key(&ctx, 1));
        assert_ne!(zero, one);
        assert!(zero.len() <= 128 && one.ends_with("-1"), "{one}");
    }

    /// A bridged call's read takes the bridge's capability, Public for both coding agents, and a
    /// public model never reads a Private workspace: refused, naming nothing from it.
    #[tokio::test]
    async fn a_public_bridged_read_of_a_private_workspace_is_withheld() {
        let ctx = test_ctx(PersonTool::Read, "channels_list", json!({})).await;
        assert_eq!(ctx.call.capability.tier(), ProviderTier::Public);
        let connection: Connection = serde_json::from_value(json!({
            "id": "lab-id", "name": "lab", "ssh_target": "wanjun@leo", "port": null,
            "identity_file": null, "proxy_jump": null, "socket_path": "/tmp/crew.sock",
            "owner_uid": 1000, "workspace_id": "lab-ws", "workspace_public_key": "00",
            "cluster_connection_id": "cluster", "mode": "private", "policy_epoch": 1,
            "status": "connected", "last_error": null, "device_id": "11", "public_key": "22",
        }))
        .expect("a saved connection");
        let snapshot = json!({
            "workspace": {"name": "secret-lab", "mode": "private", "institution_id": "ucsf"},
            "protected_channel_ids": [],
            "channels": [{"id": "c1", "name": "methods"}],
        });
        let refused = bridged(
            Duration::from_secs(600),
            ctx.content_scope(&connection, &snapshot),
        )
        .await
        .expect_err("a public model reads no Private workspace");
        let said = refused.downcast_ref::<Said>().expect("said in a sentence");
        assert_eq!(said.code, Some(crew_refusal::PUBLIC_MODEL_REFUSED));
        assert_eq!(said.sentence, PUBLIC_MODEL_READ);
        assert!(!said.sentence.contains("secret-lab"));
    }
}
