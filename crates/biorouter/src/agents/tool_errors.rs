//! Structured tool-error taxonomy (BR-51).
//!
//! A failed tool call used to be an **unstructured `is_error` bool plus a text
//! blob**. `cargo build` failing because a file does not exist, because the
//! sandbox denied the write, because the network dropped, and because the model
//! passed a garbage argument all arrived at the model — and at every downstream
//! detector — as the same shapeless thing. Nothing could tell a retryable blip
//! from a hard failure, so the model retried what could never work and gave up
//! on what a single retry would have fixed, and the loop guards (BR-31/66)
//! counted every failure alike.
//!
//! This module is the classifier. It reduces any completed tool result to an
//! optional [`ToolError`] envelope:
//!
//! ```text
//! { kind, retryable, message, at: {file, line, column}? }
//! ```
//!
//! with a small, deliberately conservative [`ToolErrorKind`] taxonomy:
//! `not_found`, `permission_denied`, `timeout`, `invalid_args`, `transient`,
//! `tool_failure`, `internal`.
//!
//! ## Three sources, in priority order
//!
//! 1. **An envelope the tool emitted itself** — `structured_content.error` on an
//!    `is_error` result, or `ErrorData.data.error`. A tool that knows *why* it
//!    failed should say so, and its word beats any guess of ours.
//! 2. **The MCP/JSON-RPC error code** — `INVALID_PARAMS` really is invalid args,
//!    `RESOURCE_NOT_FOUND` really is not-found. `INTERNAL_ERROR` is the catch-all
//!    every server reaches for, so it carries no information and falls through.
//! 3. **The error text** — curated substrings (`ENOENT`, `permission denied`,
//!    `timed out`, `connection reset`, …). This is the graceful-degradation path
//!    for the opaque third-party MCP error, which is the common case.
//!
//! Anything we cannot place is [`ToolErrorKind::ToolFailure`]: *the tool ran and
//! reported a domain failure*. Not-retryable, no guessing. Degrading to a
//! truthful "something failed" is the whole point — a taxonomy that has to
//! invent a class for every unknown error is worse than none.
//!
//! ## What the model sees
//!
//! With `annotate` on (the default) the error text is prefixed with one line:
//!
//! ```text
//! [tool_error kind=not_found retryable=false at=src/main.rs:42] The target does not exist...
//! ```
//!
//! The original human-readable text follows untouched — the annotation is a
//! *header*, never a replacement. The header is also machine-readable, and
//! [`classify`] parses it back, so the classification survives persistence,
//! reload, and a provider round-trip even for providers that drop
//! `structured_content` (which is nearly all of them).
//!
//! Everything is config-gated: `BIOROUTER_TOOL_ERROR_TAXONOMY=false` restores
//! the pre-BR-51 behaviour exactly, and `BIOROUTER_TOOL_ERROR_ANNOTATION=false`
//! keeps the classification for the detectors while leaving the model's view of
//! the text byte-identical.

use once_cell::sync::Lazy;
use regex::Regex;
use rmcp::model::{CallToolResult, Content, ErrorCode, ErrorData};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::Config;
use crate::mcp_utils::ToolResult;

/// Key under which the envelope is attached to `structured_content` (on an
/// `is_error` result) and to `ErrorData.data` (on a transport/protocol error).
pub const ENVELOPE_KEY: &str = "error";

/// Opening token of the model-visible header line.
const ANNOTATION_OPEN: &str = "[tool_error ";

/// How a tool call failed.
///
/// The `retryable` split is the point of the whole taxonomy: only [`Transient`]
/// and [`Timeout`] can plausibly succeed on an identical retry. Everything else
/// needs the model to *change something* first.
///
/// [`Transient`]: ToolErrorKind::Transient
/// [`Timeout`]: ToolErrorKind::Timeout
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolErrorKind {
    /// The file / resource / record the call names does not exist.
    NotFound,
    /// The environment refused: filesystem permissions, a sandbox rule, a
    /// Biorouter permission decision, an auth failure at the far end.
    PermissionDenied,
    /// The call exceeded its deadline. Retryable: the far end may just be slow.
    Timeout,
    /// The arguments were wrong — schema violation, missing field, unparseable
    /// value. The *model* must fix the call.
    InvalidArgs,
    /// A blip in something the call depends on: network reset, 5xx, rate limit,
    /// a lock held elsewhere. Retryable.
    Transient,
    /// The tool ran and reported a domain failure (tests failed, the build broke,
    /// the query returned an error). The default when nothing else fits.
    ToolFailure,
    /// Biorouter or the server itself malfunctioned — protocol violation,
    /// serialization failure, a panic. Not the model's fault, and not fixable by
    /// repeating the call.
    Internal,
}

impl ToolErrorKind {
    /// The stable wire name (snake_case), as it appears in the header and the
    /// persisted envelope.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::PermissionDenied => "permission_denied",
            Self::Timeout => "timeout",
            Self::InvalidArgs => "invalid_args",
            Self::Transient => "transient",
            Self::ToolFailure => "tool_failure",
            Self::Internal => "internal",
        }
    }

    /// Parse a wire name, leniently — a tool emitting its own envelope may write
    /// `notFound`, `NOT_FOUND`, `not-found` or one of the obvious synonyms, and
    /// rejecting it would throw away the one authoritative source we have.
    pub fn parse(raw: &str) -> Option<Self> {
        let normalized: String = raw
            .trim()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        Some(match normalized.as_str() {
            "notfound" | "missing" | "nosuchfile" | "resourcenotfound" => Self::NotFound,
            "permissiondenied" | "permission" | "denied" | "forbidden" | "unauthorized"
            | "accessdenied" => Self::PermissionDenied,
            "timeout" | "timedout" | "deadlineexceeded" => Self::Timeout,
            "invalidargs" | "invalidarguments" | "invalidparams" | "invalidinput"
            | "validationerror" | "badrequest" => Self::InvalidArgs,
            "transient" | "temporary" | "unavailable" | "retryable" | "ratelimited"
            | "ratelimit" => Self::Transient,
            "toolfailure" | "failure" | "error" | "failed" => Self::ToolFailure,
            "internal" | "internalerror" | "bug" | "panic" => Self::Internal,
            _ => return None,
        })
    }

    /// Could the *identical* call plausibly succeed if it were issued again?
    ///
    /// Deliberately narrow. A not-found path stays not-found, a rejected argument
    /// stays rejected, a permission denial stays denied — retrying those is the
    /// blind-retry loop BR-31 exists to stop. Only a transient dependency failure
    /// and a timeout earn a second attempt.
    pub fn retryable(self) -> bool {
        matches!(self, Self::Transient | Self::Timeout)
    }

    /// One clause telling the model what this class of failure means for its next
    /// move. Short on purpose: it is prepended to every failed tool result, so it
    /// is paid for in tokens on the unhappy path of every turn.
    pub fn guidance(self) -> &'static str {
        match self {
            Self::NotFound => {
                "The target does not exist. Locate the right one before calling again."
            }
            Self::PermissionDenied => {
                "The environment refused this. Repeating it will be refused again; \
                 ask the user or take a permitted route."
            }
            Self::Timeout => {
                "The call exceeded its deadline. One retry (or a narrower request) is reasonable."
            }
            Self::InvalidArgs => {
                "The arguments were rejected. Fix the call itself; the same arguments will fail again."
            }
            Self::Transient => {
                "A dependency failed transiently. One retry is reasonable; do not keep hammering it."
            }
            Self::ToolFailure => {
                "The tool ran and reported a failure. Read the message and address its cause."
            }
            Self::Internal => {
                "Biorouter or the server malfunctioned. Repeating the call will not help; \
                 try another route or tell the user."
            }
        }
    }
}

/// Where in a file the failure points, when the error text says so (`file:line`
/// propagation — `verification.md` gap #4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorLocation {
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}

impl ErrorLocation {
    fn render(&self) -> String {
        match (self.line, self.column) {
            (Some(line), Some(col)) => format!("{}:{line}:{col}", self.file),
            (Some(line), None) => format!("{}:{line}", self.file),
            _ => self.file.clone(),
        }
    }
}

/// The typed envelope for one failed tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolError {
    pub kind: ToolErrorKind,
    /// Always `kind.retryable()` unless the tool's own envelope overrode it — a
    /// tool that says "this one *is* worth retrying" knows better than we do.
    pub retryable: bool,
    /// The human-readable message, unchanged. The taxonomy augments the text, it
    /// never replaces it.
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<ErrorLocation>,
}

/// Cap on the message stored in the error envelope.
///
/// The envelope is a *summary* that rides alongside the content, not a second
/// copy of it: it is persisted into the session and shipped over SSE on every
/// failing call. `error_text` joins every text block, and `developer__shell`
/// emits two near-identical blocks capped at 400 000 chars each, so an
/// unbounded envelope could persist ~800 KB of duplicated output per failed
/// command. The kind and the location are both extracted from the FULL message
/// before this cap applies, so nothing that is actually read gets lost.
const MAX_TOOL_ERROR_MESSAGE_CHARS: usize = 4096;

impl ToolError {
    pub(crate) fn new(kind: ToolErrorKind, message: impl Into<String>) -> Self {
        let message = message.into();
        let at = extract_location(&message);
        let message = if message.chars().count() > MAX_TOOL_ERROR_MESSAGE_CHARS {
            let kept: String = message.chars().take(MAX_TOOL_ERROR_MESSAGE_CHARS).collect();
            format!("{kept}\n[truncated: the full output is in the tool result above]")
        } else {
            message
        };
        Self {
            kind,
            retryable: kind.retryable(),
            message,
            at,
        }
    }

    /// The model-visible header line: machine-readable *and* directive.
    pub fn header(&self) -> String {
        let at = self
            .at
            .as_ref()
            .map(|loc| format!(" at={}", loc.render()))
            .unwrap_or_default();
        format!(
            "[tool_error kind={} retryable={}{at}] {}",
            self.kind.as_str(),
            self.retryable,
            self.kind.guidance()
        )
    }

    pub(crate) fn to_envelope(&self) -> Value {
        json!({ ENVELOPE_KEY: self })
    }
}

/// BR-51 policy, resolved once per reply (config reads hit the filesystem).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolErrorTaxonomyConfig {
    /// Master switch. Off = the exact pre-BR-51 behaviour: no classification, no
    /// envelope, no header.
    pub enabled: bool,
    /// Prefix the model-visible error text with the typed header. Off keeps the
    /// classification (so the detectors and the GUI still get it) while leaving
    /// what the model reads byte-identical to before.
    pub annotate: bool,
}

impl Default for ToolErrorTaxonomyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            annotate: true,
        }
    }
}

impl ToolErrorTaxonomyConfig {
    /// Inert (pre-BR-51 behaviour).
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            annotate: false,
        }
    }

    /// Classify, but do not touch the text the model sees.
    pub fn silent() -> Self {
        Self {
            enabled: true,
            annotate: false,
        }
    }

    /// Resolve from config. Keys:
    ///
    /// * `BIOROUTER_TOOL_ERROR_TAXONOMY` (bool) — master switch.
    /// * `BIOROUTER_TOOL_ERROR_ANNOTATION` (bool) — the model-visible header.
    pub fn from_config() -> Self {
        Self::from(Config::global())
    }

    pub fn from(config: &Config) -> Self {
        let defaults = Self::default();
        let enabled = config
            .get_param::<bool>("BIOROUTER_TOOL_ERROR_TAXONOMY")
            .unwrap_or(defaults.enabled);
        Self {
            enabled,
            annotate: enabled
                && config
                    .get_param::<bool>("BIOROUTER_TOOL_ERROR_ANNOTATION")
                    .unwrap_or(defaults.annotate),
        }
    }
}

/// Classify a completed tool result. `None` when the call succeeded.
///
/// Idempotent: a result that has already been annotated classifies back to the
/// same kind (the header is parsed first), so calling this on a reloaded
/// conversation gives the same answer as calling it the moment the tool returned.
pub fn classify(result: &ToolResult<CallToolResult>) -> Option<ToolError> {
    match result {
        Err(error) => Some(classify_error_data(error)),
        Ok(call) => classify_call_result(call),
    }
}

#[derive(Debug, Clone)]
pub struct NestedToolFailure {
    pub tool_name: String,
    pub arguments: Option<String>,
    pub error: ToolError,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NestedFailureProvenance {
    kind: ToolErrorKind,
    retryable: bool,
}

/// Supplemental failures that a successful script caught, authenticated by its request.
/// Uncaught failures already have an outer error; job outcomes have their own census.
pub fn nested_tool_failures(
    tool_name: &str,
    result: &ToolResult<CallToolResult>,
) -> Vec<NestedToolFailure> {
    if !super::code_execution_extension::is_execute_code_call(tool_name) {
        return Vec::new();
    }
    let Ok(call) = result else {
        return Vec::new();
    };
    if call.is_error == Some(true) {
        return Vec::new();
    }
    let Some(records) = call
        .meta
        .as_ref()
        .and_then(|meta| meta.get("biorouter/tool-calls"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    records
        .iter()
        .filter(|record| record.get("status").and_then(Value::as_str) == Some("error"))
        .filter(|record| {
            record
                .get("operation")
                .and_then(super::operation_outcome::OperationOutcome::from_value)
                .is_none_or(|operation| operation.failure().is_none())
        })
        .filter_map(|record| {
            let name = record.get("tool")?.as_str()?.trim();
            if name.is_empty() {
                return None;
            }
            let message = record
                .get("error")
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .unwrap_or("Nested tool failed (details unavailable).");
            let mut error = match record.get("failure") {
                Some(value) => {
                    let provenance =
                        serde_json::from_value::<NestedFailureProvenance>(value.clone());
                    let mut error = ToolError::new(ToolErrorKind::ToolFailure, message);
                    if let Ok(provenance) = provenance {
                        error.kind = provenance.kind;
                        error.retryable = provenance.retryable;
                    }
                    error
                }
                None => classify_text(message),
            };
            error.message = ToolError::new(error.kind, message).message;
            Some(NestedToolFailure {
                tool_name: name.chars().take(256).collect(),
                arguments: record
                    .get("args")
                    .and_then(Value::as_str)
                    .map(|args| args.chars().take(2048).collect()),
                error,
            })
        })
        .collect()
}

/// Classify a borrowed completed call without copying its output or artifacts.
pub(crate) fn classify_call_result(call: &CallToolResult) -> Option<ToolError> {
    if call.is_error != Some(true) {
        return None;
    }
    let text = error_text(call);
    let from_tool = call
        .structured_content
        .as_ref()
        .and_then(|sc| envelope_from_value(sc, &text));
    Some(from_tool.unwrap_or_else(|| classify_text(&text)))
}

/// Classify a transport/protocol error (the `Err` arm of a tool result).
pub fn classify_error_data(error: &ErrorData) -> ToolError {
    let message = error.message.to_string();
    if let Some(envelope) = error
        .data
        .as_ref()
        .and_then(|data| envelope_from_value(data, &message))
    {
        return envelope;
    }
    if let Some(kind) = kind_from_code(error.code) {
        return ToolError::new(kind, message);
    }
    classify_text(&message)
}

/// Classify from text alone: our own header if present, else the curated
/// substrings, else [`ToolErrorKind::ToolFailure`].
pub fn classify_text(text: &str) -> ToolError {
    if let Some(kind) = kind_from_annotation(text) {
        // Preserve the tool's own claim about retryability if the header carried
        // one that disagrees with the kind's default (a tool-emitted envelope may).
        let retryable = annotation_retryable(text).unwrap_or_else(|| kind.retryable());
        let mut error = ToolError::new(kind, text);
        error.retryable = retryable;
        return error;
    }
    ToolError::new(kind_from_text(text), text)
}

/// The MCP/JSON-RPC codes that actually carry information. `INTERNAL_ERROR` is
/// the universal catch-all (every `anyhow` bubble-up lands there), so it is
/// deliberately *not* mapped — it falls through to the text heuristics.
fn kind_from_code(code: ErrorCode) -> Option<ToolErrorKind> {
    match code {
        ErrorCode::RESOURCE_NOT_FOUND | ErrorCode::METHOD_NOT_FOUND => {
            Some(ToolErrorKind::NotFound)
        }
        ErrorCode::INVALID_PARAMS | ErrorCode::INVALID_REQUEST => Some(ToolErrorKind::InvalidArgs),
        ErrorCode::PARSE_ERROR => Some(ToolErrorKind::Internal),
        _ => None,
    }
}

const TEXT_KIND_PATTERNS: &[(ToolErrorKind, &[&str])] = &[
    (
        ToolErrorKind::Timeout,
        &["timed out", "timeout", "deadline exceeded", "etimedout"],
    ),
    (
        ToolErrorKind::PermissionDenied,
        &[
            "permission denied",
            "access denied",
            "access is denied",
            "eacces",
            "eperm",
            "operation not permitted",
            "not authorized",
            "unauthorized",
            "forbidden",
            "403",
            "401",
            "read-only file system",
            "declined to run this tool",
            "did not run this tool",
        ],
    ),
    (
        ToolErrorKind::NotFound,
        &[
            "no such file",
            "no such directory",
            "not found",
            "does not exist",
            "doesn't exist",
            "enoent",
            "cannot find",
            "unknown tool",
            "404",
        ],
    ),
    (
        ToolErrorKind::Transient,
        &[
            "connection reset",
            "connection refused",
            "connection closed",
            "broken pipe",
            "temporarily unavailable",
            "service unavailable",
            "try again later",
            "rate limit",
            "too many requests",
            "429",
            "502",
            "503",
            "504",
            "econnreset",
            "econnrefused",
            "network is unreachable",
            "dns",
            "resource busy",
            "would block",
        ],
    ),
    (
        ToolErrorKind::InvalidArgs,
        &[
            "invalid params",
            "invalid parameter",
            "invalid argument",
            "invalid input",
            "missing required",
            "required property",
            "unknown field",
            "failed to parse arguments",
            "schema validation",
            "is not of type",
            "expected object",
            "expected string",
        ],
    ),
    (
        ToolErrorKind::Internal,
        &[
            "panicked at",
            "internal error",
            "protocol error",
            "serialization failed",
            "tool result validation failed",
        ],
    ),
];

/// Curated substring heuristics over the error text — the graceful-degradation
/// path for an opaque third-party MCP error, which is the common case.
///
/// Order matters and is load-bearing: a timeout mentions the network, a 403
/// mentions "not allowed", and "no such file" beats a generic "failed". Each
/// group is checked most-specific first, and anything unmatched stays
/// [`ToolErrorKind::ToolFailure`] rather than being forced into a class.
pub fn kind_from_text(text: &str) -> ToolErrorKind {
    let haystack = text.to_ascii_lowercase();

    for (kind, patterns) in TEXT_KIND_PATTERNS {
        if patterns.iter().any(|needle| haystack.contains(needle)) {
            return *kind;
        }
    }

    ToolErrorKind::ToolFailure
}

/// Read an envelope a tool emitted itself, from `structured_content` or
/// `ErrorData.data`. Accepts both `{"error": {"kind": …}}` and a bare
/// `{"kind": …}`, since MCP servers write both.
fn envelope_from_value(value: &Value, fallback_message: &str) -> Option<ToolError> {
    let body = value.get(ENVELOPE_KEY).unwrap_or(value);
    let kind = ToolErrorKind::parse(body.get("kind")?.as_str()?)?;
    let message = body
        .get("message")
        .and_then(Value::as_str)
        .filter(|m| !m.trim().is_empty())
        .unwrap_or(fallback_message)
        .to_string();
    let retryable = body
        .get("retryable")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| kind.retryable());
    let at = body
        .get("at")
        .and_then(|at| serde_json::from_value::<ErrorLocation>(at.clone()).ok())
        .or_else(|| extract_location(&message));
    Some(ToolError {
        kind,
        retryable,
        message,
        at,
    })
}

/// Parse our own header back out of an already-annotated text.
fn kind_from_annotation(text: &str) -> Option<ToolErrorKind> {
    let rest = text.strip_prefix(ANNOTATION_OPEN)?;
    let head = rest.split(']').next()?;
    let kind = head
        .split_whitespace()
        .find_map(|token| token.strip_prefix("kind="))?;
    ToolErrorKind::parse(kind)
}

fn annotation_retryable(text: &str) -> Option<bool> {
    let rest = text.strip_prefix(ANNOTATION_OPEN)?;
    let head = rest.split(']').next()?;
    head.split_whitespace()
        .find_map(|token| token.strip_prefix("retryable="))
        .and_then(|value| value.parse::<bool>().ok())
}

/// Is this text already carrying a header? Annotation must be idempotent: the
/// same result can pass through the seam more than once (a PostToolUse block
/// rewrites it in place).
pub fn is_annotated(text: &str) -> bool {
    kind_from_annotation(text).is_some()
}

// `src/main.rs:42:7`, `./tests/foo.py:9`, `C:\src\x.ts:3:1`.
static PATH_LINE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)([A-Za-z]:[\\/][^\s:]+|[\w./\\+-]*[\w+-]\.[A-Za-z][A-Za-z0-9]{0,9}):(\d{1,7})(?::(\d{1,7}))?")
        .expect("valid location regex")
});
// Python: `File "foo.py", line 12`.
static PY_TRACEBACK: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"File "([^"]+)", line (\d+)"#).expect("valid traceback regex"));

/// Pull the first `file:line[:col]` the error names, so a compiler/linter failure
/// arrives with a location the model can act on instead of only prose.
pub fn extract_location(text: &str) -> Option<ErrorLocation> {
    if let Some(caps) = PY_TRACEBACK.captures(text) {
        return Some(ErrorLocation {
            file: caps.get(1)?.as_str().to_string(),
            line: caps.get(2)?.as_str().parse().ok(),
            column: None,
        });
    }
    let caps = PATH_LINE.captures(text)?;
    Some(ErrorLocation {
        file: caps.get(1)?.as_str().to_string(),
        line: caps.get(2).and_then(|m| m.as_str().parse().ok()),
        column: caps.get(3).and_then(|m| m.as_str().parse().ok()),
    })
}

/// Concatenated text content of an `is_error` result.
fn error_text(call: &CallToolResult) -> String {
    let text = call
        .content
        .iter()
        .filter_map(|content| content.as_text().map(|t| t.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        "tool reported an error with no message".to_string()
    } else {
        text
    }
}

/// The BR-51 seam: classify a completed tool result and, when the policy says so,
/// attach the envelope (machine-readable, for the GUI and for a reload) and the
/// header (model-readable, so the model itself can branch).
///
/// Returns the (possibly rewritten) result plus the classification, which the
/// caller hands to the loop detectors. A successful call is a zero-cost
/// pass-through, as is the whole function with the taxonomy disabled.
pub fn annotate_tool_result(
    output: ToolResult<CallToolResult>,
    config: ToolErrorTaxonomyConfig,
) -> (ToolResult<CallToolResult>, Option<ToolError>) {
    if !config.enabled {
        return (output, None);
    }
    let Some(error) = classify(&output) else {
        return (output, None);
    };

    let output = match output {
        Ok(mut call) => {
            // Machine-readable: leave a tool's own structured payload alone, only
            // fill in the envelope when it did not provide one (and never
            // overwrite a non-object payload a tool chose to return).
            match call.structured_content.as_mut() {
                Some(Value::Object(existing)) => {
                    if !existing.contains_key(ENVELOPE_KEY) {
                        if let Ok(value) = serde_json::to_value(&error) {
                            existing.insert(ENVELOPE_KEY.to_string(), value);
                        }
                    }
                }
                Some(_) => {}
                None => call.structured_content = Some(error.to_envelope()),
            }
            if config.annotate {
                call.content = annotated_content(call.content, &error);
            }
            Ok(call)
        }
        Err(mut data) => {
            if data.data.is_none() {
                data.data = Some(error.to_envelope());
            }
            if config.annotate && !is_annotated(&data.message) {
                data.message = format!("{}\n{}", error.header(), data.message).into();
            }
            Err(data)
        }
    };

    (output, Some(error))
}

/// Prefix the first text block with the header, leaving every other content item
/// (images, resources, further text) untouched. A result with no text block at
/// all gets the header as its only text.
fn annotated_content(content: Vec<Content>, error: &ToolError) -> Vec<Content> {
    let mut out = Vec::with_capacity(content.len() + 1);
    let mut done = false;
    for item in content {
        match item.as_text() {
            Some(text) if !done => {
                done = true;
                if is_annotated(&text.text) {
                    out.push(item);
                } else {
                    // Carry the original annotations onto the rebuilt block.
                    // `Content::text` produces `annotations: None`, and an
                    // audience-less block is treated as user-visible by every
                    // consumer (ToolCallWithResponse.tsx checks
                    // `!annotations?.audience || …includes('user')`; the CLI TUI
                    // does the same). Dropping them therefore PROMOTES an
                    // assistant-only block into the user's transcript — for
                    // `developer__shell` that is the internal "private note:
                    // output was N lines … remainder in /var/folders/…/T/.tmpX
                    // do not show tmp file to user" block, leaking the note and
                    // an absolute temp path; for output under the truncation
                    // threshold the two blocks are identical, so the failure
                    // simply rendered twice.
                    let mut rebuilt = Content::text(format!("{}\n{}", error.header(), text.text));
                    rebuilt.annotations = item.annotations.clone();
                    out.push(rebuilt);
                }
            }
            _ => out.push(item),
        }
    }
    if !done {
        out.insert(0, Content::text(error.header()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    fn err(message: &str) -> ToolResult<CallToolResult> {
        Err(ErrorData {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(message.to_string()),
            data: None,
        })
    }

    fn tool_error(message: &str) -> ToolResult<CallToolResult> {
        Ok(CallToolResult {
            content: vec![Content::text(message)],
            structured_content: None,
            is_error: Some(true),
            meta: None,
        })
    }

    /// The annotator must not PROMOTE an assistant-only block into the user's
    /// transcript. Every consumer treats an audience-less block as user-visible,
    /// so rebuilding the first text block without carrying its annotations
    /// across silently publishes it. `developer__shell` marks its full,
    /// untruncated output assistant-only — it carries an internal note and an
    /// absolute temp path — and only became reachable here once shell started
    /// setting `is_error`, which is what gates classification.
    #[test]
    fn annotating_preserves_an_assistant_only_audience() {
        use rmcp::model::Role;

        let result = Ok(CallToolResult {
            content: vec![
                Content::text("private note: output was 412 lines … /tmp/.tmpAb3kQ")
                    .with_audience(vec![Role::Assistant]),
                Content::text("the last 100 lines").with_audience(vec![Role::User]),
            ],
            structured_content: None,
            is_error: Some(true),
            meta: None,
        });

        let (out, error) = annotate_tool_result(result, ToolErrorTaxonomyConfig::default());
        assert!(error.is_some(), "an is_error result must classify");
        let content = out.unwrap().content;

        assert_eq!(
            content[0]
                .annotations
                .as_ref()
                .and_then(|a| a.audience.clone()),
            Some(vec![Role::Assistant]),
            "the annotated block must stay assistant-only, or the private note reaches the user"
        );
        assert_eq!(
            content[1]
                .annotations
                .as_ref()
                .and_then(|a| a.audience.clone()),
            Some(vec![Role::User]),
            "untouched blocks keep their audience"
        );
        assert!(
            content[0].as_text().unwrap().text.contains("private note"),
            "the header is prefixed onto the original text, not replacing it"
        );
    }

    #[test]
    fn success_is_not_classified() {
        let ok = Ok(CallToolResult::success(vec![Content::text("all good")]));
        assert!(classify(&ok).is_none());
        let (out, error) = annotate_tool_result(ok, ToolErrorTaxonomyConfig::default());
        assert!(error.is_none());
        assert_eq!(out.unwrap().content[0].as_text().unwrap().text, "all good");
    }

    #[test]
    fn text_heuristics_cover_every_kind() {
        let cases = [
            (
                "No such file or directory (os error 2)",
                ToolErrorKind::NotFound,
            ),
            (
                "Permission denied (os error 13)",
                ToolErrorKind::PermissionDenied,
            ),
            ("operation timed out after 30s", ToolErrorKind::Timeout),
            (
                "invalid params: missing required field 'path'",
                ToolErrorKind::InvalidArgs,
            ),
            ("connection reset by peer", ToolErrorKind::Transient),
            ("HTTP 503 Service Unavailable", ToolErrorKind::Transient),
            (
                "thread 'main' panicked at src/lib.rs:10:1",
                ToolErrorKind::Internal,
            ),
            ("2 tests failed", ToolErrorKind::ToolFailure),
        ];
        for (text, expected) in cases {
            assert_eq!(kind_from_text(text), expected, "{text}");
        }
    }

    /// The whole point of the split: a blip is retryable, a hard failure is not.
    #[test]
    fn only_transient_and_timeout_are_retryable() {
        assert!(ToolErrorKind::Transient.retryable());
        assert!(ToolErrorKind::Timeout.retryable());
        for kind in [
            ToolErrorKind::NotFound,
            ToolErrorKind::PermissionDenied,
            ToolErrorKind::InvalidArgs,
            ToolErrorKind::ToolFailure,
            ToolErrorKind::Internal,
        ] {
            assert!(!kind.retryable(), "{kind:?}");
        }
    }

    /// An opaque third-party error must degrade to an honest "the tool failed",
    /// never to an invented class.
    #[test]
    fn an_opaque_error_degrades_to_tool_failure() {
        let error = classify(&err("Something went wrong in the widget")).unwrap();
        assert_eq!(error.kind, ToolErrorKind::ToolFailure);
        assert!(!error.retryable);
        assert!(error.message.contains("widget"));
    }

    #[test]
    fn error_codes_beat_text_when_they_carry_information() {
        let invalid = Err(ErrorData {
            code: ErrorCode::INVALID_PARAMS,
            // Text alone would read as a plain tool failure.
            message: Cow::from("the widget was unhappy"),
            data: None,
        });
        assert_eq!(classify(&invalid).unwrap().kind, ToolErrorKind::InvalidArgs);

        // INTERNAL_ERROR is the universal catch-all: it must not shadow the text.
        assert_eq!(
            classify(&err("connection refused")).unwrap().kind,
            ToolErrorKind::Transient
        );
    }

    /// A tool that knows why it failed outranks our guessing.
    #[test]
    fn a_tool_emitted_envelope_wins() {
        let result = Ok(CallToolResult {
            // The text alone would classify as not_found.
            content: vec![Content::text("record not found")],
            structured_content: Some(json!({
                "error": {"kind": "transient", "retryable": true, "message": "upstream flaked"}
            })),
            is_error: Some(true),
            meta: None,
        });
        let error = classify(&result).unwrap();
        assert_eq!(error.kind, ToolErrorKind::Transient);
        assert!(error.retryable);
        assert_eq!(error.message, "upstream flaked");
    }

    #[test]
    fn annotation_prefixes_the_text_and_keeps_the_original() {
        let (out, error) = annotate_tool_result(
            tool_error("No such file or directory: /tmp/nope"),
            ToolErrorTaxonomyConfig::default(),
        );
        let call = out.unwrap();
        let text = call.content[0].as_text().unwrap().text.clone();
        assert!(
            text.starts_with("[tool_error kind=not_found retryable=false]"),
            "{text}"
        );
        assert!(text.contains("No such file or directory: /tmp/nope"));
        assert_eq!(error.unwrap().kind, ToolErrorKind::NotFound);
        // The envelope is attached for machine consumers too.
        let envelope = call.structured_content.unwrap();
        assert_eq!(envelope["error"]["kind"], "not_found");
    }

    /// Reading a persisted conversation must give the same classification the
    /// live turn had: the header is parsed back rather than re-guessed.
    #[test]
    fn classification_is_idempotent_through_the_annotation() {
        let config = ToolErrorTaxonomyConfig::default();
        let (once, first) = annotate_tool_result(tool_error("connection reset by peer"), config);
        let (twice, second) = annotate_tool_result(once, config);
        assert_eq!(
            first.as_ref().map(|e| e.kind),
            Some(ToolErrorKind::Transient)
        );
        assert_eq!(
            second.as_ref().map(|e| e.kind),
            Some(ToolErrorKind::Transient)
        );
        assert!(second.unwrap().retryable);
        // And the header was not stacked twice.
        let text = twice.unwrap().content[0].as_text().unwrap().text.clone();
        assert_eq!(text.matches("[tool_error").count(), 1, "{text}");
    }

    #[test]
    fn silent_mode_classifies_without_touching_what_the_model_reads() {
        let (out, error) = annotate_tool_result(
            tool_error("Permission denied"),
            ToolErrorTaxonomyConfig::silent(),
        );
        assert_eq!(error.unwrap().kind, ToolErrorKind::PermissionDenied);
        assert_eq!(
            out.unwrap().content[0].as_text().unwrap().text,
            "Permission denied"
        );
    }

    #[test]
    fn disabled_is_an_exact_pass_through() {
        let (out, error) = annotate_tool_result(
            tool_error("Permission denied"),
            ToolErrorTaxonomyConfig::disabled(),
        );
        assert!(error.is_none());
        let call = out.unwrap();
        assert_eq!(call.content[0].as_text().unwrap().text, "Permission denied");
        assert!(call.structured_content.is_none());
    }

    #[test]
    fn locations_are_lifted_out_of_the_error_text() {
        let rust =
            extract_location("error[E0308]: mismatched types\n --> src/main.rs:42:17").unwrap();
        assert_eq!(rust.file, "src/main.rs");
        assert_eq!(rust.line, Some(42));
        assert_eq!(rust.column, Some(17));

        let python = extract_location("File \"scripts/run.py\", line 12, in <module>").unwrap();
        assert_eq!(python.file, "scripts/run.py");
        assert_eq!(python.line, Some(12));

        assert!(extract_location("everything is fine").is_none());
    }

    #[test]
    fn the_header_carries_the_location() {
        let error = ToolError::new(ToolErrorKind::ToolFailure, "boom at src/lib.rs:7:3");
        assert!(
            error.header().contains("at=src/lib.rs:7:3"),
            "{}",
            error.header()
        );
    }

    #[test]
    fn a_transport_error_is_annotated_in_place() {
        let (out, error) = annotate_tool_result(
            err("operation timed out"),
            ToolErrorTaxonomyConfig::default(),
        );
        assert!(error.unwrap().retryable);
        let data = out.unwrap_err();
        assert!(data
            .message
            .starts_with("[tool_error kind=timeout retryable=true]"));
        assert!(data.message.contains("operation timed out"));
        assert_eq!(data.data.unwrap()["error"]["kind"], "timeout");
    }

    #[test]
    fn a_result_with_no_text_still_gets_a_header() {
        let (out, _) = annotate_tool_result(
            Ok(CallToolResult {
                content: vec![],
                structured_content: None,
                is_error: Some(true),
                meta: None,
            }),
            ToolErrorTaxonomyConfig::default(),
        );
        let call = out.unwrap();
        assert!(call.content[0]
            .as_text()
            .unwrap()
            .text
            .starts_with("[tool_error"));
    }

    #[test]
    fn kinds_round_trip_through_their_wire_names() {
        for kind in [
            ToolErrorKind::NotFound,
            ToolErrorKind::PermissionDenied,
            ToolErrorKind::Timeout,
            ToolErrorKind::InvalidArgs,
            ToolErrorKind::Transient,
            ToolErrorKind::ToolFailure,
            ToolErrorKind::Internal,
        ] {
            assert_eq!(ToolErrorKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(
            ToolErrorKind::parse("NOT-FOUND"),
            Some(ToolErrorKind::NotFound)
        );
        assert_eq!(ToolErrorKind::parse("wat"), None);
    }
}
