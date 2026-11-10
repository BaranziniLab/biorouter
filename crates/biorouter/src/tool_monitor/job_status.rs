//! Reading a tool result as the status of a long-running job.
//!
//! Shared by `code_execution__wait_for` (when to stop waiting, how long to
//! sleep between checks) and [`super::poll_pacing`] (how soon a repeated
//! status check may run). See `docs/agent-loop/waiting-on-jobs.md`.
//!
//! The vocabulary is deliberately wider than the repetition guard's
//! [`super::active_job_state`], which also requires a job id: a caller of
//! `wait_for` has already named the status tool, so the id is not what tells
//! a status apart from an ordinary result.

use crate::agents::operation_outcome::{OperationOutcome, OperationStatus};
use rmcp::model::{CallToolResult, JsonObject};
use serde_json::Value;

/// What one status result says about the job it describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JobActivity {
    /// Queued, running or otherwise not finished.
    Active,
    /// Completed, failed, cancelled or otherwise over.
    Finished,
    /// Nothing in the result names a job state.
    #[default]
    Unknown,
}

/// Fields that carry a job's state, in the shapes connectors use (cdwagent
/// reports `phase`).
const STATE_KEYS: &[&str] = &["status", "state", "phase", "job_status", "jobStatus"];

const ACTIVE_STATES: &[&str] = &[
    "pending",
    "queued",
    "running",
    "in_progress",
    "in-progress",
    "inprogress",
    "executing",
    "processing",
    "starting",
    "started",
    "submitted",
    "waiting",
    "scheduled",
    "streaming",
    "accepted",
];

const FINISHED_STATES: &[&str] = &[
    "completed",
    "complete",
    "succeeded",
    "success",
    "successful",
    "done",
    "finished",
    "failed",
    "failure",
    "error",
    "errored",
    "cancelled",
    "canceled",
    "timed_out",
    "timedout",
    "timeout",
    "expired",
    "aborted",
    "killed",
    "stopped",
    "rejected",
    "exited",
];

/// Fields that say how long to wait before the next check, in seconds.
const POLL_HINT_KEYS: &[&str] = &[
    "recommended_poll_seconds",
    "recommended_poll_interval_seconds",
    "recommendedPollSeconds",
    "poll_after_seconds",
    "poll_after",
    "pollAfterSeconds",
    "poll_interval_seconds",
    "poll_interval",
    "pollIntervalSeconds",
    "next_poll_seconds",
    "retry_after_seconds",
    "retry_after",
    "retryAfter",
];

/// The longest poll hint honoured, so a connector cannot park a check for hours.
pub const MAX_POLL_HINT_SECS: u64 = 3600;

/// Classify one state word.
pub fn state_word_activity(word: &str) -> JobActivity {
    let lowered = word.trim().to_ascii_lowercase();
    if ACTIVE_STATES.contains(&lowered.as_str()) {
        JobActivity::Active
    } else if FINISHED_STATES.contains(&lowered.as_str())
        || lowered.starts_with("exited")
        || lowered.starts_with("ended")
    {
        JobActivity::Finished
    } else {
        JobActivity::Unknown
    }
}

/// The state named directly on one object (not its children). Any active
/// state wins over a finished one at the same level: waiting longer costs no
/// model tokens, stopping early costs a turn.
fn object_activity(object: &serde_json::Map<String, Value>) -> JobActivity {
    let mut seen = JobActivity::Unknown;
    for key in STATE_KEYS {
        let Some(state) = object.get(*key).and_then(Value::as_str) else {
            continue;
        };
        match state_word_activity(state) {
            JobActivity::Active => return JobActivity::Active,
            JobActivity::Finished => seen = JobActivity::Finished,
            JobActivity::Unknown => {}
        }
    }
    seen
}

/// What a JSON value says about the job: its own state fields first, then
/// those of the objects directly inside it (`{"job": {"status": …}}`). An
/// array is active while any element is, and finished when every element that
/// names a state is finished.
pub fn activity_of_value(value: &Value) -> JobActivity {
    match value {
        Value::Object(object) => match object_activity(object) {
            JobActivity::Unknown => {
                let mut seen = JobActivity::Unknown;
                for child in object.values().filter_map(Value::as_object) {
                    match object_activity(child) {
                        JobActivity::Active => return JobActivity::Active,
                        JobActivity::Finished => seen = JobActivity::Finished,
                        JobActivity::Unknown => {}
                    }
                }
                seen
            }
            known => known,
        },
        Value::Array(items) => {
            let mut seen = JobActivity::Unknown;
            for item in items {
                match activity_of_value(item) {
                    JobActivity::Active => return JobActivity::Active,
                    JobActivity::Finished => seen = JobActivity::Finished,
                    JobActivity::Unknown => {}
                }
            }
            seen
        }
        _ => JobActivity::Unknown,
    }
}

/// The JSON values a result carries: its structured content, then every text
/// part that parses as JSON.
fn result_values(result: &CallToolResult) -> Vec<Value> {
    let mut values = Vec::new();
    if let Some(structured) = &result.structured_content {
        values.push(structured.clone());
    }
    for part in result.content.iter().filter_map(|part| part.as_text()) {
        if let Ok(value) = serde_json::from_str::<Value>(part.text.trim()) {
            if value.is_object() || value.is_array() {
                values.push(value);
            }
        }
    }
    values
}

/// The developer shell's `shell_status` header: `job <id> [<label>], status: <state> (…`.
fn shell_status_activity(result: &CallToolResult) -> JobActivity {
    for part in result.content.iter().filter_map(|part| part.as_text()) {
        let Some(line) = part.text.lines().next() else {
            continue;
        };
        if !line.starts_with("job ") {
            continue;
        }
        let Some((_, rest)) = line.split_once("], status: ") else {
            continue;
        };
        return if rest.starts_with("running (") {
            JobActivity::Active
        } else {
            JobActivity::Finished
        };
    }
    JobActivity::Unknown
}

/// What a whole tool result says about the job it reports on.
///
/// A failed call says nothing about the job (the lookup failed, not
/// necessarily the job), so it reads [`JobActivity::Unknown`].
pub fn activity_of_result(tool_name: &str, result: &CallToolResult) -> JobActivity {
    if result.is_error == Some(true) {
        return JobActivity::Unknown;
    }
    if let Some(operation) = OperationOutcome::from_call(result) {
        return match operation.status {
            OperationStatus::Queued | OperationStatus::Running => JobActivity::Active,
            OperationStatus::Completed | OperationStatus::Failed | OperationStatus::Cancelled => {
                JobActivity::Finished
            }
        };
    }
    let mut seen = JobActivity::Unknown;
    for value in result_values(result) {
        match activity_of_value(&value) {
            JobActivity::Active => return JobActivity::Active,
            JobActivity::Finished => seen = JobActivity::Finished,
            JobActivity::Unknown => {}
        }
    }
    if seen == JobActivity::Unknown && tool_name.rsplit("__").next() == Some("shell_status") {
        return shell_status_activity(result);
    }
    seen
}

fn hint_from_value(value: &Value) -> Option<u64> {
    let seconds = match value {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => text.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    if !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    Some((seconds.ceil() as u64).min(MAX_POLL_HINT_SECS))
}

fn object_hint(object: &serde_json::Map<String, Value>) -> Option<u64> {
    POLL_HINT_KEYS
        .iter()
        .find_map(|key| object.get(*key).and_then(hint_from_value))
}

/// How long the result asks the caller to wait before checking again, from a
/// `recommended_poll_seconds`, `retry_after` or `poll_after` field on the
/// result or on an object directly inside it. Zero and negative values are
/// ignored; values are capped at [`MAX_POLL_HINT_SECS`].
pub fn poll_hint_seconds(result: &CallToolResult) -> Option<u64> {
    result_values(result).iter().find_map(|value| match value {
        Value::Object(object) => object_hint(object).or_else(|| {
            object
                .values()
                .filter_map(Value::as_object)
                .find_map(object_hint)
        }),
        _ => None,
    })
}

/// Whether the result reports an IDENTIFIED active job: the repetition guard's
/// own reading (a job id with an active state, an `io.biorouter/operation`
/// envelope, or a running `shell_status` header). What the poll pacing keys
/// on, so it never slows a call whose result merely contains the word
/// "running".
pub fn identified_active_job(
    tool_name: &str,
    arguments: Option<&JsonObject>,
    result: &CallToolResult,
) -> bool {
    let call = rmcp::model::CallToolRequestParams {
        task: None,
        name: tool_name.to_string().into(),
        arguments: arguments.cloned(),
        meta: None,
    };
    super::active_job_result(&super::InternalToolCall::from_tool_call(&call), result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::Content;
    use serde_json::json;

    fn text(value: Value) -> CallToolResult {
        CallToolResult::success(vec![Content::text(value.to_string())])
    }

    #[test]
    fn cdwagent_phases_read_as_active_or_finished() {
        for phase in ["queued", "executing", "streaming"] {
            let result = text(json!({"job_id": "j1", "phase": phase}));
            assert_eq!(
                activity_of_result("cdw__status", &result),
                JobActivity::Active,
                "{phase}"
            );
        }
        for phase in ["completed", "cancelled", "timed_out", "failed"] {
            let result = text(json!({"job_id": "j1", "phase": phase}));
            assert_eq!(
                activity_of_result("cdw__status", &result),
                JobActivity::Finished,
                "{phase}"
            );
        }
    }

    #[test]
    fn nested_job_objects_and_structured_content_are_read() {
        let mut result = CallToolResult::success(vec![Content::text("ok")]);
        result.structured_content = Some(json!({"ok": true, "job": {"state": "RUNNING"}}));
        assert_eq!(activity_of_result("x__y", &result), JobActivity::Active);
        let plain = text(json!({"rows": 3}));
        assert_eq!(activity_of_result("x__y", &plain), JobActivity::Unknown);
    }

    #[test]
    fn a_failed_lookup_says_nothing_about_the_job() {
        let mut result = text(json!({"status": "running"}));
        result.is_error = Some(true);
        assert_eq!(activity_of_result("x__y", &result), JobActivity::Unknown);
    }

    #[test]
    fn the_operation_envelope_wins() {
        let mut result = text(json!({"status": "running"}));
        result.meta = Some(rmcp::model::Meta(
            json!({"io.biorouter/operation": {"version": 1, "id": "op", "status": "completed"}})
                .as_object()
                .unwrap()
                .clone(),
        ));
        assert_eq!(activity_of_result("x__y", &result), JobActivity::Finished);
    }

    #[test]
    fn shell_status_headers_are_read() {
        let running = CallToolResult::success(vec![Content::text(
            "job job-1 [build], status: running (12s elapsed)\nnew output since last check:\n",
        )]);
        assert_eq!(
            activity_of_result("developer__shell_status", &running),
            JobActivity::Active
        );
        let done = CallToolResult::success(vec![Content::text(
            "job job-1 [build], status: exited(0), success (40s elapsed)\n",
        )]);
        assert_eq!(
            activity_of_result("developer__shell_status", &done),
            JobActivity::Finished
        );
    }

    #[test]
    fn poll_hints_are_read_rounded_and_capped() {
        assert_eq!(
            poll_hint_seconds(&text(json!({"recommended_poll_seconds": 60}))),
            Some(60)
        );
        assert_eq!(
            poll_hint_seconds(&text(json!({"job": {"retry_after": "4.2"}}))),
            Some(5)
        );
        assert_eq!(
            poll_hint_seconds(&text(json!({"poll_after": 0}))),
            None,
            "zero is no hint"
        );
        assert_eq!(
            poll_hint_seconds(&text(json!({"retry_after": 999_999}))),
            Some(MAX_POLL_HINT_SECS)
        );
        assert_eq!(poll_hint_seconds(&text(json!({"rows": 1}))), None);
    }

    #[test]
    fn identified_active_jobs_need_an_id() {
        let with_id = text(json!({"job_id": "j1", "status": "running"}));
        assert!(identified_active_job("x__status", None, &with_id));
        let without = text(json!({"status": "running"}));
        assert!(!identified_active_job("x__status", None, &without));
    }
}
