use biorouter::agents::turn_abort::{exit as abort_exit, TurnFailed};
use biorouter_cli::cli::cli;
use biorouter_cli::commands::needs_terminal::{self, NeedsTerminal};
use std::process::ExitCode;

// Tuned jemalloc as the global allocator (default-on `jemalloc` feature). The
// long-running CLI/TUI churns conversation buffers per turn; jemalloc returns
// freed pages to the OS far more readily than the system allocator's retained
// arenas.
#[cfg(all(feature = "jemalloc", not(target_os = "windows")))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// ⚠ Not `#[tokio::main]`: the CLI hosts agent turns too, and a subagent spawn
/// polls the child's reply on the parent's stack. See
/// `biorouter::execution::runtime`.
fn main() -> ExitCode {
    biorouter::execution::runtime::build_agent_runtime()
        .expect("build the agent runtime")
        .block_on(async_main())
}

async fn async_main() -> ExitCode {
    // BR-69: if this process was re-exec'd as the shell-sandbox helper (hidden
    // `__br-sandbox` marker, Linux only), apply the in-process Landlock/seccomp
    // restrictions and `execve` the target program. Never returns in that case;
    // a normal invocation falls straight through. Must run before any real work.
    biorouter_mcp::run_shell_sandbox_helper_if_invoked();
    tune_allocator();
    if let Err(e) = biorouter_cli::logging::setup_logging(None, None) {
        eprintln!("Warning: Failed to initialize logging: {}", e);
    }

    // Issue #56 Task 30. The CLI is a SECOND host of the same agent library, and
    // it loaded this switch nowhere: a user who turned privacy tiers off in
    // Settings got every gate still enforcing in the terminal, because the
    // fail-safe default is ON and nothing here ever read their config.
    //
    // That direction is safe, which is why it was invisible — but DR-15's
    // promise is "nothing will be impacted", not "nothing except the CLI", and a
    // control that silently applies where the user cannot see they turned it off
    // is worse than one that does not exist. Refusals here name Settings >
    // Privacy, which they had already visited.
    //
    // ⚠ NOT a third writer of the atomic. `load_privacy_tiers_from_config` is
    // the same one function `biorouterd` calls; `set_privacy_tiers_enabled` is
    // still spoken in exactly two places tree-wide, which is what Task 30's
    // Step 5 (2) counts.
    //
    // Before `cli()`, so no subcommand can run a turn against the default when
    // the user turned the feature off — and after logging, so a config failure
    // is reported rather than swallowed.
    biorouter::privacy::load_privacy_tiers_from_config();
    // Issue #56 Task 52 (DR-27). Beside the master switch's load rather than
    // anywhere else, because the omission this pair guards against is a host
    // that loads ONE of them: the CLI already skipped the master switch for a
    // whole round, and the direction it failed in was safe enough to be
    // invisible. `strict` failing quietly back to `standard` would be the same
    // kind of silence.
    biorouter::privacy::load_mixing_policy_from_record();

    match cli().await {
        Ok(()) => ExitCode::from(abort_exit::OK),
        Err(e) => {
            // A command that needs a person at a terminal and was run without
            // one declined to start; nothing failed. Its sentence says what to
            // run instead, so it is printed alone, and it exits 2 — the usage
            // error status — rather than the generic 1 (QA-D F9).
            if let Some(refusal) = e.downcast_ref::<NeedsTerminal>() {
                eprintln!("{refusal}");
                return ExitCode::from(needs_terminal::EXIT_CODE);
            }
            eprintln!("Error: {e:?}");
            // A turn that ran but did not complete its work gets its own exit
            // code, so a caller can tell "the provider rejected our key" (75)
            // from "the agent ran and disagreed with you" (0) — which is what a
            // 403 used to look like. Everything else keeps the historical rc 1.
            match e.downcast_ref::<TurnFailed>() {
                Some(failed) => ExitCode::from(failed.exit_code()),
                None => ExitCode::from(abort_exit::GENERIC),
            }
        }
    }
}

/// Lower jemalloc's dirty/muzzy decay so freed pages return to the OS within
/// ~1s instead of being retained (jemalloc's own defaults — `muzzy_decay_ms:0`,
/// `retain:true` — retain aggressively). Applied to all current arenas
/// (`MALLCTL_ARENAS_ALL` = 4096) and as the default for future arenas. Errors
/// are ignored (`background_thread` is unsupported on macOS). Set
/// `BIOROUTER_DEBUG_ALLOC=1` to print the applied values.
#[cfg(all(feature = "jemalloc", not(target_os = "windows")))]
fn tune_allocator() {
    use tikv_jemalloc_ctl::raw;
    unsafe {
        let _ = raw::write(b"arena.4096.dirty_decay_ms\0", 1000_isize);
        let _ = raw::write(b"arena.4096.muzzy_decay_ms\0", 1000_isize);
        let _ = raw::write(b"arenas.dirty_decay_ms\0", 1000_isize);
        let _ = raw::write(b"arenas.muzzy_decay_ms\0", 1000_isize);
        let _ = raw::write(b"background_thread\0", true);
        if std::env::var_os("BIOROUTER_DEBUG_ALLOC").is_some() {
            let d: isize = raw::read(b"arenas.dirty_decay_ms\0").unwrap_or(-1);
            let m: isize = raw::read(b"arenas.muzzy_decay_ms\0").unwrap_or(-1);
            eprintln!("[alloc] jemalloc active; dirty_decay_ms={d} muzzy_decay_ms={m}");
        }
    }
}

#[cfg(not(all(feature = "jemalloc", not(target_os = "windows"))))]
fn tune_allocator() {}
