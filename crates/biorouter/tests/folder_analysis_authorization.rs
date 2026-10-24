#![cfg(unix)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use biorouter::conversation::message::{ActionRequiredData, MessageContent};
use biorouter::folder_access::{FileAdmission, FolderAccessManager};
use biorouter::pending_user_action::{
    DecisionAuthority, PendingUserActions, ResolveOutcome, UserActionOutcome, UserActionRequest,
};
use biorouter::permission::{Permission, PermissionScope, ToolPermissionStore};
use biorouter_mcp::{file_access_plan, DeveloperServer};
use rmcp::model::{CallToolRequestParams, CallToolResult, Meta};
use rmcp::ServiceExt;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

static PATH_ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();

#[ctor::ctor]
fn isolate_test_paths() {
    if biorouter_mcp::paths::path_root_override().is_none() {
        let root = tempfile::tempdir().unwrap_or_else(|error| {
            eprintln!("Cannot isolate folder-analysis test paths: {error}");
            std::process::abort();
        });
        std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
        let _ = PATH_ROOT.set(root);
    }
}

struct Fixture {
    _temporary: tempfile::TempDir,
    workspace: PathBuf,
    library: PathBuf,
    session: String,
    manager: Arc<FolderAccessManager>,
    executions: Arc<AtomicUsize>,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let base = temporary.path().canonicalize().unwrap();
        let workspace = base.join("workspace");
        let library = base.join("outside-library");
        for path in [&workspace, &library] {
            std::fs::create_dir(path).unwrap();
        }
        std::fs::write(workspace.join("normal.rs"), "fn workspace_control() {}\n").unwrap();
        std::fs::write(
            library.join("outside_source.rs"),
            "fn approved_library() {}\n",
        )
        .unwrap();
        let session = Uuid::new_v4().to_string();
        let manager = Arc::new(FolderAccessManager::default());
        manager.pin_workspace(&session, &workspace).unwrap();
        Self {
            _temporary: temporary,
            workspace,
            library,
            session,
            manager,
            executions: Arc::default(),
        }
    }

    fn link_library(&self) -> PathBuf {
        let link = self.workspace.join("vendor-link");
        std::os::unix::fs::symlink(&self.library, &link).unwrap();
        link
    }

    fn start(
        &self,
        depth: u32,
        cancel: CancellationToken,
    ) -> tokio::task::JoinHandle<anyhow::Result<CallToolResult>> {
        self.start_with_server(
            depth,
            cancel,
            DeveloperServer::new().with_working_dir(self.workspace.clone()),
        )
    }

    fn start_with_server(
        &self,
        depth: u32,
        cancel: CancellationToken,
        server: DeveloperServer,
    ) -> tokio::task::JoinHandle<anyhow::Result<CallToolResult>> {
        let workspace = self.workspace.clone();
        let session = self.session.clone();
        let manager = self.manager.clone();
        let executions = self.executions.clone();
        tokio::spawn(async move {
            let arguments = serde_json::json!({"path": workspace, "max_depth": depth});
            let plan = file_access_plan("analyze", &arguments, &workspace)?.unwrap();
            let admission = manager
                .admit_analysis(
                    &session,
                    "developer__analyze",
                    &workspace,
                    &plan,
                    &cancel,
                    depth,
                )
                .await?;
            executions.fetch_add(1, Ordering::SeqCst);
            execute_analysis(server, &session, &admission, arguments).await
        })
    }

    async fn pending(&self) -> String {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                for message in PendingUserActions::global().pending_cards_for_session(&self.session)
                {
                    for content in message.content {
                        if let MessageContent::ActionRequired(action) = content {
                            if let ActionRequiredData::ToolConfirmation { id, .. } = action.data {
                                return id;
                            }
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("analysis must publish the normal folder approval card")
    }

    fn answer(&self, id: &str, permission: Permission) {
        let outcome = if matches!(permission, Permission::AllowOnce | Permission::AlwaysAllow) {
            UserActionOutcome::Approved { permission }
        } else {
            UserActionOutcome::Denied { permission }
        };
        assert_eq!(
            PendingUserActions::global().resolve_in_session(
                &self.session,
                id,
                outcome,
                DecisionAuthority::from_local_human_surface()
            ),
            ResolveOutcome::Delivered
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        PendingUserActions::global().cancel_session(&self.session);
        self.manager.end_session(&self.session);
    }
}

async fn execute_analysis(
    server: DeveloperServer,
    session: &str,
    admission: &FileAdmission,
    arguments: serde_json::Value,
) -> anyhow::Result<CallToolResult> {
    let (client_io, server_io) = tokio::io::duplex(65536);
    let serving =
        tokio::spawn(async move { server.serve(server_io).await.unwrap().waiting().await });
    let client = ().serve(client_io).await?;
    let mut meta = Meta::default();
    for (key, value) in [
        ("biorouter-session-id", session),
        ("biorouter-file-access-action", &admission.action_id),
        ("biorouter-file-access-token", admission.authority.token()),
    ] {
        meta.0.insert(key.into(), value.into());
    }
    let result = client
        .call_tool(CallToolRequestParams {
            name: "analyze".into(),
            arguments: arguments.as_object().cloned(),
            meta: Some(meta),
            task: None,
        })
        .await;
    let _ = client.cancel().await;
    let _ = serving.await;
    Ok(result?)
}

#[tokio::test]
async fn workspace_analysis_prompts_for_external_child_and_resumes_original_tool() {
    let fixture = Fixture::new();
    let link = fixture.link_library();
    let job = fixture.start(2, CancellationToken::new());
    let id = fixture.pending().await;
    let Some(UserActionRequest::FolderAccess(request)) = PendingUserActions::global().peek(&id)
    else {
        panic!("typed folder request expected")
    };
    assert_eq!(request.folder, fixture.library.display().to_string());
    assert_eq!(request.access, "read-only");
    assert!(request.once_descendants);
    assert!(request.files[0].contains(&link.display().to_string()));
    assert_eq!(
        fixture.executions.load(Ordering::SeqCst),
        0,
        "the source-reading executor must not run before approval"
    );
    assert!(!job.is_finished());
    let replacement = fixture
        .workspace
        .parent()
        .unwrap()
        .join("unapproved-library");
    std::fs::create_dir(&replacement).unwrap();
    std::fs::write(
        replacement.join("unapproved_canary.rs"),
        "fn must_not_read() {}\n",
    )
    .unwrap();
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&replacement, &link).unwrap();
    fixture.answer(&id, Permission::AllowOnce);
    let result = tokio::time::timeout(Duration::from_secs(3), job)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert_ne!(result.is_error, Some(true));
    assert!(text.contains("normal.rs"));
    assert!(text.contains("outside_source.rs"));
    assert!(!text.contains("unapproved_canary"));
    assert_eq!(fixture.executions.load(Ordering::SeqCst), 1);
    assert!(fixture.manager.list_session(&fixture.session).is_empty());
    assert!(!PendingUserActions::global().has_pending_in_session(&fixture.session));
}

#[tokio::test]
async fn declining_or_cancelling_external_analysis_never_runs_the_executor() {
    for deny in [true, false] {
        let fixture = Fixture::new();
        fixture.link_library();
        let cancel = CancellationToken::new();
        let job = fixture.start(2, cancel.clone());
        let id = fixture.pending().await;
        if deny {
            fixture.answer(&id, Permission::DenyOnce);
        } else {
            cancel.cancel();
        }
        assert!(job.await.unwrap().is_err());
        assert_eq!(fixture.executions.load(Ordering::SeqCst), 0);
        assert!(!PendingUserActions::global().has_pending_in_session(&fixture.session));
        assert!(fixture.manager.list_session(&fixture.session).is_empty());
        let repeat = fixture.start(2, CancellationToken::new());
        assert!(tokio::time::timeout(Duration::from_secs(3), repeat)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        assert!(!PendingUserActions::global().has_pending_in_session(&fixture.session));
    }
}

#[tokio::test]
async fn nested_external_links_request_each_folder_before_the_original_analysis_runs() {
    let fixture = Fixture::new();
    fixture.link_library();
    let second = fixture.workspace.parent().unwrap().join("second-library");
    std::fs::create_dir(&second).unwrap();
    std::fs::write(second.join("second_source.rs"), "fn second_approved() {}\n").unwrap();
    std::os::unix::fs::symlink(&second, fixture.library.join("second-link")).unwrap();
    let job = fixture.start(3, CancellationToken::new());
    let first = fixture.pending().await;
    fixture.answer(&first, Permission::AllowOnce);
    let second_id = fixture.pending().await;
    assert_ne!(first, second_id);
    let Some(UserActionRequest::FolderAccess(request)) =
        PendingUserActions::global().peek(&second_id)
    else {
        panic!()
    };
    assert_eq!(request.folder, second.display().to_string());
    assert_eq!(fixture.executions.load(Ordering::SeqCst), 0);
    fixture.answer(&second_id, Permission::AllowOnce);
    let text = serde_json::to_string(&job.await.unwrap().unwrap()).unwrap();
    for file in ["normal.rs", "outside_source.rs", "second_source.rs"] {
        assert!(text.contains(file), "{file}");
    }
}

#[tokio::test]
async fn ignored_and_out_of_depth_links_do_not_request_external_access() {
    for ignored in [true, false] {
        let fixture = Fixture::new();
        let depth = if ignored {
            fixture.link_library();
            std::fs::write(fixture.workspace.join(".biorouterignore"), "vendor-link\n").unwrap();
            3
        } else {
            let nested = fixture.workspace.join("nested");
            std::fs::create_dir(&nested).unwrap();
            std::os::unix::fs::symlink(&fixture.library, nested.join("vendor-link")).unwrap();
            1
        };
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            fixture.start(depth, CancellationToken::new()),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        let text = serde_json::to_string(&result).unwrap();
        assert!(text.contains("normal.rs"));
        assert!(!text.contains("outside_source.rs"));
        assert!(!PendingUserActions::global().has_pending_in_session(&fixture.session));
    }
}

#[tokio::test]
async fn workspace_ignore_patterns_keep_applying_below_an_approved_directory_alias() {
    let fixture = Fixture::new();
    fixture.link_library();
    let server = DeveloperServer::new().with_working_dir(fixture.workspace.clone());
    std::fs::write(
        fixture.library.join("private.rs"),
        "fn private_canary() {}\n",
    )
    .unwrap();
    std::fs::write(
        fixture.workspace.join(".biorouterignore"),
        "vendor-link/private.rs\nvendor-link/blocked\n",
    )
    .unwrap();
    let unapproved = fixture.workspace.parent().unwrap().join("blocked-library");
    std::fs::create_dir(&unapproved).unwrap();
    std::fs::write(unapproved.join("blocked.rs"), "fn blocked_canary() {}\n").unwrap();
    std::os::unix::fs::symlink(&unapproved, fixture.library.join("blocked")).unwrap();
    let job = fixture.start_with_server(3, CancellationToken::new(), server);
    let id = fixture.pending().await;
    fixture.answer(&id, Permission::AllowOnce);
    let result = tokio::time::timeout(Duration::from_secs(10), job)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(text.contains("normal.rs"));
    assert!(text.contains("outside_source.rs"));
    assert!(!text.contains("private.rs"));
    assert!(!text.contains("blocked.rs"));
    assert!(!PendingUserActions::global().has_pending_in_session(&fixture.session));
}

#[tokio::test]
async fn remembered_descendant_denial_cannot_be_overridden_by_recursive_folder_approval() {
    for outside in [false, true] {
        for missing in [false, true] {
            let fixture = Fixture::new();
            fixture.link_library();
            let root = if outside {
                &fixture.library
            } else {
                &fixture.workspace
            };
            let denied = root.join("private");
            std::fs::create_dir(&denied).unwrap();
            let mut permissions = ToolPermissionStore::load().unwrap();
            permissions
                .record_scoped_grant(
                    "developer__analyze",
                    PermissionScope::directory(root).unwrap(),
                    true,
                    None,
                )
                .unwrap();
            permissions
                .record_scoped_grant(
                    "developer__analyze",
                    PermissionScope::directory(&denied).unwrap(),
                    false,
                    None,
                )
                .unwrap();
            if missing {
                std::fs::remove_dir(&denied).unwrap();
            } else {
                std::fs::write(denied.join("source.rs"), "fn private_canary() {}\n").unwrap();
            }
            let error = tokio::time::timeout(
                Duration::from_secs(10),
                fixture.start(3, CancellationToken::new()),
            )
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
            assert!(error.to_string().contains("remembered directory denial"));
            assert_eq!(fixture.executions.load(Ordering::SeqCst), 0);
            assert!(!PendingUserActions::global().has_pending_in_session(&fixture.session));
            assert!(fixture.manager.list_session(&fixture.session).is_empty());
        }
    }
}
