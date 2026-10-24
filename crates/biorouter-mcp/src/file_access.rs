//! Shared file-target planning and capability-bound developer file access.
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use biorouter_sandbox::bounded_fs::{authorized_path, AccessMode, BoundedFs, BoundedPath};
use rmcp::model::{ErrorCode, ErrorData, Meta};

use crate::developer::shell::expand_path;
use crate::secret_guard::SecretGuard;

#[derive(Clone, Debug)]
pub struct FileAccessRequest {
    pub path: PathBuf,
    pub access: AccessMode,
}
#[derive(Clone, Debug, Default)]
pub struct FileAccessPlan {
    pub requests: Vec<FileAccessRequest>,
}

pub fn absolute_file_path(path: &str, cwd: &Path) -> Result<PathBuf> {
    let expanded = expand_path(path);
    let path = Path::new(&expanded);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let mut normalized = PathBuf::new();
    for part in joined.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    bail!("Path escapes its root");
                }
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    Ok(normalized)
}

pub fn file_access_plan(
    tool_name: &str,
    args: &serde_json::Value,
    cwd: &Path,
) -> Result<Option<FileAccessPlan>> {
    if tool_name.rsplit("__").next() != Some("text_editor") {
        return Ok(None);
    }
    let command = args
        .get("command")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("Missing editor command"))?;
    if !matches!(
        command,
        "view" | "write" | "str_replace" | "insert" | "undo_edit"
    ) {
        bail!("Unknown editor command '{command}'");
    }
    let path = args
        .get("path")
        .or_else(|| args.get("file_path"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("Missing editor path"))?;
    let path = absolute_file_path(path, cwd)?;
    let access = if command == "view" {
        AccessMode::Read
    } else {
        AccessMode::Write
    };
    if command == "str_replace" {
        if let Some(diff) = args.get("diff").and_then(serde_json::Value::as_str) {
            let targets = crate::developer::text_editor::diff_target_paths(&path, diff)
                .map_err(|error| anyhow!(error.message.to_string()))?;
            let mut requests = vec![FileAccessRequest {
                path,
                access: AccessMode::Read,
            }];
            requests.extend(targets.into_iter().map(|path| FileAccessRequest {
                path,
                access: AccessMode::Write,
            }));
            return Ok(Some(FileAccessPlan { requests }));
        }
    }
    Ok(Some(FileAccessPlan {
        requests: vec![FileAccessRequest { path, access }],
    }))
}

pub fn preflight_file_access(plan: &FileAccessPlan, cwd: &Path) -> Result<()> {
    let session_guard = SecretGuard::for_dir(cwd);
    for request in &plan.requests {
        if crate::memory::is_in_global_memory_store(&request.path) {
            bail!(
                "{}",
                crate::memory::global_memory_store_refusal(&request.path)
            );
        }
        let parent = request.path.parent().unwrap_or(cwd);
        if session_guard.is_denied_resolved(&request.path)
            || SecretGuard::for_dir(parent).is_denied_resolved(&request.path)
        {
            bail!(
                "Access to '{}' is restricted by secret-file policy",
                request.path.display()
            );
        }
    }
    Ok(())
}

#[derive(Clone)]
pub(crate) struct EditorAccess {
    pub workspace: Arc<BoundedFs>,
    pub skills: Option<Arc<BoundedFs>>,
    pub guard: SecretGuard,
    pub meta: Meta,
}

impl EditorAccess {
    pub fn bind(&self, requested: &Path, access: AccessMode) -> Result<EditorPath, ErrorData> {
        let string = |key: &str| self.meta.0.get(key).and_then(serde_json::Value::as_str);
        // A brokered dispatch must use the broker's retained objects even for
        // the primary workspace. Independently reopening the same label could
        // otherwise substitute a different directory after admission.
        let bound = if let Some(token) = string("biorouter-file-access-token") {
            let session =
                string("biorouter-session-id").ok_or_else(|| authorization_required(requested))?;
            let action = string("biorouter-file-access-action")
                .ok_or_else(|| authorization_required(requested))?;
            authorized_path(token, session, action, requested, access)
                .map_err(|_| authorization_required(requested))?
        } else {
            match self.workspace.bind(requested, access) {
                Ok(bound) => bound,
                Err(error) if requested.starts_with(self.workspace.root()) => {
                    return Err(file_error(format!("Cannot resolve '{}' inside the authorized directory: {error}. If another folder was intended, provide its actual path so access can be approved.", requested.display())));
                }
                Err(_)
                    if access == AccessMode::Read
                        && self
                            .skills
                            .as_ref()
                            .is_some_and(|root| requested.starts_with(root.root())) =>
                {
                    self.skills
                        .as_ref()
                        .unwrap()
                        .bind(requested, access)
                        .map_err(file_error)?
                }
                Err(_) => return Err(authorization_required(requested)),
            }
        };
        let actual = bound.absolute_path();
        // Match the spelling and the already-resolved capability identity. Do not
        // canonicalize a pathname again between the policy decision and the I/O.
        let root_guard = SecretGuard::for_bounded_path(&bound).map_err(file_error)?;
        if crate::memory::is_in_global_memory_store(&actual) {
            return Err(file_error(crate::memory::global_memory_store_refusal(
                &actual,
            )));
        }
        if self.guard.is_denied(requested)
            || self.guard.is_denied(&actual)
            || root_guard.is_denied(&actual)
        {
            return Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!(
                    "Access to '{}' is restricted by .biorouterignore or secret-file policy",
                    actual.display()
                ),
                None,
            ));
        }
        Ok(EditorPath {
            bound,
            access: self.clone(),
            display: actual,
        })
    }
}

pub(crate) struct EditorPath {
    pub bound: BoundedPath,
    pub access: EditorAccess,
    display: PathBuf,
}
impl std::ops::Deref for EditorPath {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.display
    }
}
impl EditorPath {
    pub fn read_to_string(&self) -> std::io::Result<String> {
        self.bound.read_to_string()
    }
    pub fn write(&self, content: impl AsRef<[u8]>) -> std::io::Result<()> {
        self.bound.atomic_write(content.as_ref()).map_err(|error| {
            let description = if error.kind() == std::io::ErrorKind::NotFound {
                "no longer exists"
            } else {
                "could not be written"
            };
            std::io::Error::new(
                error.kind(),
                format!(
                    "File destination '{}' {description}: {error}",
                    self.display()
                ),
            )
        })
    }
    pub fn exists(&self) -> bool {
        self.bound.exists().unwrap_or(false)
    }
    pub fn is_file(&self) -> bool {
        self.bound
            .metadata()
            .is_ok_and(|metadata| metadata.is_file())
    }
    pub fn is_dir(&self) -> bool {
        self.bound
            .metadata()
            .is_ok_and(|metadata| metadata.is_dir())
    }
}

pub(crate) fn file_error(error: impl std::fmt::Display) -> ErrorData {
    ErrorData::new(ErrorCode::INVALID_PARAMS, error.to_string(), None)
}
fn authorization_required(path: &Path) -> ErrorData {
    ErrorData::new(ErrorCode::INVALID_PARAMS, format!("authorization_required: This path is outside the working directory. Ask the user to grant access to the folder containing '{}' and retry this operation.", path.display()), Some(serde_json::json!({"type":"authorization_required", "path":path})))
}

#[cfg(test)]
pub(crate) fn test_editor_path(path: &Path) -> EditorPath {
    let root = if path.is_dir() {
        path
    } else {
        path.parent().unwrap()
    };
    let workspace = Arc::new(BoundedFs::open(root).unwrap());
    EditorAccess {
        workspace,
        skills: None,
        guard: SecretGuard::for_dir(root),
        meta: Meta::default(),
    }
    .bind(path, AccessMode::Write)
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::developer::text_editor::{
        apply_diff_bounded, save_file_history, text_editor_insert_bounded,
        text_editor_replace_bounded, text_editor_undo_bounded, text_editor_view_bounded,
        text_editor_write_bounded,
    };
    use crate::developer::undo_history::FileHistory;

    fn access(root: &Path) -> EditorAccess {
        EditorAccess {
            workspace: Arc::new(BoundedFs::open(root).unwrap()),
            skills: None,
            guard: SecretGuard::for_dir(root),
            meta: Meta::default(),
        }
    }

    #[test]
    fn planner_includes_every_patch_child_and_base_metadata() {
        let root = tempfile::tempdir().unwrap();
        let diff = "--- a/one.txt\n+++ b/one.txt\n@@ -0,0 +1 @@\n+one\n--- a/two.txt\n+++ b/two.txt\n@@ -0,0 +1 @@\n+two\n";
        let plan = file_access_plan(
            "developer__text_editor",
            &serde_json::json!({"command":"str_replace", "path":root.path(), "diff":diff}),
            root.path(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan.requests.len(), 3);
        assert_eq!(plan.requests[0].access, AccessMode::Read);
        assert_eq!(plan.requests[1].path, root.path().join("one.txt"));
        assert_eq!(plan.requests[2].path, root.path().join("two.txt"));
    }

    #[tokio::test]
    async fn patch_checks_all_children_before_creating_any_parent() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(".biorouterignore"), "blocked.txt\n").unwrap();
        let editor = access(root.path());
        let base = editor.bind(root.path(), AccessMode::Read).unwrap();
        let diff = "--- a/new/nested.txt\n+++ b/new/nested.txt\n@@ -0,0 +1 @@\n+safe\n--- a/blocked.txt\n+++ b/blocked.txt\n@@ -0,0 +1 @@\n+blocked\n";
        assert!(apply_diff_bounded(&base, diff, &FileHistory::in_memory())
            .await
            .is_err());
        assert!(!root.path().join("new").exists());
        assert!(!root.path().join("blocked.txt").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bound_editor_operations_keep_the_approved_parent_after_substitution() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("parent")).unwrap();
        std::fs::write(root.path().join("parent/file.txt"), "one\ntwo\n").unwrap();
        std::fs::write(outside.path().join("file.txt"), "OUTSIDE CANARY").unwrap();
        let editor = access(root.path());
        let file = editor
            .bind(&root.path().join("parent/file.txt"), AccessMode::Write)
            .unwrap();
        std::fs::rename(root.path().join("parent"), root.path().join("retained")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("parent")).unwrap();
        let history = FileHistory::in_memory();
        save_file_history(&file, &history).unwrap();
        text_editor_write_bounded(&file, "alpha\nbeta\n")
            .await
            .unwrap();
        text_editor_replace_bounded(&file, "alpha", "ALPHA", None, &None, &history)
            .await
            .unwrap();
        text_editor_insert_bounded(&file, 1, "middle", &history)
            .await
            .unwrap();
        let view = text_editor_view_bounded(&file, None).await.unwrap();
        assert!(serde_json::to_string(&view).unwrap().contains("ALPHA"));
        text_editor_undo_bounded(&file, &history).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("retained/file.txt")).unwrap(),
            "ALPHA\nbeta\n"
        );
        assert_eq!(
            std::fs::read_to_string(outside.path().join("file.txt")).unwrap(),
            "OUTSIDE CANARY"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn patch_and_new_parent_creation_follow_root_object_after_rename() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("workspace");
        let retained = fixture.path().join("retained");
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("existing.txt"), "before\n").unwrap();
        std::fs::write(outside.path().join("existing.txt"), "OUTSIDE CANARY").unwrap();
        let editor = access(&root);
        let base = editor.bind(&root, AccessMode::Read).unwrap();
        std::fs::rename(&root, &retained).unwrap();
        std::os::unix::fs::symlink(outside.path(), &root).unwrap();
        let diff = "--- a/existing.txt\n+++ b/existing.txt\n@@ -1 +1 @@\n-before\n+after\n--- a/new/nested.txt\n+++ b/new/nested.txt\n@@ -0,0 +1 @@\n+created\n";
        apply_diff_bounded(&base, diff, &FileHistory::in_memory())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(retained.join("existing.txt")).unwrap(),
            "after\n"
        );
        assert_eq!(
            std::fs::read_to_string(retained.join("new/nested.txt")).unwrap(),
            "created\n"
        );
        assert_eq!(
            std::fs::read_to_string(outside.path().join("existing.txt")).unwrap(),
            "OUTSIDE CANARY"
        );
        assert!(!outside.path().join("new").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_restore_retains_history_and_cannot_follow_final_symlink() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let filename = root.path().join("file.txt");
        std::fs::write(&filename, "original\n").unwrap();
        let editor = access(root.path());
        let file = editor.bind(&filename, AccessMode::Write).unwrap();
        let history = FileHistory::in_memory();
        save_file_history(&file, &history).unwrap();
        let key = file.bound.try_history_key().unwrap();
        std::fs::write(outside.path().join("canary"), "OUTSIDE CANARY").unwrap();
        std::fs::remove_file(&filename).unwrap();
        std::os::unix::fs::symlink(outside.path().join("canary"), &filename).unwrap();
        assert!(text_editor_undo_bounded(&file, &history).await.is_err());
        assert_eq!(history.depth(&key), 1);
        assert_eq!(
            std::fs::read_to_string(outside.path().join("canary")).unwrap(),
            "OUTSIDE CANARY"
        );
        std::fs::remove_file(&filename).unwrap();
        std::fs::write(&filename, "replacement\n").unwrap();
        text_editor_undo_bounded(&file, &history).await.unwrap();
        assert_eq!(std::fs::read_to_string(&filename).unwrap(), "original\n");
        assert_eq!(history.depth(&key), 0);
    }

    #[tokio::test]
    async fn external_authority_is_opaque_action_scoped_and_revocable() {
        use biorouter_sandbox::bounded_fs::{DispatchAuthority, RootGrant};
        let root = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let requested = external.path().join("result.txt");
        let mut editor = access(root.path());
        editor
            .meta
            .0
            .insert("biorouter-session-id".into(), "synthetic-session".into());
        editor.meta.0.insert(
            "biorouter-file-access-action".into(),
            "synthetic-action".into(),
        );
        editor.meta.0.insert(
            "biorouter-file-access-token".into(),
            "invented-token".into(),
        );
        assert!(editor.bind(&requested, AccessMode::Write).is_err());
        let grant = RootGrant::new(
            Arc::new(BoundedFs::open(external.path()).unwrap()),
            AccessMode::Write,
        );
        let authority = DispatchAuthority::register(
            "synthetic-session",
            "synthetic-action",
            vec![grant.clone()],
        )
        .unwrap();
        editor.meta.0.insert(
            "biorouter-file-access-token".into(),
            authority.token().into(),
        );
        let file = editor.bind(&requested, AccessMode::Write).unwrap();
        text_editor_write_bounded(&file, "approved").await.unwrap();
        grant.revoke();
        assert!(text_editor_write_bounded(&file, "must fail").await.is_err());
        assert_eq!(
            std::fs::read_to_string(&requested).unwrap().trim(),
            "approved"
        );
    }

    #[tokio::test]
    async fn exact_read_grant_cannot_write_change_action_or_outlive_dispatch() {
        use biorouter_sandbox::bounded_fs::{DispatchAuthority, RootGrant};
        let root = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let requested = external.path().join("result.txt");
        std::fs::write(&requested, "approved read").unwrap();
        std::fs::write(external.path().join("other.txt"), "not granted").unwrap();
        let filesystem = Arc::new(BoundedFs::open(external.path()).unwrap());
        let grant = RootGrant::new(filesystem, AccessMode::Read)
            .with_targets(vec![requested.clone()])
            .unwrap();
        let authority = DispatchAuthority::register("session", "action", vec![grant]).unwrap();
        let mut editor = access(root.path());
        for (key, value) in [
            ("biorouter-session-id", "session"),
            ("biorouter-file-access-action", "action"),
            ("biorouter-file-access-token", authority.token()),
        ] {
            editor.meta.0.insert(key.into(), value.into());
        }
        let read = editor.bind(&requested, AccessMode::Read).unwrap();
        assert!(editor.bind(&requested, AccessMode::Write).is_err());
        assert!(editor
            .bind(&external.path().join("other.txt"), AccessMode::Read)
            .is_err());
        editor.meta.0.insert(
            "biorouter-file-access-action".into(),
            "another action".into(),
        );
        assert!(editor.bind(&requested, AccessMode::Read).is_err());
        drop(authority);
        assert!(read.read_to_string().is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn normal_internal_links_patch_crlf_fuzzy_and_partial_hunks_still_work() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("file.txt"),
            "alpha\r\nbeta trailing  \r\nend\r\n",
        )
        .unwrap();
        std::os::unix::fs::symlink("file.txt", root.path().join("link.txt")).unwrap();
        let editor = access(root.path());
        let file = editor
            .bind(&root.path().join("link.txt"), AccessMode::Write)
            .unwrap();
        let history = FileHistory::in_memory();
        text_editor_replace_bounded(&file, "alpha", "ALPHA", None, &None, &history)
            .await
            .unwrap();
        let base = editor.bind(root.path(), AccessMode::Read).unwrap();
        let diff = "--- a/file.txt\n+++ b/file.txt\n@@ -1,3 +1,3 @@\n ALPHA\n-beta trailing\n+BETA\n end\n@@ -20,1 +20,1 @@\n-no such context at all\n+missing replacement\n";
        apply_diff_bounded(&base, diff, &history).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("file.txt")).unwrap(),
            "ALPHA\r\nBETA\r\nend\r\n"
        );
        assert!(root
            .path()
            .join("link.txt")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
    }
}
