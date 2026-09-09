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
//!   installer copies rather than symlinks, so there is no link to follow — and
//!   nothing to find beside the copy either. Resolution alone does not help
//!   there; the copy has to be told where it came from, which is what
//!   [`biorouter::system::install_origin`] reads back.
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
    // One implementation, in the crate the installer also lives in: the
    // installer has to spell a path the same way this reads one back.
    biorouter::system::real_path(exe)
}

/// [`resolve_exe_path`] applied to the running executable.
///
/// `None` only when the platform cannot report it at all; callers fall back to
/// a `PATH` lookup or to a packaged location rather than failing.
pub fn current_exe_resolved() -> Option<PathBuf> {
    std::env::current_exe().ok().map(|e| resolve_exe_path(&e))
}

/// The daemon's file name on this platform.
pub fn daemon_file_name() -> String {
    format!("biorouterd{}", std::env::consts::EXE_SUFFIX)
}

/// The `biorouterd` that belongs with the `biorouter` at `exe`.
///
/// Two layouts, in order:
///
/// 1. **Beside it.** A development tree and a packaged application both put the
///    two binaries in one directory, so a sibling is the daemon that was built
///    with this CLI rather than whichever one is earliest on `PATH`.
/// 2. **Beside the installation it was copied from.** Windows has no symlink to
///    follow: `biorouter setup-path` copies `biorouter.exe` onto `PATH` and
///    leaves the rest of the application behind, so the sibling check finds
///    nothing. The copy records where it came from
///    ([`biorouter::system::install_origin`]) and the daemon is a sibling
///    *there*.
///
/// `None` when neither holds, so the caller can fall back to a bare name and let
/// the operating system search `PATH`.
///
/// ⚠ `exe` must be the **resolved** path — see [`resolve_exe_path`].
pub fn biorouterd_for(exe: &Path) -> Option<PathBuf> {
    biorouterd_beside(exe).or_else(|| biorouterd_at_origin(exe))
}

/// The daemon sitting in the same directory as `exe`, if it is there.
///
/// Split out from [`biorouterd_for`] so a test can hand it a path instead of
/// being at the mercy of wherever the test binary itself happens to live.
pub fn biorouterd_beside(exe: &Path) -> Option<PathBuf> {
    let beside = exe.parent()?.join(daemon_file_name());
    beside.is_file().then_some(beside)
}

/// The daemon beside the installation `exe` was copied from, if the copy left a
/// breadcrumb naming it and the daemon is still there.
pub fn biorouterd_at_origin(exe: &Path) -> Option<PathBuf> {
    let origin = biorouter::system::install_origin(exe.parent()?)?;
    let candidate = origin.join(daemon_file_name());
    candidate.is_file().then_some(candidate)
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

    /// A fixture shaped like the shipped Windows application: both binaries in
    /// `resources/bin`, the interface bundle beside them in `resources/web`.
    /// Returns the `bin` directory.
    fn application(root: &Path) -> PathBuf {
        let bin = root.join("resources").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(exe_name("biorouter")), b"x").unwrap();
        std::fs::write(bin.join(daemon_file_name()), b"x").unwrap();
        let web = root.join("resources").join("web");
        std::fs::create_dir_all(&web).unwrap();
        std::fs::write(web.join("index.html"), b"<!doctype html>").unwrap();
        bin
    }

    fn exe_name(stem: &str) -> String {
        format!("{stem}{}", std::env::consts::EXE_SUFFIX)
    }

    /// A Windows install: `biorouter.exe` copied onto `PATH` on its own, with
    /// nothing beside it but the breadcrumb naming the application it came from.
    /// Returns the installed executable.
    fn windows_style_install(root: &Path, source_bin: &Path) -> PathBuf {
        let install = root.join("Local").join("Biorouter").join("bin");
        std::fs::create_dir_all(&install).unwrap();
        let exe = install.join(exe_name("biorouter"));
        std::fs::write(&exe, b"x").unwrap();
        biorouter::system::record_install_origin(source_bin, &install).unwrap();
        exe
    }

    /// The Windows half of the defect PR #183 fixed for macOS: there is no
    /// symlink to resolve, because the install is a copy, so the daemon is not
    /// anywhere near the executable that has to start it.
    #[test]
    fn the_daemon_is_found_through_the_breadcrumb_when_nothing_is_beside_the_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let source_bin = application(&tmp.path().join("Application"));
        let exe = windows_style_install(tmp.path(), &source_bin);

        assert!(
            biorouterd_beside(&exe).is_none(),
            "fixture is wrong: the daemon must not be beside the installed copy, \
             or this test cannot fail"
        );
        assert_eq!(
            biorouterd_for(&exe).map(|p| std::fs::canonicalize(p).unwrap()),
            Some(std::fs::canonicalize(source_bin.join(daemon_file_name())).unwrap()),
            "the daemon must be found beside the application the copy came from"
        );
    }

    /// A daemon actually next to the executable is the one that was built with
    /// it, so it wins over anything a breadcrumb names.
    #[test]
    fn a_daemon_beside_the_executable_beats_the_breadcrumb() {
        let tmp = tempfile::tempdir().unwrap();
        let source_bin = application(&tmp.path().join("Application"));
        let exe = windows_style_install(tmp.path(), &source_bin);
        let beside = exe.parent().unwrap().join(daemon_file_name());
        std::fs::write(&beside, b"x").unwrap();

        assert_eq!(
            biorouterd_for(&exe).map(|p| std::fs::canonicalize(p).unwrap()),
            Some(std::fs::canonicalize(&beside).unwrap()),
            "the sibling must be preferred to the recorded installation"
        );
    }

    #[test]
    fn no_daemon_and_no_breadcrumb_resolves_to_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join(exe_name("biorouter"));
        std::fs::write(&exe, b"x").unwrap();

        assert_eq!(
            biorouterd_for(&exe),
            None,
            "with nothing to go on the caller must be allowed to fall back to PATH"
        );
    }

    /// The application was uninstalled, or moved, since the CLI was installed.
    #[test]
    fn a_breadcrumb_naming_a_deleted_installation_resolves_to_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let application_root = tmp.path().join("Application");
        let source_bin = application(&application_root);
        let exe = windows_style_install(tmp.path(), &source_bin);
        std::fs::remove_dir_all(&application_root).unwrap();

        assert_eq!(
            biorouterd_for(&exe),
            None,
            "a stale breadcrumb must not produce a path to a daemon that is gone"
        );
    }
}
