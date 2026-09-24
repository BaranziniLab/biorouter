//! Keeping a spawned child's console window off the user's screen (Windows).
//!
//! On Windows a process belongs to a *subsystem*. `biorouterd` is spawned by the
//! Electron main process with `windowsHide: true`, so it has no console of its
//! own — and every console-subsystem child it then starts (a shell, `git`,
//! `node`, `taskkill`, an npm `.cmd` shim) makes Windows allocate a **brand new,
//! visible console window** for the lifetime of that child. Short-lived children
//! therefore appear as the black window that flashes on screen and vanishes,
//! once per tool call.
//!
//! `CREATE_NO_WINDOW` is the documented answer: the child's console handle is
//! not set, while redirected pipes continue to work.
//!
//! ⚠ **This cannot be fixed on the Electron side, so do not try.** `biorouterd.ts`
//! already spawns the daemon with `windowsHide: true` *and* `detached: true` on
//! Windows. Those flags govern the daemon's **own** window; they say nothing
//! about the processes it later spawns, and `detached` is load-bearing for
//! process-group lifetime on Windows, so removing it to "inherit a hidden
//! console" would trade this bug for a worse one. The flag has to be set by
//! whoever spawns the grandchild — which is this crate's callers.
//!
//! The creation flag applies only to the process being started. A helper that
//! launches another console program must set the flag on that child too. The
//! Windows Copilot helper does this for its per-action PowerShell process.
//!
//! # Why this lives in the leaf crate
//!
//! `biorouter` depends on `biorouter-mcp`, which depends on this crate — never
//! the reverse. The hottest spawn sites (`developer__shell`, background jobs,
//! Biorouter Copilot, Agent Drafter) live in `biorouter-mcp`, so they cannot
//! reach a helper defined in `biorouter` without a dependency cycle. That is
//! precisely why they went without the flag for so long, and it is why the
//! primitive belongs here, beside [`crate::environment`] — the other thing every
//! agent-spawned child needs.
//!
//! # Pairing
//!
//! A child spawned on the agent's behalf needs **both** of these, and a site that
//! gets only one is a bug:
//!
//! 1. no console window (this module);
//! 2. none of the daemon's own credentials
//!    ([`crate::environment::strip_daemon_private_env`], issue #57).
//!
//! Keep the two calls adjacent at every spawn site so a reader can see at a
//! glance that neither was forgotten.

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// `CREATE_NO_WINDOW` — "the process is a console application that is being run
/// without a console window".
///
/// From `winbase.h`. Declared here rather than pulled from a `windows-sys`
/// dependency so this crate keeps cross-compiling with no new deps.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Suppress the console window of an asynchronous child.
///
/// A no-op off Windows, so call sites need no `cfg` of their own.
#[allow(unused_variables)]
pub fn no_console_window(command: &mut tokio::process::Command) {
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
}

/// Suppress the console window of a synchronous child.
///
/// A no-op off Windows, so call sites need no `cfg` of their own.
#[allow(unused_variables)]
pub fn no_console_window_std(command: &mut std::process::Command) {
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠ **What this file can and cannot assert, and why.**
    ///
    /// *That the flag is set* is not assertable here: `std` exposes
    /// `creation_flags` as a setter with **no** matching getter on stable, so
    /// there is nothing to read back.
    ///
    /// *That no window appears* is not assertable either, and the obvious test
    /// is actively misleading. A child reports its own console with
    /// `GetConsoleWindow()`, but the answer depends on whether the **test
    /// runner** owns a console: under a terminal it does, under a pipe or in CI
    /// it does not, and in the latter case the child reports "no window" whether
    /// or not the flag was passed. Measured both ways on Windows while writing
    /// this — a spawn-and-look test passes identically with the flag deleted.
    ///
    /// So the guarantee is split in two, deliberately:
    ///
    /// * that every spawn site *calls* one of these helpers is pinned by the
    ///   census test `crates/biorouter-mcp/tests/no_console_window_census.rs`;
    /// * that the flag we pass does not break the child is pinned below.
    ///
    /// The second matters more than it looks. `CREATE_NO_WINDOW` is one of a
    /// family of creation flags, and a neighbouring one — `DETACHED_PROCESS` —
    /// also hides the window while *severing the child's standard handles*.
    /// Swapping one for the other would leave every tool call silently returning
    /// empty output, which is a far worse bug than the flash. This test fails if
    /// that ever happens.
    #[tokio::test]
    async fn a_flagged_child_still_streams_its_output_back() {
        #[cfg(windows)]
        let (program, args): (&str, &[&str]) = ("cmd", &["/C", "echo", "biorouter"]);
        #[cfg(not(windows))]
        let (program, args): (&str, &[&str]) = ("echo", &["biorouter"]);

        let mut command = tokio::process::Command::new(program);
        command.args(args);
        no_console_window(&mut command);

        let output = command.output().await.expect("the child must still spawn");

        assert!(
            output.status.success(),
            "a flagged child must still run: {output:?}"
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("biorouter"),
            "stdout must still reach the parent — a flag that detaches the child \
             would hide the window and silently empty every tool result: {output:?}"
        );
    }

    /// The same for the synchronous half, which is a separate `std` type and so
    /// a separate call into the OS.
    #[test]
    fn a_flagged_sync_child_still_streams_its_output_back() {
        #[cfg(windows)]
        let (program, args): (&str, &[&str]) = ("cmd", &["/C", "echo", "biorouter"]);
        #[cfg(not(windows))]
        let (program, args): (&str, &[&str]) = ("echo", &["biorouter"]);

        let mut command = std::process::Command::new(program);
        command.args(args);
        no_console_window_std(&mut command);

        let output = command.output().expect("the child must still spawn");

        assert!(output.status.success(), "a flagged child must still run");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("biorouter"),
            "stdout must still reach the parent: {output:?}"
        );
    }
}
