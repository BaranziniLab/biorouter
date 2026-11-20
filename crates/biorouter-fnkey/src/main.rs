//! The macOS fn key listener behind Biorouter's dictation gestures (hold fn to
//! talk, double-tap fn for hands-free).
//!
//! Electron cannot register fn as a global shortcut, and Chromium does not
//! deliver it to a page reliably, so the desktop app starts this program and
//! reads what it reports on standard output (`protocol.rs`). It is a listen-only
//! Quartz event tap: it can see key presses but never change, block or make one.
//!
//! The tap needs macOS **Input Monitoring**. The program reads that permission
//! with `CGPreflightListenEventAccess`, which never shows a prompt, and creates
//! the tap only when it is granted, because creating one without it is what
//! makes macOS ask. The prompt comes only from `request`, which the app sends
//! from a person's click in Settings. Started by the app, the program is
//! attributed to it, so the permission is the one listed as Biorouter.
//!
//! It exits when its standard input closes, so it never outlives the app.

// Built everywhere so their tests run on every CI leg; only macOS calls them.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod outbox;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod protocol;

#[cfg(target_os = "macos")]
mod mac;

#[cfg(target_os = "macos")]
fn main() {
    mac::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    // Only macOS lets an app see the fn key; the app never starts this elsewhere.
    eprintln!("biorouter-fnkey is macOS-only");
    std::process::exit(64);
}
