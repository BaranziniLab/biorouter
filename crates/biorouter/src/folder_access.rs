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

mod analysis;

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
        Self::with_skills_dir(&crate::agents::skills_extension::skills_root(
            &crate::config::paths::Paths::config_dir(),
        ))
    }
}

pub struct FileAdmission {
    pub authority: DispatchAuthority,
    pub action_id: String,
}

struct AdmissionContext<'a> {
    session: &'a str,
    action: &'a str,
    tool: &'a str,
    cwd: &'a Path,
    cancel: &'a CancellationToken,
    permissions: &'a crate::permission::ToolPermissionStore,
    scoped_enabled: bool,
}

type FileAliases = Vec<(PathBuf, PathBuf)>;

fn resolved_plan(
    plan: &biorouter_mcp::FileAccessPlan,
) -> std::io::Result<(biorouter_mcp::FileAccessPlan, FileAliases)> {
    let mut actual_plan = biorouter_mcp::FileAccessPlan::default();
    let mut aliases: Vec<(PathBuf, PathBuf)> = Vec::new();
    for request in &plan.requests {
        let actual = match aliases
            .iter()
            .find(|(original, _)| original == &request.path)
        {
            Some((_, actual)) => actual.clone(),
            None => discover_target(&request.path)?,
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
    Ok((actual_plan, aliases))
}

fn validate_plan(
    plan: &biorouter_mcp::FileAccessPlan,
    context: &AdmissionContext<'_>,
) -> anyhow::Result<()> {
    biorouter_mcp::preflight_file_access(plan, context.cwd)?;
    anyhow::ensure!(
        !context.permissions.denies_file_access(
            context.tool,
            &plan
                .requests
                .iter()
                .map(|request| request.path.clone())
                .collect::<Vec<_>>(),
            context.cwd,
            context.scoped_enabled,
        ),
        "This file operation was denied for a requested directory"
    );
    anyhow::ensure!(
        !context.permissions.denies_recursive_file_access(
            context.tool,
            &plan
                .requests
                .iter()
                .filter(|request| request.recursive && request.access == AccessMode::Read)
                .map(|request| request.path.clone())
                .collect::<Vec<_>>(),
            context.cwd,
            context.scoped_enabled,
        ),
        "Recursive analysis overlaps a remembered directory denial. Choose a narrower allowed folder, or change the remembered denial in Settings before retrying."
    );
    Ok(())
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
        #[cfg(windows)]
        validate_windows_discovery_path(&remaining)?;
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
                    if !components.as_path().as_os_str().is_empty() {
                        next.push(components.as_path());
                    }
                    remaining = next;
                    followed = true;
                    break;
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    resolved.extend(components);
                    return canonical_discovered_path(&resolved);
                }
                Err(error) => return Err(error),
            }
        }
        if !followed {
            return canonical_discovered_path(&resolved);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "Too many symbolic links in the requested file path",
    ))
}

#[cfg(windows)]
fn validate_windows_discovery_path(path: &Path) -> std::io::Result<()> {
    for component in path.components() {
        if let Component::Normal(name) = component {
            let name = name.to_string_lossy();
            if name.contains(':') || name.ends_with(['.', ' ']) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "Alternate streams and normalized path aliases are not authorized file paths",
                ));
            }
        }
    }
    Ok(())
}

fn canonical_discovered_path(path: &Path) -> std::io::Result<PathBuf> {
    let mut existing = path;
    loop {
        match std::fs::symlink_metadata(existing) {
            Ok(_) => {
                let mut canonical = existing.canonicalize()?;
                let missing = path.strip_prefix(existing).unwrap();
                if !missing.as_os_str().is_empty() {
                    canonical.push(missing);
                }
                return Ok(canonical);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                existing = existing.parent().ok_or(error)?;
            }
            Err(error) => return Err(error),
        }
    }
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

fn folder_approval_request(
    action_id: &str,
    tool_name: &str,
    folder: &Path,
    access: AccessMode,
    requests: &[biorouter_mcp::FileAccessRequest],
    aliases: &[(PathBuf, PathBuf)],
) -> FolderAccessRequest {
    FolderAccessRequest {
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
    }
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

    fn folder_declined_this_turn(&self, session_id: &str, folder: &Path) -> bool {
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session_id)
            .is_some_and(|session| {
                session
                    .denied
                    .iter()
                    .any(|denied| folder.starts_with(denied) || denied.starts_with(folder))
            })
    }

    fn folder_request_lock(
        &self,
        session_id: &str,
        folder: &Path,
        access: AccessMode,
    ) -> Arc<tokio::sync::Mutex<()>> {
        let key = format!(
            "{}:{}:{}",
            session_id,
            folder.display(),
            access_name(access)
        );
        let mut requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        requests.retain(|_, lock| lock.strong_count() > 0);
        match requests.get(&key).and_then(Weak::upgrade) {
            Some(lock) => lock,
            None => {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                requests.insert(key, Arc::downgrade(&lock));
                lock
            }
        }
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
        let lock = self.folder_request_lock(session_id, folder, access);
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
        if self.folder_declined_this_turn(session_id, folder) {
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
        anyhow::ensure!(
            filesystem.root() == folder,
            authorization_required(folder, access, "The folder changed while access was being prepared. Retry the original file operation so its current target can be shown for approval.")
        );
        let once_grants = exact_grants(&filesystem, requests)?;
        let request =
            folder_approval_request(action_id, tool_name, folder, access, requests, aliases);
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
        self.admit_with_analysis(session_id, tool_name, cwd, plan, cancel, None)
            .await
    }

    pub async fn admit_analysis(
        &self,
        session_id: &str,
        tool_name: &str,
        cwd: &Path,
        plan: &biorouter_mcp::FileAccessPlan,
        cancel: &CancellationToken,
        max_depth: u32,
    ) -> anyhow::Result<FileAdmission> {
        self.admit_with_analysis(session_id, tool_name, cwd, plan, cancel, Some(max_depth))
            .await
    }

    async fn admit_with_analysis(
        &self,
        session_id: &str,
        tool_name: &str,
        cwd: &Path,
        plan: &biorouter_mcp::FileAccessPlan,
        cancel: &CancellationToken,
        analysis_depth: Option<u32>,
    ) -> anyhow::Result<FileAdmission> {
        let action_id = Uuid::new_v4().to_string();
        let permissions = crate::permission::ToolPermissionStore::load()?;
        let context = AdmissionContext {
            session: session_id,
            action: &action_id,
            tool: tool_name,
            cwd,
            cancel,
            permissions: &permissions,
            scoped_enabled: crate::config::Config::global()
                .get_param::<bool>("PERMISSION_SCOPED_GRANTS")
                .unwrap_or(true),
        };
        let (mut actual_plan, mut aliases) = resolved_plan(plan)?;
        validate_plan(plan, &context)?;
        validate_plan(&actual_plan, &context)?;
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
        if let Some(depth) = analysis_depth {
            self.admit_analysis_targets(
                &context,
                &mut actual_plan,
                &mut roots,
                &mut aliases,
                depth,
            )
            .await?;
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

    async fn admit_analysis_targets(
        &self,
        context: &AdmissionContext<'_>,
        plan: &mut biorouter_mcp::FileAccessPlan,
        roots: &mut Vec<RootGrant>,
        aliases: &mut Vec<(PathBuf, PathBuf)>,
        max_depth: u32,
    ) -> anyhow::Result<()> {
        let Some(root) = plan.requests.first().filter(|request| request.recursive) else {
            return Ok(());
        };
        let authority = DispatchAuthority::register_aliases(
            context.session,
            context.action,
            roots.clone(),
            aliases.clone(),
        )?;
        let bound = biorouter_sandbox::bounded_fs::authorized_path(
            authority.token(),
            context.session,
            context.action,
            &root.path,
            AccessMode::Read,
        )?;
        let mut discovery = analysis::AnalysisDiscovery::new(&bound, max_depth)?;
        drop(authority);
        let mut external_folders = HashSet::new();
        loop {
            let authority = DispatchAuthority::register_aliases(
                context.session,
                context.action,
                roots.clone(),
                aliases.clone(),
            )?;
            let target = discovery.next_external(
                &authority,
                context.session,
                context.action,
                context.cancel,
            )?;
            let Some(target) = target else { return Ok(()) };
            let request = biorouter_mcp::FileAccessRequest {
                path: target.actual.clone(),
                access: AccessMode::Read,
                recursive: target.recursive,
            };
            validate_plan(
                &biorouter_mcp::FileAccessPlan {
                    requests: vec![
                        request.clone(),
                        biorouter_mcp::FileAccessRequest {
                            path: target.requested.clone(),
                            ..request.clone()
                        },
                    ],
                },
                context,
            )?;
            let folder = existing_containing_folder(&target.actual)?;
            external_folders.insert(folder.clone());
            anyhow::ensure!(
                external_folders.len() <= 128,
                "Analysis reaches too many external folders; choose a narrower folder or depth"
            );
            aliases.push((target.requested, target.actual));
            let covered = biorouter_sandbox::bounded_fs::authorized_path(
                authority.token(),
                context.session,
                context.action,
                &request.path,
                AccessMode::Read,
            )
            .is_ok_and(|bound| !request.recursive || bound.open_dir().is_ok());
            if covered {
                plan.requests.push(request);
                continue;
            }
            roots.extend(
                self.approve_folder(
                    context.session,
                    context.action,
                    context.tool,
                    context.cwd,
                    &folder,
                    std::slice::from_ref(&request),
                    aliases,
                    context.cancel,
                )
                .await?,
            );
            plan.requests.push(request);
        }
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

    #[test]
    fn discovery_canonicalizes_existing_ancestors_and_retains_missing_suffixes() {
        let fixture = tempfile::tempdir().unwrap();
        let requested = fixture.path().join("missing/subfolder/result.txt");
        assert_eq!(
            discover_target(&requested).unwrap(),
            fixture
                .path()
                .canonicalize()
                .unwrap()
                .join("missing/subfolder/result.txt")
        );
    }

    #[cfg(unix)]
    #[test]
    fn discovery_keeps_existing_files_and_final_file_links_as_file_paths() {
        let fixture = tempfile::tempdir().unwrap();
        let file = fixture.path().join("result.csv");
        let link = fixture.path().join("result-link.csv");
        std::fs::write(&file, "synthetic result").unwrap();
        std::os::unix::fs::symlink(&file, &link).unwrap();
        for requested in [&file, &link] {
            let actual = discover_target(requested).unwrap();
            assert_eq!(actual.as_os_str(), file.canonicalize().unwrap().as_os_str());
            assert_eq!(std::fs::read_to_string(actual).unwrap(), "synthetic result");
        }
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn macos_tmp_alias_is_admitted_in_the_captured_root_namespace() {
        let fixture = tempfile::Builder::new()
            .prefix("biorouter-folder-authority-")
            .tempdir_in("/tmp")
            .unwrap();
        let requested = fixture.path().join("result.txt");
        std::fs::write(&requested, "synthetic tmp alias").unwrap();
        assert_eq!(
            discover_target(&requested).unwrap(),
            requested.canonicalize().unwrap()
        );
        let manager = FolderAccessManager::with_skills_dir(&fixture.path().join("unused-skills"));
        let (admission, session) = read(&manager, fixture.path(), &requested).await.unwrap();
        assert_eq!(
            authorized_path(
                admission.authority.token(),
                &session,
                &admission.action_id,
                &requested,
                AccessMode::Read
            )
            .unwrap()
            .read_to_string()
            .unwrap(),
            "synthetic tmp alias"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_plain_and_verbatim_paths_share_the_captured_root_namespace() {
        let fixture = tempfile::tempdir().unwrap();
        let canonical = fixture.path().canonicalize().unwrap();
        let canonical_label = canonical.to_string_lossy();
        let plain = PathBuf::from(
            canonical_label
                .strip_prefix(r"\\?\")
                .unwrap_or(&canonical_label),
        );
        let requested = plain.join("result.txt");
        std::fs::write(&requested, "synthetic windows alias").unwrap();
        assert_eq!(
            discover_target(&requested).unwrap(),
            requested.canonicalize().unwrap()
        );
        assert_eq!(
            discover_target(&plain.join("missing/result.txt")).unwrap(),
            canonical.join("missing/result.txt")
        );
        let manager = FolderAccessManager::with_skills_dir(&plain.join("unused-skills"));
        let (admission, session) = read(&manager, &plain, &requested).await.unwrap();
        assert_eq!(
            authorized_path(
                admission.authority.token(),
                &session,
                &admission.action_id,
                &requested,
                AccessMode::Read
            )
            .unwrap()
            .read_to_string()
            .unwrap(),
            "synthetic windows alias"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_alias_refusals_precede_metadata_normalization() {
        let fixture = tempfile::tempdir().unwrap();
        std::fs::write(fixture.path().join("result.txt"), "synthetic existing file").unwrap();
        for name in [
            "result.txt.",
            "result.txt ",
            "result.txt:hidden",
            "missing-dir./result.txt",
        ] {
            assert_eq!(
                discover_target(&fixture.path().join(name))
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::PermissionDenied,
                "{name} must not be normalized into an authorized path"
            );
        }
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

    #[cfg(unix)]
    #[tokio::test]
    async fn a_folder_retargeted_after_discovery_cannot_supply_a_mislabelled_session_grant() {
        let fixture = tempfile::tempdir().unwrap();
        let directory = fixture.path().canonicalize().unwrap();
        let workspace = directory.join("workspace");
        let disclosed = directory.join("disclosed-results");
        let outside = directory.join("outside");
        for path in [&workspace, &disclosed, &outside] {
            std::fs::create_dir(path).unwrap();
        }
        std::fs::write(outside.join("result.txt"), "synthetic outside result").unwrap();
        let session = Uuid::new_v4().to_string();
        let manager = FolderAccessManager::default();
        manager.pin_workspace(&session, &workspace).unwrap();
        std::fs::rename(&disclosed, directory.join("original-results")).unwrap();
        std::os::unix::fs::symlink(&outside, &disclosed).unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            manager.approve_folder(
                &session,
                &Uuid::new_v4().to_string(),
                "developer__text_editor",
                &workspace,
                &disclosed,
                &[biorouter_mcp::FileAccessRequest {
                    path: disclosed.join("result.txt"),
                    access: AccessMode::Read,
                    recursive: false,
                }],
                &[],
                &CancellationToken::new(),
            ),
        )
        .await
        .expect("a retargeted folder must be refused before an approval card is parked");
        assert_authorization_required(result.err().unwrap());
        assert!(manager.list_session(&session).is_empty());
        assert!(!PendingUserActions::global().has_pending_in_session(&session));
    }
}
