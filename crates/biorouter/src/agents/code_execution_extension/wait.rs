//! `code_execution__wait_for`, and the `wait_for()` and `sleep()` functions
//! inside `execute_code` scripts: waiting on a long-running job with no model
//! turn per check.
//!
//! Every model turn re-reads the whole conversation. A session that waited on
//! database jobs by returning to the model after each status check spent a
//! quarter of its input tokens on those checks (118 of 575 requests). Here the
//! host calls the status tool itself, sleeps between checks without the
//! model, and hands back one compact result. See
//! `docs/agent-loop/waiting-on-jobs.md`.
//!
//! The loop ([`run_wait`]) knows nothing about dispatch: each check comes from
//! a [`StatusCheck`], which in production is the same admit-then-dispatch path
//! a script's own tool calls take (`CodeExecutionClient::run_check`), so the
//! permission judge, the boundary refusals, Gate C and the secret guard all
//! apply to every check.

use crate::tool_monitor::job_status::JobActivity;
use futures::future::BoxFuture;
use rmcp::model::JsonObject;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// The reserved request names `sleep()` and `wait_for()` send down the
/// script's call channel. A NUL cannot appear in a catalogue tool name.
pub(super) const SLEEP_REQUEST: &str = "\u{0}biorouter.sleep";
pub(super) const WAIT_REQUEST: &str = "\u{0}biorouter.wait_for";
/// The same, for `watch_job()` (`super::watch`).
pub(super) const WATCH_REQUEST: &str = "\u{0}biorouter.watch_job";

/// Whether a request down the script's call channel is one of the host
/// functions above rather than a tool call.
pub(super) fn is_host_request(name: &str) -> bool {
    [SLEEP_REQUEST, WAIT_REQUEST, WATCH_REQUEST].contains(&name)
}

/// `wait_for`'s timeout when none is given, in seconds.
pub(super) const DEFAULT_TIMEOUT_SECS: u64 = 240;
/// The longest one `wait_for` call waits.
///
/// The 300 s per-call timeout (`DEFAULT_EXTENSION_TIMEOUT`) is enforced by
/// `mcp_client::await_response`, on the transport of an MCP extension. Code
/// Execution is an in-process platform extension and its `call_tool` is
/// awaited with no timeout (`ExtensionManager::dispatch_tool_call` adds none),
/// so a `wait_for` can block longer than that; each CHECK it makes is an
/// ordinary short call to the status tool and stays under the transport
/// limit. Ten minutes keeps a person's Stop and steer the only waits that
/// matter: a longer wait belongs to `watch_job`, which ends the turn.
pub(super) const MAX_TIMEOUT_SECS: u64 = 600;
/// Seconds between checks when none is given.
pub(super) const DEFAULT_MIN_INTERVAL_SECS: u64 = 30;
/// The shortest interval allowed between checks.
pub(super) const MIN_INTERVAL_FLOOR_SECS: u64 = 10;
/// The longest one `sleep()` call may be.
pub(super) const MAX_SLEEP_SECS: u64 = 280;
/// How long one script may spend in `sleep()` and `wait_for()` together.
pub(super) const SCRIPT_WAIT_BUDGET_SECS: u64 = 600;
/// How much of the last status result the model is shown.
pub(super) const MAX_LAST_RESULT_BYTES: usize = 8 * 1024;
/// The most of a job's earlier output ([`DrainedOutput`]) a wait result or a
/// wake message carries, its end kept.
pub(super) const MAX_EARLIER_OUTPUT_BYTES: usize = 8 * 1024;

/// One "second" of waiting. Unit tests of this crate run the same arithmetic
/// on a 10 ms second so they do not sit through real minutes.
pub(super) fn wait_unit() -> Duration {
    if cfg!(test) {
        Duration::from_millis(10)
    } else {
        Duration::from_secs(1)
    }
}

/// A number of seconds, given as a number or a numeric string.
pub(super) fn de_seconds<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    let value = Option::<Value>::deserialize(deserializer)?;
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => Ok(number.as_f64()),
        Some(Value::String(text)) => {
            text.trim().parse::<f64>().map(Some).map_err(|_| {
                serde::de::Error::custom(format!("`{text}` is not a number of seconds"))
            })
        }
        Some(other) => Err(serde::de::Error::custom(format!(
            "expected a number of seconds, got {other}"
        ))),
    }
}

/// When `wait_for` stops.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub(super) enum WaitUntilParam {
    /// "inactive" (the default): stop when the result no longer reads as an
    /// active job. "changed": stop when the result changes (timing fields such
    /// as elapsed_seconds or heartbeat_age are ignored).
    Keyword(String),
    /// Stop when the value at the JSON pointer (for example "/status") equals
    /// one of the values (strings compare without regard to case).
    Pointer { pointer: String, values: Vec<Value> },
}

/// Arguments of `code_execution__wait_for` and of `wait_for()` in a script.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(super) struct WaitForParams {
    /// Full name of the status tool to call, server__tool (for example
    /// "cdwagent__CDW-query_job_status").
    pub tool: String,
    /// The status tool's arguments (for example {"job_id": "abc"}).
    #[serde(default)]
    pub arguments: Option<JsonObject>,
    /// When to stop: omit for "inactive", or "changed", or
    /// {"pointer": "/status", "values": ["completed", "failed"]}.
    #[serde(default)]
    pub until: Option<WaitUntilParam>,
    /// Seconds to wait in all before returning (default 240, at most 600).
    /// On timeout the job is still running: call wait_for again, or
    /// watch_job to be woken when it finishes.
    #[serde(default, deserialize_with = "de_seconds")]
    #[schemars(with = "Option<f64>")]
    pub timeout_seconds: Option<f64>,
    /// Seconds between checks (default 30, at least 10). A longer
    /// recommended_poll_seconds, retry_after or poll_after in the status
    /// result is honoured.
    #[serde(default, deserialize_with = "de_seconds")]
    #[schemars(with = "Option<f64>")]
    pub min_interval_seconds: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Until {
    Inactive,
    Changed,
    Pointer { pointer: String, values: Vec<Value> },
}

/// A validated wait.
#[derive(Debug, Clone)]
pub(super) struct WaitPlan {
    pub tool: String,
    pub arguments: JsonObject,
    pub until: Until,
    /// Seconds.
    pub timeout: u64,
    /// Seconds.
    pub min_interval: u64,
}

fn whole_seconds(value: f64, name: &str) -> Result<u64, String> {
    if !value.is_finite() || value < 0.0 {
        return Err(format!("{name} must be a number of seconds, 0 or more"));
    }
    Ok(value.ceil() as u64)
}

impl WaitPlan {
    /// Validate `params`. `max_timeout` is [`MAX_TIMEOUT_SECS`] for the
    /// top-level tool, and what is left of the script's budget inside one.
    pub(super) fn from_params(params: WaitForParams, max_timeout: u64) -> Result<Self, String> {
        let tool = params.tool.trim().to_string();
        if tool.is_empty() || !tool.contains("__") {
            return Err(format!(
                "wait_for needs `tool`, the status tool's full name as server__tool \
                 (got `{tool}`)"
            ));
        }
        if tool.starts_with(&format!("{}__", super::EXTENSION_NAME))
            || super::is_execute_code_call(&tool)
        {
            return Err(
                "wait_for cannot wait on code_execution's own tools: name the status tool \
                 of the extension that runs the job"
                    .to_string(),
            );
        }
        if crate::agents::platform_tools::is_platform_tool_name(&tool) {
            return Err(super::agent_loop_tool_refusal(&tool));
        }
        let until = match params.until {
            None => Until::Inactive,
            Some(WaitUntilParam::Keyword(word)) => {
                match word.trim().to_ascii_lowercase().as_str() {
                    "" | "inactive" | "not_active" | "finished" | "done" | "terminal" => {
                        Until::Inactive
                    }
                    "changed" | "change" | "any_change" => Until::Changed,
                    other => {
                        return Err(format!(
                            "wait_for's `until` is \"inactive\", \"changed\" or \
                             {{\"pointer\": \"/status\", \"values\": [\"completed\"]}}, not \
                             `{other}`"
                        ))
                    }
                }
            }
            Some(WaitUntilParam::Pointer { pointer, values }) => {
                if !(pointer.is_empty() || pointer.starts_with('/')) {
                    return Err(format!(
                        "wait_for's `until.pointer` is a JSON pointer such as \"/status\", \
                         not `{pointer}`"
                    ));
                }
                if values.is_empty() {
                    return Err("wait_for's `until.values` lists at least one value".to_string());
                }
                Until::Pointer { pointer, values }
            }
        };
        let timeout = match params.timeout_seconds {
            Some(value) => whole_seconds(value, "timeout_seconds")?,
            None => DEFAULT_TIMEOUT_SECS,
        }
        .min(max_timeout);
        let min_interval = match params.min_interval_seconds {
            Some(value) => whole_seconds(value, "min_interval_seconds")?,
            None => DEFAULT_MIN_INTERVAL_SECS,
        }
        .clamp(MIN_INTERVAL_FLOOR_SECS, MAX_TIMEOUT_SECS);
        Ok(Self {
            tool,
            arguments: params.arguments.unwrap_or_default(),
            until,
            timeout,
            min_interval,
        })
    }

    /// Seconds to sleep after a check whose result asked for `hint` seconds.
    pub(super) fn interval_after(&self, hint: Option<u64>) -> u64 {
        self.min_interval
            .max(hint.unwrap_or(0))
            .min(MAX_TIMEOUT_SECS)
    }
}

/// What one check returned.
#[derive(Debug, Clone)]
pub(super) struct CheckReading {
    /// The value a script would have been handed, or the check's error.
    pub value: Result<String, String>,
    pub activity: JobActivity,
    pub hint_secs: Option<u64>,
}

/// One status check, as the loop sees it.
pub(super) trait StatusCheck: Send {
    fn check(&mut self) -> BoxFuture<'_, CheckReading>;
}

/// Why a wait ended.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum WaitEnd {
    /// The result no longer reads as an active job.
    Finished,
    /// The first result named no job state, under `until: "inactive"`.
    NotAJob,
    /// The result changed, under `until: "changed"`.
    Changed,
    /// The pointer's value is one of the values.
    Matched(Value),
    /// The timeout passed while the job was still active.
    TimedOut,
    /// A check failed.
    Failed(String),
    /// Stop was pressed.
    Cancelled,
}

#[derive(Debug, Clone)]
pub(super) struct WaitReport {
    pub end: WaitEnd,
    pub checks: u32,
    /// In seconds, rounded up.
    pub elapsed_secs: u64,
    pub last: Option<String>,
    /// What the job printed before the last check, for a status tool that
    /// hands its output out once ([`DrainedOutput::earlier`]).
    pub earlier_output: Option<String>,
}

/// What a job printed across the checks of one wait, for a status tool that
/// hands each piece of its output out once.
///
/// The developer shell's `shell_status` returns only what the job printed
/// since the last call, then moves past it
/// (`job_status::hands_out_output_once`). A wait or a watch makes those calls
/// itself, so keeping only the last result threw away everything the job
/// printed before the final check, and a later call of the tool returns
/// "(no new output)": the output was gone. This keeps every check's new
/// output except the latest one, which the last result shows, up to
/// [`MAX_EARLIER_OUTPUT_BYTES`]. The end is kept rather than the start, since
/// a job's last lines say how it went.
#[derive(Debug, Clone, Default)]
pub(super) struct DrainedOutput {
    /// The output of the checks before the latest one.
    earlier: String,
    /// Bytes dropped from the front of `earlier` to keep it in bounds.
    dropped: usize,
    /// The latest check's new output, which its result shows itself.
    latest: Option<String>,
}

impl DrainedOutput {
    /// Take one check's result. Nothing is kept for a tool that does not hand
    /// its output out once.
    pub(super) fn record(&mut self, tool: &str, value: &str) {
        if let Some(previous) = self.latest.take() {
            self.keep(&previous);
        }
        self.latest =
            crate::tool_monitor::job_status::new_output_in(tool, value).map(str::to_string);
    }

    fn keep(&mut self, text: &str) {
        if !self.earlier.is_empty() {
            self.earlier.push('\n');
        }
        self.earlier.push_str(text);
        let excess = self.earlier.len().saturating_sub(MAX_EARLIER_OUTPUT_BYTES);
        if excess == 0 {
            return;
        }
        let mut start = excess;
        while !self.earlier.is_char_boundary(start) {
            start += 1;
        }
        // Start on a whole line when one begins soon after the cut.
        if let Some(newline) = self
            .earlier
            .get(start..)
            .and_then(|rest| rest.find('\n'))
            .filter(|at| *at < 200)
        {
            start += newline + 1;
        }
        self.earlier.drain(..start);
        self.dropped += start;
    }

    /// The output of the checks before the latest one, or `None` when they
    /// printed nothing.
    pub(super) fn earlier(&self) -> Option<String> {
        if self.earlier.trim().is_empty() {
            return None;
        }
        Some(if self.dropped == 0 {
            self.earlier.clone()
        } else {
            format!(
                "[{} earlier bytes not kept]\n{}",
                self.dropped, self.earlier
            )
        })
    }
}

/// The section a wait result or a wake message carries the job's earlier
/// output under.
pub(super) fn earlier_output_section(tool: &str, earlier: &str) -> String {
    format!(
        "Output from the earlier checks ({tool} returns only what is new since its last call, \
         so this is the only copy):\n{earlier}"
    )
}

/// Keys whose values change with the clock, not with the job, so `"changed"`
/// ignores them.
fn is_timing_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    matches!(
        key.as_str(),
        "age" | "now" | "time" | "eta" | "eta_seconds" | "timestamp"
    ) || key.ends_with("_age")
        || [
            "elapsed",
            "heartbeat",
            "poll",
            "retry_after",
            "updated_at",
            "checked_at",
            "last_seen",
            "duration",
            "uptime",
            "server_time",
            "remaining",
        ]
        .iter()
        .any(|part| key.contains(part))
}

fn without_timing(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .filter(|(key, _)| !is_timing_key(key))
                .map(|(key, value)| (key.clone(), without_timing(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(without_timing).collect()),
        other => other.clone(),
    }
}

/// What `"changed"` compares: the JSON without its timing fields, or the
/// text with its whitespace collapsed.
pub(super) fn comparable(value: &str) -> String {
    match serde_json::from_str::<Value>(value.trim()) {
        Ok(json) => without_timing(&json).to_string(),
        Err(_) => value.split_whitespace().collect::<Vec<_>>().join(" "),
    }
}

pub(super) fn pointer_matches(found: &Value, values: &[Value]) -> bool {
    values.iter().any(|wanted| match (found, wanted) {
        (Value::String(found), Value::String(wanted)) => found.eq_ignore_ascii_case(wanted),
        (found, wanted) => found == wanted,
    })
}

fn seconds_up(duration: Duration, unit: Duration) -> u64 {
    let unit = unit.as_nanos().max(1);
    u64::try_from(duration.as_nanos().div_ceil(unit)).unwrap_or(u64::MAX)
}

/// Whether check number `checks` (counted from 1), which returned `value`,
/// ends a wait on `until`. `first` is the comparable form of the first
/// result, kept for `"changed"`; the first check fills it in.
pub(super) fn end_of(
    until: &Until,
    value: &str,
    activity: JobActivity,
    checks: u32,
    first: &mut Option<String>,
) -> Option<WaitEnd> {
    match until {
        Until::Inactive => match activity {
            JobActivity::Active => None,
            JobActivity::Finished => Some(WaitEnd::Finished),
            JobActivity::Unknown if checks == 1 => Some(WaitEnd::NotAJob),
            JobActivity::Unknown => Some(WaitEnd::Finished),
        },
        Until::Changed => {
            let now = comparable(value);
            match first {
                None => {
                    *first = Some(now);
                    None
                }
                Some(first) if *first != now => Some(WaitEnd::Changed),
                Some(_) => None,
            }
        }
        Until::Pointer { pointer, values } => serde_json::from_str::<Value>(value.trim())
            .ok()
            .and_then(|json| json.pointer(pointer).cloned())
            .filter(|found| pointer_matches(found, values))
            .map(WaitEnd::Matched),
    }
}

/// A short form of a status result for a message the model reads later: the
/// JSON without its timing fields, or, when that is still too long, only its
/// top-level scalar fields; text is cut. At most `max_chars` characters, plus
/// a sentence naming the cut.
pub(super) fn compact_state(last: &str, max_chars: usize) -> String {
    fn cut(text: &str, max_chars: usize) -> Option<String> {
        if text.chars().count() <= max_chars {
            return None;
        }
        Some(text.chars().take(max_chars).collect())
    }
    let trimmed = last.trim();
    let Ok(json) = serde_json::from_str::<Value>(trimmed) else {
        // Text keeps its start and its end: the end of a job's output is
        // where it says how it went.
        let count = trimmed.chars().count();
        if count <= max_chars {
            return trimmed.to_string();
        }
        let head_chars = max_chars / 4;
        let head: String = trimmed.chars().take(head_chars).collect();
        let tail: String = trimmed
            .chars()
            .skip(count - (max_chars - head_chars))
            .collect();
        return format!("{head}\n[...]\n{tail}\n(cut; {count} characters in all)");
    };
    let quiet = without_timing(&json);
    let full = quiet.to_string();
    if full.chars().count() <= max_chars {
        return full;
    }
    if let Value::Object(object) = &quiet {
        let mut dropped_nested = false;
        let scalars: serde_json::Map<String, Value> = object
            .iter()
            .filter_map(|(key, value)| match value {
                Value::String(text) => Some((
                    key.clone(),
                    Value::String(
                        cut(text, 200).map_or_else(|| text.clone(), |head| format!("{head}...")),
                    ),
                )),
                Value::Number(_) | Value::Bool(_) | Value::Null => {
                    Some((key.clone(), value.clone()))
                }
                _ => {
                    dropped_nested = true;
                    None
                }
            })
            .collect();
        let short = Value::Object(scalars).to_string();
        if short.chars().count() <= max_chars {
            let what = if dropped_nested {
                "nested fields left out"
            } else {
                "long values cut"
            };
            return format!("{short} ({what})");
        }
    }
    format!(
        "{}... (cut; {} characters in all)",
        cut(&full, max_chars).unwrap_or_default(),
        full.chars().count()
    )
}

/// Call `checker` until `plan` says stop, the timeout passes, or Stop.
pub(super) async fn run_wait(
    plan: &WaitPlan,
    checker: &mut dyn StatusCheck,
    unit: Duration,
    cancellation_token: &CancellationToken,
) -> WaitReport {
    let started = tokio::time::Instant::now();
    let deadline = started + unit * u32::try_from(plan.timeout).unwrap_or(u32::MAX);
    let mut checks = 0u32;
    let mut first: Option<String> = None;
    let mut last: Option<String> = None;
    let mut output = DrainedOutput::default();
    let report = |end, checks, last, output: &DrainedOutput| WaitReport {
        end,
        checks,
        elapsed_secs: seconds_up(started.elapsed(), unit),
        last,
        earlier_output: output.earlier(),
    };
    loop {
        if cancellation_token.is_cancelled() {
            return report(WaitEnd::Cancelled, checks, last, &output);
        }
        let reading = tokio::select! {
            biased;
            () = cancellation_token.cancelled() => {
                return report(WaitEnd::Cancelled, checks, last, &output)
            }
            reading = checker.check() => reading,
        };
        checks += 1;
        let value = match reading.value {
            Ok(value) => value,
            Err(error) => return report(WaitEnd::Failed(error), checks, last, &output),
        };
        output.record(&plan.tool, &value);
        last = Some(value.clone());
        if let Some(end) = end_of(&plan.until, &value, reading.activity, checks, &mut first) {
            return report(end, checks, last, &output);
        }
        let now = tokio::time::Instant::now();
        let remaining = deadline.saturating_duration_since(now);
        // Less than a second left: the check just made was the last one.
        // Otherwise sleep to the next check, or to the deadline for one last
        // check there.
        if remaining < unit {
            return report(WaitEnd::TimedOut, checks, last, &output);
        }
        let interval = unit * u32::try_from(plan.interval_after(reading.hint_secs)).unwrap_or(1);
        tokio::select! {
            biased;
            () = cancellation_token.cancelled() => {
                return report(WaitEnd::Cancelled, checks, last, &output)
            }
            () = tokio::time::sleep(interval.min(remaining)) => {}
        }
    }
}

pub(super) fn duration_words(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds} s")
    } else {
        format!("{} min {} s", seconds / 60, seconds % 60)
    }
}

/// `last`, the last result of `tool`, cut to [`MAX_LAST_RESULT_BYTES`]: its
/// start and its end, since the end of a job's output is where it says how it
/// went.
pub(super) fn shown_result(tool: &str, last: &str) -> String {
    if last.len() <= MAX_LAST_RESULT_BYTES {
        return last.to_string();
    }
    let mut head_end = MAX_LAST_RESULT_BYTES / 4;
    while !last.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = last.len() - (MAX_LAST_RESULT_BYTES - MAX_LAST_RESULT_BYTES / 4);
    while !last.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    let again = if crate::tool_monitor::job_status::hands_out_output_once(tool) {
        ""
    } else {
        " Call the status tool directly to read all of it."
    };
    format!(
        "{}\n[...]\n{}\n(The result is {} bytes; its first {head_end} and last {} are shown.{again})",
        &last[..head_end],
        &last[tail_start..],
        last.len(),
        last.len() - tail_start,
    )
}

pub(super) fn checks_words(checks: u32) -> String {
    if checks == 1 {
        "1 check".to_string()
    } else {
        format!("{checks} checks")
    }
}

/// What the model is told. `Err` for a failed or cancelled wait. `watching`
/// is the id of this chat's live `watch_job` watch of the same call, if any:
/// a timeout then tells the model to end its turn rather than offering a
/// second watch.
pub(super) fn report_text(
    plan: &WaitPlan,
    report: &WaitReport,
    watching: Option<&str>,
) -> Result<String, String> {
    let over = format!(
        "{} over {}",
        checks_words(report.checks),
        duration_words(report.elapsed_secs)
    );
    let headline = match &report.end {
        WaitEnd::Finished => format!("wait_for: the job is no longer active ({over})."),
        WaitEnd::Changed => format!("wait_for: the result changed ({over})."),
        WaitEnd::Matched(value) => format!(
            "wait_for: {} is {value} ({over}).",
            match &plan.until {
                Until::Pointer { pointer, .. } => pointer.as_str(),
                _ => "the value",
            }
        ),
        WaitEnd::NotAJob => format!(
            "wait_for stopped after one check: the result of {} does not name a job state (no \
             status, state or phase field reading queued, running, completed and so on). To \
             wait on this result, call wait_for again with until: \"changed\" or until: \
             {{\"pointer\": \"/field\", \"values\": [...]}}.",
            plan.tool
        ),
        WaitEnd::TimedOut => match watching {
            Some(id) => format!(
                "wait_for timed out after {} ({}): the job is still running. {id} already \
                 watches this job, so end your turn now: this chat resumes by itself when it \
                 finishes. Do not check the job again meanwhile.",
                duration_words(report.elapsed_secs),
                checks_words(report.checks)
            ),
            None => format!(
                "wait_for timed out after {} ({}): the job is still running. Call \
                 code_execution__wait_for again with the same arguments to keep waiting (one \
                 model turn per wait instead of one per check), or code_execution__watch_job \
                 and end your turn to be woken when it finishes (no model turn until then).",
                duration_words(report.elapsed_secs),
                checks_words(report.checks)
            ),
        },
        WaitEnd::Failed(error) => {
            return Err(format!(
                "wait_for stopped: check {} of {} failed ({over}): {error}",
                report.checks, plan.tool
            ))
        }
        WaitEnd::Cancelled => return Err("wait_for was cancelled".to_string()),
    };
    let mut text = headline;
    if let Some(earlier) = &report.earlier_output {
        text.push('\n');
        text.push_str(&earlier_output_section(&plan.tool, earlier));
    }
    if let Some(last) = &report.last {
        text.push_str(&format!(
            "\nLast result of {}:\n{}",
            plan.tool,
            shown_result(&plan.tool, last)
        ));
    }
    Ok(text)
}

#[derive(Debug, Serialize)]
struct ScriptReport<'a> {
    state: &'a str,
    checks: u32,
    elapsed_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<&'a Value>,
    last_result: Value,
    /// What the job printed before the last check, for a status tool that
    /// hands its output out once ([`DrainedOutput`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    earlier_output: Option<&'a str>,
    /// The live watch of the same call, on a timeout: end the script and the
    /// turn, and the chat resumes when the job ends.
    #[serde(skip_serializing_if = "Option::is_none")]
    watch_id: Option<&'a str>,
}

/// What a script's `wait_for()` returns, as JSON. `Err` for a failed or
/// cancelled wait, which the script receives as a thrown error.
pub(super) fn report_json(
    plan: &WaitPlan,
    report: &WaitReport,
    watching: Option<&str>,
) -> Result<String, String> {
    let (state, value) = match &report.end {
        WaitEnd::Finished => ("finished", None),
        WaitEnd::Changed => ("changed", None),
        WaitEnd::Matched(value) => ("matched", Some(value)),
        WaitEnd::NotAJob => ("not_a_job", None),
        WaitEnd::TimedOut => ("timed_out", None),
        WaitEnd::Failed(_) | WaitEnd::Cancelled => {
            return report_text(plan, report, None).map(|_| String::new())
        }
    };
    let last_result = report.last.as_deref().map_or(Value::Null, |last| {
        serde_json::from_str::<Value>(last.trim()).unwrap_or_else(|_| Value::String(last.into()))
    });
    serde_json::to_string(&ScriptReport {
        state,
        checks: report.checks,
        elapsed_seconds: report.elapsed_secs,
        value,
        last_result,
        earlier_output: report.earlier_output.as_deref(),
        watch_id: watching.filter(|_| report.end == WaitEnd::TimedOut),
    })
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    struct Scripted {
        readings: Vec<CheckReading>,
        calls: Arc<AtomicU32>,
        at: Vec<tokio::time::Instant>,
    }

    impl Scripted {
        fn new(readings: Vec<CheckReading>) -> Self {
            Self {
                readings,
                calls: Arc::new(AtomicU32::new(0)),
                at: Vec::new(),
            }
        }
    }

    impl StatusCheck for Scripted {
        fn check(&mut self) -> BoxFuture<'_, CheckReading> {
            Box::pin(async move {
                let index = self.calls.fetch_add(1, Ordering::SeqCst) as usize;
                self.at.push(tokio::time::Instant::now());
                self.readings[index.min(self.readings.len() - 1)].clone()
            })
        }
    }

    fn reading(value: Value, activity: JobActivity, hint: Option<u64>) -> CheckReading {
        CheckReading {
            value: Ok(value.to_string()),
            activity,
            hint_secs: hint,
        }
    }

    fn plan(until: Until, timeout: u64, min_interval: u64) -> WaitPlan {
        WaitPlan {
            tool: "cdw__status".into(),
            arguments: JsonObject::new(),
            until,
            timeout,
            min_interval,
        }
    }

    const UNIT: Duration = Duration::from_secs(1);

    #[tokio::test(start_paused = true)]
    async fn stops_when_the_job_is_no_longer_active() {
        let mut checker = Scripted::new(vec![
            reading(json!({"phase": "queued"}), JobActivity::Active, None),
            reading(json!({"phase": "executing"}), JobActivity::Active, None),
            reading(json!({"phase": "completed"}), JobActivity::Finished, None),
        ]);
        let plan = plan(Until::Inactive, 240, 30);
        let report = run_wait(&plan, &mut checker, UNIT, &CancellationToken::new()).await;
        assert_eq!(report.end, WaitEnd::Finished);
        assert_eq!(report.checks, 3);
        assert_eq!(report.elapsed_secs, 60);
        let text = report_text(&plan, &report, None).unwrap();
        assert!(
            text.contains("no longer active (3 checks over 1 min 0 s)"),
            "{text}"
        );
        assert!(text.contains("\"completed\""), "{text}");
    }

    #[tokio::test(start_paused = true)]
    async fn stops_when_the_result_changes_ignoring_timing_fields() {
        let mut checker = Scripted::new(vec![
            reading(
                json!({"rows": 0, "elapsed_seconds": 1}),
                JobActivity::Unknown,
                None,
            ),
            reading(
                json!({"rows": 0, "elapsed_seconds": 31}),
                JobActivity::Unknown,
                None,
            ),
            reading(
                json!({"rows": 5, "elapsed_seconds": 61}),
                JobActivity::Unknown,
                None,
            ),
        ]);
        let plan = plan(Until::Changed, 240, 30);
        let report = run_wait(&plan, &mut checker, UNIT, &CancellationToken::new()).await;
        assert_eq!(report.end, WaitEnd::Changed);
        assert_eq!(report.checks, 3);
    }

    #[tokio::test(start_paused = true)]
    async fn stops_on_a_pointer_value() {
        let mut checker = Scripted::new(vec![
            reading(
                json!({"job": {"status": "RUNNING"}}),
                JobActivity::Active,
                None,
            ),
            reading(
                json!({"job": {"status": "Done"}}),
                JobActivity::Finished,
                None,
            ),
        ]);
        let plan = plan(
            Until::Pointer {
                pointer: "/job/status".into(),
                values: vec![json!("done"), json!("failed")],
            },
            240,
            10,
        );
        let report = run_wait(&plan, &mut checker, UNIT, &CancellationToken::new()).await;
        assert_eq!(report.end, WaitEnd::Matched(json!("Done")));
        let script = report_json(&plan, &report, None).unwrap();
        let script: Value = serde_json::from_str(&script).unwrap();
        assert_eq!(script["state"], "matched");
        assert_eq!(script["last_result"]["job"]["status"], "Done");
    }

    #[tokio::test(start_paused = true)]
    async fn times_out_while_the_job_is_still_running() {
        let mut checker = Scripted::new(vec![reading(
            json!({"status": "running"}),
            JobActivity::Active,
            None,
        )]);
        let plan = plan(Until::Inactive, 100, 30);
        let report = run_wait(&plan, &mut checker, UNIT, &CancellationToken::new()).await;
        assert_eq!(report.end, WaitEnd::TimedOut);
        // Checks at 0, 30, 60 and 90, then a last one at the deadline.
        assert_eq!(report.checks, 5);
        assert_eq!(report.elapsed_secs, 100);
        let text = report_text(&plan, &report, None).unwrap();
        assert!(text.contains("still running"), "{text}");
        assert!(text.contains("call") || text.contains("Call"), "{text}");
    }

    #[tokio::test(start_paused = true)]
    async fn honours_a_longer_recommended_poll_interval() {
        let mut checker = Scripted::new(vec![
            reading(json!({"status": "running"}), JobActivity::Active, Some(60)),
            reading(json!({"status": "running"}), JobActivity::Active, Some(5)),
            reading(json!({"status": "done"}), JobActivity::Finished, None),
        ]);
        let plan = plan(Until::Inactive, 240, 30);
        let report = run_wait(&plan, &mut checker, UNIT, &CancellationToken::new()).await;
        assert_eq!(report.end, WaitEnd::Finished);
        let gaps: Vec<u64> = checker
            .at
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).as_secs())
            .collect();
        assert_eq!(
            gaps,
            vec![60, 30],
            "a shorter hint never goes below the minimum"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn stop_interrupts_the_wait_promptly() {
        let mut checker = Scripted::new(vec![reading(
            json!({"status": "running"}),
            JobActivity::Active,
            None,
        )]);
        let plan = plan(Until::Inactive, 240, 60);
        let token = CancellationToken::new();
        let stopper = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            stopper.cancel();
        });
        let started = tokio::time::Instant::now();
        let report = run_wait(&plan, &mut checker, UNIT, &token).await;
        assert_eq!(report.end, WaitEnd::Cancelled);
        assert_eq!(started.elapsed(), Duration::from_secs(5));
        assert!(report_text(&plan, &report, None).is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn a_result_with_no_job_state_stops_after_one_check() {
        let mut checker = Scripted::new(vec![reading(
            json!({"rows": 3}),
            JobActivity::Unknown,
            None,
        )]);
        let plan = plan(Until::Inactive, 240, 30);
        let report = run_wait(&plan, &mut checker, UNIT, &CancellationToken::new()).await;
        assert_eq!(report.end, WaitEnd::NotAJob);
        assert_eq!(report.checks, 1);
        assert!(report_text(&plan, &report, None)
            .unwrap()
            .contains("until: \"changed\""));
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_check_ends_the_wait_as_an_error() {
        let mut checker = Scripted::new(vec![CheckReading {
            value: Err("Tool error from cdw__status: not found".into()),
            activity: JobActivity::Unknown,
            hint_secs: None,
        }]);
        let plan = plan(Until::Inactive, 240, 30);
        let report = run_wait(&plan, &mut checker, UNIT, &CancellationToken::new()).await;
        let error = report_text(&plan, &report, None).unwrap_err();
        assert!(error.contains("check 1 of cdw__status failed"), "{error}");
        assert!(report_json(&plan, &report, None).is_err());
    }

    fn params(value: Value) -> WaitForParams {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn plans_apply_defaults_floors_and_caps() {
        let plan = WaitPlan::from_params(
            params(json!({"tool": "cdw__status", "arguments": {"job_id": "a"}})),
            MAX_TIMEOUT_SECS,
        )
        .unwrap();
        assert_eq!(plan.timeout, DEFAULT_TIMEOUT_SECS);
        assert_eq!(plan.min_interval, DEFAULT_MIN_INTERVAL_SECS);
        assert_eq!(plan.until, Until::Inactive);

        let plan = WaitPlan::from_params(
            params(json!({"tool": "cdw__status", "timeout_seconds": 9999,
                          "min_interval_seconds": "2"})),
            MAX_TIMEOUT_SECS,
        )
        .unwrap();
        assert_eq!(plan.timeout, MAX_TIMEOUT_SECS);
        assert_eq!(plan.min_interval, MIN_INTERVAL_FLOOR_SECS);

        let inside_a_script =
            WaitPlan::from_params(params(json!({"tool": "cdw__status"})), 90).unwrap();
        assert_eq!(inside_a_script.timeout, 90);
    }

    #[test]
    fn plans_refuse_what_they_cannot_wait_on() {
        for tool in [
            "status",
            "code_execution__execute_code",
            "code_execution__wait_for",
            "platform__manage_schedule",
        ] {
            assert!(
                WaitPlan::from_params(params(json!({"tool": tool})), MAX_TIMEOUT_SECS).is_err(),
                "{tool}"
            );
        }
        assert!(WaitPlan::from_params(
            params(json!({"tool": "a__b", "until": "eventually"})),
            MAX_TIMEOUT_SECS
        )
        .is_err());
        assert!(WaitPlan::from_params(
            params(json!({"tool": "a__b", "until": {"pointer": "status", "values": ["x"]}})),
            MAX_TIMEOUT_SECS
        )
        .is_err());
        let changed = WaitPlan::from_params(
            params(json!({"tool": "a__b", "until": "changed"})),
            MAX_TIMEOUT_SECS,
        )
        .unwrap();
        assert_eq!(changed.until, Until::Changed);
    }

    #[test]
    fn compact_state_drops_timing_then_nesting_then_cuts() {
        let short = json!({"status": "running", "elapsed_seconds": 40, "job_id": "a"}).to_string();
        // Compared as JSON: key order follows serde_json's build features.
        let compact: Value = serde_json::from_str(&compact_state(&short, 200)).unwrap();
        assert_eq!(compact, json!({"job_id": "a", "status": "running"}));
        let nested =
            json!({"status": "done", "rows": 3, "sample": {"a": "x".repeat(400)}}).to_string();
        let compact = compact_state(&nested, 100);
        assert!(compact.contains(r#""status":"done""#), "{compact}");
        assert!(compact.ends_with("(nested fields left out)"), "{compact}");
        let text = "line ".repeat(100);
        let compact = compact_state(&text, 50);
        assert!(compact.starts_with("line line"), "{compact}");
        assert!(compact.contains("characters in all"), "{compact}");
        assert!(compact.chars().count() < 100, "{compact}");
    }

    #[test]
    fn long_last_results_are_cut_for_the_model() {
        let plan = plan(Until::Inactive, 240, 30);
        let report = WaitReport {
            end: WaitEnd::Finished,
            checks: 1,
            elapsed_secs: 0,
            last: Some(format!("start{}end", "x".repeat(MAX_LAST_RESULT_BYTES * 2))),
            earlier_output: None,
        };
        let text = report_text(&plan, &report, None).unwrap();
        assert!(text.len() < MAX_LAST_RESULT_BYTES + 400);
        assert!(text.contains("Call the status tool directly"), "{text}");
        // The start and the end are both kept.
        assert!(
            text.contains("Last result of cdw__status:\nstart"),
            "{text}"
        );
        assert!(text.contains("xend\n(The result is"), "{text}");
        // A tool that hands its output out once cannot return it again.
        let shell = shown_result(
            "developer__shell_status",
            &"y".repeat(MAX_LAST_RESULT_BYTES * 2),
        );
        assert!(!shell.contains("Call the status tool directly"), "{shell}");
    }

    fn shell_check(status: &str, body: &str) -> CheckReading {
        CheckReading {
            value: Ok(format!(
                "job job-1 [make], status: {status} (5s elapsed)\nnew output since last check:\n{body}"
            )),
            activity: if status == "running" {
                JobActivity::Active
            } else {
                JobActivity::Finished
            },
            hint_secs: None,
        }
    }

    /// `shell_status` hands each piece of output out once, so a wait keeps
    /// what every check returned: the model sees what the job printed, not
    /// only the last check's share of it.
    #[tokio::test(start_paused = true)]
    async fn a_wait_keeps_the_output_each_check_drained() {
        let mut checker = Scripted::new(vec![
            shell_check("running", "compiling a"),
            shell_check("running", "compiling b"),
            shell_check("running", "(no new output)"),
            shell_check("exited 1", "error: b failed"),
        ]);
        let mut shell_plan = plan(Until::Inactive, 240, 30);
        shell_plan.tool = "developer__shell_status".into();
        let report = run_wait(&shell_plan, &mut checker, UNIT, &CancellationToken::new()).await;
        assert_eq!(report.end, WaitEnd::Finished);
        assert_eq!(
            report.earlier_output.as_deref(),
            Some("compiling a\ncompiling b")
        );
        let text = report_text(&shell_plan, &report, None).unwrap();
        let earlier = text.find("compiling a").expect("the first check's output");
        let last = text
            .find("error: b failed")
            .expect("the last check's output");
        assert!(earlier < last, "in the order the job printed it: {text}");
        assert!(text.contains("so this is the only copy"), "{text}");

        let script: Value =
            serde_json::from_str(&report_json(&shell_plan, &report, None).unwrap()).unwrap();
        assert_eq!(script["earlier_output"], "compiling a\ncompiling b");

        // A tool whose every result is whole keeps nothing extra.
        let mut checker = Scripted::new(vec![
            reading(
                json!({"status": "running", "log": "a"}),
                JobActivity::Active,
                None,
            ),
            reading(
                json!({"status": "done", "log": "a b"}),
                JobActivity::Finished,
                None,
            ),
        ]);
        let other = plan(Until::Inactive, 240, 30);
        let report = run_wait(&other, &mut checker, UNIT, &CancellationToken::new()).await;
        assert_eq!(report.earlier_output, None);
    }

    /// The kept output is bounded, and keeps its end.
    #[test]
    fn drained_output_keeps_the_end_within_its_bound() {
        let tool = "developer__shell_status";
        let check = |body: &str| {
            format!("job job-1 [make], status: running (5s elapsed)\nnew output since last check:\n{body}")
        };
        let mut output = DrainedOutput::default();
        for n in 0..2000 {
            output.record(tool, &check(&format!("line {n}")));
        }
        output.record(tool, &check("(no new output)"));
        let earlier = output.earlier().unwrap();
        assert!(
            earlier.len() < MAX_EARLIER_OUTPUT_BYTES + 100,
            "{}",
            earlier.len()
        );
        assert!(earlier.starts_with('['), "says what was dropped: {earlier}");
        assert!(
            earlier.contains("earlier bytes not kept]\nline "),
            "{earlier}"
        );
        assert!(earlier.ends_with("line 1999"), "{earlier}");
    }
}
