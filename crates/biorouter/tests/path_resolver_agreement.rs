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
/// two different app-strategy tuples (the crate's own vs a hand-rolled `io/biorouter/biorouter`).
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
