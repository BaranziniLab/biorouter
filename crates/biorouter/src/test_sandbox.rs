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
/// this binary used to point that variable at a `TempDir` of their own under
/// `env_lock` and put it back, so a read taken at test time answered
/// "whichever test is relocating it at this instant". None of them does that in
/// the shared process any more — each is handed a directory, or runs in a
/// process of its own ([`in_a_process_of_its_own`]) — but a test inside such a
/// child still moves it, and this cell is still the only answer that cannot
/// move.
static SANDBOX_ROOT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Run before `main`, because the placement is the whole point.
///
/// `Config::global()` (`GLOBAL_CONFIG`), the session store's root
/// (`SHARED_STORE_ROOT`) and the global `AgentManager`'s config root
/// (`SHARED_CONFIG_ROOT`) are one-shot cells that resolve their path from
/// `BIOROUTER_PATH_ROOT` the first time anything touches them, and the tests run
/// in parallel — so a guard installed inside whichever test module "owns" the
/// hazard fixes nothing whenever another test got there first. Running before
/// any test is the only placement that cannot lose that race (the same
/// reasoning as `tests/agent.rs`).
///
/// An outer `BIOROUTER_PATH_ROOT` wins: the Task 33 gate exports its own
/// `mktemp -d` root, and a harness that wants to inspect what a run wrote must
/// be able to choose where it lands. One that `Paths` reads as absent does
/// not — unset, blank after `trim()`, or not valid UTF-8 — and the ctor asks
/// that through [`Paths::path_root_override`](crate::config::paths::Paths::path_root_override),
/// the predicate `Paths::get_dir` itself uses, so the two cannot disagree.
/// Honouring a blank root records it as the sandbox while every cell resolves
/// the developer's real directories; the `is_empty()` test this replaced did
/// exactly that for `"   "` (measured; see that function's docs).
///
/// ⚠ Setting the variable is only half of it. `BIOROUTER_PATH_ROOT` is
/// process-global and dozens of tests here relocated it under a `TempDir` of
/// their own (`env_lock::lock_env`) — now only inside a process of their own,
/// where the same hazard still applies — so whichever test first reaches the session
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
///
/// ⚠ `Config::global()` is frozen here too, the way `biorouter-cli`'s ctor
/// already froze it; until then nothing did. A review measured
/// `the_lib_test_binary_config_root_is_sandboxed` failing 3 of 9 whole-binary
/// runs once a probe test relocated `BIOROUTER_PATH_ROOT` and touched
/// `Config::global()` (0 of 9 without one: latent, not absent). Re-measured
/// here, macOS, 2026-09-21, with a temporary probe that holds the variable on
/// its own `TempDir` and touches the cell: run first (`--test-threads=1`), the
/// guard failed with *"Config::global() resolved to …/T/.tmpH1dgLK/config/
/// config.yaml, outside the sandbox"*; racing it in parallel, the probe won in
/// 13 of 60 `test_sandbox::` runs; with this line, 0 of 60 and passed in order.
/// `Config::default()` is safe before `main` for the same reasons the CLI
/// header gives, re-read here rather than assumed: with
/// `BIOROUTER_PATH_ROOT` set (it always is by now) it is one environment read,
/// two `join`s, a read of `BIOROUTER_DISABLE_KEYRING`, and `Mutex::new`s plus
/// the test-only fault/probe counters' derived `Default`s — no file, no
/// keyring. It records the keyring *service name*; it never opens the store.
/// What it does freeze is the secret BACKEND: `BIOROUTER_DISABLE_KEYRING` is
/// read now, from the outer environment, where before it was read by whichever
/// test touched the cell first — and no test here sets that variable
/// in-process; only `handle_keyring_fallback_error` does, at run time, after a
/// keyring failure. `rust.yml` sets it for this suite, and on macOS a run
/// without it can block in `SecKeychainFindGenericPassword` whether or not this
/// line exists, because provider construction reads secrets.
#[ctor::ctor]
fn sandbox_config_root_for_the_lib_test_binary() {
    if crate::config::paths::Paths::path_root_override().is_none() {
        let root = match tempfile::TempDir::new() {
            Ok(root) => root,
            Err(error) => refuse_to_run_unsandboxed(&format!(
                "could not create a scratch config root under {}: {error}",
                std::env::temp_dir().display()
            )),
        };
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
    // …and the global manager's config root, for the same reason: its first
    // `instance()` call seeds the Soul skill, built-in skills and Meditation
    // workflow into it, and that call lands at an instant no test owns. Frozen
    // here, a test holding `BIOROUTER_PATH_ROOT` on a root of its own is not
    // handed the GLOBAL manager's config-root seeding. Other writers still
    // follow the ambient root: the scheduler's workflow copy
    // (`Paths::data_dir()` at copy time; see `AgentManager::new`), and
    // `SkillsClient::new`, which seeds the built-in skills into
    // `Paths::config_dir()` as it is when a client is built.
    let _ = crate::execution::manager::AgentManager::shared_config_root();
    // …and `Config::global()`, which every config write in the binary follows.
    let _ = crate::config::Config::global();

    // Observed, not assumed — and after every freeze above, so this is what the
    // cells hold at the end of the ctor, before any test can run. Each read is
    // the cell's non-initializing `get`; see [`ResolvedBeforeMain`].
    let _ = RESOLVED_BEFORE_MAIN.set(ResolvedBeforeMain {
        session_store_root:
            crate::session::session_manager::SessionManager::shared_store_root_if_resolved(),
        agent_manager_config_root:
            crate::execution::manager::AgentManager::shared_config_root_if_resolved(),
        global_config: crate::config::Config::global_if_initialized(),
    });
}

/// Stop the binary before `main` rather than run it unsandboxed.
///
/// Without a root of its own, every cell the ctor freezes below would resolve
/// the developer's real `~/.config/biorouter` and data dir, and every test
/// would write there — the thing this module exists to prevent. `abort`, not a
/// panic: a panic cannot unwind out of a ctor, and the message is the point.
fn refuse_to_run_unsandboxed(why: &str) -> ! {
    eprintln!(
        "biorouter lib test binary: {why}. Refusing to run: without a sandbox root every \
         test would read and write the developer's real config and data directories. \
         Fix the temp directory, or export BIOROUTER_PATH_ROOT to a directory to use."
    );
    std::process::abort()
}

/// What each one-shot cell the ctor freezes held when the ctor returned, read
/// WITHOUT resolving it. `None` means nothing had resolved that cell before
/// `main`.
///
/// ⚠ This is what lets the three `…_is_sandboxed` guards below fail when a
/// freeze line is deleted, and they could not before. Each used to assert
/// `cell().starts_with(sandbox_path_root())` alone. With the freeze deleted,
/// that call is itself the first touch, and the ctor has already pointed
/// `BIOROUTER_PATH_ROOT` at the sandbox, so the cell resolves the sandbox and
/// the guard passes — measured 2026-09-21: both freeze lines deleted, all six
/// `test_sandbox` tests passed. They could only catch a cell resolved too
/// EARLY, with a non-sandbox value. The regression that matters is the other
/// one: an unfrozen cell first touched by a test holding `BIOROUTER_PATH_ROOT`
/// on its own `TempDir`, which pins the whole binary to a directory deleted
/// when that test ends. Whether that happens depends on scheduling, so a guard
/// that waits for it to happen is a coin flip; asking "was the cell resolved
/// before `main`?" is not.
struct ResolvedBeforeMain {
    session_store_root: Option<&'static std::path::Path>,
    agent_manager_config_root: Option<&'static std::path::Path>,
    global_config: Option<&'static crate::config::Config>,
}

static RESOLVED_BEFORE_MAIN: std::sync::OnceLock<ResolvedBeforeMain> = std::sync::OnceLock::new();

/// The ctor's record of what was frozen before `main`.
fn resolved_before_main() -> &'static ResolvedBeforeMain {
    RESOLVED_BEFORE_MAIN
        .get()
        .expect("the ctor records what it froze before main; an absent record means it did not run")
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
/// *before* the lock is acquired, so if any of the (then ~30) relocating tests holds
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

/// Names, in a child this binary started, the one test that child runs. See
/// [`in_a_process_of_its_own`].
const OWN_PROCESS_TEST: &str = "BIOROUTER_TEST_IN_A_PROCESS_OF_ITS_OWN";

/// **No test in this binary moves `BIOROUTER_PATH_ROOT` off the sandbox while
/// other tests are running.** A test that has to — because its subject resolves
/// `Paths` itself and cannot be handed a root — calls this first:
///
/// ```ignore
/// if !crate::test_sandbox::in_a_process_of_its_own() {
///     return;
/// }
/// let _root = crate::test_sandbox::relocate_path_root(temp.path().to_str().unwrap());
/// ```
///
/// In the test run it re-executes this binary with `--exact <this test>` under
/// a sandbox root of its own, waits, and returns `false` once that child has
/// passed (a failure panics here, carrying the child's output). In the child it
/// returns `true` and the body runs — alone in its process, so the relocation
/// it makes is seen by nothing else.
///
/// ⚠ Why this and not a lock. At least 466 tests in this binary resolve
/// `Paths`-derived state without taking `env_lock`. Measured 2026-09-21 with a
/// temporary probe: the ambient root pointed, after the ctor's freezes, at a
/// directory no test owned, and every resolution that landed there recorded
/// with its test — through `ToolPermissionStore::new` (210 tests),
/// `privacy::provenance` (193), `ManagedPolicy::load` (169), the knowledge and
/// memory roots (49), `SkillsClient` and the skill catalog, hints and
/// `RequestLog`. A lower bound: a read that fell inside another test's
/// relocation saw that test's root and was not recorded —
/// `agents::knowledge_source_tool`'s Gate H tests resolve it twice each and are
/// absent from the record. A lock
/// orders only the tests that take it, so every one of those read whatever
/// root a relocating test held at that instant — a sibling's managed policy,
/// its skills, its knowledge bases, or a directory deleted moments later.
/// Giving each a lock or a root parameter is hundreds of edits; not moving the
/// variable in-process makes all of them immune at once, which is the
/// construction `only_the_sandbox_relocates_the_path_root_and_only_in_a_process_of_its_own`
/// enforces.
///
/// The child is checked for `1 passed`, not just a zero exit: a test name that
/// matched nothing would exit 0 having run nothing.
///
/// ⚠ **Spawned under `env_lock`, waited for outside it.** A child copies this
/// process's environment at the instant it starts, so without the lock it
/// inherited whatever a sibling had set only for itself under `env_lock` at
/// that instant — `OLLAMA_HOST`, `BIOROUTER_PROVIDER`, a relocated `HOME` —
/// and kept it for its whole run. Measured 2026-09-22 with a temporary probe (a
/// sibling holding a variable under the lock, this helper spawning while it
/// held it): the child saw the sibling's value in 5 of 5 runs; with the lock,
/// 0 of 5. Waiting under the lock too would stall every test that takes it for
/// as long as the child runs.
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
        // Setting nothing: the lock alone is what keeps a sibling's temporary
        // value out of the environment the child copies.
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
/// inside a process [`in_a_process_of_its_own`] started, and it panics
/// anywhere else. That child runs one test, so every thread in it is that
/// test's, and nothing else can resolve the root this moves.
pub(crate) fn relocate_path_root(root: impl AsRef<str>) -> env_lock::EnvGuard<'static> {
    relocate_path_root_and(root, [])
}

/// [`relocate_path_root`], setting (or, with `None`, removing) `also` under
/// the same lock.
pub(crate) fn relocate_path_root_and<const N: usize>(
    root: impl AsRef<str>,
    also: [(&'static str, Option<&str>); N],
) -> env_lock::EnvGuard<'static> {
    assert!(
        std::env::var_os(OWN_PROCESS_TEST).is_some(),
        "a test moved BIOROUTER_PATH_ROOT in the shared test process. Every test that \
         resolves Paths without the env lock would follow it into a directory it does \
         not own; start the test with `if !test_sandbox::in_a_process_of_its_own() \
         {{ return; }}`"
    );
    let mut vars: Vec<(&'static str, Option<String>)> =
        vec![("BIOROUTER_PATH_ROOT", Some(root.as_ref().to_owned()))];
    vars.extend(also.map(|(name, value)| (name, value.map(str::to_owned))));
    env_lock::lock_env(vars)
}

#[cfg(test)]
mod tests {
    /// Nothing in this binary may resolve to the developer's live
    /// configuration, or to a test's soon-deleted `TempDir`.
    ///
    /// Asserting on `Config::global()` rather than on the environment is what
    /// makes the first half meaningful: it is the resolved path a write
    /// actually follows, and it is frozen at first use. The second half is what
    /// makes the guard able to fail when the ctor's freeze is deleted — see
    /// [`super::ResolvedBeforeMain`] for why the first half alone could not.
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
             Either something reached Config::global() before the sandbox was \
             installed, or nothing froze it before main and a test holding \
             BIOROUTER_PATH_ROOT on a directory of its own touched it first — and \
             every config write from this binary now follows that path."
        );
        assert!(
            super::resolved_before_main().global_config.is_some(),
            "Config::global() was not initialized before main: the ctor's freeze is \
             missing. The first test to touch it now decides where this binary's \
             config.yaml lives, and a test holding BIOROUTER_PATH_ROOT on a TempDir \
             of its own pins every later config write inside a directory that is \
             deleted when it ends. (This call resolved {path} only because the ctor's \
             sandbox happened to be the ambient root when it ran.)"
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
    ///
    /// In a process of its own, because installing a foreign root is exactly
    /// what no test may do where others are running.
    #[test]
    fn the_pin_source_ignores_a_root_another_test_has_installed() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
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
            let _guard = super::relocate_path_root(held.as_str());
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
    /// relocated (in a process of their own, some still do). Adding a lock to
    /// the writers cannot close that; not reading can. The resolver is the one place that must read it, because production
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

    /// Nothing in this crate moves `BIOROUTER_PATH_ROOT` except through this
    /// module — `pin_sandbox_path_root` (which holds it at the value it
    /// already has) and `relocate_path_root` (which refuses outside a process
    /// of its own) — so the ambient root every unlocked reader resolves is the
    /// sandbox for the life of the process. See `in_a_process_of_its_own`.
    ///
    /// The same holds for the `biorouter-server` and `biorouter-cli` lib test
    /// binaries, which carry their own copies of those helpers, and there it
    /// covers the home directory too (`HOME`, `USERPROFILE`): the skill catalog
    /// reads `~/.claude/skills` from it on every call. Checked from here because
    /// this is the one guard, not one per crate — and because those crates'
    /// `test_sandbox.rs` is `#[path]`-included by every integration binary, a
    /// copy there would run a dozen times per `cargo test`. Before their
    /// relocators moved out,
    /// `routes::session_reach::bypass_tests::the_knowledge_base_gate_fires_under_the_served_router_tree`
    /// failed once in 15 whole-binary runs with `git init …/T/.tmp…/config/
    /// knowledge/.creating-…`: its `AppState::new()` resolved the knowledge
    /// root while a sibling held the variable on a `TempDir` (the test is
    /// `#[serial]`, so the sibling was `routes::skills`' non-serial one), and
    /// that directory was deleted under `create_base`. Forced — the same
    /// relocation held across `AppState::new()`, its deletion overlapping
    /// `create_base` — 20 of 20 runs failed that way (2026-09-22).
    ///
    /// A line naming a variable as a string literal outside a comment is what
    /// an `env_lock::lock_env`, `set_var` or `remove_var` of it looks like,
    /// split across lines or not. Two spellings move nothing in this process
    /// and are allowed: a child process's `.env(`, and a read (`env::var(` /
    /// `env::var_os(`) — `HOME` is read in production, and reading
    /// `BIOROUTER_PATH_ROOT` here is the neighbouring guard's business. They
    /// are counted, not matched, so a line that reads the variable AND sets it
    /// is still caught.
    #[test]
    fn only_the_sandbox_relocates_the_path_root_and_only_in_a_process_of_its_own() {
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates/")
            .to_path_buf();
        const PATH_ROOT: &str = concat!("BIOROUTER_", "PATH_ROOT");
        // (crate, files that may move them, variables, fewest files a real walk finds)
        let audits: [(&str, &[&str], &[&str], usize); 3] = [
            (
                "biorouter",
                &["config/paths.rs", "test_sandbox.rs"],
                &[PATH_ROOT],
                100,
            ),
            (
                "biorouter-server",
                &["test_sandbox.rs"],
                &[PATH_ROOT, "HOME", "USERPROFILE"],
                30,
            ),
            (
                "biorouter-cli",
                &["test_sandbox.rs"],
                &[PATH_ROOT, "HOME", "USERPROFILE"],
                30,
            ),
        ];

        let mut offenders: Vec<String> = Vec::new();
        for (krate, allowed, variables, fewest) in audits {
            let crate_src = crates.join(krate).join("src");
            assert!(
                crate_src.is_dir(),
                "the audit walks {}; if that path is wrong it passes for the wrong reason",
                crate_src.display()
            );
            let mut scanned = 0usize;
            for entry in walkdir::WalkDir::new(&crate_src) {
                let entry =
                    entry.expect("the audit must not silently skip an unreadable directory");
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
                    if code.starts_with("//") {
                        continue;
                    }
                    for variable in variables {
                        let literal = format!("\"{variable}\"");
                        let moves = code.matches(&literal).count();
                        // `env::var(`, not `var(`: `set_var(` and
                        // `remove_var(` end in `var(` too.
                        let benign: usize = [".env(", "env::var(", "env::var_os("]
                            .iter()
                            .map(|call| code.matches(&format!("{call}{literal}")).count())
                            .sum();
                        if moves > benign {
                            offenders
                                .push(format!("{krate}/src/{rel}:{} ({variable})", number + 1));
                        }
                    }
                }
            }
            assert!(
                scanned > fewest,
                "only {scanned} files scanned under {} — the walk found nothing to audit",
                crate_src.display()
            );
        }
        assert!(
            offenders.is_empty(),
            "these move BIOROUTER_PATH_ROOT or the home directory in the shared test \
             process: {offenders:?}. Every test there that resolves Paths or the home \
             directory without the env lock follows it for as long as it is held — a \
             sibling's managed policy, skills and knowledge bases, then a deleted \
             directory. Hand the subject a directory of its own, or start the test with \
             `if !test_sandbox::in_a_process_of_its_own() {{ return; }}` and move it with \
             that crate's `test_sandbox::relocate_*` helper."
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
    ///
    /// ⚠ The second assertion is the one that notices a deleted freeze. The
    /// first cannot: without the freeze, its own `shared_store_root()` call is
    /// the first touch and resolves the sandbox the ctor has just installed.
    #[test]
    fn the_lib_test_binary_session_store_is_sandboxed() {
        let root = super::sandbox_path_root();
        let pinned = crate::session::session_manager::SessionManager::shared_store_root();
        assert!(
            pinned.starts_with(root),
            "the process session store is pinned at {}, outside the sandbox at {root}. \
             Either something resolved it before the ctor installed the sandbox, or \
             nothing froze it before main and a test holding BIOROUTER_PATH_ROOT on a \
             TempDir of its own reached it first — so this binary's sessions.db lives \
             in a directory that test deletes.",
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

    /// The same claim for the process-global `AgentManager`'s config root —
    /// the directory its first-run init seeds `update-soul`, the built-in
    /// skills and the Soul KB into. Frozen by its own call in the ctor, so it
    /// needs its own assertions: if nothing froze it, a test holding
    /// `BIOROUTER_PATH_ROOT` on its own root when `instance()` first runs is
    /// handed that seeding, and the global manager keeps seeding and reading a
    /// directory that is deleted when that test ends.
    #[test]
    fn the_lib_test_binary_agent_manager_config_root_is_sandboxed() {
        let root = super::sandbox_path_root();
        let pinned = crate::execution::manager::AgentManager::shared_config_root();
        assert!(
            pinned.starts_with(root),
            "the global AgentManager's config root is pinned at {}, outside the sandbox \
             at {root}. Either something resolved it before the ctor installed the \
             sandbox, or nothing froze it before main and a test holding \
             BIOROUTER_PATH_ROOT on a TempDir of its own reached instance() first — so \
             its first-run seeding landed in that test's directory.",
            pinned.display()
        );
        assert!(
            super::resolved_before_main()
                .agent_manager_config_root
                .is_some(),
            "the global AgentManager's config root was not resolved before main: the \
             ctor's freeze is missing. The first AgentManager::instance() now decides \
             where the Soul KB and built-in skills are seeded, and if a test is holding \
             BIOROUTER_PATH_ROOT on a TempDir of its own at that instant, they land \
             there. (This call resolved {} only because the ctor's sandbox happened to \
             be the ambient root when it ran.)",
            pinned.display()
        );
    }
}
