//! BR-24 — integration tests for per-directory / per-command-prefix permission
//! scoping, at the real tool-inspection boundary.
//!
//! The unit tests in `permission_scope.rs` pin the matching semantics. These pin
//! the thing that actually matters to a user: that a scoped grant lands a call in
//! the `approved` bucket, and that every *near miss* still lands in
//! `needs_approval`. An over-broad remembered approval is worse than no feature,
//! so the MUST-NOT cases outnumber the MUST cases here on purpose.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use biorouter::config::permission::PermissionLevel;
use biorouter::config::{BioRouterMode, PermissionManager};
use biorouter::conversation::message::{Message, ToolRequest};
use biorouter::managed::{ManagedPolicy, ManagedPolicyFile};
use biorouter::permission::permission_judge::PermissionCheckResult;
use biorouter::permission::{
    ManagedPolicyInspector, PermissionInspector, PermissionScope, SmartApproveConfig,
    ToolPermissionStore, ToolRiskRegistry,
};
use biorouter::session::Session;
use biorouter::tool_inspection::ToolInspectionManager;
use rmcp::model::{CallToolRequestParams, Tool, ToolAnnotations};
use rmcp::object;
use tempfile::TempDir;

const SHELL: &str = "developer__shell";
const EDITOR: &str = "developer__text_editor";
const MANAGE_EXTENSIONS: &str = "extensionmanager__manage_extensions";

fn tool(name: &str, read_only: bool, destructive: bool) -> Tool {
    Tool::new(
        name.to_string(),
        "desc".to_string(),
        Arc::new(object!({"type": "object", "properties": {}})),
    )
    .annotate(ToolAnnotations {
        title: None,
        read_only_hint: Some(read_only),
        destructive_hint: Some(destructive),
        idempotent_hint: None,
        open_world_hint: None,
    })
}

fn request(id: &str, name: &str, arguments: rmcp::model::JsonObject) -> ToolRequest {
    Message::assistant()
        .with_tool_request(
            id,
            Ok(CallToolRequestParams {
                task: None,
                meta: None,
                name: name.to_string().into(),
                arguments: Some(arguments),
            }),
        )
        .content
        .first()
        .and_then(|content| content.as_tool_request())
        .expect("message contains a tool request")
        .clone()
}

fn shell_request(id: &str, command: &str) -> ToolRequest {
    request(id, SHELL, object!({"command": command}))
}

fn write_request(id: &str, path: &Path) -> ToolRequest {
    request(
        id,
        EDITOR,
        object!({"command": "write", "path": path.to_str().unwrap(), "file_text": "x"}),
    )
}

fn no_managed() -> Arc<ManagedPolicy> {
    let file: ManagedPolicyFile = serde_yaml::from_str("{}").expect("empty managed yaml parses");
    Arc::new(ManagedPolicy::from_file(file))
}

/// The real inspector stack (managed → permission), with a scoped store pinned to
/// a temp dir so the test never touches the developer's own permissions file.
struct Harness {
    manager: ToolInspectionManager,
    store: Arc<RwLock<ToolPermissionStore>>,
    permission_manager: Arc<PermissionManager>,
    session: Session,
    _temp: TempDir,
}

impl Harness {
    fn new(working_dir: PathBuf) -> Self {
        let temp = TempDir::new().unwrap();
        let permission_manager = Arc::new(PermissionManager::new(temp.path().to_path_buf()));
        let managed = no_managed();

        let risks = Arc::new(ToolRiskRegistry::new());
        risks.refresh_from_tools(&[
            tool(SHELL, false, true),
            tool(EDITOR, false, false),
            tool(MANAGE_EXTENSIONS, false, false),
        ]);

        let store = Arc::new(RwLock::new(ToolPermissionStore::new_in(
            temp.path().join("permissions"),
        )));

        let mut manager = ToolInspectionManager::new();
        manager.add_inspector(Box::new(ManagedPolicyInspector::new(Arc::clone(&managed))));
        manager.add_inspector(Box::new(
            PermissionInspector::with_smart_config(
                risks,
                Arc::clone(&permission_manager),
                managed,
                Arc::new(tokio::sync::Mutex::new(None)),
                SmartApproveConfig::default(),
            )
            .with_scoped_store(Arc::clone(&store), true),
        ));

        let session = Session {
            working_dir,
            ..Session::default()
        };

        Self {
            manager,
            store,
            permission_manager,
            session,
            _temp: temp,
        }
    }

    fn grant(&self, tool_name: &str, scope: PermissionScope, allowed: bool) {
        self.store
            .write()
            .unwrap()
            .record_scoped_grant(tool_name, scope, allowed, None)
            .expect("grant recorded");
    }

    async fn decide(&self, requests: &[ToolRequest], mode: BioRouterMode) -> PermissionCheckResult {
        let results = self
            .manager
            .inspect_tools(requests, &[], mode, &self.session)
            .await
            .expect("inspection succeeds");
        self.manager
            .process_inspection_results_with_permission_inspector(requests, &results)
            .expect("permission inspector present")
    }
}

fn ids(requests: &[ToolRequest]) -> Vec<String> {
    requests.iter().map(|r| r.id.clone()).collect()
}

/// A `git status` prefix grant approves `git status`, and *only* `git status`.
#[tokio::test]
async fn a_command_prefix_grant_approves_the_prefix_and_prompts_for_everything_else() {
    let temp = TempDir::new().unwrap();
    let harness = Harness::new(temp.path().to_path_buf());
    harness.grant(
        SHELL,
        PermissionScope::command_prefix("git status").unwrap(),
        true,
    );

    let requests = vec![
        shell_request("allowed", "git status --short"),
        // MUST NOT: a different subcommand.
        shell_request("push", "git push --force origin main"),
        // MUST NOT: a chained command that merely begins with the prefix.
        shell_request("chained", "git status && rm -rf /"),
        // MUST NOT: a substring of the prefix token.
        shell_request("substring", "git statusfoo"),
        // MUST NOT: a leading assignment changes what runs.
        shell_request("assignment", "GIT_DIR=/etc git status"),
    ];

    let result = harness.decide(&requests, BioRouterMode::Approve).await;

    assert_eq!(ids(&result.approved), vec!["allowed"]);
    assert_eq!(
        ids(&result.needs_approval),
        vec!["push", "chained", "substring", "assignment"]
    );
    assert!(result.denied.is_empty());
}

/// A `/work/a` directory grant approves edits inside it, and *only* inside it.
#[tokio::test]
async fn a_directory_grant_approves_inside_and_prompts_outside() {
    let temp = TempDir::new().unwrap();
    let work = temp.path().join("work");
    fs::create_dir_all(work.join("a/nested")).unwrap();
    fs::create_dir_all(work.join("ab")).unwrap();
    fs::create_dir_all(work.join("b")).unwrap();

    let harness = Harness::new(temp.path().to_path_buf());
    harness.grant(
        EDITOR,
        PermissionScope::directory(work.join("a")).unwrap(),
        true,
    );

    let requests = vec![
        write_request("inside", &work.join("a/notes.md")),
        write_request("nested", &work.join("a/nested/deep/new.txt")),
        // MUST NOT: /work/a does not contain /work/ab.
        write_request("sibling", &work.join("ab/secret.txt")),
        // MUST NOT: an outright different directory.
        write_request("outside", &work.join("b/secret.txt")),
        // MUST NOT: `..` must not walk out of the scope.
        write_request("traversal", &work.join("a/../b/secret.txt")),
        // MUST NOT: a directory grant never authorizes a shell command, even one
        // whose text mentions the granted directory.
        shell_request("shell", &format!("rm -rf {}", work.join("a").display())),
    ];

    let result = harness.decide(&requests, BioRouterMode::Approve).await;

    assert_eq!(ids(&result.approved), vec!["inside", "nested"]);
    assert_eq!(
        ids(&result.needs_approval),
        vec!["sibling", "outside", "traversal", "shell"]
    );
    assert!(result.denied.is_empty());
}

/// The grant is keyed on the tool it was made for: a `text_editor` directory
/// grant says nothing about `shell`, and vice versa.
#[tokio::test]
async fn a_grant_does_not_transfer_to_another_tool() {
    let temp = TempDir::new().unwrap();
    let work = temp.path().join("work");
    fs::create_dir_all(&work).unwrap();

    let harness = Harness::new(temp.path().to_path_buf());
    harness.grant(EDITOR, PermissionScope::directory(&work).unwrap(), true);
    harness.grant(SHELL, PermissionScope::command_prefix("ls").unwrap(), true);

    let requests = vec![
        write_request("editor", &work.join("x.txt")),
        shell_request("shell_ls", "ls -la"),
        // The editor's directory grant must not reach the shell...
        shell_request("shell_rm", &format!("rm {}", work.join("x.txt").display())),
        // ...and the shell's `ls` prefix must not reach the editor, whose own
        // `command` argument ("write") is not a shell command line.
        request(
            "editor_ls",
            EDITOR,
            object!({"command": "ls", "path": "/etc/passwd"}),
        ),
    ];

    let result = harness.decide(&requests, BioRouterMode::Approve).await;

    assert_eq!(ids(&result.approved), vec!["editor", "shell_ls"]);
    assert_eq!(ids(&result.needs_approval), vec!["shell_rm", "editor_ls"]);
}

/// A scoped grant is subordinate to every stronger decision above it.
#[tokio::test]
async fn a_scoped_grant_cannot_override_a_user_deny_or_the_extension_gate() {
    let temp = TempDir::new().unwrap();
    let harness = Harness::new(temp.path().to_path_buf());

    // The user has said "never allow this tool". A scope must not resurrect it.
    harness
        .permission_manager
        .update_user_permission(SHELL, PermissionLevel::NeverAllow);
    harness.grant(
        SHELL,
        PermissionScope::command_prefix("git status").unwrap(),
        true,
    );
    // Extension management is always a human decision, whatever is granted.
    harness.grant(
        MANAGE_EXTENSIONS,
        PermissionScope::command_prefix("enable").unwrap(),
        true,
    );

    let requests = vec![
        shell_request("denied", "git status"),
        request(
            "extensions",
            MANAGE_EXTENSIONS,
            object!({"command": "enable", "extension_name": "evil"}),
        ),
    ];

    let result = harness.decide(&requests, BioRouterMode::Approve).await;

    assert!(result.approved.is_empty());
    assert_eq!(ids(&result.denied), vec!["denied"]);
    assert_eq!(ids(&result.needs_approval), vec!["extensions"]);
}

/// A scoped *deny* is honoured, and beats an overlapping broader allow.
#[tokio::test]
async fn a_scoped_deny_beats_an_overlapping_allow() {
    let temp = TempDir::new().unwrap();
    let harness = Harness::new(temp.path().to_path_buf());
    harness.grant(SHELL, PermissionScope::command_prefix("git").unwrap(), true);
    harness.grant(
        SHELL,
        PermissionScope::command_prefix("git push").unwrap(),
        false,
    );

    let requests = vec![
        shell_request("status", "git status"),
        shell_request("push", "git push"),
    ];
    let result = harness.decide(&requests, BioRouterMode::Approve).await;

    assert_eq!(ids(&result.approved), vec!["status"]);
    assert_eq!(ids(&result.denied), vec!["push"]);
}

/// The kill switch restores the pre-BR-24 behaviour exactly: every grant is
/// ignored and the call is confirmed as before.
#[tokio::test]
async fn the_kill_switch_ignores_every_scoped_grant() {
    let temp = TempDir::new().unwrap();
    let mut harness = Harness::new(temp.path().to_path_buf());
    harness.grant(
        SHELL,
        PermissionScope::command_prefix("git status").unwrap(),
        true,
    );

    // Rebuild the inspector with the tier disabled, same store.
    let risks = Arc::new(ToolRiskRegistry::new());
    risks.refresh_from_tools(&[tool(SHELL, false, true)]);
    let managed = no_managed();
    let mut manager = ToolInspectionManager::new();
    manager.add_inspector(Box::new(ManagedPolicyInspector::new(Arc::clone(&managed))));
    manager.add_inspector(Box::new(
        PermissionInspector::with_smart_config(
            risks,
            Arc::clone(&harness.permission_manager),
            managed,
            Arc::new(tokio::sync::Mutex::new(None)),
            SmartApproveConfig::default(),
        )
        .with_scoped_store(Arc::clone(&harness.store), false),
    ));
    harness.manager = manager;

    let requests = vec![shell_request("status", "git status")];
    let result = harness.decide(&requests, BioRouterMode::Approve).await;

    assert!(result.approved.is_empty());
    assert_eq!(ids(&result.needs_approval), vec!["status"]);
}
