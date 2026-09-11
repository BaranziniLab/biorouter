use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_stream::try_stream;
use futures::stream::{self, BoxStream};
use futures::{Stream, StreamExt};
use tokio::sync::{oneshot, Mutex};
use tokio_util::sync::CancellationToken;

use crate::config::permission::PermissionLevel;
use crate::mcp_utils::ToolResult;
use crate::permission::{Permission, PermissionConfirmation};
use rmcp::model::{Content, ServerNotification};

// ToolCallResult combines the result of a tool call with an optional notification stream that
// can be used to receive notifications from the tool.
pub struct ToolCallResult {
    pub result: Box<dyn Future<Output = ToolResult<rmcp::model::CallToolResult>> + Send + Unpin>,
    pub notification_stream: Option<Box<dyn Stream<Item = ServerNotification> + Send + Unpin>>,
}

impl From<ToolResult<rmcp::model::CallToolResult>> for ToolCallResult {
    fn from(result: ToolResult<rmcp::model::CallToolResult>) -> Self {
        Self {
            result: Box::new(futures::future::ready(result)),
            notification_stream: None,
        }
    }
}

use super::agent::{tool_stream, ToolStream};
use crate::agents::Agent;
use crate::conversation::message::{Message, ToolRequest};
use crate::session::Session;
use crate::tool_inspection::get_security_finding_id_from_results;

pub const DECLINED_RESPONSE: &str = "The user has declined to run this tool. \
    DO NOT attempt to call this tool again. \
    If there are no alternative methods to proceed, clearly explain the situation and STOP.";

pub const CHAT_MODE_TOOL_SKIPPED_RESPONSE: &str = "Let the user know the tool call was skipped in biorouter chat mode. \
                                        DO NOT apologize for skipping the tool call. DO NOT say sorry. \
                                        Provide an explanation of what the tool call would do, structured as a \
                                        plan for the user. Again, DO NOT apologize. \
                                        **Example Plan:**\n \
                                        1. **Identify Task Scope** - Determine the purpose and expected outcome.\n \
                                        2. **Outline Steps** - Break down the steps.\n \
                                        If needed, adjust the explanation based on user preferences or questions.";

pub const EXPIRED_RESPONSE: &str = "The permission prompt for this tool call expired before the \
    user answered it, so the tool was NOT run. The user is likely away. Do not silently retry the \
    same call; summarize what you were about to do and stop.";

/// PAR-04: the result recorded for a tool that was still running when the user
/// cancelled the turn, so its slot in the batch is never left empty.
///
/// Cancellation breaks the batch execution loop immediately, abandoning any tool
/// that has not yet returned. Without this backfill the post-batch loop persists
/// those requests as `tool_use` blocks with no `tool_result`, and a transcript
/// carrying an unmatched `tool_use` is one every provider rejects on replay.
pub const CANCELLED_MID_RUN_RESPONSE: &str = "The user cancelled this turn while the tool call \
    was still running, so it was interrupted and produced no result. It may have partially \
    completed. Do not assume it succeeded, and do not silently retry it.";

pub const CANCELLED_RESPONSE: &str = "The user cancelled this turn before deciding on this tool \
    call, so the tool was NOT run.";

/// How long a tool-permission prompt stays answerable before it expires (BR-62).
///
/// Deliberately generous: the human on the other end may be reading the diff, in
/// another window, or at lunch, and a premature expiry is indistinguishable from
/// a denial they never made. The TTL exists to bound the *pathological* case —
/// a confirmation that can never arrive because the client crashed, navigated
/// away, or dropped the card — which before BR-62 parked the turn (and the
/// session's turn lock) forever.
const DEFAULT_CONFIRMATION_TIMEOUT_SECS: u64 = 3600;

/// Resolve the permission-prompt TTL, honoring `BIOROUTER_CONFIRMATION_TIMEOUT_SECS`.
///
/// `pub(crate)` for [`crate::agents::script_call_gate`]: a card raised for a
/// call inside a Code Execution script is the same question as the card for a
/// direct call, so it stays answerable for exactly as long.
pub(crate) fn confirmation_timeout() -> Option<Duration> {
    parse_confirmation_timeout(std::env::var("BIOROUTER_CONFIRMATION_TIMEOUT_SECS").ok())
}

/// What a denied tool call is answered with, chosen by which inspector denied it.
///
/// One function for both places a denial is written: the agent loop's own
/// denied calls (`Agent::handle_denied_tools`) and a denied call inside a Code
/// Execution script (`script_call_gate`). The two used to be one inline match,
/// and a second inline copy is how a refusal comes to read differently
/// depending on whether the model called a tool directly or from a script.
pub(crate) fn denied_response_text(
    request_id: &str,
    inspection_results: &[crate::tool_inspection::InspectionResult],
) -> String {
    use crate::tool_inspection::InspectionAction;

    // When an inspector denied this call, tell the model why so it can adjust
    // instead of blindly retrying. The always-on catastrophic-command block
    // (security inspector) and hook denials carry a reason; surface it verbatim
    // / with context.
    let deny_reason = inspection_results.iter().find(|result| {
        result.tool_request_id == request_id
            && result.action == InspectionAction::Deny
            && !result.reason.trim().is_empty()
    });
    match deny_reason {
        Some(result) if result.inspector_name == crate::hooks::inspector::HOOK_INSPECTOR_NAME => {
            format!("{DECLINED_RESPONSE}\n\nHook feedback: {}", result.reason)
        }
        // Non-bypassable safety block: the user did not decline, the command is
        // refused outright, so return the reason directly.
        Some(result) if result.inspector_name == "security" => result.reason.clone(),
        // BR-29/BR-31: a loop guard tripped — the call repeated itself, or the
        // tool has been failing the same way over and over. The user did not
        // decline anything; telling the model they did (the old
        // DECLINED_RESPONSE) is actively misleading and leaves it unable to
        // diagnose the stop. Return the real reason.
        Some(result) if result.inspector_name == crate::tool_monitor::REPETITION_INSPECTOR_NAME => {
            result.reason.clone()
        }
        // #63: a cross-session memory shape Biorouter refuses (the whole-store
        // global read). Same reasoning as the loop guards above — the user
        // declined nothing, and the reason is the only thing that tells the
        // model the itemised call still works. `DECLINED_RESPONSE` here would be
        // both untrue and unactionable, and would read as the feature being off.
        Some(result)
            if result.inspector_name
                == crate::security::global_memory::GLOBAL_MEMORY_INSPECTOR_NAME =>
        {
            result.reason.clone()
        }
        // F4: a `workspace_set_tools` change that cannot be made — a built-in
        // capability named for removal, a knowledge base that does not exist —
        // refused BEFORE any card. Nobody was asked, so "the user has declined"
        // would be false, and the reason is the handler's own sentence.
        Some(result)
            if result.inspector_name
                == crate::agents::workspace_inspector::WORKSPACE_MUTATION_INSPECTOR_NAME =>
        {
            result.reason.clone()
        }
        _ => DECLINED_RESPONSE.to_string(),
    }
}

/// The TTL policy, split out from the env read so it can be tested without
/// mutating process-global state.
///
/// `0` disables the TTL and restores the pre-BR-62 behaviour of waiting forever
/// (still cancellable), for anyone who would rather block than risk an expiry.
/// Unset or unparseable falls back to the default.
fn parse_confirmation_timeout(raw: Option<String>) -> Option<Duration> {
    let secs = raw
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_CONFIRMATION_TIMEOUT_SECS);
    (secs > 0).then(|| Duration::from_secs(secs))
}

/// How a wait for a tool-permission decision ended (BR-62).
enum ConfirmationWait {
    /// The user (or a client acting for them) decided.
    Confirmed(PermissionConfirmation),
    /// The TTL elapsed, or the prompt was dropped without a decision.
    Expired,
    /// The turn was cancelled out from under the prompt.
    Cancelled,
}

/// Wait for this prompt's decision, bounded on three sides.
///
/// Before BR-62 this was a bare `recv()` on a per-agent channel: the only way out
/// was a decision, so a cancel could not unblock it and a decision that never came
/// hung the turn forever. Now the decision races the turn's cancellation token and
/// a TTL, so every path out of the prompt is one the loop can handle.
async fn await_confirmation(
    rx: oneshot::Receiver<PermissionConfirmation>,
    cancellation_token: Option<CancellationToken>,
    timeout: Option<Duration>,
) -> ConfirmationWait {
    let cancelled = async move {
        match cancellation_token {
            Some(token) => token.cancelled_owned().await,
            // No token: this arm must never win the race.
            None => std::future::pending::<()>().await,
        }
    };
    let expired = async move {
        match timeout {
            Some(ttl) => tokio::time::sleep(ttl).await,
            // TTL disabled: this arm must never win the race.
            None => std::future::pending::<()>().await,
        }
    };

    tokio::select! {
        // Biased so a decision that genuinely landed is honored even if the turn
        // is cancelled or the TTL fires in the same instant — the user answered.
        biased;
        decision = rx => match decision {
            Ok(confirmation) => ConfirmationWait::Confirmed(confirmation),
            // Our sender was dropped without a decision (the prompt was forgotten
            // out from under us). Treat it as an expiry rather than parking on a
            // channel nobody holds.
            Err(_) => ConfirmationWait::Expired,
        },
        () = cancelled => ConfirmationWait::Cancelled,
        () = expired => ConfirmationWait::Expired,
    }
}

/// Record `text` as the tool response for `request`, marked as an error.
///
/// Every tool request must end with a tool response — a request with none breaks
/// the *next* provider call — so declines, expiries and cancels all funnel here.
async fn write_tool_response(
    request: &ToolRequest,
    request_to_response_map: &HashMap<String, Arc<Mutex<Message>>>,
    text: &str,
) {
    if let Some(response_msg) = request_to_response_map.get(&request.id) {
        let mut response = response_msg.lock().await;
        *response = response.clone().with_tool_response_with_metadata(
            request.id.clone(),
            Ok(rmcp::model::CallToolResult {
                content: vec![Content::text(text)],
                structured_content: None,
                is_error: Some(true),
                meta: None,
            }),
            request.metadata.as_ref(),
        );
    }
}

impl Agent {
    #[allow(clippy::too_many_lines)]
    pub(crate) fn handle_approval_tool_requests<'a>(
        &'a self,
        tool_requests: &'a [ToolRequest],
        tool_futures: Arc<Mutex<Vec<(String, ToolStream)>>>,
        request_to_response_map: &'a HashMap<String, Arc<Mutex<Message>>>,
        cancellation_token: Option<CancellationToken>,
        session: &'a Session,
        inspection_results: &'a [crate::tool_inspection::InspectionResult],
    ) -> BoxStream<'a, anyhow::Result<Message>> {
        try_stream! {
        for (idx, request) in tool_requests.iter().enumerate() {
            if let Ok(tool_call) = request.tool_call.clone() {
                // PermissionRequest hooks may resolve the approval without
                // prompting the user (allow -> dispatch, deny -> declined).
                let hook_decision = {
                    let tool_input = tool_call
                        .arguments
                        .clone()
                        .map(serde_json::Value::Object)
                        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
                    let aggregate = self
                        .hooks_manager
                        .permission_request(&session.id, &session.working_dir, &tool_call.name, &tool_input)
                        .await;
                    // BR-19: this gate used to read only `aggregate.decision`, so a
                    // PermissionRequest hook's `additionalContext` / `systemMessage`
                    // was computed and thrown away. Stage them for the turn's
                    // injection point (a message yielded from here never enters the
                    // conversation, only the event stream).
                    if !aggregate.additional_context.is_empty() || !aggregate.system_messages.is_empty() {
                        self.hooks_manager.stage_tool_hook(
                            &session.id,
                            crate::hooks::StagedToolHook {
                                event: crate::hooks::HookEvent::PermissionRequest,
                                tool_request_id: request.id.clone(),
                                tool_name: tool_call.name.to_string(),
                                // Rewrites are PreToolUse-only: by here the call is
                                // already recorded in the transcript.
                                updated_input: None,
                                additional_context: aggregate.additional_context.clone(),
                                system_messages: aggregate.system_messages.clone(),
                            },
                        );
                    }
                    aggregate.decision
                };

                // Some approvals are questions for the human, not policy an
                // automation may answer: a security finding, an Auto-mode
                // sensitive-file escalation, a managed-policy ask, or (issue #63)
                // a machine-wide memory disclosure. Those inspectors exist
                // *because* automated grants must not decide the call, and a hook
                // is an automated grant that runs after the escalation-only merge,
                // where the lattice can no longer defend the verdict.
                let requires_a_human =
                    crate::tool_inspection::approval_requires_a_human(&request.id, inspection_results);

                match hook_decision {
                    // A hook `allow` on a non-delegable approval is dropped, not
                    // honoured — the card below is shown as if no hook had run.
                    // (Its `additionalContext` / `systemMessage` were already
                    // staged above and still reach the turn.)
                    Some(crate::hooks::HookDecision::Allow { reason }) if requires_a_human => {
                        tracing::warn!(
                            counter.biorouter.non_delegable_approval_hook_ignored = 1,
                            tool_name = %tool_call.name,
                            tool_request_id = %request.id,
                            reason = reason.as_deref().unwrap_or(""),
                            "PermissionRequest hook tried to auto-approve a security-raised \
                             approval; asking the user instead"
                        );
                    }
                    Some(crate::hooks::HookDecision::Allow { reason }) => {
                        tracing::info!(
                            tool_name = %tool_call.name,
                            reason = reason.as_deref().unwrap_or(""),
                            "PermissionRequest hook auto-approved tool call"
                        );
                        let (req_id, tool_result) = self.dispatch_tool_call(tool_call.clone(), request.id.clone(), cancellation_token.clone(), session).await;
                        let mut futures = tool_futures.lock().await;
                        futures.push((req_id, match tool_result {
                            Ok(result) => tool_stream(
                                result.notification_stream.unwrap_or_else(|| Box::new(stream::empty())),
                                result.result,
                            ),
                            Err(e) => tool_stream(
                                Box::new(stream::empty()),
                                futures::future::ready(Err(e)),
                            ),
                        }));
                        continue;
                    }
                    Some(crate::hooks::HookDecision::Deny { reason }) => {
                        tracing::info!(
                            tool_name = %tool_call.name,
                            reason = %reason,
                            "PermissionRequest hook denied tool call"
                        );
                        if let Some(response_msg) = request_to_response_map.get(&request.id) {
                            let mut response = response_msg.lock().await;
                            *response = response.clone().with_tool_response_with_metadata(
                                request.id.clone(),
                                Ok(rmcp::model::CallToolResult {
                                    content: vec![Content::text(format!(
                                        "{DECLINED_RESPONSE}\n\nHook feedback: {reason}"
                                    ))],
                                    structured_content: None,
                                    is_error: Some(true),
                                    meta: None,
                                }),
                                request.metadata.as_ref(),
                            );
                        }
                        yield Message::assistant()
                            .with_system_notification(
                                crate::conversation::message::SystemNotificationType::InlineMessage,
                                format!("Hook denied permission for {}: {}", tool_call.name, reason),
                            )
                            .user_only();
                        continue;
                    }
                    _ => {}
                }

                // No hook decision: notify hooks that a permission prompt is
                // being shown, then fall through to the normal confirmation.
                {
                    let mut payload = crate::hooks::HookPayload::new(
                        crate::hooks::HookEvent::Notification,
                        &session.id,
                        session.working_dir.to_string_lossy(),
                    );
                    payload.message = Some(format!("Permission required for {}", tool_call.name));
                    self.hooks_manager.fire(
                        crate::hooks::HookEvent::Notification,
                        Some("permission_prompt".to_string()),
                        payload,
                        session.working_dir.clone(),
                    );
                }

                // What the card tells the user about *why* they are being asked.
                let security_message = crate::tool_inspection::approval_prompt_for_request(
                    &request.id,
                    inspection_results,
                );

                // BR-63: give the card enough to decide with. The BR-18 registry
                // already grades every tool it handed the model this turn, and
                // the preview turns the raw arguments into the thing the user
                // actually needs to see — the command, or the diff.
                let arguments = tool_call.arguments.clone().unwrap_or_default();
                let risk = self.tool_risks.risk_for(&tool_call.name);
                let preview = crate::conversation::tool_preview::ToolPreview::for_tool_call(
                    &tool_call.name,
                    &arguments,
                );
                // BR-62: register this prompt's own channel BEFORE the card is
                // yielded, so an instant answer cannot arrive before there is a
                // sender to route it to.
                let confirmation_rx = self.register_confirmation(&request.id);

                // BR-71 Task 36b: an agent-created session's approvals belong to
                // the agent that created it. This decides within the chain's own
                // authority where it can, and otherwise surfaces the ask to a
                // person — in which case the card below is one of its two
                // surfaces and carries the same request id.
                let ask = crate::agents::approval_relay::AskId {
                    session_id: session.id.clone(),
                    request_id: request.id.clone(),
                };
                let delegation = crate::agents::approval_relay::begin_delegated_approval(
                    self,
                    request,
                    session,
                    inspection_results,
                )
                .await;

                // A card for an ask an ancestor already answered would flash and
                // vanish. `deliver` has already sent the decision into
                // `confirmation_rx`, so the wait below returns immediately.
                if !matches!(
                    delegation,
                    crate::agents::approval_relay::Delegation::DecidedByAncestor(_)
                ) {
                    let confirmation = Message::assistant()
                        .with_action_required_with_context(
                            request.id.clone(),
                            tool_call.name.to_string().clone(),
                            arguments,
                            security_message,
                            Some(risk),
                            preview,
                        )
                        .user_only();
                    // A-4, surface TWO. The chat where the escalation landed
                    // sees the SAME card value, carrying the SAME request id —
                    // which is what lets `approval_relay::lookup` route a
                    // decision posted from there back to this parked prompt.
                    //
                    // PUBLISHED, never persisted: it is another conversation's
                    // ask, and writing it into that session's history would put
                    // a foreign tool call in someone else's transcript
                    // permanently. `session_events::publish` is the ephemeral
                    // path and creates no ring of its own — a session nobody is
                    // observing simply drops it, and the card still shows in the
                    // origin's own tab.
                    //
                    // This does NOT touch reconciliation #23's ordering
                    // invariant: that is about the ORIGIN session's stream, and
                    // this publishes to a different session's bus.
                    if let crate::agents::approval_relay::Delegation::AwaitingHuman {
                        surfaced_in,
                    } = &delegation
                    {
                        crate::session_events::publish(
                            surfaced_in,
                            crate::session_events::SessionBusEvent::Agent(
                                crate::agents::AgentEvent::Message(confirmation.clone()),
                            ),
                        );
                    }
                    yield confirmation;
                }

                // The wait is now bounded on three sides: the decision itself, a
                // cancel of the turn (so a programmatic `/agent/cancel` unblocks
                // it — before BR-62 only tearing the SSE socket down did), and a
                // TTL (so a decision that never arrives cannot park the turn
                // forever).
                let wait = await_confirmation(
                    confirmation_rx,
                    cancellation_token.clone(),
                    confirmation_timeout(),
                )
                .await;
                // Answered, expired, or cancelled — either way this id is no
                // longer accepting a decision.
                self.forget_confirmation(&request.id);
                crate::agents::approval_relay::forget(&ask);

                let confirmation = match wait {
                    ConfirmationWait::Confirmed(confirmation) => confirmation,
                    ConfirmationWait::Expired => {
                        tracing::warn!(
                            tool_name = %tool_call.name,
                            request_id = %request.id,
                            "Tool permission prompt expired without a decision"
                        );
                        write_tool_response(request, request_to_response_map, EXPIRED_RESPONSE).await;
                        yield Message::assistant()
                            .with_system_notification(
                                crate::conversation::message::SystemNotificationType::InlineMessage,
                                format!(
                                    "The permission prompt for {} expired before it was answered. The tool was not run.",
                                    tool_call.name
                                ),
                            )
                            .user_only();
                        continue;
                    }
                    ConfirmationWait::Cancelled => {
                        tracing::info!(
                            tool_name = %tool_call.name,
                            request_id = %request.id,
                            "Turn cancelled while awaiting tool permission"
                        );
                        // Answer this prompt *and* every prompt we would have
                        // shown after it, so the cancelled turn still leaves a
                        // well-formed conversation (a tool request with no
                        // response breaks the next provider call).
                        for pending in &tool_requests[idx..] {
                            write_tool_response(pending, request_to_response_map, CANCELLED_RESPONSE).await;
                        }
                        break;
                    }
                };

                // Log user decision if this was a security-inspector finding
                if let Some(finding_id) = get_security_finding_id_from_results(&request.id, inspection_results) {
                    tracing::info!(
                        counter.biorouter.security_finding_user_decisions = 1,
                        decision = ?confirmation.permission,
                        finding_id = %finding_id,
                        "User security decision"
                    );
                }

                if confirmation.permission == Permission::AllowOnce || confirmation.permission == Permission::AlwaysAllow {
                    let (req_id, tool_result) = self.dispatch_tool_call(tool_call.clone(), request.id.clone(), cancellation_token.clone(), session).await;
                    let mut futures = tool_futures.lock().await;

                    futures.push((req_id, match tool_result {
                        Ok(result) => tool_stream(
                            result.notification_stream.unwrap_or_else(|| Box::new(stream::empty())),
                            result.result,
                        ),
                        Err(e) => tool_stream(
                            Box::new(stream::empty()),
                            futures::future::ready(Err(e)),
                        ),
                    }));

                    // Update the shared permission manager when user selects "Always Allow"
                    if confirmation.permission == Permission::AlwaysAllow {
                        self.tool_inspection_manager
                            .update_permission_manager(&tool_call.name, PermissionLevel::AlwaysAllow)
                            .await;
                    }
                } else {
                    // User declined - update the specific response message for this request
                    write_tool_response(request, request_to_response_map, DECLINED_RESPONSE).await;

                    if confirmation.permission == Permission::AlwaysDeny {
                        self.tool_inspection_manager
                            .update_permission_manager(&tool_call.name, PermissionLevel::NeverAllow)
                            .await;
                    }
                }
            }
        }
    }.boxed()
    }

    pub(crate) fn handle_frontend_tool_request<'a>(
        &'a self,
        tool_request: &'a ToolRequest,
        message_tool_response: Arc<Mutex<Message>>,
    ) -> BoxStream<'a, anyhow::Result<Message>> {
        try_stream! {
                if let Ok(tool_call) = tool_request.tool_call.clone() {
                    if self.is_frontend_tool(&tool_call.name).await {
                        // Send frontend tool request and wait for response
                        yield Message::assistant().with_frontend_tool_request(
                            tool_request.id.clone(),
                            Ok(tool_call.clone())
                        );

                        if let Some((id, result)) = self.tool_result_rx.lock().await.recv().await {
                            let mut response = message_tool_response.lock().await;
                            *response = response.clone().with_tool_response_with_metadata(
                                id,
                                result,
                                tool_request.metadata.as_ref(),
                            );
                        }
                    }
            }
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::permission_confirmation::PrincipalType;

    fn allow_once() -> PermissionConfirmation {
        PermissionConfirmation {
            principal_type: PrincipalType::Tool,
            permission: Permission::AllowOnce,
        }
    }

    #[test]
    fn timeout_defaults_when_unset_or_garbage() {
        assert_eq!(
            parse_confirmation_timeout(None),
            Some(Duration::from_secs(DEFAULT_CONFIRMATION_TIMEOUT_SECS))
        );
        assert_eq!(
            parse_confirmation_timeout(Some("not-a-number".into())),
            Some(Duration::from_secs(DEFAULT_CONFIRMATION_TIMEOUT_SECS))
        );
    }

    #[test]
    fn timeout_is_configurable_and_zero_disables_it() {
        assert_eq!(
            parse_confirmation_timeout(Some("90".into())),
            Some(Duration::from_secs(90))
        );
        // 0 opts back into the pre-BR-62 "wait forever" behaviour.
        assert_eq!(parse_confirmation_timeout(Some("0".into())), None);
    }

    #[tokio::test]
    async fn a_decision_resolves_the_wait() {
        let (tx, rx) = oneshot::channel();
        tx.send(allow_once()).unwrap();

        let wait = await_confirmation(rx, Some(CancellationToken::new()), None).await;

        assert!(matches!(
            wait,
            ConfirmationWait::Confirmed(c) if c.permission == Permission::AllowOnce
        ));
    }

    /// The BR-62 headline: a cancel now unblocks a prompt the turn is parked on.
    /// Before, the only way out of the wait was a decision, so a programmatic
    /// cancel left the turn (and its session turn lock) hung forever.
    #[tokio::test]
    async fn cancellation_unblocks_the_wait() {
        let (_tx, rx) = oneshot::channel();
        let token = CancellationToken::new();
        token.cancel();

        let wait = await_confirmation(rx, Some(token), None).await;

        assert!(matches!(wait, ConfirmationWait::Cancelled));
    }

    #[tokio::test]
    async fn ttl_expires_the_wait() {
        // Sender held alive, so only the TTL can end this wait.
        let (_tx, rx) = oneshot::channel();

        let wait = await_confirmation(
            rx,
            Some(CancellationToken::new()),
            Some(Duration::from_millis(10)),
        )
        .await;

        assert!(matches!(wait, ConfirmationWait::Expired));
    }

    /// A prompt dropped without a decision must not park the loop on a channel
    /// nobody holds.
    #[tokio::test]
    async fn a_dropped_prompt_expires_the_wait() {
        let (tx, rx) = oneshot::channel::<PermissionConfirmation>();
        drop(tx);

        let wait = await_confirmation(rx, Some(CancellationToken::new()), None).await;

        assert!(matches!(wait, ConfirmationWait::Expired));
    }

    /// A decision that landed is honored even if the turn is cancelled in the
    /// same instant — the user did answer, and the `biased` select respects that.
    #[tokio::test]
    async fn a_landed_decision_beats_a_simultaneous_cancel() {
        let (tx, rx) = oneshot::channel();
        tx.send(allow_once()).unwrap();
        let token = CancellationToken::new();
        token.cancel();

        let wait = await_confirmation(rx, Some(token), Some(Duration::from_millis(0))).await;

        assert!(matches!(wait, ConfirmationWait::Confirmed(_)));
    }

    /// With no cancellation token and no TTL the wait must still be driven purely
    /// by the decision — neither dummy arm may fire.
    #[tokio::test]
    async fn no_token_and_no_ttl_waits_for_the_decision() {
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let _ = tx.send(allow_once());
        });

        let wait = await_confirmation(rx, None, None).await;

        assert!(matches!(wait, ConfirmationWait::Confirmed(_)));
    }
}
