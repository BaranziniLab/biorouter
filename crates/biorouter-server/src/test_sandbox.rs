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
//! `BIOROUTER_PATH_ROOT` — but it is read through one-shot cells
//! (`session_manager::SHARED_STORE_ROOT`, `execution::manager::
//! SHARED_CONFIG_ROOT`, and the `AgentManager::instance` `OnceCell` built from
//! them), so a test that sets it *from inside* a `#[test]` fn only wins if it
//! happens to be the first test to touch the singleton. That is why it has to
//! be a `#[ctor]`: constructors run before `main`, before the harness spawns a
//! thread, so the singletons cannot already be pinned to the real database
//! when the override lands.
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
//! tests both do — since 2026-09-22 only in a process of their own
//! ([`in_a_process_of_its_own`]), where the freeze below still decides where
//! that child's cells live. Whichever test first reaches the session store
//! decides where the whole process's `sessions.db` lives, and the tests run in
//! parallel, so that test could be one holding such a lock. Its `TempDir` then
//! drops, the directory is unlinked, and the pool is pinned to a path that no
//! longer exists.
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
//!
//! ## …and the global `AgentManager`'s config root, for the same reason
//!
//! `AppState::new()` calls `AgentManager::instance()`, whose first call seeds
//! the Soul KB and the built-in skills into `AgentManager::shared_config_root()`
//! — one `Paths::config_dir()`, taken at that first ask. Tests here reach it
//! while holding a relocated root: `routes::apps`'s `declared_but_unarmed`
//! tests take `lock_env_for(root, …)` and then build an `AppState`. Measured
//! 2026-09-21 without the freeze below, running
//! `declared_but_unarmed::a_compute_block_with_the_default_sandbox_is_reported_not_swallowed`
//! before this module's guard (`--test-threads=1`): the guard reported the
//! global manager's config root pinned inside that test's `TempDir`, i.e. the
//! process's Soul KB and skills were seeded into a directory deleted when the
//! test ended. So the ctor freezes it too; it costs the same one environment
//! read and `PathBuf`.
//!
//! ## …and `Config::global()`, which every config write in the binary follows
//!
//! It is frozen here too, as the `biorouter` lib and `biorouter-cli` ctors
//! freeze it; until 2026-09-21 this crate's did not, so whichever test first
//! touched it decided where the binary's `config.yaml` lived. Measured that
//! day with the freeze line deleted, running each of the 17 tests in
//! `routes::apps`' `privacy_capability`, `skills_grant_scope` and
//! `declared_but_unarmed` modules, `config_management`'s
//! `privacy_disclosure_tests` and `routes::skills` alone before
//! `the_config_file_is_pinned_inside_the_sandbox` (`--test-threads=1`): three
//! left `Config::global()` resolved inside their own `TempDir` —
//! `declared_but_unarmed::an_armable_compute_block_is_not_reported_as_withheld`,
//! `privacy_capability::the_app_capability_report_follows_the_manifests_provider_not_the_global_one`
//! and `skills_grant_scope::an_apps_skill_grant_is_bounded_to_search_and_load`
//! — and the guard failed naming that path. (`declared_but_unarmed::
//! a_compute_block_with_the_default_sandbox_is_reported_not_swallowed`, which
//! does pin the manager's root above, does not reach this cell.) With the
//! line, all three orders pass. `Config::default()` is one
//! environment read, two `join`s, a read of `BIOROUTER_DISABLE_KEYRING` and
//! some `Mutex::new`s — no file, no keyring (the `biorouter` lib ctor's
//! comment lists them) — which is what makes it safe before `main`. What it
//! does freeze is the secret backend, read now from the outer environment
//! rather than by whichever test got there first.

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
    // `Paths`' own predicate, so a root honoured here is one every cell resolves.
    if biorouter::config::paths::Paths::path_root_override().is_none() {
        let root = sandbox_root();
        // ⚠ No fallback. This used to carry on when the directory could not be
        // made, on the theory that "a stable directory, even the real one,
        // beats a doomed one" — which froze every cell below, `Config::global()`
        // included, at the developer's real `~/.config/biorouter`, and ran
        // every test against it.
        if let Err(error) = std::fs::create_dir_all(&root) {
            eprintln!(
                "biorouter-server test binary: could not create the sandbox root {}: {error}. \
                 Refusing to run: without it every test would read and write the \
                 developer's real config and data directories. Fix the temp directory, or \
                 export BIOROUTER_PATH_ROOT to a directory to use.",
                root.display()
            );
            // Not a panic: one cannot unwind out of a ctor, and the message is the point.
            std::process::abort();
        }
        OWNED.store(true, Ordering::SeqCst);
        std::env::set_var("BIOROUTER_PATH_ROOT", &root);
    }
    // Freeze the session store's directory at whatever root is now in force,
    // before any test can relocate `BIOROUTER_PATH_ROOT` under a `TempDir` it
    // later unlinks. Unconditional: an externally supplied root needs the same
    // protection.
    let _ = biorouter::session::session_manager::SessionManager::shared_store_root();
    // …and the global manager's config root, for the same reason (module docs).
    let _ = biorouter::execution::manager::AgentManager::shared_config_root();
    // …and `Config::global()` (module docs). Its secret backend is decided by
    // this same read of the environment, so it is recorded beside the freeze.
    let secrets_from_a_file = std::env::var("BIOROUTER_DISABLE_KEYRING").is_ok();
    let _ = biorouter::config::Config::global();

    // Observed, not assumed, after every freeze: each read is the cell's
    // non-initializing `get`, so an entry is `Some` only if something above
    // really resolved it before `main`. See [`ResolvedBeforeMain`].
    let _ = RESOLVED_BEFORE_MAIN.set(ResolvedBeforeMain {
        session_store_root:
            biorouter::session::session_manager::SessionManager::shared_store_root_if_resolved(),
        agent_manager_config_root:
            biorouter::execution::manager::AgentManager::shared_config_root_if_resolved(),
        global_config: biorouter::config::Config::global_if_initialized(),
        secrets_from_a_file,
    });
}

/// What each cell the ctor freezes held when the ctor returned, read WITHOUT
/// resolving it; `None` means nothing had resolved it before `main`.
///
/// ⚠ This is the half of each guard below that can notice a deleted freeze.
/// `cell().starts_with(sandbox_root())` alone cannot: with the freeze deleted,
/// the guard's own call is the first touch, lands while the ctor's sandbox is
/// still the ambient root, and passes. (Measured in the `biorouter` lib binary,
/// whose guards were that shape: both freeze lines deleted, every guard green.)
/// It could only catch a cell resolved too early with a non-sandbox value —
/// not the one that matters, an unfrozen cell first touched by a test holding
/// `BIOROUTER_PATH_ROOT` on a `TempDir` it then deletes, which depends on
/// scheduling and so is a coin flip for any guard that waits for it.
struct ResolvedBeforeMain {
    session_store_root: Option<&'static std::path::Path>,
    agent_manager_config_root: Option<&'static std::path::Path>,
    global_config: Option<&'static biorouter::config::Config>,
    /// `Config::default()`'s own rule — `BIOROUTER_DISABLE_KEYRING` set to
    /// anything — applied to the environment the ctor froze `Config::global()`
    /// in: `true` means `secrets.yaml`, `false` the OS keychain.
    secrets_from_a_file: bool,
}

static RESOLVED_BEFORE_MAIN: std::sync::OnceLock<ResolvedBeforeMain> = std::sync::OnceLock::new();

#[cfg(test)]
fn resolved_before_main() -> &'static ResolvedBeforeMain {
    RESOLVED_BEFORE_MAIN
        .get()
        .expect("the ctor records what it froze before main; an absent record means it did not run")
}

/// Whether this binary's `Config::global()` reads secrets from `secrets.yaml`
/// rather than the OS keychain.
///
/// The backend is chosen when the config is built, and the ctor builds it
/// before `main`, so a test that sets `BIOROUTER_DISABLE_KEYRING` itself is
/// too late to change it. Ask this instead, before anything reads a secret.
#[cfg(test)]
#[allow(dead_code)] // only the binaries that build real providers ask
pub(crate) fn global_config_reads_secrets_from_a_file() -> bool {
    resolved_before_main().secrets_from_a_file
}

/// Names, in a child this binary started, the one test that child runs. See
/// [`in_a_process_of_its_own`].
#[cfg(test)]
#[allow(dead_code)] // `#[path]`-included by integration binaries that relocate nothing
const OWN_PROCESS_TEST: &str = "BIOROUTER_TEST_IN_A_PROCESS_OF_ITS_OWN";

/// **No test in this binary moves `BIOROUTER_PATH_ROOT` or the home directory
/// while other tests are running.** A test that has to — because its subject
/// resolves `Paths` or the home directory itself and cannot be handed a root —
/// calls this first:
///
/// ```ignore
/// if !crate::test_sandbox::in_a_process_of_its_own() {
///     return;
/// }
/// let _env = crate::test_sandbox::relocate_path_root(temp.path());
/// ```
///
/// In the test run it re-executes this binary with `--exact <this test>` under
/// a sandbox root of its own, waits, and returns `false` once that child has
/// passed (a failure panics here, carrying the child's output). In the child it
/// returns `true` and the body runs — alone in its process, so the relocation
/// it makes is seen by nothing else.
///
/// ⚠ Why this and not the env lock. Tests here read the ambient root without
/// taking `env_lock` — every `AppState::new()` resolves its knowledge root
/// from it (`KnowledgeService::new_default()`), and the routes resolve skill
/// and config paths on every call — so a lock orders nothing for them: while a
/// sibling held the variable on its `TempDir` they resolved THAT directory, and
/// then it was deleted.
/// `routes::session_reach::bypass_tests::the_knowledge_base_gate_fires_under_the_served_router_tree`
/// failed so, once in 15 whole-binary runs, with `git init …/T/.tmp…/config/
/// knowledge/.creating-…`; forced (a relocation held across its
/// `AppState::new()`, the deletion overlapping `create_base`), 20 of 20.
///
/// A copy of the `biorouter` lib test binary's helper of the same name, which
/// is `cfg(test)` in that crate and so cannot be shared without shipping it.
/// The child is checked for `1 passed`, not just a zero exit: a test name that
/// matched nothing would exit 0 having run nothing. It is spawned under
/// `env_lock` and waited for outside it, so it cannot copy a value a sibling
/// set only for itself (see the `biorouter` helper).
#[cfg(test)]
#[allow(dead_code)] // `#[path]`-included by integration binaries that relocate nothing
pub(crate) fn in_a_process_of_its_own() -> bool {
    let current = std::thread::current();
    let test = current
        .name()
        .expect("libtest names each test's thread after the test");
    match std::env::var(OWN_PROCESS_TEST) {
        Ok(marked) if marked == test => return true,
        Ok(marked) => panic!(
            "this process was started to run `{marked}` alone, but `{test}` asked for a \
             process of its own from inside it"
        ),
        Err(_) => {}
    }
    let root = tempfile::TempDir::new().expect("a sandbox root for the child process");
    let mut command =
        std::process::Command::new(std::env::current_exe().expect("the path of this test binary"));
    command
        .args(["--exact", test, "--test-threads=1"])
        .env(OWN_PROCESS_TEST, test)
        .env("BIOROUTER_PATH_ROOT", root.path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let child = {
        // Setting nothing: the lock alone keeps a sibling's temporary value out
        // of the environment the child copies.
        let _env = env_lock::lock_env(Vec::<(&str, Option<&str>)>::new());
        command
            .spawn()
            .expect("start this test binary again for one test")
    };
    let output = child
        .wait_with_output()
        .expect("collect the child test process's output");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains("test result: ok. 1 passed"),
        "`{test}`, run in a process of its own, did not pass ({}):\n--- stdout\n{stdout}\n--- stderr\n{stderr}",
        output.status
    );
    false
}

/// Hold `BIOROUTER_PATH_ROOT` at `root` for as long as the guard lives — only
/// inside a process [`in_a_process_of_its_own`] started; it panics anywhere
/// else.
#[cfg(test)]
#[allow(dead_code)] // `#[path]`-included by integration binaries that relocate nothing
pub(crate) fn relocate_path_root(root: &std::path::Path) -> env_lock::EnvGuard<'static> {
    relocate_path_root_and(root, [])
}

/// [`relocate_path_root`], setting (or, with `None`, removing) `also` under the
/// same lock.
#[cfg(test)]
#[allow(dead_code)] // `#[path]`-included by integration binaries that relocate nothing
pub(crate) fn relocate_path_root_and(
    root: &std::path::Path,
    also: impl IntoIterator<Item = (&'static str, Option<String>)>,
) -> env_lock::EnvGuard<'static> {
    let mut vars = vec![("BIOROUTER_PATH_ROOT", Some(utf8(root)))];
    vars.extend(also);
    relocate(vars)
}

/// [`relocate_path_root_and`] plus the home directory, under BOTH of its names
/// — `dirs::home_dir()` reads `HOME` on unix and `USERPROFILE` on Windows (see
/// `routes::apps`' `lock_env_for_home`).
#[cfg(test)]
#[allow(dead_code)] // `#[path]`-included by integration binaries that relocate nothing
pub(crate) fn relocate_path_root_and_home(
    root: &std::path::Path,
    home: &std::path::Path,
    also: impl IntoIterator<Item = (&'static str, Option<String>)>,
) -> env_lock::EnvGuard<'static> {
    let home = utf8(home);
    let mut vars = vec![("HOME", Some(home.clone())), ("USERPROFILE", Some(home))];
    vars.extend(also);
    relocate_path_root_and(root, vars)
}

#[cfg(test)]
#[allow(dead_code)] // `#[path]`-included by integration binaries that relocate nothing
fn relocate(vars: Vec<(&'static str, Option<String>)>) -> env_lock::EnvGuard<'static> {
    assert!(
        std::env::var_os(OWN_PROCESS_TEST).is_some(),
        "a test moved BIOROUTER_PATH_ROOT or the home directory in the shared test process. \
         Every test that resolves them without the env lock — every AppState::new() — \
         would follow it into a directory it does not own; start the test with \
         `if !crate::test_sandbox::in_a_process_of_its_own() {{ return; }}`"
    );
    env_lock::lock_env(vars)
}

/// A relocated root must name the directory given, not a lossy rendering of it.
#[cfg(test)]
#[allow(dead_code)] // `#[path]`-included by integration binaries that relocate nothing
fn utf8(path: &std::path::Path) -> String {
    path.to_str().expect("a UTF-8 temp path").to_owned()
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
    use biorouter::config::Config;
    use biorouter::execution::manager::AgentManager;
    use biorouter::session::session_manager::SessionManager;

    /// The same two claims for `Config::global()` — the `config.yaml` every
    /// config write in this binary follows.
    ///
    /// Asserted on `Config::global().path()`, never `Paths::config_dir()`,
    /// which re-reads the environment on every call and would answer
    /// "sandboxed" in a binary whose `GLOBAL_CONFIG` was frozen elsewhere.
    #[test]
    fn the_config_file_is_pinned_inside_the_sandbox() {
        let path = Config::global().path();
        let root = super::sandbox_root();
        assert!(
            std::path::Path::new(&path).starts_with(&root),
            "Config::global() resolved to {path}, outside the sandbox at {}. Either \
             something reached Config::global() before the ctor installed the sandbox, \
             or nothing froze it before main and a test holding BIOROUTER_PATH_ROOT on \
             a TempDir of its own built an AppState first — and every config write from \
             this binary now follows that path.",
            root.display()
        );
        assert!(
            super::resolved_before_main().global_config.is_some(),
            "Config::global() was not initialized before main: the ctor's freeze is \
             missing. The first test to touch it now decides where this binary's \
             config.yaml lives, and a test holding BIOROUTER_PATH_ROOT on a TempDir of \
             its own pins every later config write inside a directory that is deleted \
             when it ends. (This call resolved {path} only because the ctor's sandbox \
             happened to be the ambient root when it ran.)"
        );
    }

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
    /// The first assertion fails if anything reaches the session store under a
    /// different root before this runs. The second is the one that fails if a
    /// future edit drops the pin from the `#[ctor]` — the first cannot see that,
    /// because its own call then resolves the sandbox (see
    /// [`super::ResolvedBeforeMain`]).
    #[test]
    #[serial_test::serial]
    fn the_session_store_is_pinned_inside_the_sandbox() {
        let pinned = SessionManager::shared_store_root();
        assert!(
            pinned.starts_with(super::sandbox_root()),
            "the process session store is pinned at {}, outside the sandbox at {}. \
             Either something resolved it before the ctor installed the sandbox, or \
             nothing froze it before main and a test holding BIOROUTER_PATH_ROOT on a \
             TempDir of its own reached it first — so this binary's sessions.db lives \
             in a directory that test deletes.",
            pinned.display(),
            super::sandbox_root().display()
        );
        assert!(
            super::resolved_before_main().session_store_root.is_some(),
            "the process session store's root was not resolved before main: the ctor's \
             freeze is missing. The first test to reach SessionManager::instance() now \
             decides where this binary's sessions.db lives; if it holds \
             BIOROUTER_PATH_ROOT on a TempDir it deletes, the next query that needs a \
             second connection fails with (code: 14). (This call resolved {} only \
             because the ctor's sandbox happened to be the ambient root when it ran.)",
            pinned.display()
        );
    }

    /// The same two claims for the global `AgentManager`'s config root — where
    /// `AppState::new()`'s first `AgentManager::instance()` seeds the Soul KB
    /// and the built-in skills.
    #[test]
    fn the_global_agent_managers_config_root_is_pinned_inside_the_sandbox() {
        let pinned = AgentManager::shared_config_root();
        assert!(
            pinned.starts_with(super::sandbox_root()),
            "the global AgentManager's config root is pinned at {}, outside the sandbox \
             at {}. Either something resolved it before the ctor installed the sandbox, \
             or nothing froze it before main and a test holding BIOROUTER_PATH_ROOT on a \
             TempDir of its own built an AppState first — so the process's Soul KB and \
             built-in skills were seeded into that test's directory.",
            pinned.display(),
            super::sandbox_root().display()
        );
        assert!(
            super::resolved_before_main()
                .agent_manager_config_root
                .is_some(),
            "the global AgentManager's config root was not resolved before main: the \
             ctor's freeze is missing. The first AppState::new() now decides where the \
             Soul KB and built-in skills are seeded, and if a test is holding \
             BIOROUTER_PATH_ROOT on a TempDir of its own at that instant, they land \
             there. (This call resolved {} only because the ctor's sandbox happened to \
             be the ambient root when it ran.)",
            pinned.display()
        );
    }
}
