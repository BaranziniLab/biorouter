//! Where this executable really lives, as opposed to how it was invoked.
//!
//! `std::env::current_exe()` answers a different question on each platform, and
//! the difference is not cosmetic — it decides whether a command can find the
//! files that were installed *next to* the binary.
//!
//! - **macOS** returns the path the process was launched through
//!   (`_NSGetExecutablePath`). The in-app "Biorouter CLI Update" card and
//!   `biorouter setup-path` both install the CLI as a **symlink**
//!   (`crates/biorouter/src/system.rs`, `#[cfg(unix)]`), so a user who types
//!   `biorouter` gets `~/.local/bin/biorouter` — and every sibling derived from
//!   it (`../web`, `./biorouterd`) lands in their home directory instead of
//!   inside the application bundle.
//! - **Linux** reads `/proc/self/exe`, which the kernel has already resolved.
//! - **Windows** returns the real module path from `GetModuleFileNameW`, and the
//!   installer copies rather than symlinks, so there is no link to follow.
//!
//! So the resolution is a no-op on two of the three platforms — which is
//! precisely why it is unconditional rather than `#[cfg(target_os = "macos")]`.
//! A `cfg` here would mean the fix is only exercised on the one platform that
//! happens to need it today, and a Linux container with a hand-made symlink, or
//! a future Windows shim, would quietly regress.
//!
//! ⚠ `canonicalize` on Windows returns a **verbatim** path (`\\?\C:\…`). That
//! string is not universally accepted: it reaches an error message the user
//! reads and the `BIOROUTER_SERVE_UI` environment variable a child process
//! parses. `dunce::simplified` strips the prefix whenever the plain form means
//! the same thing, and is the identity function everywhere else.

use std::path::{Path, PathBuf};

/// The real path of `exe`, with symlinks followed and Windows' verbatim prefix
/// stripped.
///
/// Falls back to the path as given when it cannot be resolved — a binary that
/// has been deleted out from under a running process, or a relative path with
/// no matching working directory, is a worse thing to fail on than to guess at,
/// because every caller here treats the result as a *hint* and checks the file
/// it derives before using it.
pub fn resolve_exe_path(exe: &Path) -> PathBuf {
    std::fs::canonicalize(exe)
        .map(|p| dunce::simplified(&p).to_path_buf())
        .unwrap_or_else(|_| exe.to_path_buf())
}

/// [`resolve_exe_path`] applied to the running executable.
///
/// `None` only when the platform cannot report it at all; callers fall back to
/// a `PATH` lookup or to a packaged location rather than failing.
pub fn current_exe_resolved() -> Option<PathBuf> {
    std::env::current_exe().ok().map(|e| resolve_exe_path(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug this module exists for: through the symlink `setup-path`
    /// creates, the siblings of the executable are the siblings of the *link*.
    #[cfg(unix)]
    #[test]
    fn a_symlink_resolves_to_the_binary_it_points_at() {
        let tmp = tempfile::tempdir().unwrap();
        let real_dir = tmp.path().join("Resources").join("bin");
        std::fs::create_dir_all(&real_dir).unwrap();
        let real = real_dir.join("biorouter");
        std::fs::write(&real, b"#!/bin/sh\n").unwrap();

        let link_dir = tmp.path().join("local").join("bin");
        std::fs::create_dir_all(&link_dir).unwrap();
        let link = link_dir.join("biorouter");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        assert_eq!(
            resolve_exe_path(&link),
            std::fs::canonicalize(&real).unwrap(),
            "the link must resolve to the installed binary, not to its own directory"
        );
        assert_ne!(
            resolve_exe_path(&link).parent(),
            Some(link_dir.as_path()),
            "resolving must move the parent directory too — that is the whole point"
        );
    }

    /// Runs on every platform, and is only capable of failing on one of them:
    /// on Windows `canonicalize` returns `\\?\C:\…`, which would then be
    /// printed at the user and handed to a child process in an environment
    /// variable. Dropping `dunce::simplified` turns this red.
    #[test]
    fn the_resolved_path_is_never_a_windows_verbatim_path() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("biorouter");
        std::fs::write(&exe, b"x").unwrap();

        let resolved = resolve_exe_path(&exe);
        assert!(
            !resolved.to_string_lossy().starts_with(r"\\?\"),
            "a verbatim path leaks into the serve error text and into \
             BIOROUTER_SERVE_UI: {}",
            resolved.display()
        );
        assert!(resolved.is_absolute());
    }

    /// The string-level half of the same rule, where the prefix actually exists.
    #[cfg(windows)]
    #[test]
    fn a_verbatim_prefix_is_stripped_at_the_string_level() {
        assert_eq!(
            dunce::simplified(Path::new(r"\\?\C:\Program Files\Biorouter\biorouter.exe")),
            Path::new(r"C:\Program Files\Biorouter\biorouter.exe")
        );
    }

    /// A path that cannot be resolved is passed through rather than swallowed,
    /// so the caller's own "is this file here?" check is what decides.
    #[test]
    fn an_unresolvable_path_is_returned_unchanged() {
        let missing = Path::new("/definitely/not/here/biorouter");
        assert_eq!(resolve_exe_path(missing), missing.to_path_buf());
    }
}
