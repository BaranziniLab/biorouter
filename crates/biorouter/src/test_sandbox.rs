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

/// This binary's sandbox root, recorded before any test could move it.
///
/// ⚠ **`std::env::var("BIOROUTER_PATH_ROOT")` is NOT a way to ask what the
/// sandbox root is, at any point after `main` starts.** Around thirty tests in
/// this binary legitimately point that variable at a `TempDir` of their own
/// under `env_lock` and put it back (`logging`, `managed`, `providers::utils`,
/// `session::diagnostics`, `agents::skills_extension`, `agents::agent`,
/// `execution::manager`, `knowledge::conversation_ingest`, …), so a read taken
/// at test time answers "whichever test is relocating it at this instant",
/// which is a different directory on every run and is deleted moments later.
/// This cell is the only stable answer, and it is why it exists.
static SANDBOX_ROOT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

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
///
/// ⚠ …and freezing one singleton is only half of THAT. A test that resolves a
/// path itself, rather than through a frozen cell, still needs to know which
/// root is its own — so whichever value ends up in effect is also recorded in
/// [`SANDBOX_ROOT`] here. This is the one moment in the process's life at which
/// reading the variable answers that question, because no test has run yet.
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
    // Both branches, and after the set: an outer root is recorded as-is, and one
    // we minted is recorded as the value we just installed.
    if let Some(root) = std::env::var_os("BIOROUTER_PATH_ROOT") {
        record_sandbox_root(&root);
    }
    let _ = crate::session::session_manager::SessionManager::shared_store_root();
}

fn record_sandbox_root(root: &std::ffi::OsStr) {
    // A root that is not valid UTF-8 cannot be pinned through `env_lock`
    // (its API is `&str`), so it is left unrecorded rather than recorded
    // lossily — a mangled root would be pinned as a *different* directory.
    // `sandbox_path_root` then says so instead of silently relocating writes.
    if let Some(root) = root.to_str() {
        let _ = SANDBOX_ROOT.set(root.to_owned());
    }
}

/// The config/data root every test in this binary resolves under, unless a test
/// is deliberately relocating it.
pub(crate) fn sandbox_path_root() -> &'static str {
    SANDBOX_ROOT
        .get()
        .expect(
            "the sandbox root was recorded before main; an absent value means either the ctor \
             did not run or BIOROUTER_PATH_ROOT is not valid UTF-8 and cannot be pinned",
        )
        .as_str()
}

/// Hold `BIOROUTER_PATH_ROOT` **at the sandbox root** for as long as the guard
/// lives, so a test whose subject resolves paths from that variable resolves
/// the same root every time it asks.
///
/// ⚠ **Pin the recorded root, never the variable's current value.** Two call
/// sites used to open with
///
/// ```ignore
/// let current = std::env::var("BIOROUTER_PATH_ROOT").ok();
/// env_lock::lock_env([("BIOROUTER_PATH_ROOT", current.as_deref())])
/// ```
///
/// whose stated intent — "hold the lock, do not change the root" — is exactly
/// right and which does the opposite whenever it matters. The read happens
/// *before* the lock is acquired, so if any of the ~30 relocating tests holds
/// the lock at that instant, `current` is **that test's `TempDir`**. The pin
/// then blocks, the relocator finishes, its guard restores the sandbox root and
/// its `TempDir` is deleted — and the pin wakes up and installs the deleted
/// directory as this test's root for the whole test. Everything the test then
/// resolves (`Config::global()`, `extensions_root()`, the global memory store)
/// points outside its own sandbox at a path whose parent may be gone, which is
/// how `a_removal_prunes_the_extension_from_every_stored_session_roster` came
/// to fail a full-suite run inside `create_dir_all` with `EINVAL` while passing
/// 3/3 in isolation. A writer's lock cannot protect an unlocked reader — this
/// helper's job is to have no unlocked read to protect.
pub(crate) fn pin_sandbox_path_root() -> env_lock::EnvGuard<'static> {
    env_lock::lock_env([("BIOROUTER_PATH_ROOT", Some(sandbox_path_root()))])
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
        // The recorded root, not `std::env::var` — a read taken here answers
        // "whichever test is relocating the variable right now", and this
        // assertion would then blame the sandbox for a sibling's `TempDir`.
        let root = super::sandbox_path_root();
        let path = crate::config::Config::global().path();
        assert!(
            path.starts_with(root),
            "Config::global() resolved to {path}, outside the sandbox at {root}. \
             Something reached Config::global() before the sandbox was installed, so \
             config writes from this binary land in the developer's real config."
        );
    }

    /// The pin must ignore the live variable, and this is the situation in which
    /// they differ — the one that made `pinned_path_root` install another test's
    /// soon-to-be-deleted `TempDir`.
    ///
    /// Staged with a thread that holds `env_lock` (so it is holding a foreign
    /// root installed) and is released by a channel, not a sleep: every step is
    /// an event, so a starved runner makes this slower and never red. Nothing
    /// else in the binary can write the environment while that lock is held,
    /// which is also what makes the bare `std::env::var` read below safe here.
    ///
    /// Before the fix, the pin's source was exactly that `std::env::var` read,
    /// so the assertion below could not hold.
    #[test]
    fn the_pin_source_ignores_a_root_another_test_has_installed() {
        let sandbox = super::sandbox_path_root();
        let foreign = tempfile::TempDir::new().expect("a foreign root to install");
        let foreign_value = foreign
            .path()
            .to_str()
            .expect("a UTF-8 temp path")
            .to_owned();

        let (installed_tx, installed_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let held = foreign_value.clone();
        let holder = std::thread::spawn(move || {
            let _guard = env_lock::lock_env([("BIOROUTER_PATH_ROOT", Some(held.as_str()))]);
            installed_tx.send(()).expect("the test is still waiting");
            // Hold the foreign root — and the environment lock — until released.
            let _ = release_rx.recv();
        });
        installed_rx.recv().expect("the holder installs its root");

        let live = std::env::var("BIOROUTER_PATH_ROOT").ok();
        let pinned = super::sandbox_path_root();
        release_tx.send(()).expect("the holder is still parked");
        holder.join().expect("the holder exits cleanly");

        assert_eq!(
            live.as_deref(),
            Some(foreign_value.as_str()),
            "precondition: the holder's root really is the live value right now, so a pin \
             that reads the environment would install a directory it does not own"
        );
        assert_eq!(
            pinned, sandbox,
            "the pin followed another test's root instead of this binary's sandbox"
        );
    }

    /// Nothing in this crate may ask the environment where the sandbox root is.
    ///
    /// This is the shared state itself rather than either symptom: both flakes
    /// this guard comes from — a pin that installed another test's `TempDir`,
    /// and a sandbox assertion that compared a frozen path against a sibling's
    /// root — were an unlocked read of a variable ~30 tests in this binary
    /// relocate. Adding a lock to the writers cannot close that; not reading
    /// can. The resolver is the one place that must read it, because production
    /// honours the relocation, and this module is where the answer is recorded.
    #[test]
    fn only_the_resolver_and_the_sandbox_read_the_path_root_variable() {
        let crate_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        assert!(
            crate_src.is_dir(),
            "the audit walks {}; if that path is wrong it passes for the wrong reason",
            crate_src.display()
        );
        let needle = concat!("BIOROUTER_", "PATH_ROOT");
        let allowed = ["config/paths.rs", "test_sandbox.rs"];

        let mut offenders: Vec<String> = Vec::new();
        let mut scanned = 0usize;
        for entry in walkdir::WalkDir::new(&crate_src) {
            let entry = entry.expect("the audit must not silently skip an unreadable directory");
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let rel = path
                .strip_prefix(&crate_src)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            scanned += 1;
            if allowed.contains(&rel.as_str()) {
                continue;
            }
            let body = std::fs::read_to_string(path).expect("a readable source file");
            for (number, line) in body.lines().enumerate() {
                let code = line.trim_start();
                // Prose about the variable is fine; a read of it is not.
                if code.starts_with("//") {
                    continue;
                }
                if code.contains(needle) && (code.contains("env::var") || code.contains("var_os")) {
                    offenders.push(format!("{rel}:{}", number + 1));
                }
            }
        }
        assert!(
            scanned > 100,
            "only {scanned} files scanned — the walk found nothing to audit"
        );
        assert!(
            offenders.is_empty(),
            "these read BIOROUTER_PATH_ROOT from the environment: {offenders:?}. After main \
             starts, that answers 'whichever test is relocating it at this instant', not 'where \
             is the sandbox'. Ask `test_sandbox::sandbox_path_root()`, or take \
             `test_sandbox::pin_sandbox_path_root()` if the subject resolves paths itself. \
             Setting it is still fine under `env_lock`."
        );
    }

    /// …and the guard the helper hands out really does install that root, so a
    /// subject resolving `BIOROUTER_PATH_ROOT` under it lands in the sandbox.
    #[test]
    fn the_pin_installs_the_sandbox_root_for_its_lifetime() {
        let _pin = super::pin_sandbox_path_root();
        assert_eq!(
            std::env::var("BIOROUTER_PATH_ROOT").as_deref(),
            Ok(super::sandbox_path_root())
        );
        assert!(crate::config::paths::Paths::config_dir().starts_with(super::sandbox_path_root()));
        assert!(crate::extension_install::brxt::extensions_root()
            .starts_with(super::sandbox_path_root()));
    }

    /// The same claim for the session store, and it needs its own test because
    /// the two singletons are frozen by different calls.
    ///
    /// `Paths::data_dir()` re-reads the environment on every call, so it answers
    /// correctly even in a binary whose store was already pinned somewhere else.
    /// `shared_store_root()` is the frozen value, and it is the one that decides
    /// whether this binary's `sessions.db` can end up inside a `TempDir` a test
    /// deletes underneath it.
    ///
    /// ⚠ The expected root is the **recorded** one, not `std::env::var`. It
    /// arrived from #282 reading the variable live, which is the hazard the rest
    /// of this module exists to close: a sibling relocating the root while this
    /// test runs would make it compare a frozen store path against that
    /// sibling's `TempDir` and fail, blaming the store for a harness race.
    #[test]
    fn the_lib_test_binary_session_store_is_sandboxed() {
        let root = super::sandbox_path_root();
        let pinned = crate::session::session_manager::SessionManager::shared_store_root();
        assert!(
            pinned.starts_with(root),
            "the process session store is pinned at {}, outside the sandbox at {root}. \
             Something resolved it before the ctor did, so a test that relocates \
             BIOROUTER_PATH_ROOT can move this binary's sessions.db into a TempDir it \
             then deletes.",
            pinned.display()
        );
    }
}
