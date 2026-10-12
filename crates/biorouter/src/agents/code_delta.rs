//! The lines a Code Execution script's file edits added and removed.
//!
//! The desktop Chat summary's Code figure counts each successful `text_editor`
//! edit. A direct call carries its whole arguments in the transcript, so the
//! renderer measures it there. A call made from inside a script does not: its
//! record in `biorouter/tool-calls` keeps `args` cut at
//! `MAX_TOOL_CALL_RECORD_TEXT_BYTES` (so a real file's write is no longer
//! JSON), and records past the meta budget are dropped. So the script measures
//! its own edits here, from each sub-call's FULL arguments, and attaches the
//! total as `biorouter/code-delta`. Presentation only, like the records: the
//! model never sees it.
//!
//! The rules are the renderer's (`ui/desktop/src/components/summary/codeDelta.ts`),
//! and both test suites run the same table, `codeDeltaCases.json`, so they
//! cannot drift apart unnoticed:
//! - `write`/`create`: every line of `file_text` is added (the earlier content
//!   is not known);
//! - `str_replace`: the line difference between `old_str` and `new_str`, or
//!   the `+`/`-` lines of a unified `diff`;
//! - `insert`: every line of `new_str` is added;
//! - `undo_edit`: reverses the last counted edit of the same file.
//!
//! An edit and its undo both count, as two commits would.

use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::HashMap;

/// Result-meta key carrying a script's measured edits for the UI.
pub(crate) const CODE_DELTA_META_KEY: &str = "biorouter/code-delta";

/// Above this many line pairs (after the unchanged head and tail are set
/// aside) the changed region is counted whole rather than diffed. Must equal
/// `LCS_CELL_BUDGET` in the renderer.
const LCS_CELL_BUDGET: usize = 1_000_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct CodeDelta {
    pub(crate) added: usize,
    pub(crate) removed: usize,
}

impl CodeDelta {
    fn reversed(self) -> Self {
        Self {
            added: self.removed,
            removed: self.added,
        }
    }
}

enum EditorEdit {
    Changed(CodeDelta),
    Undo,
}

/// A script's running total of the edits its successful sub-calls made.
#[derive(Debug, Default)]
pub(crate) struct CodeDeltaTally {
    total: CodeDelta,
    /// Each file's counted edits, newest last, for `undo_edit` to take back.
    history: HashMap<String, Vec<CodeDelta>>,
    /// Whether any editing call was seen, so a script that edited nothing
    /// carries no key at all.
    observed: bool,
}

impl CodeDeltaTally {
    /// One SUCCESSFUL sub-call, by its prefixed name and full JSON arguments.
    /// Anything that is not a `text_editor` edit is ignored.
    pub(crate) fn observe(&mut self, tool_name: &str, arguments: &str) {
        if !is_editor_tool(tool_name) {
            return;
        }
        if let Ok(Value::Object(args)) = serde_json::from_str::<Value>(arguments) {
            self.editor_call(&args);
        }
    }

    fn editor_call(&mut self, args: &Map<String, Value>) {
        let Some(edit) = editor_edit(args) else {
            return;
        };
        self.observed = true;
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        match edit {
            EditorEdit::Undo => {
                if let Some(last) = self.history.get_mut(&path).and_then(Vec::pop) {
                    self.add(last.reversed());
                }
            }
            EditorEdit::Changed(delta) => {
                self.add(delta);
                self.history.entry(path).or_default().push(delta);
            }
        }
    }

    fn add(&mut self, delta: CodeDelta) {
        self.total.added = self.total.added.saturating_add(delta.added);
        self.total.removed = self.total.removed.saturating_add(delta.removed);
    }

    /// The `biorouter/code-delta` value, or `None` when the script edited nothing.
    pub(crate) fn meta_value(&self) -> Option<Value> {
        self.observed
            .then(|| serde_json::to_value(self.total).unwrap_or_default())
    }
}

/// `text_editor` is the developer extension's; `str_replace_editor` takes the
/// same arguments.
fn is_editor_tool(tool_name: &str) -> bool {
    let base = tool_name.rsplit("__").next().unwrap_or(tool_name);
    matches!(base, "text_editor" | "str_replace_editor")
}

fn editor_edit(args: &Map<String, Value>) -> Option<EditorEdit> {
    let text = |key: &str| args.get(key).and_then(Value::as_str);
    let delta = match text("command")? {
        "write" | "create" => CodeDelta {
            added: split_lines(text("file_text")?).len(),
            removed: 0,
        },
        "str_replace" => match text("diff") {
            Some(diff) => patch_delta(diff),
            None => line_delta(text("old_str")?, text("new_str")?),
        },
        "insert" => CodeDelta {
            added: split_lines(text("new_str")?).len(),
            removed: 0,
        },
        "undo_edit" => return Some(EditorEdit::Undo),
        _ => return None,
    };
    Some(EditorEdit::Changed(delta))
}

/// A text's lines: a final newline ends the last line rather than starting one.
fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

/// Lines `after` has that `before` lacks, and the reverse, by longest common
/// subsequence.
fn line_delta(before: &str, after: &str) -> CodeDelta {
    let a = split_lines(before);
    let b = split_lines(after);
    let mut start = 0;
    while start < a.len() && start < b.len() && a[start] == b[start] {
        start += 1;
    }
    let (mut end_a, mut end_b) = (a.len(), b.len());
    while end_a > start && end_b > start && a[end_a - 1] == b[end_b - 1] {
        end_a -= 1;
        end_b -= 1;
    }
    let (a, b) = (&a[start..end_a], &b[start..end_b]);
    let (n, m) = (a.len(), b.len());
    if n == 0 || m == 0 || n.saturating_mul(m) > LCS_CELL_BUDGET {
        return CodeDelta {
            added: m,
            removed: n,
        };
    }
    let mut previous = vec![0usize; m + 1];
    let mut current = vec![0usize; m + 1];
    for line in a {
        // `diagonal` is previous[j - 1], `left` is current[j - 1].
        let (mut diagonal, mut left) = (0, 0);
        for (j, other) in b.iter().enumerate() {
            let up = previous[j + 1];
            left = if line == other {
                diagonal + 1
            } else {
                up.max(left)
            };
            current[j + 1] = left;
            diagonal = up;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let common = previous[m];
    CodeDelta {
        added: m - common,
        removed: n - common,
    }
}

/// `-a[,count] +b[,count]` from a hunk header, counts defaulting to 1, or
/// `None` when the header carries no usable range.
fn hunk_counts(line: &str) -> Option<(i64, i64)> {
    fn range(text: &str) -> Option<(i64, &str)> {
        let digits = text.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        // The digits are ASCII, so these cuts fall on char boundaries.
        let rest = text.get(digits..)?;
        let Some(rest) = rest.strip_prefix(',') else {
            return Some((1, rest));
        };
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        let count = rest.get(..digits)?.parse().unwrap_or(i64::MAX);
        Some((count, rest.get(digits..)?))
    }
    let (old, rest) = range(line.strip_prefix("@@ -")?)?;
    let (new, rest) = range(rest.strip_prefix(" +")?)?;
    rest.starts_with(" @@").then_some((old, new))
}

#[derive(PartialEq)]
enum PatchState {
    Header,
    Counted,
    Loose,
}

/// The `+` and `-` lines of a unified diff's hunks. A hunk header's counts say
/// where the hunk ends, so a removed `-- x` line (written `--- x`) is not taken
/// for the next file's header. A header without counts falls back to counting
/// every `+`/`-` line that is not a `+++ `/`--- ` header.
fn patch_delta(diff: &str) -> CodeDelta {
    let mut delta = CodeDelta::default();
    let mut state = PatchState::Header;
    let (mut old_left, mut new_left) = (0i64, 0i64);
    for raw in diff.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if state == PatchState::Counted {
            match line.chars().next() {
                Some('\\') => continue,
                Some('-') => {
                    delta.removed += 1;
                    old_left -= 1;
                }
                Some('+') => {
                    delta.added += 1;
                    new_left -= 1;
                }
                Some(' ') | None => {
                    old_left -= 1;
                    new_left -= 1;
                }
                Some(_) => state = PatchState::Header,
            }
            if state == PatchState::Counted {
                if old_left <= 0 && new_left <= 0 {
                    state = PatchState::Header;
                }
                continue;
            }
        }
        if line.starts_with("@@") {
            state = match hunk_counts(line) {
                Some((old, new)) => {
                    old_left = old;
                    new_left = new;
                    if old > 0 || new > 0 {
                        PatchState::Counted
                    } else {
                        PatchState::Header
                    }
                }
                None => PatchState::Loose,
            };
            continue;
        }
        if state == PatchState::Loose {
            if line.starts_with("+++ ") || line.starts_with("--- ") {
                state = PatchState::Header;
            } else if line.starts_with('+') {
                delta.added += 1;
            } else if line.starts_with('-') {
                delta.removed += 1;
            }
        }
    }
    delta
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Case {
        name: String,
        calls: Vec<Map<String, Value>>,
        added: usize,
        removed: usize,
    }

    /// The renderer runs this same table (`codeDelta.test.ts`).
    #[test]
    fn agrees_with_the_renderer_on_every_shared_case() {
        let cases: Vec<Case> = serde_json::from_str(include_str!(
            "../../../../ui/desktop/src/components/summary/codeDeltaCases.json"
        ))
        .expect("the shared cases parse");
        assert!(cases.len() >= 10, "the shared table was found and read");
        for case in cases {
            let mut tally = CodeDeltaTally::default();
            for args in &case.calls {
                tally.observe(
                    "developer__text_editor",
                    &serde_json::to_string(args).unwrap(),
                );
            }
            assert_eq!(
                tally.total,
                CodeDelta {
                    added: case.added,
                    removed: case.removed,
                },
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn counts_a_write_far_larger_than_a_record_keeps() {
        let args = serde_json::json!({
            "command": "write",
            "path": "/p/big.ts",
            "file_text": "const x = 1;\n".repeat(4000),
        })
        .to_string();
        assert!(args.len() > 4 * 2048);
        let mut tally = CodeDeltaTally::default();
        tally.observe("developer__text_editor", &args);
        assert_eq!(
            tally.meta_value(),
            Some(serde_json::json!({ "added": 4000, "removed": 0 }))
        );
    }

    #[test]
    fn a_script_that_edited_nothing_carries_no_total() {
        let mut tally = CodeDeltaTally::default();
        tally.observe("developer__shell", r#"{"command":"echo +3 -1 > out.txt"}"#);
        tally.observe(
            "developer__text_editor",
            r#"{"command":"view","path":"/p/a"}"#,
        );
        tally.observe("developer__text_editor", "{not json");
        assert_eq!(tally.meta_value(), None);
    }

    #[test]
    fn an_edit_that_changed_no_line_still_reports_a_total() {
        let mut tally = CodeDeltaTally::default();
        tally.observe(
            "developer__text_editor",
            r#"{"command":"str_replace","path":"/p/a","old_str":"x","new_str":"x"}"#,
        );
        assert_eq!(
            tally.meta_value(),
            Some(serde_json::json!({ "added": 0, "removed": 0 }))
        );
    }

    #[test]
    fn a_huge_replacement_is_counted_whole_instead_of_diffed() {
        let before = (0..2000).map(|i| format!("a{i}\n")).collect::<String>();
        let after = (0..2000).map(|i| format!("b{i}\n")).collect::<String>();
        assert_eq!(
            line_delta(&before, &after),
            CodeDelta {
                added: 2000,
                removed: 2000,
            }
        );
    }
}
