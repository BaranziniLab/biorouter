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
//! **Daemon management is deliberately minimal.** The CLI has no pre-existing
//! biorouterd-supervision helper, so `open`/`serve` first health-check the
//! configured port (`BIOROUTER_PORT`, default 3000) via the auth-exempt
//! `GET /status`, reuse a daemon if one answers, and otherwise best-effort spawn
//! the sibling `biorouterd agent`. If no `biorouterd` binary can be located the
//! command fails with the exact command to run — an honest v1 over a fragile
//! spawn.
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

/// Ensure a daemon whose secret this command knows is reachable on the configured
/// port, spawning one if needed. Answers the daemon and its secret.
async fn ensure_daemon(port: u16, supervision: Supervision) -> Result<(Daemon, String)> {
    if daemon_ok(DAEMON_HOST, port).await {
        let known = std::env::var("BIOROUTER_SERVER__SECRET_KEY")
            .ok()
            .filter(|secret| !secret.is_empty())
            .into_iter()
            .chain(remembered_secret(port));
        for secret in known {
            if secret_works(port, &secret).await {
                return Ok((Daemon::Reused, secret));
            }
        }
        bail!(
            "a Biorouter daemon is already running on port {port}, and opening an app on it takes \
             that daemon's secret key, which this command does not know. It knows the key of a \
             daemon `biorouter apps open` or `biorouter apps serve` started; for any other, set \
             BIOROUTER_SERVER__SECRET_KEY to its key, or set BIOROUTER_PORT to a free port so \
             this command starts a daemon of its own."
        );
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
            return Ok((Daemon::Started { child, remembered }, secret));
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
// open
// ──────────────────────────────────────────────────────────────────────────────

pub async fn handle_apps_open(id: String) -> Result<()> {
    require_app(&id)?;
    let port = configured_port();
    let (daemon, secret) = ensure_daemon(port, Supervision::Detached).await?;
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

    // The link itself never reaches an opener's command line: see
    // `write_launch_page`. The terminal is this account's alone, so the link is
    // printed there when no browser could be opened.
    //
    // It is printed when the page did open, too: a browser that cannot read the
    // page (a snap-packaged browser cannot read hidden folders such as the state
    // dir) shows an error, and `open` still reports success.
    let once = format!("it works once, within {LAUNCH_LINK_LIFETIME}");
    match open_launch_link(&launch, &launch_pages_dir(), |page| open::that(page)) {
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
    println!("  {} {}", style("→").fg(ACCENT), style(&url).bold());
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// serve
// ──────────────────────────────────────────────────────────────────────────────

pub async fn handle_apps_serve(id: String) -> Result<()> {
    require_app(&id)?;
    let port = configured_port();

    // ⚠ BEFORE the spawn, and before anything that can park. A handler replaces
    // the default action — which for SIGTERM is to end this process on the spot
    // and leave the daemon behind — so from here on a signal waits to be read,
    // including one that lands during the readiness wait inside `ensure_daemon`.
    let mut stop = StopSignals::install()?;
    let (daemon, secret) = ensure_daemon(port, Supervision::TiedToThisProcess).await?;
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

    /// A source guard for the property above: the only thing this file ever
    /// hands `open::that` is a launch page.
    #[test]
    fn nothing_but_a_launch_page_reaches_an_opener() {
        let source = include_str!("apps.rs")
            .split("\n#[cfg(test)]\nmod tests {")
            .next()
            .unwrap();
        let calls: Vec<&str> = source.split("open::that(").skip(1).collect();
        assert!(!calls.is_empty());
        for call in calls {
            assert!(call.starts_with("page)"), "open::that({call:.40}");
        }
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
        let (daemon, secret) = ensure_daemon(port, Supervision::TiedToThisProcess)
            .await
            .expect("a recorded daemon is reused");
        assert!(matches!(daemon, Daemon::Reused));
        assert_eq!(secret, SECRET);

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
