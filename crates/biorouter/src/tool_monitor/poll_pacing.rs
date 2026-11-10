//! Spacing repeated status checks of a job that is still running.
//!
//! Every status check a model makes is a full model request that re-reads the
//! whole conversation. When the same call (same chat, same tool, same
//! arguments) last returned an identified active job
//! ([`super::job_status::identified_active_job`]) and is dispatched again
//! before its interval has passed, the dispatch is HELD until the interval has
//! passed, then runs. It is never refused. The interval is the larger of the
//! configured minimum ([`MIN_INTERVAL_CONFIG_KEY`], default
//! [`DEFAULT_MIN_INTERVAL_SECS`], `0` turns pacing off) and the job's own
//! `recommended_poll_seconds` / `retry_after` / `poll_after`. A hold is capped
//! at [`MAX_PACING_DELAY`] and ends at once on Stop.
//!
//! Called from `ExtensionManager::dispatch_tool_call`'s future, the one place
//! every tool call passes (the agent loop, `POST /agent/call_tool`, and every
//! call an `execute_code` script makes), around the existing call. It adds no
//! dispatch path and reads no privacy state. See
//! `docs/agent-loop/waiting-on-jobs.md`.

use super::job_status::{identified_active_job, poll_hint_seconds, MAX_POLL_HINT_SECS};
use rmcp::model::{CallToolResult, Content, JsonObject, Role};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::future::Future;
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::Duration;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// Config key for the minimum interval between repeated checks, in seconds.
pub const MIN_INTERVAL_CONFIG_KEY: &str = "BIOROUTER_STATUS_CHECK_MIN_INTERVAL_SECONDS";
/// The minimum interval when the key is unset.
pub const DEFAULT_MIN_INTERVAL_SECS: u64 = 30;
/// The longest a single check is held.
pub const MAX_PACING_DELAY: Duration = Duration::from_secs(120);
/// `_meta` key a quietly held check carries, read by Code Execution.
pub const PACED_META_KEY: &str = "biorouter/status-check-held";

/// Records older than this are dropped: a job checked an hour ago is not
/// being polled any more.
const RECORD_TTL: Duration = Duration::from_secs(3600);
const MAX_RECORDS: usize = 1024;

/// How a held check is reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacingMode {
    /// A call the model made directly: hold it and say so in its result.
    DelayAndNote,
    /// A call inside an `execute_code` script: hold it and record the hold in
    /// `_meta` ([`PACED_META_KEY`]); the script's own result reports it.
    DelayQuietly,
    /// A check `wait_for` makes: it spaces its own checks, so never hold it,
    /// but remember it so a direct check right after `wait_for` returns is
    /// spaced from it.
    RecordOnly,
}

tokio::task_local! {
    static MODE: PacingMode;
}

/// Run `future` (a dispatch's result future) with `mode`. Unscoped dispatches
/// use [`PacingMode::DelayAndNote`].
pub async fn with_mode<F: Future>(mode: PacingMode, future: F) -> F::Output {
    MODE.scope(mode, future).await
}

fn current_mode() -> PacingMode {
    MODE.try_with(|mode| *mode)
        .unwrap_or(PacingMode::DelayAndNote)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    session: String,
    tool: String,
    arguments: [u8; 32],
}

#[derive(Debug, Clone, Copy)]
struct Record {
    checked_at: Instant,
    interval: Duration,
}

static RECORDS: LazyLock<Mutex<HashMap<Key, Record>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn records() -> std::sync::MutexGuard<'static, HashMap<Key, Record>> {
    RECORDS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What [`before_dispatch`] did, handed to [`after_dispatch`].
#[derive(Debug)]
pub struct Paced {
    key: Option<Key>,
    mode: PacingMode,
    held: Duration,
    interval: Duration,
}

/// Code Execution's own tools are never paced: `wait_for` spaces its checks
/// itself, and the calls a script makes are paced one by one.
fn exempt(tool: &str) -> bool {
    tool.starts_with("code_execution__")
        || crate::agents::code_execution_extension::is_execute_code_call(tool)
}

fn key_for(session_id: &str, tool: &str, arguments: Option<&JsonObject>) -> Key {
    let canonical = arguments
        .map(|object| super::canonicalize_json(&serde_json::Value::Object(object.clone())))
        .unwrap_or(serde_json::Value::Null);
    Key {
        session: session_id.to_string(),
        tool: tool.to_string(),
        arguments: Sha256::digest(serde_json::to_vec(&canonical).unwrap_or_default()).into(),
    }
}

/// How long a check must wait, given when the same check last ran.
fn hold_for(record: Option<Record>, now: Instant) -> Duration {
    let Some(record) = record else {
        return Duration::ZERO;
    };
    let since = now.saturating_duration_since(record.checked_at);
    record.interval.saturating_sub(since).min(MAX_PACING_DELAY)
}

/// Hold a repeated check of an active job until its interval has passed.
pub async fn before_dispatch(
    session_id: &str,
    tool: &str,
    arguments: Option<&JsonObject>,
    cancellation_token: &CancellationToken,
) -> Paced {
    let mode = current_mode();
    if exempt(tool) {
        return Paced {
            key: None,
            mode,
            held: Duration::ZERO,
            interval: Duration::ZERO,
        };
    }
    let key = key_for(session_id, tool, arguments);
    let record = records().get(&key).copied();
    let interval = record.map_or(Duration::ZERO, |record| record.interval);
    let hold = if mode == PacingMode::RecordOnly {
        Duration::ZERO
    } else {
        hold_for(record, Instant::now())
    };
    let mut held = Duration::ZERO;
    if !hold.is_zero() {
        let started = Instant::now();
        tokio::select! {
            () = tokio::time::sleep(hold) => {}
            () = cancellation_token.cancelled() => {}
        }
        held = started.elapsed();
    }
    Paced {
        key: Some(key),
        mode,
        held,
        interval,
    }
}

/// One "second" of interval. This crate's unit tests use 10 ms, so a test
/// that dispatches the same status call twice is not held for half a minute.
fn pacing_unit() -> Duration {
    if cfg!(test) {
        Duration::from_millis(10)
    } else {
        Duration::from_secs(1)
    }
}

/// The configured minimum interval, `0` meaning pacing is off.
fn configured_min_interval() -> u64 {
    crate::config::Config::global()
        .get_param::<u64>(MIN_INTERVAL_CONFIG_KEY)
        .unwrap_or(DEFAULT_MIN_INTERVAL_SECS)
}

/// The interval for a job: the configured minimum, or the job's own hint when
/// that is longer. `None` when pacing is off.
pub fn interval_secs(configured: u64, hint: Option<u64>) -> Option<u64> {
    if configured == 0 {
        return None;
    }
    Some(configured.max(hint.unwrap_or(0)).min(MAX_POLL_HINT_SECS))
}

fn seconds_up(duration: Duration) -> u64 {
    let millis = duration.as_millis();
    u64::try_from(millis.div_ceil(1000)).unwrap_or(u64::MAX)
}

/// The note a held direct check carries.
pub fn held_note(held: Duration, interval: Duration) -> String {
    format!(
        "Biorouter held this status check for {} s, because the same check of this job ran \
         less than {} s ago. Every check you make re-reads the whole conversation, so wait \
         for a job with code_execution__wait_for, or with sleep() and a status call inside \
         one execute_code script, instead of checking again.",
        seconds_up(held),
        seconds_up(interval)
    )
}

/// Remember this check when its result is an identified active job, forget it
/// otherwise, and report a hold.
pub fn after_dispatch(
    paced: Paced,
    tool: &str,
    arguments: Option<&JsonObject>,
    result: &mut CallToolResult,
) {
    let Some(key) = paced.key else {
        return;
    };
    if identified_active_job(tool, arguments, result) {
        match interval_secs(configured_min_interval(), poll_hint_seconds(result)) {
            Some(interval) => remember(
                key,
                Record {
                    checked_at: Instant::now(),
                    interval: pacing_unit() * u32::try_from(interval).unwrap_or(u32::MAX),
                },
            ),
            None => {
                records().remove(&key);
            }
        }
    } else {
        records().remove(&key);
    }
    if paced.held.is_zero() {
        return;
    }
    match paced.mode {
        PacingMode::DelayAndNote => result.content.push(
            Content::text(held_note(paced.held, paced.interval))
                .with_audience(vec![Role::Assistant]),
        ),
        PacingMode::DelayQuietly => {
            let meta = result
                .meta
                .get_or_insert_with(|| rmcp::model::Meta(JsonObject::new()));
            meta.0.insert(
                PACED_META_KEY.to_string(),
                serde_json::json!({
                    "held_seconds": seconds_up(paced.held),
                    "interval_seconds": seconds_up(paced.interval),
                }),
            );
        }
        PacingMode::RecordOnly => {}
    }
}

fn remember(key: Key, record: Record) {
    let mut records = records();
    if records.len() >= MAX_RECORDS {
        let now = Instant::now();
        records.retain(|_, kept| now.saturating_duration_since(kept.checked_at) < RECORD_TTL);
        if records.len() >= MAX_RECORDS {
            if let Some(oldest) = records
                .iter()
                .min_by_key(|(_, kept)| kept.checked_at)
                .map(|(key, _)| key.clone())
            {
                records.remove(&oldest);
            }
        }
    }
    records.insert(key, record);
}

/// Seconds a quietly held check was held, from its `_meta`.
pub fn held_seconds_in(result: &CallToolResult) -> Option<u64> {
    result
        .meta
        .as_ref()?
        .get(PACED_META_KEY)?
        .get("held_seconds")?
        .as_u64()
}

#[cfg(test)]
pub(crate) fn remember_for_test(
    session_id: &str,
    tool: &str,
    arguments: Option<&JsonObject>,
    interval: Duration,
) {
    remember(
        key_for(session_id, tool, arguments),
        Record {
            checked_at: Instant::now(),
            interval,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(job: &str) -> JsonObject {
        json!({"job_id": job}).as_object().unwrap().clone()
    }

    fn active(job: &str) -> CallToolResult {
        CallToolResult::success(vec![Content::text(
            json!({"job_id": job, "status": "running"}).to_string(),
        )])
    }

    #[tokio::test(start_paused = true)]
    async fn a_too_early_repeat_is_held_until_its_interval_and_says_so() {
        let session = "pace-held";
        let tool = "cdw__status";
        remember_for_test(session, tool, Some(&args("a")), Duration::from_secs(30));
        tokio::time::advance(Duration::from_secs(10)).await;

        let started = Instant::now();
        let paced =
            before_dispatch(session, tool, Some(&args("a")), &CancellationToken::new()).await;
        assert_eq!(started.elapsed(), Duration::from_secs(20));

        let mut result = active("a");
        after_dispatch(paced, tool, Some(&args("a")), &mut result);
        let note = result
            .content
            .last()
            .unwrap()
            .as_text()
            .unwrap()
            .text
            .clone();
        assert!(note.contains("held this status check for 20 s"), "{note}");
        assert!(note.contains("code_execution__wait_for"), "{note}");
    }

    #[tokio::test(start_paused = true)]
    async fn different_arguments_and_other_chats_are_left_alone() {
        let tool = "cdw__status";
        remember_for_test(
            "pace-other",
            tool,
            Some(&args("a")),
            Duration::from_secs(30),
        );
        let started = Instant::now();
        let paced = before_dispatch(
            "pace-other",
            tool,
            Some(&args("b")),
            &CancellationToken::new(),
        )
        .await;
        assert!(paced.held.is_zero());
        let paced = before_dispatch(
            "pace-else",
            tool,
            Some(&args("a")),
            &CancellationToken::new(),
        )
        .await;
        assert!(paced.held.is_zero());
        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn a_hold_ends_on_stop() {
        let session = "pace-stop";
        let tool = "cdw__status";
        remember_for_test(session, tool, Some(&args("a")), Duration::from_secs(100));
        let token = CancellationToken::new();
        let stopper = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            stopper.cancel();
        });
        let started = Instant::now();
        let _ = before_dispatch(session, tool, Some(&args("a")), &token).await;
        assert_eq!(started.elapsed(), Duration::from_secs(2));
    }

    #[tokio::test(start_paused = true)]
    async fn wait_for_checks_are_recorded_but_never_held() {
        let session = "pace-record-only";
        let tool = "cdw__status";
        remember_for_test(session, tool, Some(&args("a")), Duration::from_secs(60));
        let started = Instant::now();
        let paced = with_mode(
            PacingMode::RecordOnly,
            before_dispatch(session, tool, Some(&args("a")), &CancellationToken::new()),
        )
        .await;
        assert_eq!(started.elapsed(), Duration::ZERO);
        assert_eq!(paced.mode, PacingMode::RecordOnly);
    }

    #[tokio::test(start_paused = true)]
    async fn a_script_call_is_held_quietly_in_meta() {
        let session = "pace-quiet";
        let tool = "cdw__status";
        remember_for_test(session, tool, Some(&args("a")), Duration::from_secs(30));
        let paced = with_mode(
            PacingMode::DelayQuietly,
            before_dispatch(session, tool, Some(&args("a")), &CancellationToken::new()),
        )
        .await;
        let mut result = active("a");
        after_dispatch(paced, tool, Some(&args("a")), &mut result);
        assert_eq!(result.content.len(), 1, "no note in the script's value");
        assert_eq!(held_seconds_in(&result), Some(30));
    }

    #[tokio::test(start_paused = true)]
    async fn a_finished_job_is_forgotten() {
        let session = "pace-finished";
        let tool = "cdw__status";
        remember_for_test(session, tool, Some(&args("a")), Duration::from_secs(30));
        let paced = with_mode(
            PacingMode::RecordOnly,
            before_dispatch(session, tool, Some(&args("a")), &CancellationToken::new()),
        )
        .await;
        let mut done = CallToolResult::success(vec![Content::text(
            json!({"job_id": "a", "status": "completed"}).to_string(),
        )]);
        after_dispatch(paced, tool, Some(&args("a")), &mut done);
        let started = Instant::now();
        let paced =
            before_dispatch(session, tool, Some(&args("a")), &CancellationToken::new()).await;
        assert!(paced.held.is_zero());
        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    #[test]
    fn code_execution_tools_are_exempt() {
        assert!(exempt("code_execution__wait_for"));
        assert!(exempt("code_execution__execute_code"));
        assert!(exempt("execute_code"));
        assert!(!exempt("cdw__status"));
    }

    #[test]
    fn the_interval_is_the_longer_of_the_minimum_and_the_hint() {
        assert_eq!(interval_secs(30, Some(5)), Some(30));
        assert_eq!(interval_secs(30, Some(60)), Some(60));
        assert_eq!(interval_secs(30, None), Some(30));
        assert_eq!(interval_secs(0, Some(60)), None, "0 turns pacing off");
        assert_eq!(
            interval_secs(30, Some(MAX_POLL_HINT_SECS * 2)),
            Some(MAX_POLL_HINT_SECS)
        );
    }

    #[test]
    fn a_hold_is_capped() {
        let now = Instant::now();
        let record = Record {
            checked_at: now,
            interval: Duration::from_secs(600),
        };
        assert_eq!(hold_for(Some(record), now), MAX_PACING_DELAY);
        assert_eq!(hold_for(None, now), Duration::ZERO);
    }
}
