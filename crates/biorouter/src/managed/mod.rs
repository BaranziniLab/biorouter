//! Managed / enterprise policy tier (BR-65).
//!
//! A trusted, admin-owned policy file that wins over user and project config for
//! two governance surfaces: **hooks** (managed hooks always run, resolved first,
//! and can force/forbid project hooks) and **permissions** (managed
//! deny/ask/allow rules over tool names). The file lives at an OS-specific
//! admin-writable path (see [`crate::config::paths::Paths::managed_policy_path`])
//! and is ownership-verified before parsing (see [`trust`]).
//!
//! Absent or untrusted → the policy is **inert**: every query returns "no rule",
//! so a machine with no managed file behaves byte-for-byte as before.

pub mod settings;
pub mod trust;

use std::path::PathBuf;
use std::sync::Arc;

use tracing::warn;

use crate::hooks::HooksConfig;

pub use settings::{ManagedCommandRule, ManagedPermissions, ManagedPolicyFile, ManagedVerdict};

/// The resolved managed policy for this process. Loaded once at startup.
#[derive(Debug, Clone, Default)]
pub struct ManagedPolicy {
    file: ManagedPolicyFile,
    /// The trusted path a policy was loaded from; `None` when absent/untrusted,
    /// which makes every query inert.
    source: Option<PathBuf>,
    load_failed: bool,
}

impl ManagedPolicy {
    /// An inert policy (no managed file). Used as the default everywhere a
    /// machine has no admin policy, and by tests.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Load + verify the managed policy once. Absent, untrusted, or malformed →
    /// an inert policy (fail-open on presence: a bad file must not brick the
    /// agent), with the reason surfaced via `warn!`.
    pub fn load() -> Arc<ManagedPolicy> {
        Arc::new(Self::load_inner())
    }

    fn load_inner() -> Self {
        Self::load_from(crate::config::paths::Paths::managed_policy_path())
    }

    /// [`Self::load_inner`] for the file at `path` rather than at
    /// `Paths::managed_policy_path()`. The tests below stage a policy in a
    /// directory of their own and pass it here, instead of pointing
    /// `BIOROUTER_PATH_ROOT` at it: every test in the binary that loads the
    /// managed policy without the env lock would otherwise have read their
    /// `deny: [developer__shell]` for as long as it was held.
    fn load_from(path: Option<PathBuf>) -> Self {
        let Some(path) = path else {
            return Self::empty();
        };
        if !path.exists() {
            return Self::empty();
        }
        if let Err(reason) = trust::verify_trusted(&path) {
            warn!(
                "managed policy: ignoring untrusted managed file at {}: {reason}",
                path.display()
            );
            return Self::empty();
        }
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(e) => {
                warn!("managed policy: failed to read {}: {e}", path.display());
                return Self {
                    load_failed: true,
                    ..Self::empty()
                };
            }
        };
        match serde_yaml::from_str::<ManagedPolicyFile>(&contents) {
            Ok(file) => ManagedPolicy {
                file,
                source: Some(path),
                load_failed: false,
            },
            Err(e) => {
                warn!("managed policy: failed to parse {}: {e}", path.display());
                Self {
                    load_failed: true,
                    ..Self::empty()
                }
            }
        }
    }

    pub fn ensure_crew_compatible(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.load_failed,
            "Crew is unavailable because the managed policy could not be loaded. Contact your administrator.");
        anyhow::ensure!(self.hooks().is_empty() && self.project_hooks_override() != Some(true),
            "Crew is unavailable with required managed hooks. Contact your administrator for a compatible managed policy.");
        Ok(())
    }

    /// Build directly from a parsed file (tests / in-memory policies). Marked
    /// active so its rules apply.
    pub fn from_file(file: ManagedPolicyFile) -> Self {
        ManagedPolicy {
            file,
            source: Some(PathBuf::from("<in-memory>")),
            load_failed: false,
        }
    }

    /// Whether a trusted managed file was loaded (drives inspector `is_enabled`).
    pub fn is_active(&self) -> bool {
        self.source.is_some()
    }

    /// Managed hook groups (empty when inert).
    pub fn hooks(&self) -> &HooksConfig {
        &self.file.hooks
    }

    /// Managed override for the user's `allow_project_hooks` opt-in, if any.
    pub fn project_hooks_override(&self) -> Option<bool> {
        if self.is_active() {
            self.file.allow_project_hooks
        } else {
            None
        }
    }

    /// The managed verdict for a tool name, or `None` if no managed rule
    /// applies (or the policy is inert). Deny > Ask > Allow.
    pub fn permission_for(&self, tool_name: &str) -> Option<ManagedVerdict> {
        if !self.is_active() {
            return None;
        }
        self.file.permission_for(tool_name)
    }
}

// These tests stage files with explicit Unix modes to exercise trust
// verification, so they are Unix-only.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Where `Paths::managed_policy_path()` puts the file under a root: the
    /// shape `load_from` is handed below.
    fn managed_file_under(root: &std::path::Path) -> PathBuf {
        root.join("managed").join("managed-policy.yaml")
    }

    /// Stage a managed file in a fresh directory and return it (kept alive for
    /// the duration) plus the policy loaded from it.
    fn load_with_managed_yaml(yaml: &str) -> (tempfile::TempDir, ManagedPolicy) {
        let dir = tempfile::tempdir().unwrap();
        let managed_dir = dir.path().join("managed");
        std::fs::create_dir_all(&managed_dir).unwrap();
        std::fs::set_permissions(&managed_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let file = managed_dir.join("managed-policy.yaml");
        std::fs::write(&file, yaml).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

        let policy = ManagedPolicy::load_from(Some(managed_file_under(dir.path())));
        (dir, policy)
    }

    /// Production reads the file `Paths::managed_policy_path()` names, and
    /// under the sandbox that is the path the tests above stage theirs at.
    /// Paths only; nothing is written.
    #[test]
    fn the_staged_path_is_the_one_production_reads() {
        let _root = crate::test_sandbox::pin_sandbox_path_root();
        assert_eq!(
            crate::config::paths::Paths::managed_policy_path(),
            Some(managed_file_under(std::path::Path::new(
                crate::test_sandbox::sandbox_path_root()
            )))
        );
    }

    #[test]
    fn absent_file_is_inert() {
        let dir = tempfile::tempdir().unwrap();
        let policy = ManagedPolicy::load_from(Some(managed_file_under(dir.path())));
        assert!(!policy.is_active());
        assert!(policy.permission_for("developer__shell").is_none());
        assert!(policy.project_hooks_override().is_none());
        assert!(policy.hooks().is_empty());
    }

    #[test]
    fn empty_policy_allows_ordinary_unscoped_admission() {
        ManagedPolicy::empty()
            .ensure_crew_compatible()
            .expect("an absent managed policy must not block ordinary Crew admission");
    }

    #[test]
    fn required_managed_hooks_refuse_crew_admission() {
        let (_dir, policy) = load_with_managed_yaml(
            "hooks:\n  PreToolUse:\n    - hooks: [{ type: command, command: 'echo managed' }]\n",
        );
        let error = policy
            .ensure_crew_compatible()
            .expect_err("required managed hooks must refuse Crew admission");
        assert!(error.to_string().contains("required managed hooks"));
    }

    #[test]
    fn forced_project_hooks_refuse_crew_admission() {
        let file: ManagedPolicyFile = serde_yaml::from_str("allow_project_hooks: true\n").unwrap();
        let error = ManagedPolicy::from_file(file)
            .ensure_crew_compatible()
            .expect_err("forced project hooks must refuse Crew admission");
        assert!(error.to_string().contains("required managed hooks"));
    }

    #[test]
    fn managed_policy_load_failure_refuses_crew_admission() {
        let (_dir, policy) = load_with_managed_yaml("permissions: [not-a-map]\n");
        let error = policy
            .ensure_crew_compatible()
            .expect_err("a managed load failure must fail closed for Crew");
        assert!(error.to_string().contains("could not be loaded"));
    }

    #[test]
    fn trusted_file_loads_and_applies() {
        let (_dir, policy) = load_with_managed_yaml(
            "permissions:\n  deny: [\"developer__shell\"]\nallow_project_hooks: false\n",
        );
        assert!(policy.is_active());
        assert_eq!(
            policy.permission_for("developer__shell"),
            Some(ManagedVerdict::Deny)
        );
        assert_eq!(policy.project_hooks_override(), Some(false));
    }

    #[test]
    fn world_writable_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let managed_dir = dir.path().join("managed");
        std::fs::create_dir_all(&managed_dir).unwrap();
        std::fs::set_permissions(&managed_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let file = managed_dir.join("managed-policy.yaml");
        std::fs::write(&file, "permissions:\n  deny: [\"developer__shell\"]\n").unwrap();
        // World-writable: an attacker could rewrite policy, so it must be ignored.
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o666)).unwrap();

        let policy = ManagedPolicy::load_from(Some(managed_file_under(dir.path())));
        assert!(!policy.is_active(), "untrusted file must be inert");
        assert!(policy.permission_for("developer__shell").is_none());
    }

    #[test]
    fn malformed_file_is_ignored() {
        let (_dir, policy) = load_with_managed_yaml("permissions: [this is not a map]\n");
        assert!(!policy.is_active());
    }

    #[test]
    fn from_file_is_active() {
        let file: ManagedPolicyFile =
            serde_yaml::from_str("permissions:\n  allow: [\"x\"]\n").unwrap();
        let policy = ManagedPolicy::from_file(file);
        assert!(policy.is_active());
        assert_eq!(policy.permission_for("x"), Some(ManagedVerdict::Allow));
    }
}
