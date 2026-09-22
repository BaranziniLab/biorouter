//! Points this test binary's Biorouter config/data/state dirs at a throwaway
//! directory before the first line of test code runs, and freezes the
//! process-global cells there so no later test can move them.
//!
//! `crates/biorouter` and `crates/biorouter-server` have had this since issue
//! #54 and BR-71 respectively. This crate was the last one without it, and it is
//! the crate with the most exposure: 27 `SessionManager::instance()` sites
//! (`commands/session.rs` 9, `cli.rs` 7, `commands/term.rs` 4,
//! `commands/schedule.rs` 3, `commands/knowledge.rs` 2, `commands/usage.rs` and
//! `commands/session_watch.rs` 1 each) and 42 `Config::global()` sites, all of
//! which resolved against the developer's real root. Measured on `main`
//! (5a404ecd) by running the two guard tests below with the ctor absent:
//!
//! ```text
//! the_config_file_is_not_the_developers:
//!     expected the per-process sandbox, got /Users/…/.config/biorouter/config.yaml
//! the_session_store_is_pinned_inside_the_sandbox:
//!     the process session store is pinned at /Users/…/.local/share/biorouter
//! ```
//!
//! That is not a theoretical reach. A plain `cargo test -p biorouter-cli --lib`
//! run against an empty `HOME` (475 passed, `main`) left one file behind:
//! `<HOME>/.config/biorouter/config.yaml`. On a real machine that same write
//! targets the developer's live config, and every `set_param` a test reaches
//! lands in it. `commands/session.rs` says so in its own words — "it opens the
//! developer's REAL session database" — which is why that comment survived the
//! sweep that corrected 24 stale copies of it elsewhere.
//!
//! Documenting a workaround is not a fix. `CLAUDE.md` tells contributors to run
//! this crate's tests under `HOME=$(mktemp -d)`, but the damage happens on the
//! DEFAULT invocation — what a developer types, and what an editor's test runner
//! emits, neither of which reads CLAUDE.md.
//!
//! An externally supplied `BIOROUTER_PATH_ROOT` still wins, so a harness that
//! already sandboxes the process keeps its own root. `tests/non_interactive.rs`,
//! `tests/serve_lifecycle.rs` and `tests/apps_serve_lifecycle.rs` pass their own
//! root to each child `Command`, which overrides the inherited one either way.
//!
//! ## Setting the variable is not enough — the cells have to be PINNED
//!
//! `BIOROUTER_PATH_ROOT` is process-global, and six files here legitimately
//! relocate it under a `TempDir` of their own with `env_lock::lock_env`
//! (`cli.rs`, `logging.rs`, `commands/{knowledge,skill,schedule,
//! extension}.rs`) — since 2026-09-22 only in a process of their own
//! ([`in_a_process_of_its_own`]), where the freeze below still decides where
//! that child's cells live. `SHARED_STORE_ROOT` and `GLOBAL_CONFIG` are one-shot cells
//! that resolve their path the first time anything touches them, and the tests
//! run in parallel — so whichever test gets there first decides for the whole
//! binary, and that test may be one holding such a lock. Its `TempDir` then
//! drops, the directory is unlinked, and the cell points at a path that no
//! longer exists.
//!
//! For the session store that is the Windows CI flake PR #282 fixed in the other
//! two crates, and its symptom looks nothing like its cause: the connection the
//! pool already opened keeps answering, because SQLite on POSIX does not care
//! that its inode lost its name, so serial queries still succeed. It is the
//! first moment two tasks want the pool at once, forcing a **second**
//! connection, that fails with `(code: 14) unable to open database file` — in
//! whatever test happened to query next.
//!
//! So the ctor freezes both cells here, at a root this module owns. Each costs
//! one environment read and some path arithmetic — `SessionManager::
//! shared_store_root()` resolves a `PathBuf`, and `Config::default()` joins two
//! paths and builds mutexes. No pool, no disk, no async runtime, which is what
//! makes them safe to run before `main`.
//!
//! ⚠ Pinning `Config::global()` also freezes its secret backend, because
//! `Config::default()` reads `BIOROUTER_DISABLE_KEYRING` at that moment. That
//! choice was already frozen by whichever of the 42 call sites ran first, so
//! this makes an existing coin-flip deterministic rather than introducing one —
//! and it constructs no keyring, it only records the service name, so nothing
//! here touches the OS keychain. Every documented invocation of this suite sets
//! `BIOROUTER_DISABLE_KEYRING=true`, including `rust.yml`.

/// This binary's sandbox root, recorded before any test could move it.
///
/// ⚠ **`std::env::var("BIOROUTER_PATH_ROOT")` is NOT a way to ask what the
/// sandbox root is, at any point after `main` starts.** The six files listed
/// above pointed that variable at a `TempDir` of their own and put it back, so a
/// read taken at test time answered "whichever test is relocating it at this
/// instant" — a different directory on every run, deleted moments later. They
/// now do it only in a process of their own, but this cell is still the only
/// answer that cannot move, and that is why it exists rather than the
/// assertions re-deriving the root themselves.
static SANDBOX_ROOT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Run before `main`, because the placement is the whole point: a guard
/// installed inside whichever test module "owns" the hazard fixes nothing
/// whenever another test got to the singleton first.
///
/// "Is a root already in force?" is asked through
/// `Paths::path_root_override`, the predicate `Paths::get_dir` itself uses, so
/// a root this ctor honours is one every cell will resolve. The
/// `var_os(..).is_none_or(|v| v.is_empty())` test this replaced honoured a
/// whitespace-only root that `Paths` reads as absent: it recorded `"   "` as the
/// sandbox, and `Config::global()` and the session store then resolved the
/// developer's real directories. Measured on this binary 2026-09-21, under a
/// throwaway `HOME`, with that test put back: `BIOROUTER_PATH_ROOT='   '`
/// failed both cell guards, naming `<HOME>/.config/biorouter/config.yaml` and
/// `<HOME>/.local/share/biorouter`.
#[ctor::ctor]
fn sandbox_config_root_for_this_test_binary() {
    if biorouter::config::paths::Paths::path_root_override().is_none() {
        let root = match tempfile::TempDir::new() {
            Ok(root) => root,
            Err(error) => refuse_to_run_unsandboxed(&format!(
                "could not create a scratch config root under {}: {error}",
                std::env::temp_dir().display()
            )),
        };
        std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
        // Leaked deliberately: a `static` is never dropped, which is exactly the
        // lifetime the sandbox needs — it must outlive the last test in the
        // binary, including anything a `#[dtor]` elsewhere runs.
        static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        let _ = ROOT.set(root);
    }
    // Both branches, and after the set: an outer root is recorded as-is, and one
    // we minted is recorded as the value we just installed.
    if let Some(root) = std::env::var_os("BIOROUTER_PATH_ROOT") {
        record_sandbox_root(&root);
    }
    // Freeze both process-global cells at whatever root is now in force, before
    // any test can relocate the variable under a `TempDir` it later unlinks.
    // Unconditional: an externally supplied root needs the same protection.
    let _ = biorouter::session::session_manager::SessionManager::shared_store_root();
    let _ = biorouter::config::Config::global();

    // Observed, not assumed, after both freezes: each read is the cell's
    // non-initializing `get`, so an entry is `Some` only if something above
    // really resolved it before `main`. See [`ResolvedBeforeMain`].
    let _ = RESOLVED_BEFORE_MAIN.set(ResolvedBeforeMain {
        session_store_root:
            biorouter::session::session_manager::SessionManager::shared_store_root_if_resolved(),
        global_config: biorouter::config::Config::global_if_initialized(),
    });
}

/// Stop the binary before `main` rather than run it unsandboxed: without a root
/// of its own, the cells frozen below resolve the developer's real
/// `~/.config/biorouter` and every test writes there. `abort`, not a panic — a
/// panic cannot unwind out of a ctor, and the message is the point.
fn refuse_to_run_unsandboxed(why: &str) -> ! {
    eprintln!(
        "biorouter-cli test binary: {why}. Refusing to run: without a sandbox root every \
         test would read and write the developer's real config and data directories. \
         Fix the temp directory, or export BIOROUTER_PATH_ROOT to a directory to use."
    );
    std::process::abort()
}

/// What each cell the ctor freezes held when the ctor returned, read WITHOUT
/// resolving it; `None` means nothing had resolved it before `main`.
///
/// ⚠ This is the half of each guard below that can notice a deleted freeze.
/// `cell().starts_with(sandbox_path_root())` alone cannot: with the freeze
/// deleted, the guard's own call is the first touch, lands while the ctor's
/// sandbox is still the ambient root, and passes. A review of this module
/// measured exactly that on 2026-09-21: both freeze lines deleted, the guards
/// still green (3 of 3). What they could catch was a cell resolved too early
/// with a non-sandbox value — not the one that matters, an unfrozen cell first
/// touched by one of the six relocating files' tests, which depends on
/// scheduling and so is a coin flip for any guard that waits for it.
struct ResolvedBeforeMain {
    session_store_root: Option<&'static std::path::Path>,
    global_config: Option<&'static biorouter::config::Config>,
}

static RESOLVED_BEFORE_MAIN: std::sync::OnceLock<ResolvedBeforeMain> = std::sync::OnceLock::new();

#[cfg(test)]
fn resolved_before_main() -> &'static ResolvedBeforeMain {
    RESOLVED_BEFORE_MAIN
        .get()
        .expect("the ctor records what it froze before main; an absent record means it did not run")
}

fn record_sandbox_root(root: &std::ffi::OsStr) {
    // A root that is not valid UTF-8 is left unrecorded rather than recorded
    // lossily — a mangled root would name a *different* directory, and
    // `sandbox_path_root` saying so beats an assertion quietly comparing against
    // the wrong path.
    if let Some(root) = root.to_str() {
        let _ = SANDBOX_ROOT.set(root.to_owned());
    }
}

/// The config/data root every test in this binary resolves under, unless a test
/// is deliberately relocating it.
#[cfg(test)]
fn sandbox_path_root() -> &'static str {
    SANDBOX_ROOT
        .get()
        .expect(
            "the sandbox root was recorded before main; an absent value means either the ctor \
             did not run or BIOROUTER_PATH_ROOT is not valid UTF-8 and cannot be recorded",
        )
        .as_str()
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
/// let _env = crate::test_sandbox::relocate_path_root(tmp.path());
/// ```
///
/// In the test run it re-executes this binary with `--exact <this test>` under
/// a sandbox root of its own, waits, and returns `false` once that child has
/// passed (a failure panics here, carrying the child's output). In the child it
/// returns `true` and the body runs — alone in its process, so the relocation
/// it makes is seen by nothing else.
///
/// ⚠ Why this and not the env lock: a lock orders only the tests that take it,
/// and tests here resolve `Paths` without it — through
/// `KnowledgeService::new_default()`, `skills_root()` and `extensions_root()`,
/// among others. While a sibling held the variable on its `TempDir` they
/// resolved THAT directory, and then it was deleted; `logging`'s test removed
/// the variable altogether, pointing each of them at a temporary `HOME`'s
/// default directories for as long as it ran.
/// The same collision is what failed the `biorouter-server` lib binary's
/// knowledge-gate test (see that crate's copy of this helper).
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
    let root = root.to_str().expect("a UTF-8 temp path").to_owned();
    let mut vars = vec![("BIOROUTER_PATH_ROOT", Some(root))];
    vars.extend(also);
    relocate(vars)
}

/// Hold the platform's home variable (`USERPROFILE` on Windows, `HOME`
/// elsewhere) at `home` and REMOVE `BIOROUTER_PATH_ROOT`, so `Paths` resolves
/// the platform's default directories under that home — in a process of its
/// own only, like [`relocate_path_root`].
#[cfg(test)]
#[allow(dead_code)] // `#[path]`-included by integration binaries that relocate nothing
pub(crate) fn relocate_home_off_the_path_root(
    home: &std::path::Path,
) -> env_lock::EnvGuard<'static> {
    let home = home.to_str().expect("a UTF-8 temp path").to_owned();
    relocate(vec![
        (
            if cfg!(windows) { "USERPROFILE" } else { "HOME" },
            Some(home),
        ),
        ("BIOROUTER_PATH_ROOT", None),
    ])
}

#[cfg(test)]
#[allow(dead_code)] // `#[path]`-included by integration binaries that relocate nothing
fn relocate(vars: Vec<(&'static str, Option<String>)>) -> env_lock::EnvGuard<'static> {
    assert!(
        std::env::var_os(OWN_PROCESS_TEST).is_some(),
        "a test moved BIOROUTER_PATH_ROOT or the home directory in the shared test process. \
         Every test that resolves them without the env lock would follow it into a \
         directory it does not own; start the test with \
         `if !crate::test_sandbox::in_a_process_of_its_own() {{ return; }}`"
    );
    env_lock::lock_env(vars)
}

#[cfg(test)]
mod tests {
    use biorouter::config::Config;
    use biorouter::session::session_manager::SessionManager;

    /// Nothing in this binary may resolve to the developer's live configuration.
    ///
    /// Asserting on `Config::global()` rather than on `Paths::config_dir()` is
    /// what makes this meaningful, and the distinction is not pedantic:
    /// `Paths::config_dir()` re-reads `BIOROUTER_PATH_ROOT` on every call, so it
    /// would answer "sandboxed" even in a binary where something had already
    /// frozen `GLOBAL_CONFIG` at the developer's real `config.yaml`.
    /// `Config::global().path()` is the file a write actually follows.
    ///
    /// The first assertion fails if anything reached `GLOBAL_CONFIG` under a
    /// different root before this runs. The second is the one that fails if a
    /// future edit drops the pin from the ctor — the first cannot see that,
    /// because its own call then resolves the sandbox (see
    /// [`super::ResolvedBeforeMain`]).
    #[test]
    fn the_config_file_is_not_the_developers() {
        // The recorded root, not `std::env::var` — a read taken here answers
        // "whichever test is relocating the variable right now", and this
        // assertion would then blame the sandbox for a sibling's `TempDir`.
        let root = super::sandbox_path_root();
        let path = Config::global().path();
        assert!(
            path.starts_with(root),
            "Config::global() resolved to {path}, outside the sandbox at {root}. \
             Either something reached Config::global() before the sandbox was \
             installed, or nothing froze it before main and a test holding \
             BIOROUTER_PATH_ROOT on a TempDir of its own touched it first — and every \
             config write from this binary now follows that path."
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

    /// Both assertions above compare two values the ctor derived from the same
    /// root, so they answer "is the process consistent with itself" and would
    /// stay green if the ctor had recorded the developer's real root without
    /// installing a sandbox at all. This is the one that notices that, and it is
    /// the only check here that names an outside reference point.
    ///
    /// ⚠ The home directory is read under **both** names. `HOME` is POSIX;
    /// Windows calls it `USERPROFILE` and does not define `HOME` outside a Git
    /// Bash session. Falling back to an empty string keeps the assertion
    /// meaningful when neither is set, because `starts_with("")` is true for
    /// every path — a missing home makes this check STRICTER, not vacuous.
    ///
    /// ⚠ The `temp_dir()` arm is not a loophole, it is Windows. `TempDir` lands
    /// under `%LOCALAPPDATA%\Temp`, which IS inside `USERPROFILE`, so a bare
    /// "outside the home directory" rule is red on every Windows runner while
    /// the sandbox is working perfectly.
    #[test]
    fn the_sandbox_root_is_not_the_developers_home() {
        let root = std::path::Path::new(super::sandbox_path_root());
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_default();
        assert!(
            !root.starts_with(&home) || root.starts_with(std::env::temp_dir()),
            "the sandbox root is {}, inside the developer's home at {home} and outside \
             the temp dir. The ctor recorded a root it did not create, so every \
             assertion in this module is comparing the real config against itself.",
            root.display()
        );
    }

    /// The same claim for the session database, and the stricter of the two.
    ///
    /// ⚠ Assert the **pinned** value, never `Paths::data_dir()`. That function
    /// re-reads the environment on every call and would pass while the store was
    /// pinned somewhere else entirely — the exact hole this module exists to
    /// close. `shared_store_root()` is the directory a query follows, frozen for
    /// the life of the process.
    ///
    /// ⚠ The second assertion is the one that notices a deleted freeze; the
    /// first cannot, for the reason [`super::ResolvedBeforeMain`] gives.
    #[test]
    fn the_session_store_is_pinned_inside_the_sandbox() {
        let root = super::sandbox_path_root();
        let pinned = SessionManager::shared_store_root();
        assert!(
            pinned.starts_with(root),
            "the process session store is pinned at {}, outside the sandbox at {root}. \
             Either something resolved it before the ctor installed the sandbox, or \
             nothing froze it before main and a test holding BIOROUTER_PATH_ROOT on a \
             TempDir of its own reached it first — so this binary's sessions.db lives \
             in a directory that test deletes, and the failure surfaces as (code: 14) \
             in whatever test queried next.",
            pinned.display()
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
}
