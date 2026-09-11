mod commands;
mod configuration;
mod error;
mod logging;
mod openapi;
mod routes;
mod state;
mod tunnel;
// `main.rs` re-declares the module tree rather than using the lib, so the bin's
// dead-code analysis walks only from `main` and does not see the accessors the
// lib's own tests use (`observer_count`, `is_closed`, `next_seq`, `session_id`).
// Same situation, same remedy, as `mod workspace` below.
#[allow(dead_code)]
mod turn_stream;
// BR-71 Task 6: the daemon compiles the turn runner, but nothing reachable from
// `main` calls it until Task 8 switches `/reply` onto it. `main.rs` re-declares
// the module tree rather than using the lib, so the bin's dead-code analysis
// walks only from `main` and would otherwise reject the whole runner. Drop this
// `allow` with the Task 8 cutover.
#[allow(dead_code)]
mod workspace;

/// `main.rs` re-declares the module tree, so `cargo test` runs a *second* copy
/// of the route tests inside the `biorouterd` binary. Sandbox it exactly like
/// the lib's copy, or that copy alone keeps writing to the real `sessions.db`.
#[cfg(test)]
mod test_sandbox;

use biorouter::config::paths::Paths;
use biorouter_mcp::{
    mcp_server_runner::{serve, McpCommand},
    AutoVisualiserRouter, ComputerControllerServer, DeveloperServer, MemoryServer,
};
use clap::{Parser, Subcommand};

// Tuned jemalloc as the global allocator for the long-running daemon (default-on
// `jemalloc` feature). Returns freed pages to the OS far more readily than the
// system allocator under the agent's per-turn allocation churn.
#[cfg(all(feature = "jemalloc", not(target_os = "windows")))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the agent server
    Agent {
        /// Stop when process PID is no longer this daemon's parent.
        ///
        /// `biorouter serve` passes its own pid, so a daemon it started cannot
        /// outlive it even when it is killed outright. Opt-in: the desktop app
        /// and a hand-started daemon do not pass it. Unix only.
        #[arg(long, value_name = "PID")]
        exit_with_parent: Option<u32>,
    },
    /// Run the MCP server
    Mcp {
        #[arg(value_parser = clap::value_parser!(McpCommand))]
        server: McpCommand,
    },
}

/// ⚠ **Not `#[tokio::main]`.** That macro builds a runtime with tokio's default
/// worker stack, which is `std::thread`'s 2 MiB, and a `workspace__subagent`
/// spawn polls the CHILD's `Agent::reply` inline on the PARENT's stack. Two
/// agent state machines on one 2 MiB worker overflowed it: every delegation
/// took this daemon down about 2.3 s in, and Electron does not respawn it, so
/// the app read "Backend disconnected" and never came back.
///
/// See `biorouter::execution::runtime` for the measurement and the size.
fn main() -> anyhow::Result<()> {
    biorouter::execution::runtime::build_agent_runtime()?.block_on(async_main())
}

async fn async_main() -> anyhow::Result<()> {
    // BR-69: if this process was re-exec'd as the shell-sandbox helper (hidden
    // `__br-sandbox` marker, Linux only), apply the in-process Landlock/seccomp
    // restrictions and `execve` the target program. Never returns in that case;
    // a normal invocation falls straight through. Must run before `Cli::parse`,
    // which would otherwise reject the hidden marker as an unknown subcommand.
    biorouter_mcp::run_shell_sandbox_helper_if_invoked();
    tune_allocator();
    let cli = Cli::parse();

    match cli.command {
        Commands::Agent { exit_with_parent } => {
            commands::agent::run(exit_with_parent).await?;
        }
        Commands::Mcp { server } => {
            logging::setup_logging(Some(&format!("mcp-{}", server.name())))?;
            match server {
                McpCommand::AutoVisualiser => serve(AutoVisualiserRouter::new()).await?,
                McpCommand::ComputerController => serve(ComputerControllerServer::new()).await?,
                McpCommand::Memory => serve(MemoryServer::new()).await?,
                McpCommand::Developer => {
                    let bash_env = Paths::config_dir().join(".bash_env");
                    serve(
                        DeveloperServer::new()
                            .extend_path_with_shell(true)
                            .bash_env_file(Some(bash_env)),
                    )
                    .await?
                }
            }
        }
    }

    Ok(())
}

/// Lower jemalloc's dirty/muzzy decay so freed pages return to the OS within
/// ~1s instead of being retained (jemalloc's own defaults retain aggressively).
/// Applied to all current arenas (`MALLCTL_ARENAS_ALL` = 4096) and as the
/// default for future arenas. Errors are ignored (`background_thread` is
/// unsupported on macOS). Set `BIOROUTER_DEBUG_ALLOC=1` to print applied values.
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
