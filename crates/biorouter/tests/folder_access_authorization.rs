use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use biorouter::agents::tool_errors::{classify_error_data, ToolErrorKind};
use biorouter::conversation::message::{ActionRequiredData, MessageContent};
use biorouter::folder_access::{FileAdmission, FolderAccessManager};
use biorouter::pending_user_action::{
    DecisionAuthority, PendingUserActions, ResolveOutcome, UserActionOutcome, UserActionRequest,
};
use biorouter::permission::Permission;
use biorouter_mcp::{FileAccessPlan, FileAccessRequest};
use biorouter_sandbox::{bounded_fs::authorized_path, AccessMode};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct Fixture {
    _temp: TempDir,
    cwd: PathBuf,
    external: PathBuf,
    session: String,
    manager: Arc<FolderAccessManager>,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("workspace");
        let external = temp.path().join("connector-results");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(&external).unwrap();
        // macOS /var aliases must not accidentally change the requested scope.
        let cwd = cwd.canonicalize().unwrap();
        let external = external.canonicalize().unwrap();
        std::fs::write(external.join("completed.csv"), "gene,value\nSYNTHETIC,1\n").unwrap();
        let session = Uuid::new_v4().to_string();
        let manager = Arc::new(FolderAccessManager::default());
        manager.pin_workspace(&session, &cwd).unwrap();
        Self {
            _temp: temp,
            cwd,
            external,
            session,
            manager,
        }
    }

    fn start(
        &self,
        requests: Vec<(PathBuf, AccessMode)>,
        cancel: CancellationToken,
    ) -> tokio::task::JoinHandle<anyhow::Result<FileAdmission>> {
        self.start_plan(
            FileAccessPlan {
                requests: requests
                    .into_iter()
                    .map(|(path, access)| FileAccessRequest {
                        path,
                        access,
                        recursive: false,
                    })
                    .collect(),
            },
            cancel,
        )
    }

    fn start_plan(
        &self,
        plan: FileAccessPlan,
        cancel: CancellationToken,
    ) -> tokio::task::JoinHandle<anyhow::Result<FileAdmission>> {
        let manager = self.manager.clone();
        let session = self.session.clone();
        let cwd = self.cwd.clone();
        tokio::spawn(biorouter::native_client::carry(async move {
            manager
                .admit(&session, "developer__text_editor", &cwd, &plan, &cancel)
                .await
        }))
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
        .expect("the pending operation must publish its approval card")
    }

    fn answer(&self, id: &str, permission: Permission) -> ResolveOutcome {
        let outcome = if matches!(permission, Permission::AllowOnce | Permission::AlwaysAllow) {
            UserActionOutcome::Approved { permission }
        } else {
            UserActionOutcome::Denied { permission }
        };
        PendingUserActions::global().resolve_in_session(
            &self.session,
            id,
            outcome,
            DecisionAuthority::from_local_human_surface(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        PendingUserActions::global().cancel_session(&self.session);
        self.manager.end_session(&self.session);
    }
}

fn path(
    admission: &FileAdmission,
    session: &str,
    requested: &Path,
    access: AccessMode,
) -> std::io::Result<biorouter_sandbox::BoundedPath> {
    authorized_path(
        admission.authority.token(),
        session,
        &admission.action_id,
        requested,
        access,
    )
}

#[tokio::test]
async fn completed_connector_path_is_read_in_place_after_exact_human_approval() {
    let fixture = Fixture::new();
    let returned_path = fixture.external.join("completed.csv");
    let job = fixture.start(
        vec![(returned_path.clone(), AccessMode::Read)],
        CancellationToken::new(),
    );
    let id = fixture.pending().await;
    let Some(UserActionRequest::FolderAccess(request)) = PendingUserActions::global().peek(&id)
    else {
        panic!("typed folder request expected")
    };
    assert_eq!(request.folder, fixture.external.display().to_string());
    assert_eq!(request.access, "read-only");
    assert_eq!(request.files, vec![returned_path.display().to_string()]);
    assert_eq!(
        fixture.answer(&id, Permission::AllowOnce),
        ResolveOutcome::Delivered
    );
    let admission = job.await.unwrap().unwrap();
    assert_eq!(
        path(
            &admission,
            &fixture.session,
            &returned_path,
            AccessMode::Read
        )
        .unwrap()
        .read_to_string()
        .unwrap(),
        "gene,value\nSYNTHETIC,1\n"
    );
    assert!(
        !fixture.cwd.join("completed.csv").exists(),
        "no shell copy or staging workaround"
    );
    assert!(fixture.manager.list_session(&fixture.session).is_empty());
    assert!(path(
        &admission,
        &fixture.session,
        &returned_path,
        AccessMode::Write
    )
    .is_err());
    assert!(path(
        &admission,
        "another-session",
        &returned_path,
        AccessMode::Read
    )
    .is_err());
    assert_eq!(
        fixture.answer(&id, Permission::AllowOnce),
        ResolveOutcome::Unknown
    );
    let token = admission.authority.token().to_string();
    let action = admission.action_id.clone();
    drop(admission);
    assert!(authorized_path(
        &token,
        &fixture.session,
        &action,
        &returned_path,
        AccessMode::Read
    )
    .is_err());
    let next = fixture.start(
        vec![(returned_path, AccessMode::Read)],
        CancellationToken::new(),
    );
    let next_id = fixture.pending().await;
    assert_ne!(
        id, next_id,
        "Allow once is never remembered as a folder grant"
    );
    fixture.answer(&next_id, Permission::DenyOnce);
    assert!(next.await.unwrap().is_err());
}

#[tokio::test]
async fn once_patch_scope_keeps_base_read_only_and_children_exact() {
    let fixture = Fixture::new();
    let target = fixture.external.join("patch.txt");
    let other = fixture.external.join("unlisted.txt");
    let job = fixture.start(
        vec![
            (fixture.external.clone(), AccessMode::Read),
            (target.clone(), AccessMode::Write),
        ],
        CancellationToken::new(),
    );
    let id = fixture.pending().await;
    fixture.answer(&id, Permission::AllowOnce);
    let admission = job.await.unwrap().unwrap();
    path(&admission, &fixture.session, &target, AccessMode::Write)
        .unwrap()
        .atomic_write(b"approved")
        .unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"approved");
    assert!(path(&admission, &fixture.session, &other, AccessMode::Write).is_err());
    assert!(path(
        &admission,
        &fixture.session,
        &fixture.external,
        AccessMode::Write
    )
    .is_err());
}

#[tokio::test]
async fn session_grant_resumes_automatically_and_revokes_already_bound_access() {
    let fixture = Fixture::new();
    let target = fixture.external.join("completed.csv");
    let job = fixture.start(
        vec![(target.clone(), AccessMode::Write)],
        CancellationToken::new(),
    );
    let id = fixture.pending().await;
    fixture.answer(&id, Permission::AlwaysAllow);
    let first = job.await.unwrap().unwrap();
    let held = path(&first, &fixture.session, &target, AccessMode::Write).unwrap();
    let second = tokio::time::timeout(
        Duration::from_secs(1),
        fixture.start(
            vec![(fixture.external.join("new.txt"), AccessMode::Write)],
            CancellationToken::new(),
        ),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    assert!(!PendingUserActions::global().has_pending_in_session(&fixture.session));
    let grants = fixture.manager.list_session(&fixture.session);
    assert_eq!(grants.len(), 1);
    assert!(grants[0].descendants);
    assert_eq!(grants[0].access, "read/write");
    assert!(fixture.manager.revoke(&fixture.session, &grants[0].id));
    assert!(!fixture.manager.revoke(&fixture.session, &grants[0].id));
    assert!(held.atomic_write(b"must not write after revoke").is_err());
    assert!(path(
        &second,
        &fixture.session,
        &fixture.external.join("new.txt"),
        AccessMode::Write
    )
    .is_err());
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "gene,value\nSYNTHETIC,1\n"
    );
}

#[tokio::test]
async fn read_grant_cannot_silently_upgrade_to_write() {
    let fixture = Fixture::new();
    let target = fixture.external.join("completed.csv");
    let read = fixture.start(
        vec![(target.clone(), AccessMode::Read)],
        CancellationToken::new(),
    );
    let id = fixture.pending().await;
    fixture.answer(&id, Permission::AlwaysAllow);
    let _read = read.await.unwrap().unwrap();
    let write = fixture.start(vec![(target, AccessMode::Write)], CancellationToken::new());
    let id = fixture.pending().await;
    let Some(UserActionRequest::FolderAccess(request)) = PendingUserActions::global().peek(&id)
    else {
        panic!()
    };
    assert_eq!(request.access, "read/write");
    fixture.answer(&id, Permission::DenyOnce);
    assert!(write.await.unwrap().is_err());
}

#[tokio::test]
async fn denied_folder_is_not_reprompted_until_the_next_user_turn() {
    let fixture = Fixture::new();
    let target = fixture.external.join("completed.csv");
    let job = fixture.start(
        vec![(target.clone(), AccessMode::Read)],
        CancellationToken::new(),
    );
    let id = fixture.pending().await;
    fixture.answer(&id, Permission::DenyOnce);
    let denied = job.await.unwrap().err().unwrap();
    let denied = denied.downcast_ref::<rmcp::model::ErrorData>().unwrap();
    assert!(denied.message.contains("Access was not approved."));
    let classified = classify_error_data(denied);
    assert_eq!(classified.kind, ToolErrorKind::PermissionDenied);
    assert!(!classified.retryable);
    assert!(fixture.manager.list_session(&fixture.session).is_empty());
    let refused = tokio::time::timeout(
        Duration::from_secs(1),
        fixture.start(
            vec![(target.clone(), AccessMode::Read)],
            CancellationToken::new(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let refused = refused.err().unwrap();
    let refused = refused.downcast_ref::<rmcp::model::ErrorData>().unwrap();
    assert!(refused
        .message
        .contains("Access was not approved for this folder during the current turn."));
    assert_eq!(
        classify_error_data(refused).kind,
        ToolErrorKind::PermissionDenied
    );
    assert!(!PendingUserActions::global().has_pending_in_session(&fixture.session));
    fixture.manager.begin_turn(&fixture.session);
    let job = fixture.start(vec![(target, AccessMode::Read)], CancellationToken::new());
    let id = fixture.pending().await;
    fixture.answer(&id, Permission::AllowOnce);
    assert!(job.await.unwrap().is_ok());
}

#[tokio::test]
async fn unproven_or_foreign_approvals_do_not_release_the_operation_and_cancel_clears_it() {
    let fixture = Fixture::new();
    let cancel = CancellationToken::new();
    let job = fixture.start(
        vec![(fixture.external.join("completed.csv"), AccessMode::Read)],
        cancel.clone(),
    );
    let id = fixture.pending().await;
    let approved = UserActionOutcome::Approved {
        permission: Permission::AlwaysAllow,
    };
    assert_eq!(
        PendingUserActions::global().resolve_in_session(
            "foreign",
            &id,
            approved.clone(),
            DecisionAuthority::unproven()
        ),
        ResolveOutcome::Unknown
    );
    assert_ne!(
        PendingUserActions::global().resolve_in_session(
            &fixture.session,
            &id,
            approved,
            DecisionAuthority::unproven()
        ),
        ResolveOutcome::Delivered
    );
    assert!(PendingUserActions::global().is_pending(&id));
    cancel.cancel();
    assert!(job.await.unwrap().is_err());
    assert!(!PendingUserActions::global().is_pending(&id));
    assert!(fixture.manager.list_session(&fixture.session).is_empty());
}

#[tokio::test]
async fn unattended_external_access_returns_bounded_recovery_without_parking() {
    let fixture = Fixture::new();
    let plan = FileAccessPlan {
        requests: vec![FileAccessRequest {
            path: fixture.external.join("completed.csv"),
            access: AccessMode::Read,
            recursive: false,
        }],
    };
    let cancel = CancellationToken::new();
    let result = biorouter::user_surface::without_human_surface(fixture.manager.admit(
        &fixture.session,
        "developer__text_editor",
        &fixture.cwd,
        &plan,
        &cancel,
    ))
    .await;
    let error = result.err().unwrap();
    let error = error.downcast_ref::<rmcp::model::ErrorData>().unwrap();
    assert_eq!(
        error.data.as_ref().unwrap()["code"],
        "authorization_required"
    );
    assert_eq!(
        classify_error_data(error).kind,
        ToolErrorKind::PermissionDenied
    );
    assert!(!classify_error_data(error).retryable);
    assert!(!error.message.contains("refused by the user"));
    assert!(error.data.as_ref().unwrap()["recovery"].is_string());
    assert!(!PendingUserActions::global().has_pending_in_session(&fixture.session));
}

#[tokio::test]
async fn concurrent_operations_share_one_session_folder_decision() {
    let fixture = Fixture::new();
    let first = fixture.start(
        vec![(fixture.external.join("one.txt"), AccessMode::Write)],
        CancellationToken::new(),
    );
    let id = fixture.pending().await;
    let second = fixture.start(
        vec![(fixture.external.join("two.txt"), AccessMode::Write)],
        CancellationToken::new(),
    );
    tokio::task::yield_now().await;
    assert_eq!(
        PendingUserActions::global()
            .pending_cards_for_session(&fixture.session)
            .len(),
        1
    );
    fixture.answer(&id, Permission::AlwaysAllow);
    assert!(first.await.unwrap().is_ok());
    assert!(second.await.unwrap().is_ok());
    assert_eq!(fixture.manager.list_session(&fixture.session).len(), 1);
    assert!(!PendingUserActions::global().has_pending_in_session(&fixture.session));
}

#[cfg(unix)]
#[tokio::test]
async fn workspace_link_asks_for_actual_folder_and_does_not_follow_later_retargeting() {
    let fixture = Fixture::new();
    let other = fixture.cwd.parent().unwrap().join("other-results");
    std::fs::create_dir(&other).unwrap();
    let link = fixture.cwd.join("results-link");
    std::os::unix::fs::symlink(&fixture.external, &link).unwrap();
    let requested = link.join("created.txt");
    let job = fixture.start(
        vec![(requested.clone(), AccessMode::Write)],
        CancellationToken::new(),
    );
    let id = fixture.pending().await;
    let Some(UserActionRequest::FolderAccess(request)) = PendingUserActions::global().peek(&id)
    else {
        panic!()
    };
    assert_eq!(request.folder, fixture.external.display().to_string());
    assert!(request.files[0].contains("requested as"));
    assert!(request.files[0].contains(requested.to_str().unwrap()));
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&other, &link).unwrap();
    fixture.answer(&id, Permission::AllowOnce);
    let admission = job.await.unwrap().unwrap();
    path(&admission, &fixture.session, &requested, AccessMode::Write)
        .unwrap()
        .atomic_write(b"approved original target")
        .unwrap();
    assert_eq!(
        std::fs::read(fixture.external.join("created.txt")).unwrap(),
        b"approved original target"
    );
    assert!(!other.join("created.txt").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn dangling_link_target_can_be_created_after_folder_approval() {
    let fixture = Fixture::new();
    let requested = fixture.cwd.join("dangling.txt");
    let actual = fixture.external.join("new-file.txt");
    std::os::unix::fs::symlink(&actual, &requested).unwrap();
    let job = fixture.start(
        vec![(requested.clone(), AccessMode::Write)],
        CancellationToken::new(),
    );
    let id = fixture.pending().await;
    fixture.answer(&id, Permission::AllowOnce);
    let admission = job.await.unwrap().unwrap();
    path(&admission, &fixture.session, &requested, AccessMode::Write)
        .unwrap()
        .atomic_write(b"synthetic new file")
        .unwrap();
    assert_eq!(std::fs::read(actual).unwrap(), b"synthetic new file");
}

#[tokio::test]
async fn recursive_analysis_once_discloses_descendants_and_stays_read_only() {
    let fixture = Fixture::new();
    let child = fixture.external.join("subfolder/source.rs");
    std::fs::create_dir(child.parent().unwrap()).unwrap();
    std::fs::write(&child, "fn synthetic() {}\n").unwrap();
    let job = fixture.start_plan(
        FileAccessPlan {
            requests: vec![FileAccessRequest {
                path: fixture.external.clone(),
                access: AccessMode::Read,
                recursive: true,
            }],
        },
        CancellationToken::new(),
    );
    let id = fixture.pending().await;
    let Some(UserActionRequest::FolderAccess(request)) = PendingUserActions::global().peek(&id)
    else {
        panic!()
    };
    assert!(request.once_descendants);
    assert!(request.prompt().contains("read-only analysis"));
    fixture.answer(&id, Permission::AllowOnce);
    let admission = job.await.unwrap().unwrap();
    assert_eq!(
        path(&admission, &fixture.session, &child, AccessMode::Read)
            .unwrap()
            .read_to_string()
            .unwrap(),
        "fn synthetic() {}\n"
    );
    assert!(path(&admission, &fixture.session, &child, AccessMode::Write).is_err());
    assert!(fixture.manager.list_session(&fixture.session).is_empty());
}

async fn approve_for_native_client(
    fixture: &Fixture,
    target: &Path,
    owner: biorouter::native_client::NativeClient,
    other: biorouter::native_client::NativeClient,
) {
    let (job,) = biorouter::native_client::scope(Some(owner.clone()), async {
        (fixture.start(
            vec![(target.to_path_buf(), AccessMode::Read)],
            CancellationToken::new(),
        ),)
    })
    .await;
    let id = fixture.pending().await;
    assert_eq!(
        fixture.answer(&id, Permission::AlwaysAllow),
        ResolveOutcome::Unproven
    );
    assert_eq!(
        PendingUserActions::global().resolve_in_session(
            &fixture.session,
            &id,
            UserActionOutcome::Approved {
                permission: Permission::AlwaysAllow
            },
            DecisionAuthority::from_native_folder_access_proof(&other),
        ),
        ResolveOutcome::Unproven
    );
    assert_eq!(
        PendingUserActions::global().resolve_in_session(
            &fixture.session,
            &id,
            UserActionOutcome::Approved {
                permission: Permission::AlwaysAllow
            },
            DecisionAuthority::from_native_folder_access_proof(&owner),
        ),
        ResolveOutcome::Delivered
    );
    job.await.unwrap().unwrap();
}

#[tokio::test]
async fn native_grants_coexist_but_cannot_cross_client_lifetimes() {
    use biorouter::native_client::{scope, NativeClientLease};
    let fixture = Fixture::new();
    let first = NativeClientLease::register().unwrap();
    let second = NativeClientLease::register().unwrap();
    let target = fixture.external.join("completed.csv");
    for (owner, other) in [
        (first.client(), second.client()),
        (second.client(), first.client()),
    ] {
        approve_for_native_client(&fixture, &target, owner, other).await;
    }
    assert!(fixture.manager.list_session(&fixture.session).is_empty());
    let first_views = scope(Some(first.client()), async {
        fixture.manager.list_session(&fixture.session)
    })
    .await;
    let second_views = scope(Some(second.client()), async {
        fixture.manager.list_session(&fixture.session)
    })
    .await;
    assert_eq!(first_views.len(), 1);
    assert_eq!(second_views.len(), 1);
    assert_ne!(first_views[0].id, second_views[0].id);
    assert!(
        !scope(Some(second.client()), async {
            fixture.manager.revoke(&fixture.session, &first_views[0].id)
        })
        .await
    );
    let old_owner = first.client();
    drop(first);
    assert!(scope(Some(old_owner), async {
        fixture.manager.list_session(&fixture.session)
    })
    .await
    .is_empty());
    assert_eq!(
        scope(Some(second.client()), async {
            fixture.manager.list_session(&fixture.session)
        })
        .await
        .len(),
        1
    );
    let next = NativeClientLease::register().unwrap();
    let denied = scope(
        Some(next.client()),
        biorouter::user_surface::without_human_surface(fixture.manager.admit(
            &fixture.session,
            "developer__text_editor",
            &fixture.cwd,
            &FileAccessPlan {
                requests: vec![FileAccessRequest {
                    path: target,
                    access: AccessMode::Read,
                    recursive: false,
                }],
            },
            &CancellationToken::new(),
        )),
    )
    .await;
    assert!(
        denied.is_err(),
        "a new app cannot inherit an old client's grant before cleanup"
    );
}

#[tokio::test]
async fn native_disconnect_revokes_retained_once_paths_and_derived_directories() {
    use biorouter::native_client::{scope, NativeClientLease};
    let fixture = Fixture::new();
    let lease = NativeClientLease::register().unwrap();
    let owner = lease.client();
    let (job,) = scope(Some(owner.clone()), async {
        (fixture.start_plan(
            FileAccessPlan {
                requests: vec![FileAccessRequest {
                    path: fixture.external.clone(),
                    access: AccessMode::Read,
                    recursive: true,
                }],
            },
            CancellationToken::new(),
        ),)
    })
    .await;
    let id = fixture.pending().await;
    assert_eq!(
        PendingUserActions::global().resolve_in_session(
            &fixture.session,
            &id,
            UserActionOutcome::Approved {
                permission: Permission::AllowOnce
            },
            DecisionAuthority::from_native_folder_access_proof(&owner),
        ),
        ResolveOutcome::Delivered
    );
    let admission = job.await.unwrap().unwrap();
    let directory = path(
        &admission,
        &fixture.session,
        &fixture.external,
        AccessMode::Read,
    )
    .unwrap();
    let derived = directory.open_dir().unwrap();
    let retained = path(
        &admission,
        &fixture.session,
        &fixture.external.join("completed.csv"),
        AccessMode::Read,
    )
    .unwrap();
    let child = derived.bind("completed.csv", AccessMode::Read).unwrap();
    assert!(retained.read_to_string().is_ok());
    drop(lease);
    assert!(retained.read_to_string().is_err());
    assert!(directory.list().is_err());
    assert!(child.read_to_string().is_err());
    assert!(derived.bind("completed.csv", AccessMode::Read).is_err());
    assert!(path(
        &admission,
        &fixture.session,
        &fixture.external.join("completed.csv"),
        AccessMode::Read
    )
    .is_err());
}

#[tokio::test]
async fn native_disconnect_releases_pending_folder_and_workspace_remains_available() {
    use biorouter::native_client::{scope, NativeClientLease};
    let fixture = Fixture::new();
    let lease = NativeClientLease::register().unwrap();
    let (job,) = scope(Some(lease.client()), async {
        (fixture.start(
            vec![(fixture.external.join("completed.csv"), AccessMode::Read)],
            CancellationToken::new(),
        ),)
    })
    .await;
    let id = fixture.pending().await;
    drop(lease);
    assert!(tokio::time::timeout(Duration::from_secs(1), job)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(!PendingUserActions::global().is_pending(&id));
    let local = fixture.cwd.join("local.txt");
    std::fs::write(&local, "WORKSPACE_CONTROL").unwrap();
    let admission = scope(
        Some(biorouter::native_client::NativeClient::unavailable()),
        biorouter::user_surface::without_human_surface(fixture.manager.admit(
            &fixture.session,
            "developer__text_editor",
            &fixture.cwd,
            &FileAccessPlan {
                requests: vec![FileAccessRequest {
                    path: local.clone(),
                    access: AccessMode::Read,
                    recursive: false,
                }],
            },
            &CancellationToken::new(),
        )),
    )
    .await
    .unwrap();
    assert_eq!(
        path(&admission, &fixture.session, &local, AccessMode::Read)
            .unwrap()
            .read_to_string()
            .unwrap(),
        "WORKSPACE_CONTROL"
    );
}
