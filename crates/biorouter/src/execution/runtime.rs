//! Tokio runtime construction for every process that can host an agent turn.
//!
//! # Why this module exists
//!
//! A `workspace__subagent` spawn polls the **child's** `Agent::reply` inline, on
//! the parent's stack — `execute_subagent` → `run_complete_subagent_task` →
//! `get_agent_messages` → `Agent::reply`. Nothing recurses; two independent
//! agent state machines simply share one worker thread's stack. Measured on a
//! debug `biorouterd` (crash report `biorouterd-2026-08-09-132353.ips`): 146
//! frames, the parent's chain alone accounting for 113 of them, ending in
//! `std::sys::pal::unix::stack_overflow::imp::signal_handler` → `abort` on a
//! `tokio-runtime-worker` thread. Every delegation took the daemon down about
//! 2.3 s in, and Electron does not respawn it, so the app read "Backend
//! disconnected" and never recovered.
//!
//! Tokio leaves [`tokio::runtime::Builder::thread_stack_size`] unset by default,
//! so its workers inherit `std::thread`'s default — 2 MiB, or whatever
//! `RUST_MIN_STACK` says. That is the whole of the bug: the same binary and the
//! same delegation succeed under `RUST_MIN_STACK=67108864`, and a release build
//! (thinner frames) succeeds unmodified.
//!
//! # Why the size is set, not the nesting removed
//!
//! Moving the child onto its own `tokio::spawn` would halve the depth, and it
//! was rejected for three reasons:
//!
//! 1. It does not actually clear the hazard. The parent's chain **on its own**
//!    reached 113 of the 146 frames — roughly three quarters of a 2 MiB stack
//!    before a subagent existed. Anything that deepens a single agent's tool
//!    path (a `render_dashboard` panel calling another figure tool, a knowledge
//!    macro's sub-agent loop) walks back into the same abort with no subagent
//!    involved.
//! 2. `tokio::spawn` does not inherit the caller's task-local scope, which
//!    `subagent_tool` already depends on: `max_pending_subagents()` is read in
//!    the requesting task *precisely because* the detached path cannot see a
//!    per-task override. A blocking spawn moved behind `tokio::spawn` would
//!    silently start reading process defaults.
//! 3. The blocking spawn is load-bearing. The parent's turn is supposed to block
//!    on the child, and cancellation is threaded through it (`run_token`, the
//!    turn lease, `ActiveWorkGuard`). Reproducing that around a `JoinHandle`
//!    means re-deriving abort-on-drop, panic propagation and lease ordering — a
//!    behaviour change, in service of a smaller number.
//!
//! Setting the stack size is one line per process, changes no semantics, and is
//! applied **identically in debug and release** so a test can never pass on a
//! configuration users do not run.

/// Stack size, in bytes, for the worker and blocking threads of any process
/// that can host an agent turn.
///
/// 16 MiB — 8× tokio's inherited default, and the same figure
/// `biorouter-acp`'s test runtimes already use. The measured overflow needed
/// slightly more than 2 MiB for 146 debug frames (~14 KiB/frame), so this
/// carries roughly 1100 frames: about 7× the deepest chain observed, which is
/// the headroom a *debug* build wants, since that is where frames are fattest.
///
/// The cost is address space, not memory: thread stacks are reserved lazily and
/// only the pages actually touched are ever committed.
pub const AGENT_WORKER_STACK_SIZE: usize = 16 * 1024 * 1024;

/// Escape hatch for a machine that finds a chain deeper than the default
/// covers. Read once, when the runtime is built.
///
/// ⚠ It can only ever RAISE the value. A lowerable knob is a way to reintroduce
/// the abort this module exists to prevent, and it would buy nothing: the
/// default's cost is reserved address space, which is free until touched.
pub const AGENT_WORKER_STACK_SIZE_ENV: &str = "BIOROUTER_WORKER_STACK_SIZE";

/// The stack size [`build_agent_runtime`] will ask for: the default, or a
/// larger value from [`AGENT_WORKER_STACK_SIZE_ENV`].
pub fn worker_stack_size() -> usize {
    std::env::var(AGENT_WORKER_STACK_SIZE_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .unwrap_or(0)
        .max(AGENT_WORKER_STACK_SIZE)
}

/// The multi-threaded runtime a Biorouter host process should run on.
///
/// Replaces `#[tokio::main]`, which offers no way to size worker stacks. Worker
/// threads are themselves spawned through tokio's blocking pool, so the one
/// `thread_stack_size` call covers both pools — the crashing thread in the
/// report above is a `tokio-runtime-worker` launched via
/// `blocking::pool::Spawner::spawn_thread`.
pub fn build_agent_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(worker_stack_size())
        .build()
}

/// Run a host process's body on a thread sized for agent work, and give back
/// what it returned.
///
/// # Why the main thread is not good enough
///
/// [`build_agent_runtime`] sizes the runtime's **worker** threads. It cannot
/// size the thread that calls `block_on`, because `block_on` drives the future
/// on the *calling* thread — and for both `biorouter` and `biorouterd` that is
/// the process's main thread.
///
/// ⚠ **On Windows the main thread's stack is fixed in the executable header**
/// (`SizeOfStackReserve`, 1 MiB by default) and no runtime call can change it.
/// Linux gives 8 MiB and macOS 8 MiB, which is why this was invisible off
/// Windows for so long. The CLI's `async_main` future is large enough on its own
/// that materialising it blew that 1 MiB immediately: **every** invocation of
/// `biorouter.exe` — including `--version` — died with
///
/// ```text
/// thread 'main' has overflowed its stack
/// ```
///
/// Measured on Windows Server 2025: a debug `biorouter.exe --version` aborted,
/// and the *same binary* with `editbin /STACK:16777216` applied printed its
/// version and ran `session list` correctly. So it is purely stack reservation,
/// not a runaway recursion.
///
/// Spawning a thread is preferred over a linker flag (`/STACK`) because it needs
/// no per-target `rustflags`, applies identically to every toolchain and
/// cross-build, and reuses [`worker_stack_size`] — so the escape hatch and the
/// documented size stay in one place instead of two.
///
/// A panic inside `body` is re-raised on the caller's thread rather than being
/// converted into an error, so panic output and exit behaviour are unchanged.
pub fn run_on_agent_stack<T, F>(body: F) -> std::io::Result<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let handle = std::thread::Builder::new()
        .name("biorouter-main".to_string())
        .stack_size(worker_stack_size())
        .spawn(body)?;
    match handle.join() {
        Ok(value) => Ok(value),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read the *actual* stack size the OS gave the calling thread.
    ///
    /// This is what makes the test below a test rather than a restatement of
    /// the constant: it asks the operating system what the worker thread got,
    /// so a `build_agent_runtime` that forgot to pass the size — or passed it to
    /// a builder whose threads are spawned elsewhere — fails.
    #[cfg(unix)]
    fn current_thread_stack_size() -> Option<usize> {
        #[cfg(target_vendor = "apple")]
        {
            Some(unsafe { libc::pthread_get_stacksize_np(libc::pthread_self()) })
        }
        #[cfg(all(target_os = "linux", target_env = "gnu"))]
        {
            unsafe {
                let mut attr: libc::pthread_attr_t = std::mem::zeroed();
                if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) != 0 {
                    return None;
                }
                let mut size: libc::size_t = 0;
                let rc = libc::pthread_attr_getstacksize(&attr, &mut size);
                libc::pthread_attr_destroy(&mut attr);
                if rc == 0 {
                    Some(size)
                } else {
                    None
                }
            }
        }
        #[cfg(not(any(target_vendor = "apple", all(target_os = "linux", target_env = "gnu"))))]
        {
            None
        }
    }

    /// The Windows analogue of the probe above.
    ///
    /// ⚠ This platform had **no coverage at all** here, and that is exactly how
    /// the main-thread gap survived: `build_agent_runtime` was tested on unix,
    /// where the main thread starts with 8 MiB and nothing was ever short of
    /// stack, while Windows gives it 1 MiB and `biorouter.exe` could not print
    /// its own `--version`.
    ///
    /// `GetCurrentThreadStackLimits` reports the reserved span of the calling
    /// thread's stack, which is the number `stack_size` asked for.
    #[cfg(windows)]
    fn current_thread_stack_size() -> Option<usize> {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentThreadStackLimits(low: *mut usize, high: *mut usize);
        }
        let (mut low, mut high) = (0usize, 0usize);
        unsafe { GetCurrentThreadStackLimits(&mut low, &mut high) };
        high.checked_sub(low).filter(|size| *size > 0)
    }

    /// **The regression guard for `biorouter.exe` failing to start on Windows.**
    ///
    /// A thread from [`run_on_agent_stack`] must really carry the larger stack.
    /// Asserted by asking the OS, for the same reason the worker test does: a
    /// test that actually overflowed would abort the binary rather than fail.
    #[test]
    #[cfg(any(unix, windows))]
    fn the_host_body_really_gets_the_larger_stack() {
        let Some(baseline) = std::thread::spawn(current_thread_stack_size)
            .join()
            .expect("probe thread")
        else {
            eprintln!("stack size is not readable on this target; skipping");
            return;
        };
        // The same negative control as the worker test: without it, a platform
        // that handed every thread a huge stack would pass this regardless.
        assert!(
            baseline < AGENT_WORKER_STACK_SIZE,
            "a default thread already has {baseline} bytes, so this test cannot              distinguish a sized host thread from an unsized one"
        );

        let measured = run_on_agent_stack(current_thread_stack_size)
            .expect("the host thread spawns")
            .expect("stack size is readable on the sized thread too");

        assert!(
            measured >= AGENT_WORKER_STACK_SIZE,
            "the host body ran on a {measured}-byte stack, but agent work needs              at least {AGENT_WORKER_STACK_SIZE}. On Windows the main thread is              pinned to 1 MiB by the executable header, so the body must run on a              thread this function sized."
        );
    }

    /// A value comes back, and a panic is still a panic.
    #[test]
    fn the_host_body_returns_its_value() {
        assert_eq!(run_on_agent_stack(|| 6 * 7).expect("spawns"), 42);
    }

    #[test]
    #[should_panic(expected = "the body panicked")]
    fn a_panic_in_the_body_is_re_raised_rather_than_swallowed() {
        let _ = run_on_agent_stack(|| panic!("the body panicked"));
    }

    /// **The regression guard for the subagent SIGABRT.**
    ///
    /// A worker thread of [`build_agent_runtime`] must really have the larger
    /// stack. Asserting the OS-reported size rather than overflowing one is
    /// deliberate: a test that actually overflowed would abort the whole test
    /// binary instead of failing, and would be sized against whatever the
    /// compiler happened to inline that day.
    #[test]
    #[cfg(unix)]
    fn a_worker_thread_really_gets_the_larger_stack() {
        let Some(baseline) = std::thread::spawn(current_thread_stack_size)
            .join()
            .expect("probe thread")
        else {
            eprintln!("stack size is not readable on this target; skipping");
            return;
        };
        // The negative control. Without it, a platform that reported a huge
        // stack for every thread would let the assertion below pass on a
        // runtime built with no `thread_stack_size` at all.
        assert!(
            baseline < AGENT_WORKER_STACK_SIZE,
            "a default thread already has {baseline} bytes, so this test cannot \
             distinguish a configured runtime from an unconfigured one"
        );

        let rt = build_agent_runtime().expect("runtime builds");
        let measured = rt
            .block_on(async { tokio::spawn(async { current_thread_stack_size() }).await })
            .expect("worker task")
            .expect("worker stack size is readable");

        assert!(
            measured >= AGENT_WORKER_STACK_SIZE,
            "a tokio worker got {measured} bytes, not the {AGENT_WORKER_STACK_SIZE} \
             `build_agent_runtime` asks for. Two agent state machines share one \
             worker stack during a subagent spawn; at the inherited {baseline}-byte \
             default a debug build aborts partway through the child's first turn."
        );
    }

    /// The override raises and cannot lower — the direction is the whole point,
    /// so both are asserted.
    #[test]
    fn the_override_can_only_raise_the_stack_size() {
        // Serialised against the whole process via `env-lock`, as every other
        // env-reading test in this crate is.
        let _guard = env_lock::lock_env([(
            AGENT_WORKER_STACK_SIZE_ENV,
            Some(&format!("{}", AGENT_WORKER_STACK_SIZE * 2)),
        )]);
        assert_eq!(worker_stack_size(), AGENT_WORKER_STACK_SIZE * 2);
        drop(_guard);

        let _guard = env_lock::lock_env([(AGENT_WORKER_STACK_SIZE_ENV, Some("65536"))]);
        assert_eq!(
            worker_stack_size(),
            AGENT_WORKER_STACK_SIZE,
            "a smaller override must not shrink the stack back into the abort"
        );
        drop(_guard);

        let _guard = env_lock::lock_env([(AGENT_WORKER_STACK_SIZE_ENV, Some("not a number"))]);
        assert_eq!(worker_stack_size(), AGENT_WORKER_STACK_SIZE);
    }
}
