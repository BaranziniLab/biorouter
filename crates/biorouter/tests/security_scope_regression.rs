//! Public permission-boundary replays for the defensive security audit.

use biorouter::config::permission::PermissionLevel;
use biorouter::config::PermissionManager;
use biorouter::permission::{CallFacts, PermissionScope};
use serde_json::json;

#[cfg(unix)]
#[test]
fn remembered_scope_refuses_unresolved_links_and_preserves_normal_new_files() {
    use std::os::unix::fs::symlink;

    let fixture = tempfile::tempdir().unwrap();
    let work = fixture.path().join("work");
    let outside = fixture.path().join("outside");
    std::fs::create_dir(&work).unwrap();
    std::fs::create_dir(&outside).unwrap();
    symlink(outside.join("new.txt"), work.join("file-link")).unwrap();
    symlink(outside.join("missing-dir"), work.join("dir-link")).unwrap();
    let scope = PermissionScope::directory(&work).unwrap();
    for path in ["file-link", "dir-link/new.txt", "new.txt"] {
        let args = json!({"path":path});
        let facts = CallFacts::for_tool_call("developer__text_editor", args.as_object(), &work);
        assert_eq!(scope.matches(&facts), path == "new.txt");
    }
}

#[test]
fn legacy_permission_file_does_not_restore_automatic_tool_wide_grants() {
    let fixture = tempfile::tempdir().unwrap();
    let permissions = PermissionManager::new(fixture.path().into());
    permissions.update_smart_approve_permission("automatic", PermissionLevel::AlwaysAllow);
    permissions.update_smart_approve_permission("denied", PermissionLevel::NeverAllow);
    permissions.update_smart_approve_permission("ask", PermissionLevel::AskBefore);
    permissions.update_user_permission("explicit", PermissionLevel::AlwaysAllow);
    let reopened = PermissionManager::new(fixture.path().into());
    assert_eq!(reopened.get_smart_approve_permission("automatic"), None);
    assert_eq!(
        reopened.get_smart_approve_permission("denied"),
        Some(PermissionLevel::NeverAllow)
    );
    assert_eq!(
        reopened.get_smart_approve_permission("ask"),
        Some(PermissionLevel::AskBefore)
    );
    assert_eq!(
        reopened.get_user_permission("explicit"),
        Some(PermissionLevel::AlwaysAllow)
    );
}
