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
//! `BIOROUTER_PATH_ROOT` is process-global, and seven files here legitimately
//! relocate it under a `TempDir` of their own with `env_lock::lock_env`
//! (`cli.rs`, `logging.rs`, `commands/{serve,knowledge,skill,schedule,
//! extension}.rs`). `SHARED_STORE_ROOT` and `GLOBAL_CONFIG` are one-shot cells
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
/// sandbox root is, at any point after `main` starts.** The seven files listed
/// above point that variable at a `TempDir` of their own and put it back, so a
/// read taken at test time answers "whichever test is relocating it at this
/// instant" — a different directory on every run, deleted moments later. This
/// cell is the only stable answer, and that is why it exists rather than the
/// assertions re-deriving the root themselves.
static SANDBOX_ROOT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Run before `main`, because the placement is the whole point: a guard
/// installed inside whichever test module "owns" the hazard fixes nothing
/// whenever another test got to the singleton first.
#[ctor::ctor]
fn sandbox_config_root_for_this_test_binary() {
    if std::env::var_os("BIOROUTER_PATH_ROOT").is_none_or(|v| v.is_empty()) {
        let root = tempfile::TempDir::new().expect("scratch config root for the cli test binary");
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
    /// It fails if a future edit drops the pin from the ctor and a relocating
    /// test wins the race to `GLOBAL_CONFIG`.
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
             Something reached Config::global() before the sandbox was installed, so \
             config writes from this binary land in the developer's real config."
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
    #[test]
    fn the_session_store_is_pinned_inside_the_sandbox() {
        let root = super::sandbox_path_root();
        let pinned = SessionManager::shared_store_root();
        assert!(
            pinned.starts_with(root),
            "the process session store is pinned at {}, outside the sandbox at {root}. \
             Something resolved it before the ctor did, so a test that relocates \
             BIOROUTER_PATH_ROOT can move this binary's sessions.db into a TempDir \
             it then deletes — and the failure surfaces as (code: 14) in whatever \
             test queried next.",
            pinned.display()
        );
    }
}
