use etcetera::{choose_app_strategy, AppStrategy, AppStrategyArgs};
use std::path::PathBuf;

pub struct Paths;

impl Paths {
    fn get_dir(dir_type: DirType) -> PathBuf {
        // ⚠ A BLANK value reads as ABSENT, deliberately. Taken literally, an empty
        // `BIOROUTER_PATH_ROOT` yields a *cwd-relative* `./config`, and a relative
        // config dir has no cross-process meaning: the daemon, the CLI and the
        // Electron main process each resolve it against a different working
        // directory, so the three stop agreeing on where config lives. Every other
        // resolver on both sides already reads blank as absent —
        // `biorouter-mcp::resolve_config_dir`, `routes::shell::home_dir`, and the
        // desktop `biorouterPaths.ts` — so this was the last holdout, and the one
        // that could aim a recursive-delete writer at `<cwd>/config`.
        // Pinned by `tests/path_resolver_agreement.rs`.
        if let Some(test_root) = Self::path_root_override() {
            let base = PathBuf::from(test_root);
            match dir_type {
                DirType::Config => base.join("config"),
                DirType::Data => base.join("data"),
                DirType::State => base.join("state"),
            }
        } else {
            let strategy = choose_app_strategy(AppStrategyArgs {
                top_level_domain: "Block".to_string(),
                author: "Block".to_string(),
                app_name: "biorouter".to_string(),
            })
            .expect("biorouter requires a home dir");

            match dir_type {
                DirType::Config => strategy.config_dir(),
                DirType::Data => strategy.data_dir(),
                DirType::State => strategy.state_dir().unwrap_or(strategy.data_dir()),
            }
        }
    }

    /// The `BIOROUTER_PATH_ROOT` value [`Self::get_dir`] honours, or `None`
    /// when it reads the variable as absent: unset, not valid UTF-8, or blank
    /// after `trim()`. A root it honours is used as given, untrimmed.
    ///
    /// ⚠ Public for the test binaries' `#[ctor]` sandboxes (`biorouter`,
    /// `biorouter-cli`, `biorouter-server`), which each ask "is a root already
    /// in force, or must I install one?" and must ask it with THIS predicate
    /// rather than a spelling of their own. The lib and CLI ctors spelled it
    /// `var_os(..).is_none_or(|v| v.is_empty())`, which honours `"   "`: the
    /// ctor then records `"   "` as the sandbox while every cell, reading it
    /// as absent here, resolves the platform's real directories. Measured on
    /// the lib and CLI test binaries 2026-09-21, under a throwaway `HOME`:
    /// with `BIOROUTER_PATH_ROOT='   '` each binary's config and session-store
    /// guards failed, naming `<HOME>/.config/biorouter/config.yaml` and
    /// `<HOME>/.local/share/biorouter`; with this predicate, both pass.
    /// In production it has exactly two callers, `get_dir` and
    /// [`Self::managed_policy_path`], so the managed-policy file and every
    /// directory above read a blank root the same way. (Some `biorouter`
    /// integration binaries ask it too, for the same question the ctors ask.)
    #[doc(hidden)]
    pub fn path_root_override() -> Option<String> {
        usable_path_root(std::env::var("BIOROUTER_PATH_ROOT").ok())
    }

    /// The user's home directory, resolved the SAME way on every platform.
    ///
    /// ⚠ Not `dirs::home_dir()`, and the difference is not cosmetic. On unix
    /// that function reads `$HOME`; on Windows it calls
    /// `SHGetKnownFolderPath(FOLDERID_Profile)` and reads no environment
    /// variable at all. So a test — or a sandboxed run — that relocates the home
    /// directory is honoured on macOS and Linux and silently ignored on Windows,
    /// where the process keeps reading the real profile.
    ///
    /// That is not hypothetical: `skill_catalog::roots()` derives
    /// `~/.claude/skills` and `~/.config/agents/skills` from the home directory,
    /// and two `routes::apps` tests that relocate it passed on macOS and Linux
    /// while failing `test (windows-latest)` with their own hermeticity message,
    /// "the skill catalog is reading directories this test does not own".
    ///
    /// Reading the platform's own variable — `USERPROFILE` on Windows, `HOME`
    /// elsewhere — is what makes relocation work identically on all three. The
    /// `dirs` call stays as the fallback for a process whose environment says
    /// nothing, which is the normal case for a real user on Windows.
    ///
    /// A BLANK value reads as absent, for the same reason `BIOROUTER_PATH_ROOT`
    /// does above: an empty home yields cwd-relative paths that no two processes
    /// agree on.
    ///
    /// This is the same rule `routes::shell::home_dir` already applied for the
    /// browser surface; it lives here now so there is one answer rather than two.
    pub fn home_dir() -> Option<PathBuf> {
        let from_env = if cfg!(windows) {
            std::env::var_os("USERPROFILE")
        } else {
            std::env::var_os("HOME")
        };
        from_env
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
            .or_else(dirs::home_dir)
    }

    pub fn config_dir() -> PathBuf {
        Self::get_dir(DirType::Config)
    }

    pub fn data_dir() -> PathBuf {
        Self::get_dir(DirType::Data)
    }

    pub fn state_dir() -> PathBuf {
        Self::get_dir(DirType::State)
    }

    pub fn in_state_dir(subpath: &str) -> PathBuf {
        Self::state_dir().join(subpath)
    }

    pub fn in_config_dir(subpath: &str) -> PathBuf {
        Self::config_dir().join(subpath)
    }

    pub fn in_data_dir(subpath: &str) -> PathBuf {
        Self::data_dir().join(subpath)
    }

    /// Trusted, admin-owned managed-policy file (BR-65). Returns `None` on
    /// platforms without a well-known admin-writable location.
    ///
    /// Deliberately has **no** dedicated env override in production — an env var
    /// is user-settable and would defeat the tamper model. Only the pre-existing
    /// test seam `BIOROUTER_PATH_ROOT` is honored (it relocates the whole config
    /// root under a temp dir the test owns).
    pub fn managed_policy_path() -> Option<PathBuf> {
        // The same reading of the variable as `get_dir`, so a blank root is
        // absent here too. A raw `std::env::var` honoured `""` and `"   "`,
        // turning them into the RELATIVE path `managed/managed-policy.yaml` —
        // a file under whatever directory the process happens to run in —
        // while every other `Paths` resolution read them as absent.
        if let Some(test_root) = Self::path_root_override() {
            return Some(
                PathBuf::from(test_root)
                    .join("managed")
                    .join("managed-policy.yaml"),
            );
        }
        #[cfg(target_os = "macos")]
        {
            Some(PathBuf::from(
                "/Library/Application Support/Biorouter/managed-policy.yaml",
            ))
        }
        #[cfg(target_os = "linux")]
        {
            Some(PathBuf::from("/etc/biorouter/managed-policy.yaml"))
        }
        #[cfg(target_os = "windows")]
        {
            std::env::var("ProgramData").ok().map(|program_data| {
                PathBuf::from(program_data)
                    .join("Biorouter")
                    .join("managed-policy.yaml")
            })
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            None
        }
    }
}

enum DirType {
    Config,
    Data,
    State,
}

/// The rule [`Paths::path_root_override`] applies to the variable's raw
/// value, apart from the environment read so a test can state it without
/// setting a process-global.
fn usable_path_root(raw: Option<String>) -> Option<String> {
    raw.filter(|root| !root.trim().is_empty())
}

#[cfg(test)]
mod path_root_override_tests {
    use super::{usable_path_root, Paths};

    /// Blank is absent, whatever the whitespace; anything else is honoured
    /// exactly as given. This is the one predicate `get_dir` and the three
    /// test-binary ctors share, so pinning it pins all four.
    #[test]
    fn a_blank_path_root_is_no_override_and_any_other_is_taken_verbatim() {
        let cases: [(Option<&str>, Option<&str>); 7] = [
            (None, None),
            (Some(""), None),
            (Some(" "), None),
            (Some("   "), None),
            (Some("\t\n "), None),
            (Some("/tmp/root"), Some("/tmp/root")),
            (Some(" /tmp/padded "), Some(" /tmp/padded ")),
        ];
        for (raw, expected) in cases {
            assert_eq!(
                usable_path_root(raw.map(str::to_owned)).as_deref(),
                expected,
                "BIOROUTER_PATH_ROOT = {raw:?}: Paths must read a blank root as absent \
                 (a sandbox ctor that honoured it would record it while every cell \
                 resolved the real directories) and must not rewrite one it honours"
            );
        }
    }

    /// `managed_policy_path` reads the variable the way `get_dir` does, so a
    /// blank root moves the managed policy nowhere. It read it raw, and made
    /// `""` and `"   "` into the relative `managed/managed-policy.yaml` — a
    /// file under whatever directory the process was started in.
    ///
    /// In a process of its own, because it has to set the variable.
    #[test]
    fn a_blank_path_root_does_not_relocate_the_managed_policy() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        for blank in ["", "   "] {
            let _root = crate::test_sandbox::relocate_path_root(blank);
            let resolved = Paths::managed_policy_path();
            assert!(
                resolved.as_deref().is_none_or(std::path::Path::is_absolute),
                "BIOROUTER_PATH_ROOT = {blank:?} put the managed policy at {resolved:?}, \
                 relative to the working directory; a blank root is no root here, as in \
                 get_dir"
            );
        }
    }
}

#[cfg(test)]
mod home_dir_tests {
    use super::Paths;

    /// The home directory must be relocatable the SAME way on all three
    /// platforms — that is the whole reason this helper exists rather than a
    /// direct `dirs::home_dir()` call.
    ///
    /// `dirs::home_dir()` reads `$HOME` on unix but calls
    /// `SHGetKnownFolderPath(FOLDERID_Profile)` on Windows, reading no
    /// environment at all — so a relocation applied on macOS and Linux was
    /// silently ignored on Windows. Two `routes::apps` tests failed exactly
    /// that way on `test (windows-latest)` while passing everywhere else.
    ///
    /// Asserted at the SOURCE rather than by setting the variable, because a
    /// test can only set the variable its own platform reads; reading the code
    /// is the only way to check the OTHER platform's branch from here.
    #[test]
    fn the_home_directory_is_resolved_from_the_environment_on_every_platform() {
        let source = include_str!("paths.rs");
        let body = source
            .split("pub fn home_dir() -> Option<PathBuf> {")
            .nth(1)
            .expect("Paths::home_dir must exist")
            .split("\n    }")
            .next()
            .expect("its body must be a block");

        assert!(
            body.contains("cfg!(windows)"),
            "the resolver must branch on the platform: the variable is \
             USERPROFILE on Windows and HOME elsewhere. Body was:\n{body}"
        );
        assert!(
            body.contains("USERPROFILE") && body.contains("\"HOME\""),
            "both platform variables must be named, or one OS is unreachable: \n{body}"
        );
        assert!(
            body.contains("dirs::home_dir"),
            "a process whose environment says nothing must still resolve — that \
             is the normal case for a real user on Windows: \n{body}"
        );
        assert!(
            body.contains("as_os_str().is_empty()") || body.contains("is_empty()"),
            "a BLANK value must read as absent, like BIOROUTER_PATH_ROOT above: \n{body}"
        );
    }

    /// …and on THIS platform, relocation actually works end to end.
    ///
    /// In a process of its own: the home directory is as process-global as
    /// `BIOROUTER_PATH_ROOT`, and the skill catalog reads `~/.claude/skills`
    /// from it for every test that builds one.
    #[test]
    #[serial_test::serial]
    fn relocating_the_home_directory_takes_effect_here() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let tmp = tempfile::tempdir().expect("a relocation target");
        let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let _env = env_lock::lock_env([(var, Some(tmp.path().to_string_lossy().into_owned()))]);
        assert_eq!(
            Paths::home_dir().as_deref(),
            Some(tmp.path()),
            "the resolver ignored the platform's own home variable"
        );
    }

    /// A blank value is absent, not a relative root. In a process of its own,
    /// for the reason above.
    #[test]
    #[serial_test::serial]
    fn a_blank_home_falls_back_rather_than_resolving_to_nothing() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let _env = env_lock::lock_env([(var, Some(String::new()))]);
        let resolved = Paths::home_dir();
        assert!(
            resolved.is_none_or(|p| !p.as_os_str().is_empty()),
            "a blank home must never resolve to an empty path"
        );
    }
}
