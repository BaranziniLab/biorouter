//! Sandbox the `biorouter` **lib** test binary's config/data root before any
//! test runs.
//!
//! The integration binary `tests/agent.rs` has had this since issue #54; the
//! lib binary never did, and BR-71 made that gap load-bearing. Several tests
//! here reach `AgentManager::instance()` — `workspace_list`'s handler (Tasks
//! 12/14/15/17) and, since Task 33, `run_complete_subagent_task`'s child
//! registration. `instance()` resolves `Paths::data_dir()` and
//! `SessionManager::instance()`, and its first initialization runs
//! `run_first_run_init`, which seeds the built-in skills and installs the Soul
//! KB plus a 3 AM schedule. Unsandboxed, a plain `cargo test -p biorouter --lib`
//! therefore writes into the developer's real `~/.config/biorouter`.
//!
//! Documenting "run it under `BIOROUTER_PATH_ROOT`" is not a fix: the damage
//! happens on the DEFAULT invocation, which is what a developer types and what
//! an editor's test runner emits. This makes the sandbox the default and lets
//! the gate keep choosing its own root.

/// Run before `main`, because the placement is the whole point.
///
/// `Config::global()`, `SessionManager::instance()` and `AGENT_MANAGER` are all
/// one-shot cells that resolve their path the first time anything touches them,
/// and the tests run in parallel — so a guard installed inside whichever test
/// module "owns" the hazard fixes nothing whenever another test got there
/// first. Running before any test is the only placement that cannot lose that
/// race (the same reasoning as `tests/agent.rs`).
///
/// An outer `BIOROUTER_PATH_ROOT` wins: the Task 33 gate exports its own
/// `mktemp -d` root, and a harness that wants to inspect what a run wrote must
/// be able to choose where it lands.
///
/// ⚠ Setting the variable is only half of it. `BIOROUTER_PATH_ROOT` is
/// process-global and dozens of tests here relocate it under a `TempDir` of
/// their own (`env_lock::lock_env`), so whichever test first reaches the session
/// store still decides where this binary's `sessions.db` lives — and if that was
/// a test holding such a lock, the store is pinned inside a directory that is
/// unlinked moments later. The already-open connection keeps answering, so it
/// stays invisible until two tasks want the pool at once and it has to open a
/// second connection: `(code: 14) unable to open database file`, surfacing as
/// whatever the *next* test was asserting. So the ctor also freezes the store's
/// root here, which costs one environment read and a `PathBuf` — no pool, no
/// disk, no runtime.
#[ctor::ctor]
fn sandbox_config_root_for_the_lib_test_binary() {
    if std::env::var_os("BIOROUTER_PATH_ROOT").is_none() {
        let root = tempfile::TempDir::new().expect("scratch config root for the lib test binary");
        std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
        // Leaked deliberately: a `static` is never dropped, which is exactly the
        // lifetime the sandbox needs — it must outlive the last test in the binary.
        static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        let _ = ROOT.set(root);
    }
    let _ = crate::session::session_manager::SessionManager::shared_store_root();
}

#[cfg(test)]
mod tests {
    /// Nothing in this binary may resolve to the developer's live configuration.
    ///
    /// Asserting on `Config::global()` rather than on the environment is what
    /// makes this meaningful: it is the resolved path a write actually follows,
    /// and it is frozen at first use. A test that only checked the env var would
    /// still pass if something had reached `Config::global()` before the ctor.
    #[test]
    fn the_lib_test_binary_config_root_is_sandboxed() {
        let root = std::env::var("BIOROUTER_PATH_ROOT")
            .expect("BIOROUTER_PATH_ROOT must be sandboxed before any test in this binary runs");
        let path = crate::config::Config::global().path();
        assert!(
            path.starts_with(&root),
            "Config::global() resolved to {path}, outside the sandbox at {root}. \
             Something reached Config::global() before the sandbox was installed, so \
             config writes from this binary land in the developer's real config."
        );
    }

    /// The same claim for the session store, and it needs its own test because
    /// the two singletons are frozen by different calls.
    ///
    /// `Paths::data_dir()` re-reads the environment on every call, so it answers
    /// correctly even in a binary whose store was already pinned somewhere else.
    /// `shared_store_root()` is the frozen value, and it is the one that decides
    /// whether this binary's `sessions.db` can end up inside a `TempDir` a test
    /// deletes underneath it.
    #[test]
    fn the_lib_test_binary_session_store_is_sandboxed() {
        let root = std::env::var("BIOROUTER_PATH_ROOT")
            .expect("BIOROUTER_PATH_ROOT must be sandboxed before any test in this binary runs");
        let pinned = crate::session::session_manager::SessionManager::shared_store_root();
        assert!(
            pinned.starts_with(&root),
            "the process session store is pinned at {}, outside the sandbox at {root}. \
             Something resolved it before the ctor did, so a test that relocates \
             BIOROUTER_PATH_ROOT can move this binary's sessions.db into a TempDir it \
             then deletes.",
            pinned.display()
        );
    }
}
