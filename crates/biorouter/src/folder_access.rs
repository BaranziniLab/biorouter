use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError, Weak};
use std::time::Duration;

use biorouter_sandbox::{AccessMode, BoundedFs, DispatchAuthority, RootGrant};
use rmcp::model::{ErrorCode, ErrorData};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::pending_user_action::{
    FolderAccessRequest, PendingUserActions, UserActionOutcome, UserActionRequest,
};
use crate::permission::Permission;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FolderGrantView {
    pub id: String,
    pub folder: String,
    pub access: String,
    pub descendants: bool,
}

struct SessionGrant {
    view: FolderGrantView,
    root: RootGrant,
}

#[derive(Default)]
struct SessionAuthority {
    primary: Option<(PathBuf, RootGrant)>,
    grants: Vec<SessionGrant>,
    denied: HashSet<PathBuf>,
}

pub struct FolderAccessManager {
    sessions: Mutex<HashMap<String, SessionAuthority>>,
    requests: Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
    skill_anchor: Option<(Arc<BoundedFs>, PathBuf)>,
    skill_filesystem: Mutex<Option<Arc<BoundedFs>>>,
}

impl Default for FolderAccessManager {
    fn default() -> Self {
        Self::with_skills_dir(&crate::config::paths::Paths::config_dir().join("skills"))
    }
}

pub struct FileAdmission {
    pub authority: DispatchAuthority,
    pub action_id: String,
}

fn access_name(access: AccessMode) -> &'static str {
    match access {
        AccessMode::Read => "read-only",
        AccessMode::Write => "read/write",
    }
}

fn access_covers(granted: AccessMode, requested: AccessMode) -> bool {
    granted == AccessMode::Write || requested == AccessMode::Read
}

pub(crate) fn authorization_required(folder: &Path, access: AccessMode, reason: &str) -> ErrorData {
    ErrorData::new(
        ErrorCode::INVALID_REQUEST,
        format!(
            "Folder authorization required for {}. {reason}",
            folder.display()
        ),
        Some(serde_json::json!({
            "code": "authorization_required",
            "folder": folder,
            "access": access_name(access),
            "recovery": "Open this chat in an interactive desktop or terminal and approve the specific folder. Do not bypass this decision with shell or computer tools.",
        })),
    )
}

fn existing_containing_folder(path: &Path) -> std::io::Result<PathBuf> {
    let mut folder = if path.is_dir() {
        Some(path)
    } else {
        path.parent()
    }
    .ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "file has no containing folder",
        )
    })?;
    while !folder.is_dir() {
        folder = folder.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "containing folder is unavailable",
            )
        })?;
    }
    let folder = folder.canonicalize()?;
    if folder.parent().is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Cannot authorize the filesystem root; choose or create a specific project folder",
        ));
    }
    Ok(folder)
}

fn discover_target(path: &Path) -> std::io::Result<PathBuf> {
    let mut remaining = path.to_path_buf();
    for _ in 0..40 {
        let mut resolved = PathBuf::new();
        let mut components = remaining.components();
        let mut followed = false;
        while let Some(component) = components.next() {
            match component {
                Component::CurDir => continue,
                Component::ParentDir => {
                    resolved.pop();
                    continue;
                }
                component => resolved.push(component.as_os_str()),
            }
            match std::fs::symlink_metadata(&resolved) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    let target = std::fs::read_link(&resolved)?;
                    let parent = resolved.parent().unwrap_or(Path::new("/"));
                    let mut next = if target.is_absolute() {
                        target
                    } else {
                        parent.join(target)
                    };
                    next.push(components.as_path());
                    remaining = next;
                    followed = true;
                    break;
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    resolved.extend(components);
                    return Ok(resolved);
                }
                Err(error) => return Err(error),
            }
        }
        if !followed {
            return Ok(resolved);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "Too many symbolic links in the requested file path",
    ))
}

fn exact_grants(
    filesystem: &Arc<BoundedFs>,
    requests: &[biorouter_mcp::FileAccessRequest],
) -> std::io::Result<Vec<RootGrant>> {
    [AccessMode::Read, AccessMode::Write]
        .into_iter()
        .filter_map(|mode| {
            if mode == AccessMode::Read
                && requests.iter().any(|request| {
                    request.recursive && request.access == mode && request.path == filesystem.root()
                })
            {
                return Some(Ok(RootGrant::new(filesystem.clone(), mode)));
            }
            let targets: Vec<_> = requests
                .iter()
                .filter(|request| request.access == mode)
                .map(|request| request.path.clone())
                .collect();
            (!targets.is_empty())
                .then(|| RootGrant::new(filesystem.clone(), mode).with_targets(targets))
        })
        .collect()
}

impl FolderAccessManager {
    fn with_skills_dir(configured: &Path) -> Self {
        let skill_anchor = (|| {
            let configured = if configured.is_absolute() {
                configured.to_path_buf()
            } else {
                std::env::current_dir().ok()?.join(configured)
            };
            let existing = configured
                .parent()?
                .ancestors()
                .find(|path| path.is_dir())?;
            let filesystem = Arc::new(BoundedFs::open(existing).ok()?);
            let relative = configured.strip_prefix(existing).ok()?.to_path_buf();
            Some((filesystem, relative))
        })();
        let manager = Self {
            sessions: Mutex::default(),
            requests: Mutex::default(),
            skill_anchor,
            skill_filesystem: Mutex::default(),
        };
        let _ = manager.skill_read_root();
        manager
    }

    fn skill_read_root(&self) -> Option<Arc<BoundedFs>> {
        let mut pinned = self
            .skill_filesystem
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if pinned.is_none() {
            let (anchor, relative) = self.skill_anchor.as_ref()?;
            let filesystem = anchor
                .bind(relative, AccessMode::Read)
                .ok()?
                .open_dir()
                .ok()?;
            // A skills alias must not silently authorize a different config directory.
            if filesystem.root() != anchor.root().join(relative) {
                return None;
            }
            *pinned = Some(Arc::new(filesystem));
        }
        pinned.clone()
    }

    pub fn global() -> &'static Self {
        static MANAGER: LazyLock<FolderAccessManager> = LazyLock::new(FolderAccessManager::default);
        &MANAGER
    }

    pub fn begin_turn(&self, session_id: &str) {
        if let Some(session) = self
            .sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(session_id)
        {
            session.denied.clear();
        }
    }

    pub fn list_session(&self, session_id: &str) -> Vec<FolderGrantView> {
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session_id)
            .map(|session| {
                session
                    .grants
                    .iter()
                    .map(|grant| grant.view.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn revoke(&self, session_id: &str, grant_id: &str) -> bool {
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(session) = sessions.get_mut(session_id) else {
            return false;
        };
        let Some(index) = session
            .grants
            .iter()
            .position(|grant| grant.view.id == grant_id)
        else {
            return false;
        };
        session.grants.remove(index).root.revoke();
        true
    }

    pub fn end_session(&self, session_id: &str) {
        if let Some(session) = self
            .sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(session_id)
        {
            if let Some((_, primary)) = session.primary {
                primary.revoke();
            }
            for grant in session.grants {
                grant.root.revoke();
            }
        }
    }

    pub fn pin_workspace(&self, session_id: &str, cwd: &Path) -> std::io::Result<()> {
        self.pin_workspace_handle(session_id, cwd, Arc::new(BoundedFs::open(cwd)?));
        Ok(())
    }

    pub(crate) fn pin_workspace_handle(
        &self,
        session_id: &str,
        cwd: &Path,
        filesystem: Arc<BoundedFs>,
    ) {
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        let session = sessions.entry(session_id.to_string()).or_default();
        if session.primary.as_ref().is_none_or(|(path, _)| path != cwd) {
            let root = RootGrant::new(filesystem, AccessMode::Write);
            if let Some((_, old)) = session.primary.replace((cwd.to_path_buf(), root)) {
                old.revoke();
            }
        }
    }

    fn roots(&self, session_id: &str, cwd: &Path) -> anyhow::Result<Vec<RootGrant>> {
        let sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        let session = sessions.get(session_id).ok_or_else(|| {
            authorization_required(
                cwd,
                AccessMode::Write,
                "The workspace has not been bound by this chat's trusted startup. Reopen the chat or select the intended folder before retrying.",
            )
        })?;
        anyhow::ensure!(
            session.primary.as_ref().is_some_and(|(path, _)| path == cwd),
            authorization_required(cwd, AccessMode::Write, "The workspace changed without a trusted folder binding. Select the intended folder before retrying.")
        );
        let mut roots = vec![session
            .primary
            .as_ref()
            .expect("primary validated above")
            .1
            .clone()];
        roots.extend(session.grants.iter().map(|grant| grant.root.clone()));
        Ok(roots)
    }

    fn covered(roots: &[RootGrant], path: &Path, access: AccessMode) -> bool {
        roots.iter().any(|root| {
            access_covers(root.access(), access) && path.starts_with(root.filesystem().root())
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn approve_folder(
        &self,
        session_id: &str,
        action_id: &str,
        tool_name: &str,
        cwd: &Path,
        folder: &Path,
        requests: &[biorouter_mcp::FileAccessRequest],
        aliases: &[(PathBuf, PathBuf)],
        cancel: &CancellationToken,
    ) -> anyhow::Result<Vec<RootGrant>> {
        let access = if requests
            .iter()
            .any(|request| request.access == AccessMode::Write)
        {
            AccessMode::Write
        } else {
            AccessMode::Read
        };
        let key = format!(
            "{}:{}:{}",
            session_id,
            folder.display(),
            access_name(access)
        );
        let lock = {
            let mut requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
            requests.retain(|_, lock| lock.strong_count() > 0);
            match requests.get(&key).and_then(Weak::upgrade) {
                Some(lock) => lock,
                None => {
                    let lock = Arc::new(tokio::sync::Mutex::new(()));
                    requests.insert(key.clone(), Arc::downgrade(&lock));
                    lock
                }
            }
        };
        let _serial = tokio::select! {
            () = cancel.cancelled() => return Err(authorization_required(folder, access, "The operation was cancelled.").into()),
            guard = lock.lock() => guard,
        };
        let roots = self.roots(session_id, cwd)?;
        if let Some(root) = roots.into_iter().find(|root| {
            requests.iter().all(|request| {
                Self::covered(std::slice::from_ref(root), &request.path, request.access)
            })
        }) {
            return Ok(vec![root]);
        }
        let denied = self
            .sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session_id)
            .is_some_and(|session| {
                session
                    .denied
                    .iter()
                    .any(|denied| folder.starts_with(denied) || denied.starts_with(folder))
            });
        if denied {
            return Err(authorization_required(folder, access, "The user declined this folder during the current turn. Do not ask again or broaden the request.").into());
        }
        if crate::user_surface::no_human_surface() {
            return Err(authorization_required(
                folder,
                access,
                "No interactive approval surface is attached.",
            )
            .into());
        }
        let filesystem = Arc::new(BoundedFs::open(folder)?);
        let once_grants = exact_grants(&filesystem, requests)?;
        let request = FolderAccessRequest {
            action_id: action_id.to_string(),
            tool_name: tool_name.to_string(),
            folder: folder.display().to_string(),
            access: access_name(access).to_string(),
            once_descendants: requests.iter().any(|request| request.recursive),
            files: requests
                .iter()
                .map(|request| {
                    let path = &request.path;
                    let original = aliases
                        .iter()
                        .filter(|(_, actual)| actual == path)
                        .map(|(original, _)| original.display().to_string())
                        .collect::<Vec<_>>();
                    if original.is_empty() {
                        path.display().to_string()
                    } else {
                        format!("{} (requested as {})", path.display(), original.join(", "))
                    }
                })
                .collect(),
        };
        let pending = PendingUserActions::global().park(
            Some(session_id),
            None,
            UserActionRequest::FolderAccess(request),
        );
        match pending.wait(Duration::from_secs(300), Some(cancel)).await {
            UserActionOutcome::Approved {
                permission: Permission::AllowOnce,
            } => Ok(once_grants),
            UserActionOutcome::Approved {
                permission: Permission::AlwaysAllow,
            } => {
                let root = RootGrant::new(filesystem, access);
                let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
                let session = sessions.entry(session_id.to_string()).or_default();
                anyhow::ensure!(
                    session.grants.len() < 128,
                    "Revoke an unused folder before authorizing another"
                );
                session.grants.push(SessionGrant {
                    view: FolderGrantView {
                        id: Uuid::new_v4().to_string(),
                        folder: folder.display().to_string(),
                        access: access_name(access).to_string(),
                        descendants: true,
                    },
                    root: root.clone(),
                });
                Ok(vec![root])
            }
            outcome => {
                self.sessions
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .entry(session_id.to_string())
                    .or_default()
                    .denied
                    .insert(folder.to_path_buf());
                Err(authorization_required(folder, access, outcome.refusal_detail()).into())
            }
        }
    }

    pub async fn admit(
        &self,
        session_id: &str,
        tool_name: &str,
        cwd: &Path,
        plan: &biorouter_mcp::FileAccessPlan,
        cancel: &CancellationToken,
    ) -> anyhow::Result<FileAdmission> {
        let action_id = Uuid::new_v4().to_string();
        let mut aliases: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut actual_plan = biorouter_mcp::FileAccessPlan::default();
        for request in &plan.requests {
            let actual = if let Some((_, actual)) = aliases
                .iter()
                .find(|(original, _)| original == &request.path)
            {
                actual.clone()
            } else {
                discover_target(&request.path)?
            };
            if actual != request.path
                && !aliases
                    .iter()
                    .any(|(original, _)| original == &request.path)
            {
                aliases.push((request.path.clone(), actual.clone()));
            }
            actual_plan.requests.push(biorouter_mcp::FileAccessRequest {
                path: actual,
                access: request.access,
                recursive: request.recursive,
            });
        }
        biorouter_mcp::preflight_file_access(&actual_plan, cwd)?;
        anyhow::ensure!(
            !crate::permission::ToolPermissionStore::load()?.denies_file_access(
                tool_name,
                &actual_plan
                    .requests
                    .iter()
                    .map(|request| request.path.clone())
                    .collect::<Vec<_>>(),
                cwd,
                crate::config::Config::global()
                    .get_param::<bool>("PERMISSION_SCOPED_GRANTS")
                    .unwrap_or(true),
            ),
            "This file operation was denied for a requested directory"
        );
        let mut roots = self.roots(session_id, cwd)?;
        if let Some(filesystem) = self.skill_read_root() {
            roots.push(RootGrant::new(filesystem, AccessMode::Read));
        }
        let mut missing: BTreeMap<PathBuf, Vec<biorouter_mcp::FileAccessRequest>> = BTreeMap::new();
        for request in &actual_plan.requests {
            if !Self::covered(&roots, &request.path, request.access) {
                let folder = existing_containing_folder(&request.path)?;
                missing.entry(folder).or_default().push(request.clone());
            }
        }
        for (folder, requests) in missing {
            roots.extend(
                self.approve_folder(
                    session_id, &action_id, tool_name, cwd, &folder, &requests, &aliases, cancel,
                )
                .await?,
            );
        }
        anyhow::ensure!(
            !cancel.is_cancelled(),
            "Folder operation was cancelled before dispatch"
        );
        for request in &actual_plan.requests {
            let root = roots
                .iter()
                .find(|root| {
                    Self::covered(std::slice::from_ref(root), &request.path, request.access)
                })
                .ok_or_else(|| {
                    authorization_required(
                        &request.path,
                        request.access,
                        "Authorization was revoked.",
                    )
                })?;
            root.filesystem().bind(&request.path, request.access)?;
        }
        let authority =
            DispatchAuthority::register_aliases(session_id, &action_id, roots, aliases)?;
        Ok(FileAdmission {
            authority,
            action_id,
        })
    }
}

#[cfg(test)]
mod skill_authority_tests {
    use super::*;
    use biorouter_sandbox::bounded_fs::authorized_path;

    async fn read(
        manager: &FolderAccessManager,
        workspace: &Path,
        requested: &Path,
    ) -> anyhow::Result<(FileAdmission, String)> {
        let session = Uuid::new_v4().to_string();
        manager.pin_workspace(&session, workspace)?;
        let admission = crate::user_surface::without_human_surface(manager.admit(
            &session,
            "developer__text_editor",
            workspace,
            &biorouter_mcp::FileAccessPlan {
                requests: vec![biorouter_mcp::FileAccessRequest {
                    path: requested.to_path_buf(),
                    access: AccessMode::Read,
                    recursive: false,
                }],
            },
            &CancellationToken::new(),
        ))
        .await?;
        Ok((admission, session))
    }

    fn assert_authorization_required(error: anyhow::Error) {
        let data = error.downcast_ref::<ErrorData>().unwrap();
        assert_eq!(
            data.data.as_ref().unwrap()["code"],
            "authorization_required"
        );
    }

    #[tokio::test]
    async fn installed_skills_remain_implicitly_readable_but_not_writable() {
        let config = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let skills = config.path().join("skills");
        std::fs::create_dir(&skills).unwrap();
        let script = skills.join("SKILL.md");
        std::fs::write(&script, "synthetic installed skill").unwrap();
        let manager = FolderAccessManager::with_skills_dir(&skills);
        let (admission, session) = read(&manager, workspace.path(), &script).await.unwrap();
        assert_eq!(
            authorized_path(
                admission.authority.token(),
                &session,
                &admission.action_id,
                &script,
                AccessMode::Read,
            )
            .unwrap()
            .read_to_string()
            .unwrap(),
            "synthetic installed skill"
        );
        assert!(authorized_path(
            admission.authority.token(),
            &session,
            &admission.action_id,
            &script,
            AccessMode::Write,
        )
        .is_err());
    }

    #[tokio::test]
    async fn skills_installed_after_manager_start_use_the_pinned_ancestor() {
        let config = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let skills = config.path().join("new-config/skills");
        let manager = FolderAccessManager::with_skills_dir(&skills);
        std::fs::create_dir_all(&skills).unwrap();
        let script = skills.join("SKILL.md");
        std::fs::write(&script, "synthetic late install").unwrap();
        assert!(read(&manager, workspace.path(), &script).await.is_ok());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_late_skills_symlink_requires_explicit_external_authorization() {
        let config = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let skills = config.path().join("skills");
        let manager = FolderAccessManager::with_skills_dir(&skills);
        std::fs::write(outside.path().join("SKILL.md"), "synthetic outside file").unwrap();
        std::os::unix::fs::symlink(outside.path(), &skills).unwrap();
        assert_authorization_required(
            read(&manager, workspace.path(), &skills.join("SKILL.md"))
                .await
                .err()
                .expect("a new skills link must not supply implicit outside authority"),
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_preexisting_skills_alias_does_not_supply_implicit_authority() {
        let config = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let other = config.path().join("other-config-content");
        std::fs::create_dir(&other).unwrap();
        std::fs::write(other.join("SKILL.md"), "synthetic unrelated config file").unwrap();
        let skills = config.path().join("skills");
        std::os::unix::fs::symlink(&other, &skills).unwrap();
        let manager = FolderAccessManager::with_skills_dir(&skills);
        assert_authorization_required(
            read(&manager, workspace.path(), &skills.join("SKILL.md"))
                .await
                .err()
                .expect("an existing alias must not silently authorize unrelated config content"),
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn replacing_a_pinned_skills_directory_cannot_widen_implicit_authority() {
        let config = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let skills = config.path().join("skills");
        std::fs::create_dir(&skills).unwrap();
        std::fs::write(skills.join("SKILL.md"), "synthetic installed skill").unwrap();
        let manager = FolderAccessManager::with_skills_dir(&skills);
        std::fs::rename(&skills, config.path().join("original-skills")).unwrap();
        std::fs::write(outside.path().join("SKILL.md"), "synthetic outside file").unwrap();
        std::os::unix::fs::symlink(outside.path(), &skills).unwrap();
        assert_authorization_required(
            read(&manager, workspace.path(), &skills.join("SKILL.md"))
                .await
                .err()
                .expect("replacing the skills name must not replace its pinned authority"),
        );
    }
}
