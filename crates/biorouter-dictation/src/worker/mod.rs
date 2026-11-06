//! The worker: the running binary started again with a hidden first argument (DD-4).
//!
//! `ggml_abort()` ends in `abort()` and an `x86-64-v3` build dies with `SIGILL` on an
//! older processor, so the engine never runs inside `biorouterd` or an interactive
//! `biorouter`. Both `main()` functions call [`run_if_invoked`] right after
//! `fork_safety::complete_libnotify_init()`, before the runtime, logging, configuration
//! and the legacy folder move, so a worker reads no configuration and writes no log.

#[cfg(not(br_no_native))]
pub mod self_test;

/// The hidden first argument. Not a clap subcommand, so it never appears in `--help`.
pub const WORKER_MARKER: &str = "__br-dictation-worker";

/// Windows pins the main thread's stack at 1 MiB in the executable header, which is why
/// `run_on_agent_stack` exists; the worker gets its own thread with 8 MiB.
#[cfg(not(br_no_native))]
const WORKER_STACK_BYTES: usize = 8 * 1024 * 1024;

/// The worker's exit codes, which the supervisor and the CI smokes read.
pub mod exit {
    /// stdin end of file or `shutdown`; a passing self test.
    pub const OK: i32 = 0;
    /// A role this binary does not have: unknown, `capture` (answered by `biorouter`'s
    /// own `main()` from PR-25, never here), the `engine` role until PR-10, or any role
    /// in a binary built without the engine. Also a self test usage error.
    pub const NO_SUCH_ROLE: i32 = 2;
    /// The CPU gate refused: MSG-CPU or MSG-CPU-ROSETTA on stderr.
    pub const UNSUPPORTED_CPU: i32 = 20;
    /// Self test: a `--model` or `--vad` file that is missing or not pinned (P-11).
    pub const NOT_PINNED: i32 = 21;
    /// Self test: the ISA, load, detector or decode check failed.
    pub const CHECK_FAILED: i32 = 22;
    /// Engine role: an unreadable or oversized frame from the parent (PR-10).
    pub const PROTOCOL: i32 = 65;
    /// The worker thread panicked.
    pub const PANICKED: i32 = 70;
}

/// Returns only when the first argument is not [`WORKER_MARKER`]; otherwise runs the
/// role and exits the process with its code.
pub fn run_if_invoked() {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(WORKER_MARKER)) {
        return;
    }
    let role = args.next();
    #[cfg_attr(br_no_native, allow(unused_variables))]
    let rest: Vec<std::ffi::OsString> = args.collect();
    let code = match role.as_ref().and_then(|r| r.to_str()) {
        #[cfg(not(br_no_native))]
        Some("self-test") => on_worker_thread(move || self_test::run(&rest)),
        // "engine" arrives with the frame loop in PR-10; "capture" is answered by
        // biorouter's main() before this call from PR-25.
        _ => exit::NO_SUCH_ROLE,
    };
    std::process::exit(code);
}

#[cfg(not(br_no_native))]
fn on_worker_thread(f: impl FnOnce() -> i32 + Send + 'static) -> i32 {
    std::thread::Builder::new()
        .name("br-dictation-worker".into())
        .stack_size(WORKER_STACK_BYTES)
        .spawn(f)
        .ok()
        .and_then(|handle| handle.join().ok())
        .unwrap_or(exit::PANICKED)
}
