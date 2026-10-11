//! `biorouter-mcp` cannot depend on `biorouter` (circular), so it carries its own
//! copy of the config-dir resolver. This test — in the one crate that can see
//! both — pins the two together.
//!
//! It exists because three hand-rolled `choose_app_strategy(...)` calls in
//! `biorouter-mcp` silently ignored `BIOROUTER_PATH_ROOT`, so an isolated run
//! (a test drive, a worktree, a per-app jail) wrote its drafted apps into, and
//! read knowledge bases out of, the user's **global** store. If someone adds a
//! fourth hand-rolled resolver, this fails.
//!
//! ⚠ There is a THIRD resolver, in a language this test cannot reach:
//! `ui/desktop/src/utils/biorouterPaths.ts`, the Electron main process's one
//! derivation (#146). It matters for the same reason as the others and more so,
//! because it WRITES — the `.brxt` handlers create, extract into and
//! *recursively delete* the extensions directory — so a disagreement there is a
//! sandbox escape rather than a stale read. Its own suite,
//! `ui/desktop/src/utils/extensionUpdater.test.ts`, pins the same rules from
//! the other side, including the `XDG_CONFIG_HOME` and Windows layouts this
//! file's `Paths::config_dir()` calls resolve through `etcetera`. The two
//! change together or not at all.
//!
//! On Windows both sides also run the move out of the folder earlier releases
//! used (R-31, `biorouter_mcp::app_folder`). Which folder each resolves for
//! each state on disk is pinned by one table both suites run,
//! `crates/biorouter-mcp/src/app_folder/resolution-cases.json`; the tests at
//! the end of this file pin that `Paths` reaches that rule, that the layout it
//! builds is etcetera's own, and that the rewrite after a move covers the files
//! the stores really use.

use biorouter::config::paths::Paths;
use serial_test::serial;
use std::path::PathBuf;

/// Runs `f` with `BIOROUTER_PATH_ROOT` pinned to `root`, restoring the prior
/// value after.
///
/// The `env_lock` guard is what makes this safe, and it does two things a bare
/// `set_var`/`remove_var` pair cannot. It restores the previous value from
/// `Drop`, so an assertion failure inside `f` cannot leak this file's scratch
/// root into the rest of the process; and it holds the same process-wide mutex
/// every other `BIOROUTER_PATH_ROOT` test in the workspace takes, which
/// `#[serial]` does not — `#[serial]` only orders tests that are themselves
/// annotated, and this variable is read by `Paths::*` on every call from
/// anywhere.
fn with_path_root<T>(root: Option<&str>, f: impl FnOnce() -> T) -> T {
    let _guard = env_lock::lock_env([("BIOROUTER_PATH_ROOT", root)]);
    f()
}

#[test]
#[serial]
fn mcp_config_dir_matches_the_authoritative_resolver() {
    // ⚠ The blank values (`Some("")`, `Some("   ")`) are deliberately NOT in
    // this list: the two resolvers genuinely disagree there, and that
    // disagreement has its own test below rather than being asserted away here.
    // Everything else must agree byte for byte, and the awkward-but-real roots
    // are in the list because both sides must take the value RAW — a resolver
    // that trimmed or normalised it would drift from the other for a directory
    // that legitimately contains spaces.
    for root in [
        None,
        Some("/tmp/br-path-agreement"),
        Some("/tmp/br-path-agreement/"),
        Some("/tmp/br path agreement"),
        Some("/tmp/ br-path-agreement"),
    ] {
        with_path_root(root, || {
            assert_eq!(
                biorouter_mcp::paths::config_dir(),
                Paths::config_dir(),
                "biorouter-mcp::paths::config_dir() drifted from biorouter::config::Paths \
                 (BIOROUTER_PATH_ROOT={root:?})"
            );
        });
    }
}

/// A `BIOROUTER_PATH_ROOT` that is **set but blank** is read three different
/// ways, and this pins the only property that must hold whichever way it is
/// eventually settled: **no resolver may answer with a different ABSOLUTE
/// directory than the others.**
///
/// Measured, and the reason this test exists:
///
/// | value      | `Paths::get_dir`        | `resolve_config_dir` |
/// |------------|-------------------------|----------------------|
/// | unset      | platform dir            | platform dir         |
/// | `/tmp/x`   | `/tmp/x/config`         | `/tmp/x/config`      |
/// | `""`       | **`config`** (relative) | platform dir         |
/// | `"   "`    | **`   /config`**        | platform dir         |
///
/// `Paths::get_dir` uses `if let Ok(test_root) = env::var(…)`, and `env::var`
/// returns `Ok("")` for a variable that is exported but empty — so the daemon
/// resolves a **cwd-relative** `./config`.
///
/// **The decision: blank means unset.** Not taste — a relative config dir has
/// no cross-process meaning at all. The daemon, the CLI and the desktop main
/// process each have their own working directory, so `./config` names a
/// different tree in each of them; mirroring the literal reading elsewhere
/// cannot restore agreement, it only spreads the damage (in the desktop's case,
/// onto a handler that recursively deletes). Every other resolver on both sides
/// of the boundary already reads a blank value as absent: `resolve_config_dir`
/// tests `!root.trim().is_empty()`, `routes::shell::home_dir` applies the same
/// rule to `HOME` (`.filter(|p| !p.as_os_str().is_empty())`), and `main.ts`'s
/// `expandBiorouterPath` has always had an `if (!pathRoot)` that an empty
/// string falls through. `ui/desktop/src/utils/biorouterPaths.ts` now states it
/// explicitly for the whole desktop main process, and its suite pins it.
///
/// ⚠ **The authoritative resolver is the one holdout, and this test does not
/// pretend otherwise.** The fix is one line in
/// `crates/biorouter/src/config/paths.rs` —
/// `std::env::var("BIOROUTER_PATH_ROOT").ok().filter(|root| !root.trim().is_empty())`
/// — which this test is written to survive: once it lands the two resolvers
/// agree outright and the second assertion's escape hatch stops being used.
/// Until then a blank root is a broken sandbox, but a broken sandbox that
/// writes to a relative path is not the same failure as one that writes to the
/// user's real tree, and only the second is a sandbox *escape*.
#[test]
#[serial]
fn a_blank_path_root_never_sends_one_resolver_into_a_tree_the_other_avoids() {
    for blank in ["", "   ", "\t"] {
        with_path_root(Some(blank), || {
            let shared = biorouter_mcp::paths::config_dir();
            assert!(
                shared.is_absolute(),
                "the shared resolver answered a blank root with a relative path ({shared:?}); \
                 a relative config dir names a different directory in every process that \
                 resolves it"
            );

            let authoritative = Paths::config_dir();
            assert!(
                authoritative == shared || !authoritative.is_absolute(),
                "a blank BIOROUTER_PATH_ROOT resolves to {authoritative:?} authoritatively but \
                 {shared:?} everywhere else — two different ABSOLUTE trees, which is the \
                 sandbox escape this seam exists to prevent"
            );
        });
    }
}

#[test]
#[serial]
fn agent_drafter_store_honours_the_sandbox_root() {
    with_path_root(Some("/tmp/br-path-agreement"), || {
        assert_eq!(
            biorouter_mcp::agent_drafter::default_root(),
            PathBuf::from("/tmp/br-path-agreement/config/agent_drafter"),
            "drafted apps must land inside the sandbox, not the user's global store"
        );
    });
}

#[test]
#[serial]
fn knowledge_store_honours_the_sandbox_root() {
    with_path_root(Some("/tmp/br-path-agreement"), || {
        assert_eq!(
            biorouter_mcp::knowledge::paths::knowledge_root().expect("knowledge root"),
            PathBuf::from("/tmp/br-path-agreement/config/knowledge"),
            "knowledge bases must be read from the sandbox, not the user's global store"
        );
    });
}

/// `with_path_root` must put the previous value back even when the closure
/// panics. An assertion failure inside any test above otherwise leaks this
/// file's scratch root into every other test running in the process — the exact
/// failure that made a full `cargo test` run red (a test resolved into a
/// `TempDir` that had already been dropped).
///
/// The panic below is caught, so the stderr backtrace it prints is expected.
#[test]
#[serial]
fn with_path_root_restores_the_previous_value_on_panic() {
    let before = std::env::var("BIOROUTER_PATH_ROOT").ok();

    let outcome = std::panic::catch_unwind(|| {
        with_path_root(Some("/tmp/br-path-agreement-leak"), || {
            panic!("simulated assertion failure inside the guarded region");
        })
    });

    assert!(outcome.is_err(), "the closure's panic must propagate");
    assert_eq!(
        std::env::var("BIOROUTER_PATH_ROOT").ok(),
        before,
        "BIOROUTER_PATH_ROOT leaked out of with_path_root when the test body panicked"
    );
}

/// The three resolvers must sit under one config root — not, as before, under
/// two different app-strategy tuples (the crate's own and a hand-rolled second one).
#[test]
#[serial]
fn all_mcp_stores_share_one_config_root() {
    with_path_root(None, || {
        let cfg = Paths::config_dir();
        assert!(biorouter_mcp::agent_drafter::default_root().starts_with(&cfg));
        assert!(biorouter_mcp::knowledge::paths::knowledge_root()
            .expect("knowledge root")
            .starts_with(&cfg));
    });
}

/// `Paths` and every `biorouter-mcp` resolver read one function, so they cannot
/// disagree about the platform directories, and there is no second
/// `AppStrategyArgs` tuple anywhere in either crate.
#[test]
#[serial]
fn paths_resolves_through_the_one_platform_function() {
    with_path_root(None, || {
        let dirs = biorouter_mcp::paths::platform_dirs().expect("a home dir");
        assert_eq!(Paths::config_dir(), dirs.config);
        assert_eq!(Paths::data_dir(), dirs.data);
        assert_eq!(
            Paths::state_dir(),
            dirs.state.clone().unwrap_or(dirs.data.clone())
        );
        assert_eq!(biorouter_mcp::paths::config_dir(), dirs.config);
    });

    let paths_source = include_str!("../src/config/paths.rs");
    assert!(
        !paths_source.contains("AppStrategyArgs") && !paths_source.contains("choose_app_strategy"),
        "Paths must not build an app strategy of its own; it reads biorouter_mcp::paths::platform_dirs"
    );
    let mcp_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../biorouter-mcp/src");
    let mut stack = vec![mcp_src.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)
            .expect("readable source dir")
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && path != mcp_src.join("paths.rs")
            {
                let code = std::fs::read_to_string(&path).unwrap();
                let calls = code
                    .lines()
                    .filter(|line| !line.trim_start().starts_with("//"))
                    .filter(|line| line.contains("choose_app_strategy("))
                    .count();
                assert_eq!(
                    calls,
                    0,
                    "{} resolves the app folder itself; use biorouter_mcp::paths::platform_dirs, \
                     which also applies the Windows move",
                    path.display()
                );
            }
        }
    }
}

/// The Windows layout `platform_dirs` builds by hand (so it can resolve the
/// moved folder) is exactly etcetera's Windows strategy for the one tuple, on a
/// machine where neither folder exists yet. Runs on every OS: etcetera's
/// Windows strategy reads `APPDATA` and `LOCALAPPDATA` wherever it runs.
#[test]
#[serial]
fn the_windows_layout_is_etceteras_windows_strategy() {
    use etcetera::AppStrategy;
    let roaming = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    let _env = env_lock::lock_env([
        (
            "APPDATA",
            Some(roaming.path().to_string_lossy().into_owned()),
        ),
        (
            "LOCALAPPDATA",
            Some(local.path().to_string_lossy().into_owned()),
        ),
    ]);
    let etcetera = etcetera::app_strategy::Windows::new(biorouter_mcp::APP_STRATEGY.clone())
        .expect("a home dir");
    let ours = biorouter_mcp::paths::windows_dirs(roaming.path(), local.path());
    assert_eq!(ours.config, etcetera.config_dir());
    assert_eq!(ours.data, etcetera.data_dir());
    assert_eq!(ours.cache, etcetera.cache_dir());
    assert_eq!(ours.state, etcetera.state_dir());
    assert_eq!(
        ours.config,
        roaming
            .path()
            .join(biorouter_mcp::app_folder::APP_AUTHOR)
            .join("biorouter")
            .join("config")
    );
}

/// The rewrite after a move names each store by a path of its own, because
/// `biorouter-mcp` cannot see the modules that own them. This pins every name
/// to its owner, so a store that moves inside the folder cannot silently drop
/// out of the rewrite.
#[test]
#[serial]
fn the_rewrite_covers_the_files_the_stores_really_use() {
    let root = tempfile::tempdir().unwrap();
    let root_str = root.path().to_string_lossy().into_owned();
    let covered: Vec<PathBuf> = biorouter_mcp::app_folder::rewrite::store_files(root.path())
        .into_iter()
        .map(|(path, _)| path)
        .collect();
    with_path_root(Some(&root_str), || {
        for owned in [
            Paths::config_dir().join(biorouter::config::base::CONFIG_YAML_NAME),
            biorouter::privacy::provenance::provenance_path(),
            biorouter::scheduler::get_default_scheduler_storage_path().unwrap(),
            biorouter_mcp::knowledge::registry::registry_path(
                &biorouter_mcp::knowledge::paths::knowledge_root().unwrap(),
            ),
            Paths::config_dir().join("daemon-profile.json"),
            Paths::config_dir().join("crew").join("connections.json"),
            Paths::config_dir().join("headless").join("settings.json"),
            Paths::in_data_dir("projects.json"),
        ] {
            assert!(
                covered.contains(&owned),
                "{} is a store that holds paths, and the rewrite does not cover it",
                owned.display()
            );
        }
        assert_eq!(
            biorouter_mcp::app_folder::rewrite::sessions_db(root.path()),
            Paths::data_dir()
                .join(biorouter::session::session_manager::SESSIONS_FOLDER)
                .join(biorouter::session::session_manager::DB_NAME)
        );
        // Directories whose every file is covered.
        for (directory, sample) in [
            (biorouter::extension_install::claim::claims_dir(), "00ff"),
            (
                biorouter::scheduler::get_default_scheduled_workflows_dir().unwrap(),
                "job.yaml",
            ),
            (
                biorouter::workflow::local_workflows::get_workflow_library_dir(true),
                "flow.yaml",
            ),
        ] {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join(sample), "{}").unwrap();
            let covered: Vec<PathBuf> =
                biorouter_mcp::app_folder::rewrite::store_files(root.path())
                    .into_iter()
                    .map(|(path, _)| path)
                    .collect();
            assert!(
                covered.contains(&directory.join(sample)),
                "{} is not covered by the rewrite",
                directory.join(sample).display()
            );
        }
    });

    // The owners that this crate cannot call spell the same names.
    for (source, spelling) in [
        (
            include_str!("../src/daemon_runtime.rs"),
            "config.join(\"daemon-profile.json\")",
        ),
        (
            include_str!("../src/crew/mod.rs"),
            "root.join(\"connections.json\")",
        ),
        (
            include_str!("../../biorouter-server/src/routes/shell.rs"),
            "Paths::config_dir().join(\"headless\").join(\"settings.json\")",
        ),
        (
            include_str!("../../biorouter-cli/src/project_tracker.rs"),
            "Paths::in_data_dir(\"projects.json\")",
        ),
    ] {
        assert!(
            source.contains(spelling),
            "the store's owner no longer spells `{spelling}`; update the rewrite's list"
        );
    }
}
