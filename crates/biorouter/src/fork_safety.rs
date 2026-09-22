//! Keeping a child alive through `fork` on macOS.
//!
//! Most spawns never fork: std starts the child with `posix_spawn`, which runs
//! no code in it. std falls back to `fork` + `exec` when it cannot use
//! `posix_spawn` — for a bare program name under an overridden `PATH`, a
//! `pre_exec` closure, a uid or gid — and on macOS `fork` runs libSystem's
//! child-side handlers before `exec`. libnotify's (`_notify_fork_child`) waits
//! on the once-gate guarding libnotify's process-wide state (`_os_alloc_once`).
//! If another thread was inside that one-time initialisation when `fork` ran,
//! the child inherits a gate held by a thread it does not have, and libplatform
//! kills it before `exec`: the crash report reads "BUG IN CLIENT OF
//! LIBPLATFORM: os_once_t is corrupt" and "crashed on child side of fork
//! pre-exec", and the parent sees `unix_wait_status(9)` with an empty stderr.
//!
//! The initialisation runs on the process's first libnotify call, which is not
//! ours to schedule: Security.framework makes it while reading trust settings,
//! which is how a `reqwest` client built on one test thread killed an extension
//! probe child forked on another in this crate's lib test binary (found by
//! interposing `notify_register_check`, 2026-09-21).

/// Finish libnotify's one-time process-wide initialisation now, on the calling
/// thread, so no later `fork` can land inside it.
///
/// Call it first in `main`, before the runtime or any other thread exists, or
/// from a test binary's constructor. It is idempotent, and on every platform
/// other than macOS it does nothing.
///
/// What the call does, read from the disassembly of `libsystem_notify` on
/// macOS 26.6: `notify_is_valid_token` runs `_os_alloc_once` for libnotify's
/// 600-byte globals, whose initialiser only sets up in-memory tables (no IPC,
/// no connection to `notifyd`), then looks token 0 up under libnotify's lock
/// and answers whether it is registered. It registers, posts and cancels
/// nothing.
///
/// Measured 2026-09-21 on macOS 26.6, one fresh process per run and eight
/// processes at a time, with eight threads making the process's first libnotify
/// call as a child was forked:
///
/// - a C reproduction lost 355 of 1600 forked children without this call and
///   0 of 1600 with it; `posix_spawn` lost 0 of 1600;
/// - the lib test binary, with an extension probe spawned by its bare name so
///   that it forked, lost 101 of 320 probe children without the call in its
///   constructor and 0 of 320 with it, runs in which every probe still forked
///   (a `pthread_atfork` child handler ran in each).
///
/// Called from `biorouterd`'s and `biorouter`'s `main` and from the `biorouter`
/// lib test binary's constructor. Production still forks wherever std cannot
/// use `posix_spawn`, for instance a stdio extension whose command
/// `resolve_command` cannot find is spawned by its bare name.
pub fn complete_libnotify_init() {
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            // <notify.h>, macOS 10.10+, in libSystem, which every macOS binary links.
            fn notify_is_valid_token(val: std::ffi::c_int) -> bool;
        }
        // SAFETY: takes a plain integer and has no preconditions; any value is
        // a valid argument, and an unknown token only yields `false`.
        let _ = unsafe { notify_is_valid_token(0) };
    }
}
