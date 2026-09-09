//! `biorouter serve` — run Biorouter and reach it from a browser.
//!
//! The command starts `biorouterd`, points it at the built interface bundle,
//! and prints the URL to open. The daemon serves the interface on its own
//! origin, so there is no proxy in the path and the WebSocket-backed features
//! work exactly as they do in the desktop application. See
//! `docs/deployment/serve-architecture.md`.
//!
//! # Why this spawns a daemon rather than being one
//!
//! `biorouter-cli` carries no dependency on `biorouter-server`, deliberately —
//! the one place that came closest to needing it duplicates a header constant
//! instead (`commands/session_watch.rs`). Running the server in-process would
//! erase that boundary and merge two large binaries into one. Spawning also
//! matches what the desktop application already does, so the product has one
//! supervision model rather than two.
//!
//! # Why the child's standard input is closed
//!
//! It is where the daemon would read a proof-of-user digest (issue #56, DR-16),
//! and browser-served Biorouter deliberately installs none: a browser session
//! cannot change its model or provider, so the tier implied by the operator's
//! choice holds for every session in that daemon. Decision SD-1 in
//! `docs/deployment/serve-decisions.md` has the reasoning. This is a deliberate
//! configuration, not a missing feature — but it does mean the daemon a `serve`
//! session talks to is less capable than the one the desktop application
//! starts, and anything assuming otherwise is wrong.

use crate::commands::exe_path::{biorouterd_for, current_exe_resolved, daemon_file_name};
use anyhow::{bail, Context, Result};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Not 3000. That is `biorouterd`'s own default, and the old `biorouter web`
/// used it too — a default that collides with the daemon this command starts is
/// a support question waiting to happen.
pub const DEFAULT_PORT: u16 = 8765;

/// How long to wait for the daemon to answer before giving up.
const READY_TIMEOUT: Duration = Duration::from_secs(60);

/// Run the browser-served interface.
#[allow(clippy::too_many_arguments)]
pub async fn handle_serve(
    host: String,
    port: u16,
    token: Option<String>,
    no_token: bool,
    web_dir: Option<PathBuf>,
    open_browser: bool,
) -> Result<()> {
    let bind_is_loopback = host_is_loopback(&host);

    // A credential that is optional on an exposed port is not a credential; it
    // is a setting nobody changed. So the token is mandatory the moment the bind
    // stops being loopback, and the refusal is up front rather than a warning.
    if !bind_is_loopback && no_token {
        bail!(
            "--no-token cannot be combined with --host {host}, which is reachable from other \
             machines.\nEither bind a loopback address (the default), or drop --no-token and \
             open the URL this command prints."
        );
    }

    let web_dir = match web_dir {
        Some(dir) => {
            if !dir.join("index.html").is_file() {
                bail!(
                    "no web interface at {} (expected an index.html there)",
                    dir.display()
                );
            }
            dir
        }
        None => resolve_web_dir()?,
    };

    let browser_token = if no_token {
        None
    } else {
        Some(token.unwrap_or_else(|| random_hex(32)))
    };
    let secret_key = random_hex(32);

    // Fail on an occupied port here, with a clear message, rather than letting
    // the child die of EADDRINUSE while a readiness probe cheerfully succeeds
    // against whatever else is listening. The probe below also watches the
    // child, so the pair covers the race this pre-flight cannot.
    preflight_port(&host, port)?;

    let daemon = resolve_biorouterd()?;
    let mut child = Command::new(&daemon)
        .arg("agent")
        .env("BIOROUTER_HOST", &host)
        .env("BIOROUTER_PORT", port.to_string())
        .env("BIOROUTER_SERVER__SECRET_KEY", &secret_key)
        .env("BIOROUTER_SERVE_UI", &web_dir)
        .envs(
            browser_token
                .iter()
                .map(|t| ("BIOROUTER_BROWSER_TOKEN", t.as_str())),
        )
        // See the module documentation: no proof-of-user digest, on purpose.
        .stdin(Stdio::null())
        .spawn()
        .with_context(|| format!("could not start {}", daemon.display()))?;

    if let Err(e) = wait_until_ready(&host, port, &mut child) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(e);
    }

    let url = browser_url(&host, port, browser_token.as_deref());
    println!("\n  Biorouter is serving at\n\n      {url}\n");
    if !bind_is_loopback {
        match reachable_address(&host) {
            Some(addr) => println!(
                "  From another machine on this network:\n\n      {}\n",
                browser_url(&addr, port, browser_token.as_deref())
            ),
            // The old implementation fell back to 127.0.0.1 here, which printed
            // a URL that could not possibly work from the other machine the user
            // had just asked to reach it from. Saying so is the whole point.
            None => println!(
                "  This port is bound on every interface, but no routable address could be \
                 determined for this machine.\n  Use its own hostname or address in place of \
                 the host above.\n"
            ),
        }
    }
    if browser_token.is_none() {
        println!("  No access token: anything that can reach this port can use it.\n");
    } else {
        println!("  The token above is shown once, and is new on every launch.\n");
    }
    println!("  The model is whichever `biorouter configure` chose; a browser cannot change it.");
    println!("  Press Ctrl-C to stop.\n");

    if open_browser {
        let _ = webbrowser::open(&url);
    }

    // Hand the terminal back to the daemon and stop when it does, or when the
    // user interrupts. Killing the child on the way out is what stops a stray
    // daemon holding the port after Ctrl-C.
    let result = tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("\nStopping.");
            Ok(())
        }
        status = tokio::task::spawn_blocking(move || child.wait()) => {
            match status {
                Ok(Ok(s)) if s.success() => Ok(()),
                Ok(Ok(s)) => bail!("biorouterd exited with {s}"),
                Ok(Err(e)) => bail!("could not wait on biorouterd: {e}"),
                Err(e) => bail!("could not wait on biorouterd: {e}"),
            }
        }
    };
    result
}

/// The URL to open, with the browser token in it.
///
/// The token is spent on the first request: the daemon exchanges it for a
/// session cookie and redirects, so it does not linger in the address bar or in
/// the `Referer` of anything the page later loads.
fn browser_url(host: &str, port: u16, token: Option<&str>) -> String {
    // A bare IPv6 address needs brackets in a URL; a hostname must not have them.
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    match token {
        Some(t) => format!("http://{host}:{port}/?t={t}"),
        None => format!("http://{host}:{port}/"),
    }
}

fn random_hex(bytes: usize) -> String {
    use rand::Rng as _;
    let mut rng = rand::thread_rng();
    (0..bytes)
        .map(|_| format!("{:02x}", rng.gen::<u8>()))
        .collect()
}

fn host_is_loopback(host: &str) -> bool {
    // A wildcard bind is reachable from other machines by definition, and
    // resolving it would answer the wrong question.
    if host == "0.0.0.0" || host == "::" || host == "[::]" {
        return false;
    }
    (host, 0)
        .to_socket_addrs()
        .map(|mut addrs| addrs.all(|a| a.ip().is_loopback()))
        .unwrap_or(false)
}

/// An address this machine can be reached at from elsewhere on its network.
///
/// The connect is to a routable address and sends nothing; it exists so the
/// operating system will pick the interface it would route through, whose local
/// address is then the answer. On a machine with no default route there is no
/// answer, and `None` says so rather than substituting a loopback address that
/// cannot work from another machine.
fn reachable_address(host: &str) -> Option<String> {
    if host != "0.0.0.0" && host != "::" && host != "[::]" {
        return Some(host.to_string());
    }
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:80").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then(|| ip.to_string())
}

fn preflight_port(host: &str, port: u16) -> Result<()> {
    let bind_host = if host == "[::]" { "::" } else { host };
    match TcpListener::bind((bind_host, port)) {
        Ok(l) => {
            drop(l);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            bail!("port {port} on {host} is already in use. Choose another with --port <n>.")
        }
        Err(e) => bail!("cannot bind {host}:{port}: {e}"),
    }
}

/// Wait for the daemon to start answering, watching the child while we do.
///
/// Watching it is the load-bearing half. A probe that only tries to connect
/// reports success against *any* listener on that port — so a daemon that died
/// on startup, next to some unrelated process holding the port, looks exactly
/// like a healthy one.
fn wait_until_ready(host: &str, port: u16, child: &mut std::process::Child) -> Result<()> {
    let connect_host = match host {
        "0.0.0.0" => "127.0.0.1",
        "::" | "[::]" => "::1",
        other => other,
    };
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().context("could not poll biorouterd")? {
            bail!("biorouterd exited during startup with {status}");
        }
        if let Ok(addrs) = (connect_host, port).to_socket_addrs() {
            for addr in addrs {
                if TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok() {
                    return Ok(());
                }
            }
        }
        if Instant::now() >= deadline {
            bail!(
                "biorouterd did not start listening on {connect_host}:{port} within {}s",
                READY_TIMEOUT.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Find the `biorouterd` that belongs with this `biorouter`.
///
/// Beside our own executable first, so a packaged application and a development
/// tree both pair the two binaries that were built together, rather than
/// whichever one is earliest on `PATH`; then beside the installation this copy
/// came from, which is the only thing a Windows install has to go on. See
/// [`crate::commands::exe_path::biorouterd_for`].
fn resolve_biorouterd() -> Result<PathBuf> {
    if let Some(found) = current_exe_resolved().and_then(|exe| biorouterd_for(&exe)) {
        return Ok(found);
    }
    // Fall back to PATH, and let the spawn report a missing binary.
    Ok(PathBuf::from(daemon_file_name()))
}

/// Candidate locations for the built interface, in order.
///
/// Returned as a list so the failure can name every one of them. An error that
/// says only "not found" leaves the reader guessing which of four layouts the
/// installation is in.
fn web_dir_candidates() -> Vec<PathBuf> {
    web_dir_candidates_for(current_exe_resolved().as_deref())
}

/// The candidate list for a given executable path.
///
/// ⚠ `exe` must be the **resolved** path, not `current_exe()` — see
/// [`crate::commands::exe_path`]. The exe-relative entries below are siblings of
/// the binary, so a symlink one directory up from the installation moves every
/// candidate with it: through `~/.local/bin/biorouter` the two became
/// `~/.local/web` and `~/ui/desktop/src/web`, and `serve` reported that the
/// interface was missing on an installation that ships it.
///
/// Resolution is not enough on Windows, where the install is a *copy* and the
/// bundle stays behind in the application. The breadcrumb entry is that case:
/// the copy records the directory it came from, and the bundle is `../web` of
/// **that**. It is consulted after the exe-relative entries — a bundle actually
/// next to the binary is the one to use — and before the Linux package
/// location, which is a fixed path that has nothing to do with this install.
fn web_dir_candidates_for(exe: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(dir) = std::env::var("BIOROUTER_SERVE_UI") {
        out.push(PathBuf::from(dir));
    }
    if let Some(dir) = exe.and_then(Path::parent) {
        // Packaged: the binaries sit in `Resources/bin`, the bundle beside
        // them in `Resources/web`.
        out.push(dir.join("..").join("web"));
        // A development tree: target/<profile>/biorouter, with the bundle
        // where `npm run build:web` writes it.
        out.push(
            dir.join("..")
                .join("..")
                .join("ui")
                .join("desktop")
                .join("src")
                .join("web"),
        );
        // A Windows install: `Resources/web` of the application this copy was
        // taken from. Pushed even when the recorded directory has since been
        // deleted, so a stale breadcrumb is named in the failure rather than
        // leaving the reader to guess why nothing was found.
        if let Some(origin) = biorouter::system::install_origin(dir) {
            out.push(origin.join("..").join("web"));
        }
    }
    // The Linux packages, where the exe-relative rule does not survive: from
    // /usr/bin, `../web` is /usr/web.
    out.push(PathBuf::from("/usr/share/biorouter/web"));
    out
}

fn resolve_web_dir() -> Result<PathBuf> {
    let candidates = web_dir_candidates();
    for candidate in &candidates {
        if candidate.join("index.html").is_file() {
            return Ok(normalise(candidate));
        }
    }
    let tried = candidates
        .iter()
        .map(|p| format!("  {}", normalise(p).display()))
        .collect::<Vec<_>>()
        .join("\n");
    bail!(
        "could not find the Biorouter web interface. Tried:\n{tried}\n\nPoint at it with \
         --web-dir <dir>, or set BIOROUTER_SERVE_UI. In a development tree, build it with \
         `cd ui/desktop && npm run build:web`."
    )
}

/// Tidy `a/b/../c` for display without touching the filesystem.
///
/// `canonicalize` is not usable here: it fails on a path that does not exist,
/// which is precisely the case the error message is describing.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    // The sibling rule moved to `exe_path` when `apps` needed it too; the test
    // that pins it stayed here, with the rest of the `serve` resolution suite.
    use crate::commands::exe_path::biorouterd_beside;

    #[test]
    fn the_default_port_does_not_collide_with_the_daemon_it_starts() {
        // biorouterd's own default is 3000, and so was the old `biorouter web`.
        assert_ne!(DEFAULT_PORT, 3000);
    }

    #[test]
    fn a_wildcard_bind_is_not_treated_as_loopback() {
        // It is reachable from other machines by definition, so it must take
        // the token requirement. Resolving it would answer a different question
        // and return true.
        assert!(!host_is_loopback("0.0.0.0"));
        assert!(!host_is_loopback("::"));
        assert!(!host_is_loopback("[::]"));
    }

    #[test]
    fn loopback_spellings_are_recognised() {
        assert!(host_is_loopback("127.0.0.1"));
        assert!(host_is_loopback("localhost"));
        assert!(host_is_loopback("::1"));
    }

    #[test]
    fn a_routable_address_is_not_loopback() {
        assert!(!host_is_loopback("192.168.1.42"));
    }

    #[test]
    fn the_url_carries_the_token_and_brackets_an_ipv6_literal() {
        assert_eq!(
            browser_url("127.0.0.1", 8765, Some("abc")),
            "http://127.0.0.1:8765/?t=abc"
        );
        assert_eq!(
            browser_url("127.0.0.1", 8765, None),
            "http://127.0.0.1:8765/"
        );
        assert_eq!(
            browser_url("::1", 8765, Some("abc")),
            "http://[::1]:8765/?t=abc"
        );
        // Already bracketed stays that way rather than becoming [[::1]].
        assert_eq!(browser_url("[::1]", 8765, None), "http://[::1]:8765/");
    }

    #[test]
    fn a_token_is_long_and_different_every_time() {
        let a = random_hex(32);
        let b = random_hex(32);
        assert_eq!(a.len(), 64, "32 bytes is 64 hex characters");
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// The error is the only thing the reader has, so it must name every place
    /// that was looked at. A bare "not found" leaves them guessing which of four
    /// installation layouts they are in.
    #[test]
    fn a_missing_interface_names_every_path_it_tried() {
        // Held because the sibling tests below set this variable, and
        // `web_dir_candidates` reads it.
        let _env = env_lock::lock_env([("BIOROUTER_SERVE_UI", None::<String>)]);
        let candidates = web_dir_candidates();
        assert!(
            candidates.len() >= 2,
            "expected several candidates, got {candidates:?}"
        );
        assert!(
            candidates.contains(&PathBuf::from("/usr/share/biorouter/web")),
            "the Linux package location must be a candidate: from /usr/bin, ../web is /usr/web"
        );
    }

    /// Restated against `web_dir_candidates_for`, which takes the executable as
    /// an argument: the previous version scanned this file's own source for the
    /// order two string literals appear in, and would have passed against a
    /// build that never read the variable at all.
    #[test]
    fn an_explicit_setting_is_looked_at_before_anything_else() {
        let _env = env_lock::lock_env([(
            "BIOROUTER_SERVE_UI",
            Some("/somewhere/explicit/web".to_string()),
        )]);
        let candidates =
            web_dir_candidates_for(Some(Path::new("/opt/Biorouter/resources/bin/biorouter")));
        assert_eq!(
            candidates.first(),
            Some(&PathBuf::from("/somewhere/explicit/web")),
            "the explicit setting must be consulted before the packaged locations: \
             {candidates:?}"
        );
    }

    /// The operator's report, reduced to a fixture: a macOS install reached
    /// through the symlink `biorouter setup-path` creates.
    ///
    /// Before the fix, `current_exe()` on macOS returned the link and every
    /// candidate below was derived from *its* directory — so the packaged
    /// bundle two directories away from the link was never looked at, and the
    /// error named `~/.local/web` and `~/ui/desktop/src/web`.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_executable_resolves_to_the_real_installation() {
        let _env = env_lock::lock_env([("BIOROUTER_SERVE_UI", None::<String>)]);
        let tmp = tempfile::tempdir().unwrap();

        let bin = tmp.path().join("Resources").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let real = bin.join("biorouter");
        std::fs::write(&real, b"#!/bin/sh\n").unwrap();
        let web = tmp.path().join("Resources").join("web");
        std::fs::create_dir_all(&web).unwrap();
        std::fs::write(web.join("index.html"), b"<!doctype html>").unwrap();

        let link_dir = tmp.path().join("local").join("bin");
        std::fs::create_dir_all(&link_dir).unwrap();
        let link = link_dir.join("biorouter");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let resolved = crate::commands::exe_path::resolve_exe_path(&link);
        let candidates = web_dir_candidates_for(Some(&resolved));
        let found = candidates
            .iter()
            .find(|c| c.join("index.html").is_file())
            .unwrap_or_else(|| {
                panic!("no candidate held the bundle that is on disk: {candidates:?}")
            });
        assert_eq!(
            normalise(found),
            std::fs::canonicalize(&web).unwrap(),
            "the packaged bundle must be found through the symlink"
        );

        // And the unresolved path is what the bug looked like, so pin that the
        // two really do differ — otherwise this test would pass on a platform
        // where it proves nothing.
        let unresolved = web_dir_candidates_for(Some(&link));
        assert!(
            !unresolved.iter().any(|c| c.join("index.html").is_file()),
            "fixture is wrong: the bundle must be unreachable from the link's \
             own directory, or this test cannot fail"
        );
    }

    /// The other half of the same defect: `serve` could not start the daemon
    /// either, because it looked for it beside the link.
    #[cfg(unix)]
    #[test]
    fn the_daemon_is_found_beside_the_real_executable() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("Resources").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let real = bin.join("biorouter");
        std::fs::write(&real, b"#!/bin/sh\n").unwrap();
        let daemon = bin.join(format!("biorouterd{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&daemon, b"#!/bin/sh\n").unwrap();

        let link_dir = tmp.path().join("local").join("bin");
        std::fs::create_dir_all(&link_dir).unwrap();
        let link = link_dir.join("biorouter");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        assert!(
            biorouterd_beside(&link).is_none(),
            "fixture is wrong: the daemon must not be beside the link"
        );
        // Compared after canonicalising both: on macOS the temp directory is
        // itself reached through `/var -> /private/var`, so the resolved path
        // is spelled differently while naming the same file.
        assert_eq!(
            biorouterd_beside(&crate::commands::exe_path::resolve_exe_path(&link))
                .map(|p| std::fs::canonicalize(p).unwrap()),
            Some(std::fs::canonicalize(&daemon).unwrap()),
            "the daemon must be found beside the binary the link points at"
        );
    }

    /// A candidate list derived from a verbatim Windows path would be printed
    /// at the user and handed to the daemon in `BIOROUTER_SERVE_UI`. Runs
    /// everywhere; only Windows can fail it.
    ///
    /// The breadcrumb is written the way `install_cli` writes it — from the
    /// parent of a **canonicalised** source path, which on Windows is
    /// `\\?\C:\…` — so the candidate derived from it is covered by the same
    /// rule.
    #[test]
    fn the_resolved_candidates_are_never_windows_verbatim_paths() {
        let _env = env_lock::lock_env([("BIOROUTER_SERVE_UI", None::<String>)]);
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp
            .path()
            .join(format!("biorouter{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&exe, b"x").unwrap();

        let source_bin = application(&tmp.path().join("Application"));
        let canonical_source = std::fs::canonicalize(
            source_bin.join(format!("biorouter{}", std::env::consts::EXE_SUFFIX)),
        )
        .unwrap();
        biorouter::system::record_install_origin(
            canonical_source.parent().unwrap(),
            exe.parent().unwrap(),
        )
        .unwrap();

        let resolved = crate::commands::exe_path::resolve_exe_path(&exe);
        let candidates = web_dir_candidates_for(Some(&resolved));
        assert_eq!(
            candidates.len(),
            4,
            "the breadcrumb-derived candidate must be present, or this test \
             covers less than it claims: {candidates:?}"
        );
        for candidate in candidates {
            assert!(
                !normalise(&candidate).to_string_lossy().starts_with(r"\\?\"),
                "verbatim path in a candidate: {}",
                candidate.display()
            );
        }
    }

    #[test]
    fn a_relative_parent_is_tidied_for_display() {
        assert_eq!(normalise(Path::new("/a/b/../web")), PathBuf::from("/a/web"));
        assert_eq!(
            normalise(Path::new("/a/b/c/../../ui/desktop/src/web")),
            PathBuf::from("/a/ui/desktop/src/web")
        );
    }

    /// A fixture shaped like the shipped Windows application: both binaries in
    /// `resources/bin`, the interface bundle beside them in `resources/web`.
    /// Returns the `bin` directory.
    fn application(root: &Path) -> PathBuf {
        let bin = root.join("resources").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let name = format!("biorouter{}", std::env::consts::EXE_SUFFIX);
        std::fs::write(bin.join(name), b"x").unwrap();
        let web = root.join("resources").join("web");
        std::fs::create_dir_all(&web).unwrap();
        std::fs::write(web.join("index.html"), b"<!doctype html>").unwrap();
        bin
    }

    /// A Windows install: `biorouter.exe` copied onto `PATH` on its own, with
    /// nothing beside it. Returns the installed executable.
    fn windows_style_install(root: &Path) -> PathBuf {
        let install = root.join("Local").join("Biorouter").join("bin");
        std::fs::create_dir_all(&install).unwrap();
        let exe = install.join(format!("biorouter{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&exe, b"x").unwrap();
        exe
    }

    /// The Windows half of the defect PR #183 fixed for macOS. `install_cli`
    /// copies `biorouter.exe` into `%LOCALAPPDATA%\Biorouter\bin` and nothing
    /// else, so there is no link to resolve and no bundle beside the copy — the
    /// two exe-relative candidates become `%LOCALAPPDATA%\Biorouter\web` and
    /// `%LOCALAPPDATA%\ui\desktop\src\web`, neither of which can ever exist.
    #[test]
    fn a_windows_style_install_finds_the_bundle_through_its_breadcrumb() {
        let _env = env_lock::lock_env([("BIOROUTER_SERVE_UI", None::<String>)]);
        let tmp = tempfile::tempdir().unwrap();
        let source_bin = application(&tmp.path().join("Application"));
        let exe = windows_style_install(tmp.path());

        // Without the breadcrumb the bundle is unreachable — otherwise this
        // test would pass against the code that shipped the bug.
        assert!(
            !web_dir_candidates_for(Some(&exe))
                .iter()
                .any(|c| c.join("index.html").is_file()),
            "fixture is wrong: the bundle must be unreachable before the breadcrumb is written"
        );

        biorouter::system::record_install_origin(&source_bin, exe.parent().unwrap()).unwrap();

        let candidates = web_dir_candidates_for(Some(&exe));
        let found = candidates
            .iter()
            .find(|c| c.join("index.html").is_file())
            .unwrap_or_else(|| {
                panic!("no candidate held the bundle that is on disk: {candidates:?}")
            });
        assert_eq!(
            std::fs::canonicalize(normalise(found)).unwrap(),
            std::fs::canonicalize(tmp.path().join("Application").join("resources").join("web"))
                .unwrap(),
            "the packaged bundle must be found through the breadcrumb"
        );
    }

    /// The breadcrumb is a fallback, not an override: a bundle sitting beside
    /// the binary is this installation's own and must win.
    #[test]
    fn the_breadcrumb_is_consulted_after_the_locations_beside_the_binary() {
        let _env = env_lock::lock_env([("BIOROUTER_SERVE_UI", None::<String>)]);
        let tmp = tempfile::tempdir().unwrap();
        let source_bin = application(&tmp.path().join("Application"));
        let exe = windows_style_install(tmp.path());
        biorouter::system::record_install_origin(&source_bin, exe.parent().unwrap()).unwrap();

        let candidates = web_dir_candidates_for(Some(&exe));
        // `real_path`, not the fixture's own spelling: on macOS a temp directory
        // is reached through `/var -> /private/var`, so the recorded origin is
        // spelled differently while naming the same directory.
        let derived = normalise(
            &biorouter::system::real_path(&source_bin)
                .join("..")
                .join("web"),
        );
        let at = candidates
            .iter()
            .position(|c| normalise(c) == derived)
            .unwrap_or_else(|| {
                panic!("the breadcrumb-derived candidate {derived:?} must be in {candidates:?}")
            });
        assert_eq!(
            at, 2,
            "expected the breadcrumb after the two exe-relative entries and before the \
             Linux package location: {candidates:?}"
        );
        assert_eq!(
            candidates.last(),
            Some(&PathBuf::from("/usr/share/biorouter/web")),
            "the fixed package location stays last: {candidates:?}"
        );
    }

    /// The application was uninstalled or moved. The stale location has to reach
    /// the error text — it is the only thing that tells the reader why an
    /// install that used to work no longer does.
    #[test]
    fn a_stale_breadcrumb_is_named_among_the_paths_that_were_tried() {
        let _env = env_lock::lock_env([("BIOROUTER_SERVE_UI", None::<String>)]);
        let tmp = tempfile::tempdir().unwrap();
        let application_root = tmp.path().join("Application");
        let source_bin = application(&application_root);
        let exe = windows_style_install(tmp.path());
        biorouter::system::record_install_origin(&source_bin, exe.parent().unwrap()).unwrap();
        // Read while the directory still exists: once it is gone nothing can
        // canonicalise it, and on macOS the fixture's own spelling (`/var/…`)
        // is not the one that was recorded (`/private/var/…`).
        let recorded = biorouter::system::real_path(&source_bin);
        std::fs::remove_dir_all(&application_root).unwrap();

        let candidates = web_dir_candidates_for(Some(&exe));
        assert!(
            !candidates.iter().any(|c| c.join("index.html").is_file()),
            "nothing should resolve: the application is gone"
        );
        // `resolve_web_dir` prints `normalise(p)` for every candidate, so this
        // is the string the reader would see.
        let tried: Vec<String> = candidates
            .iter()
            .map(|p| normalise(p).display().to_string())
            .collect();
        let expected = normalise(&recorded.join("..").join("web"))
            .display()
            .to_string();
        assert!(
            tried.contains(&expected),
            "the stale location must be named in the failure. Tried: {tried:?}"
        );
    }

    /// Every state a machine really reaches: a Unix install with no breadcrumb
    /// at all, and a file that is empty or is not a path.
    #[test]
    fn a_missing_or_unusable_breadcrumb_falls_back_without_panicking() {
        let _env = env_lock::lock_env([("BIOROUTER_SERVE_UI", None::<String>)]);
        let tmp = tempfile::tempdir().unwrap();
        let exe = windows_style_install(tmp.path());
        let install = exe.parent().unwrap().to_path_buf();

        let without = web_dir_candidates_for(Some(&exe));
        assert_eq!(
            without.len(),
            3,
            "no breadcrumb means the two exe-relative entries plus the package \
             location: {without:?}"
        );

        for content in ["", "  \n", "not a path", "./relative/bin"] {
            std::fs::write(
                install.join(biorouter::system::INSTALL_ORIGIN_FILE),
                content,
            )
            .unwrap();
            assert_eq!(
                web_dir_candidates_for(Some(&exe)),
                without,
                "an unusable breadcrumb ({content:?}) must add no candidate"
            );
        }
    }
}
