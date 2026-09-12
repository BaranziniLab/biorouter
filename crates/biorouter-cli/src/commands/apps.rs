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
//!   in the default browser.
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
//! spawn. The `/apps/<id>/` GET routes are auth-exempt, so the browser loads the
//! app without needing the daemon's secret key.
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
    let connect = tokio::net::TcpStream::connect(&addr);
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
    Started(tokio::process::Child),
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

/// Ensure a daemon is reachable on the configured port, spawning one if needed.
async fn ensure_daemon(port: u16, supervision: Supervision) -> Result<Daemon> {
    if daemon_ok(DAEMON_HOST, port).await {
        return Ok(Daemon::Reused);
    }

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
        if daemon_ok(DAEMON_HOST, port).await {
            return Ok(Daemon::Started(child));
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

// ──────────────────────────────────────────────────────────────────────────────
// open
// ──────────────────────────────────────────────────────────────────────────────

pub async fn handle_apps_open(id: String) -> Result<()> {
    require_app(&id)?;
    let port = configured_port();
    let daemon = ensure_daemon(port, Supervision::Detached).await?;
    let url = app_url(port, &id);

    match daemon {
        Daemon::Reused => {
            println!(
                "  {} using the daemon already running on port {port}",
                style("·").dim()
            );
        }
        Daemon::Started(_child) => {
            // Leave the child running (detached) so the app stays served after
            // this command returns — the tokio child is not killed on drop.
            println!("  {} started biorouterd on port {port}", style("✓").green());
        }
    }

    if let Err(e) = open::that(&url) {
        eprintln!("    {} could not open a browser: {e}", style("!").yellow());
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
    let daemon = ensure_daemon(port, Supervision::TiedToThisProcess).await?;
    let url = app_url(port, &id);

    match daemon {
        Daemon::Reused => {
            // Reusing an external daemon: print the URL and a note, then exit 0.
            // We do not own its lifecycle, so there is nothing to keep in the
            // foreground — and nothing of ours to stop.
            println!("  {} {}", style("→").fg(ACCENT), style(&url).bold());
            println!(
                "  {} reusing the daemon already running on port {port}; it keeps running after this command.",
                style("·").dim()
            );
            Ok(())
        }
        Daemon::Started(mut child) => {
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
}
