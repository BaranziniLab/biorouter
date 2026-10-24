//! BR-63: a renderable preview of *what a tool call will actually do*, attached
//! to the confirmation event so the user approves with context.
//!
//! Before this module the confirmation card showed the tool's **name** and
//! nothing else — not the shell command, not the edit. The user was asked
//! "Biorouter wants to run this tool. Allow?" with no way to tell a `ls` from an
//! `rm -rf`, which is exactly the pressure that makes people click
//! "Always Allow" blindly.
//!
//! [`ToolPreview::for_tool_call`] turns `(tool_name, arguments)` into one of a
//! few shapes the GUI knows how to render:
//!
//! * [`ToolPreview::Shell`] — the resolved command, verbatim.
//! * [`ToolPreview::FileEdit`] — a line diff (`text_editor` `str_replace` /
//!   `insert` / `diff`, and `write` over a file that already exists).
//! * [`ToolPreview::FileWrite`] — the full contents of a brand-new file.
//! * [`ToolPreview::Arguments`] — pretty-printed arguments, so *every* other
//!   tool still shows something rather than nothing.
//!
//! Every variant is **bounded**. The preview rides an SSE frame, so a
//! multi-megabyte `file_text` or a 50k-line diff must not be shipped verbatim:
//! each shape clips itself and sets `truncated`, which the card surfaces so a
//! clipped preview is never mistaken for the whole story.

use rmcp::model::JsonObject;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Longest shell command echoed onto the card.
const MAX_COMMAND_CHARS: usize = 4_000;
/// Longest new-file body echoed onto the card.
const MAX_CONTENT_CHARS: usize = 8_000;
/// Longest pretty-printed argument blob echoed onto the card.
const MAX_ARGS_CHARS: usize = 4_000;
/// Most diff lines echoed onto the card.
const MAX_DIFF_LINES: usize = 200;
/// Longest single line in any preview; one pathological line cannot blow the frame.
const MAX_LINE_CHARS: usize = 400;
/// Above this many lines on either side, the quadratic LCS is skipped in favour
/// of a plain replace-all. 600x600 u16 cells is ~700 KB, which is the most work
/// worth doing on the confirmation path.
const MAX_LCS_LINES: usize = 600;
/// A `write` over an existing file is diffed against that file, but only when
/// the file is small enough to be worth reading synchronously here.
const MAX_DIFF_BASE_BYTES: u64 = 256 * 1024;

/// One line of a rendered diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ToolPreviewLineKind {
    /// Unchanged.
    Context,
    /// Present only after the edit.
    Added,
    /// Present only before the edit.
    Removed,
}

/// One line of a rendered diff, with its provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ToolPreviewLine {
    pub kind: ToolPreviewLineKind,
    pub text: String,
}

impl ToolPreviewLine {
    fn new(kind: ToolPreviewLineKind, text: &str) -> Self {
        Self {
            kind,
            text: clip_line(text),
        }
    }
}

/// What a pending tool call will do, in a shape the confirmation card can render.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ToolPreview {
    #[serde(rename_all = "camelCase")]
    FolderAccess {
        folder: String,
        access: String,
        files: Vec<String>,
        descendants: bool,
        once_descendants: bool,
        action_id: String,
    },
    /// A command that will be handed to a shell verbatim.
    #[serde(rename_all = "camelCase")]
    Shell {
        command: String,
        /// The command was clipped to fit the frame.
        truncated: bool,
    },
    /// An edit to a file that already exists, as a line diff.
    #[serde(rename_all = "camelCase")]
    FileEdit {
        path: String,
        lines: Vec<ToolPreviewLine>,
        added: usize,
        removed: usize,
        /// Lines beyond the cap were dropped; `added`/`removed` still count the whole edit.
        truncated: bool,
    },
    /// Creation of a new file, as its full (clipped) contents.
    #[serde(rename_all = "camelCase")]
    FileWrite {
        path: String,
        content: String,
        line_count: usize,
        truncated: bool,
    },
    /// Anything else: the arguments, pretty-printed, so no tool confirms blind.
    #[serde(rename_all = "camelCase")]
    Arguments { json: String, truncated: bool },
}

impl ToolPreview {
    /// Build the preview for a pending call. `None` only when there is genuinely
    /// nothing to show (a tool invoked with no arguments at all).
    pub fn for_tool_call(tool_name: &str, arguments: &JsonObject) -> Option<Self> {
        if arguments.is_empty() {
            return None;
        }

        match base_tool_name(tool_name) {
            "shell" | "bash" | "run_command" => Some(shell_preview(arguments)),
            "text_editor" | "str_replace_editor" | "str_replace_based_edit_tool" => {
                Some(text_editor_preview(arguments))
            }
            _ => Some(arguments_preview(arguments)),
        }
    }

    /// The arguments, pretty-printed, with a budget the CALLER chooses.
    ///
    /// [`Self::for_tool_call`] clips at [`MAX_ARGS_CHARS`], and that is the
    /// right rule for a tool whose arguments are *incidental* to what it does:
    /// the card shows them so nobody confirms blind, and four thousand
    /// characters is more than enough context for that judgement.
    ///
    /// It is the wrong rule for a tool whose ARGUMENT IS THE ARTEFACT — where
    /// the question is not "does this look like the right call?" but "here is
    /// the exact text that will be published, read it". `platform__report_bug`
    /// is that tool: its body becomes a world-readable, permanent GitHub issue,
    /// and the redaction harness in `bug_report::redact` says so in as many
    /// words — "the person is the last check and the design assumes it". A card
    /// that showed the first four thousand characters of a longer body would be
    /// collecting consent for text nobody was shown.
    ///
    /// ⚠ Note what this does NOT cost: the full arguments are already on the
    /// wire. `ToolApprovalRequest` carries `arguments` beside `preview`, so the
    /// body rides the frame whether or not the preview repeats it — clipping
    /// the preview shrinks what the user can READ, not what is sent.
    ///
    /// Still clips, and still reports it. A caller-chosen budget is not the
    /// absence of one, and the caller is expected to pass a ceiling its own
    /// validation has already enforced.
    pub fn arguments_within(arguments: &JsonObject, max_chars: usize) -> Self {
        let rendered = serde_json::to_string_pretty(arguments)
            .unwrap_or_else(|_| "<unserializable arguments>".to_string());
        let (json, truncated) = clip_chars(&rendered, max_chars);
        ToolPreview::Arguments { json, truncated }
    }
}

/// Tool names arrive namespaced (`developer__shell`); the shape depends only on
/// the bare name, so an extension that re-exports `shell` still previews well.
fn base_tool_name(tool_name: &str) -> &str {
    match tool_name.rsplit_once("__") {
        Some((_, base)) => base,
        None => tool_name,
    }
}

fn str_arg<'a>(arguments: &'a JsonObject, key: &str) -> Option<&'a str> {
    arguments.get(key).and_then(|v| v.as_str())
}

fn shell_preview(arguments: &JsonObject) -> ToolPreview {
    // Fall back to the raw arguments if this "shell" does not carry a `command`
    // — better an honest argument dump than an empty command box.
    let Some(command) = str_arg(arguments, "command") else {
        return arguments_preview(arguments);
    };
    let (command, truncated) = clip_chars(command, MAX_COMMAND_CHARS);
    ToolPreview::Shell { command, truncated }
}

fn text_editor_preview(arguments: &JsonObject) -> ToolPreview {
    let Some(path) = str_arg(arguments, "path") else {
        return arguments_preview(arguments);
    };
    let command = str_arg(arguments, "command").unwrap_or_default();

    match command {
        "write" | "create" => {
            let Some(file_text) = str_arg(arguments, "file_text") else {
                return arguments_preview(arguments);
            };
            // A write over an existing file is a *replacement*: diff it, so the
            // user sees what is lost, not just what arrives.
            match existing_file_contents(path) {
                Some(old) => file_edit(path, &old, file_text),
                None => {
                    let line_count = file_text.lines().count();
                    let (content, truncated) = clip_chars(file_text, MAX_CONTENT_CHARS);
                    ToolPreview::FileWrite {
                        path: path.to_string(),
                        content,
                        line_count,
                        truncated,
                    }
                }
            }
        }
        "str_replace" => {
            // The `diff` form of str_replace carries a ready-made unified diff
            // instead of old/new strings.
            if let Some(diff) = str_arg(arguments, "diff") {
                return unified_diff_preview(path, diff);
            }
            match (str_arg(arguments, "old_str"), str_arg(arguments, "new_str")) {
                (Some(old), Some(new)) => file_edit(path, old, new),
                _ => arguments_preview(arguments),
            }
        }
        "insert" => {
            let Some(new_str) = str_arg(arguments, "new_str") else {
                return arguments_preview(arguments);
            };
            // An insert only ever adds.
            file_edit(path, "", new_str)
        }
        _ => arguments_preview(arguments),
    }
}

/// Read the file a `write` is about to clobber, so it can be diffed. Skipped for
/// anything large or unreadable — the preview then degrades to `FileWrite`.
fn existing_file_contents(path: &str) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_DIFF_BASE_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

fn file_edit(path: &str, old: &str, new: &str) -> ToolPreview {
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    let lines = diff_lines(&old_lines, &new_lines);

    let added = lines
        .iter()
        .filter(|l| l.kind == ToolPreviewLineKind::Added)
        .count();
    let removed = lines
        .iter()
        .filter(|l| l.kind == ToolPreviewLineKind::Removed)
        .count();

    let truncated = lines.len() > MAX_DIFF_LINES;
    let mut lines = lines;
    lines.truncate(MAX_DIFF_LINES);

    ToolPreview::FileEdit {
        path: path.to_string(),
        lines,
        added,
        removed,
        truncated,
    }
}

/// Re-shape an already-unified diff (the `diff` argument) into rendered lines.
///
/// The `+`/`-` markers are *stripped* here: `text` always carries bare content
/// and `kind` alone carries the provenance, exactly as it does for a diff we
/// computed ourselves. The card re-adds a marker when it renders, so a line must
/// never arrive pre-marked or it would show up as `++here`.
fn unified_diff_preview(path: &str, diff: &str) -> ToolPreview {
    let mut lines = Vec::new();
    let mut added = 0usize;
    let mut removed = 0usize;

    for raw in diff.lines() {
        // `+++`/`---` are file headers, not content; `@@` is a hunk header.
        if raw.starts_with("+++") || raw.starts_with("---") {
            lines.push(ToolPreviewLine::new(ToolPreviewLineKind::Context, raw));
            continue;
        }
        // `strip_prefix` rather than `&raw[1..]`: the marker is ASCII but the
        // rest of the line is model text, and slicing a string by byte index is
        // a panic waiting for the first multi-byte char (and a denied lint).
        let (kind, text) = if let Some(rest) = raw.strip_prefix('+') {
            added += 1;
            (ToolPreviewLineKind::Added, rest)
        } else if let Some(rest) = raw.strip_prefix('-') {
            removed += 1;
            (ToolPreviewLineKind::Removed, rest)
        } else {
            // Context lines in a unified diff are indented by one space.
            (
                ToolPreviewLineKind::Context,
                raw.strip_prefix(' ').unwrap_or(raw),
            )
        };
        lines.push(ToolPreviewLine::new(kind, text));
    }

    let truncated = lines.len() > MAX_DIFF_LINES;
    lines.truncate(MAX_DIFF_LINES);

    ToolPreview::FileEdit {
        path: path.to_string(),
        lines,
        added,
        removed,
        truncated,
    }
}

fn arguments_preview(arguments: &JsonObject) -> ToolPreview {
    ToolPreview::arguments_within(arguments, MAX_ARGS_CHARS)
}

/// An empty string has no lines; `"".lines()` already yields nothing, but being
/// explicit keeps `insert` (old = "") honest.
fn split_lines(s: &str) -> Vec<&str> {
    if s.is_empty() {
        Vec::new()
    } else {
        s.lines().collect()
    }
}

fn clip_line(text: &str) -> String {
    clip_chars(text, MAX_LINE_CHARS).0
}

/// Clip on a **character** boundary (never a byte one — these strings are
/// user/model text and splitting a multi-byte char would panic).
fn clip_chars(s: &str, max: usize) -> (String, bool) {
    if s.chars().count() <= max {
        return (s.to_string(), false);
    }
    (s.chars().take(max).collect(), true)
}

/// Line-level diff via longest-common-subsequence.
///
/// Falls back to a plain replace-all when either side is too long to diff
/// cheaply — the result is clipped to [`MAX_DIFF_LINES`] anyway, so the extra
/// precision would never reach the screen.
fn diff_lines(old: &[&str], new: &[&str]) -> Vec<ToolPreviewLine> {
    if old.is_empty() {
        return new
            .iter()
            .map(|l| ToolPreviewLine::new(ToolPreviewLineKind::Added, l))
            .collect();
    }
    if new.is_empty() {
        return old
            .iter()
            .map(|l| ToolPreviewLine::new(ToolPreviewLineKind::Removed, l))
            .collect();
    }
    if old.len() > MAX_LCS_LINES || new.len() > MAX_LCS_LINES {
        let mut lines: Vec<ToolPreviewLine> = old
            .iter()
            .map(|l| ToolPreviewLine::new(ToolPreviewLineKind::Removed, l))
            .collect();
        lines.extend(
            new.iter()
                .map(|l| ToolPreviewLine::new(ToolPreviewLineKind::Added, l)),
        );
        return lines;
    }

    // lcs[i][j] = length of the LCS of old[i..] and new[j..].
    let (n, m) = (old.len(), new.len());
    let mut lcs = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if old[i] == new[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    let mut lines = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if old[i] == new[j] {
            lines.push(ToolPreviewLine::new(ToolPreviewLineKind::Context, old[i]));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            lines.push(ToolPreviewLine::new(ToolPreviewLineKind::Removed, old[i]));
            i += 1;
        } else {
            lines.push(ToolPreviewLine::new(ToolPreviewLineKind::Added, new[j]));
            j += 1;
        }
    }
    while i < n {
        lines.push(ToolPreviewLine::new(ToolPreviewLineKind::Removed, old[i]));
        i += 1;
    }
    while j < m {
        lines.push(ToolPreviewLine::new(ToolPreviewLineKind::Added, new[j]));
        j += 1;
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(value: serde_json::Value) -> JsonObject {
        value.as_object().expect("object").clone()
    }

    fn texts(lines: &[ToolPreviewLine], kind: ToolPreviewLineKind) -> Vec<&str> {
        lines
            .iter()
            .filter(|l| l.kind == kind)
            .map(|l| l.text.as_str())
            .collect()
    }

    #[test]
    fn no_arguments_previews_nothing() {
        assert_eq!(
            ToolPreview::for_tool_call("developer__shell", &args(json!({}))),
            None
        );
    }

    #[test]
    fn shell_preview_shows_the_resolved_command() {
        let preview = ToolPreview::for_tool_call(
            "developer__shell",
            &args(json!({"command": "rm -rf /tmp/scratch"})),
        )
        .unwrap();

        assert_eq!(
            preview,
            ToolPreview::Shell {
                command: "rm -rf /tmp/scratch".to_string(),
                truncated: false,
            }
        );
    }

    #[test]
    fn shell_preview_is_found_through_any_extension_prefix() {
        // The namespace is the extension's, the shape is the tool's.
        for name in ["shell", "developer__shell", "my_ext__bash"] {
            let preview =
                ToolPreview::for_tool_call(name, &args(json!({"command": "echo hi"}))).unwrap();
            assert!(
                matches!(preview, ToolPreview::Shell { .. }),
                "{name} should preview as a shell command"
            );
        }
    }

    #[test]
    fn an_enormous_shell_command_is_clipped_and_says_so() {
        let long = "x".repeat(MAX_COMMAND_CHARS + 50);
        let preview =
            ToolPreview::for_tool_call("developer__shell", &args(json!({"command": long})))
                .unwrap();

        match preview {
            ToolPreview::Shell { command, truncated } => {
                assert_eq!(command.chars().count(), MAX_COMMAND_CHARS);
                assert!(truncated, "a clipped command must announce it was clipped");
            }
            other => panic!("expected a shell preview, got {other:?}"),
        }
    }

    #[test]
    fn str_replace_previews_a_real_diff() {
        let preview = ToolPreview::for_tool_call(
            "developer__text_editor",
            &args(json!({
                "command": "str_replace",
                "path": "/repo/src/main.rs",
                "old_str": "let a = 1;\nlet b = 2;\nlet c = 3;",
                "new_str": "let a = 1;\nlet b = 20;\nlet c = 3;",
            })),
        )
        .unwrap();

        match preview {
            ToolPreview::FileEdit {
                path,
                lines,
                added,
                removed,
                truncated,
            } => {
                assert_eq!(path, "/repo/src/main.rs");
                assert_eq!((added, removed), (1, 1));
                assert!(!truncated);
                // Only the middle line changed; the other two are context.
                assert_eq!(texts(&lines, ToolPreviewLineKind::Removed), ["let b = 2;"]);
                assert_eq!(texts(&lines, ToolPreviewLineKind::Added), ["let b = 20;"]);
                assert_eq!(
                    texts(&lines, ToolPreviewLineKind::Context),
                    ["let a = 1;", "let c = 3;"]
                );
            }
            other => panic!("expected a file edit, got {other:?}"),
        }
    }

    #[test]
    fn insert_previews_as_pure_additions() {
        let preview = ToolPreview::for_tool_call(
            "developer__text_editor",
            &args(json!({
                "command": "insert",
                "path": "/repo/a.txt",
                "insert_line": 3,
                "new_str": "fresh line",
            })),
        )
        .unwrap();

        match preview {
            ToolPreview::FileEdit {
                added,
                removed,
                lines,
                ..
            } => {
                assert_eq!((added, removed), (1, 0));
                assert_eq!(texts(&lines, ToolPreviewLineKind::Added), ["fresh line"]);
            }
            other => panic!("expected a file edit, got {other:?}"),
        }
    }

    #[test]
    fn writing_a_new_file_previews_its_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brand-new.txt");

        let preview = ToolPreview::for_tool_call(
            "developer__text_editor",
            &args(json!({
                "command": "write",
                "path": path.to_string_lossy(),
                "file_text": "one\ntwo\n",
            })),
        )
        .unwrap();

        match preview {
            ToolPreview::FileWrite {
                content,
                line_count,
                truncated,
                ..
            } => {
                assert_eq!(content, "one\ntwo\n");
                assert_eq!(line_count, 2);
                assert!(!truncated);
            }
            other => panic!("expected a file write, got {other:?}"),
        }
    }

    #[test]
    fn writing_over_an_existing_file_previews_the_diff_not_just_the_new_text() {
        // The whole point: a `write` silently *replaces*. The user must see what
        // is being destroyed, which the raw arguments never showed.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.txt");
        std::fs::write(&path, "keep\nold value\n").unwrap();

        let preview = ToolPreview::for_tool_call(
            "developer__text_editor",
            &args(json!({
                "command": "write",
                "path": path.to_string_lossy(),
                "file_text": "keep\nnew value\n",
            })),
        )
        .unwrap();

        match preview {
            ToolPreview::FileEdit {
                lines,
                added,
                removed,
                ..
            } => {
                assert_eq!((added, removed), (1, 1));
                assert_eq!(texts(&lines, ToolPreviewLineKind::Removed), ["old value"]);
                assert_eq!(texts(&lines, ToolPreviewLineKind::Added), ["new value"]);
                assert_eq!(texts(&lines, ToolPreviewLineKind::Context), ["keep"]);
            }
            other => panic!("expected a diff against the existing file, got {other:?}"),
        }
    }

    #[test]
    fn a_ready_made_unified_diff_is_rendered_as_one() {
        let preview = ToolPreview::for_tool_call(
            "developer__text_editor",
            &args(json!({
                "command": "str_replace",
                "path": "/repo/a.rs",
                "diff": "@@ -1,2 +1,2 @@\n context\n-gone\n+here\n",
            })),
        )
        .unwrap();

        match preview {
            ToolPreview::FileEdit {
                lines,
                added,
                removed,
                ..
            } => {
                assert_eq!((added, removed), (1, 1));
                // Markers are stripped: `kind` carries the provenance, so the
                // card can render its own marker without doubling it up.
                assert_eq!(texts(&lines, ToolPreviewLineKind::Added), ["here"]);
                assert_eq!(texts(&lines, ToolPreviewLineKind::Removed), ["gone"]);
                assert_eq!(
                    texts(&lines, ToolPreviewLineKind::Context),
                    ["@@ -1,2 +1,2 @@", "context"]
                );
            }
            other => panic!("expected a file edit, got {other:?}"),
        }
    }

    #[test]
    fn a_unified_diff_line_with_multibyte_text_survives_marker_stripping() {
        // Stripping the marker by byte index would panic on the first non-ASCII
        // line, and diffs are full of model prose.
        let preview = ToolPreview::for_tool_call(
            "developer__text_editor",
            &args(json!({
                "command": "str_replace",
                "path": "/repo/a.rs",
                "diff": "-café ☕\n+naïve 🚀\n",
            })),
        )
        .unwrap();

        match preview {
            ToolPreview::FileEdit { lines, .. } => {
                assert_eq!(texts(&lines, ToolPreviewLineKind::Removed), ["café ☕"]);
                assert_eq!(texts(&lines, ToolPreviewLineKind::Added), ["naïve 🚀"]);
            }
            other => panic!("expected a file edit, got {other:?}"),
        }
    }

    #[test]
    fn a_huge_diff_is_clipped_but_still_reports_the_true_totals() {
        let old = (0..500)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let new = (0..500)
            .map(|i| format!("changed {i}"))
            .collect::<Vec<_>>()
            .join("\n");

        let preview = ToolPreview::for_tool_call(
            "developer__text_editor",
            &args(json!({
                "command": "str_replace",
                "path": "/repo/big.rs",
                "old_str": old,
                "new_str": new,
            })),
        )
        .unwrap();

        match preview {
            ToolPreview::FileEdit {
                lines,
                added,
                removed,
                truncated,
                ..
            } => {
                assert!(truncated, "a 1000-line diff must be clipped");
                assert_eq!(lines.len(), MAX_DIFF_LINES);
                // The counts describe the whole edit, not the clipped window —
                // otherwise the card would understate the blast radius.
                assert_eq!((added, removed), (500, 500));
            }
            other => panic!("expected a file edit, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_tool_still_shows_its_arguments() {
        let preview = ToolPreview::for_tool_call(
            "spoke__cypher_query",
            &args(json!({"query": "MATCH (n) RETURN n"})),
        )
        .unwrap();

        match preview {
            ToolPreview::Arguments { json, truncated } => {
                assert!(json.contains("MATCH (n) RETURN n"));
                assert!(!truncated);
            }
            other => panic!("expected an argument dump, got {other:?}"),
        }
    }

    #[test]
    fn a_malformed_call_degrades_to_its_arguments_rather_than_lying() {
        // `write` with no `file_text`: show what we do have.
        let preview = ToolPreview::for_tool_call(
            "developer__text_editor",
            &args(json!({"command": "write", "path": "/repo/a.rs"})),
        )
        .unwrap();
        assert!(matches!(preview, ToolPreview::Arguments { .. }));

        // A "shell" with no `command`.
        let preview =
            ToolPreview::for_tool_call("developer__shell", &args(json!({"nope": 1}))).unwrap();
        assert!(matches!(preview, ToolPreview::Arguments { .. }));
    }

    #[test]
    fn multibyte_text_is_clipped_on_a_char_boundary() {
        // Clipping bytes here would panic; the model emits plenty of non-ASCII.
        let long = "é".repeat(MAX_COMMAND_CHARS + 10);
        let preview =
            ToolPreview::for_tool_call("developer__shell", &args(json!({"command": long})))
                .unwrap();

        match preview {
            ToolPreview::Shell { command, truncated } => {
                assert!(truncated);
                assert_eq!(command.chars().count(), MAX_COMMAND_CHARS);
            }
            other => panic!("expected a shell preview, got {other:?}"),
        }
    }

    #[test]
    fn one_pathological_line_cannot_blow_up_the_frame() {
        let preview = ToolPreview::for_tool_call(
            "developer__text_editor",
            &args(json!({
                "command": "str_replace",
                "path": "/repo/a.rs",
                "old_str": "short",
                "new_str": "y".repeat(MAX_LINE_CHARS + 100),
            })),
        )
        .unwrap();

        match preview {
            ToolPreview::FileEdit { lines, .. } => {
                assert!(lines
                    .iter()
                    .all(|l| l.text.chars().count() <= MAX_LINE_CHARS));
            }
            other => panic!("expected a file edit, got {other:?}"),
        }
    }

    #[test]
    fn the_wire_shape_is_tagged_by_kind() {
        // The GUI discriminates on `kind`; lock the contract down.
        let value = serde_json::to_value(ToolPreview::Shell {
            command: "ls".into(),
            truncated: false,
        })
        .unwrap();
        assert_eq!(value["kind"], "shell");
        assert_eq!(value["command"], "ls");

        let value = serde_json::to_value(ToolPreview::FileEdit {
            path: "/a".into(),
            lines: vec![ToolPreviewLine::new(ToolPreviewLineKind::Added, "hi")],
            added: 1,
            removed: 0,
            truncated: false,
        })
        .unwrap();
        assert_eq!(value["kind"], "fileEdit");
        assert_eq!(value["lines"][0]["kind"], "added");
        assert_eq!(value["lineCount"], serde_json::Value::Null);
    }
}
