//! Managed / enterprise policy tier (BR-65).
//!
//! A trusted, admin-owned policy file that wins over user and project config for
//! three governance surfaces: **hooks** (managed hooks always run, resolved first,
//! and can force/forbid project hooks), **permissions** (managed
//! deny/ask/allow rules over tool names) and **dictation** (the `dictation:`
//! block, [`dictation`]). The file lives at an OS-specific
//! admin-writable path (see [`crate::config::paths::Paths::managed_policy_path`])
//! and is ownership-verified before parsing (see [`trust`]).
//!
//! Absent → the policy is **inert**: every query returns "no rule", so a
//! machine with no managed file behaves byte-for-byte as before. Untrusted,
//! unreadable or malformed → the hooks and permissions are inert as well, but
//! dictation is off ([`dictation`], O-27): the `dictation:` block fails closed
//! for itself while the rest of the file fails open.

pub mod dictation;
pub mod settings;
pub mod trust;

use std::path::PathBuf;
use std::sync::Arc;

use tracing::warn;

use crate::hooks::HooksConfig;
use crate::permission::managed_inspector::MANAGED_DENY_REASON;

pub use dictation::{
    Allowance, DictationPolicy, Rule, MANAGED_DICTATION_CAPTURE_OFF, MANAGED_DICTATION_OFF,
};
pub use settings::{ManagedCommandRule, ManagedPermissions, ManagedPolicyFile, ManagedVerdict};

/// The resolved managed policy. [`Self::load`] reads, verifies and parses the
/// file on every call and nothing caches the result, so each caller sees the
/// file as it is when it asks.
#[derive(Debug, Clone, Default)]
pub struct ManagedPolicy {
    file: ManagedPolicyFile,
    /// The trusted path a policy was loaded from; `None` when absent/untrusted,
    /// which makes every hook and permission query inert.
    source: Option<PathBuf>,
    load_failed: bool,
    /// The `dictation:` block, resolved when this value is built (DD-90).
    dictation: DictationPolicy,
}

impl ManagedPolicy {
    /// An inert policy (no managed file). Used as the default everywhere a
    /// machine has no admin policy, and by tests.
    pub fn empty() -> Self {
        Self::default()
    }

    /// No hooks or permissions, and dictation off: what a managed file that
    /// exists but could not be trusted or read leaves behind.
    fn dictation_off() -> Self {
        Self {
            dictation: DictationPolicy::off(),
            ..Self::empty()
        }
    }

    /// Load + verify the managed policy. Absent → an inert policy. Untrusted,
    /// unreadable or malformed → inert hooks and permissions (fail-open on
    /// presence: a bad file must not brick the agent) but dictation off, with
    /// the reason surfaced via `warn!`. Reads the file on every call.
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
        // `exists()` answers false when the check itself fails, which would
        // read a file nobody could look at as no file at all. Dictation fails
        // closed on that; the rest of the policy keeps treating it as absent,
        // as it did before.
        match path.try_exists() {
            Ok(true) => {}
            Ok(false) => return Self::empty(),
            Err(e) => {
                warn!(
                    "managed policy: cannot tell whether {} exists ({e}); dictation is off",
                    path.display()
                );
                return Self::dictation_off();
            }
        }
        if let Err(reason) = trust::verify_trusted(&path) {
            warn!(
                "managed policy: ignoring untrusted managed file at {}: {reason}; dictation is off",
                path.display()
            );
            return Self::dictation_off();
        }
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(e) => {
                warn!(
                    "managed policy: failed to read {}: {e}; dictation is off",
                    path.display()
                );
                return Self {
                    load_failed: true,
                    ..Self::dictation_off()
                };
            }
        };
        match serde_yaml::from_str::<ManagedPolicyFile>(&contents) {
            Ok(file) => {
                let dictation = DictationPolicy::from_block(file.dictation.as_ref(), &path);
                ManagedPolicy {
                    file,
                    source: Some(path),
                    load_failed: false,
                    dictation,
                }
            }
            Err(e) => {
                warn!(
                    "managed policy: failed to parse {}: {e}; dictation is off",
                    path.display()
                );
                Self {
                    load_failed: true,
                    ..Self::dictation_off()
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
        let source = PathBuf::from("<in-memory>");
        let dictation = DictationPolicy::from_block(file.dictation.as_ref(), &source);
        ManagedPolicy {
            file,
            source: Some(source),
            load_failed: false,
            dictation,
        }
    }

    /// Whether a trusted managed file was loaded. Hooks and permissions read
    /// only a trusted file.
    fn trusted(&self) -> bool {
        self.source.is_some()
    }

    /// Whether this policy has anything to enforce, which drives the managed
    /// inspector's `is_enabled`: a trusted file was loaded, or the dictation
    /// block is managed. The second includes a file that could not be trusted,
    /// read or parsed, because dictation fails closed (O-27) and its tool
    /// denials must reach the inspector even then.
    pub fn is_active(&self) -> bool {
        self.trusted() || self.dictation.managed
    }

    /// The administrator's dictation rules (DD-90), computed when this value
    /// was built.
    pub fn dictation(&self) -> &DictationPolicy {
        &self.dictation
    }

    /// Managed hook groups (empty when inert).
    pub fn hooks(&self) -> &HooksConfig {
        &self.file.hooks
    }

    /// Managed override for the user's `allow_project_hooks` opt-in, if any.
    pub fn project_hooks_override(&self) -> Option<bool> {
        if self.trusted() {
            self.file.allow_project_hooks
        } else {
            None
        }
    }

    /// The managed verdict for a tool name, or `None` if no managed rule
    /// applies (or the policy is inert). Deny > Ask > Allow.
    ///
    /// Dictation policy comes first: while it keeps the agent from writing the
    /// dictionary (`enabled: false`, `dictionary: false`,
    /// `dictionary_agent_writes: false`, or a file that failed to load), every
    /// `dictionary__*` tool is denied, whatever `permissions` says.
    pub fn permission_for(&self, tool_name: &str) -> Option<ManagedVerdict> {
        if self.dictation.denies_tool(tool_name) {
            return Some(ManagedVerdict::Deny);
        }
        if !self.trusted() {
            return None;
        }
        self.file.permission_for(tool_name)
    }

    /// The refusal a door that dispatches tool calls outside the agent loop
    /// (`POST /agent/call_tool`) gives a tool this policy denies. Without it a
    /// managed deny, which the inspector chain enforces, did not hold there.
    ///
    /// Only `Deny` is refused. A managed `Ask` is not: the route's callers
    /// include the person's own actions in the desktop app, and a refusal there
    /// would turn "ask first" into "never".
    pub fn uninspected_boundary_refusal(&self, tool_name: &str) -> Option<&'static str> {
        (self.permission_for(tool_name) == Some(ManagedVerdict::Deny))
            .then_some(MANAGED_DENY_REASON)
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
        assert!(!policy.trusted(), "untrusted file must be inert");
        assert!(policy.permission_for("developer__shell").is_none());
        assert!(policy.project_hooks_override().is_none());
        assert!(policy.hooks().is_empty());
        // ...except for dictation, which fails closed (O-27).
        assert_dictation_off(&policy);
    }

    #[test]
    fn malformed_file_is_ignored() {
        let (_dir, policy) = load_with_managed_yaml("permissions: [this is not a map]\n");
        assert!(!policy.trusted());
        assert!(policy.permission_for("developer__shell").is_none());
        assert_dictation_off(&policy);
    }

    /// Dictation is off and the agent's dictionary tools are denied, and the
    /// managed inspector is enabled so that denial reaches the agent loop.
    fn assert_dictation_off(policy: &ManagedPolicy) {
        let dictation = policy.dictation();
        assert!(dictation.managed);
        assert_eq!(dictation.enabled.reason(), Some(MANAGED_DICTATION_OFF));
        assert_eq!(dictation.capture.reason(), Some(MANAGED_DICTATION_OFF));
        assert!(!dictation.install("whisper-small.en-q8_0").allowed);
        assert_eq!(
            policy.permission_for("dictionary__add_terms"),
            Some(ManagedVerdict::Deny)
        );
        assert!(policy.is_active(), "the managed inspector must run");
        assert_eq!(
            policy.uninspected_boundary_refusal("dictionary__list_terms"),
            Some(MANAGED_DENY_REASON)
        );
    }

    /// Stage `yaml` with explicit file and folder modes.
    fn stage(dir: &std::path::Path, yaml: &str, file_mode: u32, dir_mode: u32) -> PathBuf {
        let managed_dir = dir.join("managed");
        std::fs::create_dir_all(&managed_dir).unwrap();
        let file = managed_dir.join("managed-policy.yaml");
        std::fs::write(&file, yaml).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(file_mode)).unwrap();
        std::fs::set_permissions(&managed_dir, std::fs::Permissions::from_mode(dir_mode)).unwrap();
        file
    }

    /// Group writable files and folders are untrusted the same way a file owned
    /// by another account is (`trust::verify_trusted` refuses all three), and
    /// all of them turn dictation off while the rest of the file is ignored.
    /// A file owned by another account cannot be staged without root.
    #[test]
    fn an_untrusted_file_turns_dictation_off() {
        for (file_mode, dir_mode) in [(0o664, 0o755), (0o644, 0o775), (0o644, 0o777)] {
            let dir = tempfile::tempdir().unwrap();
            stage(
                dir.path(),
                "dictation:\n  enabled: true\npermissions:\n  deny: [\"developer__shell\"]\n",
                file_mode,
                dir_mode,
            );
            let policy = ManagedPolicy::load_from(Some(managed_file_under(dir.path())));
            assert!(!policy.trusted(), "{file_mode:o}/{dir_mode:o}");
            assert!(policy.permission_for("developer__shell").is_none());
            assert_dictation_off(&policy);
            assert!(
                !policy.load_failed,
                "an untrusted file is not a failed load"
            );
        }
    }

    /// A file that exists but cannot be read is a failed load: Crew refuses it
    /// as before, and dictation is off. A folder in the file's place fails the
    /// read on every system.
    #[test]
    fn an_unreadable_file_turns_dictation_off() {
        let dir = tempfile::tempdir().unwrap();
        let managed_dir = dir.path().join("managed");
        std::fs::create_dir_all(managed_dir.join("managed-policy.yaml")).unwrap();
        for folder in [managed_dir.join("managed-policy.yaml"), managed_dir.clone()] {
            std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let policy = ManagedPolicy::load_from(Some(managed_file_under(dir.path())));
        assert!(policy.load_failed);
        assert!(policy.ensure_crew_compatible().is_err());
        assert_dictation_off(&policy);
    }

    /// When the existence check itself fails (a folder this account cannot
    /// search), dictation is off; the rest of the policy treats the file as
    /// absent, as it did before.
    #[test]
    fn a_policy_that_cannot_be_looked_at_turns_dictation_off() {
        // SAFETY: `geteuid` only reads the process's effective uid.
        if unsafe { libc::geteuid() } == 0 {
            return; // root searches any folder, so the check cannot fail
        }
        let dir = tempfile::tempdir().unwrap();
        stage(dir.path(), "permissions: {}\n", 0o644, 0o000);
        let policy = ManagedPolicy::load_from(Some(managed_file_under(dir.path())));
        let managed_dir = dir.path().join("managed");
        std::fs::set_permissions(&managed_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!policy.trusted());
        assert!(!policy.load_failed);
        assert!(policy.ensure_crew_compatible().is_ok());
        assert_dictation_off(&policy);
    }

    #[test]
    fn a_trusted_block_is_read_from_the_file() {
        let (_dir, policy) = load_with_managed_yaml(
            "dictation:\n  capture: false\n  allowed_models: [whisper-small.en-q8_0]\n",
        );
        let dictation = policy.dictation();
        assert!(dictation.managed);
        assert!(dictation.enabled.is_allowed());
        assert_eq!(
            dictation.capture.reason(),
            Some(MANAGED_DICTATION_CAPTURE_OFF)
        );
        assert!(dictation.install("whisper-small.en-q8_0").allowed);
        assert!(!dictation.install("whisper-large-v3-turbo-q5_0").allowed);
        assert!(policy.permission_for("dictionary__add_terms").is_none());
    }

    /// O-27 through the real load: an unknown key turns dictation off and the
    /// file's `permissions.deny` still holds.
    #[test]
    fn an_unknown_key_in_a_trusted_file_turns_only_dictation_off() {
        let (_dir, policy) = load_with_managed_yaml(
            "dictation:\n  enabled: true\n  a_newer_key: true\npermissions:\n  deny: [\"developer__shell\"]\n",
        );
        assert!(policy.trusted());
        assert!(!policy.load_failed);
        assert_eq!(
            policy.permission_for("developer__shell"),
            Some(ManagedVerdict::Deny)
        );
        assert_dictation_off(&policy);
    }

    /// O-27 through the real load, for the likeliest slip: a block whose keys
    /// lost their indentation reads as an empty `dictation:` plus a top-level
    /// `enabled: false` the file ignores. Dictation must be off, not on, and
    /// `permissions.deny` still holds.
    #[test]
    fn an_unindented_block_in_a_trusted_file_turns_only_dictation_off() {
        let (_dir, policy) = load_with_managed_yaml(
            "dictation:\nenabled: false\npermissions:\n  deny: [\"developer__shell\"]\n",
        );
        assert!(policy.trusted());
        assert!(!policy.load_failed);
        assert_eq!(
            policy.permission_for("developer__shell"),
            Some(ManagedVerdict::Deny)
        );
        assert_dictation_off(&policy);
    }

    /// Each load reads the file as it is now, so an administrator's change
    /// applies at the next request.
    #[test]
    fn each_load_reads_the_file_again() {
        let dir = tempfile::tempdir().unwrap();
        let file = stage(dir.path(), "dictation:\n  enabled: true\n", 0o644, 0o755);
        let path = Some(managed_file_under(dir.path()));
        assert!(ManagedPolicy::load_from(path.clone())
            .dictation()
            .enabled
            .is_allowed());
        std::fs::write(&file, "dictation:\n  enabled: false\n").unwrap();
        assert_dictation_off(&ManagedPolicy::load_from(path.clone()));
        std::fs::remove_file(&file).unwrap();
        assert!(!ManagedPolicy::load_from(path).dictation().managed);
    }

    /// A managed deny holds at the door that skips the inspector chain; a
    /// managed ask and allow do not refuse there.
    #[test]
    fn the_boundary_refusal_follows_managed_deny_only() {
        let file: ManagedPolicyFile = serde_yaml::from_str(
            "permissions:\n  deny: [\"developer__shell\"]\n  ask: [\"memory__.*\"]\n  allow: [\"todo__write\"]\n",
        )
        .unwrap();
        let policy = ManagedPolicy::from_file(file);
        assert_eq!(
            policy.uninspected_boundary_refusal("developer__shell"),
            Some(MANAGED_DENY_REASON)
        );
        assert_eq!(policy.uninspected_boundary_refusal("memory__store"), None);
        assert_eq!(policy.uninspected_boundary_refusal("todo__write"), None);
        assert_eq!(
            policy.uninspected_boundary_refusal("dictionary__add_terms"),
            None
        );
        assert_eq!(
            ManagedPolicy::empty().uninspected_boundary_refusal("developer__shell"),
            None
        );
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
