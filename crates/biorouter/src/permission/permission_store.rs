use crate::config::paths::Paths;
use crate::conversation::message::ToolRequest;
use crate::permission::permission_scope::{CallFacts, PermissionScope};
use anyhow::Result;
use blake3::Hasher;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use std::{fs::File, path::PathBuf};

/// Schema version written by this build. `1` had only the exact-args
/// `permissions` map; `2` adds BR-24's `scoped` grants. A v1 file loads
/// unchanged (`scoped` defaults to empty), and a v2 file read by a v1 build
/// simply ignores the field — so the bump is informational, not a migration.
const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolPermissionRecord {
    tool_name: String,
    allowed: bool,
    context_hash: String, // Hash of the tool's arguments/context to differentiate similar calls
    #[serde(skip_serializing_if = "Option::is_none")] // Don't serialize if None
    readable_context: Option<String>,
    timestamp: i64,
    expiry: Option<i64>, // Optional expiry timestamp
}

/// BR-24: a remembered approval scoped to a **directory** or a **command
/// prefix**, rather than to one exact set of arguments (the `permissions` map
/// above) or to a whole tool name for all time
/// ([`crate::config::permission::PermissionLevel::AlwaysAllow`]).
///
/// The grant is keyed on the fully-qualified, namespaced tool name: a grant
/// recorded for `developer__shell` says nothing about some other extension's
/// `shell`, even though the two share a base name.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ScopedPermissionGrant {
    pub tool_name: String,
    pub scope: PermissionScope,
    pub allowed: bool,
    pub timestamp: i64,
    /// Optional expiry timestamp (seconds since epoch).
    pub expiry: Option<i64>,
}

impl ScopedPermissionGrant {
    fn is_live(&self, now: i64) -> bool {
        self.expiry.is_none_or(|expiry| expiry > now)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolPermissionStore {
    permissions: HashMap<String, Vec<ToolPermissionRecord>>,
    /// BR-24 scoped grants. `default` so a v1 store on disk still loads.
    #[serde(default)]
    scoped: Vec<ScopedPermissionGrant>,
    version: u32, // For future schema migrations
    #[serde(skip)] // Don't serialize this field
    permissions_dir: PathBuf,
}

impl Default for ToolPermissionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolPermissionStore {
    pub fn new() -> Self {
        Self {
            permissions: HashMap::new(),
            scoped: Vec::new(),
            version: SCHEMA_VERSION,
            permissions_dir: Paths::config_dir().join("permissions"),
        }
    }

    /// An in-memory store rooted at `permissions_dir`. Used by tests and by
    /// embedders that keep permissions outside the user's config dir.
    pub fn new_in(permissions_dir: PathBuf) -> Self {
        Self {
            permissions_dir,
            ..Self::new()
        }
    }

    pub fn load() -> Result<Self> {
        let store = Self::new();
        let file_path = store.permissions_dir.join("tool_permissions.json");

        if !file_path.exists() {
            return Ok(store);
        }

        let file = File::open(file_path)?;
        let mut permissions: ToolPermissionStore = serde_json::from_reader(file)?;
        permissions.permissions_dir = store.permissions_dir;

        // Clean up expired entries on load
        permissions.cleanup_expired()?;

        Ok(permissions)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.permissions_dir)?;

        let path = self.permissions_dir.join("tool_permissions.json");
        let temp_path = path.with_extension("tmp");

        // Write complete content to temporary file
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&temp_path, &content)?;

        // Atomically rename temp file to target file
        std::fs::rename(temp_path, path)?;

        Ok(())
    }

    pub fn check_permission(&self, tool_request: &ToolRequest) -> Option<bool> {
        // A malformed request carries no recoverable permission key; degrade to
        // "no stored decision" (fail-closed: the caller will prompt) instead of
        // panicking and crashing the loop.
        let tool_call = match tool_request.tool_call.as_ref() {
            Ok(tool_call) => tool_call,
            Err(e) => {
                tracing::warn!("check_permission on malformed tool request: {e}");
                return None;
            }
        };
        let context_hash = hash_tool_context(tool_request);
        let key = format!("{}:{}", tool_call.name, context_hash);

        self.permissions.get(&key).and_then(|records| {
            records
                .iter()
                .rfind(|record| record.expiry.is_none_or(|exp| exp > Utc::now().timestamp()))
                .map(|record| record.allowed)
        })
    }

    pub fn record_permission(
        &mut self,
        tool_request: &ToolRequest,
        allowed: bool,
        expiry_duration: Option<Duration>,
    ) -> anyhow::Result<()> {
        // Refuse to persist a permission for a request we cannot read, rather
        // than panicking on the Err tool_call.
        let tool_call = match tool_request.tool_call.as_ref() {
            Ok(tool_call) => tool_call,
            Err(e) => {
                tracing::warn!("refusing to record permission for malformed tool request: {e}");
                anyhow::bail!("cannot record permission for a malformed tool request: {e}");
            }
        };
        let context_hash = hash_tool_context(tool_request);
        let key = format!("{}:{}", tool_call.name, context_hash);

        let record = ToolPermissionRecord {
            tool_name: tool_call.name.to_string().clone(),
            allowed,
            context_hash,
            readable_context: Some(tool_request.to_readable_string()),
            timestamp: Utc::now().timestamp(),
            expiry: expiry_duration.map(|d| Utc::now().timestamp() + d.as_secs() as i64),
        };

        self.permissions.entry(key).or_default().push(record);

        self.save()?;
        Ok(())
    }

    /// BR-24: remember a **scoped** approval — "always allow this tool in this
    /// directory", "always allow this tool for this command prefix".
    ///
    /// The scope is re-validated here even though the [`PermissionScope`]
    /// constructors already enforce it: this is the one function that turns a
    /// scope into something persistent and silently reusable, so it refuses the
    /// blanket grants (`/`, an empty command prefix) unconditionally.
    ///
    /// Re-granting the same `(tool, scope)` pair replaces the previous grant
    /// rather than stacking a second one, so a later deny genuinely overrides an
    /// earlier allow instead of leaving both in the file.
    pub fn record_scoped_grant(
        &mut self,
        tool_name: &str,
        scope: PermissionScope,
        allowed: bool,
        expiry_duration: Option<Duration>,
    ) -> Result<()> {
        if tool_name.is_empty() {
            anyhow::bail!("cannot record a scoped permission for an unnamed tool");
        }
        scope.validate()?;

        let now = Utc::now().timestamp();
        self.scoped
            .retain(|grant| !(grant.tool_name == tool_name && grant.scope == scope));
        self.scoped.push(ScopedPermissionGrant {
            tool_name: tool_name.to_string(),
            scope,
            allowed,
            timestamp: now,
            expiry: expiry_duration.map(|d| now + d.as_secs() as i64),
        });

        self.save()?;
        Ok(())
    }

    /// The remembered scoped decision for a pending call, or `None` when no live
    /// grant matches it (the caller then prompts, exactly as before).
    ///
    /// **A matching deny always wins**, whatever the order in the file. The
    /// proposal called for last-wins precedence; deny-wins is the strictly safer
    /// reading of the same rule set, and the only one where a bug in the ordering
    /// cannot manufacture an approval. `record_scoped_grant` replaces same-scope
    /// grants in place, so the two rules only diverge when *different* scopes
    /// disagree about one call — in which case the user has said "never here" and
    /// "always this", and we honour the "never".
    pub fn matching_scoped_grant(
        &self,
        tool_name: &str,
        facts: &CallFacts,
    ) -> Option<ScopedPermissionGrant> {
        let now = Utc::now().timestamp();
        let mut allow: Option<&ScopedPermissionGrant> = None;

        for grant in &self.scoped {
            if grant.tool_name != tool_name || !grant.is_live(now) || !grant.scope.matches(facts) {
                continue;
            }
            if !grant.allowed {
                return Some(grant.clone());
            }
            allow = Some(grant);
        }
        allow.cloned()
    }

    /// Apply configured directory denials to every target in a complete file plan.
    /// A patch touching several directories is denied when any child is denied;
    /// remembered allows never provide filesystem authority through this check.
    /// `scoped_enabled` must be the same `PERMISSION_SCOPED_GRANTS` setting used
    /// by the permission inspector. Callers also check their discovered alias targets.
    pub fn denies_file_access(
        &self,
        tool_name: &str,
        paths: &[PathBuf],
        working_dir: &std::path::Path,
        scoped_enabled: bool,
    ) -> bool {
        if !scoped_enabled {
            return false;
        }
        let facts: Vec<_> = paths
            .iter()
            .map(|path| CallFacts::for_file_path(path, working_dir))
            .collect();
        let now = Utc::now().timestamp();
        self.scoped.iter().any(|grant| {
            grant.tool_name == tool_name
                && !grant.allowed
                && grant.is_live(now)
                && matches!(&grant.scope, PermissionScope::Directory { .. })
                && facts.iter().any(|facts| grant.scope.matches(facts))
        })
    }

    /// Recursive reads cannot include a remembered denied directory, including
    /// one created after admission. Check captured labels as well as aliases
    /// resolved at admission; later pathname changes must not erase a denial.
    pub fn denies_recursive_file_access(
        &self,
        tool_name: &str,
        paths: &[PathBuf],
        working_dir: &std::path::Path,
        scoped_enabled: bool,
    ) -> bool {
        if !scoped_enabled {
            return false;
        }
        let paths: Vec<_> = paths
            .iter()
            .flat_map(|path| {
                let absolute = if path.is_absolute() {
                    path.clone()
                } else {
                    working_dir.join(path)
                };
                std::iter::once(absolute).chain(CallFacts::for_file_path(path, working_dir).paths)
            })
            .collect();
        let now = Utc::now().timestamp();
        self.scoped.iter().any(|grant| {
            let PermissionScope::Directory { dir } = &grant.scope else {
                return false;
            };
            grant.tool_name == tool_name
                && !grant.allowed
                && grant.is_live(now)
                && grant.scope.validate().is_ok()
                && paths.iter().any(|path| {
                    path.is_absolute() && (path.starts_with(dir) || dir.starts_with(path))
                })
        })
    }

    /// Every live scoped grant, for the settings UI.
    pub fn scoped_grants(&self) -> Vec<ScopedPermissionGrant> {
        let now = Utc::now().timestamp();
        self.scoped
            .iter()
            .filter(|grant| grant.is_live(now))
            .cloned()
            .collect()
    }

    /// Forget every scoped grant for a tool. Returns how many were dropped.
    pub fn revoke_scoped_grants(&mut self, tool_name: &str) -> Result<usize> {
        let before = self.scoped.len();
        self.scoped.retain(|grant| grant.tool_name != tool_name);
        let removed = before - self.scoped.len();
        if removed > 0 {
            self.save()?;
        }
        Ok(removed)
    }

    pub fn cleanup_expired(&mut self) -> anyhow::Result<()> {
        let now = Utc::now().timestamp();
        let mut changed = false;

        self.permissions.retain(|_, records| {
            records.retain(|record| record.expiry.is_none_or(|exp| exp > now));
            changed = changed || records.is_empty();
            !records.is_empty()
        });

        let scoped_before = self.scoped.len();
        self.scoped.retain(|grant| grant.is_live(now));
        changed = changed || self.scoped.len() != scoped_before;

        if changed {
            self.save()?;
        }
        Ok(())
    }
}

/// The identity of one **specific** ask: this tool, with these exact arguments.
///
/// This is the key `check_permission` and `record_permission` have always built
/// (`"{tool_name}:{blake3(args)}"`). BR-71 Task 36b lifted it out so the
/// approval relay keys a delegated grant on exactly the same thing the
/// remembered-permission tier does, rather than inventing a second identity for
/// "that very specific ask".
///
/// `None` for a malformed request (an `Err` `tool_call`) — the same fail-closed
/// degradation `check_permission` already performs.
pub fn exact_call_key(tool_request: &ToolRequest) -> Option<String> {
    let tool_call = tool_request.tool_call.as_ref().ok()?;
    Some(format!(
        "{}:{}",
        tool_call.name,
        hash_tool_context(tool_request)
    ))
}

/// Hash of the tool's arguments, to differentiate similar calls. A malformed
/// request (`Err` `tool_call`) hashes as empty rather than panicking, so it
/// degrades to a stable, argument-less key.
///
/// A free function since Task 36b: it never read `self`.
pub(crate) fn hash_tool_context(tool_request: &ToolRequest) -> String {
    let mut hasher = Hasher::new();
    let serialized = tool_request
        .tool_call
        .as_ref()
        .ok()
        .and_then(|tool_call| serde_json::to_string(&tool_call.arguments).ok())
        .unwrap_or_default();
    hasher.update(serialized.as_bytes());
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{ErrorCode, ErrorData};
    use rmcp::object;
    use std::path::Path;
    use tempfile::TempDir;

    /// A request whose `tool_call` is an `Err` (poisoned/missing payload).
    fn malformed_request() -> ToolRequest {
        ToolRequest {
            id: "bad".to_string(),
            tool_call: Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "poisoned tool_call payload".to_string(),
                None,
            )),
            metadata: None,
            tool_meta: None,
        }
    }

    #[test]
    fn check_permission_treats_malformed_request_as_absent() {
        let store = ToolPermissionStore::new();
        // Must degrade to "no stored permission" rather than panicking on Err.
        assert_eq!(store.check_permission(&malformed_request()), None);
    }

    #[test]
    fn record_permission_rejects_malformed_request() {
        let mut store = ToolPermissionStore::new();
        // Must fail closed (no persistence) instead of panicking on Err.
        let result = store.record_permission(&malformed_request(), true, None);
        assert!(result.is_err());
        assert!(store.permissions.is_empty());
    }

    #[test]
    fn hash_tool_context_is_stable_for_malformed_request() {
        // Hashing a malformed request must not panic and stays deterministic.
        let a = hash_tool_context(&malformed_request());
        let b = hash_tool_context(&malformed_request());
        assert_eq!(a, b);
    }

    #[test]
    fn exact_call_key_is_the_key_the_store_looks_up() {
        let store = ToolPermissionStore::new();
        let request = ToolRequest {
            id: "call-1".to_string(),
            tool_call: Ok(rmcp::model::CallToolRequestParams {
                meta: None,
                name: "developer__shell".into(),
                arguments: Some(object!({"command": "ls"})),
                task: None,
            }),
            metadata: None,
            tool_meta: None,
        };
        assert_eq!(
            exact_call_key(&request).unwrap(),
            format!("developer__shell:{}", hash_tool_context(&request))
        );
        // Fail-closed on a request that carries no recoverable identity.
        assert_eq!(exact_call_key(&malformed_request()), None);
        // And the store still finds nothing for it (unchanged behaviour).
        assert_eq!(store.check_permission(&malformed_request()), None);
    }

    // ------------------------------------------------- BR-24: scoped grants

    const SHELL: &str = "developer__shell";
    const EDITOR: &str = "developer__text_editor";

    fn store_in(temp: &TempDir) -> ToolPermissionStore {
        ToolPermissionStore::new_in(temp.path().join("permissions"))
    }

    fn shell_facts(command: &str) -> CallFacts {
        CallFacts::for_tool_call(SHELL, Some(&object!({"command": command})), Path::new("/"))
    }

    fn editor_facts(path: &Path, working_dir: &Path) -> CallFacts {
        CallFacts::for_tool_call(
            EDITOR,
            Some(&object!({"command": "write", "path": path.to_str().unwrap(), "file_text": "x"})),
            working_dir,
        )
    }

    fn allowed(store: &ToolPermissionStore, tool: &str, facts: &CallFacts) -> Option<bool> {
        store
            .matching_scoped_grant(tool, facts)
            .map(|grant| grant.allowed)
    }

    #[test]
    fn file_access_denial_checks_every_patch_child_despite_overlapping_allows() {
        let temp = TempDir::new().unwrap();
        let workspace = temp.path().canonicalize().unwrap();
        let private = workspace.join("restricted");
        std::fs::create_dir(&private).unwrap();
        let mut store = store_in(&temp);
        store
            .record_scoped_grant(
                EDITOR,
                PermissionScope::directory(&private).unwrap(),
                false,
                None,
            )
            .unwrap();
        store
            .record_scoped_grant(
                EDITOR,
                PermissionScope::directory(&workspace).unwrap(),
                true,
                None,
            )
            .unwrap();
        let targets = vec![
            workspace.clone(),
            workspace.join("ordinary.txt"),
            private.join("new-parent/patch-child.txt"),
        ];
        assert!(store.denies_file_access(EDITOR, &targets, &workspace, true));
        assert!(!store.denies_file_access(EDITOR, &targets[..2], &workspace, true));
        assert!(!store.denies_file_access(EDITOR, &targets, &workspace, false));
        assert!(!store.denies_file_access("other__text_editor", &targets, &workspace, true));
        assert!(!store.denies_file_access(EDITOR, &[], &workspace, true));
        let persisted: ToolPermissionStore = serde_json::from_reader(
            File::open(store.permissions_dir.join("tool_permissions.json")).unwrap(),
        )
        .unwrap();
        assert!(persisted.denies_file_access(EDITOR, &targets, &workspace, true));
    }

    #[test]
    fn file_access_denials_ignore_expired_invalid_and_command_prefix_scopes() {
        let temp = TempDir::new().unwrap();
        let workspace = temp.path().canonicalize().unwrap();
        let target = workspace.join("ordinary.txt");
        let mut store = store_in(&temp);
        store
            .record_scoped_grant(
                EDITOR,
                PermissionScope::directory(&workspace).unwrap(),
                false,
                Some(Duration::ZERO),
            )
            .unwrap();
        store
            .record_scoped_grant(
                EDITOR,
                PermissionScope::command_prefix("write").unwrap(),
                false,
                None,
            )
            .unwrap();
        store.scoped.push(ScopedPermissionGrant {
            tool_name: EDITOR.into(),
            scope: PermissionScope::Directory {
                dir: PathBuf::from("relative-invalid-scope"),
            },
            allowed: false,
            timestamp: Utc::now().timestamp(),
            expiry: None,
        });
        assert!(!store.denies_file_access(EDITOR, &[target], &workspace, true));
    }

    #[test]
    fn recursive_file_denials_include_absent_descendants_without_matching_siblings() {
        let temp = TempDir::new().unwrap();
        let workspace = temp.path().canonicalize().unwrap();
        let private = workspace.join("restricted");
        let sibling = workspace.join("restricted-sibling");
        std::fs::create_dir(&private).unwrap();
        std::fs::create_dir(&sibling).unwrap();
        let mut store = store_in(&temp);
        store
            .record_scoped_grant(
                EDITOR,
                PermissionScope::directory(&workspace).unwrap(),
                true,
                None,
            )
            .unwrap();
        store
            .record_scoped_grant(
                EDITOR,
                PermissionScope::directory(&private).unwrap(),
                false,
                None,
            )
            .unwrap();
        std::fs::remove_dir(&private).unwrap();
        assert!(store.denies_recursive_file_access(
            EDITOR,
            std::slice::from_ref(&workspace),
            &workspace,
            true,
        ));
        assert!(store.denies_recursive_file_access(
            EDITOR,
            &[private.join("future/source.rs")],
            &workspace,
            true,
        ));
        assert!(!store.denies_recursive_file_access(EDITOR, &[sibling], &workspace, true));
        assert!(!store.denies_recursive_file_access(
            EDITOR,
            std::slice::from_ref(&workspace),
            &workspace,
            false,
        ));
        assert!(!store.denies_recursive_file_access(
            "other__text_editor",
            std::slice::from_ref(&workspace),
            &workspace,
            true,
        ));
    }

    #[test]
    fn file_access_denials_resolve_relative_targets_without_prefix_sibling_matches() {
        let temp = TempDir::new().unwrap();
        let workspace = temp.path().canonicalize().unwrap();
        let private = workspace.join("restricted");
        std::fs::create_dir(&private).unwrap();
        std::fs::create_dir(workspace.join("restricted-sibling")).unwrap();
        let mut store = store_in(&temp);
        store
            .record_scoped_grant(
                EDITOR,
                PermissionScope::directory(&private).unwrap(),
                false,
                None,
            )
            .unwrap();
        assert!(store.denies_file_access(
            EDITOR,
            &[PathBuf::from("restricted/new-child.txt")],
            &workspace,
            true,
        ));
        assert!(!store.denies_file_access(
            EDITOR,
            &[PathBuf::from("restricted-sibling/new-child.txt")],
            &workspace,
            true,
        ));
    }

    #[cfg(unix)]
    #[test]
    fn file_access_denials_cover_canonical_aliases_and_discovered_dangling_targets() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let workspace = temp.path().canonicalize().unwrap();
        let private = workspace.join("restricted");
        std::fs::create_dir(&private).unwrap();
        std::fs::write(private.join("existing.txt"), "SYNTHETIC").unwrap();
        let alias = workspace.join("ordinary-alias");
        symlink(private.join("existing.txt"), &alias).unwrap();
        let dangling = workspace.join("dangling-alias");
        let discovered = private.join("new-parent/new-child.txt");
        symlink(&discovered, &dangling).unwrap();
        let mut store = store_in(&temp);
        store
            .record_scoped_grant(
                EDITOR,
                PermissionScope::directory(&private).unwrap(),
                false,
                None,
            )
            .unwrap();
        assert!(store.denies_file_access(EDITOR, &[alias], &workspace, true));
        assert!(store.denies_file_access(EDITOR, &[dangling, discovered], &workspace, true,));
        assert_eq!(
            std::fs::read(private.join("existing.txt")).unwrap(),
            b"SYNTHETIC"
        );
    }

    #[test]
    fn a_command_prefix_grant_allows_the_prefix_and_nothing_else() {
        let temp = TempDir::new().unwrap();
        let mut store = store_in(&temp);
        store
            .record_scoped_grant(
                SHELL,
                PermissionScope::command_prefix("git status").unwrap(),
                true,
                None,
            )
            .unwrap();

        assert_eq!(
            allowed(&store, SHELL, &shell_facts("git status")),
            Some(true)
        );
        assert_eq!(
            allowed(&store, SHELL, &shell_facts("git status --short")),
            Some(true)
        );
        // MUST NOT: a different subcommand is a different command.
        assert_eq!(allowed(&store, SHELL, &shell_facts("git push")), None);
        // MUST NOT: a chained command that merely starts with the prefix.
        assert_eq!(
            allowed(&store, SHELL, &shell_facts("git status && rm -rf /")),
            None
        );
    }

    #[test]
    fn a_scoped_grant_does_not_leak_to_another_extensions_tool() {
        let temp = TempDir::new().unwrap();
        let mut store = store_in(&temp);
        store
            .record_scoped_grant(
                SHELL,
                PermissionScope::command_prefix("git status").unwrap(),
                true,
                None,
            )
            .unwrap();

        // Same base name, different extension: the grant says nothing about it.
        let other = "sketchy__shell";
        let facts = CallFacts::for_tool_call(
            other,
            Some(&object!({"command": "git status"})),
            Path::new("/"),
        );
        assert_eq!(allowed(&store, other, &facts), None);
    }

    #[test]
    fn a_directory_grant_allows_inside_and_not_a_name_prefixed_sibling() {
        let temp = TempDir::new().unwrap();
        let work = temp.path().join("work");
        std::fs::create_dir_all(work.join("a")).unwrap();
        std::fs::create_dir_all(work.join("ab")).unwrap();

        let mut store = store_in(&temp);
        store
            .record_scoped_grant(
                EDITOR,
                PermissionScope::directory(work.join("a")).unwrap(),
                true,
                None,
            )
            .unwrap();

        assert_eq!(
            allowed(
                &store,
                EDITOR,
                &editor_facts(&work.join("a/x.txt"), temp.path())
            ),
            Some(true)
        );
        // MUST NOT: /work/a does not contain /work/ab.
        assert_eq!(
            allowed(
                &store,
                EDITOR,
                &editor_facts(&work.join("ab/x.txt"), temp.path())
            ),
            None
        );
    }

    #[test]
    fn a_matching_deny_wins_over_a_matching_allow() {
        let temp = TempDir::new().unwrap();
        let mut store = store_in(&temp);
        // Broad allow recorded first, narrow deny second...
        store
            .record_scoped_grant(
                SHELL,
                PermissionScope::command_prefix("git").unwrap(),
                true,
                None,
            )
            .unwrap();
        store
            .record_scoped_grant(
                SHELL,
                PermissionScope::command_prefix("git push").unwrap(),
                false,
                None,
            )
            .unwrap();
        assert_eq!(
            allowed(&store, SHELL, &shell_facts("git push")),
            Some(false)
        );
        assert_eq!(
            allowed(&store, SHELL, &shell_facts("git status")),
            Some(true)
        );

        // ...and the same holds when the allow is recorded last: deny wins on
        // matching, not on file order.
        store
            .record_scoped_grant(
                SHELL,
                PermissionScope::command_prefix("git").unwrap(),
                true,
                None,
            )
            .unwrap();
        assert_eq!(
            allowed(&store, SHELL, &shell_facts("git push")),
            Some(false)
        );
    }

    #[test]
    fn re_granting_the_same_scope_replaces_it() {
        let temp = TempDir::new().unwrap();
        let mut store = store_in(&temp);
        let scope = PermissionScope::command_prefix("cargo test").unwrap();

        store
            .record_scoped_grant(SHELL, scope.clone(), true, None)
            .unwrap();
        store
            .record_scoped_grant(SHELL, scope.clone(), false, None)
            .unwrap();

        assert_eq!(store.scoped_grants().len(), 1);
        assert_eq!(
            allowed(&store, SHELL, &shell_facts("cargo test")),
            Some(false)
        );
    }

    #[test]
    fn an_expired_scoped_grant_stops_matching_and_is_pruned() {
        let temp = TempDir::new().unwrap();
        let mut store = store_in(&temp);
        store
            .record_scoped_grant(
                SHELL,
                PermissionScope::command_prefix("git status").unwrap(),
                true,
                Some(Duration::from_secs(60)),
            )
            .unwrap();
        assert_eq!(
            allowed(&store, SHELL, &shell_facts("git status")),
            Some(true)
        );

        // Backdate it past its expiry.
        store.scoped[0].expiry = Some(Utc::now().timestamp() - 1);
        assert_eq!(allowed(&store, SHELL, &shell_facts("git status")), None);
        assert!(store.scoped_grants().is_empty());

        store.cleanup_expired().unwrap();
        assert!(store.scoped.is_empty());
    }

    #[test]
    fn revoking_forgets_every_grant_for_a_tool() {
        let temp = TempDir::new().unwrap();
        let mut store = store_in(&temp);
        store
            .record_scoped_grant(
                SHELL,
                PermissionScope::command_prefix("git status").unwrap(),
                true,
                None,
            )
            .unwrap();
        store
            .record_scoped_grant(
                SHELL,
                PermissionScope::command_prefix("ls").unwrap(),
                true,
                None,
            )
            .unwrap();

        assert_eq!(store.revoke_scoped_grants(SHELL).unwrap(), 2);
        assert_eq!(allowed(&store, SHELL, &shell_facts("git status")), None);
    }

    #[test]
    fn the_store_refuses_to_persist_a_blanket_scope() {
        let temp = TempDir::new().unwrap();
        let mut store = store_in(&temp);

        // An empty command prefix matches every command; `/` contains every path.
        assert!(store
            .record_scoped_grant(
                SHELL,
                PermissionScope::CommandPrefix { tokens: vec![] },
                true,
                None
            )
            .is_err());
        assert!(store
            .record_scoped_grant(
                EDITOR,
                PermissionScope::Directory {
                    dir: PathBuf::from("/")
                },
                true,
                None
            )
            .is_err());
        assert!(store.scoped.is_empty());
    }

    #[test]
    fn a_v1_store_on_disk_loads_with_no_scoped_grants() {
        // The schema bump must not orphan an existing permissions file.
        let v1 = r#"{"permissions":{},"version":1}"#;
        let store: ToolPermissionStore = serde_json::from_str(v1).expect("v1 file still parses");
        assert!(store.scoped.is_empty());
        assert_eq!(store.version, 1);
    }
}
