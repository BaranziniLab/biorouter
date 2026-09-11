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
//! reads it with [`current`] and hands it to the task that dispatches the
//! script's calls. Absent means the caller is not the agent loop: `POST
//! /agent/call_tool`, which a person drives and which bypasses every inspector
//! for the outer call too. There is no model decision to gate there, so a
//! script run that way behaves exactly as it always has.
//!
//! [`ToolInspector`]: crate::tool_inspection::ToolInspector
//! [`Agent::dispatch_tool_call`]: crate::agents::Agent::dispatch_tool_call

use std::sync::Arc;
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
    PendingUserActions, ToolApprovalRequest, UserActionOutcome, UserActionRequest,
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

/// Run `tool_body` with `gate` judging any call a script makes inside it.
///
/// ⚠ Wrap the tool's BODY. `dispatch_tool_call` returns a future, and a scope
/// around the dispatch alone is gone before the script runs — the shape of the
/// #160 hang, and of every scope bug in that family.
pub(crate) async fn judging_script_calls<F: std::future::Future>(
    gate: Arc<ScriptCallGate>,
    tool_body: F,
) -> F::Output {
    SCRIPT_CALL_GATE.scope(gate, tool_body).await
}

/// The gate installed around the tool body running on this task, if any.
pub(crate) fn current() -> Option<Arc<ScriptCallGate>> {
    SCRIPT_CALL_GATE.try_with(Arc::clone).ok()
}

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
    /// path arguments, and its id as the only surface an ask may be put on.
    session: Session,
    /// For the PreToolUse rewrites this gate's own inspection staged, and the
    /// PermissionRequest hooks consulted before a card.
    hooks: Arc<HooksManager>,
}

impl ScriptCallGate {
    pub(crate) fn new(
        inspections: Arc<ToolInspectionManager>,
        mode: BioRouterMode,
        session: Session,
        hooks: Arc<HooksManager>,
    ) -> Self {
        Self {
            inspections,
            mode,
            session,
            hooks,
        }
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
        let outcome = parked.wait(approval_ttl(), Some(cancel)).await;
        self.verdict_for_answer(call, outcome, cancel).await
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
        let session = SessionManager::new(dir.join("sessions"))
            .create_session(dir.to_path_buf(), "gate".into(), SessionType::User)
            .await
            .expect("a session");
        (
            ScriptCallGate::new(Arc::new(inspections), mode, session, hooks),
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
        let dispatched = dispatch_script(f, code, cancel).await;
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
        let call = CallToolRequestParams {
            task: None,
            meta: None,
            name: EXECUTE_CODE.into(),
            arguments: Some(object!({ "code": code })),
        };
        let (_, dispatched) = f
            .agent
            .dispatch_tool_call(call, "outer-execute-code".into(), Some(cancel), &f.session)
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
        let outcome = f
            .agent
            .handle_confirmation_for_session(
                &f.session.id,
                card.id.clone(),
                PermissionConfirmation {
                    principal_type: PrincipalType::Tool,
                    permission,
                },
                DecisionAuthority::unproven(),
            )
            .await;
        assert_eq!(
            outcome,
            crate::agents::ConfirmationOutcome::Delivered,
            "the card's decision must reach the parked call"
        );
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
