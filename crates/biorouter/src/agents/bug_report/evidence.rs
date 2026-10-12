//! What actually went wrong, read out of the session itself.
//!
//! ## There is nothing to read but the transcript
//!
//! Biorouter keeps no failures table, no error index and no persisted failure
//! record. `tool_monitor`'s `ToolOutcome` is in-memory and per-turn, so by the
//! time anyone says "report a bug" it is gone. The one durable account of a
//! failed call is the tool response in `messages.content_json`, which is why
//! this module reads the [`Conversation`] and nothing else.
//!
//! ## Two spellings of failure, and only one of them looks like one
//!
//! ```text
//! {"type":"toolResponse","toolResult":{"status":"error","error":"…","error_kind":"not_found"}}
//! {"type":"toolResponse","toolResult":{"status":"success","value":{"content":[…],"isError":true}}}
//! ```
//!
//! The first is a transport-level failure: the call never produced a result.
//! The second is a tool that ran and reported a domain failure — a build that
//! broke, a query that errored — and it is by far the more common one. A scan
//! that tests only `status == "error"` sees a clean session and reports
//! nothing, which is the shape of "the tool says it found no problems" while
//! the user is looking at a red error in the transcript.
//!
//! Both are exactly [`ToolResult::Err`] and `Ok(r) if r.is_error == Some(true)`
//! in memory, which is what [`tool_errors::classify`] already distinguishes —
//! so the taxonomy is taken from there rather than re-derived, and a failure
//! keeps the `kind`/`retryable` grading BR-51 gave it.

use std::collections::HashMap;

use crate::agents::tool_errors::{self, ToolError, ToolErrorKind};
use crate::conversation::message::{Message, MessageContent};
use crate::conversation::Conversation;
use crate::session::session_manager::Session;

/// How much of a failure message is kept. Long enough for a stack trace's first
/// frames, short enough that ten of them still fit in an issue body.
const MESSAGE_CLIP_CHARS: usize = 800;

/// How many distinct failures are carried into the report.
const MAX_FAILURES: usize = 12;

/// How much of the user's own prose is carried, per message.
const USER_CLIP_CHARS: usize = 400;

/// The last N user messages, so the report can say what was being attempted.
const USER_MESSAGES_KEPT: usize = 4;

/// One failed tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolFailure {
    /// The tool as the model called it, e.g. `developer__shell`. `None` when the
    /// response has no matching request in the transcript — which happens on a
    /// conversation truncated by compaction, and is worth saying rather than
    /// guessing at.
    pub tool_name: Option<String>,
    /// BR-51's grading.
    pub kind: ToolErrorKind,
    pub retryable: bool,
    /// The failure text, clipped.
    pub message: String,
    /// How many times this same (tool, kind, message) failed. A tool that failed
    /// nine times identically is one bug, not nine.
    pub occurrences: usize,
    /// The arguments the call was made with, clipped and pretty-printed.
    /// Frequently the actual bug — a path that does not exist, a flag that is
    /// not supported.
    pub arguments: Option<String>,
    /// The failure looks like Biorouter refusing **on purpose** — a privacy
    /// boundary, a permission decision — rather than something breaking.
    ///
    /// ⚠ Load-bearing, and discovered from a real bundle. The only failed call
    /// in the one real session read while building this was Gate C refusing
    /// `workspace_read_conversation` on a public model, and every coarse signal
    /// says "hard bug": not retryable, `ToolFailure` kind, a long error string.
    /// Filing it would be filing "the security boundary worked" as a defect.
    ///
    /// Detected from constants `privacy::refusal` exports and this module
    /// imports, so a reword there is a compile error rather than a silent
    /// mis-grading. It is a HINT, not a filter: the failure is still reported
    /// to the model, labelled, because a refusal genuinely can be a bug (the
    /// wrong thing refused) and only the model can read which.
    pub looks_deliberate: bool,
}

impl ToolFailure {
    /// The line this failure contributes to the issue body.
    pub fn to_line(&self) -> String {
        let name = self.tool_name.as_deref().unwrap_or("(unmatched response)");
        let repeat = if self.occurrences > 1 {
            format!(" ×{}", self.occurrences)
        } else {
            String::new()
        };
        let deliberate = if self.looks_deliberate {
            " [looks like a deliberate refusal, not a malfunction]"
        } else {
            ""
        };
        format!(
            "- `{name}`{repeat} — {} ({}){deliberate}: {}",
            self.kind.as_str(),
            if self.retryable {
                "retryable"
            } else {
                "not retryable"
            },
            self.message
        )
    }
}

/// Everything the report is written from.
#[derive(Debug, Clone)]
pub struct Evidence {
    pub session_id: String,
    /// Distinct failures, most frequent first, then in transcript order.
    pub failures: Vec<ToolFailure>,
    /// Total failed calls, before de-duplication.
    pub total_failed_calls: usize,
    /// Total tool calls in the session, so "3 of 4 calls failed" is available.
    pub total_tool_calls: usize,
    /// The user's own recent prose, clipped.
    pub recent_user_messages: Vec<String>,
    /// Tool output that was externalized and is not in the transcript.
    /// Present only under `BIOROUTER_SESSION_BLOB_LAZY_LOAD`; a report that
    /// silently omitted it would understate what the model could not see.
    pub externalized_results: usize,
    pub app_version: String,
    pub os: String,
    pub os_version: String,
    pub architecture: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub enabled_extensions: Vec<String>,
    /// The working directory, scrubbed by the caller before it is rendered.
    pub working_dir: String,
}

impl Evidence {
    /// Is there enough here to write a report from without the user saying
    /// anything?
    ///
    /// ⚠ The bar is deliberately not "a failure exists". A single retryable
    /// `transient` — one 429, one connection reset — is the ordinary weather of
    /// a long session and is not what anyone means by "report a bug"; filing on
    /// it produces noise that a maintainer has to close. What counts is a
    /// failure that is *not* the model's own fault to fix and *not* expected to
    /// clear on its own.
    pub fn is_conclusive(&self) -> bool {
        self.failures.iter().any(|failure| {
            !failure.retryable
                && !failure.looks_deliberate
                && !matches!(
                    failure.kind,
                    // The model passed bad arguments. That is a model mistake,
                    // not a Biorouter defect, and the model is the one thing in
                    // the loop that can already see it.
                    //
                    // `PermissionDenied` joins it for the same reason: the
                    // taxonomy defines that class as "the environment refused",
                    // which includes a Biorouter permission decision. A control
                    // doing its job is not a bug report.
                    ToolErrorKind::InvalidArgs | ToolErrorKind::PermissionDenied
                )
        }) || self
            .failures
            .iter()
            .any(|failure| failure.occurrences >= 3 && !failure.looks_deliberate)
    }

    /// The one-line diagnosis, when there is one.
    pub fn headline(&self) -> Option<String> {
        let worst = self
            .failures
            .iter()
            .max_by_key(|f| (usize::from(!f.retryable), f.occurrences))?;
        let name = worst.tool_name.as_deref().unwrap_or("a tool call");
        Some(format!(
            "{name} failed with {} — {}",
            worst.kind.as_str(),
            first_line(&worst.message)
        ))
    }
}

/// Does this failure text read as Biorouter refusing on purpose?
///
/// Keyed on constants the privacy layer EXPORTS, not on prose copied from it:
/// `ASK_THE_USER_TO_SWITCH` is composed into every gate refusal that a model
/// could clear by changing the chat's model, and `TURN_REFUSAL_MARKER` marks
/// the turn barrier. A reword on that side moves the constant and this keeps
/// working; a rename breaks the build here, which is the point.
///
/// It is deliberately narrow. Missing a refusal costs an over-eager push-back
/// and an unlabelled line — both safe. Claiming a real malfunction is deliberate
/// would suppress the report, so the check errs the other way.
fn looks_deliberate(text: &str) -> bool {
    use crate::privacy::refusal::{ASK_THE_USER_TO_SWITCH, TURN_REFUSAL_MARKER};
    text.contains(ASK_THE_USER_TO_SWITCH) || text.contains(TURN_REFUSAL_MARKER)
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or(text).trim()
}

fn clip(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }
    let kept: String = trimmed.chars().take(limit).collect();
    format!("{kept}… (clipped)")
}

/// The text of a tool response, for the failure message.
fn response_text(result: &crate::mcp_utils::ToolResult<rmcp::model::CallToolResult>) -> String {
    match result {
        Err(error) => error.message.to_string(),
        Ok(call) => call
            .content
            .iter()
            .filter_map(|content| content.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// Read the failures out of a conversation.
///
/// Exposed separately from [`collect`] so it can be tested against a real
/// exported session with no `Session`, `Config` or filesystem in scope.
pub fn failures_in(conversation: &Conversation) -> (Vec<ToolFailure>, usize, usize, usize) {
    // Request id -> (tool name, arguments). Built first: a response carries the
    // id and nothing else, and the id of a call is the only thing that names it.
    let mut calls: HashMap<String, (String, Option<String>)> = HashMap::new();
    for message in conversation.messages() {
        for content in &message.content {
            if let MessageContent::ToolRequest(request) = content {
                if let Ok(call) = &request.tool_call {
                    let arguments = call.arguments.as_ref().and_then(|args| {
                        serde_json::to_string(args)
                            .ok()
                            .filter(|rendered| rendered != "{}")
                    });
                    calls.insert(request.id.clone(), (call.name.to_string(), arguments));
                }
            }
        }
    }

    // Keyed on what makes two failures the SAME failure: the tool, the grade and
    // the message. Not the request id, which is unique per call and would report
    // one loop of nine identical failures as nine separate bugs.
    let mut seen: HashMap<(Option<String>, ToolErrorKind, String), usize> = HashMap::new();
    let mut order: Vec<(Option<String>, ToolErrorKind, String)> = Vec::new();
    let mut failures: HashMap<(Option<String>, ToolErrorKind, String), ToolFailure> =
        HashMap::new();

    let mut total_failed = 0usize;
    let mut total_calls = 0usize;
    let mut externalized = 0usize;

    for message in conversation.messages() {
        for content in &message.content {
            let MessageContent::ToolResponse(response) = content else {
                continue;
            };
            total_calls += 1;

            if let Ok(call) = &response.tool_result {
                externalized += call
                    .content
                    .iter()
                    .filter(|item| {
                        item.as_text().is_some_and(|text| {
                            crate::session::message_blobs::content_json_has_stub(&text.text)
                        })
                    })
                    .count();
            }

            // ⚠ BOTH spellings, via the one classifier that already knows the
            // difference. `classify` returns `Some` for `Err(_)` AND for an
            // `Ok` whose `is_error` is set; a hand-rolled `status == "error"`
            // test misses every tool that reported its own domain failure.
            let Some(ToolError {
                kind,
                retryable,
                message,
                ..
            }) = tool_errors::classify(&response.tool_result)
            else {
                continue;
            };
            total_failed += 1;

            let (tool_name, arguments) = calls
                .get(&response.id)
                .map(|(name, args)| (Some(name.clone()), args.clone()))
                .unwrap_or((None, None));

            // `message` is BR-51's envelope summary, which is capped short. The
            // response body is the fuller text and is what a maintainer needs;
            // fall back to the envelope when there is no body.
            let body = response_text(&response.tool_result);
            let text = clip(
                if body.trim().is_empty() {
                    &message
                } else {
                    &body
                },
                MESSAGE_CLIP_CHARS,
            );

            let key = (tool_name.clone(), kind, text.clone());
            *seen.entry(key.clone()).or_insert(0) += 1;
            failures.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                ToolFailure {
                    tool_name,
                    kind,
                    retryable,
                    looks_deliberate: looks_deliberate(&body) || looks_deliberate(&message),
                    message: text,
                    occurrences: 0,
                    arguments: arguments.map(|args| clip(&args, 300)),
                }
            });
        }
    }

    let mut collected: Vec<ToolFailure> = order
        .into_iter()
        .map(|key| {
            let count = seen[&key];
            let mut failure = failures.remove(&key).expect("keyed from the same map");
            failure.occurrences = count;
            failure
        })
        .collect();

    // Most repeated first, then the un-retryable, then transcript order — which
    // `sort_by_key` preserves because it is stable.
    collected.sort_by_key(|failure| {
        (
            std::cmp::Reverse(failure.occurrences),
            usize::from(failure.retryable),
        )
    });
    collected.truncate(MAX_FAILURES);

    (collected, total_failed, total_calls, externalized)
}

/// Phrases that carry no information about the bug, only the request to file
/// one. Lower-case, matched as a prefix after trimming punctuation, and only
/// where the phrase ends at a word boundary.
///
/// `/bug` is the slash command: its typed text (`/bug the chart panel is
/// blank`) is the user-visible row the command leaves behind, so the
/// description is whatever follows it.
const TRIGGER_PHRASES: &[&str] = &[
    "/bug",
    "report a bug to biorouter",
    "report a bug in biorouter",
    "file an issue with biorouter",
    "report a problem to biorouter",
    "report a bug",
    "report this bug",
    "file a bug report",
    "file a bug",
    "file an issue",
    "report an issue",
    "report a problem",
    "report this",
    "biorouter",
];

/// How much has to be left after the trigger phrase for it to be a description.
///
/// A sentence, roughly. Short enough that "the chart panel is blank" counts,
/// long enough that "yes", "please" and "the app" do not.
const DESCRIPTION_FLOOR_CHARS: usize = 30;

/// What the user said the problem is, taken from their own last message.
///
/// ⚠ Exists because of a measured live failure, not a hypothetical. Asked
/// *"Report a bug to BioRouter: the Auto Visualiser renders a blank panel for a
/// single-row dataset"*, gpt-5.5 reached for the tool correctly and called it
/// with `{"action": "analyze"}` — no `description`, though the parameter is
/// documented and the user's words were one message above. The tool then asked
/// the user to describe a problem they had just described. A model that omits
/// an optional argument is the ordinary case, so the tool reads the transcript
/// rather than depending on it.
///
/// Returns `None` for a bare trigger ("report a bug"), which genuinely carries
/// nothing and genuinely needs the question.
pub fn described_problem(recent_user_messages: &[String]) -> Option<String> {
    let last = recent_user_messages.last()?;
    let trimmed = last.trim();
    let lowered = trimmed.to_ascii_lowercase();

    // Longest phrase first, so "report a bug to biorouter" is not consumed as
    // "report a bug" leaving " to biorouter" behind.
    //
    // ⚠ `get(cut..)`, not `[cut..]`. The offset is computed against `lowered`
    // and applied to `trimmed`, which is sound today only because
    // `to_ascii_lowercase` is byte-for-byte and every trigger phrase is ASCII —
    // two facts a later edit can break without touching this line. This runs on
    // whatever the user typed, so the failure mode of getting it wrong is a
    // panic in the middle of their turn. A `None` here simply means no phrase
    // was stripped.
    let mut best = trimmed;
    for phrase in TRIGGER_PHRASES {
        let Some(rest) = lowered.strip_prefix(phrase) else {
            continue;
        };
        // A phrase that runs on into a word is not that phrase: "biorouterd
        // crashes …" must not lose its first nine letters, nor "/bugfix …"
        // its first four.
        if rest.chars().next().is_some_and(char::is_alphanumeric) {
            continue;
        }
        let cut = trimmed.len().saturating_sub(rest.len());
        let Some(candidate) = trimmed.get(cut..) else {
            continue;
        };
        let candidate = candidate.trim_start_matches([':', ',', '-', '—', '.', ' ']);
        if candidate.len() < best.len() {
            best = candidate;
        }
    }

    let best = best.trim();
    (best.chars().count() >= DESCRIPTION_FLOOR_CHARS).then(|| best.to_string())
}

/// `text` without the composer's chip and quotation markup, trimmed.
///
/// The composer appends both to the message it sends (a chip the user placed
/// becomes a `<biorouter-ref …>` tag, a quoted passage a `<biorouter-quote>`
/// block) and neither is the user's prose. The split is
/// [`resource_refs::split_composer_text`](crate::agents::resource_refs::split_composer_text),
/// the one implementation the `/bug` command's expansion uses too; this half
/// drops the quotations, which are someone else's words.
fn strip_composer_markup(text: &str) -> String {
    crate::agents::resource_refs::split_composer_text(text)
        .prose
        .trim()
        .to_string()
}

/// The user's own recent prose, most recent last.
///
/// ⚠ Only rows the USER can see. A slash command stores the typed text
/// user-visible and agent-hidden, and its expansion user-HIDDEN and
/// agent-visible (`Agent::reply`'s `execute_command` branch); hook context is
/// user-hidden too. Read without this filter, `/bug` would have the reporter
/// quote the model-only expansion back as "YOUR last message", and hidden rows
/// would crowd the user's own words out of the [`USER_MESSAGES_KEPT`] slots.
fn recent_user_text(conversation: &Conversation) -> Vec<String> {
    let mut out: Vec<String> = conversation
        .messages()
        .iter()
        .rev()
        .filter(|message| message.role == rmcp::model::Role::User)
        .filter(|message| message.metadata.user_visible)
        .filter_map(|message| {
            let text = message
                .content
                .iter()
                .filter_map(|content| match content {
                    MessageContent::Text(text) => Some(text.text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            let text = strip_composer_markup(&text);
            (!text.is_empty()).then(|| clip(&text, USER_CLIP_CHARS))
        })
        .take(USER_MESSAGES_KEPT)
        .collect();
    out.reverse();
    out
}

/// Assemble everything the report is written from.
///
/// `conversation` is a parameter rather than read from `session` because the
/// two disagree, and the caller is the one that knows which to use: the row
/// handed to `dispatch_tool_call` carries the snapshot taken at the top of the
/// turn, which is missing everything the turn has done since. See
/// `bug_report::conversation_for`.
pub fn collect(session: &Session, conversation: &Conversation) -> Evidence {
    let system = crate::session::SystemInfo::collect();
    let (failures, total_failed_calls, total_tool_calls, externalized_results) =
        failures_in(conversation);

    Evidence {
        session_id: session.id.clone(),
        failures,
        total_failed_calls,
        total_tool_calls,
        recent_user_messages: recent_user_text(conversation),
        externalized_results,
        app_version: system.app_version,
        os: system.os,
        os_version: system.os_version,
        architecture: system.architecture,
        // The SESSION's binding, not the global config's. `SystemInfo::collect`
        // reads `Config::global()`, which is the machine default and is simply a
        // different provider whenever the chat bound its own — the ordinary case
        // for a private session, and precisely the field a provider bug report
        // is about.
        provider: session.provider_name.clone().or(system.provider),
        model: session
            .model_config
            .as_ref()
            .map(|config| config.model_name.clone())
            .or(system.model),
        enabled_extensions: system.enabled_extensions,
        working_dir: session.working_dir.display().to_string(),
    }
}

/// A [`Conversation`] built from an exported `session.json`, for tests and for
/// a caller that has only the export.
pub fn conversation_from_export(export: &str) -> anyhow::Result<Conversation> {
    let value: serde_json::Value = serde_json::from_str(export)?;
    let messages = value
        .get("conversation")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("the export has no `conversation`"))?;
    let messages: Vec<Message> = serde_json::from_value(messages)?;
    Ok(Conversation::new_unvalidated(messages))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{CallToolRequestParams, CallToolResult, Content, ErrorCode, ErrorData};
    use std::borrow::Cow;

    fn request(id: &str, name: &str, arguments: serde_json::Value) -> MessageContent {
        MessageContent::ToolRequest(crate::conversation::message::ToolRequest {
            id: id.to_string(),
            tool_call: Ok(CallToolRequestParams {
                task: None,
                name: Cow::from(name.to_string()),
                arguments: arguments.as_object().cloned(),
                meta: None,
            }),
            metadata: None,
            tool_meta: None,
        })
    }

    /// The transport-level failure: `{"status":"error"}`.
    fn transport_error(id: &str, message: &str) -> MessageContent {
        MessageContent::ToolResponse(crate::conversation::message::ToolResponse {
            id: id.to_string(),
            tool_result: Err(ErrorData::new(
                ErrorCode::RESOURCE_NOT_FOUND,
                message.to_string(),
                None,
            )),
            metadata: None,
        })
    }

    /// The domain failure a scan for `status == "error"` never sees:
    /// `{"status":"success","value":{"isError":true}}`.
    fn domain_error(id: &str, message: &str) -> MessageContent {
        MessageContent::ToolResponse(crate::conversation::message::ToolResponse {
            id: id.to_string(),
            tool_result: Ok(CallToolResult {
                content: vec![Content::text(message.to_string())],
                structured_content: None,
                is_error: Some(true),
                meta: None,
            }),
            metadata: None,
        })
    }

    fn success(id: &str) -> MessageContent {
        MessageContent::ToolResponse(crate::conversation::message::ToolResponse {
            id: id.to_string(),
            tool_result: Ok(CallToolResult {
                content: vec![Content::text("fine".to_string())],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
            metadata: None,
        })
    }

    fn conversation(content: Vec<MessageContent>) -> Conversation {
        Conversation::new_unvalidated(vec![content
            .into_iter()
            .fold(Message::assistant(), Message::with_content)])
    }

    /// ⚠ The load-bearing test. A tool that ran and reported a domain failure
    /// serializes as a SUCCESS with `isError: true`, and it is the common
    /// spelling. A scan that missed it would answer "no problems found" while
    /// the user looked at a red error in the transcript.
    #[test]
    fn both_spellings_of_a_failed_call_are_found() {
        let (failures, total_failed, total_calls, _) = failures_in(&conversation(vec![
            request(
                "a",
                "developer__shell",
                serde_json::json!({"command": "ls"}),
            ),
            transport_error("a", "no such file or directory"),
            request("b", "developer__text_editor", serde_json::json!({})),
            domain_error("b", "the build failed: 3 errors"),
            request("c", "developer__shell", serde_json::json!({})),
            success("c"),
        ]));
        assert_eq!(total_calls, 3);
        assert_eq!(total_failed, 2, "{failures:#?}");
        let names: Vec<_> = failures
            .iter()
            .map(|f| f.tool_name.clone().unwrap())
            .collect();
        assert!(names.contains(&"developer__shell".to_string()), "{names:?}");
        assert!(
            names.contains(&"developer__text_editor".to_string()),
            "the `isError: true` spelling was missed: {names:?}"
        );
    }

    #[test]
    fn a_failure_carries_its_taxonomy_and_the_call_s_arguments() {
        let (failures, ..) = failures_in(&conversation(vec![
            request(
                "a",
                "developer__shell",
                serde_json::json!({"command": "cargo build"}),
            ),
            transport_error("a", "No such file or directory (os error 2)"),
        ]));
        assert_eq!(failures[0].kind, ToolErrorKind::NotFound);
        assert!(!failures[0].retryable);
        assert!(
            failures[0]
                .arguments
                .as_deref()
                .is_some_and(|args| args.contains("cargo build")),
            "{failures:#?}"
        );
    }

    /// One loop of nine identical failures is one bug.
    #[test]
    fn identical_failures_collapse_and_keep_their_count() {
        let mut content = Vec::new();
        for i in 0..9 {
            let id = format!("call-{i}");
            content.push(request(&id, "developer__shell", serde_json::json!({})));
            content.push(transport_error(&id, "connection reset by peer"));
        }
        let (failures, total_failed, ..) = failures_in(&conversation(content));
        assert_eq!(failures.len(), 1, "{failures:#?}");
        assert_eq!(failures[0].occurrences, 9);
        assert_eq!(
            total_failed, 9,
            "the raw count is still reported; only the report is de-duplicated"
        );
    }

    /// A response whose request is gone (compaction truncated it) is reported as
    /// unmatched rather than attributed to whatever tool was nearby.
    #[test]
    fn an_orphaned_response_is_not_attributed_to_a_neighbour() {
        let (failures, ..) = failures_in(&conversation(vec![
            request("a", "developer__shell", serde_json::json!({})),
            success("a"),
            transport_error("orphan", "it broke"),
        ]));
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].tool_name, None);
        assert!(failures[0].to_line().contains("(unmatched response)"));
    }

    fn evidence_with(failures: Vec<ToolFailure>) -> Evidence {
        Evidence {
            session_id: "s".into(),
            failures,
            total_failed_calls: 0,
            total_tool_calls: 0,
            recent_user_messages: Vec::new(),
            externalized_results: 0,
            app_version: "1.0.0".into(),
            os: "macos".into(),
            os_version: "27.0".into(),
            architecture: "aarch64".into(),
            provider: None,
            model: None,
            enabled_extensions: Vec::new(),
            working_dir: "/tmp".into(),
        }
    }

    fn failure(kind: ToolErrorKind, retryable: bool, occurrences: usize) -> ToolFailure {
        ToolFailure {
            tool_name: Some("developer__shell".into()),
            kind,
            retryable,
            message: "boom".into(),
            occurrences,
            arguments: None,
            looks_deliberate: false,
        }
    }

    /// ⚠ "A failure exists" is NOT the bar, and the difference is the whole
    /// push-back behaviour. One 429 is the ordinary weather of a long session;
    /// filing on it produces an issue a maintainer has to close.
    /// ⚠ Discovered from a real bundle, not imagined. The ONE failed call in
    /// the only real 30-message session read while building this was Gate C
    /// refusing `workspace_read_conversation` on a public model. Every coarse
    /// signal reads "hard bug" — not retryable, `ToolFailure` kind, a long
    /// error string — and filing it would have filed "the privacy boundary
    /// worked" as a Biorouter defect.
    #[test]
    fn a_deliberate_privacy_refusal_is_labelled_and_does_not_justify_filing() {
        let refusal = crate::privacy::refusal::workspace_out_of_reach();
        let (failures, ..) = failures_in(&conversation(vec![
            request(
                "a",
                "workspace__workspace_read_conversation",
                serde_json::json!({"session_id": "20260723_3"}),
            ),
            domain_error("a", &refusal),
        ]));
        assert_eq!(failures.len(), 1);
        assert!(failures[0].looks_deliberate, "{failures:#?}");
        assert!(
            failures[0]
                .to_line()
                .contains("looks like a deliberate refusal"),
            "the model must be told, not have it hidden: {}",
            failures[0].to_line()
        );

        let mut evidence = evidence_with(failures);
        assert!(
            !evidence.is_conclusive(),
            "a control doing its job is not grounds to file unprompted"
        );
        // Still reported, so a genuinely wrong refusal can be described.
        evidence.failures[0].looks_deliberate = false;
        assert!(evidence.is_conclusive());
    }

    /// Even nine of them. Repetition is how a model reacts to a refusal that
    /// forecloses the retry; it is not nine bugs.
    #[test]
    fn repeating_a_deliberate_refusal_still_does_not_justify_filing() {
        let mut failure = failure(ToolErrorKind::ToolFailure, false, 9);
        failure.looks_deliberate = true;
        assert!(!evidence_with(vec![failure]).is_conclusive());
    }

    /// The environment refusing is the taxonomy's own definition of
    /// `permission_denied`, and a control doing its job is not a defect.
    #[test]
    fn a_permission_denial_alone_is_not_conclusive() {
        assert!(
            !evidence_with(vec![failure(ToolErrorKind::PermissionDenied, false, 1)])
                .is_conclusive()
        );
    }

    #[test]
    fn a_lone_retryable_blip_is_not_conclusive() {
        assert!(!evidence_with(vec![failure(ToolErrorKind::Transient, true, 1)]).is_conclusive());
        assert!(!evidence_with(Vec::new()).is_conclusive());
    }

    /// The model passing bad arguments is a model mistake it can already see.
    #[test]
    fn bad_arguments_alone_are_not_conclusive() {
        assert!(
            !evidence_with(vec![failure(ToolErrorKind::InvalidArgs, false, 1)]).is_conclusive()
        );
    }

    #[test]
    fn a_hard_failure_or_a_repeated_one_is_conclusive() {
        assert!(evidence_with(vec![failure(ToolErrorKind::Internal, false, 1)]).is_conclusive());
        assert!(evidence_with(vec![failure(ToolErrorKind::NotFound, false, 1)]).is_conclusive());
        // Three of the same blip is no longer a blip.
        assert!(evidence_with(vec![failure(ToolErrorKind::Transient, true, 3)]).is_conclusive());
    }

    #[test]
    fn the_headline_names_the_worst_failure() {
        let evidence = evidence_with(vec![
            failure(ToolErrorKind::Transient, true, 5),
            failure(ToolErrorKind::Internal, false, 1),
        ]);
        let headline = evidence.headline().unwrap();
        assert!(headline.contains("internal"), "{headline}");
    }

    /// ⚠ From a live run, not from imagination. gpt-5.5 called this tool with
    /// `{"action": "analyze"}` and nothing else while the user's description
    /// sat one message above it, and the tool asked them to describe a problem
    /// they had just described.
    #[test]
    fn the_users_own_last_message_supplies_the_description_the_model_omitted() {
        let described = described_problem(&[
            "hello".to_string(),
            "Report a bug to BioRouter: when I ask the Auto Visualiser for a chart of a \
             dataset with only one row, the artifact panel renders blank."
                .to_string(),
        ])
        .expect("a described problem is recognised");
        assert!(
            described.starts_with("when I ask the Auto Visualiser"),
            "{described}"
        );
        assert!(
            !described.to_lowercase().contains("report a bug"),
            "{described}"
        );
    }

    /// ⚠ The longest trigger wins, or "report a bug to biorouter" leaves
    /// " to biorouter" behind and the tool reports that as the bug.
    #[test]
    fn the_longest_trigger_phrase_is_the_one_stripped() {
        let described = described_problem(&[
            "report a bug to biorouter — the sidebar will not resize below 1200 pixels".to_string(),
        ])
        .unwrap();
        assert_eq!(described, "the sidebar will not resize below 1200 pixels");
    }

    /// ⚠ Arbitrary user text, including non-ASCII, must not panic.
    ///
    /// The offset is computed against an ASCII-lowercased copy and applied to
    /// the original. That is sound only while every trigger phrase is ASCII and
    /// the lowercasing is byte-for-byte, and this runs on whatever the user
    /// typed — so getting it wrong panics mid-turn rather than misreporting.
    #[test]
    fn multibyte_prose_is_handled_without_panicking() {
        for text in [
            "报告一个错误：图表面板是空白的，当数据集只有一行时。",
            "Report a bug: le panneau d'artefact est complètement vide — j'attendais un graphique.",
            "report a bug — 🐛 the artifact panel renders blank for one-row datasets",
            "RÉPORT A BUG: something went wrong in a way that is quite long indeed",
        ] {
            // The assertion is that this returns at all.
            let _ = described_problem(&[text.to_string()]);
        }
    }

    /// A bare trigger carries nothing, and must still produce the question.
    #[test]
    fn a_bare_request_is_not_a_description() {
        for bare in [
            "report a bug",
            "report a bug.",
            "file an issue",
            "biorouter: report a bug",
            "report a bug please",
        ] {
            assert!(
                described_problem(&[bare.to_string()]).is_none(),
                "`{bare}` carries no problem statement"
            );
        }
        assert!(described_problem(&[]).is_none());
    }

    /// A message that never mentions filing at all is still a description --
    /// "the panel is blank, tell someone" arrives that way.
    #[test]
    fn prose_with_no_trigger_at_all_still_counts() {
        assert_eq!(
            described_problem(&["The artifact panel is blank for one-row datasets.".to_string()])
                .as_deref(),
            Some("The artifact panel is blank for one-row datasets.")
        );
    }

    /// `/bug` is a trigger like "report a bug": what follows it is the
    /// description, and a bare `/bug` is not one.
    #[test]
    fn the_slash_command_is_a_trigger_phrase() {
        assert_eq!(
            described_problem(&["/bug the chart panel is blank for one row".to_string()])
                .as_deref(),
            Some("the chart panel is blank for one row")
        );
        assert_eq!(
            described_problem(&["/bug: the chart panel is blank for one row".to_string()])
                .as_deref(),
            Some("the chart panel is blank for one row")
        );
        assert!(described_problem(&["/bug".to_string()]).is_none());
        assert!(described_problem(&["/bug  ".to_string()]).is_none());
    }

    /// A trigger that runs on into a word is not the trigger.
    #[test]
    fn a_trigger_phrase_must_end_at_a_word_boundary() {
        assert_eq!(
            described_problem(&["biorouterd crashes when the laptop wakes from sleep".to_string()])
                .as_deref(),
            Some("biorouterd crashes when the laptop wakes from sleep")
        );
        assert_eq!(
            described_problem(&["/bugfix the chart panel is blank for one row".to_string()])
                .as_deref(),
            Some("/bugfix the chart panel is blank for one row")
        );
    }

    /// ⚠ Only what the USER can see. After `/bug`, the transcript holds the
    /// typed text (user-visible, agent-hidden) and the command's expansion
    /// (user-hidden, agent-visible); hook context is user-hidden too. Without
    /// the filter the reporter quoted the model-only expansion back as the
    /// user's own last message.
    #[test]
    fn user_hidden_rows_are_not_the_user_s_words() {
        let conversation = Conversation::new_unvalidated(vec![
            Message::user().with_text("earlier question"),
            Message::user()
                .with_text("/bug the chart panel is blank for one row")
                .with_visibility(true, false),
            Message::user()
                .with_text(
                    "The user typed /bug: they want to report a problem with Biorouter itself.",
                )
                .with_visibility(false, true),
            Message::user()
                .with_text("<hook-context>injected by a hook</hook-context>")
                .with_visibility(false, true),
        ]);
        let recent = recent_user_text(&conversation);
        assert_eq!(
            recent,
            vec![
                "earlier question".to_string(),
                "/bug the chart panel is blank for one row".to_string()
            ]
        );
        assert_eq!(
            described_problem(&recent).as_deref(),
            Some("the chart panel is blank for one row")
        );
    }

    /// The composer appends a chip's `<biorouter-ref …>` tag and a quoted
    /// passage's `<biorouter-quote>…</biorouter-quote>` block to the message.
    /// Neither is the user's prose.
    #[test]
    fn composer_chips_and_quotes_are_not_the_user_s_words() {
        let conversation = Conversation::new_unvalidated(vec![
            Message::user().with_text(
                "the knowledge panel shows no pages \
                 <biorouter-ref type=\"knowledge_base\" id=\"soul\" label=\"Soul &amp; Body\"> \
                 <biorouter-ref type=\"skill\" name=\"rna-qc\"/>",
            ),
            Message::user().with_text(
                "explain this <biorouter-quote>{\"source\":\"report.md\",\"text\":\"cohort \
                 of 41 participants\"}</biorouter-quote>",
            ),
            // Nothing but a chip: nothing the user said.
            Message::user().with_text("<biorouter-ref type=\"extension\" name=\"developer\">"),
        ]);
        assert_eq!(
            recent_user_text(&conversation),
            vec![
                "the knowledge panel shows no pages".to_string(),
                "explain this".to_string()
            ]
        );
        // A malformed tag is not markup the composer wrote, and stays.
        assert_eq!(
            strip_composer_markup("a <biorouter-reference type=\"x\"> b"),
            "a <biorouter-reference type=\"x\"> b"
        );
    }

    #[test]
    fn the_user_s_own_recent_prose_is_kept_in_order() {
        let conversation = Conversation::new_unvalidated(vec![
            Message::user().with_text("first"),
            Message::assistant().with_text("ok"),
            Message::user().with_text("second"),
        ]);
        assert_eq!(recent_user_text(&conversation), vec!["first", "second"]);
    }
}
