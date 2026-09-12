//! The permission decision for every tool call a Code Execution script makes
//! (QA finding F7, 2026-09-10).
//!
//! # The defect
//!
//! In Code Execution mode — the shipped default — the model's directly callable
//! roster collapses to `code_execution__*` and a handful of exemptions
//! (`reply_parts::survives_code_execution_filter`); every other tool is reached
//! by writing JavaScript inside `execute_code`. The permission system judged the
//! call the agent loop DISPATCHED, which was `execute_code`, and nothing below
//! it: the script's own calls went from the JS sandbox straight to
//! `ExtensionManager::dispatch_tool_call`, where no [`ToolInspector`] runs. So
//! one approval of the script — or one `always_allow` entry for
//! `code_execution__execute_code` — covered every call it made, in every mode.
//! Measured in Manual mode: `echo` through `developer__shell` and a
//! `developer__analyze` that was on no allow list both ran without a card.
//!
//! # The rule
//!
//! **A call a script makes faces the decision it would face as a direct call.**
//! Concretely, for each call the sandbox hands over:
//!
//! 1. every inspector the agent loop runs, on the call's *evaluated* arguments,
//!    in the agent's own mode and on the capability the `execute_code` call was
//!    admitted on — managed policy, the security floor, sensitive operations,
//!    the memory / session-store / knowledge-delete gates, workspace mutation,
//!    the user's PreToolUse hooks (with their rewrites re-judged) and the
//!    permission inspector, whose `always_allow` / `never_allow` / scoped grants
//!    are keyed by the INNER tool's name;
//! 2. a denial comes back into the script as a tool error it can catch;
//! 3. an ask goes to the person on the same card a direct call gets — naming the
//!    inner tool and carrying its arguments — after the user's PermissionRequest
//!    hooks have had the chance to answer it, exactly as for a direct call; and
//!    "Always allow" / "Always deny" on that card are recorded under the inner
//!    tool's name.
//!
//! `code_execution__execute_code` itself is judged exactly as before, so nothing
//! that asked before stops asking: an `always_allow` entry for it still means
//! "do not ask me before running a script". What it no longer means is "…and
//! allow everything the script calls".
//!
//! # What is deliberately different from a direct call
//!
//! * **No loop guard.** The repetition inspector watches the MODEL's call stream
//!   for a model repeating itself; a script's loop is the script doing its job,
//!   and the `execute_code` call that contains it already passed the guard.
//! * **No conversation history.** The one inspector that reads history is
//!   sensitive-ops' criterion-5 provenance, and it reads it only to EXEMPT a
//!   repository this session demonstrably created. Without history it exempts
//!   nothing, so a script's recursive delete of such a repository asks where the
//!   same direct call might not — the direction this gate may err in.
//! * **No approval delegation** (`approval_relay`). A direct call in an
//!   agent-created session may be answered by the agent that created it; a
//!   script's ask always goes to a person. Asking a person instead of an agent
//!   is never the weaker answer.
//!
//!   ⚠ That is about *who decides*, and it says nothing about *where the
//!   question appears* — which is where D10 went wrong. A card parks on the
//!   session that raised it, so a script's ask inside a **sub-agent** was
//!   published to the child's own conversation and nowhere else: a person
//!   watching the chat that delegated the work saw the `subagent` tool call stop
//!   with no card and no explanation, and a child with no tab at all
//!   (`visible: false`, a fan-out past the four-tab cap, a terminal session) had
//!   no surface anywhere. The same call made *directly* by the child's model
//!   already surfaces in the root chat — `approval_relay` publishes it there
//!   when it reports `AwaitingHuman` — so in the shipped Code Execution default,
//!   where nearly every call is a script's, the escalation was missing from the
//!   path that carries almost all the traffic.
//!   [`approval_relay::surface_where_a_person_is_watching`] closes that: the
//!   same card, in the root of the delegation tree, answerable from either
//!   surface, with no ancestor **agent** consulted and proof of user untouched.
//!
//! [`approval_relay::surface_where_a_person_is_watching`]:
//!     crate::agents::approval_relay::surface_where_a_person_is_watching
//! * **A hook's context is dropped, not injected.** A PreToolUse or
//!   PermissionRequest hook's `additionalContext` / `systemMessage` has no
//!   channel into a script that is still running, and left staged it would leak
//!   into a later turn as context about a call that finished long ago — the
//!   coding-agent bridge's reasoning, verbatim (`BridgeGrant::call`).
//!
//! # How the judge reaches the script
//!
//! [`Agent::dispatch_tool_call`] builds a [`ScriptCallGate`] for an
//! `execute_code` call and runs the TOOL BODY — the future it returns, not the
//! dispatch that builds it — inside [`judging_script_calls`]. `execute_code`
//! asks [`judge_for`] for it and hands it to the task that dispatches the
//! script's calls.
//!
//! ## An absent judge is two situations, and only one of them is benign
//!
//! A task-local is unreachable across a `tokio::spawn`, so "no gate on this
//! task" is evidence of nothing by itself. It is equally the shape of
//!
//! * a **person** running a script — `POST /agent/call_tool`, an Agent Drafter
//!   app, the coding-agent bridge (which judges with its own `BridgeGrant`).
//!   No agent loop dispatched it, every inspector was bypassed for the outer
//!   call too, and there is no model decision to gate: benign, and behaves
//!   exactly as it always has; and of
//! * the agent loop dispatching a script whose **scope did not survive** the
//!   trip down to the handler, which would run every call inside it *unjudged*.
//!
//! Collapsing those into one `None` made the whole control rest on the shape of
//! the call graph: a `tokio::spawn` inserted anywhere between
//! [`Agent::dispatch_tool_call`] and `handle_execute_code` would silently
//! disable it — no type error, no refusal, no log. So they are told apart by a
//! record a spawn cannot lose: [`DispatchedByAgentLoop`], a process-global count
//! of the sessions the agent loop currently has an `execute_code` body in flight
//! for, taken by [`judging_script_calls`] itself and released when that body
//! ends. Gate absent **and** that record present is
//! [`ScriptJudging::JudgeLost`], and it refuses the whole script.
//!
//! It errs the safe way round. Its one false positive is a *person* dispatching
//! a script through one of the ungated doors for a session whose own turn is
//! already inside one — and there the answer is a loud refusal (a tool error and
//! a `tracing::error!`), never a silent grant.
//!
//! [`ToolInspector`]: crate::tool_inspection::ToolInspector
//! [`Agent::dispatch_tool_call`]: crate::agents::Agent::dispatch_tool_call

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::Duration;

use rmcp::model::{CallToolRequestParams, JsonObject};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::config::permission::PermissionLevel;
use crate::config::BioRouterMode;
use crate::conversation::message::ToolRequest;
use crate::conversation::tool_preview::ToolPreview;
use crate::hooks::{HookDecision, HookEvent, HookPayload, HooksManager};
use crate::pending_user_action::{
    PendingUserAction, PendingUserActions, ToolApprovalRequest, UserActionOutcome,
    UserActionRequest,
};
use crate::permission::tool_risk::ToolRiskRegistry;
use crate::permission::Permission;
use crate::privacy::CallCapability;
use crate::session::Session;
use crate::tool_inspection::{InspectionResult, ToolInspectionManager};

use super::tool_execution::{
    denied_response_text, CANCELLED_RESPONSE, DECLINED_RESPONSE, EXPIRED_RESPONSE,
};

tokio::task_local! {
    /// The judge for the calls a script makes, installed around the body of the
    /// `execute_code` call that runs it. See the module header.
    static SCRIPT_CALL_GATE: Arc<ScriptCallGate>;
}

/// The sessions the agent loop currently has an `execute_code` tool body in
/// flight for, and how many (one turn may dispatch several scripts at once).
///
/// The durable half of [`judge_for`]: a `tokio::spawn` loses the task-local
/// above, and cannot touch this. Keyed by session because that is the one thing
/// `handle_execute_code` is handed that identifies the dispatch — see the module
/// header for why a false positive here is a refusal rather than a grant.
static AGENT_LOOP_SCRIPTS: LazyLock<Mutex<HashMap<String, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The agent loop's record that it is running a script for one session with a
/// judge installed. Held for exactly the life of the tool body, so a dropped
/// (cancelled) body releases it too.
pub(crate) struct DispatchedByAgentLoop {
    session_id: String,
}

impl DispatchedByAgentLoop {
    pub(crate) fn record(session_id: &str) -> Self {
        *AGENT_LOOP_SCRIPTS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(session_id.to_string())
            .or_default() += 1;
        Self {
            session_id: session_id.to_string(),
        }
    }
}

impl Drop for DispatchedByAgentLoop {
    fn drop(&mut self) {
        let mut scripts = AGENT_LOOP_SCRIPTS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // Remove the key at zero rather than leaving a `0` behind: the map is
        // process-global in a daemon that outlives every session in it.
        if let Some(count) = scripts.get_mut(&self.session_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                scripts.remove(&self.session_id);
            }
        }
    }
}

fn agent_loop_is_running_a_script(session_id: &str) -> bool {
    AGENT_LOOP_SCRIPTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains_key(session_id)
}

/// Run `tool_body` with `gate` judging any call a script makes inside it.
///
/// ⚠ Wrap the tool's BODY. `dispatch_tool_call` returns a future, and a scope
/// around the dispatch alone is gone before the script runs — the shape of the
/// #160 hang, and of every scope bug in that family.
pub(crate) async fn judging_script_calls<F: std::future::Future>(
    gate: Arc<ScriptCallGate>,
    tool_body: F,
) -> F::Output {
    // Taken HERE rather than in the agent loop, so the record and the scope it
    // vouches for are created and released by the same expression and can never
    // be wired up one without the other.
    let _dispatched = DispatchedByAgentLoop::record(&gate.session.id);
    SCRIPT_CALL_GATE.scope(gate, tool_body).await
}

/// No [`ScriptCallGate`] is installed on the task that asked.
///
/// A type of its own rather than a `None`, because on its own this is not a
/// decision — [`judge_for`] is what decides what it means.
#[derive(Debug)]
struct NoGateOnThisTask;

/// The gate installed around the tool body running on this task, if any.
fn current() -> Result<Arc<ScriptCallGate>, NoGateOnThisTask> {
    SCRIPT_CALL_GATE
        .try_with(Arc::clone)
        .map_err(|_| NoGateOnThisTask)
}

/// Who, if anyone, judges the calls the script about to run makes.
///
/// No `Debug`: [`ScriptCallGate`] has none, and it holds the inspector stack,
/// the session and the hooks manager — none of which belongs in a log line.
pub(crate) enum ScriptJudging {
    /// The agent loop dispatched this script and its judge is right here.
    By(Arc<ScriptCallGate>),
    /// Nothing in the agent loop dispatched it: a person did, through a door
    /// that bypasses every inspector for the outer call too. Unchanged
    /// behaviour — see the module header.
    PersonDriven,
    /// The agent loop IS running a script for this session, and this task
    /// cannot see its judge. Refuse: running on would put every call the script
    /// makes past the permission system.
    JudgeLost,
}

/// Which of the three situations in the module header this `execute_code` call
/// is in. The ONE place an absent gate is given a meaning.
pub(crate) fn judge_for(session_id: &str) -> ScriptJudging {
    match current() {
        Ok(gate) => ScriptJudging::By(gate),
        Err(NoGateOnThisTask) if agent_loop_is_running_a_script(session_id) => {
            ScriptJudging::JudgeLost
        }
        Err(NoGateOnThisTask) => ScriptJudging::PersonDriven,
    }
}

/// What a script whose judge did not reach it is answered with. Deliberately
/// says it is a defect: there is no user action that fixes it, and a sentence
/// that reads like a permission refusal would send them looking for a setting.
pub(crate) const JUDGE_LOST_REFUSAL: &str =
    "This script was not run. Biorouter dispatched it but the permission judge for the tool \
     calls it would make did not reach it, so those calls could not be put to you — and running \
     them unjudged is not an option. This is a defect in Biorouter, not something you can allow: \
     please report it.";

/// Inspectors that do not judge a script's calls. See the module header.
const NOT_FOR_SCRIPT_CALLS: &[&str] = &[crate::tool_monitor::REPETITION_INSPECTOR_NAME];

/// …plus the hook inspector, when a PreToolUse rewrite is being re-judged:
/// re-running it would execute the user's hook commands a second time and let a
/// rewrite trigger another rewrite (BR-19, as on the agent's own path).
const NOT_FOR_A_REWRITE: &[&str] = &[
    crate::tool_monitor::REPETITION_INSPECTOR_NAME,
    crate::hooks::inspector::HOOK_INSPECTOR_NAME,
];

/// What the script's call gets.
#[derive(Debug)]
pub(crate) enum ScriptCallVerdict {
    /// Dispatch exactly this call. It may differ from what the script asked
    /// for: a PreToolUse hook may have rewritten its arguments, and what runs is
    /// what was judged.
    Run(CallToolRequestParams),
    /// Do not dispatch it.
    Refuse(ScriptCallRefusal),
}

/// A call the gate did not let through.
#[derive(Debug)]
pub(crate) struct ScriptCallRefusal {
    /// The telemetry label on the script's executed-calls record.
    pub(crate) kind: &'static str,
    /// What the script (and, if the script does not catch it, the model) is
    /// told. The same sentence a direct call would get.
    pub(crate) message: String,
    /// What the user's executed-calls view says, in their terms. Never the
    /// arguments and never a hook's free-form text.
    pub(crate) user_note: &'static str,
}

impl ScriptCallRefusal {
    fn new(kind: &'static str, message: impl Into<String>, user_note: &'static str) -> Self {
        Self {
            kind,
            message: message.into(),
            user_note,
        }
    }
}

/// Everything one `execute_code` call's judge needs, taken from the agent that
/// dispatched it.
///
/// A snapshot rather than a handle back to the `Agent`, for the reason the
/// coding-agent bridge's grant is one: the script's calls are dispatched from a
/// task the agent does not own, and a judge that outlived its call would be an
/// authority with no owner.
pub struct ScriptCallGate {
    /// The agent's own inspector stack — the same `Arc` its direct calls use,
    /// so a user's "Always allow" recorded here is the one they will see there.
    inspections: Arc<ToolInspectionManager>,
    /// The agent's mode. `Agent::config.biorouter_mode` is the value the reply
    /// loop hands its own inspectors, so a script's calls and the loop's cannot
    /// be judged in two different modes.
    mode: BioRouterMode,
    /// The session whose turn dispatched the script: its working directory for
    /// path arguments, and the home of every ask this judge raises.
    session: Session,
    /// The store the delegation chain above [`Self::session`] is walked in, so a
    /// card raised inside a sub-agent can also be shown where a person is
    /// watching (D10).
    ///
    /// `None` only where there is no store to walk — a hand-built test gate. A
    /// missing store degrades to today's behaviour (the child's own tab, and
    /// nothing else) and says so in a log, rather than failing the call: a
    /// script's ask that cannot be *escalated* is still an ask a person may
    /// answer in the child's tab.
    sessions: Option<Arc<crate::session::SessionManager>>,
    /// For the PreToolUse rewrites this gate's own inspection staged, and the
    /// PermissionRequest hooks consulted before a card.
    hooks: Arc<HooksManager>,
    /// The global tool-dispatch concurrency permit the `execute_code` call this
    /// judge belongs to is holding, handed back for the duration of a parked ask
    /// (#246 review, finding 2). `None` where there is none to hand back: a test
    /// gate, or a dispatch the semaphore exempts.
    ///
    /// Set after construction because the permit is acquired *inside* the tool
    /// body, below the point where the agent still has the pieces this gate is
    /// built from — see `Agent::dispatch_tool_call`.
    parking_permit: Mutex<Option<crate::agents::tool_dispatch_limits::DispatchPermitHandle>>,
}

impl ScriptCallGate {
    pub(crate) fn new(
        inspections: Arc<ToolInspectionManager>,
        mode: BioRouterMode,
        session: Session,
        hooks: Arc<HooksManager>,
        sessions: Option<Arc<crate::session::SessionManager>>,
    ) -> Self {
        Self {
            inspections,
            mode,
            session,
            hooks,
            sessions,
            parking_permit: Mutex::new(None),
        }
    }

    /// Hand this judge the dispatch permit its `execute_code` call holds, so an
    /// ask parked on a person does not hold one of the eight the whole daemon
    /// shares. Called once, before any judging.
    pub(crate) fn hold_dispatch_permit(
        &self,
        handle: Option<crate::agents::tool_dispatch_limits::DispatchPermitHandle>,
    ) {
        *self
            .parking_permit
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = handle;
    }

    /// The handle, cloned out — never read across an `await`, because the lock is
    /// a `std::sync::Mutex`.
    fn parked_permit_handle(
        &self,
    ) -> Option<crate::agents::tool_dispatch_limits::DispatchPermitHandle> {
        self.parking_permit
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Decide one call a script made.
    ///
    /// `capability` is the one the `execute_code` call was admitted on, threaded
    /// down rather than sampled (issue #56). `risks` grades the script's own
    /// catalogue. `cancel` is the turn's token: Stop releases a parked ask.
    pub(crate) async fn judge(
        &self,
        call: CallToolRequestParams,
        capability: CallCapability,
        risks: &ToolRiskRegistry,
        cancel: &CancellationToken,
    ) -> ScriptCallVerdict {
        // Minted here so every exit below — refusals included — goes through the
        // drain, because every exit ran the user's hooks.
        let request_id = format!("script_call_{}", uuid::Uuid::new_v4());
        let verdict = self
            .judge_one(request_id.clone(), call, capability, risks, cancel)
            .await;
        self.discard_staged_hook_context(&request_id);
        verdict
    }

    async fn judge_one(
        &self,
        request_id: String,
        call: CallToolRequestParams,
        capability: CallCapability,
        risks: &ToolRiskRegistry,
        cancel: &CancellationToken,
    ) -> ScriptCallVerdict {
        let name = call.name.to_string();
        let mut requests = vec![ToolRequest {
            id: request_id,
            tool_call: Ok(call),
            metadata: None,
            tool_meta: None,
        }];

        let mut inspections = match self
            .inspections
            .inspect_script_calls(
                NOT_FOR_SCRIPT_CALLS,
                &requests,
                &[],
                self.mode,
                &self.session,
                capability,
                risks,
            )
            .await
        {
            Ok(inspections) => inspections,
            Err(error) => return unjudged(&name, &error.to_string()),
        };
        if let Err(error) = self
            .collect_hook_rewrites(&mut requests, &mut inspections, capability, risks)
            .await
        {
            return unjudged(&name, &error.to_string());
        }

        // No permission decision must never read as approval.
        let Some(decision) = self
            .inspections
            .process_inspection_results_with_permission_inspector(&requests, &inspections)
        else {
            return unjudged(&name, "no permission decision was reached");
        };

        if let Some(denied) = decision.denied.first() {
            return ScriptCallVerdict::Refuse(ScriptCallRefusal::new(
                "permission_denied",
                denied_response_text(&denied.id, &inspections),
                "Not run: refused by your tool permissions",
            ));
        }
        // What runs is what was judged — taken out of the verdict, not out of
        // the script's request, so a hook rewrite cannot be undone here.
        if let Some(approved) = decision.approved.into_iter().next() {
            return match approved.tool_call {
                Ok(call) => ScriptCallVerdict::Run(call),
                Err(error) => unjudged(&name, &error.to_string()),
            };
        }
        let Some(pending) = decision.needs_approval.into_iter().next() else {
            return unjudged(&name, "the call was neither allowed, denied nor put to you");
        };
        self.ask_a_person(pending, &inspections, risks, cancel)
            .await
    }

    /// BR-19, on the script's path: apply what the user's PreToolUse hooks asked
    /// to rewrite, and judge the rewritten call again.
    ///
    /// Scoped to this call's own request id, never the session's whole buffer:
    /// a turn can run several scripts at once, and a session-wide take would
    /// steal a sibling's rewrite (the bridge learned this — see
    /// `BridgeGrant::collect_hook_rewrites`).
    async fn collect_hook_rewrites(
        &self,
        requests: &mut [ToolRequest],
        inspections: &mut Vec<InspectionResult>,
        capability: CallCapability,
        risks: &ToolRiskRegistry,
    ) -> anyhow::Result<()> {
        let ids: Vec<String> = requests.iter().map(|request| request.id.clone()).collect();
        let rewrites = self
            .hooks
            .take_tool_input_rewrites_for(&self.session.id, &ids);
        if rewrites.is_empty() || crate::hooks::apply_tool_input_rewrites(requests, &rewrites) == 0
        {
            return Ok(());
        }
        let mut revalidated = self
            .inspections
            .inspect_script_calls(
                NOT_FOR_A_REWRITE,
                requests,
                &[],
                self.mode,
                &self.session,
                capability,
                risks,
            )
            .await?;
        inspections
            .retain(|result| result.inspector_name == crate::hooks::inspector::HOOK_INSPECTOR_NAME);
        inspections.append(&mut revalidated);
        Ok(())
    }

    /// Put an ask to the person, the way the agent loop puts a direct call's.
    async fn ask_a_person(
        &self,
        pending: ToolRequest,
        inspections: &[InspectionResult],
        risks: &ToolRiskRegistry,
        cancel: &CancellationToken,
    ) -> ScriptCallVerdict {
        let call = match pending.tool_call {
            Ok(call) => call,
            Err(error) => return unjudged("the call", &error.to_string()),
        };
        let name = call.name.to_string();
        let arguments: JsonObject = call.arguments.clone().unwrap_or_default();

        if let Some(answered) = self
            .answered_by_permission_request_hooks(&pending.id, &call, &arguments, inspections)
            .await
        {
            return answered;
        }

        // An approval is an authorization, so it needs the exact session that
        // will display it and accept the answer — never an unscoped queue
        // another session could claim (the bridge's rule, #40).
        if self.session.id.is_empty() {
            return ScriptCallVerdict::Refuse(ScriptCallRefusal::new(
                "approval_unavailable",
                format!(
                    "`{name}` needs your approval, and this script is not running in a \
                     conversation that can show an approval card. It was not run."
                ),
                "Not run: no conversation to ask in",
            ));
        }
        self.notify_permission_prompt(&name);

        let request = UserActionRequest::ToolApproval(ToolApprovalRequest {
            tool_name: name.clone(),
            arguments: arguments.clone(),
            // Exactly what a direct call's card carries: the inspectors' reasons,
            // and nothing when none of them explained anything. ⚠ Do not add a
            // "this came from a script" line here. The desktop reads ANY prompt
            // as a security finding — it draws it as a warning banner and
            // withholds "Always allow" (`ToolCallConfirmation.tsx`) — so a
            // provenance note turned every ordinary script ask into an alarm the
            // user could only answer once per call, which was measured in the
            // running app. The transcript's step row already shows the script.
            prompt: crate::tool_inspection::approval_prompt_for_request(&pending.id, inspections),
            risk: Some(risks.risk_for(&name)),
            preview: ToolPreview::for_tool_call(&name, &arguments),
            // The same answer a direct call's card takes: any surface of this
            // session may give it. Nothing about a script's call makes it an
            // authorization a model must be proven unable to grant.
            requires_user_proof: false,
        });
        let parked = PendingUserActions::global().park(Some(&self.session.id), None, request);
        // D10: a card raised inside a delegated child is also shown to the person
        // watching the conversation that delegated the work. Before the wait, so
        // a user already looking at that chat cannot answer a card that has not
        // been recorded as answerable there yet.
        self.escalate_to_a_watching_conversation(&parked).await;
        // #246 review, finding 2. This wait is up to `approval_ttl()` long
        // (default 3600 s; `Duration::MAX` when `BIOROUTER_CONFIRMATION_TIMEOUT_SECS=0`),
        // and it happens INSIDE the `execute_code` tool body, which holds one of
        // the eight tool-dispatch permits the whole daemon shares. Before F7
        // `execute_code` could not park at all, so eight scripts parked on cards
        // would now stall every other tool call in the process — the user's own
        // foreground conversation included. Hand the permit back while we wait;
        // the script queues for it again before it resumes doing work.
        let wait = parked.wait(approval_ttl(), Some(cancel));
        let outcome = match self.parked_permit_handle() {
            Some(permit) => permit.while_parked(wait).await,
            None => wait.await,
        };
        self.verdict_for_answer(call, outcome, cancel).await
    }

    /// **D10.** Show a delegated child's card in the conversation a person is
    /// actually watching, as well as in the child's own tab.
    ///
    /// The policy — which conversation, and why the root rather than the
    /// immediate parent — lives in
    /// [`crate::agents::approval_relay::surface_where_a_person_is_watching`],
    /// beside the direct-call escalation it mirrors, so the two cannot pick
    /// different destinations. All this adds is the store to walk and the log
    /// line.
    ///
    /// Failure is silent by design: no ancestor, or no store to walk, leaves the
    /// ask exactly where it was before this existed — in the child's own tab,
    /// which a person may still answer.
    async fn escalate_to_a_watching_conversation(&self, parked: &PendingUserAction) {
        if self.session.parent_session_id.is_none() {
            return;
        }
        let Some(sessions) = self.sessions.as_deref() else {
            tracing::warn!(
                session_id = %self.session.id,
                "a script's approval card inside a delegated conversation could not be \
                 escalated: this judge holds no session store, so only that conversation's \
                 own tab can answer it"
            );
            return;
        };
        if let Some(watching) = crate::agents::approval_relay::surface_where_a_person_is_watching(
            sessions,
            &self.session,
            parked,
        )
        .await
        {
            tracing::debug!(
                child_session_id = %self.session.id,
                watching_session_id = %watching,
                request_id = parked.id(),
                "surfaced a delegated script's approval card in the conversation that \
                 delegated the work"
            );
        }
    }

    /// The user's PermissionRequest hooks answer before any card, as they do for
    /// a direct call (`handle_approval_tool_requests`) — except where a security
    /// inspector raised the ask, which only a person may answer. `None` means
    /// the ask still goes to the person.
    async fn answered_by_permission_request_hooks(
        &self,
        request_id: &str,
        call: &CallToolRequestParams,
        arguments: &JsonObject,
        inspections: &[InspectionResult],
    ) -> Option<ScriptCallVerdict> {
        let hook = self
            .hooks
            .permission_request(
                &self.session.id,
                &self.session.working_dir,
                &call.name,
                &Value::Object(arguments.clone()),
            )
            .await;
        let requires_a_human =
            crate::tool_inspection::approval_requires_a_human(request_id, inspections);
        match hook.decision {
            Some(HookDecision::Allow { .. }) if requires_a_human => {
                tracing::warn!(
                    counter.biorouter.non_delegable_approval_hook_ignored = 1,
                    tool_name = %call.name,
                    "PermissionRequest hook tried to auto-approve a security-raised approval \
                     inside a script; asking the user instead"
                );
                None
            }
            Some(HookDecision::Allow { .. }) => Some(ScriptCallVerdict::Run(call.clone())),
            Some(HookDecision::Deny { reason }) => {
                Some(ScriptCallVerdict::Refuse(ScriptCallRefusal::new(
                    "hook_denied",
                    format!("{DECLINED_RESPONSE}\n\nHook feedback: {reason}"),
                    "Not run: refused by your PermissionRequest hook",
                )))
            }
            Some(HookDecision::Ask { .. }) | None => None,
        }
    }

    /// Tell the user's Notification hooks a permission prompt is waiting — the
    /// direct path does, and a desktop notification is how a user away from the
    /// window learns a script is parked on them.
    fn notify_permission_prompt(&self, name: &str) {
        let mut payload = HookPayload::new(
            HookEvent::Notification,
            &self.session.id,
            self.session.working_dir.to_string_lossy(),
        );
        payload.message = Some(format!("Permission required for {name}"));
        self.hooks.fire(
            HookEvent::Notification,
            Some("permission_prompt".to_string()),
            payload,
            self.session.working_dir.clone(),
        );
    }

    /// What the person's answer — or the lack of one — means for the call.
    async fn verdict_for_answer(
        &self,
        call: CallToolRequestParams,
        outcome: UserActionOutcome,
        cancel: &CancellationToken,
    ) -> ScriptCallVerdict {
        let name = call.name.to_string();
        match outcome {
            UserActionOutcome::Approved { .. } if cancel.is_cancelled() => {
                ScriptCallVerdict::Refuse(ScriptCallRefusal::new(
                    "cancelled",
                    CANCELLED_RESPONSE,
                    "Not run: the turn was stopped",
                ))
            }
            UserActionOutcome::Approved { permission } => {
                if permission == Permission::AlwaysAllow {
                    self.inspections
                        .update_permission_manager(&name, PermissionLevel::AlwaysAllow)
                        .await;
                }
                ScriptCallVerdict::Run(call)
            }
            UserActionOutcome::Denied { permission } => {
                if permission == Permission::AlwaysDeny {
                    self.inspections
                        .update_permission_manager(&name, PermissionLevel::NeverAllow)
                        .await;
                }
                ScriptCallVerdict::Refuse(ScriptCallRefusal::new(
                    "approval_declined",
                    DECLINED_RESPONSE,
                    "Not run: you declined it",
                ))
            }
            UserActionOutcome::TimedOut => ScriptCallVerdict::Refuse(ScriptCallRefusal::new(
                "approval_expired",
                EXPIRED_RESPONSE,
                "Not run: the approval expired",
            )),
            UserActionOutcome::Cancelled if cancel.is_cancelled() => {
                ScriptCallVerdict::Refuse(ScriptCallRefusal::new(
                    "cancelled",
                    CANCELLED_RESPONSE,
                    "Not run: the turn was stopped",
                ))
            }
            // `park` answers Cancelled at once where nobody could ever answer —
            // a scheduled run, say. Say so, rather than "cancelled", which reads
            // as though someone decided.
            UserActionOutcome::Cancelled if crate::user_surface::no_human_surface() => {
                ScriptCallVerdict::Refuse(ScriptCallRefusal::new(
                    "approval_unavailable",
                    format!(
                        "`{name}` needs a person's approval, and nobody can be asked in this \
                         run, so it was not run. Do not retry it here; say what you needed it \
                         for."
                    ),
                    "Not run: nobody could be asked",
                ))
            }
            UserActionOutcome::Cancelled => ScriptCallVerdict::Refuse(ScriptCallRefusal::new(
                "approval_dismissed",
                CANCELLED_RESPONSE,
                "Not run: the approval was dismissed",
            )),
            // `Provided` / `SecretsConfigured` cannot answer a tool approval —
            // `PendingUserActions` refuses them — so reaching here is `Failed`.
            other => ScriptCallVerdict::Refuse(ScriptCallRefusal::new(
                "approval_unavailable",
                format!(
                    "`{name}` needed your approval, and the request {}. It was not run.",
                    other.refusal_detail()
                ),
                "Not run: the approval could not be shown",
            )),
        }
    }

    /// A hook's staged context for this call has nowhere to go. See the module
    /// header; logged at debug so "my hook's additionalContext never appeared"
    /// has an answer.
    fn discard_staged_hook_context(&self, request_id: &str) {
        let dropped = self
            .hooks
            .drain_tool_hook_context_for(&self.session.id, &[request_id.to_string()]);
        for staged in dropped {
            if staged.additional_context.is_empty() && staged.system_messages.is_empty() {
                continue;
            }
            tracing::debug!(
                tool = %staged.tool_name,
                context = staged.additional_context.len(),
                messages = staged.system_messages.len(),
                "a hook returned context for a tool call inside a Code Execution script; \
                 a running script has no channel to receive it, so it was dropped rather \
                 than left to leak into a later turn"
            );
        }
    }
}

/// How long a script's ask stays answerable: exactly as long as a direct
/// call's (`BIOROUTER_CONFIRMATION_TIMEOUT_SECS`, where `0` waits until the turn
/// ends). Nothing holds a socket open while it waits — only the script's own
/// thread, which the turn's cancellation releases.
fn approval_ttl() -> Duration {
    super::tool_execution::confirmation_timeout().unwrap_or(Duration::MAX)
}

/// A call the machinery could not judge. Refused: no decision is not a yes.
fn unjudged(name: &str, detail: &str) -> ScriptCallVerdict {
    tracing::warn!(tool = %name, detail, "a script's tool call could not be judged; refusing it");
    ScriptCallVerdict::Refuse(ScriptCallRefusal::new(
        "permission_unavailable",
        format!(
            "`{name}` was not run: Biorouter could not reach a permission decision for it \
             ({detail})."
        ),
        "Not run: no permission decision could be reached",
    ))
}

/// A gate built by hand, for tests that exercise it — or `execute_code`'s use
/// of it — without an agent. The agent-path tests below build none of this:
/// they go through `Agent::dispatch_tool_call`, which is the only way to prove
/// the judge reaches a script at all.
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Arc;

    use super::ScriptCallGate;
    use crate::config::permission::PermissionManager;
    use crate::config::BioRouterMode;
    use crate::hooks::HooksManager;
    use crate::session::session_manager::SessionType;
    use crate::session::SessionManager;
    use crate::tool_inspection::ToolInspectionManager;

    /// A user PreToolUse hook that rewrites every `developer__shell` call's
    /// command to `command` — the fixture shape the bridge's BR-19 tests use.
    pub(crate) fn hooks_rewriting_shell_to(command: &str) -> Arc<HooksManager> {
        let output = serde_json::json!({
            "hookSpecificOutput": { "updatedInput": { "command": command } }
        })
        .to_string();
        let hook = if cfg!(target_os = "windows") {
            // cmd.exe keeps the JSON's double quotes and would echo single ones.
            format!("echo {output}")
        } else {
            format!("echo '{}'", output.replace('\'', "'\"'\"'"))
        };
        let yaml = format!(
            "PreToolUse:\n  - matcher: \"developer__shell\"\n    hooks:\n      - type: command\n        command: {}\n",
            serde_json::to_string(&hook).expect("a json string"),
        );
        Arc::new(HooksManager::with_config(
            serde_yaml::from_str(&yaml).expect("the hook config parses"),
            false,
            Arc::new(tokio::sync::Mutex::new(None)),
        ))
    }

    /// A gate over the inspectors a verdict is read off — the permission
    /// inspector and the user's hooks, plus the security floor on request —
    /// with its own permission table in `dir`.
    pub(crate) async fn gate(
        dir: &std::path::Path,
        mode: BioRouterMode,
        hooks: Arc<HooksManager>,
        with_security: bool,
    ) -> (ScriptCallGate, Arc<PermissionManager>) {
        let permissions = Arc::new(PermissionManager::new(dir.join("config")));
        let mut inspections = ToolInspectionManager::new();
        if with_security {
            inspections.add_inspector(Box::new(
                crate::security::security_inspector::SecurityInspector::new(),
            ));
        }
        inspections.add_inspector(Box::new(
            crate::permission::permission_inspector::PermissionInspector::new(
                Arc::new(crate::permission::tool_risk::ToolRiskRegistry::new()),
                Arc::clone(&permissions),
                Arc::new(crate::managed::ManagedPolicy::empty()),
                Arc::new(tokio::sync::Mutex::new(None)),
            ),
        ));
        inspections.add_inspector(Box::new(crate::hooks::HookInspector::new(Arc::clone(
            &hooks,
        ))));
        let sessions = Arc::new(SessionManager::new(dir.join("sessions")));
        let session = sessions
            .create_session(dir.to_path_buf(), "gate".into(), SessionType::User)
            .await
            .expect("a session");
        (
            ScriptCallGate::new(Arc::new(inspections), mode, session, hooks, Some(sessions)),
            permissions,
        )
    }

    pub(crate) fn shell_call(command: &str) -> rmcp::model::CallToolRequestParams {
        rmcp::model::CallToolRequestParams {
            task: None,
            meta: None,
            name: "developer__shell".into(),
            arguments: Some(rmcp::object!({ "command": command })),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use rmcp::model::{CallToolRequestParams, CallToolResult, JsonObject, RawContent};
    use rmcp::object;
    use tokio_util::sync::CancellationToken;

    use crate::action_required_manager::ActionRequiredManager;
    use crate::agents::extension::ExtensionConfig;
    use crate::agents::{Agent, AgentConfig};
    use crate::config::permission::{PermissionLevel, PermissionManager};
    use crate::config::BioRouterMode;
    use crate::conversation::message::{ActionRequiredData, MessageContent};
    use crate::pending_user_action::DecisionAuthority;
    use crate::permission::permission_confirmation::PrincipalType;
    use crate::permission::{Permission, PermissionConfirmation};
    use crate::session::session_manager::SessionType;
    use crate::session::{Session, SessionManager};

    const EXECUTE_CODE: &str = "code_execution__execute_code";
    const SHELL: &str = "developer__shell";

    /// A real agent — its own inspector stack, its own permission table — with
    /// the two extensions the QA run used: `developer` and `code_execution`.
    ///
    /// The permission table is private to the fixture and pre-seeded exactly as
    /// the QA sandbox's was in the one entry that matters: the SCRIPT is always
    /// allowed. Everything the script calls must then stand on its own name.
    struct Fixture {
        agent: Arc<Agent>,
        session: Session,
        permissions: Arc<PermissionManager>,
        dir: tempfile::TempDir,
    }

    async fn fixture(mode: BioRouterMode) -> Fixture {
        fixture_with(mode, &[]).await
    }

    /// As [`fixture`], plus the named bundled Platform capabilities.
    async fn fixture_with(mode: BioRouterMode, platform: &[&str]) -> Fixture {
        let dir = tempfile::TempDir::new().expect("a scratch directory");
        let sessions = Arc::new(SessionManager::new(dir.path().join("sessions")));
        let permissions = Arc::new(PermissionManager::new(dir.path().join("config")));
        let agent = Arc::new(Agent::with_config(AgentConfig::new(
            Arc::clone(&sessions),
            Arc::clone(&permissions),
            None,
            mode,
        )));
        agent
            .add_extension(ExtensionConfig::Builtin {
                name: "developer".into(),
                description: "developer".into(),
                display_name: Some("Developer".into()),
                timeout: Some(300),
                bundled: Some(true),
                available_tools: vec![],
            })
            .await
            .expect("enable developer");
        agent
            .add_extension(ExtensionConfig::Platform {
                name: "code_execution".into(),
                description: "code execution".into(),
                bundled: Some(true),
                available_tools: vec![],
            })
            .await
            .expect("enable code_execution");
        for name in platform {
            agent
                .add_extension(ExtensionConfig::Platform {
                    name: (*name).into(),
                    description: (*name).into(),
                    bundled: Some(true),
                    available_tools: vec![],
                })
                .await
                .unwrap_or_else(|error| panic!("enable {name}: {error}"));
        }
        let session = sessions
            .create_session(
                dir.path().to_path_buf(),
                "script gate".into(),
                SessionType::User,
            )
            .await
            .expect("a session");
        permissions.update_user_permission(EXECUTE_CODE, PermissionLevel::AlwaysAllow);
        // A card left over from another test that minted the same session id
        // (they are `YYYYMMDD_N` per database) must not be read as ours.
        ActionRequiredManager::global().drain_requests(&session.id);
        Fixture {
            agent,
            session,
            permissions,
            dir,
        }
    }

    fn text_of(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|content| match &content.raw {
                RawContent::Text(text) => Some(text.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Dispatch `execute_code` through [`Agent::dispatch_tool_call`] — the one
    /// function every model-initiated call reaches, approved or not — and drive
    /// the script's body on its own task, the way the reply loop's batch does.
    ///
    /// ⚠ Deliberately NOT `ExtensionManager::dispatch_tool_call`: that is the
    /// path a PERSON drives (`POST /agent/call_tool`), and it is the agent's
    /// dispatch that has to carry the script's judge down to its calls.
    async fn run_script(
        f: &Fixture,
        code: &str,
        cancel: CancellationToken,
    ) -> tokio::task::JoinHandle<(bool, String)> {
        run_script_in(f, &f.session, code, cancel).await
    }

    /// As [`run_script`], but for a script the agent dispatches in `session` —
    /// a delegated child, say — rather than in the fixture's own chat.
    async fn run_script_in(
        f: &Fixture,
        session: &Session,
        code: &str,
        cancel: CancellationToken,
    ) -> tokio::task::JoinHandle<(bool, String)> {
        let dispatched = dispatch_script_in(f, session, code, cancel).await;
        tokio::spawn(async move {
            let result = dispatched
                .result
                .await
                .expect("execute_code returns a result");
            (result.is_error.unwrap_or(false), text_of(&result))
        })
    }

    async fn dispatch_script(
        f: &Fixture,
        code: &str,
        cancel: CancellationToken,
    ) -> crate::agents::tool_execution::ToolCallResult {
        dispatch_script_in(f, &f.session, code, cancel).await
    }

    async fn dispatch_script_in(
        f: &Fixture,
        session: &Session,
        code: &str,
        cancel: CancellationToken,
    ) -> crate::agents::tool_execution::ToolCallResult {
        let call = CallToolRequestParams {
            task: None,
            meta: None,
            name: EXECUTE_CODE.into(),
            arguments: Some(object!({ "code": code })),
        };
        let (_, dispatched) = f
            .agent
            .dispatch_tool_call(call, "outer-execute-code".into(), Some(cancel), session)
            .await;
        dispatched.expect("execute_code dispatches")
    }

    struct Card {
        id: String,
        tool_name: String,
        arguments: JsonObject,
        prompt: Option<String>,
    }

    /// The next approval card published for this session, or `None` if the
    /// script finished without one.
    ///
    /// Raced against the script itself rather than a bare timeout: a script
    /// that runs its call without asking finishes, and waiting out a clock
    /// after that proves nothing but patience.
    async fn card_or_completion(
        session_id: &str,
        script: &mut tokio::task::JoinHandle<(bool, String)>,
    ) -> Result<Card, (bool, String)> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            for message in ActionRequiredManager::global().drain_requests(session_id) {
                for content in &message.content {
                    let MessageContent::ActionRequired(action) = content else {
                        continue;
                    };
                    if let ActionRequiredData::ToolConfirmation {
                        id,
                        tool_name,
                        arguments,
                        prompt,
                        ..
                    } = &action.data
                    {
                        return Ok(Card {
                            id: id.clone(),
                            tool_name: tool_name.clone(),
                            arguments: arguments.clone(),
                            prompt: prompt.clone(),
                        });
                    }
                }
            }
            if script.is_finished() {
                return Err(script.await.expect("the script task completes"));
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "neither an approval card nor a finished script within 60s"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn answer(f: &Fixture, card: &Card, permission: Permission) {
        let outcome = answer_from(f, &f.session.id, card, permission).await;
        assert_eq!(
            outcome,
            crate::agents::ConfirmationOutcome::Delivered,
            "the card's decision must reach the parked call"
        );
    }

    /// Answer `card` the way `POST /action-required/tool-confirmation` does when
    /// the click happened in `from_session_id` — the one thing that distinguishes
    /// a decision made in the child's own tab from one made where the card was
    /// escalated to.
    async fn answer_from(
        f: &Fixture,
        from_session_id: &str,
        card: &Card,
        permission: Permission,
    ) -> crate::agents::ConfirmationOutcome {
        f.agent
            .handle_confirmation_for_session(
                from_session_id,
                card.id.clone(),
                PermissionConfirmation {
                    principal_type: PrincipalType::Tool,
                    permission,
                },
                DecisionAuthority::unproven(),
            )
            .await
    }

    /// A delegated child of the fixture's own chat: a `SubAgent` row whose
    /// `parent_session_id` names the conversation that spawned it, which is the
    /// shape `create_subagent_session` writes.
    ///
    /// Read back from the store rather than mutated in place, because
    /// `ScriptCallGate` snapshots the `Session` it is handed — a child whose
    /// parent is only in the database is a child the gate cannot see.
    async fn delegated_child(f: &Fixture) -> Session {
        let sessions = &f.agent.config.session_manager;
        let child = sessions
            .create_session(
                f.dir.path().to_path_buf(),
                "delegated".into(),
                SessionType::SubAgent,
            )
            .await
            .expect("a child session");
        sessions
            .update(&child.id)
            .parent_session_id(Some(f.session.id.clone()))
            .apply()
            .await
            .expect("the child records its parent");
        ActionRequiredManager::global().drain_requests(&child.id);
        let child = sessions
            .get_session(&child.id, false)
            .await
            .expect("the child reads back");
        assert_eq!(
            child.parent_session_id.as_deref(),
            Some(f.session.id.as_str()),
            "the fixture only discriminates if the child really is delegated"
        );
        child
    }

    /// The next approval card to reach `watcher`, the bus feed `POST /reply` and
    /// `GET /sessions/{id}/events` both drain — i.e. what a person watching that
    /// conversation sees.
    async fn card_on_bus(
        watcher: &mut crate::session_events::Subscription,
        within: Duration,
    ) -> Option<Card> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let remaining = deadline.checked_duration_since(tokio::time::Instant::now())?;
            let Ok(Ok(event)) = tokio::time::timeout(remaining, watcher.recv()).await else {
                return None;
            };
            let crate::session_events::SessionBusEvent::Agent(crate::agents::AgentEvent::Message(
                message,
            )) = event
            else {
                continue;
            };
            for content in &message.content {
                let MessageContent::ActionRequired(action) = content else {
                    continue;
                };
                if let ActionRequiredData::ToolConfirmation {
                    id,
                    tool_name,
                    arguments,
                    prompt,
                    ..
                } = &action.data
                {
                    assert!(
                        !message.is_agent_visible(),
                        "an escalated card must stay out of the watching agent's context: \
                         the decision is the person's, not the parent model's"
                    );
                    return Some(Card {
                        id: id.clone(),
                        tool_name: tool_name.clone(),
                        arguments: arguments.clone(),
                        prompt: prompt.clone(),
                    });
                }
            }
        }
    }

    async fn finish(script: tokio::task::JoinHandle<(bool, String)>) -> (bool, String) {
        tokio::time::timeout(Duration::from_secs(60), script)
            .await
            .expect("the script finishes once answered")
            .expect("the script task completes")
    }

    /// `text` as a JavaScript string literal, for splicing a path into a script.
    ///
    /// ⚠ Not `"{path}"`. A Windows path's backslashes are escape sequences in a
    /// JS string — `\r` becomes a carriage return, `\U` loses its backslash — so
    /// the call the script makes would name a path that is not the one the
    /// test means, and a boundary check keyed on that path would silently miss.
    /// A JSON string literal is a valid JS one, with every backslash escaped.
    fn js_string(text: &str) -> String {
        serde_json::to_string(text).expect("a string always serialises")
    }

    /// The value the script handed `record_result`, out of `execute_code`'s
    /// `Result: <json>` text.
    fn recorded(output: &str) -> serde_json::Value {
        let json = output
            .strip_prefix("Result: ")
            .unwrap_or_else(|| panic!("not a script result: {output}"));
        serde_json::from_str(json).unwrap_or_else(|e| panic!("{e}: {output}"))
    }

    /// F7's headline, measured the way the QA run measured it: Manual mode, the
    /// script itself on the user's always-allow list, and a shell call inside
    /// it. The shell call is not on that list, so it must be put to the user —
    /// on a card that names `developer__shell` and carries the command — and it
    /// must run once they allow it.
    #[tokio::test]
    #[serial_test::serial]
    async fn manual_mode_asks_for_a_scripts_shell_call_under_the_inner_tools_name() {
        let f = fixture(BioRouterMode::Approve).await;
        let mut script = run_script(
            &f,
            r#"import { shell } from "developer";
               record_result(shell({ command: "echo SCRIPT-GATE-ALLOWED" }));"#,
            CancellationToken::new(),
        )
        .await;

        let card = match card_or_completion(&f.session.id, &mut script).await {
            Ok(card) => card,
            Err((_, output)) => panic!(
                "the script's developer__shell call ran with no approval card, because the \
                 always-allowed script vouched for it: {output}"
            ),
        };
        assert_eq!(
            card.tool_name, SHELL,
            "the card must name the tool the script called, not the script"
        );
        assert_eq!(
            card.arguments.get("command").and_then(|v| v.as_str()),
            Some("echo SCRIPT-GATE-ALLOWED"),
            "the card must carry the call's own evaluated arguments"
        );
        // The same card a direct call gets. An ordinary Manual-mode ask carries
        // no prompt, and that is load-bearing: the desktop reads any prompt as a
        // security finding, draws it as a warning and withholds "Always allow"
        // — measured in the running app when a provenance line was added here.
        assert_eq!(
            card.prompt, None,
            "an ordinary script ask must not look like a security finding"
        );

        answer(&f, &card, Permission::AllowOnce).await;
        let (is_error, output) = finish(script).await;
        assert!(!is_error, "an allowed call runs: {output}");
        assert!(
            output.contains("SCRIPT-GATE-ALLOWED"),
            "the allowed shell call's output reaches the script: {output}"
        );
    }

    /// The other answer on the same card: a denial comes back INTO the script
    /// as a tool error it can catch, and the script goes on — it is not ended
    /// silently, and the command never runs.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_denied_card_is_a_catchable_tool_error_and_the_script_goes_on() {
        let f = fixture(BioRouterMode::Approve).await;
        let marker = f.dir.path().join("denied-marker");
        let command = js_string(&format!("touch '{}'", marker.display()));
        let code = format!(
            r#"import {{ shell }} from "developer";
               let caught = null;
               try {{ shell({{ command: {command} }}); }}
               catch (e) {{ caught = String(e); }}
               record_result({{ caught, continued: true }});"#
        );
        let mut script = run_script(&f, &code, CancellationToken::new()).await;

        let card = match card_or_completion(&f.session.id, &mut script).await {
            Ok(card) => card,
            Err((_, output)) => {
                panic!("the script's developer__shell call ran with no approval card: {output}")
            }
        };
        assert_eq!(card.tool_name, SHELL);
        answer(&f, &card, Permission::DenyOnce).await;

        let (is_error, output) = finish(script).await;
        assert!(
            !is_error,
            "the script caught the refusal and finished: {output}"
        );
        let result = recorded(&output);
        assert_eq!(
            result["continued"],
            serde_json::json!(true),
            "the script must go on past a refused call: {output}"
        );
        let caught = result["caught"].as_str().unwrap_or_default();
        assert!(
            caught.contains(SHELL) && caught.contains("declined"),
            "the error the script caught must name the refused tool and say why: {output}"
        );
        assert!(!marker.exists(), "a denied command must not have run");
    }

    /// `always_deny` is keyed by the INNER tool's name, exactly as it is for a
    /// direct call: the refusal needs no card, it is an error the script can
    /// catch, and the script's next call is judged on its own name.
    #[tokio::test]
    #[serial_test::serial]
    async fn always_deny_on_the_inner_tool_refuses_it_and_the_script_continues() {
        let f = fixture(BioRouterMode::Approve).await;
        f.permissions
            .update_user_permission(SHELL, PermissionLevel::NeverAllow);
        f.permissions
            .update_user_permission("developer__text_editor", PermissionLevel::AlwaysAllow);
        let marker = f.dir.path().join("never-marker");
        let command = js_string(&format!("touch '{}'", marker.display()));
        // The developer server's path jail is the process working directory
        // here, which `cargo test` sets to this crate's root — so the next call
        // reads a file that is certainly inside it.
        let code = format!(
            r#"import {{ shell, text_editor }} from "developer";
               let caught = null;
               try {{ shell({{ command: {command} }}); }}
               catch (e) {{ caught = String(e); }}
               const after = text_editor({{ command: "view", path: "Cargo.toml" }});
               record_result({{ caught, after }});"#
        );
        let mut script = run_script(&f, &code, CancellationToken::new()).await;

        let (is_error, output) = match card_or_completion(&f.session.id, &mut script).await {
            Ok(card) => panic!(
                "a call the user always denies must be refused without a card, got one for {}",
                card.tool_name
            ),
            Err(finished) => finished,
        };
        assert!(
            !is_error,
            "the script caught the refusal and finished: {output}"
        );
        let result = recorded(&output);
        let caught = result["caught"].as_str().unwrap_or_default();
        assert!(
            caught.contains(SHELL) && caught.contains("declined"),
            "the always-denied call must come back into the script as a tool error: {output}"
        );
        assert!(
            result["after"]
                .as_str()
                .is_some_and(|text| text.contains("[package]")),
            "the script's next call must still run: {output}"
        );
        assert!(
            !marker.exists(),
            "an always-denied command must not have run"
        );
    }

    /// Auto mode is unchanged: the same script call runs, and no card is
    /// raised for it.
    #[tokio::test]
    #[serial_test::serial]
    async fn auto_mode_runs_a_scripts_shell_call_with_no_card() {
        let f = fixture(BioRouterMode::Auto).await;
        let mut script = run_script(
            &f,
            r#"import { shell } from "developer";
               record_result(shell({ command: "echo SCRIPT-GATE-AUTO" }));"#,
            CancellationToken::new(),
        )
        .await;

        let (is_error, output) = match card_or_completion(&f.session.id, &mut script).await {
            Ok(card) => panic!("Auto mode raised a card for {}", card.tool_name),
            Err(finished) => finished,
        };
        assert!(!is_error, "{output}");
        assert!(output.contains("SCRIPT-GATE-AUTO"), "{output}");
    }

    /// The sensitive-operations inspector — the one that asks even in Auto mode
    /// — judges a script's call too, on its evaluated arguments, and the card
    /// carries its own reason (which is also what makes the desktop withhold
    /// "Always allow" on it, as for a direct call).
    ///
    /// The target is `/etc`, which a test user cannot write, so a regression
    /// here fails with a permission error rather than touching the system.
    /// Not on Windows: the fixture is a POSIX command line, for the reason
    /// `sensitive_ops`' own tests give above their `cfg`s.
    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    #[serial_test::serial]
    async fn auto_mode_still_asks_for_a_scripts_sensitive_write() {
        let f = fixture(BioRouterMode::Auto).await;
        let mut script = run_script(
            &f,
            r#"import { shell } from "developer";
               let caught = null;
               try { shell({ command: "echo probe > /etc/biorouter-f7-sensitive-probe" }); }
               catch (e) { caught = String(e); }
               record_result({ caught });"#,
            CancellationToken::new(),
        )
        .await;

        let card = card_or_completion(&f.session.id, &mut script)
            .await
            .unwrap_or_else(|(_, output)| {
                panic!("a sensitive write inside a script must ask even in Auto mode: {output}")
            });
        assert_eq!(card.tool_name, SHELL);
        assert!(
            card.prompt
                .as_deref()
                .is_some_and(|prompt| prompt.contains("Sensitive system operation")),
            "the card must carry the sensitive-ops reason: {:?}",
            card.prompt
        );
        answer(&f, &card, Permission::DenyOnce).await;

        let (is_error, output) = finish(script).await;
        assert!(!is_error, "{output}");
        assert!(
            recorded(&output)["caught"]
                .as_str()
                .is_some_and(|caught| caught.contains("declined")),
            "{output}"
        );
    }

    /// `always_allow` is keyed by the inner tool's name in the other direction
    /// too: a call the user allowed by name runs with no card, script or not.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_call_the_user_always_allows_by_name_runs_with_no_card() {
        let f = fixture(BioRouterMode::Approve).await;
        f.permissions
            .update_user_permission(SHELL, PermissionLevel::AlwaysAllow);
        let mut script = run_script(
            &f,
            r#"import { shell } from "developer";
               record_result(shell({ command: "echo SCRIPT-GATE-BY-NAME" }));"#,
            CancellationToken::new(),
        )
        .await;

        let (is_error, output) = match card_or_completion(&f.session.id, &mut script).await {
            Ok(card) => panic!(
                "a call allowed by its own name must not ask, got a card for {}",
                card.tool_name
            ),
            Err(finished) => finished,
        };
        assert!(!is_error, "{output}");
        assert!(output.contains("SCRIPT-GATE-BY-NAME"), "{output}");
    }

    /// "Always allow" on a script call's card is recorded under the INNER
    /// tool's name — the name the card showed — so the script's next call to it
    /// needs no card, and neither will a direct one.
    #[tokio::test]
    #[serial_test::serial]
    async fn always_allow_on_a_scripts_card_is_recorded_under_the_inner_tools_name() {
        let f = fixture(BioRouterMode::Approve).await;
        let mut script = run_script(
            &f,
            r#"import { shell } from "developer";
               const first = shell({ command: "echo SCRIPT-GATE-FIRST" });
               const second = shell({ command: "echo SCRIPT-GATE-SECOND" });
               record_result({ first, second });"#,
            CancellationToken::new(),
        )
        .await;

        let card = card_or_completion(&f.session.id, &mut script)
            .await
            .unwrap_or_else(|(_, output)| panic!("the first call must ask: {output}"));
        assert_eq!(card.tool_name, SHELL);
        answer(&f, &card, Permission::AlwaysAllow).await;

        let (is_error, output) = match card_or_completion(&f.session.id, &mut script).await {
            Ok(card) => panic!(
                "a tool the user just always-allowed asked again: {}",
                card.tool_name
            ),
            Err(finished) => finished,
        };
        assert!(!is_error, "{output}");
        assert!(
            output.contains("SCRIPT-GATE-FIRST") && output.contains("SCRIPT-GATE-SECOND"),
            "{output}"
        );
        assert_eq!(
            f.permissions.get_user_permission(SHELL),
            Some(PermissionLevel::AlwaysAllow),
            "the grant belongs to the tool the card named"
        );
        assert_eq!(
            f.permissions.get_user_permission(EXECUTE_CODE),
            Some(PermissionLevel::AlwaysAllow),
            "and the script's own entry is untouched"
        );
    }

    /// Smart mode grades a script's call from the tool's OWN annotations, read
    /// out of the script's catalogue. The agent's registry is graded from the
    /// model's roster, which in Code Execution mode holds none of these tools —
    /// graded from it, the read-only `chatrecall` would read `Unknown` and ask
    /// like a shell.
    #[tokio::test]
    #[serial_test::serial]
    async fn smart_mode_grades_a_scripts_calls_from_the_scripts_own_catalogue() {
        let f = fixture_with(BioRouterMode::SmartApprove, &["chatrecall", "todo"]).await;
        let mut script = run_script(
            &f,
            r#"import { chatrecall } from "chatrecall";
               import { todo_write } from "todo";
               const recalled = chatrecall({ query: "script gate smart probe" });
               const written = todo_write({ content: "- [ ] SCRIPT-GATE-SMART" });
               record_result({ recalled: typeof recalled, written: typeof written });"#,
            CancellationToken::new(),
        )
        .await;

        let card = card_or_completion(&f.session.id, &mut script)
            .await
            .unwrap_or_else(|(_, output)| {
                panic!("a script's non-read-only call must ask in Smart mode: {output}")
            });
        assert_eq!(
            card.tool_name, "todo__todo_write",
            "the read-only chatrecall must pass on its own grade; only the write asks"
        );
        answer(&f, &card, Permission::AllowOnce).await;

        let (is_error, output) = finish(script).await;
        assert!(!is_error, "{output}");
    }

    /// Where nobody can be asked — a scheduled run — a script's ask is refused
    /// at once, as every other ask in that run is, instead of parking a card no
    /// interface drains for the whole time-to-live.
    ///
    /// ⚠ The script is driven INSIDE the scope, not spawned: the production
    /// scope (`scheduler.rs`) covers the whole run, and a task-local does not
    /// follow a spawn — which is exactly what `execute_code`'s own tool handler
    /// has to compensate for.
    #[tokio::test]
    #[serial_test::serial]
    async fn with_nobody_to_ask_a_scripts_ask_is_refused_at_once_not_parked() {
        let f = fixture(BioRouterMode::Approve).await;
        let (is_error, output) = crate::user_surface::without_human_surface(async {
            let dispatched = dispatch_script(
                &f,
                r#"import { shell } from "developer";
                   let caught = null;
                   try { shell({ command: "echo SCRIPT-GATE-UNATTENDED" }); }
                   catch (e) { caught = String(e); }
                   record_result({ caught });"#,
                CancellationToken::new(),
            )
            .await;
            let result = tokio::time::timeout(Duration::from_secs(60), dispatched.result)
                .await
                .expect("an unattended ask must not park")
                .expect("execute_code returns a result");
            (result.is_error.unwrap_or(false), text_of(&result))
        })
        .await;

        assert!(!is_error, "{output}");
        let caught = recorded(&output)["caught"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(
            caught.contains(SHELL) && caught.contains("nobody can be asked"),
            "the refusal must say why: {output}"
        );
        assert!(
            ActionRequiredManager::global()
                .drain_requests(&f.session.id)
                .is_empty(),
            "no card may be published where nobody can answer it"
        );
    }

    /// The refusals `execute_code` owes at its own boundary come FIRST, so
    /// nothing they refuse becomes something a card can allow: a script's shell
    /// command naming the machine-wide memory store is refused outright, as it
    /// always was, even in Manual mode where the judge would otherwise ask.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_boundary_refusal_stays_a_refusal_and_never_becomes_a_card() {
        // The store is resolved here and again inside the boundary check, both
        // through the process-global `BIOROUTER_PATH_ROOT`, which other tests
        // change under `env_lock`. Hold env_lock's one global mutex for the whole
        // test, writing nothing, so no such writer can land between the reads —
        // and no value read outside the lock is ever republished (see the same
        // guard in `code_execution_extension`'s rewrite test).
        let _env = env_lock::lock_env(Vec::<(&str, Option<&str>)>::new());

        let f = fixture(BioRouterMode::Approve).await;
        let store = biorouter_mcp::global_memory_dir().join("probe.txt");
        let command = js_string(&format!("cat '{}'", store.display()));
        let code = format!(
            r#"import {{ shell }} from "developer";
               let caught = null;
               try {{ shell({{ command: {command} }}); }}
               catch (e) {{ caught = String(e); }}
               record_result({{ caught }});"#
        );
        let mut script = run_script(&f, &code, CancellationToken::new()).await;

        let (is_error, output) = match card_or_completion(&f.session.id, &mut script).await {
            Ok(card) => panic!(
                "a call the boundary refuses must not be put to the user, got a card for {}",
                card.tool_name
            ),
            Err(finished) => finished,
        };
        assert!(!is_error, "{output}");
        assert!(
            recorded(&output)["caught"]
                .as_str()
                .is_some_and(|caught| caught.contains("global memory store")),
            "{output}"
        );
    }

    /// BR-19 on the script's path: a PreToolUse hook's rewrite of a script's
    /// call is applied, and the rewritten call — not the script's — is what the
    /// gate hands back to be dispatched.
    #[tokio::test]
    async fn a_hook_rewrite_of_a_scripts_call_is_what_runs() {
        use super::test_support::{gate, hooks_rewriting_shell_to, shell_call};
        use super::ScriptCallVerdict;

        let dir = tempfile::TempDir::new().expect("a scratch directory");
        let hooks = hooks_rewriting_shell_to("echo SCRIPT-GATE-REWRITTEN");
        let (gate, permissions) = gate(dir.path(), BioRouterMode::Approve, hooks, false).await;
        permissions.update_user_permission(SHELL, PermissionLevel::AlwaysAllow);

        let verdict = gate
            .judge(
                shell_call("echo SCRIPT-GATE-ORIGINAL"),
                crate::privacy::CallCapability::for_test_restricted(),
                &crate::permission::tool_risk::ToolRiskRegistry::new(),
                &CancellationToken::new(),
            )
            .await;
        let ScriptCallVerdict::Run(call) = verdict else {
            panic!("an always-allowed call with a benign rewrite runs: {verdict:?}");
        };
        assert_eq!(
            call.arguments
                .as_ref()
                .and_then(|args| args.get("command"))
                .and_then(|command| command.as_str()),
            Some("echo SCRIPT-GATE-REWRITTEN"),
            "what runs is what the hook rewrote it to"
        );
    }

    /// …and the rewrite is judged AGAIN, by the inspectors that only saw the
    /// script's original: the user's always-allow for `developer__shell` does
    /// not carry a rewritten `rm -rf /` past the catastrophic-command block.
    #[tokio::test]
    async fn a_rewritten_script_call_is_judged_again_before_it_runs() {
        use super::test_support::{gate, hooks_rewriting_shell_to, shell_call};
        use super::ScriptCallVerdict;

        let dir = tempfile::TempDir::new().expect("a scratch directory");
        let hooks = hooks_rewriting_shell_to("rm -rf /");
        let (gate, permissions) = gate(dir.path(), BioRouterMode::Approve, hooks, true).await;
        permissions.update_user_permission(SHELL, PermissionLevel::AlwaysAllow);

        let verdict = gate
            .judge(
                shell_call("ls"),
                crate::privacy::CallCapability::for_test_restricted(),
                &crate::permission::tool_risk::ToolRiskRegistry::new(),
                &CancellationToken::new(),
            )
            .await;
        match verdict {
            ScriptCallVerdict::Refuse(refusal) => {
                assert_eq!(refusal.kind, "permission_denied", "{refusal:?}");
            }
            ScriptCallVerdict::Run(call) => {
                panic!("a rewritten catastrophic command must not run: {call:?}")
            }
        }
    }

    /// `execute_code` through the door a PERSON's dispatch takes —
    /// `ExtensionManager::dispatch_tool_call`, with no judge scope anywhere
    /// above it. The only way to reach the handler the way a lost scope would.
    async fn dispatch_with_no_scope(f: &Fixture, code: &str) -> (bool, String) {
        let call = CallToolRequestParams {
            task: None,
            meta: None,
            name: EXECUTE_CODE.into(),
            arguments: Some(object!({ "code": code })),
        };
        let dispatched = f
            .agent
            .extension_manager
            .dispatch_tool_call(
                &f.session.id,
                call,
                crate::privacy::CallCapability::for_test_restricted(),
                CancellationToken::new(),
            )
            .await
            .expect("execute_code dispatches");
        let result = tokio::time::timeout(Duration::from_secs(60), dispatched.result)
            .await
            .expect("execute_code returns a result")
            .expect("execute_code returns a result");
        (result.is_error.unwrap_or(false), text_of(&result))
    }

    /// An absent judge is two situations, and only one of them may run.
    ///
    /// The benign half is a **person** dispatching a script through a door that
    /// judges nothing — `POST /agent/call_tool`, an Agent Drafter app, the
    /// coding-agent bridge. It runs, exactly as it always has.
    ///
    /// The other half is the agent loop's OWN dispatch arriving with its scope
    /// lost: the shape a `tokio::spawn` inserted anywhere between
    /// `Agent::dispatch_tool_call` and `handle_execute_code` would produce. Until
    /// this test, `try_with(..).ok()` collapsed it into the benign one and the
    /// script ran with every call inside it unjudged — a silent, permissive
    /// failure in a permission control, the opposite polarity from `unjudged()`
    /// three lines away. It must now be refused.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_script_whose_judge_was_lost_is_refused_and_a_person_driven_one_still_runs() {
        let f = fixture(BioRouterMode::Approve).await;

        // 1. Nothing recorded: a person drove it, and it runs.
        let (is_error, output) = dispatch_with_no_scope(
            &f,
            r#"import { shell } from "developer";
               record_result(shell({ command: "echo SCRIPT-GATE-PERSON-DRIVEN" }));"#,
        )
        .await;
        assert!(!is_error, "a person-driven script must still run: {output}");
        assert!(
            output.contains("SCRIPT-GATE-PERSON-DRIVEN"),
            "…and its call must have run: {output}"
        );

        // 2. The agent loop IS running a script for this session — and the
        //    handler cannot see the judge. Refused, and nothing inside it ran.
        let recorded = super::DispatchedByAgentLoop::record(&f.session.id);
        let (is_error, output) = dispatch_with_no_scope(
            &f,
            r#"import { shell } from "developer";
               record_result(shell({ command: "echo SCRIPT-GATE-UNJUDGED" }));"#,
        )
        .await;
        drop(recorded);

        assert!(
            is_error,
            "a script the agent loop dispatched with no judge must be refused: {output}"
        );
        assert!(
            output.contains("permission judge"),
            "…and the refusal must say what was missing: {output}"
        );
        assert!(
            !output.contains("SCRIPT-GATE-UNJUDGED"),
            "…and its shell call must never have run: {output}"
        );
        assert!(
            ActionRequiredManager::global()
                .drain_requests(&f.session.id)
                .is_empty(),
            "a lost judge is a defect, not a decision to put to the user"
        );

        // 3. …and the record is released with the body, so the next person-driven
        //    dispatch is benign again rather than permanently refused.
        let (is_error, output) = dispatch_with_no_scope(
            &f,
            r#"import { shell } from "developer";
               record_result(shell({ command: "echo SCRIPT-GATE-RELEASED" }));"#,
        )
        .await;
        assert!(!is_error, "{output}");
        assert!(output.contains("SCRIPT-GATE-RELEASED"), "{output}");
    }

    /// #246 review, finding 2. `execute_code` takes a permit from the
    /// process-global eight-permit dispatch semaphore and holds it for the whole
    /// tool body — which, since F7, contains every approval card a script's
    /// sub-call parks on, for up to `approval_ttl()` (3600 s by default,
    /// `Duration::MAX` when the confirmation timeout is 0). Eight scripts parked
    /// on cards would stall every other tool call in the daemon, the user's own
    /// foreground conversation included. So a parked ask must hold no permit.
    ///
    /// Measured by filling the ceiling to exactly one free permit before the
    /// script runs: the script's dispatch takes the last one, and while it is
    /// parked on its card an unrelated dispatch must still be able to acquire.
    /// Before the fix that acquisition never completes.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_parked_scripts_ask_holds_no_global_dispatch_permit() {
        use crate::agents::tool_dispatch_limits;

        let f = fixture(BioRouterMode::Approve).await;
        let dir = f.dir.path().to_path_buf();

        // Fill the ceiling to one free permit. `probe` names no file, so these
        // are concurrency permits and nothing else. Generous timeout: other
        // tests in this binary hold permits briefly and release them.
        let mut filled = Vec::new();
        for _ in 0..tool_dispatch_limits::max_concurrent_tools().saturating_sub(1) {
            filled.push(
                tokio::time::timeout(
                    Duration::from_secs(30),
                    tool_dispatch_limits::acquire("probe", None, &dir),
                )
                .await
                .expect("the binary's other tool dispatches release their permits"),
            );
        }

        let mut script = run_script(
            &f,
            r#"import { shell } from "developer";
               record_result(shell({ command: "echo SCRIPT-GATE-PARKED" }));"#,
            CancellationToken::new(),
        )
        .await;
        let card = card_or_completion(&f.session.id, &mut script)
            .await
            .unwrap_or_else(|(_, output)| panic!("the shell call must ask: {output}"));
        assert_eq!(card.tool_name, SHELL);

        // The script is parked on that card and must therefore be holding
        // nothing: the last permit has to be available to an unrelated tool.
        let unrelated = tokio::time::timeout(
            Duration::from_secs(10),
            tool_dispatch_limits::acquire("probe", None, &dir),
        )
        .await;
        assert!(
            unrelated.is_ok(),
            "a script parked on an approval card held its dispatch permit; eight of \
             those stall every other tool call in the daemon"
        );

        // Free everything before answering: the script queues for a permit again
        // when it resumes, and would otherwise be waiting on this test.
        drop(unrelated);
        drop(filled);

        answer(&f, &card, Permission::AllowOnce).await;
        let (is_error, output) = finish(script).await;
        assert!(!is_error, "the answered call still runs: {output}");
        assert!(output.contains("SCRIPT-GATE-PARKED"), "{output}");
    }

    /// The record the refusal above keys on is taken by `judging_script_calls`
    /// itself, so it cannot be wired up without the scope it vouches for — and
    /// it is released when the body ends, cancelled bodies included.
    #[tokio::test]
    async fn the_agent_loop_record_lives_exactly_as_long_as_the_scope() {
        let session = "script-gate-record-probe";
        assert!(!super::agent_loop_is_running_a_script(session));
        {
            let _held = super::DispatchedByAgentLoop::record(session);
            assert!(super::agent_loop_is_running_a_script(session));
            let _nested = super::DispatchedByAgentLoop::record(session);
            assert!(super::agent_loop_is_running_a_script(session));
        }
        assert!(
            !super::agent_loop_is_running_a_script(session),
            "the record must not outlive the bodies that took it"
        );
    }

    /// **D10.** A card a script raises inside a DELEGATED child must reach the
    /// conversation the person is actually watching.
    ///
    /// Measured in the running app: a script's ask inside a subagent published
    /// to the child's session and nowhere else, so a user watching the parent
    /// saw the `subagent` tool call sit there with no card, no card anywhere in
    /// that chat, and no explanation — and a child running without a tab (a
    /// `visible: false` spawn, a fan-out past the four-tab cap, a terminal
    /// session) had no surface at all. The card then sat out its full
    /// `approval_ttl()` — 3600 s by default — and the run was lost.
    ///
    /// The same call made DIRECTLY by the child's model already escalates:
    /// `approval_relay::begin_delegated_approval` returns
    /// `AwaitingHuman { surfaced_in: root }` and `handle_approval_tool_requests`
    /// publishes the identical card into that session's bus. This asserts the
    /// script path does the same, because in the shipped Code Execution default
    /// the script path is how nearly every tool call is made.
    ///
    /// The bus is the right place to assert: `POST /reply` and
    /// `GET /sessions/{id}/events` BOTH drain it (`routes/reply.rs` +
    /// `routes/session_events.rs`), so a frame published there is what the
    /// parent's tab renders whether the user is driving that chat or observing
    /// it.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_subagents_script_ask_surfaces_where_the_person_watching_the_parent_is() {
        let f = fixture(BioRouterMode::Approve).await;
        let child = delegated_child(&f).await;
        // Subscribed BEFORE the script runs: `session_events::publish` is a pure
        // lookup that creates no ring, so a card published to a session nobody
        // is watching is dropped — subscribing afterwards would measure the
        // race, not the behaviour.
        let mut watching_the_parent = crate::session_events::subscribe(&f.session.id);

        let mut script = run_script_in(
            &f,
            &child,
            r#"import { shell } from "developer";
               record_result(shell({ command: "echo SCRIPT-GATE-DELEGATED" }));"#,
            CancellationToken::new(),
        )
        .await;

        let childs_card = card_or_completion(&child.id, &mut script)
            .await
            .unwrap_or_else(|(_, output)| panic!("the child's shell call must ask: {output}"));
        assert_eq!(childs_card.tool_name, SHELL);

        let escalated = card_on_bus(&mut watching_the_parent, Duration::from_secs(30))
            .await
            .expect(
                "a script's approval card inside a subagent never reached the conversation the \
                 person is watching: the parent's chat shows an unexplained stall while the \
                 child parks for its whole time-to-live",
            );
        assert_eq!(
            escalated.id, childs_card.id,
            "the escalated card must be the SAME ask — one decision, two surfaces — not a \
             second question with its own id"
        );
        assert_eq!(escalated.tool_name, SHELL);
        assert_eq!(
            escalated.arguments.get("command").and_then(|v| v.as_str()),
            Some("echo SCRIPT-GATE-DELEGATED"),
            "the watching person needs the call's own arguments to decide"
        );
        assert_eq!(
            escalated.prompt, None,
            "an ordinary script ask must not look like a security finding — the desktop \
             draws any prompt as a warning banner and withholds Always allow"
        );

        answer(&f, &childs_card, Permission::AllowOnce).await;
        let (is_error, output) = finish(script).await;
        assert!(!is_error, "the allowed call runs: {output}");
        assert!(output.contains("SCRIPT-GATE-DELEGATED"), "{output}");
    }

    /// The other half of D10, and the half that makes the card worth showing: a
    /// person clicking Allow in the PARENT's chat resolves the child's parked
    /// call.
    ///
    /// Publishing without this would be worse than the bug — a card the user can
    /// see, click, and watch do nothing, because
    /// `PendingUserActions::resolve_in_session` compares the posting session id
    /// against the parked entry's and answers `Unknown` for anything else.
    ///
    /// Note what is NOT relaxed: the decision still comes from a person, through
    /// the same `DecisionAuthority` the route samples from the request. Nothing
    /// asks the parent AGENT, and `approval_relay`'s ancestor consultation is
    /// not reached from here at all.
    #[tokio::test]
    #[serial_test::serial]
    async fn the_person_watching_the_parent_can_answer_the_childs_card() {
        let f = fixture(BioRouterMode::Approve).await;
        let child = delegated_child(&f).await;
        let mut watching_the_parent = crate::session_events::subscribe(&f.session.id);

        let mut script = run_script_in(
            &f,
            &child,
            r#"import { shell } from "developer";
               record_result(shell({ command: "echo SCRIPT-GATE-ANSWERED-ABOVE" }));"#,
            CancellationToken::new(),
        )
        .await;
        let card = card_or_completion(&child.id, &mut script)
            .await
            .unwrap_or_else(|(_, output)| panic!("the child's shell call must ask: {output}"));
        // Drain the escalated copy so the assertion below is about answering it,
        // not about whether it arrived — that is the test above.
        card_on_bus(&mut watching_the_parent, Duration::from_secs(30))
            .await
            .expect("the escalated card must reach the parent first");

        let outcome = answer_from(&f, &f.session.id, &card, Permission::AllowOnce).await;
        assert_eq!(
            outcome,
            crate::agents::ConfirmationOutcome::Delivered,
            "Allow clicked in the watching conversation must release the child's parked \
             call; anything else leaves the user looking at a card that does nothing"
        );

        let (is_error, output) = finish(script).await;
        assert!(!is_error, "the allowed call runs: {output}");
        assert!(output.contains("SCRIPT-GATE-ANSWERED-ABOVE"), "{output}");
    }

    /// A decision may come from the child's own tab or from where it was
    /// escalated — and from nowhere else. The escalation widens the answering
    /// scope by exactly one session, so an unrelated chat that happens to know
    /// the request id still resolves nothing (#40's rule, unchanged).
    #[tokio::test]
    #[serial_test::serial]
    async fn an_unrelated_conversation_still_cannot_answer_the_childs_card() {
        let f = fixture(BioRouterMode::Approve).await;
        let child = delegated_child(&f).await;
        let bystander = f
            .agent
            .config
            .session_manager
            .create_session(
                f.dir.path().to_path_buf(),
                "bystander".into(),
                SessionType::User,
            )
            .await
            .expect("a bystander session");

        let mut script = run_script_in(
            &f,
            &child,
            r#"import { shell } from "developer";
               record_result(shell({ command: "echo SCRIPT-GATE-BYSTANDER" }));"#,
            CancellationToken::new(),
        )
        .await;
        let card = card_or_completion(&child.id, &mut script)
            .await
            .unwrap_or_else(|(_, output)| panic!("the child's shell call must ask: {output}"));

        let refused = answer_from(&f, &bystander.id, &card, Permission::AllowOnce).await;
        assert_eq!(
            refused,
            crate::agents::ConfirmationOutcome::Unknown,
            "a session that is neither the child nor an escalation surface must not be able \
             to grant the child's call"
        );

        answer_from(&f, &child.id, &card, Permission::DenyOnce).await;
        let (_, output) = finish(script).await;
        assert!(
            !output.contains("SCRIPT-GATE-BYSTANDER"),
            "the bystander's Allow must not have run the command: {output}"
        );
    }

    /// A root conversation has nowhere to escalate to, and must not gain a
    /// second card: the one the drain yields IS the person's. Guards against an
    /// escalation that fires for every session and shows every ordinary chat its
    /// own card twice.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_root_chats_script_ask_is_published_once() {
        let f = fixture(BioRouterMode::Approve).await;
        assert!(
            f.session.parent_session_id.is_none(),
            "the fixture's chat must be a root for this to measure anything"
        );
        let mut watching = crate::session_events::subscribe(&f.session.id);

        let mut script = run_script(
            &f,
            r#"import { shell } from "developer";
               record_result(shell({ command: "echo SCRIPT-GATE-ROOT-ONCE" }));"#,
            CancellationToken::new(),
        )
        .await;
        let card = card_or_completion(&f.session.id, &mut script)
            .await
            .unwrap_or_else(|(_, output)| panic!("the shell call must ask: {output}"));

        assert!(
            card_on_bus(&mut watching, Duration::from_millis(750))
                .await
                .is_none(),
            "a root chat's own ask must not also be published to its bus as an escalation; \
             the drain already yields it into that chat's stream"
        );

        answer(&f, &card, Permission::AllowOnce).await;
        let (is_error, output) = finish(script).await;
        assert!(!is_error, "{output}");
    }

    #[test]
    fn both_name_forms_of_execute_code_get_a_judge_and_nothing_else_does() {
        use crate::agents::code_execution_extension::is_execute_code_call;
        assert!(is_execute_code_call(EXECUTE_CODE));
        assert!(
            is_execute_code_call("execute_code"),
            "models strip prefixes, and the manager resolves the bare name"
        );
        for other in [
            "code_execution__read_module",
            "code_execution__search_modules",
            "developer__execute_code",
            "code_executionexecute_code",
            "code_execution__execute_code_extra",
            SHELL,
        ] {
            assert!(!is_execute_code_call(other), "{other}");
        }
    }
}
