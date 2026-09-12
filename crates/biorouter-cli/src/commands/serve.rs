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
//!
//! # Why the daemon cannot outlive this command
//!
//! The daemon holds the port, and it answers the browser token and serves the
//! shell carrying its secret for as long as it runs — so stopping `serve` is
//! the only way an operator has to revoke the URL it printed. Two layers make
//! that hold however `serve` ends:
//!
//! 1. Every way out of [`handle_serve`] after the spawn — SIGINT, SIGTERM, the
//!    daemon dying, a startup that never became ready — goes through
//!    [`stop_daemon`], which asks the daemon to stop, waits, and then kills
//!    and reaps it.
//! 2. On Unix the daemon is started with `--exit-with-parent <our pid>` and
//!    stops itself once this process is gone, which covers the endings that run
//!    no code at all: SIGKILL, a crash.
//!
//! Before this, the only thing that ever stopped the daemon was a terminal's
//! Ctrl-C, which reaches the whole foreground process group and so the daemon
//! directly. `kill <pid of serve>` from anywhere else left it running.
//!
//! # Where the browser token comes from
//!
//! `--token`, else `BIOROUTER_BROWSER_TOKEN` from this shell, else one minted
//! for this launch — the same order [`resolve_web_dir`] uses for the interface
//! directory, and for the same reason: a command line outranks the environment
//! it runs in, and both outrank a default. The variable is what a service unit
//! has to work with (a systemd `EnvironmentFile`, read only by the service
//! user), and it is the one way an address stays valid across a restart that
//! nobody is watching. It used to be ignored here and overwritten with a random
//! token, so the documented service deployment printed an address the operator
//! could not know and refused the one they had published.

use crate::commands::exe_path::{biorouterd_for, current_exe_resolved, daemon_file_name};
use anyhow::{bail, Context, Result};
use std::net::{TcpListener, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};

/// Not 3000. That is `biorouterd`'s own default, and the old `biorouter web`
/// used it too — a default that collides with the daemon this command starts is
/// a support question waiting to happen.
pub const DEFAULT_PORT: u16 = 8765;

/// How long to wait for the daemon to answer before giving up.
const READY_TIMEOUT: Duration = Duration::from_secs(60);

/// How long the daemon gets to shut down on its own before it is killed.
///
/// Its graceful shutdown waits for open connections to finish, and a browser
/// tab that is still open holds some that never do — so without a limit,
/// stopping `serve` with a tab open would wait forever. The daemon applies the
/// same figure to itself when it finds itself orphaned.
const STOP_GRACE: Duration = Duration::from_secs(10);

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

    let web_dir = resolve_web_dir(web_dir)?;

    let browser_token = choose_browser_token(
        token,
        no_token,
        std::env::var("BIOROUTER_BROWSER_TOKEN").ok(),
        || random_hex(32),
    );
    let secret_key = random_hex(32);

    // Fail on an occupied port here, with a clear message, rather than letting
    // the child die of EADDRINUSE while a readiness probe cheerfully succeeds
    // against whatever else is listening. The probe below also watches the
    // child, so the pair covers the race this pre-flight cannot.
    preflight_port(&host, port)?;

    // Listen for a stop request BEFORE the daemon exists. A handler replaces
    // the default action — which for SIGTERM is to end this process on the spot
    // and leave the daemon behind — so from here on a signal waits to be read,
    // including one that lands during the readiness wait below.
    let mut stop = StopSignals::install()?;

    let daemon = resolve_biorouterd()?;
    let mut command = Command::new(&daemon);
    command.arg("agent");
    // The second layer; see the module documentation.
    #[cfg(unix)]
    command
        .arg("--exit-with-parent")
        .arg(std::process::id().to_string());
    match browser_token.value() {
        Some(token) => {
            command.env("BIOROUTER_BROWSER_TOKEN", token);
        }
        // ⚠ Removed, not merely unset. The child inherits this shell's
        // environment, so `--no-token` in a shell that exports a token would
        // leave the daemon demanding one while the URL printed below carries
        // none: every open a 401, and nothing on screen to say why.
        None => {
            command.env_remove("BIOROUTER_BROWSER_TOKEN");
        }
    }
    let mut child = command
        .env("BIOROUTER_HOST", &host)
        .env("BIOROUTER_PORT", port.to_string())
        .env("BIOROUTER_SERVER__SECRET_KEY", &secret_key)
        .env("BIOROUTER_SERVE_UI", &web_dir)
        // See the module documentation: no proof-of-user digest, on purpose.
        .stdin(Stdio::null())
        // A backstop for a panic unwinding through here. Every ordinary path
        // goes through `stop_daemon`, which asks before it insists.
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("could not start {}", daemon.display()))?;

    let outcome: Result<()> = async {
        tokio::select! {
            ready = wait_until_ready(&host, port, &mut child) => ready?,
            _ = stop.recv() => {
                println!("\nStopping.");
                return Ok(());
            }
        }

        let url = browser_url(&host, port, browser_token.value());
        print_banner(&url, &host, port, &browser_token, bind_is_loopback);
        if open_browser {
            let _ = webbrowser::open(&url);
        }

        tokio::select! {
            _ = stop.recv() => {
                println!("\nStopping.");
                Ok(())
            }
            status = child.wait() => match status {
                Ok(s) if s.success() => Ok(()),
                Ok(s) => bail!("biorouterd exited with {s}"),
                Err(e) => bail!("could not wait on biorouterd: {e}"),
            },
        }
    }
    .await;

    stop_daemon(&mut child, &mut stop).await;
    outcome
}

/// What the operator reads once the daemon is answering.
fn print_banner(
    url: &str,
    host: &str,
    port: u16,
    browser_token: &BrowserToken,
    bind_is_loopback: bool,
) {
    println!("\n  Biorouter is serving at\n\n      {url}\n");
    if !bind_is_loopback {
        match reachable_address(host) {
            Some(addr) => println!(
                "  From another machine on this network:\n\n      {}\n",
                browser_url(&addr, port, browser_token.value())
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
    println!("  {}\n", browser_token.provenance());
    println!("  The model is whichever `biorouter configure` chose; a browser cannot change it.");
    println!("  Press Ctrl-C to stop.\n");
}

/// The requests to stop that `serve` honours: SIGINT and SIGTERM on Unix,
/// Ctrl-C elsewhere.
///
/// SIGHUP is deliberately left alone. A terminal hanging up signals the whole
/// foreground process group, daemon included; `nohup` exists to make both
/// ignore it, and installing a handler here would override that. Any other
/// SIGHUP ends `serve` the default way and the daemon's parent watch follows.
struct StopSignals {
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
}

impl StopSignals {
    fn install() -> Result<Self> {
        #[cfg(unix)]
        use tokio::signal::unix::{signal, SignalKind};
        Ok(Self {
            #[cfg(unix)]
            interrupt: signal(SignalKind::interrupt()).context("could not listen for SIGINT")?,
            #[cfg(unix)]
            terminate: signal(SignalKind::terminate()).context("could not listen for SIGTERM")?,
        })
    }

    /// Resolve on the next request to stop.
    async fn recv(&mut self) {
        #[cfg(unix)]
        tokio::select! {
            _ = self.interrupt.recv() => {}
            _ = self.terminate.recv() => {}
        }
        // A console Ctrl-C reaches every process attached to the console, so on
        // Windows the daemon has been told as well and is already stopping.
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

/// Stop the daemon and reap it: ask, give it [`STOP_GRACE`], then insist.
///
/// A second request to stop while it is shutting down skips the rest of the
/// wait. A daemon that has already exited is only reaped, so every path can end
/// here without first asking whether it needs to.
async fn stop_daemon(child: &mut Child, stop: &mut StopSignals) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    ask_to_stop(child);
    tokio::select! {
        waited = tokio::time::timeout(STOP_GRACE, child.wait()) => {
            if matches!(waited, Ok(Ok(_))) {
                return;
            }
            // The usual reason, measured: an open browser tab keeps the
            // renderer's 25 s catalog long poll parked on the daemon, and its
            // graceful shutdown waits for that request to finish.
            eprintln!(
                "biorouterd did not finish within {}s (an open browser tab keeps a request \
                 waiting); killing it.",
                STOP_GRACE.as_secs()
            );
        }
        _ = stop.recv() => eprintln!("Killing biorouterd."),
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
}

/// Ask the daemon to shut down gracefully: SIGTERM, which it handles exactly as
/// it handles Ctrl-C — draining connections and taking a llama-server sidecar
/// down with it.
#[cfg(unix)]
fn ask_to_stop(child: &Child) {
    let Some(pid) = child.id().and_then(|p| libc::pid_t::try_from(p).ok()) else {
        return;
    };
    // SAFETY: `kill(2)` has no memory-safety preconditions. `id()` is `None`
    // once the child has been reaped, so this pid is still our own child and
    // cannot have been recycled for an unrelated process.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
}

/// Windows has no SIGTERM to send. A console Ctrl-C has usually reached the
/// daemon already; if nothing has, [`stop_daemon`] kills it once the grace has
/// passed.
#[cfg(not(unix))]
fn ask_to_stop(_child: &Child) {}

/// The access token this launch will use, and where it came from.
///
/// The provenance is carried rather than recomputed because it changes what the
/// operator is told: "shown once, and new on every launch" is true of a minted
/// token and false of one they chose, and printing it over an operator's own
/// token would be an instruction to go looking for a new address that does not
/// exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowserToken {
    /// `--no-token`: the gate is off. Only reachable for a loopback bind.
    Off,
    /// `--token <t>`.
    Flag(String),
    /// `BIOROUTER_BROWSER_TOKEN` in this shell.
    Variable(String),
    /// Minted for this launch, because nothing named one.
    Minted(String),
}

impl BrowserToken {
    /// The token itself, or `None` when there is no gate.
    pub(crate) fn value(&self) -> Option<&str> {
        match self {
            BrowserToken::Off => None,
            BrowserToken::Flag(t) | BrowserToken::Variable(t) | BrowserToken::Minted(t) => Some(t),
        }
    }

    /// The line under the banner.
    fn provenance(&self) -> &'static str {
        match self {
            BrowserToken::Off => "No access token: anything that can reach this port can use it.",
            BrowserToken::Flag(_) => {
                "The token above is the one --token named; it works until you change it."
            }
            BrowserToken::Variable(_) => {
                "The token above came from BIOROUTER_BROWSER_TOKEN; it works until you change it."
            }
            BrowserToken::Minted(_) => "The token above is shown once, and is new on every launch.",
        }
    }
}

/// Decide the token: `--no-token`, else `--token`, else
/// `BIOROUTER_BROWSER_TOKEN`, else one minted here.
///
/// `mint` is passed in so a test can state the order without matching a random
/// string, and the variable is passed in rather than read here for the reason
/// [`choose_web_dir`] gives: other tests in this binary read the process
/// environment, and a test that sets one races them.
///
/// A blank variable reads as unset, exactly as `BIOROUTER_SERVE_UI`'s does. The
/// value is trimmed: it arrives from an environment file, where a stray newline
/// or a trailing space is a typo rather than part of a credential, and a token
/// that differs from the published one by an invisible byte fails as a flat 401
/// with nothing on screen to explain it.
pub(crate) fn choose_browser_token(
    flag: Option<String>,
    no_token: bool,
    variable: Option<String>,
    mint: impl FnOnce() -> String,
) -> BrowserToken {
    // ⚠ First, and before the variable is even looked at. `--no-token` is a
    // refusal to have a gate; a token this shell happens to export is not a
    // reason to put one back that the printed URL would not carry.
    if no_token {
        return BrowserToken::Off;
    }
    if let Some(token) = flag {
        return BrowserToken::Flag(token);
    }
    match variable.map(|t| t.trim().to_string()) {
        Some(token) if !token.is_empty() => BrowserToken::Variable(token),
        _ => BrowserToken::Minted(mint()),
    }
}

/// The URL to open, with the browser token in it.
///
/// Opening it exchanges the token for a session cookie and redirects, so the
/// token does not linger in the address bar or in the `Referer` of anything the
/// page later loads. It is not *spent*, which is how this comment used to put
/// it: the exchange works as often as the token is presented, for anyone who
/// has it, until the daemon stops. Decision SD-9 in
/// `docs/deployment/serve-decisions.md` records why that is deliberate.
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
///
/// Asynchronous so that a request to stop can interrupt it: the wait can run
/// for a minute, and the daemon is already running for all of it.
async fn wait_until_ready(host: &str, port: u16, child: &mut Child) -> Result<()> {
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
        if let Ok(addrs) = tokio::net::lookup_host((connect_host, port)).await {
            for addr in addrs {
                let attempt = tokio::time::timeout(
                    Duration::from_millis(250),
                    tokio::net::TcpStream::connect(addr),
                );
                if matches!(attempt.await, Ok(Ok(_))) {
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
        tokio::time::sleep(Duration::from_millis(100)).await;
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

/// Where the interface comes from: `--web-dir`, else `BIOROUTER_SERVE_UI`, else
/// the first of [`web_dir_candidates`] that holds one.
fn resolve_web_dir(flag: Option<PathBuf>) -> Result<PathBuf> {
    choose_web_dir(
        flag,
        std::env::var_os("BIOROUTER_SERVE_UI"),
        web_dir_candidates,
    )
}

/// [`resolve_web_dir`], with what it reads passed in.
///
/// A directory the operator names — with the flag or with the variable — is
/// used as named or refused, never skipped. The variable used to be only the
/// first *candidate* of the search, so one naming a directory with no
/// `index.html` was passed over in silence and `serve` went on to serve
/// whatever the search found next: a bundle the operator had not chosen, with
/// nothing to say so, while the same path given as `--web-dir` was refused.
/// The flag wins when both are set, as a command line does over the
/// environment it runs in.
fn choose_web_dir(
    flag: Option<PathBuf>,
    variable: Option<std::ffi::OsString>,
    candidates: impl FnOnce() -> Vec<PathBuf>,
) -> Result<PathBuf> {
    let named = match (flag, variable) {
        (Some(dir), _) => Some((dir, "--web-dir")),
        // Blank reads as unset, as it does for `BIOROUTER_PATH_ROOT`: taken
        // literally, an empty path is the working directory.
        (None, Some(dir)) if !dir.to_string_lossy().trim().is_empty() => {
            Some((PathBuf::from(dir), "BIOROUTER_SERVE_UI"))
        }
        (None, _) => None,
    };
    if let Some((dir, source)) = named {
        if !dir.join("index.html").is_file() {
            bail!(
                "no web interface at {} (expected an index.html there; the path came from \
                 {source})",
                dir.display()
            );
        }
        return Ok(dir);
    }

    let candidates = candidates();
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

/// Where to look for the built interface when none was named, in order.
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
    // `cfg(unix)` for the same reason that test is: it needs a symlink, and an
    // import only one platform uses is a warning on the other one.
    #[cfg(unix)]
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

    fn minted() -> String {
        "minted".to_string()
    }

    /// The order, stated once: flag, then variable, then a minted token.
    #[test]
    fn the_token_comes_from_the_flag_then_the_variable_then_a_fresh_one() {
        assert_eq!(
            choose_browser_token(Some("from-the-flag".into()), false, None, minted),
            BrowserToken::Flag("from-the-flag".into())
        );
        assert_eq!(
            choose_browser_token(None, false, Some("from-the-file".into()), minted),
            BrowserToken::Variable("from-the-file".into())
        );
        assert_eq!(
            choose_browser_token(None, false, None, minted),
            BrowserToken::Minted("minted".into())
        );
    }

    /// A command line outranks the environment it runs in, as it does for
    /// `--web-dir` against `BIOROUTER_SERVE_UI`.
    #[test]
    fn the_token_flag_takes_precedence_over_the_variable() {
        assert_eq!(
            choose_browser_token(
                Some("from-the-flag".into()),
                false,
                Some("from-the-file".into()),
                minted
            ),
            BrowserToken::Flag("from-the-flag".into())
        );
    }

    /// The measured defect (2026-09-12): `BIOROUTER_BROWSER_TOKEN=<t> biorouter
    /// serve` printed a random token and answered `?t=<t>` with 401, so the
    /// systemd deployment in `docs/deployment/headless-linux.md` — whose whole
    /// point is a token that survives a restart — could not work as written.
    ///
    /// Fails the shipped command, which minted a token here.
    #[test]
    fn an_operator_supplied_token_is_used_rather_than_overwritten() {
        let chosen = choose_browser_token(None, false, Some("token-from-the-file".into()), || {
            panic!("a token was named; nothing may be minted over it")
        });
        assert_eq!(chosen.value(), Some("token-from-the-file"));
        assert!(
            chosen.provenance().contains("BIOROUTER_BROWSER_TOKEN"),
            "the operator must be told the token is theirs, not a new one: {}",
            chosen.provenance()
        );
    }

    /// Blank reads as unset, as it does for `BIOROUTER_SERVE_UI` — and a value
    /// that is only whitespace is a mistake in an environment file, not a
    /// credential.
    #[test]
    fn a_blank_token_variable_is_not_a_choice() {
        for blank in ["", "  ", "\n"] {
            assert_eq!(
                choose_browser_token(None, false, Some(blank.into()), minted),
                BrowserToken::Minted("minted".into()),
                "{blank:?}"
            );
        }
    }

    /// A newline an environment file left behind is not part of the credential.
    #[test]
    fn a_token_from_the_environment_is_trimmed() {
        assert_eq!(
            choose_browser_token(None, false, Some("  tok\n".into()), minted),
            BrowserToken::Variable("tok".into())
        );
    }

    /// `--no-token` is a refusal to have a gate. An inherited token must not put
    /// one back: the daemon would demand it and the URL printed here would not
    /// carry it, so every open would be a 401 with nothing on screen to say why.
    #[test]
    fn no_token_beats_a_token_this_shell_happens_to_export() {
        let chosen = choose_browser_token(None, true, Some("inherited".into()), || {
            panic!("--no-token mints nothing")
        });
        assert_eq!(chosen, BrowserToken::Off);
        assert_eq!(chosen.value(), None);
    }

    /// The tests above pass the variable in, so on their own they would pass
    /// against a build that never read it. This one goes through the real
    /// environment, exactly as `serve_reads_the_variable_it_documents` does for
    /// the interface directory.
    #[test]
    fn serve_reads_the_token_variable_it_documents() {
        let _env = env_lock::lock_env([(
            "BIOROUTER_BROWSER_TOKEN",
            Some("token-from-the-environment".to_string()),
        )]);
        let chosen = choose_browser_token(
            None,
            false,
            std::env::var("BIOROUTER_BROWSER_TOKEN").ok(),
            minted,
        );
        assert_eq!(chosen.value(), Some("token-from-the-environment"));
    }

    /// The error is the only thing the reader has, so it must name every place
    /// that was looked at. A bare "not found" leaves them guessing which of four
    /// installation layouts they are in.
    #[test]
    fn a_missing_interface_names_every_path_it_tried() {
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

    /// A search that would find a bundle, so the tests below can tell a
    /// refusal from a quiet fall-back to something else — which is exactly
    /// what finding F8 was.
    fn a_bundle_found_elsewhere(root: &Path) -> impl FnOnce() -> Vec<PathBuf> {
        let web = root.join("found-elsewhere");
        std::fs::create_dir_all(&web).unwrap();
        std::fs::write(web.join("index.html"), b"<!doctype html>").unwrap();
        move || vec![web]
    }

    fn an_interface_at(dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("index.html"), b"<!doctype html>").unwrap();
        dir.to_path_buf()
    }

    /// Finding F8 of the 2026-09-10 QA run: `--web-dir` naming a directory
    /// with no interface was fatal, while `BIOROUTER_SERVE_UI` naming the same
    /// directory was skipped and `serve` served the next bundle it found.
    #[test]
    fn a_named_directory_without_an_interface_is_refused_however_it_was_named() {
        let tmp = tempfile::tempdir().unwrap();
        let typo = tmp.path().join("wbe");
        for (flag, variable, source) in [
            (Some(typo.clone()), None, "--web-dir"),
            (
                None,
                Some(typo.clone().into_os_string()),
                "BIOROUTER_SERVE_UI",
            ),
        ] {
            let err = choose_web_dir(flag, variable, a_bundle_found_elsewhere(tmp.path()))
                .expect_err("a named directory with no index.html must be refused, not skipped")
                .to_string();
            assert!(
                err.starts_with(&format!(
                    "no web interface at {} (expected an index.html there",
                    typo.display()
                )),
                "both spellings must fail with the same message: {err}"
            );
            assert!(
                err.contains(source),
                "the refusal must say where the path came from: {err}"
            );
        }
    }

    #[test]
    fn a_named_directory_is_used_in_preference_to_anything_the_search_finds() {
        let tmp = tempfile::tempdir().unwrap();
        let named = an_interface_at(&tmp.path().join("named"));
        assert_eq!(
            choose_web_dir(
                None,
                Some(named.clone().into_os_string()),
                a_bundle_found_elsewhere(tmp.path())
            )
            .unwrap(),
            named
        );
        assert_eq!(
            choose_web_dir(
                Some(named.clone()),
                None,
                a_bundle_found_elsewhere(tmp.path())
            )
            .unwrap(),
            named
        );
    }

    /// The flag wins, and the variable is not even checked when it does: a
    /// stale export in a shell profile must not fail a command line that says
    /// exactly what to serve.
    #[test]
    fn the_flag_takes_precedence_over_the_variable() {
        let tmp = tempfile::tempdir().unwrap();
        let flag = an_interface_at(&tmp.path().join("flag"));
        let stale = tmp.path().join("stale").into_os_string();
        assert_eq!(
            choose_web_dir(
                Some(flag.clone()),
                Some(stale),
                a_bundle_found_elsewhere(tmp.path())
            )
            .unwrap(),
            flag
        );
    }

    #[test]
    fn a_blank_variable_is_not_a_choice() {
        let tmp = tempfile::tempdir().unwrap();
        for blank in ["", "  "] {
            let found = choose_web_dir(
                None,
                Some(blank.into()),
                a_bundle_found_elsewhere(tmp.path()),
            )
            .unwrap();
            assert!(
                found.ends_with("found-elsewhere"),
                "a blank value must fall through to the search, got {found:?}"
            );
        }
    }

    /// The tests above pass the variable in, so on their own they would pass
    /// against a build that never read it. This one goes through the real
    /// environment.
    #[test]
    fn serve_reads_the_variable_it_documents() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("no-interface-here");
        let _env =
            env_lock::lock_env([("BIOROUTER_SERVE_UI", Some(missing.display().to_string()))]);
        let err = resolve_web_dir(None)
            .expect_err("a variable naming a directory with no interface must be refused")
            .to_string();
        assert!(
            err.contains("the path came from BIOROUTER_SERVE_UI"),
            "{err}"
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
