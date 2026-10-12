//! `biorouter apps` subcommands — list, open, and serve the Biorouter apps
//! authored by Agent Drafter (design §7, Apps SDK v2 Phase 6 "CLI parity").
//!
//! Apps live on disk under `~/.config/biorouter/agent_drafter/<id>/`, each a
//! directory with a `manifest.json` plus its bundle. `biorouterd` serves them at
//! `/apps/<id>/`. These subcommands are a thin, dependency-light view over that
//! store:
//!
//! * `list`  — read every `<id>/manifest.json` (via `serde_json` directly, so no
//!   coupling to the store's internal `Manifest` shape) and print a table, or
//!   `--json` for machine output.
//! * `open`  — ensure a daemon is up and open `http://<host>:<port>/apps/<id>/`
//!   in the default browser, through a one-time link that never goes on a
//!   command line (see [`write_launch_page`]).
//! * `serve` — ensure a daemon is up, print the URL, and stay in the foreground
//!   until it is stopped (when it started the daemon) or return immediately with
//!   a note (when it reused a running one).
//!
//! ## Which of the two owns the daemon's lifetime
//!
//! `open` is done the moment the browser has the URL, so a daemon it started is
//! left running deliberately — closing it would close the page that was just
//! opened. `serve` is the opposite: it stays in the foreground *because* it owns
//! the daemon, so every way out of it must take the daemon with it. That is the
//! same guarantee `biorouter serve` makes, through the same two layers and the
//! same code ([`crate::commands::serve::stop_daemon`] and `--exit-with-parent`):
//! a signal handler installed BEFORE the spawn, every exit path routed through a
//! bounded stop-then-kill, and on Unix a daemon that stops itself when its
//! launcher is gone.
//!
//! ⚠ It handled `ctrl_c` alone, and its Ctrl-C arm killed the child while
//! SIGTERM did not run at all — the default action ended this process on the
//! spot and left the daemon holding the port, the app and the daemon's secret.
//! `apps open` must keep passing [`Supervision::Detached`]: tying its daemon to
//! this process would kill the daemon as `open` returned.
//!
//! **The profile's shared daemon comes first.** When the desktop app or
//! `biorouter crew` runs this profile's shared daemon, `open` and `serve` mint the
//! link there, over its private socket, and send the browser to that daemon's own
//! listener ([`shared_daemon_launch`]); nothing is started. They used to start a
//! second full daemon beside it that never exited. A port the person names in
//! `BIOROUTER_PORT` keeps its meaning: the shared daemon is used then only when
//! it listens on that port ([`named_port`]).
//!
//! **Otherwise daemon management is deliberately minimal.** From the configured
//! port (`BIOROUTER_PORT`, default 3000) up, `open`/`serve` health-check each port
//! via the auth-exempt `GET /status`, taking as a Biorouter daemon only one that
//! answers `ok` ([`port_state`]) and passing over a port anything else holds,
//! reuse a daemon whose key they know, and otherwise best-effort spawn the sibling
//! `biorouterd agent`. If no `biorouterd` binary can be located the command fails
//! with the exact command to run — an honest v1 over a fragile spawn.
//!
//! **Opening an app takes the daemon's secret (W2-HRD-1).** A daemon serves an
//! app's page only to a browser holding that app's access cookie, set by a
//! one-time link the daemon hands out on `POST /apps/<id>/launch`, which needs
//! the secret. The `/apps/<id>/` routes used to be auth-exempt, and any local
//! account could then read an app's socket token off its page. So a daemon these
//! commands start gets a secret they generate, and a daemon they did not start is
//! used only when `BIOROUTER_SERVER__SECRET_KEY` names its secret.
//!
//! Both commands remember the secret of a daemon they start, in a record only
//! this account can read, for as long as that daemon runs. A launch link works
//! once and for five minutes, so the next link has to come from somewhere: for a
//! daemon `open` left running it is the next `open`, and for one `serve` holds in
//! the foreground it is an `open` (or a second `serve`) in another terminal.
//! `serve` used to keep its daemon's secret to itself, so the advice it printed,
//! to run it again, reached a daemon nothing could use: a second `serve` or
//! `open` refused it as unknown, and the only way on was to stop the first. The
//! record of a `serve` daemon is removed when `serve` stops it.
//!
//! In-terminal rendering of an app is explicitly OUT of scope (design §7); the
//! app always opens in a real browser.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use console::{style, Color};
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::commands::serve::{stop_daemon, StopSignals};

const ACCENT: Color = Color::Color256(137);

/// One row of the `apps list` output. Populated straight from `manifest.json`
/// so a manifest field the store doesn't (yet) model — like `archetype` — still
/// surfaces if present.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct AppRow {
    id: String,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    archetype: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    updated_at: u64,
}

/// The Agent Drafter store root (`~/.config/biorouter/agent_drafter`). Reuses the
/// same resolver the MCP server and `/apps` routes use so all three agree.
fn store_root() -> PathBuf {
    biorouter_mcp::agent_drafter::default_root()
}

/// Read every `<root>/<id>/manifest.json`, newest-updated first. Directories
/// without a readable/parsable manifest are skipped rather than failing the
/// whole listing.
fn collect_apps(root: &Path) -> Vec<AppRow> {
    let mut rows = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return rows;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let manifest_path = dir.join("manifest.json");
        let Ok(raw) = std::fs::read_to_string(&manifest_path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        let dir_id = entry.file_name().to_string_lossy().to_string();
        let id = value
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or(&dir_id)
            .to_string();
        let title = value
            .get("title")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&id)
            .to_string();
        let archetype = value
            .get("archetype")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let kind = value
            .get("kind")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let updated_at = value
            .get("updated_at")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        rows.push(AppRow {
            id,
            title,
            archetype,
            kind,
            updated_at,
        });
    }
    rows.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id)));
    rows
}

/// Format a unix timestamp as a local `YYYY-MM-DD HH:MM`, or `-` when unknown.
fn fmt_updated(secs: u64) -> String {
    if secs == 0 {
        return "-".to_string();
    }
    match chrono::DateTime::from_timestamp(secs as i64, 0) {
        Some(dt) => dt
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        None => "-".to_string(),
    }
}

/// Render the human table as plain text (no ANSI), so it is unit-testable. The
/// styled header is printed separately by the handler.
fn format_table(rows: &[AppRow]) -> String {
    if rows.is_empty() {
        return "no apps installed".to_string();
    }
    let id_w = rows
        .iter()
        .map(|r| r.id.len())
        .chain(std::iter::once("ID".len()))
        .max()
        .unwrap_or(2);
    let title_w = rows
        .iter()
        .map(|r| r.title.len())
        .chain(std::iter::once("TITLE".len()))
        .max()
        .unwrap_or(5);
    let arch_w = rows
        .iter()
        .map(|r| r.archetype.as_deref().unwrap_or("-").len())
        .chain(std::iter::once("ARCHETYPE".len()))
        .max()
        .unwrap_or(9);

    let mut out = String::new();
    out.push_str(&format!(
        "{:<id_w$}  {:<title_w$}  {:<arch_w$}  {}\n",
        "ID", "TITLE", "ARCHETYPE", "UPDATED"
    ));
    for r in rows {
        out.push_str(&format!(
            "{:<id_w$}  {:<title_w$}  {:<arch_w$}  {}\n",
            r.id,
            r.title,
            r.archetype.as_deref().unwrap_or("-"),
            fmt_updated(r.updated_at),
        ));
    }
    out.trim_end().to_string()
}

/// Machine-readable JSON array of apps.
fn format_json(rows: &[AppRow]) -> Result<String> {
    Ok(serde_json::to_string_pretty(rows)?)
}

// ──────────────────────────────────────────────────────────────────────────────
// list
// ──────────────────────────────────────────────────────────────────────────────

pub async fn handle_apps_list(json: bool) -> Result<()> {
    let root = store_root();
    let rows = collect_apps(&root);

    if json {
        println!("{}", format_json(&rows)?);
        return Ok(());
    }

    println!("  {} {}", style("▌").fg(ACCENT), style("Apps").bold());
    if rows.is_empty() {
        println!("    {}", style("none installed").dim());
        return Ok(());
    }
    for line in format_table(&rows).lines() {
        println!("    {line}");
    }
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// daemon plumbing (open / serve)
// ──────────────────────────────────────────────────────────────────────────────

pub(crate) const DAEMON_HOST: &str = "127.0.0.1";

/// The port `biorouterd agent` binds by default (`BIOROUTER_PORT`, else 3000 —
/// the same default `biorouter-server`'s `Settings` uses).
pub(crate) fn configured_port() -> u16 {
    std::env::var("BIOROUTER_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3000)
}

/// True if a Biorouter daemon answers `GET /status` on `host:port`. `/status` is
/// auth-exempt, so this needs no secret key. Implemented with a raw socket to
/// avoid pulling in an HTTP client dependency.
pub(crate) async fn daemon_ok(host: &str, port: u16) -> bool {
    let addr = format!("{host}:{port}");
    let connect = biorouter::net::connect_non_inheritable(&addr);
    let mut stream = match tokio::time::timeout(Duration::from_millis(600), connect).await {
        Ok(Ok(s)) => s,
        _ => return false,
    };
    let req = format!("GET /status HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    if stream.write_all(req.as_bytes()).await.is_err() {
        return false;
    }
    let mut buf = [0u8; 64];
    match tokio::time::timeout(Duration::from_millis(600), stream.read(&mut buf)).await {
        Ok(Ok(n)) if n > 0 => {
            let head = String::from_utf8_lossy(&buf[..n]);
            head.starts_with("HTTP/1.1 200") || head.starts_with("HTTP/1.0 200")
        }
        _ => false,
    }
}

/// What answers on a loopback port, as far as `apps open` and `apps serve` care.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PortState {
    /// Nothing accepts a connection: a daemon can be started here.
    Free,
    /// A Biorouter daemon: `GET /status` answers 200 with the body `ok`.
    Biorouter,
    /// Something else holds the port: a dev server's catch-all that answers 200
    /// to any path, a listener that never answers, another service.
    Foreign,
}

/// What answers on `port` (see [`PortState`]).
///
/// Stricter than [`daemon_ok`], which takes any 200 and which other commands
/// share: a Vite or CRA dev server on :3000 answers `/status` with its page, and
/// this command then said a Biorouter daemon whose key it did not know held the
/// port. HTTP/1.0, like [`with_secret`], so the body comes unchunked.
async fn port_state(port: u16) -> PortState {
    let addr = format!("{DAEMON_HOST}:{port}");
    let connect = biorouter::net::connect_non_inheritable(&addr);
    let mut stream = match tokio::time::timeout(Duration::from_millis(600), connect).await {
        Ok(Ok(stream)) => stream,
        _ => return PortState::Free,
    };
    let request = format!("GET /status HTTP/1.0\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).await.is_err() {
        return PortState::Foreign;
    }
    let mut response = Vec::new();
    let read = tokio::time::timeout(
        Duration::from_secs(2),
        (&mut stream).take(4096).read_to_end(&mut response),
    )
    .await;
    if !matches!(read, Ok(Ok(_))) {
        return PortState::Foreign;
    }
    let text = String::from_utf8_lossy(&response);
    match text.split_once("\r\n\r\n") {
        Some((head, body))
            if head.split_whitespace().nth(1) == Some("200") && body.trim() == "ok" =>
        {
            PortState::Biorouter
        }
        _ => PortState::Foreign,
    }
}

/// How many ports `apps open` and `apps serve` try, from the configured one up,
/// before giving up on finding one that is free or holds a daemon they can use.
/// The exported launchers try the same six (3000–3005 by default).
const PORT_ATTEMPTS: u16 = 6;

/// Locate the `biorouterd` binary: prefer the sibling of the running `biorouter`
/// executable (dev tree and installed app both colocate them), then the sibling
/// of the installation this copy came from (a Windows install has nothing else
/// to go on), else fall back to the bare name so the OS resolves it on `PATH`.
///
/// The whole rule lives in [`crate::commands::exe_path::biorouterd_for`], which
/// `serve` also calls — the two commands start the same daemon, and two copies
/// of "where is it" are two chances to fix only one of them.
fn biorouterd_path() -> PathBuf {
    use crate::commands::exe_path;
    exe_path::current_exe_resolved()
        .and_then(|exe| exe_path::biorouterd_for(&exe))
        .unwrap_or_else(|| PathBuf::from(exe_path::daemon_file_name()))
}

/// The command string we tell users to run when we can't (or won't) start the
/// daemon for them.
fn start_daemon_hint(port: u16) -> String {
    if port == 3000 {
        "start the daemon first: biorouterd agent".to_string()
    } else {
        format!("start the daemon first: BIOROUTER_PORT={port} biorouterd agent")
    }
}

/// Result of ensuring a daemon is reachable.
enum Daemon {
    /// A daemon was already listening; we did not start it.
    Reused,
    /// We spawned `biorouterd agent`; hold the child so `serve` can supervise it.
    Started {
        child: tokio::process::Child,
        /// Whether its secret was recorded, so that a later `apps open` or
        /// `apps serve` can mint another launch link on it while it runs.
        remembered: bool,
    },
}

/// One request to the daemon on `port`, carrying its secret. The status and the
/// body, or `None` when nothing answered in time. A raw socket like
/// [`daemon_ok`]; the body of a refusal or a launch link is small, so it is read
/// whole (bounded) until the daemon closes the connection.
///
/// ⚠ HTTP/1.0 on purpose. The daemon's compression layer makes every body's
/// length unknown up front, so an HTTP/1.1 answer comes chunked, and this reader
/// would hand the chunk sizes to the JSON parser (measured: the launch link was
/// refused while the daemon answered 200). A 1.0 answer is the plain body, ended
/// by the close.
async fn with_secret(port: u16, method: &str, path: &str, secret: &str) -> Option<(u16, String)> {
    let addr = format!("{DAEMON_HOST}:{port}");
    let connect = biorouter::net::connect_non_inheritable(&addr);
    let mut stream = tokio::time::timeout(Duration::from_secs(2), connect)
        .await
        .ok()?
        .ok()?;
    let request = format!(
        "{method} {path} HTTP/1.0\r\nHost: {addr}\r\nX-Secret-Key: {secret}\r\n\
         Content-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.ok()?;
    let mut response = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        (&mut stream).take(1 << 20).read_to_end(&mut response),
    )
    .await
    .ok()?
    .ok()?;
    let text = String::from_utf8_lossy(&response);
    let (head, body) = text.split_once("\r\n\r\n")?;
    let status = head.split_whitespace().nth(1)?.parse().ok()?;
    Some((status, body.to_string()))
}

/// Whether `secret` is the secret of the daemon on `port`.
async fn secret_works(port: u16, secret: &str) -> bool {
    matches!(
        with_secret(port, "GET", "/apps", secret).await,
        Some((200, _))
    )
}

/// A secret for a daemon this command starts: 32 bytes from the system generator.
fn new_daemon_secret() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Where `apps open` and `apps serve` remember the secret of the daemon they
/// started on `port`: a record readable by this account alone, in Biorouter's
/// state folder.
fn remembered_daemon_path(port: u16) -> PathBuf {
    biorouter::config::paths::Paths::state_dir()
        .join("apps-daemon")
        .join(format!("{port}.json"))
}

#[derive(serde::Deserialize, Serialize)]
struct RememberedDaemon {
    port: u16,
    secret: String,
}

/// Record `secret` as the secret of the daemon this command started on `port`,
/// and answer whether the record was written. Without it a later `apps open` or
/// `apps serve` cannot mint a launch link on this daemon while it runs.
fn remember_daemon(port: u16, secret: &str) -> bool {
    let path = remembered_daemon_path(port);
    let written = path
        .parent()
        .ok_or_else(|| anyhow!("no folder for {}", path.display()))
        .and_then(biorouter::daemon_runtime::private_directory)
        .and_then(|()| {
            biorouter::daemon_runtime::write_private(
                &path,
                &RememberedDaemon {
                    port,
                    secret: secret.to_string(),
                },
            )
        });
    match written {
        Ok(()) => true,
        Err(error) => {
            eprintln!(
                "    {} could not record this daemon's key ({error}), so a later \
                 `biorouter apps open` cannot use this daemon",
                style("!").yellow()
            );
            false
        }
    }
}

fn remembered_secret(port: u16) -> Option<String> {
    let record: RememberedDaemon =
        biorouter::daemon_runtime::read_private(&remembered_daemon_path(port)).ok()?;
    (record.port == port).then_some(record.secret)
}

/// Remove the record of the daemon on `port` once `serve` has stopped it, if the
/// record still names `secret`. A record naming another secret describes a
/// daemon started on the port since, and is left alone.
fn forget_daemon(port: u16, secret: &str) {
    if remembered_secret(port).is_some_and(|recorded| recorded == secret) {
        let _ = std::fs::remove_file(remembered_daemon_path(port));
    }
}

/// How long the daemon keeps a launch link open, in the words the person reads.
/// The daemon's own figure is `APP_LAUNCH_TTL` in `biorouter-server`'s `auth`,
/// which this crate cannot import; a test pins the two together.
const LAUNCH_LINK_LIFETIME: &str = "five minutes";

/// The one-time address a browser opens app `id` at (W2-HRD-1): the daemon's
/// launch link, minted with its secret. Opening it sets the app's access cookie
/// and lands on the app; it works once, for [`LAUNCH_LINK_LIFETIME`].
async fn launch_url(port: u16, id: &str, secret: &str) -> Result<String> {
    // The id goes into a request line, so it must be an app name and nothing else.
    biorouter_mcp::agent_drafter::store::validate_artifact_id(id)
        .map_err(|_| anyhow!("'{id}' is not an app name"))?;
    let answer = with_secret(port, "POST", &format!("/apps/{id}/launch"), secret).await;
    let path = match &answer {
        Some((200, body)) => serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|value| value.get("path")?.as_str().map(str::to_string)),
        _ => None,
    };
    match path {
        Some(path) if is_launch_path(&path, id) => Ok(format!("http://{DAEMON_HOST}:{port}{path}")),
        _ => bail!(
            "the daemon on port {port} would not open '{id}'{}",
            match answer {
                Some((404, _)) => ": it has no such app".to_string(),
                Some((status, _)) => format!(" (HTTP {status})"),
                None => ": it did not answer".to_string(),
            }
        ),
    }
}

/// Whether `path` is exactly app `id`'s page with a launch token and nothing else.
fn is_launch_path(path: &str, id: &str) -> bool {
    path.strip_prefix(&format!("/apps/{id}/?t="))
        .is_some_and(|token| {
            token.len() == 64
                && token
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        })
}

/// How long a launch page is kept. Its link expired minutes before, so a page
/// this old opens nothing; it is removed the next time a page is written.
const LAUNCH_PAGE_STALE: Duration = Duration::from_secs(10 * 60);

/// Where `apps open` and `biorouter serve --open` write the pages that hand a
/// link carrying a credential to the browser.
pub(crate) fn launch_pages_dir() -> PathBuf {
    biorouter::config::paths::Paths::state_dir().join("app-launch")
}

/// Write `launch` into a page only this account can read, which sends the
/// browser on to it, and answer that page's path: the only thing an opener is
/// handed (W2-HRD-1).
///
/// ⚠ Never hand the link itself to `open`, `xdg-open` or a browser. Their
/// arguments are readable by every account on the machine (`ps` on macOS,
/// `/proc/<pid>/cmdline` on Linux, for as long as a browser started that way
/// runs). An app's launch link opens the app for whoever redeems it FIRST: a
/// co-tenant polling for `?t=` beats a browser that is still starting, keeps the
/// app's cookie for the daemon's run, and the owner sees only a refusal. The
/// browser token `biorouter serve` prints is worse, because it works until the
/// daemon stops and its cookie is handed the daemon's secret with the
/// interface. Jupyter hands its token over the same way. The page is mode 0600
/// in a 0700 folder this account owns, and names the link twice: a meta
/// refresh, and a link to click if the refresh does not run. The daemon answers
/// either link with a page of its own that sets the cookie, because a `file:`
/// page starting the navigation would keep a redirect from carrying it
/// (`routes::apps::launch_bounce`, `routes::web_ui::exchange_bounce`).
pub(crate) fn write_launch_page(directory: &Path, launch: &str) -> Result<PathBuf> {
    use std::io::Write;

    tighten_own_folder(directory);
    biorouter::daemon_runtime::private_directory(directory)?;
    remove_stale_launch_pages(directory, std::time::SystemTime::now());
    let target = html_attribute(launch);
    let mut page = tempfile::Builder::new()
        .prefix("launch-")
        .suffix(".html")
        .rand_bytes(16)
        .tempfile_in(directory)?;
    writeln!(
        page,
        "<!doctype html><meta charset=utf-8>\
         <meta name=referrer content=no-referrer>\
         <meta http-equiv=\"refresh\" content=\"0;url={target}\">\
         <title>Opening Biorouter</title>\
         <p>Opening Biorouter. If nothing happens, <a href=\"{target}\">open it here</a>.</p>"
    )?;
    page.as_file().sync_all()?;
    Ok(page.into_temp_path().keep()?)
}

/// Make `directory` 0700 when it is a real folder this account owns and others
/// can enter, so one loosened folder does not keep every later `apps open` from
/// opening a browser. A link, or another account's folder, is left for
/// `private_directory` to refuse.
fn tighten_own_folder(directory: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let Ok(folder) = std::fs::symlink_metadata(directory) else {
            return;
        };
        // SAFETY: geteuid has no preconditions and cannot fail.
        let own = folder.uid() == unsafe { libc::geteuid() };
        if folder.is_dir() && own && folder.permissions().mode() & 0o077 != 0 {
            let _ = std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700));
        }
    }
    #[cfg(not(unix))]
    let _ = directory;
}

/// Remove the launch pages in `directory` older than [`LAUNCH_PAGE_STALE`].
/// Best effort: a page left behind holds an expired link.
fn remove_stale_launch_pages(directory: &Path, now: std::time::SystemTime) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("launch-") && name.ends_with(".html")) {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= LAUNCH_PAGE_STALE);
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// `value` escaped for a double-quoted HTML attribute.
fn html_attribute(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Open `launch`, a link that carries a credential, in the default browser
/// through a launch page in `directory`, calling `opener` with the page and
/// never with the link. Answers the page, for a caller that removes it later, or
/// what went wrong, for the caller to print the link instead.
pub(crate) fn open_launch_link(
    launch: &str,
    directory: &Path,
    opener: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<PathBuf> {
    let page = write_launch_page(directory, launch)
        .map_err(|error| anyhow!("could not write the page that opens the browser: {error}"))?;
    match opener(&page) {
        Ok(()) => Ok(page),
        Err(error) => {
            let _ = std::fs::remove_file(&page);
            Err(anyhow!("could not open a browser: {error}"))
        }
    }
}

/// Open the launch page `page` in the default browser. Only a launch page is
/// ever handed to this: see [`write_launch_page`] for why the link itself never
/// reaches an opener.
///
/// A browser, and not whatever opens `.html` files, which on a developer's
/// machine is often an editor: `webbrowser` asks the platform for the handler of
/// `https:` and opens the page in it. The page goes over as its `file:` address.
/// On macOS that is a LaunchServices call, which starts no process whose
/// arguments could be read; elsewhere it is the browser's argument, which names
/// the page and not the link.
pub(crate) fn open_page_in_browser(page: &Path) -> std::io::Result<()> {
    let page_url = url::Url::from_file_path(page).map_err(|()| {
        std::io::Error::other(format!("{} is not an absolute path", page.display()))
    })?;
    webbrowser::open(page_url.as_str())
}

/// `CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW`: a detached daemon's own
/// process group, and no console to receive this one's Ctrl-C or close.
#[cfg(windows)]
const DETACHED_DAEMON_FLAGS: u32 = 0x0000_0200 | 0x0800_0000;

/// Whether a daemon this command starts is tied to this process's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Supervision {
    /// `apps open`: the daemon outlives this command, which is the point — the
    /// page it just opened needs it.
    Detached,
    /// `apps serve`: the daemon is this process's to stop, however this process
    /// ends. On Unix it is also told to stop itself if this process is gone,
    /// which covers the endings that run no code here at all.
    TiedToThisProcess,
}

/// The secret of the daemon on `port`, when this command knows one that works:
/// `BIOROUTER_SERVER__SECRET_KEY`, or the record of a daemon `apps open` or
/// `apps serve` started there.
async fn known_secret(port: u16) -> Option<String> {
    let known = std::env::var("BIOROUTER_SERVER__SECRET_KEY")
        .ok()
        .filter(|secret| !secret.is_empty())
        .into_iter()
        .chain(remembered_secret(port));
    for secret in known {
        if secret_works(port, &secret).await {
            return Some(secret);
        }
    }
    None
}

/// Ensure a daemon whose secret this command knows is reachable, from the
/// configured port up, spawning one if needed. Answers the daemon, its secret and
/// the port it is on.
///
/// A port held by something that is not Biorouter is passed over for the next,
/// as the exported launchers do; it used to stop the command with a claim that a
/// Biorouter daemon whose key it lacked was running there. A Biorouter daemon on
/// the configured port whose key this command does not know still stops it: that
/// is a daemon the person pointed it at.
async fn ensure_daemon(first: u16, supervision: Supervision) -> Result<(Daemon, String, u16)> {
    let mut port = first;
    loop {
        match port_state(port).await {
            PortState::Free => break,
            PortState::Biorouter => {
                if let Some(secret) = known_secret(port).await {
                    return Ok((Daemon::Reused, secret, port));
                }
                if port == first {
                    bail!(
                        "a Biorouter daemon is already running on port {port}, and opening an app on it takes \
                         that daemon's secret key, which this command does not know. It knows the key of a \
                         daemon `biorouter apps open` or `biorouter apps serve` started; for any other, set \
                         BIOROUTER_SERVER__SECRET_KEY to its key, or set BIOROUTER_PORT to a free port so \
                         this command starts a daemon of its own."
                    );
                }
                eprintln!(
                    "    {} port {port} has a Biorouter daemon this command has no key for; trying the next",
                    style("·").dim()
                );
            }
            PortState::Foreign => eprintln!(
                "    {} port {port} is in use by something that is not Biorouter; trying the next",
                style("·").dim()
            ),
        }
        port = match port.checked_add(1) {
            Some(next) if next - first < PORT_ATTEMPTS => next,
            _ => bail!(
                "no port from {first} to {port} is free for a Biorouter daemon. Set BIOROUTER_PORT \
                 to a free port."
            ),
        };
    }

    let secret = new_daemon_secret();
    let bin = biorouterd_path();
    let mut command = tokio::process::Command::new(&bin);
    command.arg("agent");
    // ⚠ `Detached` must pass nothing: the flag makes the daemon stop when this
    // process is gone, and `apps open` is gone as soon as it has opened the page.
    #[cfg(unix)]
    if supervision == Supervision::TiedToThisProcess {
        command
            .arg("--exit-with-parent")
            .arg(std::process::id().to_string());
    }
    // A daemon `apps open` leaves running must not share this console. Windows
    // sends Ctrl-C, and the close of the console window, to every process
    // attached to it, and the daemon stops on either: the next Ctrl-C at that
    // prompt, or closing the window, took the open app down with it. Its own
    // process group and no console, as `run.ps1` gives the daemon it starts.
    #[cfg(windows)]
    if supervision == Supervision::Detached {
        command.creation_flags(DETACHED_DAEMON_FLAGS);
    }
    let mut child = command
        .env("BIOROUTER_PORT", port.to_string())
        // The secret rides the environment, which only this account can read,
        // and never the command line.
        .env("BIOROUTER_SERVER__SECRET_KEY", &secret)
        // Issue #56 DR-16: `biorouterd agent` now reads one line off stdin at
        // startup (the launcher's user-action digest). `Command` INHERITS fd 0,
        // so without this the spawned daemon would consume a line of the CLI's
        // own stdin whenever that stdin is not a terminal — stealing input meant
        // for `biorouter`, or adding a 2s stall to daemon startup if the pipe is
        // open and idle. `/dev/null` is not a terminal and is instantly at EOF,
        // so the daemon reads nothing, installs no digest, and fails closed —
        // which is correct for a launcher that mints no key.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        // A backstop for a panic unwinding out of `serve` before its stop runs.
        // Every ordinary path goes through `stop_daemon`, which asks first.
        // Harmless for `Detached`, whose child is deliberately leaked below.
        .kill_on_drop(supervision == Supervision::TiedToThisProcess)
        .spawn()
        .map_err(|e| {
            anyhow!(
                "could not start biorouterd ({e}). {}",
                start_daemon_hint(port)
            )
        })?;

    // Poll for readiness, but bail early if the child dies (e.g. the port is
    // already taken by a non-Biorouter process).
    //
    // The same minute `biorouter serve` allows. It was 25s, which a debug daemon
    // on a loaded machine can miss — and missing it kills a daemon that was
    // about to answer and reports a failure that is only a slow start.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            bail!(
                "biorouterd exited before becoming ready (status: {status}). \
                 Is port {port} already in use? Try a different BIOROUTER_PORT."
            );
        }
        if secret_works(port, &secret).await {
            // Both supervisions: the next launch link for this daemon comes from
            // a later `apps open` or `apps serve`, which can mint it only with
            // this secret. `serve` removes the record when it stops the daemon.
            let remembered = remember_daemon(port, &secret);
            return Ok((Daemon::Started { child, remembered }, secret, port));
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = child.start_kill();
            bail!("biorouterd did not become ready on port {port} within 60s");
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

/// Confirm the app exists on disk before we bother a daemon.
fn require_app(id: &str) -> Result<()> {
    let manifest = store_root().join(id).join("manifest.json");
    if !manifest.exists() {
        bail!(
            "no app '{id}' in the store ({}). Run `biorouter apps list` to see installed apps.",
            store_root().display()
        );
    }
    Ok(())
}

fn app_url(port: u16, id: &str) -> String {
    format!("http://{DAEMON_HOST}:{port}/apps/{id}/")
}

/// The command that mints another launch link for app `id` on the daemon on
/// `port`, spelled so it reaches that port.
fn open_again_command(port: u16, id: &str) -> String {
    if port == 3000 {
        format!("biorouter apps open {id}")
    } else {
        format!("BIOROUTER_PORT={port} biorouter apps open {id}")
    }
}

/// What `apps serve` tells the person about the link it printed, and where the
/// next one comes from. Each answer is true of the daemon it describes: one it
/// started and recorded, one it started and could not record, or one it reused
/// with a key it already knew.
fn serve_link_advice(port: u16, id: &str, started: Option<bool>) -> String {
    let once = format!("the address below opens the app once, within {LAUNCH_LINK_LIFETIME}");
    match started {
        Some(true) => format!(
            "{once}. For another, run `{}` in another terminal while this one keeps serving.",
            open_again_command(port, id)
        ),
        Some(false) => format!("{once}. For another, stop this with Ctrl-C and run it again."),
        None => format!("{once}. Run this again for another."),
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// the profile's shared daemon
// ──────────────────────────────────────────────────────────────────────────────

/// `base` as an origin a browser on this machine is sent to: plain http to a
/// loopback host on an explicit port. A daemon listening on every interface names
/// `0.0.0.0` or `[::]`, an address to listen on and not one to visit, so it
/// becomes the loopback address of its family. `None` for anything else.
// Reached only through the shared daemon, which is unix-only for now.
#[cfg_attr(not(unix), allow(dead_code))]
fn loopback_origin(base: &str) -> Option<String> {
    let url = url::Url::parse(base).ok()?;
    if url.scheme() != "http"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let port = url.port()?;
    let host = match url.host_str()? {
        "127.0.0.1" | "0.0.0.0" => "127.0.0.1",
        "localhost" => "localhost",
        "[::1]" | "[::]" => "[::1]",
        _ => return None,
    };
    Some(format!("http://{host}:{port}"))
}

/// The address a browser on this machine opens app `id` at, from the shared
/// daemon's launch answer: the link on the daemon's own listener. That is the
/// answer's `url` when it names one, else the link's path on `listener` (the
/// daemon's `/headless/health` `api_base_url`, for a daemon from before the
/// answer named it). `None` unless the answer is exactly `id`'s page with a
/// token, on a loopback listener.
#[cfg_attr(not(unix), allow(dead_code))]
fn shared_launch_link(
    answer: &serde_json::Value,
    id: &str,
    listener: Option<&str>,
) -> Option<String> {
    let path = answer.get("path")?.as_str()?;
    if !is_launch_path(path, id) {
        return None;
    }
    match answer.get("url") {
        Some(serde_json::Value::String(url)) => {
            let origin = url.strip_suffix(path)?;
            (loopback_origin(origin)? == origin).then(|| url.clone())
        }
        None | Some(serde_json::Value::Null) => {
            Some(format!("{}{path}", loopback_origin(listener?)?))
        }
        Some(_) => None,
    }
}

/// Mint app `id`'s launch link on the shared daemon `client` reaches.
#[cfg(unix)]
async fn launch_on_shared_daemon(
    client: &crate::daemon_client::CrewClient,
    id: &str,
) -> Result<String> {
    // The id goes into a request line, so it must be an app name and nothing else.
    biorouter_mcp::agent_drafter::store::validate_artifact_id(id)
        .map_err(|_| anyhow!("'{id}' is not an app name"))?;
    // `request_text`, not `request`: the daemon refuses a missing app in plain
    // text, which `request` reports only as "not JSON".
    let answer = client
        .request_text("POST", &format!("/apps/{id}/launch"), None)
        .await
        .map_err(
            |error| match error.downcast_ref::<crate::daemon_client::DaemonRefusal>() {
                Some(refusal) if refusal.status == 404 => {
                    anyhow!(
                        "this profile's Biorouter daemon would not open '{id}': it has no such app"
                    )
                }
                _ => anyhow!("this profile's Biorouter daemon would not open '{id}': {error}"),
            },
        )?;
    let answer: serde_json::Value = serde_json::from_str(&answer).unwrap_or_default();
    let listener = if answer.get("url").is_some_and(|url| !url.is_null()) {
        None
    } else {
        client
            .request("GET", "/headless/health", None)
            .await
            .ok()
            .and_then(|health| Some(health.get("api_base_url")?.as_str()?.to_string()))
    };
    shared_launch_link(&answer, id, listener.as_deref()).ok_or_else(|| {
        anyhow!("this profile's Biorouter daemon answered with an unexpected address for '{id}'")
    })
}

/// App `id`'s launch link on this profile's shared daemon, the one the desktop
/// app and `biorouter crew` run, when one is running and this command can use
/// it: `Ok(None)` sends the caller on to a daemon of its own.
///
/// These commands used to look only at `BIOROUTER_PORT` (default 3000), so with
/// the desktop app open they started a second full daemon, with its own MCP
/// servers and scheduler, that never exited. The shared daemon is reached over
/// its private socket with the key its starter saved, as every `crew` command
/// reaches it; nothing is started here (`connect(true)`), and a daemon this
/// command cannot use (started by an older Biorouter, or its key refused) is left
/// running and passed over, with a line saying why.
///
/// A port named in `BIOROUTER_PORT` is honoured: see [`named_port`].
async fn shared_daemon_launch(id: &str) -> Result<Option<String>> {
    let named = named_port(std::env::var("BIOROUTER_PORT").ok().as_deref());
    shared_daemon_launch_on(id, named).await
}

/// The port the person named in `BIOROUTER_PORT` (`value`), if they named one.
///
/// Someone who runs `BIOROUTER_PORT=8080 biorouter apps serve <id>` to match an
/// SSH tunnel or a firewall rule means that port, and the shared daemon listens
/// on one of its own choosing. So a named port sends these commands past the
/// shared daemon unless it is the port that daemon listens on
/// ([`on_named_port`]). ⚠ Not "skip the shared daemon whenever the variable is
/// set": a shell the agent runs inside the shared daemon inherits the daemon's
/// own `BIOROUTER_PORT`, and that shell is where the agent itself runs
/// `apps open`. The desktop starts the daemon on a port it names, which then
/// matches; `biorouter crew` starts it with `0`, which asks for any free port and
/// names none.
fn named_port(value: Option<&str>) -> Option<u16> {
    value?.trim().parse::<u16>().ok().filter(|port| *port != 0)
}

/// Whether `link`, minted on the shared daemon, is on the port the person named.
#[cfg_attr(not(unix), allow(dead_code))]
fn on_named_port(link: &str, named: Option<u16>) -> bool {
    match named {
        None => true,
        Some(port) => url::Url::parse(link).ok().and_then(|url| url.port()) == Some(port),
    }
}

/// [`shared_daemon_launch`], for the port the person named (`named`).
async fn shared_daemon_launch_on(id: &str, named: Option<u16>) -> Result<Option<String>> {
    #[cfg(not(unix))]
    {
        let _ = (id, named);
        Ok(None)
    }
    #[cfg(unix)]
    {
        use crate::daemon_client::{CrewClient, Restated, DAEMON_NOT_RUNNING_CODE};
        let client = match CrewClient::connect(true).await {
            Ok(client) => client,
            Err(error) => {
                let absent = error
                    .downcast_ref::<Restated>()
                    .is_some_and(|restated| restated.code == Some(DAEMON_NOT_RUNNING_CODE));
                if !absent {
                    let reason = error.to_string();
                    eprintln!(
                        "    {} not using this profile's Biorouter daemon: {}",
                        style("·").dim(),
                        reason.lines().next().unwrap_or_default()
                    );
                }
                return Ok(None);
            }
        };
        // A named port is checked before anything is minted, so a shared daemon
        // on another port is passed over without a link being made for nobody,
        // and without its refusals deciding a command meant for another daemon.
        if let Some(port) = named {
            let listener = client
                .request("GET", "/headless/health", None)
                .await
                .ok()
                .and_then(|health| Some(health.get("api_base_url")?.as_str()?.to_string()));
            if !listener
                .as_deref()
                .is_some_and(|listener| on_named_port(listener, named))
            {
                eprintln!(
                    "    {} not using this profile's Biorouter daemon, which listens on {}: \
                     BIOROUTER_PORT names port {port}. Unset BIOROUTER_PORT to use it.",
                    style("·").dim(),
                    listener.as_deref().unwrap_or("another port"),
                );
                return Ok(None);
            }
        }
        let link = launch_on_shared_daemon(&client, id).await?;
        // And the link itself, which is what the browser is sent to.
        Ok(on_named_port(&link, named).then_some(link))
    }
}

/// Open `launch` in the default browser through a private launch page, and print
/// it for when no browser opened or one could not read the page.
fn open_or_print(launch: &str) {
    // The link itself never reaches an opener's command line: see
    // `write_launch_page`. The terminal is this account's alone, so the link is
    // printed there when no browser could be opened.
    //
    // It is printed when the page did open, too: a browser that cannot read the
    // page (a snap-packaged browser cannot read hidden folders such as the state
    // dir) shows an error, and `open` still reports success.
    let once = format!("it works once, within {LAUNCH_LINK_LIFETIME}");
    match open_launch_link(launch, &launch_pages_dir(), open_page_in_browser) {
        Ok(_) => println!(
            "  {} if the app does not appear, open this address instead ({once}): {launch}",
            style("·").dim()
        ),
        Err(e) => {
            eprintln!("    {} {e}", style("!").yellow());
            println!(
                "  {} open this address instead ({once}): {launch}",
                style("·").dim()
            );
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// open
// ──────────────────────────────────────────────────────────────────────────────

pub async fn handle_apps_open(id: String) -> Result<()> {
    require_app(&id)?;
    if let Some(launch) = shared_daemon_launch(&id).await? {
        println!(
            "  {} using this profile's Biorouter daemon",
            style("·").dim()
        );
        open_or_print(&launch);
        return Ok(());
    }
    let (daemon, secret, port) = ensure_daemon(configured_port(), Supervision::Detached).await?;
    let url = app_url(port, &id);
    let launch = launch_url(port, &id, &secret).await?;

    match daemon {
        Daemon::Reused => {
            println!(
                "  {} using the daemon already running on port {port}",
                style("·").dim()
            );
        }
        Daemon::Started { .. } => {
            // Leave the child running (detached) so the app stays served after
            // this command returns — the tokio child is not killed on drop.
            println!("  {} started biorouterd on port {port}", style("✓").green());
        }
    }

    open_or_print(&launch);
    println!("  {} {}", style("→").fg(ACCENT), style(&url).bold());
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// serve
// ──────────────────────────────────────────────────────────────────────────────

pub async fn handle_apps_serve(id: String) -> Result<()> {
    require_app(&id)?;
    // The profile's shared daemon is already served by whoever runs it, so there
    // is nothing to hold in the foreground: print a link and return, as for any
    // daemon this command did not start.
    if let Some(launch) = shared_daemon_launch(&id).await? {
        println!(
            "  {} the address below opens the app once, within {LAUNCH_LINK_LIFETIME}. \
             Run this again for another.",
            style("·").dim()
        );
        println!("  {} {}", style("→").fg(ACCENT), style(&launch).bold());
        println!(
            "  {} served by this profile's Biorouter daemon; it keeps running after this command.",
            style("·").dim()
        );
        return Ok(());
    }

    // ⚠ BEFORE the spawn, and before anything that can park. A handler replaces
    // the default action — which for SIGTERM is to end this process on the spot
    // and leave the daemon behind — so from here on a signal waits to be read,
    // including one that lands during the readiness wait inside `ensure_daemon`.
    let mut stop = StopSignals::install()?;
    let (daemon, secret, port) =
        ensure_daemon(configured_port(), Supervision::TiedToThisProcess).await?;
    let url = match launch_url(port, &id, &secret).await {
        Ok(url) => url,
        Err(error) => {
            if let Daemon::Started { mut child, .. } = daemon {
                stop_daemon(&mut child, &mut stop).await;
                forget_daemon(port, &secret);
            }
            return Err(error);
        }
    };
    let started = match &daemon {
        Daemon::Reused => None,
        Daemon::Started { remembered, .. } => Some(*remembered),
    };
    println!(
        "  {} {}",
        style("·").dim(),
        serve_link_advice(port, &id, started)
    );

    match daemon {
        Daemon::Reused => {
            // Reusing an external daemon: print the URL and a note, then exit 0.
            // We do not own its lifecycle, so there is nothing to keep in the
            // foreground — and nothing of ours to stop, or to forget.
            println!("  {} {}", style("→").fg(ACCENT), style(&url).bold());
            println!(
                "  {} reusing the daemon already running on port {port}; it keeps running after this command.",
                style("·").dim()
            );
            Ok(())
        }
        Daemon::Started { mut child, .. } => {
            println!("  {} {}", style("→").fg(ACCENT), style(&url).bold());
            println!(
                "  {} serving on port {port}. Press Ctrl-C to stop.",
                style("✓").green()
            );
            // Stay in the foreground until the daemon exits or we are asked to
            // stop — and then stop it, whichever of the two happened.
            tokio::select! {
                status = child.wait() => {
                    match status {
                        Ok(s) => println!("  {} biorouterd exited ({s})", style("·").dim()),
                        Err(e) => eprintln!("  {} waiting on biorouterd failed: {e}", style("!").yellow()),
                    }
                }
                _ = stop.recv() => {
                    println!("\n  {} stopping biorouterd…", style("·").dim());
                }
            }
            // Asks, waits out the grace, then kills and reaps. A daemon that has
            // already exited is only reaped, so this is safe on both arms.
            stop_daemon(&mut child, &mut stop).await;
            // After the stop, not before: until the daemon is gone an `apps open`
            // may still use it. A daemon started on the port since has a record
            // of its own, which this leaves alone.
            forget_daemon(port, &secret);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_manifest(root: &Path, id: &str, json: &str) {
        let dir = root.join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("manifest.json"), json).unwrap();
    }

    #[test]
    fn empty_store_lists_nothing() {
        let tmp = TempDir::new().unwrap();
        let rows = collect_apps(tmp.path());
        assert!(rows.is_empty());
        assert_eq!(format_table(&rows), "no apps installed");
        assert_eq!(format_json(&rows).unwrap(), "[]");
    }

    #[test]
    fn missing_store_dir_does_not_panic() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("does-not-exist");
        let rows = collect_apps(&missing);
        assert!(rows.is_empty());
    }

    #[test]
    fn collects_and_sorts_by_updated_desc() {
        let tmp = TempDir::new().unwrap();
        write_manifest(
            tmp.path(),
            "alpha",
            r#"{"id":"alpha","title":"Alpha","kind":"static","updated_at":100}"#,
        );
        write_manifest(
            tmp.path(),
            "beta",
            r#"{"id":"beta","title":"Beta App","kind":"agentic","archetype":"dashboard","updated_at":300}"#,
        );
        write_manifest(
            tmp.path(),
            "gamma",
            r#"{"id":"gamma","title":"Gamma","kind":"static","updated_at":200}"#,
        );
        // A directory without a manifest is skipped, not fatal.
        fs::create_dir_all(tmp.path().join("no-manifest")).unwrap();
        // A malformed manifest is skipped too.
        write_manifest(tmp.path(), "broken", "{not json");

        let rows = collect_apps(tmp.path());
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["beta", "gamma", "alpha"]);
        assert_eq!(rows[0].archetype.as_deref(), Some("dashboard"));
        assert_eq!(rows[0].kind.as_deref(), Some("agentic"));
        assert!(rows[2].archetype.is_none());
    }

    #[test]
    fn title_falls_back_to_id_when_absent() {
        let tmp = TempDir::new().unwrap();
        write_manifest(tmp.path(), "solo", r#"{"id":"solo","updated_at":1}"#);
        let rows = collect_apps(tmp.path());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "solo");
    }

    #[test]
    fn table_contains_headers_and_values() {
        let tmp = TempDir::new().unwrap();
        write_manifest(
            tmp.path(),
            "beta",
            r#"{"id":"beta","title":"Beta App","archetype":"dashboard","updated_at":300}"#,
        );
        let rows = collect_apps(tmp.path());
        let table = format_table(&rows);
        assert!(table.contains("ID"));
        assert!(table.contains("TITLE"));
        assert!(table.contains("ARCHETYPE"));
        assert!(table.contains("UPDATED"));
        assert!(table.contains("beta"));
        assert!(table.contains("Beta App"));
        assert!(table.contains("dashboard"));
        // Archetype-less rows render a dash placeholder.
        write_manifest(tmp.path(), "plain", r#"{"id":"plain","updated_at":1}"#);
        let rows = collect_apps(tmp.path());
        assert!(format_table(&rows).contains(" - "));
    }

    #[test]
    fn json_output_is_valid_array_of_apps() {
        let tmp = TempDir::new().unwrap();
        write_manifest(
            tmp.path(),
            "beta",
            r#"{"id":"beta","title":"Beta App","archetype":"dashboard","kind":"agentic","updated_at":300}"#,
        );
        let rows = collect_apps(tmp.path());
        let json = format_json(&rows).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], "beta");
        assert_eq!(arr[0]["title"], "Beta App");
        assert_eq!(arr[0]["archetype"], "dashboard");
        assert_eq!(arr[0]["kind"], "agentic");
        assert_eq!(arr[0]["updated_at"], 300);
    }

    #[test]
    fn json_omits_absent_optional_fields() {
        let tmp = TempDir::new().unwrap();
        write_manifest(tmp.path(), "plain", r#"{"id":"plain","updated_at":1}"#);
        let rows = collect_apps(tmp.path());
        let json = format_json(&rows).unwrap();
        assert!(!json.contains("archetype"));
        assert!(!json.contains("\"kind\""));
    }

    #[test]
    fn configured_port_defaults_to_3000() {
        // No env manipulation (tests share a process); just assert the parse of
        // an explicit value and the default fall-through logic via fmt.
        assert_eq!(
            start_daemon_hint(3000),
            "start the daemon first: biorouterd agent"
        );
        assert_eq!(
            start_daemon_hint(8080),
            "start the daemon first: BIOROUTER_PORT=8080 biorouterd agent"
        );
    }

    #[test]
    fn app_url_is_well_formed() {
        assert_eq!(app_url(3000, "beta"), "http://127.0.0.1:3000/apps/beta/");
    }

    /// W2-HRD-1: only exactly the app's page with a launch token is opened.
    #[test]
    fn only_the_apps_own_launch_link_is_opened() {
        let token = "ab".repeat(32);
        assert!(is_launch_path(&format!("/apps/beta/?t={token}"), "beta"));
        for path in [
            format!("/apps/other/?t={token}"),
            format!("/apps/beta/?t={token}&next=/sessions"),
            format!("/apps/beta/?t={}", token.to_uppercase()),
            format!("/apps/beta/?t={}", "ab".repeat(31)),
            format!("/apps/beta/agent?t={token}"),
            format!("//evil.test/apps/beta/?t={token}"),
            "/apps/beta/".to_string(),
        ] {
            assert!(!is_launch_path(&path, "beta"), "{path}");
        }
    }

    fn a_launch_link() -> String {
        format!("http://127.0.0.1:3000/apps/beta/?t={}", "ab".repeat(32))
    }

    /// W2-HRD-1: the opener is handed a page only this account can read, and
    /// never the link. The link opens the app for whoever redeems it first, and
    /// an opener's arguments are readable by every account on the machine.
    #[test]
    fn the_opener_is_handed_a_private_page_and_never_the_link() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("app-launch");
        let launch = a_launch_link();
        let mut handed = Vec::new();
        let written = open_launch_link(&launch, &dir, |page| {
            handed.push(page.to_path_buf());
            Ok(())
        })
        .unwrap();

        assert_eq!(handed.len(), 1);
        let page = &handed[0];
        assert_eq!(
            &written, page,
            "the caller is told which page to remove later"
        );
        let argument = page.to_string_lossy();
        assert!(!argument.contains("?t="), "{argument}");
        assert!(!argument.contains(&"ab".repeat(32)), "{argument}");
        assert_eq!(page.parent(), Some(dir.as_path()));
        let html = fs::read_to_string(page).unwrap();
        assert!(
            html.contains(&format!("content=\"0;url={launch}\"")),
            "{html}"
        );
        assert!(html.contains(&format!("href=\"{launch}\"")), "{html}");
        assert!(html.contains("<meta name=referrer content=no-referrer>"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(page), 0o600, "the page is this account's alone");
            assert_eq!(mode(&dir), 0o700, "and so is its folder");
        }
    }

    /// With no browser to open, the caller learns it and prints the link to the
    /// terminal instead; the page is still this account's alone.
    #[test]
    fn a_failed_opener_is_reported_so_the_link_is_printed_instead() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("app-launch");
        let error = open_launch_link(&a_launch_link(), &dir, |_| {
            Err(std::io::Error::other("no display"))
        })
        .unwrap_err();
        assert!(error.to_string().contains("no display"), "{error}");
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            0,
            "a page no browser will read is not left holding the link"
        );
    }

    /// A folder of ours that others can enter is closed to them before a page
    /// goes in it, rather than left to stop every later `apps open`.
    #[cfg(unix)]
    #[test]
    fn a_loose_folder_of_ours_is_tightened_before_a_page_goes_in() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("app-launch");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        let page = write_launch_page(&dir, &a_launch_link()).unwrap();
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&page).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    /// A folder that is a link is not used for a page, and nothing is opened:
    /// the caller prints the link instead.
    #[cfg(unix)]
    #[test]
    fn a_folder_that_is_a_link_is_refused() {
        let tmp = TempDir::new().unwrap();
        let elsewhere = tmp.path().join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        let dir = tmp.path().join("app-launch");
        std::os::unix::fs::symlink(&elsewhere, &dir).unwrap();
        let mut opened = false;
        let result = open_launch_link(&a_launch_link(), &dir, |_| {
            opened = true;
            Ok(())
        });
        assert!(result.is_err());
        assert!(!opened);
        assert_eq!(
            fs::read_dir(&elsewhere).unwrap().count(),
            0,
            "no page was written"
        );
    }

    /// Pages whose link has long expired are removed when the next is written;
    /// other files in the folder, and recent pages, are left alone.
    #[test]
    fn expired_launch_pages_are_swept() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("app-launch");
        let first = write_launch_page(&dir, &a_launch_link()).unwrap();
        let recent = write_launch_page(&dir, &a_launch_link()).unwrap();
        let other = dir.join("notes.txt");
        fs::write(&other, "keep").unwrap();
        let long_ago = std::time::SystemTime::now() - LAUNCH_PAGE_STALE - Duration::from_secs(1);
        fs::File::options()
            .write(true)
            .open(&first)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();
        let next = write_launch_page(&dir, &a_launch_link()).unwrap();
        assert!(!first.exists(), "an expired page is removed");
        assert!(recent.exists() && next.exists() && other.exists());
    }

    #[test]
    fn a_launch_page_escapes_what_it_quotes() {
        assert_eq!(
            html_attribute(r#"a&b"c'd<e>f"#),
            "a&amp;b&quot;c&#39;d&lt;e&gt;f"
        );
    }

    /// A source guard for the property above: this file opens a browser in one
    /// place, `open_page_in_browser`, which is handed the page and never the
    /// link, and no other opener appears at all.
    #[test]
    fn nothing_but_a_launch_page_reaches_an_opener() {
        let source = include_str!("apps.rs")
            .split("\n#[cfg(test)]\nmod tests {")
            .next()
            .unwrap();
        let calls: Vec<&str> = source.split("webbrowser::").skip(1).collect();
        assert_eq!(calls.len(), 1, "one browser opener, and only one");
        assert!(
            calls[0].starts_with("open(page_url.as_str())"),
            "webbrowser::{:.40}",
            calls[0]
        );
        for opener in [
            "open::that",
            "open::with",
            "open::commands",
            "Command::new(\"open\")",
            "Command::new(\"xdg-open\")",
        ] {
            assert!(!source.contains(opener), "{opener}");
        }
        assert!(source.contains("fn open_page_in_browser(page: &Path)"));
    }

    /// A page goes to the browser as a `file:` address, which needs an absolute
    /// path; a relative one is refused before any browser is asked.
    #[test]
    fn a_relative_page_is_not_opened() {
        let error = open_page_in_browser(Path::new("launch-relative.html")).unwrap_err();
        assert!(
            error.to_string().contains("not an absolute path"),
            "{error}"
        );
    }

    #[test]
    fn a_daemon_secret_is_64_hex_and_fresh() {
        let one = new_daemon_secret();
        assert_eq!(one.len(), 64);
        assert!(one.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(one, new_daemon_secret());
    }

    /// A stand-in for `biorouterd` on a free loopback port. `/status` answers
    /// anyone; `/apps` and `POST /apps/{app}/launch` answer only `secret`, and
    /// the launch route mints a fresh link each time, as the daemon's does.
    async fn a_daemon_with(secret: &'static str, app: &'static str) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut chunk = [0u8; 1024];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match stream.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => request.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let text = String::from_utf8_lossy(&request);
                    let mut line = text.lines().next().unwrap_or_default().split_whitespace();
                    let keyed = text
                        .lines()
                        .any(|header| header == format!("X-Secret-Key: {secret}"));
                    let (status, body) = match (line.next(), line.next()) {
                        (Some("GET"), Some("/status")) => ("200 OK", "ok".to_string()),
                        (Some("GET"), Some("/apps")) if keyed => ("200 OK", "[]".to_string()),
                        (Some("POST"), Some(path))
                            if keyed && path == format!("/apps/{app}/launch") =>
                        {
                            (
                                "200 OK",
                                serde_json::json!({
                                    "path": format!("/apps/{app}/?t={}", new_daemon_secret())
                                })
                                .to_string(),
                            )
                        }
                        _ => ("401 Unauthorized", String::new()),
                    };
                    let answer = format!(
                        "HTTP/1.0 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(answer.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        port
    }

    /// W2-HRD-1, the dead end `apps serve` left: its daemon's key was kept to
    /// itself, so once the one link it printed was spent or five minutes old, a
    /// second `apps serve` or `apps open` on the port refused the daemon as
    /// unknown and the only way on was to stop the first. Every started daemon
    /// is now recorded, and a second command mints another link on it.
    ///
    /// The first `ensure_daemon` below is the shipped behaviour for a `serve`
    /// daemon (nothing recorded); `tests/apps_serve_lifecycle.rs` covers the
    /// same with the real binaries, which is where the record is written.
    #[tokio::test]
    async fn a_daemon_apps_serve_started_gives_a_second_command_another_link() {
        const SECRET: &str = "5ec2e75ec2e75ec2e75ec2e75ec2e75ec2e75ec2e75ec2e75ec2e75ec2e75ec2";
        let port = a_daemon_with(SECRET, "beta").await;

        let unknown = match ensure_daemon(port, Supervision::TiedToThisProcess).await {
            Ok(_) => panic!("a daemon whose key was never recorded must be refused"),
            Err(refused) => refused.to_string(),
        };
        assert!(unknown.contains("does not know"), "{unknown}");

        assert!(remember_daemon(port, SECRET));
        let (daemon, secret, reused) = ensure_daemon(port, Supervision::TiedToThisProcess)
            .await
            .expect("a recorded daemon is reused");
        assert!(matches!(daemon, Daemon::Reused));
        assert_eq!(secret, SECRET);
        assert_eq!(reused, port);

        let first = launch_url(port, "beta", &secret).await.unwrap();
        let second = launch_url(port, "beta", &secret).await.unwrap();
        for link in [&first, &second] {
            let path = link
                .strip_prefix(&format!("http://127.0.0.1:{port}"))
                .unwrap();
            assert!(is_launch_path(path, "beta"), "{link}");
        }
        assert_ne!(first, second, "each command gets a link of its own");

        // A stop forgets only the record that names the daemon it stopped.
        forget_daemon(port, "a-daemon-started-on-the-port-since");
        assert_eq!(remembered_secret(port).as_deref(), Some(SECRET));
        forget_daemon(port, SECRET);
        assert_eq!(remembered_secret(port), None);
        assert!(!remembered_daemon_path(port).exists());
    }

    /// The line `apps serve` prints under its link is true of the daemon it
    /// describes. It said "run this again for another" of a daemon a second run
    /// could not use.
    #[test]
    fn the_advice_under_a_served_link_names_a_way_that_works() {
        let recorded = serve_link_advice(3000, "beta", Some(true));
        assert!(
            recorded.contains("`biorouter apps open beta`"),
            "{recorded}"
        );
        assert!(
            recorded.contains("while this one keeps serving"),
            "{recorded}"
        );
        assert!(!recorded.contains("run this again"), "{recorded}");
        assert!(serve_link_advice(4100, "beta", Some(true))
            .contains("`BIOROUTER_PORT=4100 biorouter apps open beta`"));
        let unrecorded = serve_link_advice(3000, "beta", Some(false));
        assert!(unrecorded.contains("stop this with Ctrl-C"), "{unrecorded}");
        assert!(!unrecorded.contains("apps open"), "{unrecorded}");
        for advice in [recorded, unrecorded, serve_link_advice(3000, "beta", None)] {
            assert!(advice.contains("once, within five minutes"), "{advice}");
        }
    }

    /// A server that answers every request with `status` and `body`, as a dev
    /// server's catch-all does, on `listener`.
    fn answer_everything(
        listener: tokio::net::TcpListener,
        status: &'static str,
        body: &'static str,
    ) {
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut chunk = [0u8; 1024];
                    let _ = stream.read(&mut chunk).await;
                    let answer = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(answer.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
    }

    /// Two listeners on adjacent loopback ports, the lower one first.
    async fn adjacent_listeners() -> (tokio::net::TcpListener, tokio::net::TcpListener) {
        for _ in 0..50 {
            let lower = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = lower.local_addr().unwrap().port();
            let Some(next) = port.checked_add(1) else {
                continue;
            };
            if let Ok(upper) = tokio::net::TcpListener::bind(("127.0.0.1", next)).await {
                return (lower, upper);
            }
        }
        panic!("no two adjacent loopback ports were free");
    }

    /// Only a daemon that says `ok` on `/status` is a Biorouter daemon. Any 200
    /// used to be, so a dev server's catch-all on :3000 stopped `apps open` with
    /// a claim that a Biorouter daemon whose key it lacked was running there.
    #[tokio::test]
    async fn only_a_status_that_says_ok_is_a_biorouter_daemon() {
        let daemon = a_daemon_with("k", "beta").await;
        assert_eq!(port_state(daemon).await, PortState::Biorouter);

        for (status, body) in [
            ("200 OK", "<!doctype html><div id=root></div>"),
            ("200 OK", ""),
            ("404 Not Found", "ok"),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            answer_everything(listener, status, body);
            assert_eq!(
                port_state(port).await,
                PortState::Foreign,
                "{status} {body:?}"
            );
        }

        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let free = closed.local_addr().unwrap().port();
        drop(closed);
        assert_eq!(port_state(free).await, PortState::Free);
    }

    /// A port held by something that is not Biorouter is passed over for the
    /// next, as the exported launchers do, and a daemon there that this command
    /// started is reused.
    #[tokio::test]
    async fn a_port_held_by_something_else_is_passed_over() {
        const SECRET: &str = "a11a11a11a11a11a11a11a11a11a11a11a11a11a11a11a11a11a11a11a11a11a";
        let (foreign, relay) = adjacent_listeners().await;
        let first = foreign.local_addr().unwrap().port();
        answer_everything(foreign, "200 OK", "<!doctype html>");
        let next = relay.local_addr().unwrap().port();
        // `a_daemon_with` binds a port of its own, so stand the daemon up on `next`
        // through the listener already holding it, which hands its connections to
        // the same stand-in. ⚠ Not dropped and bound again: macOS hands out
        // ephemeral ports in order, so the stand-in's own `:0` took `next` in the
        // gap and the second bind failed with `AddrInUse`.
        let daemon = a_daemon_with(SECRET, "beta").await;
        tokio::spawn(async move {
            while let Ok((mut inbound, _)) = relay.accept().await {
                tokio::spawn(async move {
                    if let Ok(mut outbound) =
                        tokio::net::TcpStream::connect(("127.0.0.1", daemon)).await
                    {
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                    }
                });
            }
        });
        assert!(remember_daemon(next, SECRET));

        let (daemon_state, secret, port) = ensure_daemon(first, Supervision::Detached)
            .await
            .expect("the daemon on the next port is reused");
        assert!(matches!(daemon_state, Daemon::Reused));
        assert_eq!((secret.as_str(), port), (SECRET, next));
        forget_daemon(next, SECRET);
    }

    #[test]
    fn a_shared_daemons_listener_is_a_loopback_origin() {
        for (base, origin) in [
            ("http://127.0.0.1:51851", Some("http://127.0.0.1:51851")),
            ("http://0.0.0.0:51851", Some("http://127.0.0.1:51851")),
            ("http://[::]:51851", Some("http://[::1]:51851")),
            ("http://localhost:51851/", Some("http://localhost:51851")),
            ("https://127.0.0.1:51851", None),
            ("http://10.0.0.5:51851", None),
            ("http://127.0.0.1", None),
            ("http://127.0.0.1:51851/below", None),
            ("http://u@127.0.0.1:51851", None),
            ("not a url", None),
        ] {
            assert_eq!(loopback_origin(base).as_deref(), origin, "{base}");
        }
    }

    /// The link `apps open` hands the browser for a shared daemon is the one on
    /// the daemon's own listener: the daemon's `url`, or else its path on the
    /// listener the daemon names in `/headless/health`.
    #[test]
    fn a_shared_daemons_link_is_on_its_own_listener() {
        let token = "ab".repeat(32);
        let path = format!("/apps/beta/?t={token}");
        let url = format!("http://127.0.0.1:51851{path}");
        let answer = serde_json::json!({ "path": path, "url": url });
        assert_eq!(shared_launch_link(&answer, "beta", None), Some(url.clone()));

        let older = serde_json::json!({ "path": path });
        assert_eq!(
            shared_launch_link(&older, "beta", Some("http://0.0.0.0:51851")),
            Some(url.clone())
        );
        assert_eq!(shared_launch_link(&older, "beta", None), None);
        assert_eq!(
            shared_launch_link(&older, "beta", Some("http://10.0.0.5:1")),
            None
        );

        for wrong in [
            serde_json::json!({ "path": path, "url": format!("http://127.0.0.1:51851/apps/other/?t={token}") }),
            serde_json::json!({ "path": path, "url": format!("{url}&next=/sessions") }),
            serde_json::json!({ "path": path, "url": format!("http://evil.test:1{path}") }),
            serde_json::json!({ "path": path, "url": 42 }),
            serde_json::json!({ "path": format!("/apps/other/?t={token}"), "url": url }),
        ] {
            assert_eq!(
                shared_launch_link(&wrong, "beta", Some("http://127.0.0.1:1")),
                None,
                "{wrong}"
            );
        }
    }

    /// A stand-in for the profile's shared daemon on its private socket: its
    /// identity, the route a client checks its key on, an app's launch route
    /// (answering `url` only when `names_listener`) and `/headless/health`.
    #[cfg(unix)]
    async fn a_shared_daemon(
        names_listener: bool,
    ) -> (
        biorouter::daemon_runtime::Descriptor,
        std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    ) {
        use biorouter::daemon_runtime::{self, Descriptor, Endpoint};
        use bytes::Bytes;
        use http_body_util::Full;
        use hyper::{server::conn::http1, service::service_fn, Request, Response};
        use hyper_util::rt::TokioIo;
        use std::os::unix::fs::PermissionsExt;

        let directory = daemon_runtime::runtime_directory();
        daemon_runtime::private_directory(&directory).unwrap();
        let socket = directory.join("daemon.sock");
        let _ = fs::remove_file(&socket);
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        let descriptor = Descriptor {
            version: daemon_runtime::VERSION,
            profile_id: daemon_runtime::profile_identity().unwrap(),
            instance_id: "33333333-3333-4333-8333-333333333333".to_owned(),
            pid: std::process::id(),
            endpoint: Endpoint::Unix { path: socket },
            api_secret: "shared-daemon-secret-for-apps-tests".to_owned(),
            user_action_installed: true,
        };
        daemon_runtime::write_private(&daemon_runtime::descriptor_path(), &descriptor).unwrap();
        daemon_runtime::write_user_action_key(&descriptor, &"cd".repeat(32)).unwrap();
        let identity = descriptor.identity();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let requests = std::sync::Arc::clone(&seen);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let identity = identity.clone();
                let requests = std::sync::Arc::clone(&requests);
                tokio::spawn(async move {
                    let service = service_fn(move |request: Request<hyper::body::Incoming>| {
                        let identity = identity.clone();
                        let requests = std::sync::Arc::clone(&requests);
                        async move {
                            let line = format!("{} {}", request.method(), request.uri().path());
                            requests.lock().unwrap().push(line.clone());
                            let path = format!("/apps/beta/?t={}", "ef".repeat(32));
                            let body = match line.as_str() {
                                "GET /daemon/identity" => serde_json::to_value(&identity).unwrap(),
                                "GET /crew/connections" => serde_json::json!([]),
                                "POST /apps/beta/launch" if names_listener => serde_json::json!({
                                    "path": path,
                                    "url": format!("http://127.0.0.1:51851{path}"),
                                }),
                                "POST /apps/beta/launch" => serde_json::json!({ "path": path }),
                                "GET /headless/health" => serde_json::json!({
                                    "status": "ok",
                                    "api_base_url": "http://127.0.0.1:51900",
                                }),
                                _ => {
                                    return Ok::<_, std::convert::Infallible>(
                                        Response::builder()
                                            .status(404)
                                            .header("content-type", "text/plain")
                                            .body(Full::new(Bytes::from_static(b"no such app")))
                                            .unwrap(),
                                    )
                                }
                            };
                            Ok(Response::builder()
                                .header("content-type", "application/json")
                                .body(Full::new(Bytes::from(body.to_string())))
                                .unwrap())
                        }
                    });
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        (descriptor, seen)
    }

    #[cfg(unix)]
    fn forget_shared_daemon() {
        let _ = fs::remove_file(biorouter::daemon_runtime::descriptor_path());
        let _ = fs::remove_file(biorouter::daemon_runtime::user_action_key_path());
    }

    /// With the desktop app open, `apps open` and `apps serve` started a second
    /// full daemon, keyless and never exiting, beside the profile's shared one.
    /// They now mint the link on the shared daemon and send the browser to its
    /// own listener.
    #[cfg(unix)]
    #[tokio::test]
    #[serial_test::serial]
    async fn apps_open_prefers_the_profiles_shared_daemon() {
        // `_on(…, None)`, not `shared_daemon_launch`: a test run from a shell
        // inside a daemon inherits that daemon's BIOROUTER_PORT.
        forget_shared_daemon();
        assert_eq!(
            shared_daemon_launch_on("beta", None).await.unwrap(),
            None,
            "with no shared daemon the command starts one of its own"
        );

        let (_descriptor, seen) = a_shared_daemon(true).await;
        let link = shared_daemon_launch_on("beta", None).await.unwrap();
        assert_eq!(
            link,
            Some(format!(
                "http://127.0.0.1:51851/apps/beta/?t={}",
                "ef".repeat(32)
            ))
        );
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .any(|line| line == "POST /apps/beta/launch"));
        let missing = shared_daemon_launch_on("gone", None)
            .await
            .unwrap_err()
            .to_string();
        assert!(missing.contains("it has no such app"), "{missing}");
        forget_shared_daemon();

        // A daemon from before the answer named its listener: the listener its
        // health route names.
        let (_descriptor, _seen) = a_shared_daemon(false).await;
        assert_eq!(
            shared_daemon_launch_on("beta", None).await.unwrap(),
            Some(format!(
                "http://127.0.0.1:51900/apps/beta/?t={}",
                "ef".repeat(32)
            ))
        );
        forget_shared_daemon();
    }

    /// `BIOROUTER_PORT=8080 biorouter apps serve <id>`, run to match an SSH
    /// tunnel, printed a link on the shared daemon's own port and returned, with
    /// nothing listening on 8080. A named port is honoured; `0`, which a shell
    /// inside a `crew`-started daemon inherits, names none.
    #[test]
    fn a_port_the_person_names_is_one_they_mean() {
        for (value, named) in [
            (None, None),
            (Some("0"), None),
            (Some(""), None),
            (Some("eighty"), None),
            (Some("70000"), None),
            (Some("8080"), Some(8080)),
            (Some(" 8080\n"), Some(8080)),
        ] {
            assert_eq!(named_port(value), named, "{value:?}");
        }
        let link = format!("http://127.0.0.1:51851/apps/beta/?t={}", "ef".repeat(32));
        assert!(on_named_port(&link, None));
        assert!(on_named_port(&link, Some(51851)));
        assert!(on_named_port("http://0.0.0.0:51851", Some(51851)));
        assert!(!on_named_port(&link, Some(8080)));
        assert!(!on_named_port("not a url", Some(8080)));
    }

    #[cfg(unix)]
    #[tokio::test]
    #[serial_test::serial]
    async fn a_named_port_passes_over_a_shared_daemon_on_another() {
        forget_shared_daemon();
        // The stand-in's health route names 51900, and so does the link it
        // mints when its answer names no listener.
        let (_descriptor, seen) = a_shared_daemon(false).await;
        assert_eq!(
            shared_daemon_launch_on("beta", Some(8080)).await.unwrap(),
            None
        );
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|line| line.ends_with("/launch")),
            "nothing is minted on a daemon the command will not use"
        );
        // The shell inside the daemon: its own port is the one named.
        assert_eq!(
            shared_daemon_launch_on("beta", Some(51900)).await.unwrap(),
            Some(format!(
                "http://127.0.0.1:51900/apps/beta/?t={}",
                "ef".repeat(32)
            ))
        );
        forget_shared_daemon();

        // A link off the named port is never handed out, whatever health said.
        let (_descriptor, _seen) = a_shared_daemon(true).await;
        assert_eq!(
            shared_daemon_launch_on("beta", Some(51900)).await.unwrap(),
            None
        );
        forget_shared_daemon();
    }

    /// What the person is told about a link's lifetime is the daemon's figure,
    /// which lives in a crate this one cannot import.
    #[test]
    fn the_lifetime_the_person_is_told_is_the_daemons() {
        let auth = include_str!("../../../biorouter-server/src/auth.rs");
        assert!(
            auth.contains("const APP_LAUNCH_TTL: Duration = Duration::from_secs(5 * 60);"),
            "the daemon's launch-link lifetime changed; update LAUNCH_LINK_LIFETIME"
        );
        assert_eq!(LAUNCH_LINK_LIFETIME, "five minutes");
    }
}
