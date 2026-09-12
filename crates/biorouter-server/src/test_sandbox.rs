//! Points a test binary's Biorouter data/config/state dirs at a throwaway
//! directory, before the first line of test code runs — and pins the
//! process-global session store to it, so no later test can move it.
//!
//! Without this, `AppState::new()` — and every test that goes through it —
//! opens the **developer's real** `sessions.db`. Tests then create, rename and
//! delete rows in the same database that holds the user's actual chats. Nothing
//! has been lost so far only because the tests happen to clean up after
//! themselves; an interrupted or buggy test is one mistake away from touching
//! real data.
//!
//! The mechanism already exists — `biorouter::config::Paths::get_dir` honours
//! `BIOROUTER_PATH_ROOT` — but it is read through a `LazyLock`
//! (`session_manager::SHARED_STORE_ROOT`) and a `OnceCell`
//! (`AgentManager::instance`), so a test that sets it *from inside* a `#[test]`
//! fn only wins if it happens to be the first test to touch the singleton. That
//! is why it has to be a `#[ctor]`: constructors run before `main`, before the
//! harness spawns a thread, so the singletons cannot already be pinned to the
//! real database when the override lands.
//!
//! Isolation is therefore per test *binary*, which is exactly the granularity
//! the flake needed: `#[serial]` already serialises tests within one binary, but
//! `cargo test` runs several binaries in parallel against one file, and SQLite
//! answered that with `(code: 5) database is locked` about one run in five.
//!
//! An externally supplied `BIOROUTER_PATH_ROOT` is left alone, so a harness that
//! already sandboxes the process keeps its own root.
//!
//! ## Setting the variable is not enough — the store has to be PINNED
//!
//! Redirecting the variable closed the cross-process half and left a
//! within-process half open, which is the Windows CI flake that rotated through
//! the route tests (`activity_clamps_an_absurd_window` and friends answering 500
//! where the test wanted 200).
//!
//! `BIOROUTER_PATH_ROOT` is process-global, and a `#[test]` may legitimately
//! relocate it under a `TempDir` to get its own config/skills/knowledge root —
//! `routes::apps`'s `lock_env_for` and `routes::config_management`'s privacy
//! tests both do. Whichever test first reaches the session store decides where
//! the whole process's `sessions.db` lives, and the tests run in parallel, so
//! that test could be one holding such a lock. Its `TempDir` then drops, the
//! directory is unlinked, and the pool is pinned to a path that no longer
//! exists.
//!
//! What happens next looks nothing like its cause. The connection the pool
//! already opened keeps answering, because SQLite on POSIX does not care that
//! its inode lost its name — so a serial query still returns 200. It is the
//! first moment two tasks want the pool at once, forcing it to open a **second**
//! connection, that fails with `(code: 14) unable to open database file`, and
//! every route that touches the store turns that into a 500. The victim is
//! whichever test queried next, which is why the failing name rotates and why
//! the same test can pass in the `biorouter_server` lib binary and fail in the
//! `biorouterd` bin binary minutes apart.
//!
//! So the ctor also calls `SessionManager::shared_store_root()`, whose side
//! effect is to freeze that path here, at a root this module owns and only the
//! `#[dtor]` deletes. It resolves one environment variable into a `PathBuf` —
//! no pool, no disk, no async runtime — which is what makes it safe to run
//! before `main`. The ad-hoc warm-ups (`AppState::new()` before the env lock)
//! that four `routes::apps` tests carried for this are no longer needed, and
//! four out of ~25 relocating sites is why the flake outlived them.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Set only when *we* created the sandbox, so the destructor can never delete a
/// root that was handed to us from outside.
static OWNED: AtomicBool = AtomicBool::new(false);

/// Per-process sandbox root. Derived from the pid so the destructor can
/// recompute it without shared state, and so parallel test binaries never
/// collide.
fn sandbox_root() -> PathBuf {
    std::env::temp_dir().join(format!("biorouter-test-root-{}", std::process::id()))
}

#[ctor::ctor]
fn redirect_biorouter_dirs_into_a_sandbox() {
    if !std::env::var("BIOROUTER_PATH_ROOT").is_ok_and(|v| !v.trim().is_empty()) {
        let root = sandbox_root();
        if std::fs::create_dir_all(&root).is_ok() {
            OWNED.store(true, Ordering::SeqCst);
            std::env::set_var("BIOROUTER_PATH_ROOT", &root);
        }
    }
    // Freeze the session store's directory at whatever root is now in force,
    // before any test can relocate `BIOROUTER_PATH_ROOT` under a `TempDir` it
    // later unlinks. Unconditional: an externally supplied root needs the same
    // protection, and so does the fallback where the sandbox could not be
    // created — a stable directory, even the real one, beats a doomed one.
    let _ = biorouter::session::session_manager::SessionManager::shared_store_root();
}

#[ctor::dtor]
fn remove_the_sandbox() {
    if OWNED.load(Ordering::SeqCst) {
        let _ = std::fs::remove_dir_all(sandbox_root());
    }
}

#[cfg(test)]
mod tests {
    use biorouter::config::paths::Paths;
    use biorouter::session::session_manager::SessionManager;

    /// The point of the whole module: the session database a test opens must not
    /// be the developer's. `Paths::data_dir()` is what
    /// `SessionManager::instance()` resolves `sessions.db` under.
    ///
    /// ⚠ The home directory is read under **both** names, and the fallback is
    /// not defensive padding. `HOME` is a POSIX variable; Windows calls it
    /// `USERPROFILE` and does not define `HOME` outside a Git Bash session, so
    /// `var("HOME").expect("HOME")` panicked this test on any runner that
    /// happened not to have one. It is the only unguarded `HOME` read left in
    /// the tree: `security::session_store` and `security::global_memory` both
    /// already spell it `HOME` or `USERPROFILE`, and this now matches them.
    /// Falling back to an empty string keeps the assertion meaningful when
    /// neither is set, because `starts_with("")` is true for every path, so a
    /// missing home makes the check STRICTER rather than vacuous.
    #[test]
    #[serial_test::serial]
    fn the_session_database_is_not_the_developers() {
        // ⚠ `#[serial]` is the WRONG mutex for this assertion, and that cost a red
        // `main` (`055cb087`, `test (ubuntu-latest)`:
        // "expected the per-process sandbox, got /tmp/.tmpMnHK86/data").
        //
        // `Paths::data_dir()` re-reads `BIOROUTER_PATH_ROOT` on every call, and
        // this file is `#[path]`-included by
        // `tests/session_store_survives_a_relocated_path_root.rs`, whose own test
        // relocates that variable under **`env_lock`** — a different lock from
        // `serial_test`'s. So the two ran concurrently and this read the
        // relocator's `TempDir`. Latent since that binary landed; #280 changed how
        // many sessions the suite creates, which moved the schedule enough to open
        // the window. A scheduling-dependent assertion, not a
        // scheduling-dependent product.
        //
        // So take the WRITERS' lock — the rule `pinned_path_root` was corrected to
        // obey — and take it while **setting nothing**. ⚠ Pinning the variable to
        // `sandbox_root()` here would serialise correctly and make the assertion
        // VACUOUS: `Paths::data_dir()` reads the same variable, so it would be
        // asserting the value it had just written. An empty set acquires the mutex
        // and leaves the ctor's answer in place, which is what is under test.
        // `sandbox_root()` is derived from `temp_dir()` and the pid, never from the
        // environment, so comparing against it is not circular either.
        let _env = env_lock::lock_env(Vec::<(&str, Option<&str>)>::new());
        let data_dir = Paths::data_dir();
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_default();
        assert!(
            !data_dir.starts_with(&home) || data_dir.starts_with(std::env::temp_dir()),
            "tests resolved the real data dir {}",
            data_dir.display()
        );
        assert!(
            data_dir.starts_with(super::sandbox_root()),
            "expected the per-process sandbox, got {}",
            data_dir.display()
        );
    }

    /// Asserting the *pinned* root rather than the environment variable is what
    /// makes this meaningful, and it is a strictly stronger claim than the test
    /// above. `Paths::data_dir()` re-reads the environment on every call, so it
    /// answers correctly even in a binary where some earlier test had already
    /// pinned the store somewhere else. `shared_store_root()` is the value a
    /// query actually follows, frozen for the life of the process.
    ///
    /// It fails if a future edit drops the pin from the `#[ctor]`, or if
    /// anything in this crate reaches the session store before `main` under a
    /// different root.
    #[test]
    #[serial_test::serial]
    fn the_session_store_is_pinned_inside_the_sandbox() {
        let pinned = SessionManager::shared_store_root();
        assert!(
            pinned.starts_with(super::sandbox_root()),
            "the process session store is pinned at {}, outside the sandbox at {}. \
             Something resolved it before the ctor did, so a test that relocates \
             BIOROUTER_PATH_ROOT can move this binary's sessions.db into a TempDir \
             it then deletes.",
            pinned.display(),
            super::sandbox_root().display()
        );
    }
}
