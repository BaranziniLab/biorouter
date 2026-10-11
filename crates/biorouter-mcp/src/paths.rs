//! The one env-aware path resolver for `biorouter-mcp`.
//!
//! This crate cannot depend on `biorouter` (that would be circular), which is
//! why `biorouter::config::Paths` — the authoritative resolver — is not
//! reachable from here. Before this module existed, three separate places
//! hand-rolled a `choose_app_strategy(...)` call and each of them silently
//! ignored `BIOROUTER_PATH_ROOT`:
//!
//!   * `agent_drafter::default_root()`      — drafted apps
//!   * `agent_drafter::skills_root_for_export()` — installed skills
//!   * `knowledge::paths::knowledge_root()` — knowledge bases (and with a
//!     *different* app-strategy tuple, `io/biorouter/biorouter`)
//!
//! A fourth, `memory::global_memory_dir()` — the global memory store — was
//! found the same way and now routes through here too.
//!
//! The consequence was that a sandboxed run — a test drive, a worktree, a
//! per-app jail — wrote drafted apps into, read knowledge bases out of, and
//! rewrote the memories in, the user's **global** store.
//!
//! `biorouter` carries a cross-crate test (`tests/path_resolver_agreement.rs`)
//! asserting this module agrees with `biorouter::config::Paths` byte for byte,
//! and each store that routes through here is pinned to it. Adding a store
//! here means adding its assertion too.
//!
//! Every platform directory in both crates, `Paths` included, now comes from
//! [`platform_dirs`], because on Windows the answer depends on whether the
//! folder earlier releases used has been moved yet ([`crate::app_folder`]);
//! the same test fails on any other `choose_app_strategy` call in this crate. Not every
//! caller honours `BIOROUTER_PATH_ROOT`, and that part is deliberate: the
//! cache-dir resolvers (`webdocuments`, `autovisualiser`) are outside this
//! module's remit, and `developer::undo_history` hand-rolls the same
//! `BIOROUTER_PATH_ROOT` branch correctly.

use etcetera::{choose_app_strategy, AppStrategy, HomeDirError};
use std::path::{Path, PathBuf};

/// The `BIOROUTER_PATH_ROOT` this crate honours: unset, not UTF-8 and blank
/// (after `trim()`) all read as absent, the rule `biorouter::config::Paths`
/// applies too.
pub fn path_root_override() -> Option<String> {
    std::env::var("BIOROUTER_PATH_ROOT")
        .ok()
        .filter(|root| !root.trim().is_empty())
}

/// The platform directories, before any `BIOROUTER_PATH_ROOT` override. Every
/// resolver in both crates comes through here, `biorouter::config::Paths`
/// included, so there is one answer to "where is the app folder".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformDirs {
    pub config: PathBuf,
    pub data: PathBuf,
    /// `None` on Windows, where `Paths` falls back to `data`.
    pub state: Option<PathBuf>,
    pub cache: PathBuf,
}

/// [`PlatformDirs`] for this machine.
///
/// On Windows the app folder is the one [`crate::app_folder`] resolves right
/// now. This never moves anything (only the daemon does, as it starts); while
/// it answers with the folder earlier releases used, this process holds that
/// folder's lock, so no move happens under a path it may have frozen. Not
/// under `BIOROUTER_PATH_ROOT`, where the real folders are not this process's
/// to hold. Elsewhere it is etcetera's XDG strategy, which never reads the
/// author.
pub fn platform_dirs() -> Result<PlatformDirs, HomeDirError> {
    if cfg!(windows) {
        let (roaming, local) = crate::app_folder::windows_bases().ok_or(HomeDirError)?;
        Ok(windows_platform_dirs(
            &roaming,
            &local,
            path_root_override().is_some(),
            crate::app_folder::process_pins(),
        ))
    } else {
        let strategy = choose_app_strategy(crate::APP_STRATEGY.clone())?;
        Ok(PlatformDirs {
            config: strategy.config_dir(),
            data: strategy.data_dir(),
            state: strategy.state_dir(),
            cache: strategy.cache_dir(),
        })
    }
}

/// What [`platform_dirs`] answers on Windows, with the process's locks passed
/// in: outside a sandbox it first holds the lock of any legacy folder it is
/// about to answer with, then lays out the folders. Compiled everywhere, so the
/// hold is tested on every OS.
pub fn windows_platform_dirs(
    roaming_base: &Path,
    local_base: &Path,
    sandboxed: bool,
    pins: &crate::app_folder::Pins,
) -> PlatformDirs {
    use crate::app_folder::AppRoot;
    if !sandboxed {
        pins.hold_while_legacy(&[&AppRoot::under(roaming_base), &AppRoot::under(local_base)]);
    }
    windows_dirs(roaming_base, local_base)
}

/// The Windows layout under the two bases etcetera's Windows strategy reads
/// (`%APPDATA%` and `%LOCALAPPDATA%`): `<root>\config`, `<root>\data` and
/// `<local root>\cache`, each root resolved by [`crate::app_folder::AppRoot::resolve`].
/// Compiled everywhere, so the layout is tested on every OS.
pub fn windows_dirs(roaming_base: &Path, local_base: &Path) -> PlatformDirs {
    use crate::app_folder::AppRoot;
    let roaming = AppRoot::under(roaming_base).resolve().to_path_buf();
    let local = AppRoot::under(local_base).resolve().to_path_buf();
    PlatformDirs {
        config: roaming.join("config"),
        data: roaming.join("data"),
        state: None,
        cache: local.join("cache"),
    }
}

/// Resolve the config dir the way `biorouter::config::Paths::get_dir(Config)`
/// does: `$BIOROUTER_PATH_ROOT/config` when the override is set and non-empty,
/// otherwise the platform config dir.
pub fn config_dir() -> PathBuf {
    let root = std::env::var("BIOROUTER_PATH_ROOT").ok();
    resolve_config_dir(root.as_deref(), &platform_config_dir())
}

/// `<config>/<sub>`.
pub fn in_config_dir(sub: &str) -> PathBuf {
    config_dir().join(sub)
}

/// The pure core of [`config_dir`], split out so it is testable without
/// mutating process env (which races every other test in the binary).
pub fn resolve_config_dir(root_env: Option<&str>, platform_fallback: &Path) -> PathBuf {
    match root_env {
        Some(root) if !root.trim().is_empty() => PathBuf::from(root).join("config"),
        _ => platform_fallback.to_path_buf(),
    }
}

fn platform_config_dir() -> PathBuf {
    platform_dirs()
        .map(|dirs| dirs.config)
        .unwrap_or_else(|_| PathBuf::from(".config/biorouter"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_root_override_wins() {
        let fallback = PathBuf::from("/home/u/.config/biorouter");
        assert_eq!(
            resolve_config_dir(Some("/tmp/sandbox"), &fallback),
            PathBuf::from("/tmp/sandbox/config")
        );
    }

    #[test]
    fn empty_or_absent_override_falls_back_to_the_platform_dir() {
        let fallback = PathBuf::from("/home/u/.config/biorouter");
        assert_eq!(resolve_config_dir(None, &fallback), fallback);
        assert_eq!(resolve_config_dir(Some(""), &fallback), fallback);
        assert_eq!(resolve_config_dir(Some("   "), &fallback), fallback);
    }

    /// A resolver outside a sandbox holds the lock of the legacy folder it
    /// answers with, so the daemon cannot move it away; under a sandbox it
    /// holds nothing, because the real folders are not its to keep.
    #[test]
    fn windows_resolution_holds_the_legacy_folder_it_answers_with() {
        use crate::app_folder::{move_for_process, Pins, PRODUCTION_OPS};
        let roaming = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let legacy = roaming
            .path()
            .join(crate::app_folder::LEGACY_APP_AUTHOR)
            .join(crate::app_folder::APP_NAME);
        std::fs::create_dir_all(legacy.join("config")).unwrap();

        let sandboxed = Pins::new();
        let dirs = windows_platform_dirs(roaming.path(), local.path(), true, &sandboxed);
        assert_eq!(dirs.config, legacy.join("config"));
        assert!(
            sandboxed.is_empty(),
            "a sandboxed process held a real folder"
        );

        let pins = Pins::new();
        let dirs = windows_platform_dirs(roaming.path(), local.path(), false, &pins);
        assert_eq!(dirs.config, legacy.join("config"));
        assert!(
            !pins.is_empty(),
            "the resolver did not hold the folder it answered with"
        );

        let ops = crate::app_folder::MoveOps {
            rename: PRODUCTION_OPS.rename,
            retry_delays: &[],
        };
        let daemon = move_for_process(
            None,
            || Some((roaming.path().to_path_buf(), local.path().to_path_buf())),
            &ops,
        );
        assert_eq!(daemon.roaming.unwrap().outcome.token(), "in-use");
        assert!(legacy.exists());
    }

    #[test]
    fn in_config_dir_appends_the_subdir() {
        // Exercises the real resolver; only asserts the tail so it holds
        // whether or not BIOROUTER_PATH_ROOT is set in the ambient env.
        assert!(in_config_dir("agent_drafter").ends_with("agent_drafter"));
    }
}
