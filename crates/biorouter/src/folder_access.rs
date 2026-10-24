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

#[derive(Default)]
pub struct FolderAccessManager {
    sessions: Mutex<HashMap<String, SessionAuthority>>,
    requests: Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
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

fn authorization_required(folder: &Path, access: AccessMode, reason: &str) -> ErrorData {
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
    requests: &[(PathBuf, AccessMode)],
) -> std::io::Result<Vec<RootGrant>> {
    [AccessMode::Read, AccessMode::Write]
        .into_iter()
        .filter_map(|mode| {
            let targets: Vec<_> = requests
                .iter()
                .filter(|(_, access)| *access == mode)
                .map(|(path, _)| path.clone())
                .collect();
            (!targets.is_empty())
                .then(|| RootGrant::new(filesystem.clone(), mode).with_targets(targets))
        })
        .collect()
}

impl FolderAccessManager {
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

    fn roots(&self, session_id: &str, cwd: &Path) -> std::io::Result<Vec<RootGrant>> {
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        let session = sessions.entry(session_id.to_string()).or_default();
        if session.primary.as_ref().is_none_or(|(path, _)| path != cwd) {
            let root = RootGrant::new(Arc::new(BoundedFs::open(cwd)?), AccessMode::Write);
            if let Some((_, old)) = session.primary.replace((cwd.to_path_buf(), root)) {
                old.revoke();
            }
        }
        let mut roots = vec![session
            .primary
            .as_ref()
            .expect("primary bound above")
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
        requests: &[(PathBuf, AccessMode)],
        aliases: &[(PathBuf, PathBuf)],
        cancel: &CancellationToken,
    ) -> anyhow::Result<Vec<RootGrant>> {
        let access = if requests.iter().any(|(_, mode)| *mode == AccessMode::Write) {
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
            requests
                .iter()
                .all(|(path, mode)| Self::covered(std::slice::from_ref(root), path, *mode))
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
            files: requests
                .iter()
                .map(|(path, _)| {
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
        let mut aliases = Vec::new();
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
            });
        }
        biorouter_mcp::preflight_file_access(&actual_plan, cwd)?;
        let mut roots = self.roots(session_id, cwd)?;
        let skills = crate::config::paths::Paths::config_dir().join("skills");
        if let Ok(filesystem) = BoundedFs::open(skills) {
            roots.push(RootGrant::new(Arc::new(filesystem), AccessMode::Read));
        }
        let mut missing: BTreeMap<PathBuf, Vec<(PathBuf, AccessMode)>> = BTreeMap::new();
        for request in &actual_plan.requests {
            if !Self::covered(&roots, &request.path, request.access) {
                let folder = existing_containing_folder(&request.path)?;
                missing
                    .entry(folder)
                    .or_default()
                    .push((request.path.clone(), request.access));
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
