use anyhow::Result;
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};

use crate::config::BioRouterMode;
use crate::conversation::message::{Message, ToolRequest};
use crate::permission::permission_inspector::PermissionInspector;
use crate::permission::permission_judge::PermissionCheckResult;
use crate::privacy::CallCapability;
use crate::session::Session;

/// Result of inspecting a tool call
#[derive(Debug, Clone)]
pub struct InspectionResult {
    pub tool_request_id: String,
    pub action: InspectionAction,
    pub reason: String,
    pub confidence: f32,
    pub inspector_name: String,
    pub finding_id: Option<String>,
}

/// Action to take based on inspection result
#[derive(Debug, Clone, PartialEq)]
pub enum InspectionAction {
    /// Allow the tool to execute without user intervention
    Allow,
    /// Deny the tool execution completely
    Deny,
    /// Require user approval before execution (with optional warning message)
    RequireApproval(Option<String>),
    /// Advisory only (BR-29 soft stage): the tool still runs and the permission
    /// verdict is untouched, but `reason` is injected into the model's context so
    /// it can correct course before a later hard stop.
    Warn,
}

/// Trait for all tool inspectors
#[async_trait]
pub trait ToolInspector: Send + Sync {
    /// Name of this inspector (for logging/debugging)
    fn name(&self) -> &'static str;

    /// Inspect tool requests and return results
    async fn inspect(
        &self,
        tool_requests: &[ToolRequest],
        messages: &[Message],
        biorouter_mode: BioRouterMode,
        session: &Session,
    ) -> Result<Vec<InspectionResult>>;

    async fn inspect_with_capability(
        &self,
        tool_requests: &[ToolRequest],
        messages: &[Message],
        biorouter_mode: BioRouterMode,
        session: &Session,
        _capability: Option<CallCapability>,
    ) -> Result<Vec<InspectionResult>> {
        // Most inspectors do not depend on provider privacy. The workspace
        // crossing inspector overrides this so bridge calls can retain the
        // capability pinned when their grant was issued.
        self.inspect(tool_requests, messages, biorouter_mode, session)
            .await
    }

    /// Whether this inspector is enabled
    fn is_enabled(&self) -> bool {
        true
    }

    /// Allow downcasting to concrete types
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Manages all tool inspectors and coordinates their results
pub struct ToolInspectionManager {
    inspectors: Vec<Box<dyn ToolInspector>>,
}

impl ToolInspectionManager {
    pub fn new() -> Self {
        Self {
            inspectors: Vec::new(),
        }
    }

    /// Add an inspector to the manager
    /// Inspectors run in the order they are added
    pub fn add_inspector(&mut self, inspector: Box<dyn ToolInspector>) {
        self.inspectors.push(inspector);
    }

    /// Run all inspectors on the tool requests
    pub async fn inspect_tools(
        &self,
        tool_requests: &[ToolRequest],
        messages: &[Message],
        biorouter_mode: BioRouterMode,
        session: &Session,
    ) -> Result<Vec<InspectionResult>> {
        self.inspect_tools_with_capability(tool_requests, messages, biorouter_mode, session, None)
            .await
    }

    pub async fn inspect_tools_with_capability(
        &self,
        tool_requests: &[ToolRequest],
        messages: &[Message],
        biorouter_mode: BioRouterMode,
        session: &Session,
        capability: Option<CallCapability>,
    ) -> Result<Vec<InspectionResult>> {
        self.inspect_tools_excluding_with_capability(
            &[],
            tool_requests,
            messages,
            biorouter_mode,
            session,
            capability,
        )
        .await
    }

    /// Run every inspector *except* the named ones.
    ///
    /// BR-19: used to re-validate a tool call whose input a PreToolUse hook
    /// rewrote. The rewrite must not be a way around the security/permission
    /// gates, so the other inspectors run again on the new arguments — but the
    /// hook inspector is excluded, because re-running it would execute the
    /// user's hook commands a second time (side effects) and let a rewrite
    /// trigger another rewrite.
    pub async fn inspect_tools_excluding(
        &self,
        excluded: &[&str],
        tool_requests: &[ToolRequest],
        messages: &[Message],
        biorouter_mode: BioRouterMode,
        session: &Session,
    ) -> Result<Vec<InspectionResult>> {
        self.inspect_tools_excluding_with_capability(
            excluded,
            tool_requests,
            messages,
            biorouter_mode,
            session,
            None,
        )
        .await
    }

    pub async fn inspect_tools_excluding_with_capability(
        &self,
        excluded: &[&str],
        tool_requests: &[ToolRequest],
        messages: &[Message],
        biorouter_mode: BioRouterMode,
        session: &Session,
        capability: Option<CallCapability>,
    ) -> Result<Vec<InspectionResult>> {
        self.run_inspectors(
            excluded,
            tool_requests,
            messages,
            biorouter_mode,
            session,
            capability,
            None,
        )
        .await
    }

    /// Inspect the tool calls a Code Execution script makes (QA finding F7).
    ///
    /// The same inspectors, in the same order, with the same escalation-only
    /// merge downstream, as a direct call — with two differences, both forced:
    ///
    /// * `capability` is required, not optional. A script's calls inherit the
    ///   capability its `execute_code` call was ADMITTED on (issue #56), so no
    ///   inspector on this path may sample one of its own.
    /// * The permission inspector reads each tool's BR-18 risk grade out of
    ///   `risks` — the script's own catalogue — rather than the registry the
    ///   agent refreshes from the model's roster, which in Code Execution mode
    ///   has never graded the tools a script can reach. See
    ///   [`PermissionInspector::inspect_graded`].
    ///
    /// `excluded` names inspectors that must not judge a script's call; the
    /// caller states them rather than this function assuming them.
    #[allow(clippy::too_many_arguments)]
    pub async fn inspect_script_calls(
        &self,
        excluded: &[&str],
        tool_requests: &[ToolRequest],
        messages: &[Message],
        biorouter_mode: BioRouterMode,
        session: &Session,
        capability: CallCapability,
        risks: &crate::permission::tool_risk::ToolRiskRegistry,
    ) -> Result<Vec<InspectionResult>> {
        self.run_inspectors(
            excluded,
            tool_requests,
            messages,
            biorouter_mode,
            session,
            Some(capability),
            Some(risks),
        )
        .await
    }

    /// The one loop over the inspectors. `risks` overrides where the permission
    /// inspector reads risk grades from; `None` is its own registry.
    #[allow(clippy::too_many_arguments)]
    async fn run_inspectors(
        &self,
        excluded: &[&str],
        tool_requests: &[ToolRequest],
        messages: &[Message],
        biorouter_mode: BioRouterMode,
        session: &Session,
        capability: Option<CallCapability>,
        risks: Option<&crate::permission::tool_risk::ToolRiskRegistry>,
    ) -> Result<Vec<InspectionResult>> {
        let mut all_results = Vec::new();

        for inspector in &self.inspectors {
            if !inspector.is_enabled() || excluded.contains(&inspector.name()) {
                continue;
            }

            tracing::debug!(
                inspector_name = inspector.name(),
                tool_count = tool_requests.len(),
                "Running tool inspector"
            );

            let graded = risks.and_then(|risks| {
                inspector
                    .as_any()
                    .downcast_ref::<PermissionInspector>()
                    .map(|permission| (permission, risks))
            });
            let outcome = match graded {
                Some((permission, risks)) => {
                    permission
                        .inspect_graded(tool_requests, biorouter_mode, session, risks)
                        .await
                }
                None => {
                    inspector
                        .inspect_with_capability(
                            tool_requests,
                            messages,
                            biorouter_mode,
                            session,
                            capability,
                        )
                        .await
                }
            };
            match outcome {
                Ok(results) => {
                    tracing::debug!(
                        inspector_name = inspector.name(),
                        result_count = results.len(),
                        "Tool inspector completed"
                    );
                    all_results.extend(results);
                }
                Err(e) => {
                    tracing::error!(
                        inspector_name = inspector.name(),
                        error = %e,
                        "Tool inspector failed"
                    );
                    // Continue with other inspectors even if one fails
                }
            }
        }

        Ok(all_results)
    }

    /// Get list of registered inspector names
    pub fn inspector_names(&self) -> Vec<&'static str> {
        self.inspectors.iter().map(|i| i.name()).collect()
    }

    /// Update the permission manager for a specific tool
    pub async fn update_permission_manager(
        &self,
        tool_name: &str,
        permission_level: crate::config::permission::PermissionLevel,
    ) {
        for inspector in &self.inspectors {
            if inspector.name() == "permission" {
                // Downcast to PermissionInspector to access permission manager
                if let Some(permission_inspector) =
                    inspector.as_any().downcast_ref::<PermissionInspector>()
                {
                    permission_inspector
                        .permission_manager
                        .update_user_permission(tool_name, permission_level);
                    return;
                }
            }
        }
        tracing::warn!("Permission inspector not found for permission manager update");
    }

    /// Process inspection results using the permission inspector
    /// This delegates to the permission inspector's process_inspection_results method
    pub fn process_inspection_results_with_permission_inspector(
        &self,
        remaining_requests: &[ToolRequest],
        inspection_results: &[InspectionResult],
    ) -> Option<PermissionCheckResult> {
        for inspector in &self.inspectors {
            if inspector.name() == "permission" {
                if let Some(permission_inspector) =
                    inspector.as_any().downcast_ref::<PermissionInspector>()
                {
                    return Some(
                        permission_inspector
                            .process_inspection_results(remaining_requests, inspection_results),
                    );
                }
            }
        }
        tracing::warn!("Permission inspector not found for processing inspection results");
        None
    }
}

impl Default for ToolInspectionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Where an inspection result sits in the escalation lattice.
///
/// `Deny > RequireApproval > Allow`, with `Allow` (and the advisory `Warn`)
/// carrying no escalation at all — they are the identity, which is what makes
/// the merge escalation-only: an inspector can raise a verdict, never lower one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Escalation {
    /// [`InspectionAction::RequireApproval`] — route through the user.
    Ask,
    /// [`InspectionAction::Deny`] — refuse, and keep refusing.
    Deny,
}

impl Escalation {
    /// `None` for the actions that never move a request: `Allow` defers to
    /// whatever else decided, and `Warn` is advisory (see
    /// [`collect_warning_reasons`]).
    fn of(action: &InspectionAction) -> Option<Self> {
        match action {
            InspectionAction::Deny => Some(Self::Deny),
            InspectionAction::RequireApproval(_) => Some(Self::Ask),
            InspectionAction::Allow | InspectionAction::Warn => None,
        }
    }
}

/// Apply inspection results to permission check results.
///
/// This is the generic permission-mixing logic that works for all inspector
/// types. Two properties it must hold, both learned the hard way:
///
/// * **Escalation-only.** An inspector's `Allow` never lowers another's `Deny`
///   or `RequireApproval`, which is what lets a security gate survive `Auto`
///   mode's blanket allow, a user `AlwaysAllow`, and a `SmartApprove` read-only
///   grade.
/// * **A partition, computed order-independently.** Every request ends in
///   exactly one of `approved` / `needs_approval` / `denied`, and which
///   inspector ran first cannot change which one. Registration order in
///   [`crate::agents::Agent`] is a readability choice; a merge that depended on
///   it would be making security decisions by accident.
///
/// The second property is not bookkeeping. Downstream, a request in `denied`
/// gets a refusal written as its tool response, and a request in
/// `needs_approval` gets an approval card whose "Allow once" *dispatches the
/// tool* and overwrites that response. A call in both lists is therefore a call
/// whose denial one click undoes — so a later inspector's ask could reopen an
/// earlier inspector's (or the user's own) refusal.
pub fn apply_inspection_results_to_permissions(
    mut permission_result: PermissionCheckResult,
    inspection_results: &[InspectionResult],
) -> PermissionCheckResult {
    if inspection_results.is_empty() {
        return permission_result;
    }

    // Create a map of tool requests by ID for easy lookup
    let mut all_requests: HashMap<String, ToolRequest> = HashMap::new();

    // Collect all tool requests
    for req in &permission_result.approved {
        all_requests.insert(req.id.clone(), req.clone());
    }
    for req in &permission_result.needs_approval {
        all_requests.insert(req.id.clone(), req.clone());
    }
    for req in &permission_result.denied {
        all_requests.insert(req.id.clone(), req.clone());
    }

    // Fold every inspector's verdict for a request into the single strongest
    // one *before* touching the result sets. Doing it in one pass over the
    // results instead would make the outcome depend on the order they arrived
    // in: a Deny seen after an Ask removes the request from `needs_approval`,
    // but a Deny seen before one used to be quietly re-queued for approval.
    let mut strongest: HashMap<&str, Escalation> = HashMap::new();
    for result in inspection_results {
        let Some(level) = Escalation::of(&result.action) else {
            continue;
        };
        let slot = strongest
            .entry(result.tool_request_id.as_str())
            .or_insert(level);
        *slot = (*slot).max(level);
    }

    for result in inspection_results {
        tracing::info!(
            inspector_name = result.inspector_name,
            tool_request_id = %result.tool_request_id,
            action = ?result.action,
            confidence = result.confidence,
            reason = %result.reason,
            finding_id = ?result.finding_id,
            applied = ?strongest.get(result.tool_request_id.as_str()),
            "Applying inspection result"
        );
    }

    // Apply each request's folded verdict once, walking the results in order so
    // the resulting vectors stay deterministic (a `HashMap` iteration would not).
    let mut applied: HashSet<&str> = HashSet::new();
    for result in inspection_results {
        let request_id = result.tool_request_id.as_str();
        let Some(level) = strongest.get(request_id).copied() else {
            continue;
        };
        if !applied.insert(request_id) {
            continue;
        }
        let Some(request) = all_requests.get(request_id) else {
            // A verdict about a request that is not in this batch at all.
            continue;
        };

        match level {
            Escalation::Deny => {
                permission_result
                    .approved
                    .retain(|req| req.id != request_id);
                permission_result
                    .needs_approval
                    .retain(|req| req.id != request_id);
                if !permission_result.denied.iter().any(|r| r.id == request_id) {
                    permission_result.denied.push(request.clone());
                }
            }
            Escalation::Ask => {
                permission_result
                    .approved
                    .retain(|req| req.id != request_id);
                // A denial already on the books — the permission baseline's (a
                // user `NeverAllow`, a managed force-deny) or another
                // inspector's — outranks an ask. Never re-queue it for a card.
                if permission_result.denied.iter().any(|r| r.id == request_id) {
                    permission_result
                        .needs_approval
                        .retain(|req| req.id != request_id);
                    continue;
                }
                if !permission_result
                    .needs_approval
                    .iter()
                    .any(|r| r.id == request_id)
                {
                    permission_result.needs_approval.push(request.clone());
                }
            }
        }
    }

    permission_result
}

/// How prominently an inspector's approval explanation is shown on the card.
/// Lower sorts first.
fn approval_message_rank(inspector_name: &str) -> u8 {
    if inspector_name == crate::security::global_memory::GLOBAL_MEMORY_INSPECTOR_NAME {
        // The only explanation that names *what data is about to cross a
        // boundary*, and which category. It leads.
        0
    } else if NON_DELEGABLE_APPROVAL_INSPECTORS.contains(&inspector_name) {
        // "Policy requires approval", "flagged as potentially dangerous" — still
        // security-raised, still above ordinary prompt text.
        1
    } else {
        2
    }
}

/// The explanation shown on a tool's approval card: **every** distinct reason an
/// inspector gave for escalating this call, most consequential first, blank-line
/// separated. `None` when nobody explained anything — an ordinary permission
/// prompt the card renders on its own.
///
/// This used to take the first inspection result for the request and read a
/// message off it *if that one happened to have one*, which failed two ways at
/// once: a second inspector's explanation was dropped, and any earlier
/// result without a message (an `Allow`, a bare `RequireApproval(None)`, an
/// advisory `Warn`) swallowed the explanation behind it and left the user an
/// unexplained card. Both silently hid the issue-#63 cross-session disclosure
/// text — the part that names the category and says the store is machine-wide —
/// behind, say, a managed-policy ask.
pub fn approval_prompt_for_request(
    tool_request_id: &str,
    inspection_results: &[InspectionResult],
) -> Option<String> {
    let mut messages: Vec<(u8, &str)> = Vec::new();
    for result in inspection_results {
        if result.tool_request_id != tool_request_id {
            continue;
        }
        let InspectionAction::RequireApproval(Some(message)) = &result.action else {
            continue;
        };
        let message = message.trim();
        if message.is_empty() || messages.iter().any(|(_, seen)| *seen == message) {
            continue;
        }
        messages.push((approval_message_rank(&result.inspector_name), message));
    }
    if messages.is_empty() {
        return None;
    }
    // Stable, so inspectors sharing a rank keep their registration order.
    messages.sort_by_key(|(rank, _)| *rank);

    Some(
        messages
            .into_iter()
            .map(|(_, message)| message)
            .collect::<Vec<_>>()
            .join("\n\n"),
    )
}

/// Collect the advisory (`InspectionAction::Warn`) reasons from a round of
/// inspection, in inspector order and de-duplicated.
///
/// These are the soft-stage nudges — a loop guard telling the model it is
/// repeating itself, for instance. They do not change any permission verdict;
/// the agent injects them into the model's context for the next turn.
pub fn collect_warning_reasons(inspection_results: &[InspectionResult]) -> Vec<String> {
    let mut reasons: Vec<String> = Vec::new();
    for result in inspection_results {
        if result.action != InspectionAction::Warn {
            continue;
        }
        let reason = result.reason.trim();
        if reason.is_empty() || reasons.iter().any(|existing| existing == reason) {
            continue;
        }
        reasons.push(reason.to_string());
    }
    reasons
}

/// Frame soft-stage warnings as a single system-authored guidance block. Unlike
/// hook output (untrusted, third-party text), these strings are authored by
/// Biorouter itself, so they are presented as first-party guidance.
pub fn frame_loop_warnings(reasons: &[String]) -> String {
    format!(
        "<biorouter-loop-guard>\n{}\n</biorouter-loop-guard>",
        reasons.join("\n")
    )
}

pub fn get_security_finding_id_from_results(
    tool_request_id: &str,
    inspection_results: &[InspectionResult],
) -> Option<String> {
    inspection_results
        .iter()
        .find(|result| {
            result.tool_request_id == tool_request_id
                && result.inspector_name
                    == crate::security::security_inspector::SECURITY_INSPECTOR_NAME
        })
        .and_then(|result| result.finding_id.clone())
}

/// Inspectors whose `RequireApproval` is a question **for the human** rather
/// than a policy decision an automation may stand in for.
///
/// A `PermissionRequest` hook is a convenience: the user writes it once, in
/// advance, so routine prompts stop interrupting them. That is a reasonable
/// thing to delegate — but only for approvals the *permission mode* raised.
/// These inspectors escalate because something about this specific call is
/// dangerous or discloses data the user has not seen, and each of them exists
/// precisely because automated grants (Auto mode, `AlwaysAllow`, a SmartApprove
/// read-only grade, an org policy) must not decide it. A hook is one more
/// automated grant, and it runs *after* the escalation-only merge, where the
/// lattice can no longer protect the verdict.
///
/// Deny is unaffected in both directions: a hook may always deny, and these
/// inspectors' own `Deny` never reaches this path at all.
pub const NON_DELEGABLE_APPROVAL_INSPECTORS: &[&str] = &[
    // Issue #63 — cross-session (machine-wide) memory disclosure.
    crate::security::global_memory::GLOBAL_MEMORY_INSPECTOR_NAME,
    // The command policy engine's `ask` (BR-21).
    crate::security::security_inspector::SECURITY_INSPECTOR_NAME,
    // Auto-mode escalation of writes to SSH keys, keychains, system dirs, …
    crate::security::sensitive_ops::SENSITIVE_OPS_INSPECTOR_NAME,
    // A trusted admin's policy; a project-local hook is not the admin.
    crate::permission::managed_inspector::MANAGED_INSPECTOR_NAME,
    // Permanent destruction of a knowledge base — its pages, its raw sources
    // and its whole git history. A hook that could answer this is a hook that
    // can delete the user's curated knowledge without them seeing it, and
    // unlike an extension a knowledge base cannot be reinstalled.
    crate::security::knowledge_delete::KNOWLEDGE_DELETE_INSPECTOR_NAME,
];

/// Whether this tool request's approval must be answered by the user in person.
///
/// True when any inspector in [`NON_DELEGABLE_APPROVAL_INSPECTORS`] asked for
/// approval on it. The caller ([`crate::agents::Agent::handle_approval_tool_requests`])
/// uses this to ignore a `PermissionRequest` hook's `allow` and show the card
/// anyway.
pub fn approval_requires_a_human(
    tool_request_id: &str,
    inspection_results: &[InspectionResult],
) -> bool {
    inspection_results.iter().any(|result| {
        result.tool_request_id == tool_request_id
            && matches!(result.action, InspectionAction::RequireApproval(_))
            && NON_DELEGABLE_APPROVAL_INSPECTORS.contains(&result.inspector_name.as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::ToolRequest;
    use rmcp::model::CallToolRequestParams;
    use rmcp::object;

    fn request(id: &str) -> ToolRequest {
        ToolRequest {
            id: id.to_string(),
            tool_call: Ok(CallToolRequestParams {
                task: None,
                name: "test_tool".into(),
                arguments: Some(object!({})),
                meta: None,
            }),
            metadata: None,
            tool_meta: None,
        }
    }

    fn result(id: &str, inspector: &str, action: InspectionAction) -> InspectionResult {
        InspectionResult {
            tool_request_id: id.to_string(),
            action,
            reason: format!("{inspector} said so"),
            confidence: 1.0,
            inspector_name: inspector.to_string(),
            finding_id: None,
        }
    }

    fn approved_baseline(id: &str) -> PermissionCheckResult {
        PermissionCheckResult {
            approved: vec![request(id)],
            needs_approval: vec![],
            denied: vec![],
        }
    }

    /// Every request appears in exactly one of the three sets. Nothing downstream
    /// re-checks this: `handle_approved_and_denied_tools` writes the denial and
    /// then `handle_approval_tool_requests` shows a card for the *same* id, whose
    /// AllowOnce dispatches the call and overwrites the denial with a real result.
    fn assert_partition(result: &PermissionCheckResult, context: &str) {
        for req in result
            .approved
            .iter()
            .chain(&result.needs_approval)
            .chain(&result.denied)
        {
            let memberships = [
                result.approved.iter().any(|r| r.id == req.id),
                result.needs_approval.iter().any(|r| r.id == req.id),
                result.denied.iter().any(|r| r.id == req.id),
            ]
            .iter()
            .filter(|present| **present)
            .count();
            assert_eq!(
                memberships, 1,
                "{context}: {} is in {memberships} result sets; the three sets must partition \
                 the batch",
                req.id
            );
        }
    }

    #[test]
    fn test_apply_inspection_results() {
        let inspection_results = vec![InspectionResult {
            tool_request_id: "req_1".to_string(),
            action: InspectionAction::Deny,
            reason: "Test denial".to_string(),
            confidence: 0.9,
            inspector_name: "test_inspector".to_string(),
            finding_id: Some("TEST-001".to_string()),
        }];

        let updated_result = apply_inspection_results_to_permissions(
            approved_baseline("req_1"),
            &inspection_results,
        );

        assert_eq!(updated_result.approved.len(), 0);
        assert_eq!(updated_result.denied.len(), 1);
        assert_eq!(updated_result.denied[0].id, "req_1");
    }

    /// The merge is a **lattice**, not a sequence: `Deny > RequireApproval >
    /// Allow`, and which inspector happened to run first must not change the
    /// outcome. Registration order is a readability choice in
    /// `Agent::create_tool_inspection_manager`, so a merge that depends on it is
    /// a security decision made by accident.
    ///
    /// The failure this pins is not cosmetic. A call left in *both* `denied` and
    /// `needs_approval` gets its denial written first and an approval card second
    /// — and "AllowOnce" on that card dispatches the tool, overwriting the denial.
    /// A second inspector's ask therefore *undoes* the first inspector's refusal.
    #[test]
    fn a_deny_and_an_ask_never_coexist_whichever_ran_first() {
        let deny = result("req_1", "security", InspectionAction::Deny);
        let ask = result(
            "req_1",
            "global_memory",
            InspectionAction::RequireApproval(Some("cross-session memory".into())),
        );

        for (order, results) in [
            ("deny then ask", vec![deny.clone(), ask.clone()]),
            ("ask then deny", vec![ask, deny]),
        ] {
            let merged =
                apply_inspection_results_to_permissions(approved_baseline("req_1"), &results);

            assert_partition(&merged, order);
            assert_eq!(merged.denied.len(), 1, "{order}: the deny must stand");
            assert!(
                merged.needs_approval.is_empty(),
                "{order}: a denied call must never also be queued for an approval \
                 card; the card's AllowOnce dispatches it"
            );
            assert!(merged.approved.is_empty(), "{order}");
        }
    }

    /// A user `NeverAllow`, and a managed force-deny in a gating mode, reach the
    /// merge as the **baseline** `denied` set — before any inspection result is
    /// applied. An escalation to `RequireApproval` afterwards must respect it, not
    /// hand the user a card that turns their own "never" into a "once".
    #[test]
    fn an_ask_cannot_revive_a_call_the_baseline_already_denied() {
        let baseline = PermissionCheckResult {
            approved: vec![],
            needs_approval: vec![],
            denied: vec![request("req_1")],
        };
        let results = vec![result(
            "req_1",
            "global_memory",
            InspectionAction::RequireApproval(Some("cross-session memory".into())),
        )];

        let merged = apply_inspection_results_to_permissions(baseline, &results);

        assert_partition(&merged, "baseline deny + later ask");
        assert_eq!(merged.denied.len(), 1);
        assert!(
            merged.needs_approval.is_empty(),
            "an inspector's ask must not resurrect a call the permission baseline denied"
        );
    }

    // --- who may answer an approval ---------------------------------------

    /// Every inspector that escalates *because the call itself is dangerous or
    /// discloses unseen data* raises an approval only a human may answer. Each
    /// of these exists to stop an automated grant from deciding the call; a
    /// `PermissionRequest` hook is one more automated grant.
    #[test]
    fn a_security_raised_approval_is_not_delegable() {
        for inspector in NON_DELEGABLE_APPROVAL_INSPECTORS {
            let results = vec![result(
                "req_1",
                inspector,
                InspectionAction::RequireApproval(Some("because".into())),
            )];
            assert!(
                approval_requires_a_human("req_1", &results),
                "{inspector}'s approval must reach the user in person"
            );
        }
    }

    /// The bound is narrow on purpose: an ordinary permission-mode prompt is
    /// exactly what a `PermissionRequest` hook is for, and a hook that can no
    /// longer answer one is a broken feature, not a safer one.
    #[test]
    fn an_ordinary_permission_prompt_stays_delegable() {
        for inspector in ["permission", "hooks", "repetition"] {
            let results = vec![result(
                "req_1",
                inspector,
                InspectionAction::RequireApproval(None),
            )];
            assert!(
                !approval_requires_a_human("req_1", &results),
                "{inspector} raises ordinary approvals; a hook may still answer them"
            );
        }
        assert!(
            !approval_requires_a_human("req_1", &[]),
            "no inspection result at all is an ordinary prompt"
        );
    }

    /// The predicate is per-request and per-action: a security inspector that
    /// *allowed* this call, or that asked about a **different** one, must not
    /// make this approval non-delegable.
    #[test]
    fn only_this_requests_own_security_ask_counts() {
        let results = vec![
            result("req_1", "security", InspectionAction::Allow),
            result(
                "req_2",
                "global_memory",
                InspectionAction::RequireApproval(Some("other call".into())),
            ),
        ];
        assert!(!approval_requires_a_human("req_1", &results));
        assert!(approval_requires_a_human("req_2", &results));
    }

    // --- what the card says ------------------------------------------------

    const MEMORY_CARD: &str =
        "🔒 Cross-session memory read.\nThe global memory category \"clinical\"…";

    fn ask(id: &str, inspector: &str, message: &str) -> InspectionResult {
        result(
            id,
            inspector,
            InspectionAction::RequireApproval(Some(message.to_string())),
        )
    }

    /// Two inspectors can escalate the same call for two different reasons. The
    /// card must carry both — dropping one leaves the user consenting to a
    /// question they were never shown.
    #[test]
    fn every_approval_reason_reaches_the_card() {
        let results = vec![
            ask("req_1", "managed", "Your organization requires approval."),
            ask("req_1", "global_memory", MEMORY_CARD),
        ];

        let prompt = approval_prompt_for_request("req_1", &results).expect("a card explanation");

        assert!(
            prompt.contains("Your organization requires approval."),
            "the managed reason must survive: {prompt}"
        );
        assert!(
            prompt.contains("clinical"),
            "the cross-session disclosure explanation must survive: {prompt}"
        );
    }

    /// …and the disclosure comes first. It is the only one of these that names
    /// *what data is about to cross a boundary*; the others say "policy requires
    /// approval", which the user can act on without reading it twice.
    #[test]
    fn the_cross_session_disclosure_is_never_buried() {
        let results = vec![
            ask("req_1", "managed", "Your organization requires approval."),
            ask("req_1", "security", "🔒 Security Alert: flagged."),
            ask("req_1", "global_memory", MEMORY_CARD),
        ];

        let prompt = approval_prompt_for_request("req_1", &results).expect("a card explanation");

        assert!(
            prompt.starts_with("🔒 Cross-session memory read."),
            "the memory disclosure must lead the card: {prompt}"
        );
    }

    /// The sharpest form of the bug this replaces: selection used to take the
    /// *first result for the request* and then ask whether that one carried a
    /// message. Any earlier result without one — an `Allow`, a bare
    /// `RequireApproval(None)`, an advisory `Warn` — swallowed every explanation
    /// behind it, and the user got an unexplained card.
    #[test]
    fn an_earlier_messageless_result_cannot_swallow_the_explanation() {
        for leading in [
            InspectionAction::Allow,
            InspectionAction::RequireApproval(None),
            InspectionAction::Warn,
        ] {
            let results = vec![
                result("req_1", "permission", leading.clone()),
                ask("req_1", "global_memory", MEMORY_CARD),
            ];

            let prompt = approval_prompt_for_request("req_1", &results);

            assert_eq!(
                prompt.as_deref(),
                Some(MEMORY_CARD),
                "a leading {leading:?} must not hide the explanation behind it"
            );
        }
    }

    /// Reasons are per request, de-duplicated, and absent when nobody explained
    /// anything (a bare `RequireApproval(None)` is the ordinary permission
    /// prompt, which the card renders on its own).
    #[test]
    fn approval_reasons_are_scoped_and_deduplicated() {
        let results = vec![
            ask("req_1", "global_memory", MEMORY_CARD),
            ask("req_1", "sensitive_ops", MEMORY_CARD),
            ask("req_2", "managed", "a different call entirely"),
        ];

        let prompt = approval_prompt_for_request("req_1", &results).expect("a card explanation");
        assert_eq!(prompt, MEMORY_CARD, "an identical reason is not repeated");
        assert!(
            !prompt.contains("a different call entirely"),
            "another request's reason must not leak onto this card: {prompt}"
        );

        assert_eq!(
            approval_prompt_for_request(
                "req_3",
                &[result(
                    "req_3",
                    "permission",
                    InspectionAction::RequireApproval(None)
                )]
            ),
            None
        );
    }

    /// Three inspectors disagreeing about one call still resolve to the top of the
    /// lattice, and the other requests in the batch are untouched.
    #[test]
    fn the_strongest_verdict_wins_across_a_whole_batch() {
        let baseline = PermissionCheckResult {
            approved: vec![request("req_1"), request("req_2")],
            needs_approval: vec![],
            denied: vec![],
        };
        let results = vec![
            result("req_1", "sensitive_ops", InspectionAction::Allow),
            result(
                "req_1",
                "global_memory",
                InspectionAction::RequireApproval(None),
            ),
            result("req_1", "managed", InspectionAction::Deny),
            result("req_2", "hooks", InspectionAction::Warn),
        ];

        let merged = apply_inspection_results_to_permissions(baseline, &results);

        assert_partition(&merged, "mixed batch");
        assert_eq!(merged.denied.len(), 1);
        assert_eq!(merged.denied[0].id, "req_1");
        assert_eq!(
            merged.approved.len(),
            1,
            "an advisory Warn is not a permission verdict; req_2 stays approved"
        );
        assert_eq!(merged.approved[0].id, "req_2");
    }
}
