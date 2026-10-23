use crate::agents::tool_errors::ToolErrorKind;
use crate::config::BioRouterMode;
use crate::conversation::message::{Message, ToolRequest};
use crate::mcp_utils::ToolResult;
use crate::tool_inspection::{InspectionAction, InspectionResult, ToolInspector};
use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::{CallToolRequestParams, CallToolResult, Role};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Inspector name used by the repetition guard; the agent keys the honest
/// "repetition, not user decline" deny message off this.
pub const REPETITION_INSPECTOR_NAME: &str = "repetition";

/// Finding id for the hard stop (the call is denied).
pub const REPETITION_HARD_FINDING_ID: &str = "REP-001";

/// Finding id for the non-blocking soft warning (the call still runs, but the
/// model is told it is repeating itself) — BR-29.
pub const REPETITION_SOFT_FINDING_ID: &str = "REP-002";

/// BR-30 near-duplicate ("arg-tweak") repetition, soft stage.
pub const REPETITION_NEAR_DUP_SOFT_FINDING_ID: &str = "REP-003";
/// BR-30 near-duplicate ("arg-tweak") repetition, hard stage.
pub const REPETITION_NEAR_DUP_HARD_FINDING_ID: &str = "REP-004";
/// BR-30 A/B/A/B oscillation, soft stage.
pub const REPETITION_OSCILLATION_SOFT_FINDING_ID: &str = "REP-005";
/// BR-30 A/B/A/B oscillation, hard stage.
pub const REPETITION_OSCILLATION_HARD_FINDING_ID: &str = "REP-006";
/// BR-31 repeated-failing-result guard, hard stage: the next call to a tool that
/// has already failed the same way `hard_stop_at` times in a row is denied.
pub const FAILURE_LOOP_HARD_FINDING_ID: &str = "REP-007";

/// Nth consecutive *near*-identical call to the same tool that earns a
/// non-blocking warning.
pub const DEFAULT_NEAR_DUP_SOFT_WARN: u32 = 4;
/// Nth call of an `A/B/A/B` alternation that earns a non-blocking warning
/// (4 = two full cycles, the same trigger OpenHands' `StuckDetector` uses).
pub const DEFAULT_OSCILLATION_SOFT_WARN: u32 = 4;
/// Persistent near-duplicate or alternating calls without a changed result stop
/// after the model has had another full warning window to recover.
pub const DEFAULT_SEMANTIC_HARD_STOP: u32 = 8;
/// How similar two normalized argument sets must be to count as "the same call
/// with a tweak". Deliberately high: a different filename or a different numeric
/// range scores far below this, so ordinary iteration over distinct inputs does
/// not trip the detector.
pub const DEFAULT_ARG_SIMILARITY_THRESHOLD: f32 = 0.9;

/// BR-31: Nth consecutive failure of the same tool with the same error that earns
/// a first, hedged nudge.
pub const DEFAULT_FAILURE_SOFT_WARN: u32 = 3;
/// BR-31: Nth consecutive identical failure that escalates the nudge to
/// "you are not making progress; change approach or ask the user".
pub const DEFAULT_FAILURE_ESCALATE_WARN: u32 = 5;
/// BR-31: Nth consecutive identical failure after which the *next* call to that
/// tool is denied outright.
pub const DEFAULT_FAILURE_HARD_STOP: u32 = 6;
/// BR-31: how similar two normalized error signatures must be to count as "the
/// same failure". Lower than the argument threshold because two runs of one
/// failing command produce near-identical prose, and because the signature is
/// already normalized (case, whitespace, digit runs).
pub const DEFAULT_ERROR_SIMILARITY_THRESHOLD: f32 = 0.85;

/// How many recent tool calls the semantic heuristics look back over.
const LOOP_WINDOW: usize = 20;

/// Longest error prefix kept in a signature. Tool errors can carry a whole
/// stack trace or a megabyte of stderr; the headline is what identifies the
/// failure, and comparing the tail is both slow and noisy.
const MAX_SIGNATURE_LEN: usize = 400;

/// Argument keys whose values are volatile plumbing (request ids, timestamps,
/// nonces). They differ on every call even when the call is *semantically* the
/// same, so byte-exact comparison misses the repeat. They are dropped before
/// comparison. Note `id` itself is **not** on this list: for many tools `id` is
/// the thing being fetched, and ignoring it would make legitimate iteration over
/// distinct ids look like a loop.
const VOLATILE_ARG_KEYS: &[&str] = &[
    "request_id",
    "requestid",
    "trace_id",
    "traceid",
    "correlation_id",
    "idempotency_key",
    "session_id",
    "sessionid",
    "nonce",
    "timestamp",
    "created_at",
    "updated_at",
    "_meta",
];

/// BR-30: configuration for the semantic (beyond byte-exact) loop heuristics.
///
/// Both heuristics warn before stopping persistent repetition. Completed calls
/// with changed results clear their window, so long tasks are judged by progress
/// rather than their total number of actions. Each stage can be disabled.
#[derive(Debug, Clone)]
pub struct SemanticLoopConfig {
    /// Master switch for both heuristics.
    pub enabled: bool,
    /// Normalized-argument similarity at or above which two calls to the same
    /// tool count as near-duplicates. `0.0..=1.0`.
    pub similarity_threshold: f32,
    /// Nth consecutive near-duplicate call that warns. `None` disables.
    pub near_dup_soft_warn: Option<u32>,
    /// Nth consecutive near-duplicate call that is denied. `None` disables.
    pub near_dup_hard_stop: Option<u32>,
    /// Nth call of an alternating `A/B/A/B` run that warns. `None` disables.
    pub oscillation_soft_warn: Option<u32>,
    /// Nth call of an alternating `A/B/A/B` run that is denied. `None` disables.
    pub oscillation_hard_stop: Option<u32>,
}

impl Default for SemanticLoopConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            similarity_threshold: DEFAULT_ARG_SIMILARITY_THRESHOLD,
            near_dup_soft_warn: Some(DEFAULT_NEAR_DUP_SOFT_WARN),
            near_dup_hard_stop: Some(DEFAULT_SEMANTIC_HARD_STOP),
            oscillation_soft_warn: Some(DEFAULT_OSCILLATION_SOFT_WARN),
            oscillation_hard_stop: Some(DEFAULT_SEMANTIC_HARD_STOP),
        }
    }
}

impl SemanticLoopConfig {
    /// Byte-exact detection only (the pre-BR-30 behavior).
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
}

/// BR-31: configuration for the repeated-failing-result ("no progress") detector.
///
/// The three stages are expressed as "the Nth consecutive failure of this tool
/// with the same error":
///
/// * `soft_warn_at` — a hedged nudge is injected right after the failing result.
/// * `escalate_at` — the nudge becomes an explicit "you are not making progress;
///   change approach or ask the user".
/// * `hard_stop_at` — the *next* call to that tool is denied (the failure already
///   happened; only a future call can be blocked).
///
/// Unlike BR-30's heuristics, the hard stage is **on by default**: a run of
/// identical failures is evidence, not a guess — the tool ran, and it produced
/// the same error every time. The stop is also self-clearing: the denial is
/// itself an error with a different signature, so it breaks the streak and the
/// model gets to try again (and will be stopped again if it resumes the loop).
#[derive(Debug, Clone)]
pub struct FailureLoopConfig {
    /// Master switch.
    pub enabled: bool,
    /// Normalized-error-signature similarity at or above which two failures of
    /// the same tool count as "the same failure". `0.0..=1.0`.
    pub similarity_threshold: f32,
    /// Nth consecutive identical failure that earns a nudge. `None` disables.
    pub soft_warn_at: Option<u32>,
    /// Nth consecutive identical failure that escalates the nudge. `None` disables.
    pub escalate_at: Option<u32>,
    /// Nth consecutive identical failure after which the next call to the tool is
    /// denied. `None` disables the hard stage (nudge-only).
    pub hard_stop_at: Option<u32>,
    /// Stop persistent retryable failures after the same recovery warnings.
    /// A successful changed result clears the streak before a later retry;
    /// `false` keeps retryable failures advisory-only.
    pub deny_retryable: bool,
}

impl Default for FailureLoopConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            similarity_threshold: DEFAULT_ERROR_SIMILARITY_THRESHOLD,
            soft_warn_at: Some(DEFAULT_FAILURE_SOFT_WARN),
            escalate_at: Some(DEFAULT_FAILURE_ESCALATE_WARN),
            hard_stop_at: Some(DEFAULT_FAILURE_HARD_STOP),
            deny_retryable: true,
        }
    }
}

impl FailureLoopConfig {
    /// No result-aware detection at all (the pre-BR-31 behavior).
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
}

/// BR-31: one completed tool call, reduced to what the no-progress detector
/// needs — which tool ran, and, if it failed, a normalized signature of its error.
///
/// BR-51 adds the failure's *class*: the detectors used to treat "the file does
/// not exist" and "the network blipped" as the same shapeless failure, and only
/// one of those is worth trying again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutcome {
    pub tool_name: String,
    /// `None` when the call succeeded; `Some(signature)` when it failed.
    pub failure: Option<String>,
    /// BR-51: the taxonomy class of the failure, `None` on success (or when the
    /// taxonomy is disabled and the outcome was built without one).
    pub kind: Option<ToolErrorKind>,
}

impl ToolOutcome {
    /// Could this exact call plausibly succeed if it were issued again? `false`
    /// for a success (there is nothing to retry) and for any unclassified failure
    /// — the conservative answer, matching the pre-BR-51 assumption.
    pub fn retryable(&self) -> bool {
        self.failure.is_some() && self.kind.is_some_and(ToolErrorKind::retryable)
    }
}

/// Normalize an error message into a comparison signature: lowercased,
/// whitespace-collapsed, digit runs folded to `#`, truncated to the headline.
///
/// Folding digits is what makes two runs of the *same* failure compare equal
/// when the error embeds a pid, a timestamp, a byte offset, or a line number.
fn error_signature(text: &str) -> String {
    let collapsed = collapse_whitespace(text).to_lowercase();
    let mut signature = String::with_capacity(collapsed.len().min(MAX_SIGNATURE_LEN));
    let mut in_digits = false;
    for ch in collapsed.chars() {
        if signature.chars().count() >= MAX_SIGNATURE_LEN {
            break;
        }
        if ch.is_ascii_digit() {
            if !in_digits {
                signature.push('#');
                in_digits = true;
            }
        } else {
            in_digits = false;
            signature.push(ch);
        }
    }
    signature
}

/// The error text of a failed tool result, or `None` when it succeeded. A tool
/// can fail two ways: a transport/protocol `Err`, or an `Ok` result flagged
/// `is_error` (the MCP "the tool ran and reported a failure" path) — both count.
fn failure_text(result: &ToolResult<CallToolResult>) -> Option<String> {
    match result {
        Err(error) => Some(error.message.to_string()),
        Ok(call) if call.is_error == Some(true) => {
            let text = call
                .content
                .iter()
                .filter_map(|content| content.as_text().map(|text| text.text.as_str()))
                .collect::<Vec<_>>()
                .join(" ");
            Some(if text.trim().is_empty() {
                "tool reported an error with no message".to_string()
            } else {
                text
            })
        }
        Ok(_) => None,
    }
}

/// Reduce a completed tool result to a [`ToolOutcome`].
///
/// The BR-51 classification is derived from the same result, so it is available
/// whether the taxonomy annotated the text or not (and, when it did, is read back
/// off the header rather than re-guessed).
pub fn tool_outcome(tool_name: &str, result: &ToolResult<CallToolResult>) -> ToolOutcome {
    let failure = failure_text(result)
        .as_deref()
        .map(error_signature)
        .filter(|signature| !signature.is_empty());
    ToolOutcome {
        tool_name: tool_name.to_string(),
        kind: failure
            .is_some()
            .then(|| crate::agents::tool_errors::classify(result).map(|error| error.kind))
            .flatten(),
        failure,
    }
}

/// The outcome of `request_id` as recorded in a tool-response message the agent
/// has just written (the BR-31 result-collection seam), or `None` when that
/// message carries no response for it.
pub fn outcome_from_response_message(
    tool_name: &str,
    request_id: &str,
    message: &Message,
) -> Option<ToolOutcome> {
    message
        .content
        .iter()
        .filter_map(|content| content.as_tool_response())
        .find(|response| response.id == request_id)
        .map(|response| tool_outcome(tool_name, &response.tool_result))
}

/// Every completed tool call since the last genuine user turn, in order, paired
/// with whether it failed. Requests and responses are matched by id, so a batch
/// of parallel calls is attributed correctly.
pub fn tool_outcomes_since_last_user_turn(messages: &[Message]) -> Vec<ToolOutcome> {
    let start = last_user_turn_index(messages);
    let mut names: HashMap<&str, String> = HashMap::new();
    let mut outcomes = Vec::new();

    for message in &messages[start..] {
        for content in &message.content {
            if let Some(request) = content.as_tool_request() {
                if let Ok(tool_call) = &request.tool_call {
                    names.insert(request.id.as_str(), tool_call.name.to_string());
                }
            } else if let Some(response) = content.as_tool_response() {
                let Some(tool_name) = names.get(response.id.as_str()) else {
                    continue;
                };
                outcomes.push(tool_outcome(tool_name, &response.tool_result));
            }
        }
    }

    if outcomes.len() > LOOP_WINDOW {
        outcomes.drain(..outcomes.len() - LOOP_WINDOW);
    }
    outcomes
}

/// Length of the trailing run of failures of `tool_name` whose error signatures
/// all match the most recent one. `0` when that tool's last call succeeded, or
/// when it has not run since the last user turn.
///
/// Only that tool's own outcomes are considered, so interleaved work with other
/// tools neither resets nor hides the streak. A *success* of the tool ends the
/// run (progress), and so does a failure with a materially different error — a
/// new error is new information, which is the opposite of being stuck.
pub fn failing_streak(outcomes: &[ToolOutcome], tool_name: &str, threshold: f32) -> u32 {
    let mut own = outcomes
        .iter()
        .rev()
        .filter(|outcome| outcome.tool_name == tool_name);

    let Some(signature) = own.next().and_then(|outcome| outcome.failure.as_deref()) else {
        return 0;
    };

    let mut streak = 1u32;
    for outcome in own {
        match outcome.failure.as_deref() {
            Some(previous) if signatures_match(previous, signature, threshold) => streak += 1,
            _ => break,
        }
    }
    streak
}

fn signatures_match(a: &str, b: &str, threshold: f32) -> bool {
    a == b || string_similarity(a, b) >= threshold
}

/// BR-51: the taxonomy class of `tool_name`'s most recent failure, or `None` if
/// its last call succeeded, it has not run, or the failure was never classified.
pub fn latest_failure_kind(outcomes: &[ToolOutcome], tool_name: &str) -> Option<ToolErrorKind> {
    outcomes
        .iter()
        .rev()
        .find(|outcome| outcome.tool_name == tool_name)
        .filter(|outcome| outcome.failure.is_some())
        .and_then(|outcome| outcome.kind)
}

/// Is `tool_name`'s current failing run made of failures a retry could survive?
fn streak_is_retryable(outcomes: &[ToolOutcome], tool_name: &str) -> bool {
    latest_failure_kind(outcomes, tool_name).is_some_and(ToolErrorKind::retryable)
}

/// BR-31: the nudge (if any) owed to `tool_name` after its latest result, given
/// every outcome since the last user turn (including the batch that just ran).
///
/// Returns `None` below the soft threshold, the hedged nudge between soft and
/// escalate, and the escalated nudge at or above it. The hard stop is *not*
/// expressed here — a failure that already happened cannot be un-run; it is
/// enforced on the next call by [`RepetitionInspector`].
pub fn failure_loop_nudge(
    config: &FailureLoopConfig,
    outcomes: &[ToolOutcome],
    tool_name: &str,
) -> Option<String> {
    if !config.enabled {
        return None;
    }
    let streak = failing_streak(outcomes, tool_name, config.similarity_threshold);
    let fired = |threshold: Option<u32>| threshold.is_some_and(|at| at > 0 && streak >= at);
    let kind = latest_failure_kind(outcomes, tool_name);

    if fired(config.escalate_at) {
        Some(escalated_failure_nudge(
            tool_name,
            streak,
            // A retryable streak is never hard-stopped unless the operator opted
            // in, so do not threaten the model with a block that will not come.
            config
                .hard_stop_at
                .filter(|_| config.deny_retryable || !kind.is_some_and(ToolErrorKind::retryable)),
            kind,
        ))
    } else if fired(config.soft_warn_at) {
        Some(soft_failure_nudge(tool_name, streak, kind))
    } else {
        None
    }
}

/// BR-51: what the nudge can honestly say about the *class* of failure being
/// repeated. "Running it again will produce the same error again" is true of a
/// missing file and false of a flaky network — telling the model the wrong one
/// is worse than telling it nothing.
fn failure_diagnosis(kind: Option<ToolErrorKind>) -> String {
    match kind {
        Some(kind) if kind.retryable() => format!(
            " The failures are classified '{}', i.e. retryable, so the tool itself may recover, \
             but blind repetition is not a plan: back off, change the request, or take a \
             different route.",
            kind.as_str()
        ),
        Some(kind) => format!(
            " The failures are classified '{}', which is not retryable: {}",
            kind.as_str(),
            kind.guidance()
        ),
        None => String::new(),
    }
}

fn soft_failure_nudge(tool_name: &str, streak: u32, kind: Option<ToolErrorKind>) -> String {
    let repetition = if kind.is_some_and(ToolErrorKind::retryable) {
        "Running it again may well produce the same error again."
    } else {
        "Running it again will produce the same error again."
    };
    let diagnosis = failure_diagnosis(kind);
    format!(
        "No progress: '{tool_name}' has now failed {streak} times in a row with the \
         same error. {repetition} Read what the \
         error actually says and fix its cause, or take a different route: a different \
         tool, a different input, or stopping to tell the user what is blocking you.{diagnosis}"
    )
}

fn escalated_failure_nudge(
    tool_name: &str,
    streak: u32,
    hard_stop_at: Option<u32>,
    kind: Option<ToolErrorKind>,
) -> String {
    let stop_clause = match hard_stop_at {
        Some(at) if at > streak => format!(
            " Further calls to '{tool_name}' will be blocked automatically once it has \
             failed the same way {at} times."
        ),
        Some(_) => format!(
            " The next call to '{tool_name}' will be blocked automatically unless \
             something changes."
        ),
        None => String::new(),
    };
    let diagnosis = failure_diagnosis(kind);
    format!(
        "You are not making progress: '{tool_name}' has failed {streak} times in a row \
         with the same error, and the earlier warning changed nothing. Stop retrying it. \
         Do one of these instead: fix the underlying cause the error names, use a \
         different tool or approach, or stop and ask the user for what you are \
         missing.{stop_clause}{diagnosis}"
    )
}

/// The message the model sees in place of the tool result when the no-progress
/// guard denies a call. Like BR-29's, it states the real reason rather than
/// claiming the user declined.
fn failure_stop_reason(tool_name: &str, streak: u32) -> String {
    format!(
        "Biorouter did not run this tool call: '{tool_name}' has already failed \
         {streak} times in a row with the same error, and the warnings changed nothing. \
         The user did NOT decline it; this is an automatic no-progress guard. Repeating \
         a call that keeps failing the same way cannot make progress. Fix the cause the \
         error names, use a different tool or approach, or stop and tell the user exactly \
         what is blocking you. (If you call '{tool_name}' again and it keeps failing the \
         same way, it will be stopped again.)"
    )
}

// Helper struct for internal tracking
#[derive(Debug, Clone)]
struct InternalToolCall {
    name: String,
    parameters: Value,
    /// `parameters` with volatile keys dropped and strings whitespace-collapsed.
    /// Two calls that differ only in plumbing compare equal here.
    normalized: Value,
}

impl InternalToolCall {
    /// Exact match: same tool and JSON-equivalent arguments (BR-29).
    fn matches(&self, other: &InternalToolCall) -> bool {
        self.name == other.name && self.parameters == other.parameters
    }

    /// Same tool and same *normalized* arguments — identical modulo volatile
    /// plumbing and whitespace. Used as the oscillation signature.
    fn same_shape(&self, other: &InternalToolCall) -> bool {
        self.name == other.name && self.normalized == other.normalized
    }

    /// Same tool, and normalized arguments at least `threshold` similar: the
    /// "same call with an argument tweak" relation.
    fn near_matches(&self, other: &InternalToolCall, threshold: f32) -> bool {
        self.name == other.name && arg_similarity(&self.normalized, &other.normalized) >= threshold
    }

    /// A stable key for "this exact call". Two calls with the same name and the
    /// same arguments have the same signature within a user turn.
    fn signature(&self) -> String {
        format!("{}\n{}", self.name, self.parameters)
    }

    fn from_tool_call(tool_call: &CallToolRequestParams) -> Self {
        let name = tool_call.name.to_string();
        let raw_parameters = tool_call
            .arguments
            .as_ref()
            .map(|obj| Value::Object(obj.clone()))
            .unwrap_or(Value::Null);
        let parameters = canonicalize_json(&raw_parameters);
        let normalized = normalize_args(&parameters);
        Self {
            name,
            parameters,
            normalized,
        }
    }

    fn from_request(tool_request: &ToolRequest) -> Option<Self> {
        tool_request
            .tool_call
            .as_ref()
            .ok()
            .map(Self::from_tool_call)
    }
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));

            let mut sorted = Map::new();
            for (key, value) in entries {
                sorted.insert(key.clone(), canonicalize_json(value));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json).collect()),
        other => other.clone(),
    }
}

/// Drop volatile keys and collapse whitespace inside strings, recursively.
fn normalize_args(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, val) in map {
                if is_volatile_key(key) {
                    continue;
                }
                out.insert(key.clone(), normalize_args(val));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(normalize_args).collect()),
        Value::String(text) => Value::String(collapse_whitespace(text)),
        other => other.clone(),
    }
}

fn is_volatile_key(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    VOLATILE_ARG_KEYS.contains(&lowered.as_str())
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Flatten a normalized argument value into `path -> leaf` pairs.
fn flatten_leaves(value: &Value, path: &str, out: &mut BTreeMap<String, Value>) {
    match value {
        Value::Object(map) => {
            if map.is_empty() {
                out.insert(path.to_string(), Value::Object(Map::new()));
                return;
            }
            for (key, val) in map {
                let child = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                flatten_leaves(val, &child, out);
            }
        }
        Value::Array(items) => {
            if items.is_empty() {
                out.insert(path.to_string(), Value::Array(Vec::new()));
                return;
            }
            for (index, item) in items.iter().enumerate() {
                flatten_leaves(item, &format!("{path}[{index}]"), out);
            }
        }
        leaf => {
            out.insert(path.to_string(), leaf.clone());
        }
    }
}

/// Similarity of two *normalized* argument sets, in `0.0..=1.0`.
///
/// Leaves are compared per path: equal leaves score 1, two strings score their
/// character-trigram Jaccard scaled by their length ratio, anything else scores
/// 0. A path present on only one side scores 0. The result is the mean over the
/// union of paths, so a single changed filename in a two-argument call drops the
/// score to ~0.5 — well below any usable threshold — while a one-token tweak
/// inside a long command stays high. That asymmetry is the point: it flags
/// "same call, nudged" and ignores "different input, same shape".
fn arg_similarity(a: &Value, b: &Value) -> f32 {
    if a == b {
        return 1.0;
    }
    let mut left = BTreeMap::new();
    let mut right = BTreeMap::new();
    flatten_leaves(a, "", &mut left);
    flatten_leaves(b, "", &mut right);

    let paths: HashSet<&String> = left.keys().chain(right.keys()).collect();
    if paths.is_empty() {
        return 1.0;
    }

    let total: f32 = paths
        .iter()
        .map(|path| match (left.get(*path), right.get(*path)) {
            (Some(lhs), Some(rhs)) => leaf_similarity(lhs, rhs),
            _ => 0.0,
        })
        .sum();

    total / paths.len() as f32
}

fn leaf_similarity(a: &Value, b: &Value) -> f32 {
    if a == b {
        return 1.0;
    }
    match (a, b) {
        (Value::String(lhs), Value::String(rhs)) => string_similarity(lhs, rhs),
        _ => 0.0,
    }
}

/// Character-trigram Jaccard similarity, scaled by the length ratio so that a
/// much longer string never scores high against a short prefix of itself.
fn string_similarity(a: &str, b: &str) -> f32 {
    let left: Vec<char> = a.chars().collect();
    let right: Vec<char> = b.chars().collect();
    if left.len() < 3 || right.len() < 3 {
        // Too short for trigrams; only exact equality (handled above) counts.
        return 0.0;
    }

    let left_grams = trigrams(&left);
    let right_grams = trigrams(&right);
    let intersection = left_grams.intersection(&right_grams).count() as f32;
    let union = left_grams.union(&right_grams).count() as f32;
    if union == 0.0 {
        return 0.0;
    }

    let length_ratio = left.len().min(right.len()) as f32 / left.len().max(right.len()) as f32;
    (intersection / union) * length_ratio
}

fn trigrams(chars: &[char]) -> HashSet<String> {
    chars
        .windows(3)
        .map(|window| window.iter().collect::<String>())
        .collect()
}

/// Length of the trailing run of calls that are all near-duplicates of the most
/// recent call, plus whether that run is byte-exact throughout.
///
/// Every member is compared against the *last* call rather than against its
/// predecessor: chained pairwise comparison would let a slowly drifting sequence
/// (each call 0.9 similar to the one before, but nothing like the first)
/// accumulate an arbitrarily long "streak".
fn trailing_near_dup_run(seq: &[InternalToolCall], threshold: f32) -> (u32, bool) {
    let Some(current) = seq.last() else {
        return (0, true);
    };
    let mut run = 1u32;
    let mut all_exact = true;
    for previous in seq.iter().rev().skip(1) {
        if !previous.near_matches(current, threshold) {
            break;
        }
        if !previous.matches(current) {
            all_exact = false;
        }
        run += 1;
    }
    (run, all_exact)
}

/// Length of the trailing `A/B/A/B` alternation ending at the last call, where
/// `A != B` and each of `A`, `B` recurs with the same normalized shape. Returns
/// 0 when the tail is not an alternation.
fn trailing_oscillation_run(seq: &[InternalToolCall]) -> u32 {
    let len = seq.len();
    if len < 4 {
        return 0;
    }
    let (current, previous) = (&seq[len - 1], &seq[len - 2]);
    if current.same_shape(previous) {
        // A/A is plain repetition, not oscillation — BR-29 owns that.
        return 0;
    }

    let mut run = 2u32;
    let mut index = len - 3;
    loop {
        let expected = if (len - 1 - index).is_multiple_of(2) {
            current
        } else {
            previous
        };
        if !seq[index].same_shape(expected) {
            break;
        }
        run += 1;
        if index == 0 {
            break;
        }
        index -= 1;
    }

    if run >= 4 {
        run
    } else {
        0
    }
}

/// Index of the first message after the last genuine user turn. Loop-guard
/// nudges and other agent-authored injections are `Message::user()` with
/// `user_visible == false`, and tool *responses* are also user-role messages, so
/// neither resets the window.
fn last_user_turn_index(messages: &[Message]) -> usize {
    messages
        .iter()
        .rposition(|message| {
            message.role == Role::User
                && message.metadata.user_visible
                && !message
                    .content
                    .iter()
                    .any(|content| content.as_tool_response().is_some())
        })
        .map(|index| index + 1)
        .unwrap_or(0)
}

#[derive(Debug)]
struct CallObservation {
    id: String,
    call: InternalToolCall,
    outcome: Option<ToolOutcome>,
    fingerprint: Option<[u8; 32]>,
    active_job: bool,
    response_index: Option<usize>,
}

struct RepetitionHistory {
    calls: Vec<InternalToolCall>,
    completed: Vec<InternalToolCall>,
    outcomes: Vec<ToolOutcome>,
    limit: usize,
}

fn call_observations(messages: &[Message], limit: usize) -> Vec<CallObservation> {
    let start = last_user_turn_index(messages);
    let mut calls = Vec::with_capacity(limit.min(LOOP_WINDOW));
    let mut first_request_index = messages.len();
    'messages: for (index, message) in messages.iter().enumerate().skip(start).rev() {
        for request in message
            .content
            .iter()
            .rev()
            .filter_map(|part| part.as_tool_request())
        {
            if let Some(call) = InternalToolCall::from_request(request) {
                first_request_index = index;
                calls.push(CallObservation {
                    id: request.id.clone(),
                    call,
                    outcome: None,
                    fingerprint: None,
                    active_job: false,
                    response_index: None,
                });
                if calls.len() == limit {
                    break 'messages;
                }
            }
        }
    }
    calls.reverse();
    let indices: HashMap<_, _> = calls
        .iter()
        .enumerate()
        .map(|(i, call)| (call.id.clone(), i))
        .collect();
    for (index, message) in messages.iter().enumerate().skip(first_request_index) {
        for response in message
            .content
            .iter()
            .filter_map(|part| part.as_tool_response())
        {
            let Some(&call_index) = indices.get(&response.id) else {
                continue;
            };
            let observation = &mut calls[call_index];
            observation.outcome = Some(tool_outcome(&observation.call.name, &response.tool_result));
            observation.response_index = Some(index);
            if let Ok(result) = &response.tool_result {
                if observation
                    .outcome
                    .as_ref()
                    .is_some_and(|outcome| outcome.failure.is_none())
                {
                    observation.active_job = active_job_result(&observation.call, result);
                    observation.fingerprint = Some(result_fingerprint(result));
                }
            }
        }
    }
    calls
}

fn result_fingerprint(result: &CallToolResult) -> [u8; 32] {
    let mut value = serde_json::json!({
        "content": result.content,
        "structured_content": result.structured_content,
    });
    if let Some(content) = value.get_mut("content").and_then(Value::as_array_mut) {
        for part in content {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                    part["text"] = parsed;
                }
            }
        }
    }
    let bytes = serde_json::to_vec(&normalize_args(&value)).unwrap_or_default();
    Sha256::digest(bytes).into()
}

fn active_job_state(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    let has_id = [
        "job_id",
        "jobId",
        "operation_id",
        "operationId",
        "task_id",
        "taskId",
        "session_id",
        "sessionId",
        "run_id",
        "runId",
        "check_run_id",
    ]
    .iter()
    .any(|key| {
        object
            .get(*key)
            .is_some_and(|id| id.as_str().is_some_and(|id| !id.is_empty()) || id.is_u64())
    });
    has_id
        && ["status", "state"].iter().any(|key| {
            object
                .get(*key)
                .and_then(Value::as_str)
                .is_some_and(|state| {
                    matches!(
                        state.to_ascii_lowercase().as_str(),
                        "pending" | "queued" | "running" | "in_progress" | "in-progress"
                    )
                })
        })
}

fn active_job_result(call: &InternalToolCall, result: &CallToolResult) -> bool {
    if result
        .structured_content
        .as_ref()
        .is_some_and(active_job_state)
        || result
            .content
            .iter()
            .filter_map(|part| part.as_text())
            .any(|text| {
                serde_json::from_str::<Value>(&text.text)
                    .ok()
                    .as_ref()
                    .is_some_and(active_job_state)
            })
    {
        return true;
    }
    // The built-in shell predates structured job results. Match its actual
    // job-bound status header, never arbitrary tool prose containing "running".
    if call.name.rsplit("__").next() != Some("shell_status") {
        return false;
    }
    let Some(id) = call.parameters.get("job_id").and_then(Value::as_str) else {
        return false;
    };
    result
        .content
        .iter()
        .filter_map(|part| part.as_text())
        .any(|text| {
            text.text.lines().next().is_some_and(|line| {
                line.starts_with(&format!("job {id} [")) && line.contains("], status: running (")
            })
        })
}

fn observation_progress(
    calls: &[CallObservation],
    index: usize,
    fresh_input: bool,
    threshold: f32,
) -> bool {
    let current = &calls[index];
    let Some(fingerprint) = current.fingerprint else {
        return false;
    };
    if current.active_job {
        return true;
    }
    let previous = calls[..index].iter().rev().find(|previous| {
        previous.call.near_matches(&current.call, threshold) && previous.outcome.is_some()
    });
    match previous {
        Some(previous) => previous.fingerprint != Some(fingerprint),
        None => fresh_input,
    }
}

fn last_progress_index(
    calls: &[CallObservation],
    fresh_input: bool,
    threshold: f32,
) -> Option<usize> {
    (0..calls.len())
        .rev()
        .find(|index| observation_progress(calls, *index, fresh_input, threshold))
}

/// Whether a newly completed tool call produced a changed successful result,
/// succeeded on a new input, or confirmed that an identified job is still active.
/// Only a bounded recent window is retained while comparing results.
pub fn tool_progress_since(messages: &[Message], start_index: usize) -> bool {
    let calls = call_observations(messages, LOOP_WINDOW);
    (0..calls.len()).any(|index| {
        calls[index]
            .response_index
            .is_some_and(|response| response >= start_index)
            && observation_progress(&calls, index, true, DEFAULT_ARG_SIMILARITY_THRESHOLD)
    })
}

/// Check newly completed response identities even when compaction renumbered
/// the conversation while the tool batch was running.
pub fn tool_progress_for_responses(messages: &[Message], response_ids: &HashSet<String>) -> bool {
    let calls = call_observations(messages, LOOP_WINDOW);
    (0..calls.len()).any(|index| {
        response_ids.contains(&calls[index].id)
            && calls[index].response_index.is_some()
            && observation_progress(&calls, index, true, DEFAULT_ARG_SIMILARITY_THRESHOLD)
    })
}

/// Failure evidence after the most recent successful change of approach.
/// Re-reading an unchanged result does not repeatedly clear a failing streak.
pub fn tool_outcomes_since_last_progress(
    messages: &[Message],
    config: &FailureLoopConfig,
) -> Vec<ToolOutcome> {
    let limit = [config.soft_warn_at, config.escalate_at, config.hard_stop_at]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(0)
        .max(LOOP_WINDOW as u32) as usize;
    let calls = call_observations(messages, limit);
    let start = last_progress_index(&calls, true, DEFAULT_ARG_SIMILARITY_THRESHOLD).unwrap_or(0);
    calls
        .into_iter()
        .skip(start)
        .filter_map(|call| call.outcome)
        .collect()
}

/// Staged repetition guard (BR-29).
///
/// Exact tool-call signatures are counted while completed results remain
/// unchanged, including duplicates in the current batch. Successful changed
/// results, active jobs and a new user turn clear the no-progress window.
///
/// * `soft_warn_at` — the call still runs, but a non-blocking warning is emitted
///   (`InspectionAction::Warn`) and injected into the model's context so it can
///   change approach before it is stopped.
/// * `hard_stop_at` — the call is denied (`InspectionAction::Deny`).
///
/// A single hard deny with no prior nudge was the old behavior; the soft stage
/// gives the model one chance to break the loop itself.
/// BR-30 adds two heuristics on top, over the calls made since the last user
/// turn: *near-duplicate* repeats (same tool, arguments only tweaked) and
/// *`A/B/A/B` oscillation*. Both warn before blocking persistent repetition.
///
/// BR-31 adds the result-aware stage: a tool that keeps *failing the same way*
/// is denied here, on its next call. The nudges that precede that stop are
/// emitted at the result-collection seam in the agent loop (a failure cannot be
/// pre-empted — only the call after it can).
#[derive(Debug)]
pub struct RepetitionInspector {
    /// Nth identical call in one user turn that earns a non-blocking warning.
    /// `None` (or `>= hard_stop_at`) disables the soft stage.
    soft_warn_at: Option<u32>,
    /// Nth identical call in one user turn that is denied. `None` disables the guard.
    hard_stop_at: Option<u32>,
    /// BR-30 near-duplicate + oscillation heuristics.
    semantic: SemanticLoopConfig,
    /// BR-31 repeated-failing-result guard (hard stage only; see above).
    failure: FailureLoopConfig,
}

impl RepetitionInspector {
    /// Hard-stop-only guard: deny once a call has repeated *more than*
    /// `max_repetitions` times in one user turn. No soft stage or semantic heuristics.
    pub fn new(max_repetitions: Option<u32>) -> Self {
        Self {
            soft_warn_at: None,
            hard_stop_at: max_repetitions.map(|max| max.saturating_add(1)),
            semantic: SemanticLoopConfig::disabled(),
            failure: FailureLoopConfig::disabled(),
        }
    }

    /// Staged guard: warn on the `soft_warn_at`-th identical call, deny on the
    /// `hard_stop_at`-th. If `soft_warn_at >= hard_stop_at` the soft stage never
    /// fires and this degrades to a hard stop. The BR-30 semantic heuristics run
    /// with their default staged configuration; override with
    /// [`RepetitionInspector::with_semantic`].
    pub fn staged(soft_warn_at: u32, hard_stop_at: u32) -> Self {
        Self {
            soft_warn_at: (soft_warn_at > 0).then_some(soft_warn_at),
            hard_stop_at: (hard_stop_at > 0).then_some(hard_stop_at),
            semantic: SemanticLoopConfig::default(),
            failure: FailureLoopConfig::default(),
        }
    }

    /// Replace the BR-30 semantic-loop configuration.
    pub fn with_semantic(mut self, semantic: SemanticLoopConfig) -> Self {
        self.semantic = semantic;
        self
    }

    /// Replace the BR-31 repeated-failing-result configuration.
    pub fn with_failure_loop(mut self, failure: FailureLoopConfig) -> Self {
        self.failure = failure;
        self
    }

    fn history(&self, messages: &[Message]) -> RepetitionHistory {
        let history_limit = [
            self.soft_warn_at,
            self.hard_stop_at,
            self.semantic.near_dup_soft_warn,
            self.semantic.near_dup_hard_stop,
            self.semantic.oscillation_soft_warn,
            self.semantic.oscillation_hard_stop,
            self.failure.soft_warn_at,
            self.failure.escalate_at,
            self.failure.hard_stop_at,
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(0)
        .max(LOOP_WINDOW as u32) as usize;
        let observations = call_observations(messages, history_limit);
        let progress =
            last_progress_index(&observations, false, self.semantic.similarity_threshold);
        let start = progress
            .map(|index| index + usize::from(observations[index].active_job))
            .unwrap_or(0);
        let turn_calls: Vec<_> = observations
            .iter()
            .skip(start)
            .map(|item| item.call.clone())
            .collect();
        let outcomes = if self.failure.enabled {
            let start =
                last_progress_index(&observations, true, self.semantic.similarity_threshold)
                    .unwrap_or(0);
            observations
                .iter()
                .skip(start)
                .filter_map(|call| call.outcome.clone())
                .collect()
        } else {
            Vec::new()
        };
        let completed = observations
            .iter()
            .skip(start)
            .filter(|item| item.outcome.is_some())
            .map(|item| item.call.clone())
            .collect();
        RepetitionHistory {
            calls: turn_calls,
            completed,
            outcomes,
            limit: history_limit,
        }
    }

    /// The message the model sees in place of the tool result when a call is
    /// hard-stopped. It states the *real* reason — the repetition guard fired —
    /// rather than claiming the user declined (which was the old, misleading
    /// `DECLINED_RESPONSE`).
    fn hard_stop_reason(tool_name: &str, repeat_count: u32) -> String {
        format!(
            "Biorouter stopped this tool call: '{tool_name}' has now been called \
             with identical arguments {repeat_count} times without a changed result. The user did \
             NOT decline it; this is an automatic repetition guard. Repeating the \
             same call will not produce a different result. Change approach: vary \
             the arguments, use a different tool, or explain what is blocking you \
             and stop."
        )
    }

    /// Non-blocking nudge injected into the model's context. The call still ran.
    fn soft_warn_reason(tool_name: &str, repeat_count: u32, hard_stop_at: Option<u32>) -> String {
        let stop = hard_stop_at
            .map(|limit| {
                format!(" It will be stopped automatically on the {limit}th unchanged call.")
            })
            .unwrap_or_default();
        format!(
            "Repetition warning: you have called '{tool_name}' with identical \
             arguments {repeat_count} times without a changed result.{stop} \
             Change approach now: vary the arguments, use a different tool, or \
             explain what is blocking you and stop."
        )
    }

    fn inspect_exact(
        &self,
        request_id: &str,
        tool_name: &str,
        repeat_count: u32,
    ) -> Option<(InspectionResult, u32)> {
        let (action, reason, finding_id) = if self.hard_stop_at.is_some_and(|at| repeat_count >= at)
        {
            (
                InspectionAction::Deny,
                Self::hard_stop_reason(tool_name, repeat_count),
                REPETITION_HARD_FINDING_ID,
            )
        } else if self.soft_warn_at.is_some_and(|at| repeat_count >= at) {
            (
                InspectionAction::Warn,
                Self::soft_warn_reason(tool_name, repeat_count, self.hard_stop_at),
                REPETITION_SOFT_FINDING_ID,
            )
        } else {
            return None;
        };
        Some((
            InspectionResult {
                tool_request_id: request_id.to_string(),
                action,
                reason,
                confidence: 1.0,
                inspector_name: REPETITION_INSPECTOR_NAME.to_string(),
                finding_id: Some(finding_id.to_string()),
            },
            repeat_count,
        ))
    }

    /// BR-30 near-duplicate nudge. Hedged, because the heuristic can be wrong:
    /// a model that really is iterating over distinct inputs should ignore it.
    fn near_dup_warn_reason(tool_name: &str, run: u32) -> String {
        format!(
            "Possible loop: '{tool_name}' has now been called {run} times in a row \
             with only minor argument tweaks (the calls are near-identical once \
             ids and whitespace are ignored). Small edits to the same call rarely \
             produce a different result. If you are deliberately iterating over \
             genuinely different inputs, ignore this and continue; otherwise stop \
             tweaking and change approach: use a different tool, re-read what the \
             last result actually said, or explain what is blocking you."
        )
    }

    fn near_dup_stop_reason(tool_name: &str, run: u32) -> String {
        format!(
            "Biorouter stopped this tool call: '{tool_name}' has been called {run} \
             times in a row with near-identical arguments (only minor tweaks). The \
             user did NOT decline it; this is an automatic loop guard. Change \
             approach: use a different tool, re-read the last result, or explain \
             what is blocking you and stop."
        )
    }

    /// BR-30 oscillation nudge (`A/B/A/B`).
    fn oscillation_warn_reason(tool_name: &str, run: u32) -> String {
        format!(
            "Possible loop: your last {run} tool calls alternate between the same \
             two calls (…the most recent being '{tool_name}'), each repeated with \
             the same arguments. An A/B/A/B cycle repeats work rather than making \
             progress. If you are polling for a result that genuinely changes over \
             time, say so and continue; otherwise break the cycle: try a different \
             approach or explain what is blocking you."
        )
    }

    fn oscillation_stop_reason(tool_name: &str, run: u32) -> String {
        format!(
            "Biorouter stopped this tool call: your last {run} tool calls alternate \
             between the same two calls (…the most recent being '{tool_name}'), each \
             with unchanged arguments. The user did NOT decline it; this is an \
             automatic loop guard. Break the cycle: try a different approach, or \
             explain what is blocking you and stop."
        )
    }

    /// BR-30: near-duplicate + oscillation heuristics over the recent call
    /// window. Returns at most one result, the most severe that applies, paired
    /// with the run length that tripped it (BR-67 reports it).
    fn inspect_semantic(
        &self,
        tool_request_id: &str,
        tool_name: &str,
        window: &[InternalToolCall],
        completed: &[InternalToolCall],
    ) -> Option<(InspectionResult, u32)> {
        if !self.semantic.enabled {
            return None;
        }

        let (near_dup_run, all_exact) =
            trailing_near_dup_run(window, self.semantic.similarity_threshold);
        // A run of byte-identical calls is the BR-29 guard's business; only flag
        // it here when at least one member differs (an actual argument tweak).
        let near_dup_run = if all_exact { 0 } else { near_dup_run };
        let oscillation_run = trailing_oscillation_run(window);
        let mut confirmed = completed.to_vec();
        confirmed.push(window.last()?.clone());
        let (confirmed_near, confirmed_exact) =
            trailing_near_dup_run(&confirmed, self.semantic.similarity_threshold);
        let confirmed_near = if confirmed_exact {
            0
        } else {
            confirmed_near.saturating_sub(1)
        };
        let confirmed_oscillation = trailing_oscillation_run(&confirmed).saturating_sub(1);

        let fired = |threshold: Option<u32>, run: u32| -> bool {
            threshold.is_some_and(|at| at > 0 && run >= at)
        };

        let (action, reason, finding_id, run) =
            if fired(self.semantic.near_dup_hard_stop, confirmed_near) {
                (
                    InspectionAction::Deny,
                    Self::near_dup_stop_reason(tool_name, confirmed_near),
                    REPETITION_NEAR_DUP_HARD_FINDING_ID,
                    confirmed_near,
                )
            } else if fired(self.semantic.oscillation_hard_stop, confirmed_oscillation) {
                (
                    InspectionAction::Deny,
                    Self::oscillation_stop_reason(tool_name, confirmed_oscillation),
                    REPETITION_OSCILLATION_HARD_FINDING_ID,
                    confirmed_oscillation,
                )
            } else if fired(self.semantic.near_dup_soft_warn, near_dup_run) {
                (
                    InspectionAction::Warn,
                    Self::near_dup_warn_reason(tool_name, near_dup_run),
                    REPETITION_NEAR_DUP_SOFT_FINDING_ID,
                    near_dup_run,
                )
            } else if fired(self.semantic.oscillation_soft_warn, oscillation_run) {
                (
                    InspectionAction::Warn,
                    Self::oscillation_warn_reason(tool_name, oscillation_run),
                    REPETITION_OSCILLATION_SOFT_FINDING_ID,
                    oscillation_run,
                )
            } else {
                return None;
            };

        Some((
            InspectionResult {
                tool_request_id: tool_request_id.to_string(),
                action,
                reason,
                // Heuristic, not a proof: below the 1.0 the byte-exact guard claims.
                confidence: 0.8,
                inspector_name: REPETITION_INSPECTOR_NAME.to_string(),
                finding_id: Some(finding_id.to_string()),
            },
            run,
        ))
    }

    /// BR-31 hard stage: deny a call to a tool that has already failed the same
    /// way `hard_stop_at` times in a row since the last user turn. Paired with
    /// the failing streak that tripped it (BR-67 reports it).
    fn inspect_failure_loop(
        &self,
        tool_request_id: &str,
        tool_name: &str,
        outcomes: &[ToolOutcome],
    ) -> Option<(InspectionResult, u32)> {
        if !self.failure.enabled {
            return None;
        }
        let hard_stop_at = self.failure.hard_stop_at.filter(|at| *at > 0)?;
        let streak = failing_streak(outcomes, tool_name, self.failure.similarity_threshold);
        if streak < hard_stop_at {
            return None;
        }
        // BR-51: the hard stop rests on "an identical failure cannot become a
        // success" — which is not true of a timeout or a transient dependency
        // error. Denying the call that would finally have worked is a worse
        // failure mode than one more retry, so a retryable streak is nudged, not
        // blocked (unless the operator opted in).
        if !self.failure.deny_retryable && streak_is_retryable(outcomes, tool_name) {
            return None;
        }

        Some((
            InspectionResult {
                tool_request_id: tool_request_id.to_string(),
                action: InspectionAction::Deny,
                reason: failure_stop_reason(tool_name, streak),
                // The tool ran and failed, every time: this is observed, not guessed.
                confidence: 1.0,
                inspector_name: REPETITION_INSPECTOR_NAME.to_string(),
                finding_id: Some(FAILURE_LOOP_HARD_FINDING_ID.to_string()),
            },
            streak,
        ))
    }
}

/// BR-67: report a repetition-guard verdict to the loop-safety trace.
///
/// Only the tool *name*, the stable finding id and the run length that tripped
/// the guard travel; the reason prose (which quotes the model's own framing)
/// never does.
fn emit_repetition_verdict(session_id: &str, tool_name: &str, result: &InspectionResult, run: u32) {
    use crate::observability::loop_safety::{LoopSafetyEvent, LoopSafetyKind};

    let kind = match result.action {
        InspectionAction::Deny => LoopSafetyKind::RepetitionStop,
        _ => LoopSafetyKind::RepetitionWarn,
    };
    let mut event = LoopSafetyEvent::new(kind)
        .session(session_id)
        .tool(tool_name)
        .count(run);
    if let Some(finding_id) = &result.finding_id {
        event = event.finding_id(finding_id);
    }
    crate::observability::loop_safety::emit(event);
}

#[async_trait]
impl ToolInspector for RepetitionInspector {
    fn name(&self) -> &'static str {
        REPETITION_INSPECTOR_NAME
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn inspect(
        &self,
        tool_requests: &[ToolRequest],
        messages: &[Message],
        _biorouter_mode: BioRouterMode,
        session: &crate::session::Session,
    ) -> Result<Vec<InspectionResult>> {
        let mut results = Vec::new();
        if self.soft_warn_at.is_none()
            && self.hard_stop_at.is_none()
            && !self.semantic.enabled
            && !self.failure.enabled
        {
            return Ok(results);
        }

        let RepetitionHistory {
            calls: turn_calls,
            completed,
            outcomes,
            limit: history_limit,
        } = self.history(messages);
        let mut seen: HashMap<String, u32> = HashMap::new();
        for call in &turn_calls {
            *seen.entry(call.signature()).or_insert(0) += 1;
        }

        let mut window = if self.semantic.enabled {
            turn_calls
        } else {
            Vec::new()
        };

        for tool_request in tool_requests {
            if let Some(call) = InternalToolCall::from_request(tool_request) {
                let count = seen.entry(call.signature()).or_insert(0);
                *count += 1;
                let repeat_count = *count;

                window.push(call);
                if window.len() > history_limit {
                    window.remove(0);
                }

                let tool_name = tool_request
                    .tool_call
                    .as_ref()
                    .map(|tool_call| tool_call.name.to_string())
                    .unwrap_or_else(|_| "unknown".to_string());

                let exact_result = self.inspect_exact(&tool_request.id, &tool_name, repeat_count);

                let failure_result =
                    self.inspect_failure_loop(&tool_request.id, &tool_name, &outcomes);

                let semantic_result =
                    self.inspect_semantic(&tool_request.id, &tool_name, &window, &completed);
                let verdict = [exact_result, failure_result, semantic_result]
                    .into_iter()
                    .flatten()
                    .min_by_key(|(result, _)| match result.action {
                        InspectionAction::Deny => 0,
                        _ => 1,
                    });
                if let Some((result, run)) = verdict {
                    // BR-67: the guard tripping is exactly the moment an operator
                    // needs on the record — which guard, on which tool, at what
                    // run length, and whether the call was stopped or just warned.
                    emit_repetition_verdict(&session.id, &tool_name, &result, run);
                    results.push(result);
                }
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::object;

    fn call(name: &str, args: Value) -> InternalToolCall {
        InternalToolCall::from_tool_call(&CallToolRequestParams {
            task: None,
            meta: None,
            name: name.to_string().into(),
            arguments: args.as_object().cloned(),
        })
    }

    #[test]
    fn volatile_keys_and_whitespace_normalize_away() {
        let a = call(
            "search",
            object!({"query": "TP53  variants", "request_id": "abc-1"}).into(),
        );
        let b = call(
            "search",
            object!({"query": "TP53 variants", "request_id": "abc-2"}).into(),
        );

        assert!(
            !a.matches(&b),
            "byte-exact comparison must still see a diff"
        );
        assert!(
            a.same_shape(&b),
            "normalization must erase ids + whitespace"
        );
        assert!(a.near_matches(&b, DEFAULT_ARG_SIMILARITY_THRESHOLD));
    }

    #[test]
    fn a_tweaked_long_command_is_a_near_duplicate() {
        let a = call(
            "shell",
            object!({"command": "grep -rn 'fn assemble_turn_context' crates/biorouter/src/agents"})
                .into(),
        );
        let b = call(
            "shell",
            object!({"command": "grep -rn 'fn assemble_turn_context' crates/biorouter/src/agent"})
                .into(),
        );
        assert!(
            a.near_matches(&b, DEFAULT_ARG_SIMILARITY_THRESHOLD),
            "similarity was {}",
            arg_similarity(&a.normalized, &b.normalized)
        );
    }

    // The false-positive that matters: iterating over genuinely different inputs
    // has the same call *shape* but must NOT read as a near-duplicate.
    #[test]
    fn distinct_targets_are_not_near_duplicates() {
        let a = call("read_file", object!({"path": "src/one.rs"}).into());
        let b = call("read_file", object!({"path": "src/two.rs"}).into());
        assert!(!a.near_matches(&b, DEFAULT_ARG_SIMILARITY_THRESHOLD));

        let page1 = call(
            "read_file",
            object!({"path": "big.csv", "offset": 0}).into(),
        );
        let page2 = call(
            "read_file",
            object!({"path": "big.csv", "offset": 500}).into(),
        );
        assert!(
            !page1.near_matches(&page2, DEFAULT_ARG_SIMILARITY_THRESHOLD),
            "pagination is progress, not a loop (similarity {})",
            arg_similarity(&page1.normalized, &page2.normalized)
        );

        let x = call("shell", object!({"command": "pytest test_alpha.py"}).into());
        let y = call("shell", object!({"command": "pytest test_gamma.py"}).into());
        assert!(
            !x.near_matches(&y, DEFAULT_ARG_SIMILARITY_THRESHOLD),
            "similarity was {}",
            arg_similarity(&x.normalized, &y.normalized)
        );
    }

    #[test]
    fn different_tools_never_match() {
        let a = call("read_file", object!({"path": "a.rs"}).into());
        let b = call("write_file", object!({"path": "a.rs"}).into());
        assert!(!a.near_matches(&b, DEFAULT_ARG_SIMILARITY_THRESHOLD));
        assert!(!a.same_shape(&b));
    }

    #[test]
    fn near_dup_run_anchors_on_the_current_call() {
        // A slowly drifting sequence: each call resembles its predecessor but the
        // first and last are unrelated. It must not accumulate a long run.
        let seq = vec![
            call("shell", object!({"command": "aaaaaaaaaaaaaaaaaaaa"}).into()),
            call("shell", object!({"command": "aaaaaaaaaabbbbbbbbbb"}).into()),
            call("shell", object!({"command": "bbbbbbbbbbbbbbbbbbbb"}).into()),
        ];
        let (run, _) = trailing_near_dup_run(&seq, DEFAULT_ARG_SIMILARITY_THRESHOLD);
        assert_eq!(run, 1);
    }

    #[test]
    fn near_dup_run_reports_whether_the_run_is_byte_exact() {
        let identical = call("search", object!({"query": "TP53"}).into());
        let seq = vec![identical.clone(), identical.clone(), identical];
        let (run, all_exact) = trailing_near_dup_run(&seq, DEFAULT_ARG_SIMILARITY_THRESHOLD);
        assert_eq!(run, 3);
        assert!(all_exact, "a byte-exact run belongs to the BR-29 guard");

        let seq = vec![
            call("search", object!({"query": "TP53  variants"}).into()),
            call("search", object!({"query": "TP53 variants"}).into()),
        ];
        let (run, all_exact) = trailing_near_dup_run(&seq, DEFAULT_ARG_SIMILARITY_THRESHOLD);
        assert_eq!(run, 2);
        assert!(!all_exact);
    }

    #[test]
    fn oscillation_needs_two_full_cycles_of_two_distinct_calls() {
        let a = call("read_file", object!({"path": "a.rs"}).into());
        let b = call("shell", object!({"command": "cargo build"}).into());

        assert_eq!(trailing_oscillation_run(&[a.clone(), b.clone()]), 0);
        assert_eq!(
            trailing_oscillation_run(&[a.clone(), b.clone(), a.clone()]),
            0,
            "A/B/A is one and a half cycles, which is not enough"
        );
        assert_eq!(
            trailing_oscillation_run(&[a.clone(), b.clone(), a.clone(), b.clone()]),
            4
        );
        assert_eq!(
            trailing_oscillation_run(&[
                a.clone(),
                b.clone(),
                a.clone(),
                b.clone(),
                a.clone(),
                b.clone()
            ]),
            6
        );

        // A/A/A/A is plain repetition (BR-29), not oscillation.
        assert_eq!(
            trailing_oscillation_run(&[a.clone(), a.clone(), a.clone(), a.clone()]),
            0
        );
        // A third distinct call breaks the alternation.
        let c = call("shell", object!({"command": "cargo fmt"}).into());
        assert_eq!(trailing_oscillation_run(&[a.clone(), b.clone(), c, b]), 0);
    }

    #[test]
    fn oscillation_ignores_volatile_argument_churn() {
        let a1 = call("poll", object!({"job": "x", "request_id": "1"}).into());
        let b1 = call("wait", object!({"secs": 5, "request_id": "2"}).into());
        let a2 = call("poll", object!({"job": "x", "request_id": "3"}).into());
        let b2 = call("wait", object!({"secs": 5, "request_id": "4"}).into());
        assert_eq!(trailing_oscillation_run(&[a1, b1, a2, b2]), 4);
    }

    // ---------------------------------------------------------------------
    // BR-31: repeated-failing-result / no-progress detector
    // ---------------------------------------------------------------------

    fn failed(tool_name: &str, error: &str) -> ToolOutcome {
        ToolOutcome {
            tool_name: tool_name.to_string(),
            failure: Some(error_signature(error)),
            kind: Some(crate::agents::tool_errors::kind_from_text(error)),
        }
    }

    fn succeeded(tool_name: &str) -> ToolOutcome {
        ToolOutcome {
            tool_name: tool_name.to_string(),
            failure: None,
            kind: None,
        }
    }

    fn error_result(text: &str) -> ToolResult<CallToolResult> {
        Ok(CallToolResult {
            content: vec![rmcp::model::Content::text(text)],
            structured_content: None,
            is_error: Some(true),
            meta: None,
        })
    }

    fn ok_result(text: &str) -> ToolResult<CallToolResult> {
        Ok(CallToolResult {
            content: vec![rmcp::model::Content::text(text)],
            structured_content: None,
            is_error: Some(false),
            meta: None,
        })
    }

    #[test]
    fn error_signature_folds_case_whitespace_and_digit_runs() {
        // The same failure re-run: only a pid and a timestamp differ.
        let first = error_signature("Error: process 4821 failed at 12:03:44\n  exit code 1");
        let second = error_signature("error: process 991 failed at 09:17:02   exit code 1");
        assert_eq!(first, second, "ids and clocks must not fork the signature");

        // A materially different error must not collapse into the same signature.
        assert_ne!(
            error_signature("no such file or directory: config.yaml"),
            error_signature("permission denied: config.yaml")
        );
    }

    #[test]
    fn failing_streak_counts_consecutive_identical_failures_of_one_tool() {
        let outcomes = vec![
            failed("shell", "cargo: command not found"),
            failed("shell", "cargo: command not found"),
            failed("shell", "cargo: command not found"),
        ];
        assert_eq!(
            failing_streak(&outcomes, "shell", DEFAULT_ERROR_SIMILARITY_THRESHOLD),
            3
        );
        // A tool that never ran has no streak.
        assert_eq!(
            failing_streak(&outcomes, "read_file", DEFAULT_ERROR_SIMILARITY_THRESHOLD),
            0
        );
    }

    #[test]
    fn a_success_or_a_new_error_breaks_the_streak() {
        // Success is progress.
        let outcomes = vec![
            failed("shell", "cargo: command not found"),
            failed("shell", "cargo: command not found"),
            succeeded("shell"),
        ];
        assert_eq!(
            failing_streak(&outcomes, "shell", DEFAULT_ERROR_SIMILARITY_THRESHOLD),
            0
        );

        // A *different* error is new information — the model learned something.
        let outcomes = vec![
            failed("shell", "cargo: command not found"),
            failed("shell", "cargo: command not found"),
            failed(
                "shell",
                "error[E0433]: failed to resolve: use of undeclared crate",
            ),
        ];
        assert_eq!(
            failing_streak(&outcomes, "shell", DEFAULT_ERROR_SIMILARITY_THRESHOLD),
            1
        );
    }

    #[test]
    fn interleaved_other_tools_neither_reset_nor_hide_a_streak() {
        let outcomes = vec![
            failed("shell", "cargo: command not found"),
            succeeded("read_file"),
            failed("shell", "cargo: command not found"),
            succeeded("read_file"),
            failed("shell", "cargo: command not found"),
        ];
        assert_eq!(
            failing_streak(&outcomes, "shell", DEFAULT_ERROR_SIMILARITY_THRESHOLD),
            3,
            "a tool's own failures are the streak; other tools are irrelevant"
        );
    }

    #[test]
    fn nudges_escalate_then_hand_off_to_the_hard_stop() {
        let config = FailureLoopConfig::default();
        let mut outcomes = vec![failed("shell", "cargo: command not found")];
        assert!(
            failure_loop_nudge(&config, &outcomes, "shell").is_none(),
            "one failure is not a loop"
        );

        outcomes.push(failed("shell", "cargo: command not found"));
        assert!(failure_loop_nudge(&config, &outcomes, "shell").is_none());

        outcomes.push(failed("shell", "cargo: command not found"));
        let soft = failure_loop_nudge(&config, &outcomes, "shell").expect("soft nudge at 3");
        assert!(soft.starts_with("No progress"), "{soft}");
        assert!(soft.contains("failed 3 times"), "{soft}");

        outcomes.push(failed("shell", "cargo: command not found"));
        outcomes.push(failed("shell", "cargo: command not found"));
        let escalated =
            failure_loop_nudge(&config, &outcomes, "shell").expect("escalated nudge at 5");
        assert!(
            escalated.starts_with("You are not making progress"),
            "{escalated}"
        );
        assert!(
            escalated.contains("ask the user"),
            "the escalation must offer the honest way out: {escalated}"
        );
        assert!(
            escalated.contains("blocked automatically"),
            "the model must be told the stop is coming: {escalated}"
        );
    }

    #[test]
    fn a_disabled_detector_never_nudges() {
        let outcomes = vec![
            failed("shell", "boom"),
            failed("shell", "boom"),
            failed("shell", "boom"),
            failed("shell", "boom"),
        ];
        assert!(failure_loop_nudge(&FailureLoopConfig::disabled(), &outcomes, "shell").is_none());
    }

    #[test]
    fn outcomes_pair_requests_with_responses_and_reset_at_a_user_turn() {
        let request = |id: &str, name: &str| {
            Message::assistant().with_tool_request(
                id,
                Ok(CallToolRequestParams {
                    task: None,
                    meta: None,
                    name: name.to_string().into(),
                    arguments: None,
                }),
            )
        };

        let messages = vec![
            Message::user().with_text("build it"),
            request("call_1", "shell"),
            Message::user().with_tool_response("call_1", error_result("cargo: command not found")),
            request("call_2", "shell"),
            Message::user().with_tool_response("call_2", ok_result("done")),
        ];

        let outcomes = tool_outcomes_since_last_user_turn(&messages);
        assert_eq!(
            outcomes,
            vec![
                failed("shell", "cargo: command not found"),
                succeeded("shell"),
            ]
        );

        // A genuine user turn is a fresh start: whatever failed before it is not
        // this turn's loop.
        let mut with_new_turn = messages.clone();
        with_new_turn.push(Message::user().with_text("try again"));
        assert!(tool_outcomes_since_last_user_turn(&with_new_turn).is_empty());

        // …but a loop-guard nudge (hidden, agent-authored) must NOT reset it.
        let mut with_nudge = messages;
        with_nudge.push(
            Message::user()
                .with_text("<biorouter-loop-guard>…</biorouter-loop-guard>")
                .with_visibility(false, true),
        );
        assert_eq!(tool_outcomes_since_last_user_turn(&with_nudge).len(), 2);
    }

    #[test]
    fn a_transport_error_counts_as_a_failure() {
        let outcome = tool_outcome(
            "shell",
            &Err(rmcp::model::ErrorData::new(
                rmcp::model::ErrorCode::INTERNAL_ERROR,
                "connection closed",
                None,
            )),
        );
        assert_eq!(outcome.failure.as_deref(), Some("connection closed"));
    }

    // ---------------------------------------------------------------------
    // BR-51: the taxonomy the no-progress detector now reads
    // ---------------------------------------------------------------------

    #[test]
    fn an_outcome_carries_the_class_of_its_failure() {
        let missing = tool_outcome("read_file", &error_result("No such file or directory"));
        assert_eq!(missing.kind, Some(ToolErrorKind::NotFound));
        assert!(!missing.retryable());

        let flaky = tool_outcome("fetch", &error_result("connection reset by peer"));
        assert_eq!(flaky.kind, Some(ToolErrorKind::Transient));
        assert!(flaky.retryable());

        let fine = tool_outcome("shell", &ok_result("done"));
        assert_eq!(fine.kind, None);
        assert!(!fine.retryable());
    }

    /// The BR-51 headline for BR-31: the hard stop rests on "an identical failure
    /// cannot become a success", which is false for a timeout. A run of timeouts
    /// is nudged, never blocked — blocking the retry that would finally have
    /// worked strands the turn.
    #[test]
    fn retryable_streaks_can_be_configured_advisory_only() {
        let config = FailureLoopConfig {
            deny_retryable: false,
            ..FailureLoopConfig::default()
        };
        let inspector = RepetitionInspector::new(None).with_failure_loop(config.clone());
        let outcomes = vec![failed("fetch", "operation timed out"); 6];
        assert_eq!(
            failing_streak(&outcomes, "fetch", DEFAULT_ERROR_SIMILARITY_THRESHOLD),
            6
        );

        // Nudged: the model is told it is hammering something flaky…
        let nudge =
            failure_loop_nudge(&config, &outcomes, "fetch").expect("a six-failure streak nudges");
        assert!(nudge.contains("timeout"), "{nudge}");
        // …and not threatened with a block that will not come.
        assert!(!nudge.contains("blocked"), "{nudge}");

        // …but not denied.
        assert!(inspector
            .inspect_failure_loop("req-1", "fetch", &outcomes)
            .is_none());

        // A hard failure of the same length still is.
        let hard = vec![failed("shell", "cargo: command not found"); 6];
        let (result, streak) = inspector
            .inspect_failure_loop("req-2", "shell", &hard)
            .expect("a hard streak is denied");
        assert!(matches!(result.action, InspectionAction::Deny));
        assert_eq!(streak, 6);
    }

    /// Persistent retryable errors are also evidence when recovery changed nothing.
    #[test]
    fn persistent_retryable_errors_stop_after_the_recovery_window() {
        let inspector =
            RepetitionInspector::new(None).with_failure_loop(FailureLoopConfig::default());
        let outcomes = vec![failed("fetch", "operation timed out"); 6];

        let (result, _) = inspector
            .inspect_failure_loop("req-1", "fetch", &outcomes)
            .expect("persistent unchanged failure is denied by default");
        assert!(matches!(result.action, InspectionAction::Deny));
    }

    /// An unclassified failure (a `ToolOutcome` built without the taxonomy) must
    /// behave exactly as it did before BR-51: conservatively, as a hard failure.
    #[test]
    fn an_unclassified_failure_is_treated_as_hard() {
        let inspector =
            RepetitionInspector::new(None).with_failure_loop(FailureLoopConfig::default());
        let outcomes = vec![
            ToolOutcome {
                tool_name: "fetch".to_string(),
                failure: Some("timed out".to_string()),
                kind: None,
            };
            6
        ];

        assert!(inspector
            .inspect_failure_loop("req-1", "fetch", &outcomes)
            .is_some());
    }

    fn completed_call(
        messages: &mut Vec<Message>,
        name: &str,
        args: Value,
        result: ToolResult<CallToolResult>,
    ) {
        let id = format!("request_{}", messages.len());
        messages.push(Message::assistant().with_tool_request(
            &id,
            Ok(CallToolRequestParams {
                task: None,
                meta: None,
                name: name.to_string().into(),
                arguments: args.as_object().cloned(),
            }),
        ));
        messages.push(Message::user().with_tool_response(&id, result));
    }

    async fn inspect_next(
        inspector: &RepetitionInspector,
        messages: &[Message],
        name: &str,
        args: Value,
    ) -> Vec<InspectionResult> {
        let message = Message::assistant().with_tool_request(
            "next",
            Ok(CallToolRequestParams {
                task: None,
                meta: None,
                name: name.to_string().into(),
                arguments: args.as_object().cloned(),
            }),
        );
        let request = message.content[0].as_tool_request().unwrap().clone();
        inspector
            .inspect(
                &[request],
                messages,
                BioRouterMode::Approve,
                &crate::session::Session::default(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn changed_results_allow_more_than_the_old_call_ceiling() {
        let inspector = RepetitionInspector::staged(3, 5);
        let mut messages = Vec::new();
        for index in 0..250 {
            let start = messages.len();
            completed_call(
                &mut messages,
                "read_file",
                serde_json::json!({"path": "progress.log"}),
                ok_result(&format!("completed item {index}")),
            );
            assert!(tool_progress_since(&messages, start));
            assert!(inspect_next(
                &inspector,
                &messages,
                "read_file",
                serde_json::json!({"path": "progress.log"})
            )
            .await
            .is_empty());
        }
        assert_eq!(call_observations(&messages, LOOP_WINDOW).len(), LOOP_WINDOW);
    }

    #[tokio::test]
    async fn unchanged_successes_warn_then_stop() {
        let inspector = RepetitionInspector::staged(3, 5);
        let mut messages = Vec::new();
        for _ in 0..2 {
            completed_call(
                &mut messages,
                "read_file",
                serde_json::json!({"path": "main.rs"}),
                ok_result("unchanged"),
            );
        }
        let warning = inspect_next(
            &inspector,
            &messages,
            "read_file",
            serde_json::json!({"path": "main.rs"}),
        )
        .await;
        assert_eq!(warning[0].action, InspectionAction::Warn);
        for _ in 0..2 {
            completed_call(
                &mut messages,
                "read_file",
                serde_json::json!({"path": "main.rs"}),
                ok_result("unchanged"),
            );
        }
        let stopped = inspect_next(
            &inspector,
            &messages,
            "read_file",
            serde_json::json!({"path": "main.rs"}),
        )
        .await;
        assert_eq!(stopped[0].action, InspectionAction::Deny);
        assert!(!tool_progress_since(&messages, messages.len() - 1));
    }

    #[tokio::test]
    async fn unchanged_oscillation_stops_but_changing_results_do_not() {
        let inspector = RepetitionInspector::new(None).with_semantic(SemanticLoopConfig::default());
        let mut messages = Vec::new();
        for index in 0..8 {
            let name = if index % 2 == 0 { "inspect" } else { "check" };
            completed_call(
                &mut messages,
                name,
                serde_json::json!({"id": 1}),
                ok_result(name),
            );
        }
        let stopped = inspect_next(
            &inspector,
            &messages,
            "inspect",
            serde_json::json!({"id": 1}),
        )
        .await;
        assert_eq!(
            stopped[0].finding_id.as_deref(),
            Some(REPETITION_OSCILLATION_HARD_FINDING_ID)
        );
        completed_call(
            &mut messages,
            "check",
            serde_json::json!({"id": 1}),
            ok_result("new result"),
        );
        assert!(inspect_next(
            &inspector,
            &messages,
            "inspect",
            serde_json::json!({"id": 1})
        )
        .await
        .is_empty());
    }

    #[tokio::test]
    async fn identified_active_jobs_can_be_polled_without_a_turn_cutoff() {
        let inspector = RepetitionInspector::staged(3, 5);
        let mut messages = Vec::new();
        for _ in 0..250 {
            let mut result = ok_result("no new output").unwrap();
            result.structured_content =
                Some(serde_json::json!({"job_id": "build-1", "status": "running"}));
            completed_call(
                &mut messages,
                "job_status",
                serde_json::json!({"job_id": "build-1"}),
                Ok(result),
            );
        }
        assert!(inspect_next(
            &inspector,
            &messages,
            "job_status",
            serde_json::json!({"job_id": "build-1"})
        )
        .await
        .is_empty());
        assert!(tool_progress_since(&messages, messages.len() - 1));
        let shell = call(
            "developer__shell_status",
            serde_json::json!({"job_id": "shell-1"}),
        );
        assert!(active_job_result(&shell, &ok_result("job shell-1 [test], status: running (60s elapsed)\nnew output since last check:\n(no new output)").unwrap()));
        assert!(!active_job_result(
            &shell,
            &ok_result("job another [test], status: running (60s elapsed)").unwrap()
        ));
    }

    #[tokio::test]
    async fn running_prose_or_unidentified_status_does_not_disable_the_guards() {
        let inspector = RepetitionInspector::staged(3, 5);
        for text in [
            "the word running appears in this file",
            r#"{"status":"running"}"#,
        ] {
            let mut messages = Vec::new();
            for _ in 0..5 {
                completed_call(
                    &mut messages,
                    "read_file",
                    serde_json::json!({"path": "main.rs"}),
                    ok_result(text),
                );
            }
            let results = inspect_next(
                &inspector,
                &messages,
                "read_file",
                serde_json::json!({"path": "main.rs"}),
            )
            .await;
            assert_eq!(results[0].action, InspectionAction::Deny);
        }
    }

    #[tokio::test]
    async fn a_successful_repair_allows_retrying_a_previously_failing_tool() {
        let inspector =
            RepetitionInspector::new(None).with_failure_loop(FailureLoopConfig::default());
        let mut messages = Vec::new();
        for _ in 0..6 {
            completed_call(
                &mut messages,
                "shell",
                serde_json::json!({"command": "cargo build"}),
                error_result("cargo: command not found"),
            );
        }
        assert_eq!(
            inspect_next(
                &inspector,
                &messages,
                "shell",
                serde_json::json!({"command": "cargo build"})
            )
            .await[0]
                .action,
            InspectionAction::Deny
        );
        completed_call(
            &mut messages,
            "text_editor",
            serde_json::json!({"command": "replace", "path": "toolchain.env"}),
            ok_result("updated toolchain path"),
        );
        assert!(inspect_next(
            &inspector,
            &messages,
            "shell",
            serde_json::json!({"command": "cargo build"})
        )
        .await
        .is_empty());
    }

    #[tokio::test]
    async fn configured_thresholds_larger_than_the_default_window_are_respected() {
        let inspector = RepetitionInspector::new(Some(30));
        let mut messages = Vec::new();
        for _ in 0..30 {
            completed_call(
                &mut messages,
                "read_file",
                serde_json::json!({"path": "main.rs"}),
                ok_result("unchanged"),
            );
        }
        let results = inspect_next(
            &inspector,
            &messages,
            "read_file",
            serde_json::json!({"path": "main.rs"}),
        )
        .await;
        assert_eq!(results[0].action, InspectionAction::Deny);
    }

    #[tokio::test]
    async fn semantic_default_warning_has_a_recovery_window_before_stopping() {
        let inspector = RepetitionInspector::new(None).with_semantic(SemanticLoopConfig::default());
        let mut messages = Vec::new();
        let prefix = "same long search command argument ".repeat(8);
        for index in 0..8 {
            let args = serde_json::json!({"command": format!("{prefix}{index}")});
            let results = inspect_next(&inspector, &messages, "shell", args.clone()).await;
            if index >= 3 {
                assert_eq!(results[0].action, InspectionAction::Warn);
            } else {
                assert!(results.is_empty());
            }
            completed_call(&mut messages, "shell", args, ok_result("no matches"));
        }
        let args = serde_json::json!({"command": format!("{prefix}8")});
        let stopped = inspect_next(&inspector, &messages, "shell", args.clone()).await;
        assert_eq!(
            stopped[0].finding_id.as_deref(),
            Some(REPETITION_NEAR_DUP_HARD_FINDING_ID)
        );
        let warn_only = RepetitionInspector::new(None).with_semantic(SemanticLoopConfig {
            near_dup_hard_stop: None,
            ..SemanticLoopConfig::default()
        });
        assert_eq!(
            inspect_next(&warn_only, &messages, "shell", args).await[0].action,
            InspectionAction::Warn
        );
    }

    #[test]
    fn successful_pagination_is_progress_but_metadata_only_changes_are_not() {
        let mut messages = Vec::new();
        for page in 0..50 {
            let start = messages.len();
            completed_call(
                &mut messages,
                "list_pages",
                serde_json::json!({"page": page}),
                ok_result(&format!("page {page}")),
            );
            assert!(tool_progress_since(&messages, start));
        }
        let mut result = ok_result("unchanged").unwrap();
        result.structured_content = Some(serde_json::json!({"timestamp": "first"}));
        completed_call(
            &mut messages,
            "read",
            serde_json::json!({"id": 1}),
            Ok(result.clone()),
        );
        let start = messages.len();
        result.structured_content = Some(serde_json::json!({"timestamp": "second"}));
        completed_call(
            &mut messages,
            "read",
            serde_json::json!({"id": 1}),
            Ok(result),
        );
        assert!(!tool_progress_since(&messages, start));
        for timestamp in ["first", "second"] {
            let start = messages.len();
            completed_call(
                &mut messages,
                "read_json",
                serde_json::json!({"id": 1}),
                ok_result(
                    &serde_json::json!({"timestamp": timestamp, "value": "same"}).to_string(),
                ),
            );
            if timestamp == "second" {
                assert!(!tool_progress_since(&messages, start));
            }
        }
    }

    #[test]
    fn configured_failure_warnings_above_the_default_history_window_are_reachable() {
        let config = FailureLoopConfig {
            soft_warn_at: Some(30),
            escalate_at: Some(35),
            hard_stop_at: Some(40),
            ..FailureLoopConfig::default()
        };
        let mut messages = Vec::new();
        for count in 1..=250 {
            completed_call(
                &mut messages,
                "read_file",
                serde_json::json!({"path": "missing.rs"}),
                error_result("No such file or directory"),
            );
            let outcomes = tool_outcomes_since_last_progress(&messages, &config);
            assert_eq!(outcomes.len(), count.min(40));
            if count == 29 {
                assert!(failure_loop_nudge(&config, &outcomes, "read_file").is_none());
            } else if count == 30 {
                let warning = failure_loop_nudge(&config, &outcomes, "read_file").unwrap();
                assert!(warning.contains("failed 30 times"), "{warning}");
                assert!(!warning.contains("earlier warning"), "{warning}");
            } else if count == 40 {
                let warning = failure_loop_nudge(&config, &outcomes, "read_file").unwrap();
                assert!(warning.contains("failed 40 times"), "{warning}");
                assert!(warning.contains("earlier warning"), "{warning}");
                let inspector = RepetitionInspector::new(None).with_failure_loop(config.clone());
                assert!(inspector
                    .inspect_failure_loop("next", "read_file", &outcomes)
                    .is_some());
            }
        }
    }

    #[tokio::test]
    async fn retryable_failures_allow_transient_recovery_before_the_hard_stage() {
        let inspector =
            RepetitionInspector::new(None).with_failure_loop(FailureLoopConfig::default());
        let mut messages = Vec::new();
        for _ in 0..3 {
            completed_call(
                &mut messages,
                "fetch",
                serde_json::json!({"url": "https://example.test/data"}),
                error_result("HTTP 429 rate limit exceeded"),
            );
        }
        assert!(inspect_next(
            &inspector,
            &messages,
            "fetch",
            serde_json::json!({"url": "https://example.test/data"})
        )
        .await
        .is_empty());
        completed_call(
            &mut messages,
            "fetch",
            serde_json::json!({"url": "https://example.test/data"}),
            ok_result("new data"),
        );
        for _ in 0..3 {
            completed_call(
                &mut messages,
                "fetch",
                serde_json::json!({"url": "https://example.test/data"}),
                error_result("HTTP 429 rate limit exceeded"),
            );
        }
        assert!(inspect_next(
            &inspector,
            &messages,
            "fetch",
            serde_json::json!({"url": "https://example.test/data"})
        )
        .await
        .is_empty());
    }

    #[tokio::test]
    async fn unobserved_distinct_batch_calls_are_not_semantically_denied() {
        let inspector = RepetitionInspector::new(None).with_semantic(SemanticLoopConfig::default());
        let prefix = "read a long distinct-input search ".repeat(8);
        let requests = (0..20)
            .map(|index| {
                let message = Message::assistant().with_tool_request(
                    format!("batch-{index}"),
                    Ok(CallToolRequestParams {
                        task: None,
                        meta: None,
                        name: "search".into(),
                        arguments: Some(
                            serde_json::from_value(
                                serde_json::json!({"query": format!("{prefix}{index}")}),
                            )
                            .unwrap(),
                        ),
                    }),
                );
                message.content[0].as_tool_request().unwrap().clone()
            })
            .collect::<Vec<_>>();
        let results = inspector
            .inspect(
                &requests,
                &[],
                BioRouterMode::Approve,
                &crate::session::Session::default(),
            )
            .await
            .unwrap();
        assert!(results
            .iter()
            .all(|result| result.action == InspectionAction::Warn));
    }

    #[tokio::test]
    async fn a_configured_soft_warning_works_with_the_hard_stage_disabled() {
        let inspector = RepetitionInspector::staged(30, 0)
            .with_semantic(SemanticLoopConfig::disabled())
            .with_failure_loop(FailureLoopConfig::disabled());
        let mut messages = Vec::new();
        for _ in 0..29 {
            completed_call(
                &mut messages,
                "read_file",
                serde_json::json!({"path": "main.rs"}),
                ok_result("unchanged"),
            );
        }
        let results = inspect_next(
            &inspector,
            &messages,
            "read_file",
            serde_json::json!({"path": "main.rs"}),
        )
        .await;
        assert_eq!(results[0].action, InspectionAction::Warn);
        assert!(!results[0].reason.contains("stopped automatically"));
    }

    #[test]
    fn response_identity_detects_progress_after_a_history_rewrite() {
        let mut messages = Vec::new();
        for index in 0..30 {
            completed_call(
                &mut messages,
                "read",
                serde_json::json!({"id": index}),
                ok_result("old data"),
            );
        }
        let old_length = messages.len();
        completed_call(
            &mut messages,
            "read",
            serde_json::json!({"id": "new"}),
            ok_result("new data"),
        );
        let response_id = messages.last().unwrap().content[0]
            .as_tool_response()
            .unwrap()
            .id
            .clone();
        let response_ids = HashSet::from([response_id]);
        let rewritten = messages.split_off(old_length);
        assert!(!tool_progress_since(&rewritten, old_length));
        assert!(tool_progress_for_responses(&rewritten, &response_ids));
        assert!(!tool_progress_for_responses(
            &rewritten,
            &HashSet::from(["unrelated".to_string()])
        ));
    }
}
