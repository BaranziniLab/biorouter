//! Integration tests for the BR-65 managed/enterprise policy tier at the
//! tool-inspection boundary: a trusted admin policy's deny/ask/allow verdicts
//! flowing through `ManagedPolicyInspector` + `PermissionInspector` and the
//! escalation-only merge, proving the non-bypassable + baseline-allow semantics.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::sync::Arc;

use biorouter::config::permission::PermissionLevel;
use biorouter::config::{BioRouterMode, PermissionManager};
use biorouter::conversation::message::{Message, ToolRequest};
use biorouter::managed::{ManagedPolicy, ManagedPolicyFile};
use biorouter::permission::{
    ManagedPolicyInspector, PermissionInspector, SmartApproveConfig, ToolRiskRegistry,
};
use biorouter::session::Session;
use biorouter::tool_inspection::ToolInspectionManager;
use rmcp::model::CallToolRequestParams;
use rmcp::object;
use tempfile::TempDir;

fn tool_request(id: &str, name: &str) -> ToolRequest {
    Message::assistant()
        .with_tool_request(
            id,
            Ok(CallToolRequestParams {
                task: None,
                meta: None,
                name: name.to_string().into(),
                arguments: Some(object!({"command": "echo hi"})),
            }),
        )
        .content
        .first()
        .and_then(|content| content.as_tool_request())
        .expect("message contains a tool request")
        .clone()
}

fn managed(yaml: &str) -> Arc<ManagedPolicy> {
    let file: ManagedPolicyFile = serde_yaml::from_str(yaml).expect("managed yaml parses");
    Arc::new(ManagedPolicy::from_file(file))
}

/// Build a manager wiring the managed inspector (first) + the permission
/// inspector sharing the same managed policy — mirroring the agent's real
/// registration order. Returns the manager plus its permission manager so tests
/// can seed user permissions.
fn build_manager(
    managed: Arc<ManagedPolicy>,
) -> (ToolInspectionManager, Arc<PermissionManager>, TempDir) {
    let temp = TempDir::new().unwrap();
    let permission_manager = Arc::new(PermissionManager::new(temp.path().to_path_buf()));
    let mut manager = ToolInspectionManager::new();
    manager.add_inspector(Box::new(ManagedPolicyInspector::new(Arc::clone(&managed))));
    // BR-18: an empty risk registry + the default smart policy — every tool here
    // grades `Unknown`, so the managed/user decisions under test are unaffected.
    manager.add_inspector(Box::new(PermissionInspector::with_smart_config(
        Arc::new(ToolRiskRegistry::new()),
        Arc::clone(&permission_manager),
        managed,
        Arc::new(tokio::sync::Mutex::new(None)),
        SmartApproveConfig::default(),
    )));
    (manager, permission_manager, temp)
}

async fn decide(
    manager: &ToolInspectionManager,
    requests: &[ToolRequest],
    mode: BioRouterMode,
) -> biorouter::permission::permission_judge::PermissionCheckResult {
    let session = Session::default();
    let results = manager
        .inspect_tools(requests, &[], mode, &session)
        .await
        .expect("inspection succeeds");
    manager
        .process_inspection_results_with_permission_inspector(requests, &results)
        .expect("permission inspector present")
}

/// A managed `deny` hard-blocks a tool **even in Auto mode**, proving it is
/// non-bypassable via the escalation-only merge.
#[tokio::test]
async fn managed_deny_blocks_even_in_auto_mode() {
    let (manager, _pm, _tmp) =
        build_manager(managed("permissions:\n  deny: [\"developer__shell\"]\n"));
    let requests = vec![tool_request("r1", "developer__shell")];
    let result = decide(&manager, &requests, BioRouterMode::Auto).await;
    assert_eq!(
        result.denied.len(),
        1,
        "managed deny must ride over Auto Allow"
    );
    assert_eq!(result.denied[0].id, "r1");
    assert!(result.approved.is_empty());
}

/// A managed `ask` forces an approval prompt regardless of mode.
#[tokio::test]
async fn managed_ask_requires_approval_in_auto_mode() {
    let (manager, _pm, _tmp) =
        build_manager(managed("permissions:\n  ask: [\"developer__shell\"]\n"));
    let requests = vec![tool_request("r1", "developer__shell")];
    let result = decide(&manager, &requests, BioRouterMode::Auto).await;
    assert_eq!(result.needs_approval.len(), 1);
    assert_eq!(result.needs_approval[0].id, "r1");
    assert!(result.approved.is_empty());
}

/// A managed `allow` on a tool the user set `NeverAllow` still resolves to
/// approved — the one precedence the escalation-only merge cannot express, so it
/// lives in the permission baseline.
#[tokio::test]
async fn managed_allow_overrides_user_never_allow() {
    let policy = managed("permissions:\n  allow: [\"developer__shell\"]\n");
    let (manager, pm, _tmp) = build_manager(policy);
    pm.update_user_permission("developer__shell", PermissionLevel::NeverAllow);

    let requests = vec![tool_request("r1", "developer__shell")];
    let result = decide(&manager, &requests, BioRouterMode::Approve).await;
    assert_eq!(
        result.approved.len(),
        1,
        "managed allow must beat user NeverAllow"
    );
    assert_eq!(result.approved[0].id, "r1");
    assert!(result.denied.is_empty());
}

/// Regression guard: with no managed file, a user `NeverAllow` is honored
/// (managed inspector is disabled and the baseline is a no-op).
#[tokio::test]
async fn no_managed_file_preserves_user_never_allow() {
    let (manager, pm, _tmp) = build_manager(Arc::new(ManagedPolicy::empty()));
    pm.update_user_permission("developer__shell", PermissionLevel::NeverAllow);

    let requests = vec![tool_request("r1", "developer__shell")];
    let result = decide(&manager, &requests, BioRouterMode::Approve).await;
    assert_eq!(
        result.denied.len(),
        1,
        "user NeverAllow unchanged without managed policy"
    );
    assert!(result.approved.is_empty());
}
