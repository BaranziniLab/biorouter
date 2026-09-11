use crate::agents::extension_manager_extension::MANAGE_EXTENSIONS_TOOL_NAME_COMPLETE;
use crate::agents::types::SharedProvider;
use crate::config::permission::PermissionLevel;
use crate::config::{BioRouterMode, Config, PermissionManager};
use crate::conversation::message::{Message, ToolRequest};
use crate::managed::{ManagedPolicy, ManagedVerdict};
use crate::permission::managed_inspector::{MANAGED_ASK_REASON, MANAGED_DENY_REASON};
use crate::permission::permission_judge::{detect_read_only_tools, PermissionCheckResult};
use crate::permission::permission_scope::CallFacts;
use crate::permission::permission_store::ToolPermissionStore;
use crate::permission::tool_risk::{SmartApproveConfig, ToolRisk, ToolRiskRegistry};
use crate::tool_inspection::{InspectionAction, InspectionResult, ToolInspector};
use anyhow::Result;
use async_trait::async_trait;
use std::path::Path;
use std::sync::{Arc, RwLock};

/// BR-24 kill switch. Scoped grants only ever fire on a grant the user explicitly
/// created, so the default (`true`) changes no behaviour on its own; the flag
/// exists so a deployment can turn the whole tier off without clearing the store.
const SCOPED_GRANTS_PARAM: &str = "PERMISSION_SCOPED_GRANTS";

/// Permission Inspector that handles tool permission checking
pub struct PermissionInspector {
    /// BR-18: annotation-derived per-tool risk grades, refreshed every turn from
    /// the exact tool list handed to the model. Replaces the two permanently
    /// empty `readonly_tools`/`regular_tools` sets that made the read-only
    /// short-circuit unreachable and left SmartApprove == Approve.
    risks: Arc<ToolRiskRegistry>,
    /// BR-18: the smart tier's confirmation policy (risk threshold, fail-closed
    /// handling of un-gradeable tools, and the opt-in LLM judge).
    smart: SmartApproveConfig,
    /// Provider used only by the (opt-in) LLM permission judge. Holds `None`
    /// until the agent installs one — the judge then simply does not run.
    provider: SharedProvider,
    pub permission_manager: Arc<PermissionManager>,
    /// Trusted admin policy consulted *before* the user's own permission table,
    /// so a managed force-allow wins even over a user `NeverAllow` (the one
    /// decision the escalation-only merge cannot express). Inert when absent.
    managed: Arc<ManagedPolicy>,
    /// BR-24: remembered *scoped* grants — "always allow in this directory",
    /// "always allow this command prefix". The middle grades between the store's
    /// exact-args reuse (too narrow to ever fire twice) and a tool-name-wide
    /// `PermissionLevel::AlwaysAllow` (so broad that granting it for `shell`
    /// blesses every future command).
    scoped_store: Arc<RwLock<ToolPermissionStore>>,
    /// Whether the scoped tier is consulted at all ([`SCOPED_GRANTS_PARAM`]).
    scoped_enabled: bool,
}

/// What the deterministic (no-LLM) pass concluded for a single tool request.
enum Verdict {
    /// A final decision, with the reason to surface.
    Decided(InspectionAction, String),
    /// SmartApprove could not grade this tool from its annotations and the LLM
    /// judge is enabled — batch it into one classification round-trip.
    AskJudge,
}

impl PermissionInspector {
    pub fn new(
        risks: Arc<ToolRiskRegistry>,
        permission_manager: Arc<PermissionManager>,
        managed: Arc<ManagedPolicy>,
        provider: SharedProvider,
    ) -> Self {
        Self::with_smart_config(
            risks,
            permission_manager,
            managed,
            provider,
            SmartApproveConfig::from_config(),
        )
    }

    /// Construct with an explicit smart-tier policy, bypassing the process-wide
    /// config. Used by tests and by callers that pin their own policy.
    pub fn with_smart_config(
        risks: Arc<ToolRiskRegistry>,
        permission_manager: Arc<PermissionManager>,
        managed: Arc<ManagedPolicy>,
        provider: SharedProvider,
        smart: SmartApproveConfig,
    ) -> Self {
        // A store we cannot read is an empty store: the tier then grants nothing
        // and every call falls through to the existing prompt.
        let scoped_store = ToolPermissionStore::load().unwrap_or_else(|e| {
            tracing::warn!("Failed to load scoped tool permissions ({e}); starting empty");
            ToolPermissionStore::new()
        });

        Self {
            risks,
            smart,
            provider,
            permission_manager,
            managed,
            scoped_store: Arc::new(RwLock::new(scoped_store)),
            scoped_enabled: Config::global()
                .get_param::<bool>(SCOPED_GRANTS_PARAM)
                .unwrap_or(true),
        }
    }

    /// Pin the scoped-grant store (and the tier's kill switch), bypassing both
    /// the on-disk store and the process-wide config. For tests and for
    /// embedders that keep permissions outside the user's config dir.
    pub fn with_scoped_store(
        mut self,
        scoped_store: Arc<RwLock<ToolPermissionStore>>,
        enabled: bool,
    ) -> Self {
        self.scoped_store = scoped_store;
        self.scoped_enabled = enabled;
        self
    }

    /// The scoped-grant store, so the confirmation path can record a grant when
    /// the user picks "always allow in this directory" / "always allow this
    /// command prefix".
    pub fn scoped_store(&self) -> Arc<RwLock<ToolPermissionStore>> {
        Arc::clone(&self.scoped_store)
    }

    /// Process inspection results into permission decisions
    /// This method takes all inspection results and converts them into a PermissionCheckResult
    /// that can be used by the agent to determine which tools to approve, deny, or ask for approval
    pub fn process_inspection_results(
        &self,
        remaining_requests: &[ToolRequest],
        inspection_results: &[InspectionResult],
    ) -> PermissionCheckResult {
        use crate::tool_inspection::apply_inspection_results_to_permissions;

        // Start with permission inspector's decisions as the baseline
        let mut permission_check_result = PermissionCheckResult {
            approved: vec![],
            needs_approval: vec![],
            denied: vec![],
        };

        // Apply permission inspector results first (baseline behavior)
        let permission_results: Vec<_> = inspection_results
            .iter()
            .filter(|result| result.inspector_name == "permission")
            .collect();

        for request in remaining_requests {
            // Find the permission decision for this request
            if let Some(permission_result) = permission_results
                .iter()
                .find(|result| result.tool_request_id == request.id)
            {
                match permission_result.action {
                    InspectionAction::Allow => {
                        permission_check_result.approved.push(request.clone());
                    }
                    InspectionAction::Deny => {
                        permission_check_result.denied.push(request.clone());
                    }
                    InspectionAction::RequireApproval(_) => {
                        permission_check_result.needs_approval.push(request.clone());
                    }
                    // The permission inspector never emits an advisory Warn; if
                    // one ever appears it is not a permission decision, so fall
                    // back to the same "ask the user" default as a missing result.
                    InspectionAction::Warn => {
                        permission_check_result.needs_approval.push(request.clone());
                    }
                }
            } else {
                // If no permission result found, default to needs approval for safety
                permission_check_result.needs_approval.push(request.clone());
            }
        }

        // Apply security and other inspector results as overrides
        let non_permission_results: Vec<_> = inspection_results
            .iter()
            .filter(|result| result.inspector_name != "permission")
            .cloned()
            .collect();

        if !non_permission_results.is_empty() {
            permission_check_result = apply_inspection_results_to_permissions(
                permission_check_result,
                &non_permission_results,
            );
        }

        permission_check_result
    }

    /// Managed policy baseline: a trusted admin policy is consulted ahead of the
    /// user's own permission table. This is the only place a managed force-*allow*
    /// can win over a user `NeverAllow` (the escalation-only merge can raise,
    /// never lower). Deny/Ask are belt-and-suspenders here (also enforced by the
    /// dedicated managed inspector). Only meaningful when the model is gating
    /// (Approve/SmartApprove) — Auto allows everything and Chat skips.
    fn managed_verdict(&self, tool_name: &str, mode: BioRouterMode) -> Option<Verdict> {
        if !matches!(mode, BioRouterMode::Approve | BioRouterMode::SmartApprove) {
            return None;
        }
        Some(match self.managed.permission_for(tool_name)? {
            ManagedVerdict::Deny => {
                Verdict::Decided(InspectionAction::Deny, MANAGED_DENY_REASON.to_string())
            }
            ManagedVerdict::Ask => Verdict::Decided(
                InspectionAction::RequireApproval(Some(MANAGED_ASK_REASON.to_string())),
                MANAGED_ASK_REASON.to_string(),
            ),
            ManagedVerdict::Allow => Verdict::Decided(
                InspectionAction::Allow,
                "Allowed by your organization's managed policy".to_string(),
            ),
        })
    }

    /// The user's own stored per-tool preference. Wins over every automatic
    /// grading below — an explicit `AlwaysAllow`/`NeverAllow` is a human decision.
    fn user_verdict(&self, tool_name: &str) -> Option<Verdict> {
        Some(
            match self.permission_manager.get_user_permission(tool_name)? {
                PermissionLevel::AlwaysAllow => Verdict::Decided(
                    InspectionAction::Allow,
                    "User permission allows this tool".to_string(),
                ),
                PermissionLevel::NeverAllow => Verdict::Decided(
                    InspectionAction::Deny,
                    "User permission denies this tool".to_string(),
                ),
                PermissionLevel::AskBefore => Verdict::Decided(
                    InspectionAction::RequireApproval(None),
                    "Tool requires user approval".to_string(),
                ),
            },
        )
    }

    /// BR-18: the smart tier — the whole reason `SmartApprove` now differs from
    /// `Approve`. Auto-approves what the tool's own MCP annotations grade as
    /// read-only; confirms everything at or above the configured risk threshold.
    /// A tool the annotations cannot grade fails closed, unless the opt-in LLM
    /// judge is enabled, in which case it is batched for one classification whose
    /// verdict is cached as a `smart_approve` permission level.
    ///
    /// `risks` is where the grade is read from: the agent's own registry for a
    /// direct call, a Code Execution script's catalogue for a call the script
    /// makes (see [`Self::inspect_graded`]).
    fn smart_verdict(&self, tool_name: &str, risks: &ToolRiskRegistry) -> Verdict {
        if !self.smart.enabled {
            // Kill switch: behave exactly like Approve.
            return Verdict::Decided(
                InspectionAction::RequireApproval(None),
                "Tool requires user approval".to_string(),
            );
        }

        // A judge verdict cached on an earlier turn (or in an earlier session).
        if let Some(level) = self
            .permission_manager
            .get_smart_approve_permission(tool_name)
        {
            return match level {
                PermissionLevel::AlwaysAllow => Verdict::Decided(
                    InspectionAction::Allow,
                    "Smart approve: previously classified as read-only".to_string(),
                ),
                PermissionLevel::NeverAllow => Verdict::Decided(
                    InspectionAction::Deny,
                    "Smart approve denies this tool".to_string(),
                ),
                PermissionLevel::AskBefore => Verdict::Decided(
                    InspectionAction::RequireApproval(None),
                    "Tool requires user approval".to_string(),
                ),
            };
        }

        let risk = risks.risk_for(tool_name);
        if !self.smart.requires_confirmation(risk) {
            return Verdict::Decided(
                InspectionAction::Allow,
                match risk {
                    ToolRisk::Low => "Tool is annotated read-only (low risk)".to_string(),
                    other => format!(
                        "Tool risk ({}) is below the smart-approve threshold",
                        other.label()
                    ),
                },
            );
        }

        if risk == ToolRisk::Unknown && self.smart.judge_unknown {
            return Verdict::AskJudge;
        }

        Verdict::Decided(
            InspectionAction::RequireApproval(None),
            format!("Tool requires user approval ({} risk)", risk.label()),
        )
    }

    /// BR-24: a remembered grant scoped to a directory or a command prefix.
    ///
    /// `None` means "no live grant matches this call" — which is also what every
    /// unparseable, unrecognised or ambiguous call yields, because the matching
    /// in [`crate::permission::permission_scope`] fails closed throughout. The
    /// caller then falls through to the existing behaviour and prompts.
    fn scoped_verdict(
        &self,
        tool_name: &str,
        request: &ToolRequest,
        working_dir: &Path,
    ) -> Option<Verdict> {
        if !self.scoped_enabled {
            return None;
        }
        let tool_call = request.tool_call.as_ref().ok()?;

        let facts = CallFacts::for_tool_call(tool_name, tool_call.arguments.as_ref(), working_dir);
        if !facts.is_scopable() {
            // Nothing a scope can be matched against — not a command, and no
            // path we can vouch for. Never guess.
            return None;
        }

        let grant = self
            .scoped_store
            .read()
            .ok()?
            .matching_scoped_grant(tool_name, &facts)?;

        Some(if grant.allowed {
            Verdict::Decided(
                InspectionAction::Allow,
                format!(
                    "Allowed by a remembered permission scoped to {}",
                    grant.scope.describe()
                ),
            )
        } else {
            Verdict::Decided(
                InspectionAction::Deny,
                format!(
                    "Denied by a remembered permission scoped to {}",
                    grant.scope.describe()
                ),
            )
        })
    }

    /// The deterministic, no-LLM decision for one tool.
    fn deterministic_verdict(
        &self,
        tool_name: &str,
        request: &ToolRequest,
        mode: BioRouterMode,
        working_dir: &Path,
        risks: &ToolRiskRegistry,
    ) -> Verdict {
        if let Some(verdict) = self.managed_verdict(tool_name, mode) {
            return verdict;
        }

        match mode {
            // Chat never reaches here (filtered in `inspect`); Auto allows all.
            BioRouterMode::Chat | BioRouterMode::Auto => Verdict::Decided(
                InspectionAction::Allow,
                "Auto mode - all tools approved".to_string(),
            ),
            BioRouterMode::Approve | BioRouterMode::SmartApprove => {
                // 1. The user's own stored preference.
                if let Some(verdict) = self.user_verdict(tool_name) {
                    return verdict;
                }

                // 2. Enabling/disabling extensions rewires the agent's own tool
                //    surface — always a human decision, in both gating modes.
                //    Deliberately *above* the scoped tier: no directory or
                //    command-prefix grant may buy its way past this gate.
                if tool_name == MANAGE_EXTENSIONS_TOOL_NAME_COMPLETE {
                    return Verdict::Decided(
                        InspectionAction::RequireApproval(Some(
                            "Extension management requires approval for security".to_string(),
                        )),
                        "Extension management requires user approval".to_string(),
                    );
                }

                // 3. BR-24: a scoped grant the user made earlier. It sits below
                //    the managed policy, the tool-level preference and the
                //    extension gate — none of which it can override — and above
                //    the blanket ask, so it relieves approval fatigue in
                //    `Approve` too, not only in `SmartApprove`.
                if let Some(verdict) = self.scoped_verdict(tool_name, request, working_dir) {
                    return verdict;
                }

                // 4. Approve confirms everything else; SmartApprove grades it.
                if mode == BioRouterMode::Approve {
                    return Verdict::Decided(
                        InspectionAction::RequireApproval(None),
                        "Tool requires user approval".to_string(),
                    );
                }

                self.smart_verdict(tool_name, risks)
            }
        }
    }

    /// Ask the LLM judge which of the un-gradeable tools are read-only, caching
    /// each positive verdict as a `smart_approve` permission level so a tool is
    /// judged at most once. Fails closed: `detect_read_only_tools` returns an
    /// empty list on any provider error, so every candidate then needs approval,
    /// and nothing is cached (a transient failure must not pin a tool forever).
    async fn judge_read_only(&self, candidates: &[&ToolRequest]) -> Vec<String> {
        if candidates.is_empty() {
            return vec![];
        }
        let provider = { self.provider.lock().await.clone() };
        let Some(provider) = provider else {
            tracing::debug!("Smart-approve judge is enabled but no provider is installed");
            return vec![];
        };

        let read_only = detect_read_only_tools(provider, candidates.to_vec()).await;
        for tool_name in &read_only {
            self.permission_manager
                .update_smart_approve_permission(tool_name, PermissionLevel::AlwaysAllow);
        }
        read_only
    }

    /// The whole permission decision, with each tool's BR-18 risk grade read out
    /// of `risks` instead of the registry the agent refreshes from the model's
    /// roster.
    ///
    /// ⚠ The registry is the ONLY input that differs, and it differs for one
    /// caller: a tool call a Code Execution script makes (QA finding F7). In
    /// Code Execution mode the model's roster collapses to `code_execution__*`
    /// and a handful of exemptions, so the agent's registry has never graded the
    /// ~76 tools a script can reach, and every one of them would read `Unknown`
    /// — fail-closed, so Smart mode would confirm a read-only `chatrecall` as if
    /// it were a shell. The script's own catalogue (the exact tool list its
    /// imports are built from, carrying each tool's own MCP annotations) is the
    /// analogue of "the exact tool list handed to the model", so its grades are
    /// the ones a direct call to the same tool would get.
    ///
    /// Everything else — the managed policy, the user's own `always_allow` /
    /// `never_allow` entries, the extension-management gate, scoped grants, the
    /// smart-approve kill switch and the opt-in judge — is this inspector's
    /// ordinary sequence, keyed by the tool's own name. The direct path calls
    /// this with `&self.risks`, so it cannot drift from it.
    pub(crate) async fn inspect_graded(
        &self,
        tool_requests: &[ToolRequest],
        biorouter_mode: BioRouterMode,
        session: &crate::session::Session,
        risks: &ToolRiskRegistry,
    ) -> Result<Vec<InspectionResult>> {
        // Chat mode skips tools entirely; the agent splices a canned response.
        if biorouter_mode == BioRouterMode::Chat {
            return Ok(vec![]);
        }

        let mut results = Vec::new();
        let mut judge_candidates: Vec<&ToolRequest> = Vec::new();
        // BR-24: what a relative path argument is resolved against.
        let working_dir = session.working_dir.as_path();

        // Pass 1 — deterministic (managed policy → user preference → extension
        // gate → scoped grant → risk grade).
        for request in tool_requests {
            let Ok(tool_call) = &request.tool_call else {
                continue;
            };
            match self.deterministic_verdict(
                &tool_call.name,
                request,
                biorouter_mode,
                working_dir,
                risks,
            ) {
                Verdict::Decided(action, reason) => results.push(InspectionResult {
                    tool_request_id: request.id.clone(),
                    action,
                    reason,
                    confidence: 1.0, // Permission decisions are definitive
                    inspector_name: self.name().to_string(),
                    finding_id: None,
                }),
                Verdict::AskJudge => judge_candidates.push(request),
            }
        }

        // Pass 2 — one LLM round-trip for whatever the annotations could not grade.
        if !judge_candidates.is_empty() {
            let read_only = self.judge_read_only(&judge_candidates).await;
            for request in judge_candidates {
                let Ok(tool_call) = &request.tool_call else {
                    continue;
                };
                let (action, reason) = if read_only.iter().any(|n| n == tool_call.name.as_ref()) {
                    (
                        InspectionAction::Allow,
                        "Smart approve: judged read-only".to_string(),
                    )
                } else {
                    (
                        InspectionAction::RequireApproval(None),
                        "Tool requires user approval (unknown risk)".to_string(),
                    )
                };
                results.push(InspectionResult {
                    tool_request_id: request.id.clone(),
                    action,
                    reason,
                    confidence: 1.0,
                    inspector_name: self.name().to_string(),
                    finding_id: None,
                });
            }
        }

        Ok(results)
    }
}

#[async_trait]
impl ToolInspector for PermissionInspector {
    fn name(&self) -> &'static str {
        "permission"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn inspect(
        &self,
        tool_requests: &[ToolRequest],
        _messages: &[Message],
        biorouter_mode: BioRouterMode,
        session: &crate::session::Session,
    ) -> Result<Vec<InspectionResult>> {
        self.inspect_graded(tool_requests, biorouter_mode, session, &self.risks)
            .await
    }
}
