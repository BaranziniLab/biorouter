//! Integration tests for the hooks system at the tool-inspection boundary:
//! PreToolUse hooks flowing through HookInspector into InspectionResults and
//! the permission-mixing logic, plus project-level hook file loading.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::sync::Arc;

use biorouter::config::BioRouterMode;
use biorouter::conversation::message::Message;
use biorouter::hooks::{
    HookDecision, HookInspector, HooksConfig, HooksManager, StopHookVerdict, STOP_HOOK_BLOCK_CAP,
};
use biorouter::permission::permission_judge::PermissionCheckResult;
use biorouter::session::Session;
use biorouter::tool_inspection::{
    apply_inspection_results_to_permissions, InspectionAction, ToolInspector,
};
use rmcp::model::CallToolRequestParams;
use rmcp::object;
use tokio::sync::Mutex;

fn manager_from_yaml(yaml: &str) -> Arc<HooksManager> {
    let config: HooksConfig = serde_yaml::from_str(yaml).expect("test yaml parses");
    Arc::new(HooksManager::with_config(
        config,
        true,
        Arc::new(Mutex::new(None)),
    ))
}

fn tool_request(id: &str, name: &str) -> biorouter::conversation::message::ToolRequest {
    tool_request_with(id, name, object!({"command": "echo hi"}))
}

fn tool_request_with(
    id: &str,
    name: &str,
    arguments: serde_json::Map<String, serde_json::Value>,
) -> biorouter::conversation::message::ToolRequest {
    let message = Message::assistant().with_tool_request(
        id,
        Ok(CallToolRequestParams {
            task: None,
            meta: None,
            name: name.to_string().into(),
            arguments: Some(arguments),
        }),
    );
    message
        .content
        .first()
        .and_then(|content| content.as_tool_request())
        .expect("message contains a tool request")
        .clone()
}

fn session_in(dir: &std::path::Path) -> Session {
    Session {
        working_dir: dir.to_path_buf(),
        ..Session::default()
    }
}

#[tokio::test]
async fn pre_tool_use_deny_becomes_deny_inspection_result() {
    let manager = manager_from_yaml(
        r#"
PreToolUse:
  - matcher: "developer__shell"
    hooks:
      - type: command
        command: "echo 'rm is forbidden here' >&2; exit 2"
"#,
    );
    let inspector = HookInspector::new(manager);
    let session = Session::default();

    let results = inspector
        .inspect(
            &[tool_request("req_1", "developer__shell")],
            &[],
            BioRouterMode::Approve,
            &session,
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].action, InspectionAction::Deny);
    assert_eq!(results[0].reason, "rm is forbidden here");
    assert_eq!(results[0].inspector_name, "hooks");

    // The denial flows through the permission-mixing logic.
    let permission_result = PermissionCheckResult {
        approved: vec![tool_request("req_1", "developer__shell")],
        needs_approval: vec![],
        denied: vec![],
    };
    let mixed = apply_inspection_results_to_permissions(permission_result, &results);
    assert!(mixed.approved.is_empty());
    assert_eq!(mixed.denied.len(), 1);
}

#[tokio::test]
async fn pre_tool_use_ask_becomes_require_approval() {
    let manager = manager_from_yaml(
        r#"
PreToolUse:
  - hooks:
      - type: command
        command: "echo '{\"hookSpecificOutput\":{\"permissionDecision\":\"ask\",\"permissionDecisionReason\":\"hook wants eyes on this\"}}'"
"#,
    );
    let inspector = HookInspector::new(manager);

    let results = inspector
        .inspect(
            &[tool_request("req_1", "anything")],
            &[],
            BioRouterMode::Approve,
            &Session::default(),
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    match &results[0].action {
        InspectionAction::RequireApproval(Some(message)) => {
            assert_eq!(message, "hook wants eyes on this");
        }
        other => panic!("expected RequireApproval, got {:?}", other),
    }
}

#[tokio::test]
async fn unmatched_tool_produces_no_results() {
    let manager = manager_from_yaml(
        r#"
PreToolUse:
  - matcher: "developer__shell"
    hooks:
      - type: command
        command: "exit 2"
"#,
    );
    let inspector = HookInspector::new(manager);

    let results = inspector
        .inspect(
            &[tool_request("req_1", "memory__store")],
            &[],
            BioRouterMode::Approve,
            &Session::default(),
        )
        .await
        .unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn failing_hook_is_failure_open_in_inspector() {
    let manager = manager_from_yaml(
        r#"
PreToolUse:
  - hooks:
      - type: command
        command: "exit 1"
"#,
    );
    let inspector = HookInspector::new(manager);

    let results = inspector
        .inspect(
            &[tool_request("req_1", "anything")],
            &[],
            BioRouterMode::Approve,
            &Session::default(),
        )
        .await
        .unwrap();
    assert!(
        results.is_empty(),
        "non-zero exit (other than 2) must not block"
    );
}

#[tokio::test]
async fn project_hooks_file_drives_pre_tool_use() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".biorouter")).unwrap();
    std::fs::write(
        dir.path().join(".biorouter/hooks.yaml"),
        r#"hooks:
  PreToolUse:
    - matcher: "developer__.*"
      hooks:
        - type: command
          command: "echo 'project says no' >&2; exit 2"
"#,
    )
    .unwrap();

    // allow_project_hooks = true
    let manager = Arc::new(HooksManager::with_config(
        HooksConfig::default(),
        true,
        Arc::new(Mutex::new(None)),
    ));
    let inspector = HookInspector::new(manager);
    let session = session_in(dir.path());

    let results = inspector
        .inspect(
            &[tool_request("req_1", "developer__text_editor")],
            &[],
            BioRouterMode::Approve,
            &session,
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].action, InspectionAction::Deny);
    assert_eq!(results[0].reason, "project says no");

    // Without the opt-in, the same project file is ignored.
    let manager = Arc::new(HooksManager::with_config(
        HooksConfig::default(),
        false,
        Arc::new(Mutex::new(None)),
    ));
    let inspector = HookInspector::new(manager);
    let results = inspector
        .inspect(
            &[tool_request("req_1", "developer__text_editor")],
            &[],
            BioRouterMode::Approve,
            &session,
        )
        .await
        .unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn permission_request_wrapper_returns_allow_decision() {
    let manager = manager_from_yaml(
        r#"
PermissionRequest:
  - matcher: "developer__shell"
    hooks:
      - type: command
        command: "echo '{\"hookSpecificOutput\":{\"permissionDecision\":\"allow\",\"permissionDecisionReason\":\"trusted command\"}}'"
"#,
    );
    let aggregate = manager
        .permission_request(
            "sess",
            std::path::Path::new("/tmp"),
            "developer__shell",
            &serde_json::json!({"command": "ls"}),
        )
        .await;
    assert_eq!(
        aggregate.decision,
        Some(HookDecision::Allow {
            reason: Some("trusted command".to_string())
        })
    );
}

#[tokio::test]
async fn stop_block_loop_respects_cap_end_to_end() {
    let manager = manager_from_yaml(
        r#"
Stop:
  - hooks:
      - type: command
        command: "echo 'tests have not been run' >&2; exit 2"
"#,
    );
    let mut blocked = 0;
    loop {
        match manager
            .stop("sess", std::path::Path::new("/tmp"), None)
            .await
        {
            StopHookVerdict::Blocked { reason } => {
                assert_eq!(reason, "tests have not been run");
                blocked += 1;
                assert!(blocked <= STOP_HOOK_BLOCK_CAP, "must not exceed the cap");
            }
            StopHookVerdict::CapReached => break,
            StopHookVerdict::Proceed => panic!("hook always blocks; Proceed unexpected"),
        }
    }
    assert_eq!(blocked, STOP_HOOK_BLOCK_CAP);
}

#[tokio::test]
async fn user_prompt_submit_context_and_block() {
    let manager = manager_from_yaml(
        r#"
UserPromptSubmit:
  - hooks:
      - type: command
        command: "cat | grep -q forbidden && { echo 'prompt contains a forbidden term' >&2; exit 2; } || echo 'lab context: BSL-2 protocols apply'"
"#,
    );

    let blocked = manager
        .user_prompt_submit(
            "sess",
            std::path::Path::new("/tmp"),
            "do the forbidden thing",
        )
        .await;
    assert!(blocked.is_denied());
    assert_eq!(
        blocked.deny_reason(),
        Some("prompt contains a forbidden term")
    );

    let allowed = manager
        .user_prompt_submit("sess", std::path::Path::new("/tmp"), "hello")
        .await;
    assert!(!allowed.is_denied());
    assert_eq!(
        allowed.joined_context().as_deref(),
        Some("lab context: BSL-2 protocols apply")
    );
}

/// BR-27: a group scoped by `input_matcher` only fires on tool calls whose
/// arguments match, so the guard costs nothing on the other 99% of shell calls.
#[tokio::test]
async fn input_matcher_gates_the_inspector_on_tool_arguments() {
    let manager = manager_from_yaml(
        r#"
PreToolUse:
  - matcher: "developer__shell"
    input_matcher:
      command: "rm\\s+-rf"
    hooks:
      - type: command
        command: "echo 'recursive delete blocked' >&2; exit 2"
"#,
    );
    let inspector = HookInspector::new(manager);

    let results = inspector
        .inspect(
            &[tool_request_with(
                "req_1",
                "developer__shell",
                object!({"command": "rm -rf /tmp/scratch"}),
            )],
            &[],
            BioRouterMode::Approve,
            &Session::default(),
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].action, InspectionAction::Deny);
    assert_eq!(results[0].reason, "recursive delete blocked");

    // Same tool, benign arguments: the guard never runs.
    let results = inspector
        .inspect(
            &[tool_request_with(
                "req_2",
                "developer__shell",
                object!({"command": "ls -la"}),
            )],
            &[],
            BioRouterMode::Approve,
            &Session::default(),
        )
        .await
        .unwrap();
    assert!(results.is_empty());
}
