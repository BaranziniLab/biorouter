//! Stops the subprocess group in flight when a terminating signal arrives, so a cancelled command leaves no
//! orphan: the group gets SIGTERM, a short grace, then SIGKILL, and only then does this process end.

use std::sync::Once;
use std::sync::atomic::{AtomicI32, Ordering};

static ACTIVE_GROUP: AtomicI32 = AtomicI32::new(0);
static INSTALL: Once = Once::new();
const FORWARDED: [i32; 3] = [libc::SIGHUP, libc::SIGINT, libc::SIGTERM];
const GRACE_POLLS: u32 = 40;
const GRACE_POLL: libc::timespec = libc::timespec {
    tv_sec: 0,
    tv_nsec: 50_000_000,
};

/// Makes SIGHUP, SIGINT and SIGTERM stop the tracked subprocess group before they end this process. Only
/// the CLI's sandbox path calls it, so a host application that loads the library keeps its own handlers.
pub(super) fn forward_to_active_group() {
    INSTALL.call_once(install);
}

/// Marks one subprocess group as the one a terminating signal must reach. Dropping it clears the mark.
/// One group is tracked at a time, which matches how commands run their subprocesses.
pub(super) struct ActiveGroup;

impl ActiveGroup {
    pub(super) fn track(process_group: i32) -> Self {
        ACTIVE_GROUP.store(process_group, Ordering::SeqCst);
        Self
    }
}

impl Drop for ActiveGroup {
    fn drop(&mut self) {
        ACTIVE_GROUP.store(0, Ordering::SeqCst);
    }
}

fn install() {
    let handler = forward as extern "C" fn(i32) as libc::sighandler_t;
    for signal in FORWARDED {
        let previous = unsafe { libc::signal(signal, handler) };
        if previous == libc::SIG_IGN {
            unsafe { libc::signal(signal, libc::SIG_IGN) };
        }
    }
}

extern "C" fn forward(signal: i32) {
    let group = ACTIVE_GROUP.load(Ordering::SeqCst);
    if group > 0 {
        stop_group(group);
    }
    unsafe {
        libc::signal(signal, libc::SIG_DFL);
        libc::raise(signal);
    }
}

/// Uses only async-signal-safe calls, because it runs inside the signal handler.
fn stop_group(group: i32) {
    unsafe { libc::kill(-group, libc::SIGTERM) };
    for _ in 0..GRACE_POLLS {
        unsafe { libc::waitpid(group, std::ptr::null_mut(), libc::WNOHANG) };
        if unsafe { libc::kill(-group, 0) } != 0 {
            return;
        }
        unsafe { libc::nanosleep(&GRACE_POLL, std::ptr::null_mut()) };
    }
    unsafe { libc::kill(-group, libc::SIGKILL) };
}

#[cfg(test)]
#[path = "interrupt_tests.rs"]
mod tests;
