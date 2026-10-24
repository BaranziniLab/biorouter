//! BR-18 — integration tests for the revived read-only auto-approve + per-action
//! risk grading, at the real tool-inspection boundary.
//!
//! The bug these lock down: `PermissionInspector`'s `readonly_tools`/`regular_tools`
//! sets were constructed empty with no setter and no second construction site, so
//! the read-only short-circuit could never fire and the LLM permission judge had
//! zero callers — which made `SmartApprove` behaviourally identical to `Approve`
//! and prompted the user on *every* read.
//!
//! The load-bearing assertions are therefore comparative: the same request under
//! `Approve` vs `SmartApprove` must now land in different buckets.

use std::collections::HashSet;
use std::sync::Arc;

use biorouter::config::permission::PermissionLevel;
use biorouter::config::{BioRouterMode, PermissionManager};
use biorouter::conversation::message::{Message, ToolRequest};
use biorouter::managed::{ManagedPolicy, ManagedPolicyFile};
use biorouter::permission::permission_judge::PermissionCheckResult;
use biorouter::permission::{
    ManagedPolicyInspector, PermissionInspector, SmartApproveConfig, ToolRisk, ToolRiskRegistry,
};
use biorouter::session::Session;
use biorouter::tool_inspection::ToolInspectionManager;
use rmcp::model::{CallToolRequestParams, Tool, ToolAnnotations};
use rmcp::object;
use tempfile::TempDir;

const READ_TOOL: &str = "developer__text_editor_view";
const WRITE_TOOL: &str = "developer__text_editor_write";
const SHELL_TOOL: &str = "developer__shell";
const UNANNOTATED_TOOL: &str = "thirdparty__mystery";
const MANAGE_EXTENSIONS: &str = "extensionmanager__manage_extensions";

fn tool(name: &str, annotations: Option<ToolAnnotations>) -> Tool {
    let t = Tool::new(
        name.to_string(),
        "desc".to_string(),
        Arc::new(object!({"type": "object", "properties": {}})),
    );
    match annotations {
        Some(a) => t.annotate(a),
        None => t,
    }
}

fn annot(read_only: bool, destructive: bool) -> ToolAnnotations {
    ToolAnnotations {
        title: None,
        read_only_hint: Some(read_only),
        destructive_hint: Some(destructive),
        idempotent_hint: None,
        open_world_hint: None,
    }
}

fn tool_request(id: &str, name: &str) -> ToolRequest {
    Message::assistant()
        .with_tool_request(
            id,
            Ok(CallToolRequestParams {
                task: None,
                meta: None,
                name: name.to_string().into(),
                arguments: Some(object!({"path": "/tmp/x"})),
            }),
        )
        .content
        .first()
        .and_then(|content| content.as_tool_request())
        .expect("message contains a tool request")
        .clone()
}

/// The tool surface a real session would hand the model: an annotated reader, an
/// annotated non-destructive writer, an annotated destructive shell, one tool
/// with no annotations at all, and the extension-management tool.
fn realistic_tools() -> Vec<Tool> {
    vec![
        tool(READ_TOOL, Some(annot(true, false))),
        tool(WRITE_TOOL, Some(annot(false, false))),
        tool(SHELL_TOOL, Some(annot(false, true))),
        tool(UNANNOTATED_TOOL, None),
        tool(MANAGE_EXTENSIONS, Some(annot(false, false))),
    ]
}

fn no_managed() -> Arc<ManagedPolicy> {
    let file: ManagedPolicyFile = serde_yaml::from_str("{}").expect("empty managed yaml parses");
    Arc::new(ManagedPolicy::from_file(file))
}

/// Mirrors the agent's real registration order (managed → permission), with the
/// risk registry refreshed from `tools` exactly as `prepare_tools_and_prompt` does.
fn build_manager(
    smart: SmartApproveConfig,
    tools: &[Tool],
) -> (ToolInspectionManager, Arc<PermissionManager>, TempDir) {
    let temp = TempDir::new().unwrap();
    let permission_manager = Arc::new(PermissionManager::new(temp.path().to_path_buf()));
    let managed = no_managed();

    let risks = Arc::new(ToolRiskRegistry::new());
    risks.refresh_from_tools(tools);

    let mut manager = ToolInspectionManager::new();
    manager.add_inspector(Box::new(ManagedPolicyInspector::new(Arc::clone(&managed))));
    manager.add_inspector(Box::new(PermissionInspector::with_smart_config(
        risks,
        Arc::clone(&permission_manager),
        managed,
        // No provider installed: the LLM judge cannot run, so these tests only
        // exercise the free, deterministic annotation path.
        Arc::new(tokio::sync::Mutex::new(None)),
        smart,
    )));
    (manager, permission_manager, temp)
}

async fn decide(
    manager: &ToolInspectionManager,
    requests: &[ToolRequest],
    mode: BioRouterMode,
) -> PermissionCheckResult {
    let session = Session::default();
    let results = manager
        .inspect_tools(requests, &[], mode, &session)
        .await
        .expect("inspection succeeds");
    manager
        .process_inspection_results_with_permission_inspector(requests, &results)
        .expect("permission inspector present")
}

fn names(requests: &[ToolRequest]) -> HashSet<String> {
    requests
        .iter()
        .filter_map(|r| r.tool_call.as_ref().ok())
        .map(|c| c.name.to_string())
        .collect()
}

fn all_requests() -> Vec<ToolRequest> {
    vec![
        tool_request("r-read", READ_TOOL),
        tool_request("r-write", WRITE_TOOL),
        tool_request("r-shell", SHELL_TOOL),
        tool_request("r-unknown", UNANNOTATED_TOOL),
    ]
}

/// The headline behaviour: with the default policy, `SmartApprove` silently runs
/// a read-only-annotated tool while `Approve` still stops to ask. Before BR-18
/// both prompted — the two modes were the same mode.
#[tokio::test]
async fn smart_approve_auto_approves_reads_where_approve_asks() {
    let (manager, _pm, _tmp) = build_manager(SmartApproveConfig::default(), &realistic_tools());
    let requests = all_requests();

    let approve = decide(&manager, &requests, BioRouterMode::Approve).await;
    assert!(
        approve.approved.is_empty(),
        "Approve must confirm every tool, including reads"
    );
    assert_eq!(approve.needs_approval.len(), 4);

    let smart = decide(&manager, &requests, BioRouterMode::SmartApprove).await;
    assert_eq!(
        names(&smart.approved),
        HashSet::from([READ_TOOL.to_string()]),
        "only the read-only-annotated tool auto-approves"
    );
    assert_eq!(
        names(&smart.needs_approval),
        HashSet::from([
            WRITE_TOOL.to_string(),
            SHELL_TOOL.to_string(),
            UNANNOTATED_TOOL.to_string(),
        ]),
    );
    assert!(smart.denied.is_empty());
}

/// A tool with no usable annotation grades `Unknown` and is confirmed — the
/// fail-closed default the review asked for ("fail-safe on unknown").
#[tokio::test]
async fn unannotated_tool_fails_closed_in_smart_mode() {
    let (manager, _pm, _tmp) = build_manager(SmartApproveConfig::default(), &realistic_tools());
    let requests = vec![tool_request("r1", UNANNOTATED_TOOL)];
    let result = decide(&manager, &requests, BioRouterMode::SmartApprove).await;
    assert_eq!(
        names(&result.needs_approval),
        HashSet::from([UNANNOTATED_TOOL.to_string()])
    );
    assert!(result.approved.is_empty());
}

/// A tool call naming something the registry has never graded (an extension that
/// vanished mid-turn, a hallucinated name) must not slip through as "low risk".
#[tokio::test]
async fn tool_absent_from_the_registry_fails_closed() {
    let (manager, _pm, _tmp) = build_manager(SmartApproveConfig::default(), &realistic_tools());
    let requests = vec![tool_request("r1", "ghost__gone")];
    let result = decide(&manager, &requests, BioRouterMode::SmartApprove).await;
    assert_eq!(result.needs_approval.len(), 1);
    assert!(result.approved.is_empty());
}

/// A destructive-annotated tool grades High and is confirmed even if the
/// threshold is raised — and a *self-contradictory* annotation (read-only AND
/// destructive) must be treated as destructive, not auto-approved.
#[tokio::test]
async fn contradictory_read_only_plus_destructive_annotation_is_not_auto_approved() {
    let liar = "evil__pretend_read";
    let tools = vec![tool(liar, Some(annot(true, true)))];
    let (manager, _pm, _tmp) = build_manager(SmartApproveConfig::default(), &tools);
    let requests = vec![tool_request("r1", liar)];
    let result = decide(&manager, &requests, BioRouterMode::SmartApprove).await;
    assert!(
        result.approved.is_empty(),
        "a tool claiming both read-only and destructive must fail closed"
    );
    assert_eq!(result.needs_approval.len(), 1);
}

/// Raising the threshold to `high` lets non-destructive writers through too —
/// the `ConfirmRisky(threshold, …)` knob actually moves the line.
#[tokio::test]
async fn threshold_high_also_auto_approves_non_destructive_writes() {
    let smart = SmartApproveConfig {
        threshold: ToolRisk::High,
        ..SmartApproveConfig::default()
    };
    let (manager, _pm, _tmp) = build_manager(smart, &realistic_tools());
    let result = decide(&manager, &all_requests(), BioRouterMode::SmartApprove).await;

    assert_eq!(
        names(&result.approved),
        HashSet::from([READ_TOOL.to_string(), WRITE_TOOL.to_string()]),
    );
    assert_eq!(
        names(&result.needs_approval),
        HashSet::from([SHELL_TOOL.to_string(), UNANNOTATED_TOOL.to_string()]),
        "the destructive shell and the un-gradeable tool still need a human"
    );
}

/// The kill switch restores the pre-BR-18 behaviour exactly: SmartApprove == Approve.
#[tokio::test]
async fn disabling_risk_grading_makes_smart_approve_identical_to_approve() {
    let smart = SmartApproveConfig {
        enabled: false,
        ..SmartApproveConfig::default()
    };
    let (manager, _pm, _tmp) = build_manager(smart, &realistic_tools());
    let requests = all_requests();

    let approve = decide(&manager, &requests, BioRouterMode::Approve).await;
    let smart_result = decide(&manager, &requests, BioRouterMode::SmartApprove).await;

    assert!(smart_result.approved.is_empty());
    assert_eq!(
        names(&smart_result.needs_approval),
        names(&approve.needs_approval)
    );
}

/// A user's explicit `NeverAllow` outranks a read-only annotation — the automatic
/// grading may never *lower* a decision the human already made.
#[tokio::test]
async fn user_never_allow_beats_a_read_only_annotation() {
    let (manager, pm, _tmp) = build_manager(SmartApproveConfig::default(), &realistic_tools());
    pm.update_user_permission(READ_TOOL, PermissionLevel::NeverAllow);

    let requests = vec![tool_request("r1", READ_TOOL)];
    let result = decide(&manager, &requests, BioRouterMode::SmartApprove).await;
    assert_eq!(
        names(&result.denied),
        HashSet::from([READ_TOOL.to_string()])
    );
    assert!(result.approved.is_empty());
}

/// Enabling/disabling extensions rewires the agent's own tool surface, so it stays
/// a human decision in the smart tier even though it grades merely `Medium`.
#[tokio::test]
async fn extension_management_still_requires_approval_in_smart_mode() {
    let (manager, _pm, _tmp) = build_manager(SmartApproveConfig::default(), &realistic_tools());
    let requests = vec![tool_request("r1", MANAGE_EXTENSIONS)];
    let result = decide(&manager, &requests, BioRouterMode::SmartApprove).await;
    assert_eq!(result.needs_approval.len(), 1);
    assert!(result.approved.is_empty());
}

/// Auto still approves everything and Chat still emits no permission verdict —
/// the risk tier must not leak into the other two modes.
#[tokio::test]
async fn auto_and_chat_modes_are_unchanged() {
    let (manager, _pm, _tmp) = build_manager(SmartApproveConfig::default(), &realistic_tools());
    let requests = all_requests();

    let auto = decide(&manager, &requests, BioRouterMode::Auto).await;
    assert_eq!(auto.approved.len(), 4);
    assert!(auto.needs_approval.is_empty() && auto.denied.is_empty());

    // Chat: the permission inspector returns no results at all, so the merge's
    // fail-closed default puts every request in needs_approval (the agent never
    // gets that far — it splices a canned skip response first).
    let session = Session::default();
    let results = manager
        .inspect_tools(&requests, &[], BioRouterMode::Chat, &session)
        .await
        .expect("inspection succeeds");
    assert!(
        results.iter().all(|r| r.inspector_name != "permission"),
        "Chat mode yields no permission verdicts"
    );
}

/// Legacy automatic tool-wide grants cannot authorize a different invocation.
#[tokio::test]
async fn legacy_smart_approve_allow_requires_a_fresh_decision() {
    let (manager, pm, _tmp) = build_manager(SmartApproveConfig::default(), &realistic_tools());
    pm.update_smart_approve_permission(UNANNOTATED_TOOL, PermissionLevel::AlwaysAllow);

    let requests = vec![tool_request("r1", UNANNOTATED_TOOL)];
    let result = decide(&manager, &requests, BioRouterMode::SmartApprove).await;
    assert!(result.approved.is_empty());
    assert_eq!(result.needs_approval.len(), 1);

    // ...but only in the smart tier. Plain Approve ignores the cache.
    let approve = decide(&manager, &requests, BioRouterMode::Approve).await;
    assert!(approve.approved.is_empty());
    assert_eq!(approve.needs_approval.len(), 1);
}

#[tokio::test]
async fn explicit_user_allow_still_authorizes_unannotated_tools() {
    let (manager, pm, _tmp) = build_manager(SmartApproveConfig::default(), &realistic_tools());
    pm.update_user_permission(UNANNOTATED_TOOL, PermissionLevel::AlwaysAllow);

    let requests = vec![tool_request("r1", UNANNOTATED_TOOL)];
    let result = decide(&manager, &requests, BioRouterMode::SmartApprove).await;
    assert_eq!(result.approved.len(), 1);
    assert!(result.needs_approval.is_empty());
}

#[tokio::test]
async fn legacy_smart_approve_restrictions_still_apply() {
    let (manager, pm, _tmp) = build_manager(SmartApproveConfig::default(), &realistic_tools());
    pm.update_smart_approve_permission(READ_TOOL, PermissionLevel::NeverAllow);
    pm.update_smart_approve_permission(WRITE_TOOL, PermissionLevel::AskBefore);

    let result = decide(&manager, &all_requests(), BioRouterMode::SmartApprove).await;
    assert_eq!(
        names(&result.denied),
        HashSet::from([READ_TOOL.to_string()])
    );
    assert!(names(&result.needs_approval).contains(WRITE_TOOL));
}
