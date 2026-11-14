//! Updating a `biorouter serve` installation from its own browser page (SD-19,
//! owner ruling 2026-11-14).
//!
//! `biorouter serve` runs on a server nobody sits at, so the desktop app's
//! updater (electron-updater, in the Electron main process) never reaches it.
//! This module is the whole of the replacement, shared by the two processes that
//! take part:
//!
//! - **the daemon** (`biorouterd`, through `routes::serve_update` in
//!   `biorouter-server`) checks the release feed, downloads the file for its
//!   install kind, verifies the release's published SHA-256 digest, and, when
//!   the person in the browser asks, installs it ([`apply`]) and exits with
//!   [`RESTART_EXIT_CODE`];
//! - **`biorouter serve`** (in `biorouter-cli`) reads the [`records::RestartPlan`]
//!   the daemon left, re-executes itself on the same address and token, and rolls
//!   the installation back ([`apply::rollback`]) when the new version does not
//!   come up.
//!
//! The two never talk except through those records and the exit code, which is
//! why the shared code lives here rather than in either binary.
//!
//! ⚠ **Only a daemon `serve` started with the restart contract offers any of
//! this.** `serve` declares it in [`SUPERVISOR_ENV`]; the desktop app's daemon,
//! a hand-started `biorouterd` and a `serve` from before this contract never set
//! it, so they never install anything and never exit with the restart code.

pub mod apply;
pub mod download;
pub mod feed;
pub mod install;
pub mod records;
pub mod updater;

/// The exit status a `serve` daemon ends with to ask `serve` to restart it on
/// the version now installed. `EX_TEMPFAIL` from `sysexits.h`: "try again", which
/// is what the supervisor does. Any other status keeps its old meaning.
pub const RESTART_EXIT_CODE: i32 = 75;

/// Set by `biorouter serve` on the daemon it starts, to [`SUPERVISOR_CONTRACT`]:
/// "I handle [`RESTART_EXIT_CODE`] by re-executing myself". A daemon without it
/// never offers a one-click update, because nothing would bring it back.
pub const SUPERVISOR_ENV: &str = "BIOROUTER_SERVE_SUPERVISOR";

/// The one contract [`SUPERVISOR_ENV`] can name today. Versioned so a later
/// change to the handshake can be told from this one.
pub const SUPERVISOR_CONTRACT: &str = "restart-v1";

/// The version of the code running in this process, as releases spell it.
pub fn running_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Is `latest` strictly newer than `current`?
///
/// Dotted numeric segments, compared left to right; a missing segment is 0 and
/// a leading `v` is ignored. A pre-release or build suffix (`-rc.1`, `+abc`) is
/// dropped rather than ordered: releases are plain `x.y.z`, and a feed that
/// offers anything else is offering something this updater does not install.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let a = numeric_segments(latest);
    let b = numeric_segments(current);
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

/// `v1.94.1` → `1.94.1`; `1.95.0-rc.1` → `1.95.0`.
pub fn normalize_version(raw: &str) -> String {
    let trimmed = raw.trim();
    let without_v = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);
    without_v
        .split(['-', '+'])
        .next()
        .unwrap_or_default()
        .to_string()
}

fn numeric_segments(raw: &str) -> Vec<u64> {
    normalize_version(raw)
        .split('.')
        .map(|part| part.trim().parse::<u64>().unwrap_or(0))
        .collect()
}

/// Read the version a Biorouter binary reports, `biorouter 1.95.0` or
/// `biorouterd 1.95.0`, from its `--version` output: the last word that parses
/// as a dotted version.
pub fn version_from_output(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .rev()
        .map(normalize_version)
        .find(|word| {
            !word.is_empty()
                && word.split('.').count() >= 2
                && word.split('.').all(|p| p.parse::<u64>().is_ok())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_compares_numerically_not_as_text() {
        assert!(is_newer("1.10.0", "1.9.9"));
        assert!(is_newer("v1.95.0", "1.94.1"));
        assert!(is_newer("2.0", "1.99.99"));
        assert!(!is_newer("1.94.1", "1.94.1"));
        assert!(!is_newer("1.94.0", "1.94.1"));
        assert!(!is_newer("1.94", "1.94.0"));
    }

    #[test]
    fn a_suffix_is_dropped_rather_than_ordered() {
        assert_eq!(normalize_version("v1.95.0-rc.1"), "1.95.0");
        assert_eq!(normalize_version(" 1.95.0+build.7 "), "1.95.0");
        assert!(!is_newer("1.94.1-rc.9", "1.94.1"));
    }

    #[test]
    fn the_version_is_read_from_either_binary_s_banner() {
        assert_eq!(
            version_from_output("biorouterd 1.95.0\n").as_deref(),
            Some("1.95.0")
        );
        assert_eq!(
            version_from_output("biorouter 1.94.1").as_deref(),
            Some("1.94.1")
        );
        assert_eq!(version_from_output("no version here"), None);
    }
}
