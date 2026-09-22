use crate::config::base::Config;
use crate::config::extensions::get_enabled_extensions;
use crate::config::paths::Paths;
use crate::providers::utils::LOGS_TO_KEEP;
use crate::session::session_manager::{ModelUsageRow, UsageTotals};
use crate::session::SessionManager;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Cursor;
use std::io::Write;
use utoipa::ToSchema;
use zip::write::FileOptions;
use zip::ZipWriter;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SystemInfo {
    pub app_version: String,
    pub os: String,
    pub os_version: String,
    pub architecture: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub enabled_extensions: Vec<String>,
}

impl SystemInfo {
    pub fn collect() -> Self {
        let config = Config::global();
        let provider = config.get_biorouter_provider().ok();
        let model = config.get_biorouter_model().ok();
        let enabled_extensions = get_enabled_extensions()
            .into_iter()
            .map(|ext| ext.name().to_string())
            .collect();

        Self {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            os_version: sys_info::os_release().unwrap_or_else(|_| "unknown".to_string()),
            architecture: std::env::consts::ARCH.to_string(),
            provider,
            model,
            enabled_extensions,
        }
    }

    pub fn to_text(&self) -> String {
        format!(
            "App Version: {}\n\
             OS: {}\n\
             OS Version: {}\n\
             Architecture: {}\n\
             Provider: {}\n\
             Model: {}\n\
             Enabled Extensions: {}\n\
             Timestamp: {}\n",
            self.app_version,
            self.os,
            self.os_version,
            self.architecture,
            self.provider.as_deref().unwrap_or("unknown"),
            self.model.as_deref().unwrap_or("unknown"),
            self.enabled_extensions.join(", "),
            chrono::Utc::now().to_rfc3339()
        )
    }
}

pub fn get_system_info() -> SystemInfo {
    SystemInfo::collect()
}

/// Token accounting for one session plus month-to-date totals, so a future
/// "my dashboard says N but Biorouter shows M" report is self-diagnosing.
///
/// The crux of issue #1: the chat headline used to show the LAST TURN's
/// `total_tokens` (context-window occupancy) while the vendor bills the
/// ACCUMULATED sum across every turn. This bundle records both, per model, plus
/// the cache buckets, so the two numbers can be reconciled without re-running.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UsageDiagnostics {
    pub session_id: String,
    /// Last turn's context-window occupancy (what the old headline showed).
    pub last_turn_total_tokens: Option<i32>,
    pub last_turn_input_tokens: Option<i32>,
    pub last_turn_output_tokens: Option<i32>,
    /// Lifetime accumulated counters. New events are billed totals; legacy
    /// databases can contain context totals, so the ledger summary is authoritative.
    pub accumulated_total_tokens: Option<i64>,
    pub accumulated_input_tokens: Option<i64>,
    pub accumulated_output_tokens: Option<i64>,
    /// Cache tokens summed over this session's turns.
    pub session_cache_read_tokens: Option<i64>,
    pub session_cache_creation_tokens: Option<i64>,
    /// Per-`(model, provider)` breakdown for this session.
    pub per_model: Vec<ModelUsageRow>,
    /// Current local month, `YYYY-MM`.
    pub month: String,
    pub month_to_date: UsageTotals,
    pub all_time: UsageTotals,
}

impl UsageDiagnostics {
    pub async fn collect(
        session_manager: &SessionManager,
        session_id: &str,
    ) -> anyhow::Result<Self> {
        let session = session_manager.get_session(session_id, false).await?;
        let per_model = session_manager.get_session_model_usage(session_id).await?;
        let summary = session_manager.get_usage_summary().await?;

        let session_cache_read_tokens = per_model
            .iter()
            .try_fold(0_i64, |sum, row| Some(sum + row.cache_read_tokens?));
        let session_cache_creation_tokens = per_model
            .iter()
            .try_fold(0_i64, |sum, row| Some(sum + row.cache_creation_tokens?));

        Ok(Self {
            session_id: session_id.to_string(),
            last_turn_total_tokens: session.total_tokens,
            last_turn_input_tokens: session.input_tokens,
            last_turn_output_tokens: session.output_tokens,
            accumulated_total_tokens: session.accumulated_total_tokens,
            accumulated_input_tokens: session.accumulated_input_tokens,
            accumulated_output_tokens: session.accumulated_output_tokens,
            session_cache_read_tokens,
            session_cache_creation_tokens,
            per_model,
            month: summary.month,
            month_to_date: summary.month_to_date,
            all_time: summary.all_time,
        })
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("Session: {}\n\n", self.session_id));
        out.push_str("Last turn (context-window occupancy, NOT the billed total):\n");
        out.push_str(&format!(
            "  total={}  input={}  output={}\n\n",
            opt_i32(self.last_turn_total_tokens),
            opt_i32(self.last_turn_input_tokens),
            opt_i32(self.last_turn_output_tokens),
        ));
        out.push_str("Accumulated session counters (legacy rows may use context totals):\n");
        out.push_str(&format!(
            "  total={}  input={}  output={}\n",
            opt_i64(self.accumulated_total_tokens),
            opt_i64(self.accumulated_input_tokens),
            opt_i64(self.accumulated_output_tokens),
        ));
        out.push_str(&format!(
            "  cache_read={}  cache_creation={}\n\n",
            opt_i64(self.session_cache_read_tokens),
            opt_i64(self.session_cache_creation_tokens),
        ));

        out.push_str("Per-model (this session):\n");
        for row in &self.per_model {
            out.push_str(&format!(
                "  {} / {}: input={} output={} cache_read={} cache_creation={} total={} turns={}\n",
                row.provider.as_deref().unwrap_or("unknown"),
                row.model_id.as_deref().unwrap_or("unknown"),
                row.input_tokens,
                row.output_tokens,
                opt_i64(row.cache_read_tokens),
                opt_i64(row.cache_creation_tokens),
                opt_i64(row.total_tokens),
                row.turns,
            ));
        }

        out.push_str(&format!("\nMonth-to-date ({}):\n", self.month));
        out.push_str(&totals_text(&self.month_to_date));
        out.push_str("\nAll-time:\n");
        out.push_str(&totals_text(&self.all_time));
        out
    }
}

fn opt_i32(v: Option<i32>) -> String {
    v.map_or_else(|| "-".to_string(), |n| n.to_string())
}

fn opt_i64(v: Option<i64>) -> String {
    v.map_or_else(|| "-".to_string(), |n| n.to_string())
}

fn totals_text(t: &UsageTotals) -> String {
    let cost = t.cost.map_or_else(
        || "unpriced".to_string(),
        |c| {
            let flag = match (t.has_unpriced, t.cost_excludes_cache) {
                (true, true) => " (known subtotal; unpriced usage and cache excluded)",
                (true, false) => " (known subtotal; some usage unpriced)",
                (false, true) => " (known subtotal; cache excluded)",
                (false, false) => "",
            };
            format!("${c:.4}{flag}")
        },
    );
    format!(
        "  input={} output={} cache_read={} cache_creation={} total={} turns={} cost={}\n",
        t.input_tokens,
        t.output_tokens,
        opt_i64(t.cache_read_tokens),
        opt_i64(t.cache_creation_tokens),
        opt_i64(t.total_tokens),
        t.turns,
        cost,
    )
}

/// The session a `llm_request.*.jsonl` names in its header line, if any.
///
/// `RequestLog::start` writes that header first and always (see
/// `crate::providers::utils::RequestLog`). `None` means the file predates that
/// header, is not JSONL, or was opened outside a scoped session — in every one
/// of those cases the file's provenance is unknown.
fn log_session_id(contents: &str) -> Option<String> {
    let header: serde_json::Value = serde_json::from_str(contents.lines().next()?).ok()?;
    header.get("session_id")?.as_str().map(str::to_string)
}

/// Whether a log file may be shipped in the bundle for `session_id`.
///
/// **Fail-closed on purpose.** An unattributable log is excluded rather than
/// included: the bundle is emailed to a third party, so "I could not tell whose
/// conversation this was" has to mean "do not send it". The previous rule —
/// newest ten files, whoever produced them — put another chat's prompts into a
/// bug report about this one.
fn log_belongs_to_session(contents: &str, session_id: &str) -> bool {
    log_session_id(contents).is_some_and(|id| id == session_id)
}

/// Does this key name a credential? Matched case-insensitively on a substring,
/// so `SPOKEAGENT_PASSCODE`, `api_key` and `GITHUB_TOKEN` all hit.
///
/// Deliberately broad: over-redacting a bundle costs a support engineer one
/// follow-up question, under-redacting one mails a working credential to an
/// inbox and, for a GitHub issue, to the public.
fn is_secret_key(key: &str) -> bool {
    const NEEDLES: &[&str] = &[
        "key",
        "token",
        "secret",
        "password",
        "passcode",
        "passwd",
        "credential",
        "auth",
        "cookie",
        "session",
        "signature",
        "private",
    ];
    let key = key.to_ascii_lowercase();
    NEEDLES.iter().any(|needle| key.contains(needle))
}

const REDACTED: &str = "[redacted by diagnostics bundle]";

/// Replace credential-shaped values in `config.yaml` before it is zipped.
///
/// Two rules, because extension entries need both: any value whose key looks
/// like a credential, and **every** value inside an `envs` map (a `.brxt`
/// install writes the extension's environment straight into the config entry,
/// and `CDW_USERNAME` names a person while matching no credential needle).
///
/// A file that does not parse is replaced wholesale rather than passed through:
/// the fallback for "I could not inspect this" is not "send it anyway".
fn redact_config_yaml(raw: &str) -> String {
    let Ok(mut doc) = serde_yaml::from_str::<serde_yaml::Value>(raw) else {
        return "# config.yaml was omitted from this bundle: it could not be parsed as YAML,\n\
                # so its values could not be checked for credentials.\n"
            .to_string();
    };
    redact_yaml_value(&mut doc, false);
    serde_yaml::to_string(&doc).unwrap_or_else(|_| {
        "# config.yaml was omitted from this bundle: it could not be re-serialized.\n".to_string()
    })
}

fn redact_yaml_value(value: &mut serde_yaml::Value, redact_every_leaf: bool) {
    match value {
        serde_yaml::Value::Mapping(map) => {
            let keys: Vec<serde_yaml::Value> = map.keys().cloned().collect();
            for key in keys {
                let key_str = key.as_str().unwrap_or_default().to_string();
                let Some(child) = map.get_mut(&key) else {
                    continue;
                };
                if redact_every_leaf || is_secret_key(&key_str) {
                    *child = serde_yaml::Value::String(REDACTED.to_string());
                    continue;
                }
                // A `.brxt` install writes the extension's whole environment
                // here; treat all of it as credential material.
                redact_yaml_value(child, key_str.eq_ignore_ascii_case("envs"));
            }
        }
        serde_yaml::Value::Sequence(items) => {
            for item in items {
                redact_yaml_value(item, redact_every_leaf);
            }
        }
        _ => {}
    }
}

/// The session's own request logs, newest first, capped at [`LOGS_TO_KEEP`].
///
/// Split out of [`generate_diagnostics`] to keep that function under the
/// `too_many_lines` baseline, and because the walk is the one part of the
/// bundle with a moving target underneath it: `flush_request_log` rotates
/// these files while this reads them.
type ZipOut<'a> = ZipWriter<Cursor<&'a mut Vec<u8>>>;

/// How many bytes of component (`cli/`, `server/`) log the bundle will carry.
///
/// A daemon that has been up for a week writes megabytes; the tail is where the
/// failure being reported is, and a report nobody can open helps nobody.
const COMPONENT_LOG_BUDGET_BYTES: usize = 256 * 1024;

/// Which component log directories are swept, and what each is called in the
/// bundle. `debug/` is deliberately absent: it is written only under an
/// explicit debug flag and carries whole payloads.
const COMPONENT_LOG_DIRS: &[&str] = &["cli", "server"];

/// The outcome of the request-log sweep, so [`generate_diagnostics`] can say
/// which of the two very different "no logs" situations happened.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct RequestLogSweep {
    /// Files written into the bundle.
    shipped: usize,
    /// Files that existed but named a different session, or named none at all.
    /// ⚠ This is the number that used to be invisible. A bundle with no logs
    /// read identically whether the machine had never made a request or whether
    /// every log on it was unattributable — and the second was the normal case.
    excluded: usize,
}

fn push_session_logs(
    zip: &mut ZipOut<'_>,
    options: FileOptions,
    logs_dir: &std::path::Path,
    session_id: &str,
    notes: &mut Vec<String>,
) -> anyhow::Result<RequestLogSweep> {
    let mut sweep = RequestLogSweep::default();
    let mut log_files: Vec<_> = match fs::read_dir(logs_dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "jsonl")
            })
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            notes.push(format!(
                "logs: could not list {}: {error}",
                logs_dir.display()
            ));
            Vec::new()
        }
    };

    log_files.sort_by_key(|e| e.metadata().ok().and_then(|m| m.modified().ok()));

    // Newest first, but only this session's logs — see
    // `log_belongs_to_session`. `take(LOGS_TO_KEEP)` on the *unfiltered*
    // list is what shipped other chats' prompts: the ten newest files on
    // this machine have nothing to do with the session being reported.
    for entry in log_files.iter().rev() {
        if sweep.shipped == LOGS_TO_KEEP {
            break;
        }
        let path = entry.path();
        // A log that rotated out from under this walk is not a reason to
        // lose the whole report; note it and take the next one.
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                notes.push(format!("logs: could not read {}: {error}", path.display()));
                continue;
            }
        };
        if !log_belongs_to_session(&String::from_utf8_lossy(&bytes), session_id) {
            sweep.excluded += 1;
            continue;
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default();
        zip.start_file(format!("logs/{}", name), options)?;
        zip.write_all(&bytes)?;
        sweep.shipped += 1;
    }
    Ok(sweep)
}

/// Is this tracing line one the bundle may carry?
///
/// The component logs are the daemon's own, shared by every session on the
/// machine, so the rule that governs the request logs — ship only what this
/// session produced — cannot be applied: a tracing line names no session.
///
/// The line drawn instead is by LEVEL. `WARN` and `ERROR` are where a failure
/// announces itself, and the tracing layer emits them for control-flow events:
/// an extension that would not load, a provider that returned 429, a path that
/// could not be read. `INFO`/`DEBUG`/`TRACE` are where content goes — a request
/// body, a tool's output, a prompt fragment — and shipping those would put
/// another chat's material into a report about this one, which is exactly the
/// defect `log_belongs_to_session` exists to prevent.
///
/// ⚠ A WARN/ERROR line can still quote a path or an identifier, so this is a
/// large reduction in exposure, not a redactor. The bundle is saved to disk by
/// a native dialog and attached by the user deliberately; the *issue body* the
/// bug-report tool posts is scrubbed separately and never carries these lines.
fn is_diagnostic_level_line(line: &str) -> bool {
    // `2026-09-05T17:34:53.579160Z  WARN biorouter::…: message`
    let Some(rest) = line.split_once('Z').map(|(_, rest)| rest.trim_start()) else {
        return false;
    };
    rest.starts_with("WARN") || rest.starts_with("ERROR")
}

/// The tail of the newest `cli/` and `server/` log, filtered to WARN/ERROR.
///
/// ⚠ **These were absent from every bundle ever produced**, and not by policy:
/// the sweep above walks the ROOT of `logs/` for `*.jsonl` only, so
/// `logs/server/<date>/*.log` and `logs/cli/<date>/*.log` were never candidates.
/// The daemon's own startup failures, extension-load errors and provider errors
/// — the first thing anyone reading a bug report looks for — were in neither the
/// bundle nor the notes.
fn push_component_logs(
    zip: &mut ZipOut<'_>,
    options: FileOptions,
    logs_dir: &std::path::Path,
    notes: &mut Vec<String>,
) -> anyhow::Result<usize> {
    let mut shipped = 0usize;
    for component in COMPONENT_LOG_DIRS {
        let Some(path) = newest_component_log(&logs_dir.join(component)) else {
            continue;
        };
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) => {
                notes.push(format!(
                    "logs/{component}: could not read {}: {error}",
                    path.display()
                ));
                continue;
            }
        };
        let filtered = tail_diagnostic_lines(&raw, COMPONENT_LOG_BUDGET_BYTES);
        if filtered.is_empty() {
            continue;
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("{component}.log"));
        zip.start_file(format!("logs/{component}/{name}"), options)?;
        zip.write_all(filtered.as_bytes())?;
        shipped += 1;
    }
    Ok(shipped)
}

/// The most recently modified `*.log` under `<component>/<date>/`.
fn newest_component_log(component_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut newest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for day in fs::read_dir(component_dir).ok()?.filter_map(Result::ok) {
        let day_path = day.path();
        if !day_path.is_dir() {
            continue;
        }
        let Ok(files) = fs::read_dir(&day_path) else {
            continue;
        };
        for file in files.filter_map(Result::ok) {
            let path = file.path();
            if path.extension().is_some_and(|ext| ext == "log") {
                let modified = file
                    .metadata()
                    .ok()
                    .and_then(|meta| meta.modified().ok())
                    .unwrap_or(std::time::UNIX_EPOCH);
                if newest.as_ref().is_none_or(|(best, _)| modified > *best) {
                    newest = Some((modified, path));
                }
            }
        }
    }
    newest.map(|(_, path)| path)
}

/// The WARN/ERROR lines of `raw`, newest last, within `budget` bytes.
///
/// The tail rather than the head: a log that has outgrown the budget has done
/// so because the daemon ran for a long time, and the failure being reported
/// happened at the end of it.
fn tail_diagnostic_lines(raw: &str, budget: usize) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut size = 0usize;
    for line in raw.lines().rev() {
        if !is_diagnostic_level_line(line) {
            continue;
        }
        size += line.len() + 1;
        if size > budget {
            break;
        }
        kept.push(line);
    }
    kept.reverse();
    if kept.is_empty() {
        return String::new();
    }
    format!(
        "# Only WARN and ERROR lines are included; INFO/DEBUG carry other chats' content.\n\
         # Newest last, tail-limited to {budget} bytes.\n{}\n",
        kept.join("\n")
    )
}

/// What the log sweep found, in a sentence a person can act on.
///
/// Pure, so the wording is testable without building a zip.
fn log_summary(sweep: &RequestLogSweep, component_logs: usize) -> String {
    let mut out = String::from("Log files in this bundle\n========================\n\n");
    out.push_str(&format!(
        "Request logs (logs/*.jsonl) for this session: {}\n",
        sweep.shipped
    ));
    if sweep.excluded > 0 {
        out.push_str(&format!(
            "Request logs left out because their header names a different session, or \
             names none at all: {}\n",
            sweep.excluded
        ));
        if sweep.shipped == 0 {
            out.push_str(
                "\n  ⚠ Every request log on this machine was unattributable, so NONE of this \n\
                 \x20   session's requests are in this bundle. That is a known shape of an older \n\
                 \x20   build: the header line was only stamped with a session id for scheduled \n\
                 \x20   jobs and sub-agents, never for an ordinary chat turn. The transcript in \n\
                 \x20   session.json is complete and is the better evidence either way.\n",
            );
        }
    }
    out.push_str(&format!(
        "\nComponent logs (logs/cli/, logs/server/): {component_logs}\n"
    ));
    if component_logs > 0 {
        out.push_str(
            "  Filtered to WARN and ERROR lines only, newest last. INFO and DEBUG lines are \n\
             \x20 excluded because they carry content from every chat on this machine, not just \n\
             \x20 the one being reported.\n",
        );
    }
    if sweep.shipped == 0 && sweep.excluded == 0 && component_logs == 0 {
        out.push_str("\nNo log files were found on this machine at all.\n");
    }
    out
}

/// Every file under `scheduled_workflows/`, each read best-effort.
fn push_scheduled_workflows(
    zip: &mut ZipOut<'_>,
    options: FileOptions,
    scheduled_workflows_dir: &std::path::Path,
    notes: &mut Vec<String>,
) -> anyhow::Result<()> {
    if scheduled_workflows_dir.exists() && scheduled_workflows_dir.is_dir() {
        match fs::read_dir(scheduled_workflows_dir) {
            Ok(entries) => {
                for entry in entries.filter_map(|entry| entry.ok()) {
                    let path = entry.path();
                    if !path.is_file() {
                        continue;
                    }
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy())
                        .unwrap_or_default();
                    match fs::read(&path) {
                        Ok(bytes) => {
                            zip.start_file(format!("scheduled_workflows/{}", name), options)?;
                            zip.write_all(&bytes)?;
                        }
                        Err(error) => notes.push(format!(
                            "scheduled_workflows/{name}: could not read {}: {error}",
                            path.display()
                        )),
                    }
                }
            }
            Err(error) => notes.push(format!(
                "scheduled_workflows: could not list {}: {error}",
                scheduled_workflows_dir.display()
            )),
        }
    }
    Ok(())
}

/// Where a bundle's files are read from: the request/component logs, the
/// config file and the data dir (`schedule.json`, `scheduled_workflows/`).
///
/// [`generate_diagnostics`] resolves them from [`Paths`] once, first, exactly as
/// it always did. The tests hand in directories of their own instead — see
/// `sources_in` in the tests below for why that is the whole of their
/// isolation.
pub(crate) struct DiagnosticsSources {
    logs_dir: std::path::PathBuf,
    config_path: std::path::PathBuf,
    data_dir: std::path::PathBuf,
}

impl DiagnosticsSources {
    /// The directories a bundle reads on this machine, as `Paths` says now.
    pub(crate) fn resolve() -> Self {
        Self {
            logs_dir: Paths::in_state_dir("logs"),
            config_path: Paths::config_dir().join("config.yaml"),
            data_dir: Paths::data_dir(),
        }
    }

    /// Where the request-log sweep looks — the directory `RequestLog` writes.
    #[cfg(test)]
    pub(crate) fn logs_dir(&self) -> &std::path::Path {
        &self.logs_dir
    }

    /// The config file the bundle redacts and ships — the one
    /// `Config::global()` writes.
    #[cfg(test)]
    pub(crate) fn config_path(&self) -> &std::path::Path {
        &self.config_path
    }

    /// The `schedule.json` the bundle ships — the one the scheduler writes.
    pub(crate) fn schedule_json(&self) -> std::path::PathBuf {
        self.data_dir.join("schedule.json")
    }

    /// The directory whose files the bundle ships as `scheduled_workflows/` —
    /// the one the scheduler copies each job's workflow into.
    pub(crate) fn scheduled_workflows_dir(&self) -> std::path::PathBuf {
        self.data_dir.join("scheduled_workflows")
    }
}

pub async fn generate_diagnostics(
    session_manager: &SessionManager,
    session_id: &str,
) -> anyhow::Result<Vec<u8>> {
    generate_diagnostics_from(session_manager, session_id, &DiagnosticsSources::resolve()).await
}

async fn generate_diagnostics_from(
    session_manager: &SessionManager,
    session_id: &str,
    sources: &DiagnosticsSources,
) -> anyhow::Result<Vec<u8>> {
    let DiagnosticsSources {
        logs_dir,
        config_path,
        data_dir: _,
    } = sources;

    let system_info = SystemInfo::collect();

    // What could not be collected, and why.
    //
    // ⚠ **Every optional source below is best-effort, and that is the whole
    // point of this function.** A diagnostics bundle exists so somebody can
    // report a bug; a collector that refuses to produce one because a single
    // file was unreadable has failed at the one job it has, and it fails
    // hardest in exactly the situation that produces bug reports — a machine
    // where something is already wrong.
    //
    // The concrete case that used to lose: `flush_request_log` rotates
    // `llm_request.{i}.jsonl` → `{i+1}` while this walk holds paths it read
    // from an earlier `read_dir`. A turn running in another window during the
    // report is enough. The read then hits ENOENT, `?` propagated it, the
    // route turned it into a bodyless 500, and the user was told only
    // "Failed to generate diagnostics."
    //
    // So a failure is RECORDED rather than raised, and the notes ship inside
    // the bundle. Silently omitting a file would be worse than failing: the
    // person reading the report needs to know the transcript is missing, not
    // conclude the chat was empty.
    let mut notes: Vec<String> = Vec::new();

    let mut buffer = Vec::new();
    {
        let mut zip = ZipWriter::new(Cursor::new(&mut buffer));
        let options = FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        let sweep = push_session_logs(&mut zip, options, logs_dir, session_id, &mut notes)?;
        let component_logs = push_component_logs(&mut zip, options, logs_dir, &mut notes)?;

        // ⚠ Written ALWAYS, and that is the fix. The log sweep used to report
        // nothing at all: `collection-notes.txt` appears only when a file could
        // not be *read*, so a bundle whose every request log was excluded as
        // unattributable looked exactly like a bundle from a machine that had
        // never made a request. The second reading was the natural one and the
        // first was the truth on every desktop turn, because the interactive
        // path did not scope a session id onto the log header.
        //
        // Deliberately NOT folded into `collection-notes.txt`: that file's
        // contract is "a part could not be collected", and its absence is a
        // signal a test pins. "There are no logs on this machine" is not a
        // failure, and an unconditional summary is what makes absence legible
        // without overloading a failure channel.
        zip.start_file("logs-summary.txt", options)?;
        zip.write_all(log_summary(&sweep, component_logs).as_bytes())?;

        // The transcript is the most valuable thing in the bundle and still not
        // worth failing over: logs plus system info are a usable report, and no
        // bundle at all is not.
        match session_manager.export_session(session_id).await {
            Ok(session_data) => {
                zip.start_file("session.json", options)?;
                zip.write_all(session_data.as_bytes())?;
            }
            Err(error) => notes.push(format!(
                "session.json: could not export session '{session_id}': {error:#}"
            )),
        }

        if config_path.exists() {
            match fs::read(config_path) {
                Ok(raw) => {
                    zip.start_file("config.yaml", options)?;
                    zip.write_all(redact_config_yaml(&String::from_utf8_lossy(&raw)).as_bytes())?;
                }
                Err(error) => notes.push(format!(
                    "config.yaml: could not read {}: {error}",
                    config_path.display()
                )),
            }
        }

        zip.start_file("system.txt", options)?;
        zip.write_all(system_info.to_text().as_bytes())?;

        // Token accounting so a usage-mismatch report is self-diagnosing
        // (last-turn vs accumulated vs MTD, per model, incl. cache buckets).
        if let Ok(usage) = UsageDiagnostics::collect(session_manager, session_id).await {
            zip.start_file("usage.txt", options)?;
            zip.write_all(usage.to_text().as_bytes())?;
        }

        let schedule_json = sources.schedule_json();
        if schedule_json.exists() {
            match fs::read(&schedule_json) {
                Ok(bytes) => {
                    zip.start_file("schedule.json", options)?;
                    zip.write_all(&bytes)?;
                }
                Err(error) => notes.push(format!(
                    "schedule.json: could not read {}: {error}",
                    schedule_json.display()
                )),
            }
        }

        push_scheduled_workflows(
            &mut zip,
            options,
            &sources.scheduled_workflows_dir(),
            &mut notes,
        )?;

        // Last, so it can report on everything above it. Absent when nothing
        // went wrong, so its presence is itself the signal.
        if !notes.is_empty() {
            zip.start_file("collection-notes.txt", options)?;
            zip.write_all(
                format!(
                    "Some parts of this bundle could not be collected. The rest of it is \
                     complete and still usable for a bug report.\n\n{}\n",
                    notes.join("\n")
                )
                .as_bytes(),
            )?;
        }

        zip.finish()?;
    }

    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::session_manager::{SessionType, UsageLedgerEntry};
    use tempfile::TempDir;

    /// A bundle's sources inside `temp`, handed to the collector directly.
    ///
    /// ⚠ **No test here may set `BIOROUTER_PATH_ROOT`, and this helper is why
    /// none needs to.** They all used to point it at their `TempDir` under the
    /// env lock and let `generate_diagnostics` resolve `Paths`. The lock orders
    /// the tests that TAKE it; it does nothing about the ones that only read
    /// the variable, and `RequestLog::start` is one: it resolves
    /// `Paths::in_state_dir("logs")` the instant a request starts, and the
    /// provider tests — `providers::versa_azure::routing_tests`,
    /// `bedrock_namespace_tests`, `bedrock`, `knowledge_source_tool`, 70
    /// requests in one whole-lib run (census, 2026-09-21) — open one per
    /// request without the lock. One that starts while a test here holds the
    /// variable writes into that test's logs dir, and its flush renames
    /// `llm_request.{i}` → `{i+1}` under the test's own fixtures. Forced on
    /// the previous shape (a probe opening a `RequestLog` while
    /// `the_bundle_ships_only_the_named_session_s_llm_logs` held its root),
    /// that test failed 10 of 10 with `left: ["logs/llm_request.1.jsonl"]`;
    /// with this helper, 0 of 10. Unforced, whole-lib runs on this branch
    /// failed it as `left: []` and as `left: [".0", ".1"]` (a review counted
    /// 2 of 31). A test whose directory is never the ambient root cannot be
    /// handed anyone's write, whatever the scheduling — which is the property,
    /// rather than a lock that could only ever cover the writers that ask.
    ///
    /// `the_diagnostics_sources_are_never_relocated_through_the_environment`
    /// below holds that line.
    fn sources_in(temp: &TempDir) -> DiagnosticsSources {
        DiagnosticsSources {
            logs_dir: temp.path().join("state").join("logs"),
            config_path: temp.path().join("config").join("config.yaml"),
            data_dir: temp.path().join("data"),
        }
    }

    /// Nothing in this module relocates the process's path root, or reads a
    /// bundle's sources from it. See [`sources_in`] for the measured reason; a
    /// test that goes back to relocating it is exposed to every unlocked
    /// `RequestLog` in the binary again, and fails only when one lands inside
    /// its window.
    ///
    /// The last two needles are the other half. `generate_diagnostics` and
    /// `DiagnosticsSources::resolve` take the sources from `Paths` — in this
    /// binary, the sandbox that every test shares — so a test calling either
    /// bundles whatever logs, `config.yaml` and `schedule.json` its siblings
    /// have written there, and any assertion on the bundle's contents depends
    /// on which of them ran first. The production pairing they stand for is
    /// pinned where each writer lives, under the sandbox pin, instead:
    /// `providers::utils`' `a_request_log_lands_where_the_diagnostics_bundle_reads`,
    /// `config::base`'s `the_config_file_is_the_one_the_diagnostics_bundle_reads`
    /// and `scheduler`'s `the_schedule_lands_where_the_diagnostics_bundle_reads`.
    ///
    /// Over source text because the failure it prevents has no runtime signal
    /// until the race is lost. The needles are assembled so this test's own
    /// text is not a match.
    #[test]
    fn the_diagnostics_sources_are_never_relocated_through_the_environment() {
        let source = include_str!("diagnostics.rs");
        let (_, tests) = source
            .split_once(concat!("#[cfg(test)]\n", "mod tests {"))
            .expect("this file has a tests module");
        let needles = [
            concat!("lock_", "env("),
            concat!("pin_sandbox_", "path_root("),
            concat!("set_", "var("),
            concat!("generate_", "diagnostics("),
            concat!("DiagnosticsSources::", "resolve("),
        ];
        let offenders: Vec<&str> = tests
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .filter(|line| needles.iter().any(|needle| line.contains(needle)))
            .collect();
        assert!(
            offenders.is_empty(),
            "a diagnostics test moves the process-global path root again, or reads a \
             bundle's sources from it:\n  {}\n\
             Unlocked writers follow that variable — every provider test's \
             RequestLog does — so a test holding it on its own TempDir receives \
             their files and their rotation of llm_request.N, and a test that reads \
             through it (generate_diagnostics, DiagnosticsSources::resolve) bundles \
             whatever its siblings wrote into the shared sandbox. Pass the \
             directories with `sources_in(&temp)` to `generate_diagnostics_from` \
             instead.",
            offenders.join("\n  ")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn diagnostics_bundle_succeeds_before_any_logs_exist() {
        let temp = TempDir::new().unwrap();
        let sources = sources_in(&temp);
        let sm = SessionManager::new(temp.path().join("sessions"));
        let session = sm
            .create_session(
                temp.path().to_path_buf(),
                "diagnostics".into(),
                SessionType::User,
            )
            .await
            .unwrap();

        let bundle = generate_diagnostics_from(&sm, &session.id, &sources)
            .await
            .unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bundle)).unwrap();

        assert!(archive.by_name("session.json").is_ok());
        assert!(archive.by_name("system.txt").is_ok());
        assert!(archive.by_name("usage.txt").is_ok());
    }

    /// A source that cannot be read must cost that source, not the report.
    ///
    /// This is the whole reason the collector is best-effort. The bundle exists
    /// so somebody can report a bug, and the machine producing one is by
    /// definition a machine where something is already wrong. A collector that
    /// refuses to produce anything because one file was unreadable fails at its
    /// only job, in exactly the situation it was built for.
    ///
    /// The transcript is the strongest case: it is the most valuable thing in
    /// the bundle, and still not worth failing over. A naming a session that
    /// does not exist stands in for every way `export_session` can fail (a
    /// store error, a corrupt row, a session deleted between the reach check
    /// and this call), because they all arrive here as the same `Err`.
    ///
    /// ⚠ The note is the load-bearing half. Silently omitting `session.json`
    /// would be WORSE than failing: whoever reads the report would conclude the
    /// chat was empty rather than that the transcript could not be read.
    #[tokio::test(flavor = "current_thread")]
    async fn an_unreadable_source_costs_that_source_and_not_the_whole_bundle() {
        let temp = TempDir::new().unwrap();
        let sources = sources_in(&temp);
        let sm = SessionManager::new(temp.path().join("sessions"));

        let bundle = generate_diagnostics_from(&sm, "no-such-session-20260820", &sources)
            .await
            .expect("an unreadable transcript must not lose the bundle");

        let entries = bundle_entries(bundle);
        let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();

        assert!(
            names.contains(&"system.txt"),
            "the parts that COULD be collected must still ship: {names:?}"
        );
        assert!(
            !names.contains(&"session.json"),
            "the part that could not be read must be absent rather than empty: {names:?}"
        );

        let notes = entries
            .iter()
            .find(|(name, _)| name == "collection-notes.txt")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
            .expect("a bundle missing a part must say which part");
        assert!(
            notes.contains("session.json"),
            "the note must name what is missing, or the reader concludes the chat was empty: {notes}"
        );
        assert!(
            notes.contains("no-such-session-20260820"),
            "and name the session it could not export: {notes}"
        );
    }

    /// ⚠ And the notes file must be ABSENT when nothing went wrong, so that its
    /// presence is itself the signal. A notes file on every bundle is one
    /// nobody opens.
    #[tokio::test(flavor = "current_thread")]
    async fn a_clean_collection_ships_no_notes_file() {
        let temp = TempDir::new().unwrap();
        let sources = sources_in(&temp);
        let sm = SessionManager::new(temp.path().join("sessions"));
        let session = sm
            .create_session(
                temp.path().to_path_buf(),
                "diagnostics".into(),
                SessionType::User,
            )
            .await
            .unwrap();

        let entries = bundle_entries(
            generate_diagnostics_from(&sm, &session.id, &sources)
                .await
                .unwrap(),
        );
        let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
        assert!(names.contains(&"session.json"), "{names:?}");
        assert!(
            !names.contains(&"collection-notes.txt"),
            "nothing failed, so nothing should be reported as having failed: {names:?}"
        );
    }

    /// Read every entry of a bundle as `(name, bytes)`.
    fn bundle_entries(bundle: Vec<u8>) -> Vec<(String, Vec<u8>)> {
        use std::io::Read;
        let mut archive = zip::ZipArchive::new(Cursor::new(bundle)).unwrap();
        (0..archive.len())
            .map(|i| {
                let mut file = archive.by_index(i).unwrap();
                let name = file.name().to_string();
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).unwrap();
                (name, bytes)
            })
            .collect()
    }

    /// ⚠ The assertion is on the **zip's bytes**, not on the filter's return
    /// value: a correct filter whose result the bundler ignores is the failure
    /// this test exists to catch.
    #[tokio::test(flavor = "current_thread")]
    async fn the_bundle_ships_only_the_named_session_s_llm_logs() {
        let temp = TempDir::new().unwrap();
        let sources = sources_in(&temp);
        let sm = SessionManager::new(temp.path().join("sessions"));
        let session = sm
            .create_session(
                temp.path().to_path_buf(),
                "diagnostics".into(),
                SessionType::User,
            )
            .await
            .unwrap();

        let logs_dir = sources.logs_dir.clone();
        fs::create_dir_all(&logs_dir).unwrap();

        // Three logs on this machine: one from the session being reported, one
        // from somebody else's chat, one from before headers existed.
        fs::write(
            logs_dir.join("llm_request.0.jsonl"),
            format!(
                "{{\"session_id\":\"{}\",\"model_config\":{{}}}}\n{{\"data\":\"MINE\"}}\n",
                session.id
            ),
        )
        .unwrap();
        fs::write(
            logs_dir.join("llm_request.1.jsonl"),
            "{\"session_id\":\"some-other-chat\"}\n{\"data\":\"THEIR PRIVATE PROMPT\"}\n",
        )
        .unwrap();
        fs::write(
            logs_dir.join("llm_request.2.jsonl"),
            "{\"model_config\":{}}\n{\"data\":\"UNATTRIBUTED PROMPT\"}\n",
        )
        .unwrap();

        let entries = bundle_entries(
            generate_diagnostics_from(&sm, &session.id, &sources)
                .await
                .unwrap(),
        );

        let log_names: Vec<&str> = entries
            .iter()
            .map(|(name, _)| name.as_str())
            .filter(|name| name.starts_with("logs/"))
            .collect();
        assert_eq!(log_names, vec!["logs/llm_request.0.jsonl"]);

        let all_bytes: Vec<u8> = entries.iter().flat_map(|(_, b)| b.clone()).collect();
        let all = String::from_utf8_lossy(&all_bytes);
        assert!(
            all.contains("MINE"),
            "the reported session's own log is missing"
        );
        assert!(
            !all.contains("THEIR PRIVATE PROMPT"),
            "another chat's transcript is in the bundle"
        );
        assert!(
            !all.contains("UNATTRIBUTED PROMPT"),
            "an unattributable transcript is in the bundle"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_bundled_config_has_its_credentials_redacted() {
        let temp = TempDir::new().unwrap();
        let sources = sources_in(&temp);
        let sm = SessionManager::new(temp.path().join("sessions"));
        let session = sm
            .create_session(temp.path().to_path_buf(), "diag".into(), SessionType::User)
            .await
            .unwrap();

        let config_dir = sources.config_path.parent().unwrap().to_path_buf();
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.yaml"),
            "BIOROUTER_PROVIDER: anthropic\n\
             ANTHROPIC_API_KEY: sk-ant-LIVE-CREDENTIAL\n\
             extensions:\n\
             \x20 spoke:\n\
             \x20   name: SPOKEAgent\n\
             \x20   envs:\n\
             \x20     SPOKEAGENT_PASSCODE: hunter2\n\
             \x20     CDW_USERNAME: CAMPUS\\\\jdoe\n",
        )
        .unwrap();

        let entries = bundle_entries(
            generate_diagnostics_from(&sm, &session.id, &sources)
                .await
                .unwrap(),
        );
        let (_, config) = entries
            .iter()
            .find(|(name, _)| name == "config.yaml")
            .expect("config.yaml is in the bundle");
        let config = String::from_utf8_lossy(config);

        assert!(!config.contains("sk-ant-LIVE-CREDENTIAL"), "{config}");
        assert!(!config.contains("hunter2"), "{config}");
        assert!(!config.contains("jdoe"), "{config}");
        // Still diagnostic: the shape of the config survives.
        assert!(config.contains("BIOROUTER_PROVIDER: anthropic"), "{config}");
        assert!(config.contains("SPOKEAgent"), "{config}");
    }

    /// The summary distinguishes "there were none" from "there were some and
    /// none of them could be attributed".
    ///
    /// ⚠ These two used to be the SAME bundle: an empty `logs/` directory and a
    /// directory full of `"session_id":null` headers both produced a zip with no
    /// `logs/` entry and no note. The second was the state of every machine
    /// running an ordinary chat, so a reader who took the bundle at face value
    /// concluded the app had made no requests.
    #[test]
    fn the_log_summary_separates_absent_logs_from_unattributable_ones() {
        let none = log_summary(&RequestLogSweep::default(), 0);
        assert!(
            none.contains("No log files were found on this machine at all"),
            "{none}"
        );
        assert!(!none.contains("unattributable"), "{none}");

        let all_excluded = log_summary(
            &RequestLogSweep {
                shipped: 0,
                excluded: 7,
            },
            0,
        );
        assert!(
            all_excluded.contains("names none at all: 7"),
            "{all_excluded}"
        );
        assert!(
            all_excluded.contains("Every request log on this machine was unattributable"),
            "{all_excluded}"
        );
        assert!(
            !all_excluded.contains("No log files were found on this machine at all"),
            "seven files were found; saying none were is the lie this fixes: {all_excluded}"
        );
    }

    /// Only WARN and ERROR survive the component-log filter.
    ///
    /// The level test is what stands in for `log_belongs_to_session` on a log
    /// that names no session: INFO and DEBUG are where another chat's content
    /// is, so a filter that let them through would re-open the defect the
    /// request-log attribution rule closed.
    #[test]
    fn component_logs_keep_only_diagnostic_levels() {
        let raw = concat!(
            "2026-09-05T17:34:53.579160Z  INFO biorouterd::x: listening on 127.0.0.1:1\n",
            "2026-09-05T17:34:54.000000Z DEBUG biorouter::y: prompt fragment from another chat\n",
            "2026-09-05T17:34:55.000000Z  WARN biorouter::z: extension failed to load\n",
            "2026-09-05T17:34:56.000000Z ERROR biorouter::w: provider returned 429\n",
            "2026-09-05T17:34:57.000000Z TRACE biorouter::v: noise\n",
        );
        let filtered = tail_diagnostic_lines(raw, 64 * 1024);
        assert!(filtered.contains("extension failed to load"), "{filtered}");
        assert!(filtered.contains("provider returned 429"), "{filtered}");
        assert!(!filtered.contains("listening on"), "{filtered}");
        assert!(
            !filtered.contains("prompt fragment from another chat"),
            "a DEBUG line carries other sessions' content: {filtered}"
        );
        assert!(!filtered.contains("noise"), "{filtered}");
        // Order is preserved, newest last, so a reader scans down to the failure.
        let warn = filtered.find("extension failed").unwrap();
        let error = filtered.find("provider returned").unwrap();
        assert!(warn < error, "{filtered}");
    }

    /// A log with nothing but INFO produces no entry rather than an empty file.
    #[test]
    fn a_component_log_with_no_warnings_ships_nothing() {
        let raw = "2026-09-05T17:34:53.579160Z  INFO biorouterd::x: fine\n";
        assert!(tail_diagnostic_lines(raw, 1024).is_empty());
        assert!(tail_diagnostic_lines("", 1024).is_empty());
        // A line the parser cannot place is not silently promoted.
        assert!(tail_diagnostic_lines("panicked at 'boom'\n", 1024).is_empty());
    }

    /// The tail is kept, not the head: a long-running daemon's failure is at
    /// the end of its log.
    #[test]
    fn the_component_log_tail_is_what_survives_the_budget() {
        let mut raw = String::new();
        for i in 0..500 {
            raw.push_str(&format!(
                "2026-09-05T17:34:53.579160Z ERROR biorouter::x: failure number {i}\n"
            ));
        }
        let filtered = tail_diagnostic_lines(&raw, 512);
        assert!(filtered.contains("failure number 499"), "{filtered}");
        assert!(!filtered.contains("failure number 0\n"), "{filtered}");
        assert!(filtered.len() < 1024, "{}", filtered.len());
    }

    /// Every bundle carries the summary, whatever the sweep found.
    #[tokio::test(flavor = "current_thread")]
    async fn every_bundle_carries_a_log_summary() {
        let temp = TempDir::new().unwrap();
        let sources = sources_in(&temp);
        let sm = SessionManager::new(temp.path().join("sessions"));
        let session = sm
            .create_session(
                temp.path().to_path_buf(),
                "diagnostics".into(),
                SessionType::User,
            )
            .await
            .unwrap();

        let entries = bundle_entries(
            generate_diagnostics_from(&sm, &session.id, &sources)
                .await
                .unwrap(),
        );
        let summary = entries
            .iter()
            .find(|(name, _)| name == "logs-summary.txt")
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
            .expect("every bundle explains what its log sweep found");
        assert!(summary.contains("Request logs"), "{summary}");
    }

    /// The daemon's own log reaches the bundle at all.
    ///
    /// ⚠ It never did: the sweep walked the ROOT of `logs/` for `*.jsonl`, so
    /// `logs/server/<date>/*.log` was not a candidate under any circumstances.
    #[tokio::test(flavor = "current_thread")]
    async fn the_daemon_s_own_warnings_reach_the_bundle() {
        let temp = TempDir::new().unwrap();
        let sources = sources_in(&temp);
        let day = sources.logs_dir.clone().join("server").join("2026-09-05");
        fs::create_dir_all(&day).unwrap();
        fs::write(
            day.join("20260905_103453-biorouterd.log"),
            "2026-09-05T17:34:53.579160Z  INFO biorouterd::x: listening\n\
             2026-09-05T17:34:55.000000Z ERROR biorouter::z: the extension did not load\n",
        )
        .unwrap();

        let sm = SessionManager::new(temp.path().join("sessions"));
        let session = sm
            .create_session(
                temp.path().to_path_buf(),
                "diagnostics".into(),
                SessionType::User,
            )
            .await
            .unwrap();

        let entries = bundle_entries(
            generate_diagnostics_from(&sm, &session.id, &sources)
                .await
                .unwrap(),
        );
        let (name, bytes) = entries
            .iter()
            .find(|(name, _)| name.starts_with("logs/server/"))
            .expect("the daemon's own log is in the bundle");
        let text = String::from_utf8_lossy(bytes);
        assert!(name.ends_with("-biorouterd.log"), "{name}");
        assert!(text.contains("the extension did not load"), "{text}");
        assert!(!text.contains("listening"), "INFO must not ship: {text}");
    }

    #[test]
    fn an_unattributable_log_is_not_this_session_s() {
        assert!(log_belongs_to_session(
            "{\"session_id\":\"abc\"}\n{\"data\":1}",
            "abc"
        ));
        assert!(!log_belongs_to_session("{\"session_id\":\"abc\"}\n", "def"));
        assert!(!log_belongs_to_session("{\"session_id\":null}\n", "abc"));
        assert!(!log_belongs_to_session("{\"model_config\":{}}\n", "abc"));
        assert!(!log_belongs_to_session("not json at all\n", "abc"));
        assert!(!log_belongs_to_session("", "abc"));
    }

    #[test]
    fn redaction_covers_credential_keys_and_every_extension_env() {
        let out = redact_config_yaml(
            "OPENAI_API_KEY: sk-live\n\
             GITHUB_TOKEN: ghp_live\n\
             harmless: keep-me\n\
             extensions:\n\
             \x20 a:\n\
             \x20   envs:\n\
             \x20     ANY_NAME_AT_ALL: value\n",
        );
        assert!(!out.contains("sk-live"), "{out}");
        assert!(!out.contains("ghp_live"), "{out}");
        assert!(!out.contains("value"), "{out}");
        assert!(out.contains("keep-me"), "{out}");
    }

    #[test]
    fn unparseable_config_is_replaced_rather_than_shipped() {
        // A tab in the indentation: YAML forbids it, so this cannot parse.
        let out = redact_config_yaml("root:\n\tANTHROPIC_API_KEY: sk-live\n");
        assert!(!out.contains("sk-live"), "{out}");
        assert!(out.starts_with('#'), "{out}");
    }

    #[tokio::test]
    async fn usage_diagnostics_records_cache_and_distinguishes_last_turn_from_accumulated() {
        let temp = TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        let session = sm
            .create_session(temp.path().to_path_buf(), "diag".into(), SessionType::User)
            .await
            .unwrap();

        sm.apply_usage_event(UsageLedgerEntry {
            event_key: "diagnostics-provider-call-1".to_string(),
            session_id: session.id.clone(),
            schedule_id: None,
            current_total_tokens: Some(740),
            current_input_tokens: Some(100),
            current_output_tokens: Some(40),
            billed_total_tokens: Some(740),
            input_tokens: Some(100),
            output_tokens: Some(40),
            model_id: Some("claude-sonnet-4-20250514".to_string()),
            provider: Some("anthropic".to_string()),
            cache_read_tokens: Some(500),
            cache_creation_tokens: Some(100),
        })
        .await
        .unwrap();
        sm.apply_usage_event(UsageLedgerEntry {
            event_key: "diagnostics-provider-call-2".to_string(),
            session_id: session.id.clone(),
            schedule_id: None,
            current_total_tokens: Some(215),
            current_input_tokens: Some(10),
            current_output_tokens: Some(5),
            billed_total_tokens: Some(215),
            input_tokens: Some(10),
            output_tokens: Some(5),
            model_id: Some("claude-sonnet-4-20250514".to_string()),
            provider: Some("anthropic".to_string()),
            cache_read_tokens: Some(200),
            cache_creation_tokens: Some(0),
        })
        .await
        .unwrap();

        let diag = UsageDiagnostics::collect(&sm, &session.id).await.unwrap();
        assert_eq!(diag.last_turn_total_tokens, Some(215));
        assert_eq!(diag.last_turn_input_tokens, Some(10));
        assert_eq!(diag.last_turn_output_tokens, Some(5));
        assert_eq!(diag.accumulated_total_tokens, Some(955));
        assert_eq!(diag.accumulated_input_tokens, Some(110));
        assert_eq!(diag.accumulated_output_tokens, Some(45));
        // Session cache totals sum across turns: read 500+200=700, creation 100.
        assert_eq!(diag.session_cache_read_tokens, Some(700));
        assert_eq!(diag.session_cache_creation_tokens, Some(100));
        assert_eq!(diag.per_model.len(), 1);
        assert_eq!(diag.per_model[0].cache_read_tokens, Some(700));
        assert_eq!(diag.per_model[0].turns, 2);
        // MTD picks up both turns; priced because Claude is on the pricing card.
        assert!(diag.month_to_date.cost.is_some());
        assert_eq!(diag.month_to_date.cache_read_tokens, Some(700));

        let text = diag.to_text();
        assert!(text.contains("Last turn"));
        assert!(text.contains("Accumulated session counters"));
        assert!(text.contains("cache_read=700"));
        assert!(text.contains("Month-to-date"));
    }
}
