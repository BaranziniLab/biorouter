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
    /// Per user data that must not roam: large files such as speech models,
    /// which a roaming profile would copy at every logon (DD-19). On Windows
    /// it is `<local root>\data`, beside `cache` under `%LOCALAPPDATA%`; on
    /// macOS and Linux it is `data`, because the XDG data folder never roams.
    pub local_data: PathBuf,
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
        let data = strategy.data_dir();
        Ok(PlatformDirs {
            config: strategy.config_dir(),
            local_data: data.clone(),
            data,
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
/// (`%APPDATA%` and `%LOCALAPPDATA%`): `<root>\config`, `<root>\data`,
/// `<local root>\cache` and `<local root>\data`, each root resolved by
/// [`crate::app_folder::AppRoot::resolve`]. Compiled everywhere, so the layout
/// is tested on every OS.
///
/// `local_data` has no etcetera counterpart (its Windows strategy has one data
/// folder, the roaming one). It sits beside `cache` under the same resolved
/// local root, so the legacy folder rule, the daemon's move of the local base
/// and the lock a resolver holds on it all apply to it unchanged.
pub fn windows_dirs(roaming_base: &Path, local_base: &Path) -> PlatformDirs {
    use crate::app_folder::AppRoot;
    let roaming = AppRoot::under(roaming_base).resolve().to_path_buf();
    let local = AppRoot::under(local_base).resolve().to_path_buf();
    PlatformDirs {
        config: roaming.join("config"),
        data: roaming.join("data"),
        state: None,
        cache: local.join("cache"),
        local_data: local.join("data"),
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

    /// On Windows the local data folder sits under `%LOCALAPPDATA%`, beside
    /// the cache, and never under the roaming base: a speech model of 0.3 to
    /// 0.6 GB there would be copied at every logon on a roaming profile.
    #[test]
    fn the_windows_local_data_folder_is_beside_the_cache_and_never_roams() {
        use crate::app_folder::{APP_AUTHOR, APP_NAME};
        let roaming = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let dirs = windows_dirs(roaming.path(), local.path());
        assert_eq!(
            dirs.local_data,
            local.path().join(APP_AUTHOR).join(APP_NAME).join("data")
        );
        assert_eq!(dirs.local_data.parent(), dirs.cache.parent());
        assert!(!dirs.local_data.starts_with(roaming.path()));
        assert_ne!(dirs.local_data, dirs.data);
    }

    /// The XDG data folder does not roam, so on macOS and Linux the local
    /// data folder is the data folder itself.
    #[cfg(not(windows))]
    #[test]
    fn off_windows_the_local_data_folder_is_the_data_folder() {
        let dirs = platform_dirs().expect("a home dir");
        assert_eq!(dirs.local_data, dirs.data);
    }

    /// `<base>/<token>/<rest>` for the path tokens `resolution-cases.json`
    /// defines (its `about` field spells them out).
    fn case_path(base: &Path, spec: &str) -> PathBuf {
        use crate::app_folder::{APP_AUTHOR, APP_NAME, LEGACY_APP_AUTHOR, MOVE_LOCK, MOVE_MARKER};
        let spec = spec.trim_end_matches('/');
        let (head, rest) = spec.split_once('/').unwrap_or((spec, ""));
        let root = match head {
            "current" => base.join(APP_AUTHOR).join(APP_NAME),
            "legacy" => base.join(LEGACY_APP_AUTHOR).join(APP_NAME),
            "currentParent" => base.join(APP_AUTHOR),
            "legacyParent" => base.join(LEGACY_APP_AUTHOR),
            "lookalike" => base.join(format!("{LEGACY_APP_AUTHOR}2")).join(APP_NAME),
            other => panic!("unknown path token {other:?}"),
        };
        rest.split('/')
            .filter(|segment| !segment.is_empty())
            .fold(root, |path, segment| {
                path.join(match segment {
                    "marker" => MOVE_MARKER,
                    "lock" => MOVE_LOCK,
                    other => other,
                })
            })
    }

    /// Directories a case makes unreadable, given their permissions back on
    /// drop so a failed assertion leaves nothing undeletable behind.
    struct Unreadable(Vec<PathBuf>);

    impl Unreadable {
        /// `None` where the case cannot be set up: not unix, or permissions
        /// that are not enforced (running as root).
        fn make(dirs: Vec<PathBuf>) -> Option<Self> {
            if dirs.is_empty() {
                return Some(Self(dirs));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                for dir in &dirs {
                    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o000)).unwrap();
                }
                let guard = Self(dirs);
                if guard.0.iter().any(|dir| std::fs::read_dir(dir).is_ok()) {
                    return None;
                }
                Some(guard)
            }
            #[cfg(not(unix))]
            {
                drop(dirs);
                None
            }
        }
    }

    impl Drop for Unreadable {
        fn drop(&mut self) {
            #[cfg(unix)]
            for dir in &self.0 {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
            }
        }
    }

    /// The local data folder obeys the legacy folder rule: every resolution
    /// state of the shared table, built under the LOCAL base, puts it in the
    /// folder the table names. The table needs no row of its own for it,
    /// because each case describes one base and the local base was already
    /// one (its cache lives there).
    #[test]
    fn the_local_data_folder_follows_every_resolution_case_of_the_local_base() {
        let table: serde_json::Value =
            serde_json::from_str(include_str!("app_folder/resolution-cases.json")).unwrap();
        let cases = table["resolution"].as_array().expect("resolution cases");
        assert!(cases.len() >= 8, "the table lost resolution cases");
        let strings = |case: &serde_json::Value, key: &str| -> Vec<String> {
            case[key]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(|item| item.as_str().unwrap().to_owned())
                        .collect()
                })
                .unwrap_or_default()
        };
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let roaming = tempfile::tempdir().unwrap();
            let local = tempfile::tempdir().unwrap();
            for spec in strings(case, "before") {
                let path = case_path(local.path(), &spec);
                if spec.ends_with('/') {
                    std::fs::create_dir_all(&path).unwrap();
                } else {
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(&path, &spec).unwrap();
                }
            }
            let unreadable = strings(case, "unreadable")
                .iter()
                .map(|spec| case_path(local.path(), spec))
                .collect();
            let Some(_unreadable) = Unreadable::make(unreadable) else {
                eprintln!("skipped here: {name}");
                continue;
            };
            let resolved = case_path(local.path(), case["resolves"].as_str().unwrap());
            let dirs = windows_dirs(roaming.path(), local.path());
            assert_eq!(
                dirs.local_data,
                resolved.join("data"),
                "{name}: the local data folder is not in the folder the rule resolves"
            );
            assert_eq!(dirs.cache, resolved.join("cache"), "{name}");
        }
    }

    /// A legacy local folder is held while a resolver answers with it, so the
    /// daemon cannot move a model folder out from under a process using it;
    /// once nothing holds it, the move takes the folder whole and the layout
    /// follows it.
    #[test]
    fn the_local_data_folder_is_held_and_then_moved_with_the_local_base() {
        use crate::app_folder::{move_for_process, Pins, MOVE_MARKER, PRODUCTION_OPS};
        let roaming = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let legacy = case_path(local.path(), "legacy");
        let model = std::path::Path::new("dictation")
            .join("models")
            .join("example")
            .join("model.bin");
        std::fs::create_dir_all(legacy.join("data").join(model.parent().unwrap())).unwrap();
        std::fs::write(legacy.join("data").join(&model), b"weights").unwrap();

        let pins = Pins::new();
        let dirs = windows_platform_dirs(roaming.path(), local.path(), false, &pins);
        assert_eq!(dirs.local_data, legacy.join("data"));
        assert!(
            !pins.is_empty(),
            "a resolver answering with the legacy local folder did not hold it"
        );

        let ops = crate::app_folder::MoveOps {
            rename: PRODUCTION_OPS.rename,
            retry_delays: &[],
        };
        let bases = || Some((roaming.path().to_path_buf(), local.path().to_path_buf()));
        // Each report is dropped at the end of its statement: a report keeps
        // the locks of the legacy folders it still resolves, and the first
        // one still resolves this one.
        let first = move_for_process(None, bases, &ops)
            .local
            .unwrap()
            .outcome
            .token();
        assert_eq!(first, "in-use");
        assert!(legacy.join("data").join(&model).is_file());

        drop(pins);
        let second = move_for_process(None, bases, &ops)
            .local
            .unwrap()
            .outcome
            .token();
        assert_eq!(second, "moved");
        let current = case_path(local.path(), "current");
        let dirs = windows_dirs(roaming.path(), local.path());
        assert_eq!(dirs.local_data, current.join("data"));
        assert_eq!(
            std::fs::read(dirs.local_data.join(&model)).unwrap(),
            b"weights"
        );
        assert!(
            !current.join(MOVE_MARKER).exists(),
            "the local base holds no stored paths, so its move leaves no marker"
        );
    }

    #[test]
    fn in_config_dir_appends_the_subdir() {
        // Exercises the real resolver; only asserts the tail so it holds
        // whether or not BIOROUTER_PATH_ROOT is set in the ambient env.
        assert!(in_config_dir("agent_drafter").ends_with("agent_drafter"));
    }
}
