//! How a headless `biorouter run` says how it ended, beyond "it finished" or
//! "it aborted": the fields its final output carries, and the exit statuses
//! for the two endings that are not failures of the run itself.
//!
//! * **Why it stopped (#540).** `stop_reason`, `actions`, `tool_calls` and
//!   `wall_s` ride on the stream-json `complete` event and in the `json`
//!   document's `metadata`. A limit stop (the action limit, the tool-call
//!   limit, the stall stop, the per-reply budget, a usage limit) exits 3 like a
//!   monitor's stop ([`super::supervision_stop::limit_stopped`]).
//! * **Required files (#535).** A run given `--require-file` that still ends
//!   without one of them as a non-empty regular file exits
//!   [`DELIVERABLES_EXIT_CODE`], 4, and lists them in `missing_files`.
//! * **A session store that ran out of space (#549).** A headless run whose
//!   store filled up keeps going with its unsaved rows in memory. When rows are
//!   still unsaved at the end, it exits [`STORE_INCOMPLETE_EXIT_CODE`], 5:
//!   the answer is there, the stored chat is not whole.
//!
//! Precedence, highest first: a turn that aborted (70 and up), then 4, then 3,
//! then 5, then 0. Whatever wins the status, every line that applies is
//! printed on stderr and every field is in the final output.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// The exit status of a run that ended without a required file.
pub const DELIVERABLES_EXIT_CODE: u8 = 4;

/// The exit status of a run that finished with rows its session store could
/// not save.
pub const STORE_INCOMPLETE_EXIT_CODE: u8 = 5;

/// A required file a run ended without, and why it does not count.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct MissingFile {
    /// The path as it was given to `--require-file`.
    pub path: String,
    /// `does not exist`, `is empty`, `is not a regular file` or `could not be
    /// read`.
    pub reason: String,
}

/// The fields a headless run adds to its final output. Every one is optional,
/// so a reader that ignores them is unaffected, and an interactive session's
/// output never carries them.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct RunReport {
    /// `end_turn`, `turn_limit`, `tool_call_limit`, `stall_stop`,
    /// `budget_stop`, `usage_limit_stop`, `monitor_stop`, `aborted` or
    /// `cancelled`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    /// Model requests the turn made (not provider retries, not side requests
    /// such as a title or a compaction).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<u32>,
    /// Tool calls the turn dispatched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<u32>,
    /// Seconds from the start of the turn to its final output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_s: Option<f64>,
    /// The required files the run ended without. Present only when the run
    /// had required files, and empty when every one was there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_files: Option<Vec<MissingFile>>,
    /// Present, and `true`, only when the session store ran out of space
    /// during the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_degraded: Option<bool>,
    /// With `store_degraded`: the rows still not saved when the run ended. `0`
    /// means every row was saved once space came back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsaved_rows: Option<u64>,
}

impl RunReport {
    /// The required files the run ended without (empty without any).
    pub fn missing(&self) -> &[MissingFile] {
        self.missing_files.as_deref().unwrap_or_default()
    }

    /// The rows the session store could not save.
    pub fn unsaved(&self) -> u64 {
        self.unsaved_rows.unwrap_or(0)
    }
}

/// Which of `paths` are not a non-empty regular file, each resolved against
/// `dir` when relative, in the order given. The same test the run's done gate
/// applies ([`biorouter::agents::types::SuccessCheck::FileNonEmpty`]).
pub async fn missing_required_files(paths: &[String], dir: &Path) -> Vec<MissingFile> {
    let mut missing = Vec::new();
    for path in paths {
        let resolved = biorouter::agents::retry::resolve_check_path(path, Some(dir));
        if let Some(problem) = biorouter::agents::retry::required_file_problem(&resolved).await {
            missing.push(MissingFile {
                path: path.clone(),
                reason: problem.describe().to_string(),
            });
        }
    }
    missing
}

/// A `biorouter run` that ended without one or more of its required files. A
/// type, so `main` prints the sentence alone and exits
/// [`DELIVERABLES_EXIT_CODE`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliverablesMissing(String);

impl DeliverablesMissing {
    pub fn new(missing: &[MissingFile]) -> Self {
        let named = missing
            .iter()
            .map(|file| format!("{} {}", file.path, file.reason))
            .collect::<Vec<_>>()
            .join("; ");
        Self(format!(
            "This run ended without its required files (exit status {DELIVERABLES_EXIT_CODE}): \
             {named}."
        ))
    }
}

impl std::fmt::Display for DeliverablesMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DeliverablesMissing {}

/// A `biorouter run` that finished while its session store could not save
/// every row. A type, so `main` prints the sentence alone and exits
/// [`STORE_INCOMPLETE_EXIT_CODE`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreIncomplete(String);

impl StoreIncomplete {
    pub fn new(unsaved: u64, no_session: bool) -> Self {
        let rows = if unsaved == 1 { "row" } else { "rows" };
        let resume = if no_session {
            ""
        } else {
            " Resuming this chat will not show everything this run did."
        };
        Self(format!(
            "This run finished, but the session store ran out of space and {unsaved} {rows} \
             of it could not be saved (exit status {STORE_INCOMPLETE_EXIT_CODE}).{resume}"
        ))
    }
}

impl std::fmt::Display for StoreIncomplete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StoreIncomplete {}

/// The one stderr line a run prints when its store first runs out of space.
pub fn store_degraded_warning() -> &'static str {
    "Warning: the session store is out of space. This run keeps going and holds \
     what it cannot save in memory; it saves it, in order, if space comes back."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_required_files_names_each_file_and_why() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("answer.txt"), b"42\n").unwrap();
        std::fs::write(dir.path().join("trace.md"), b"").unwrap();
        std::fs::create_dir(dir.path().join("out")).unwrap();
        let paths = ["answer.txt", "trace.md", "out", "gone.txt"].map(String::from);
        let missing = missing_required_files(&paths, dir.path()).await;
        assert_eq!(
            missing,
            vec![
                MissingFile {
                    path: "trace.md".into(),
                    reason: "is empty".into()
                },
                MissingFile {
                    path: "out".into(),
                    reason: "is not a regular file".into()
                },
                MissingFile {
                    path: "gone.txt".into(),
                    reason: "does not exist".into()
                },
            ]
        );
        let absolute = dir.path().join("answer.txt").to_string_lossy().into_owned();
        assert!(
            missing_required_files(&[absolute], Path::new("/nonexistent"))
                .await
                .is_empty()
        );
    }

    #[test]
    fn the_lines_name_the_files_the_rows_and_the_status() {
        let line = DeliverablesMissing::new(&[
            MissingFile {
                path: "answer.txt".into(),
                reason: "does not exist".into(),
            },
            MissingFile {
                path: "trace.md".into(),
                reason: "is empty".into(),
            },
        ])
        .to_string();
        assert!(line.contains("(exit status 4)"), "{line}");
        assert!(line.contains("answer.txt does not exist"), "{line}");
        assert!(line.contains("trace.md is empty"), "{line}");
        assert!(!line.contains('\n'), "one line: {line}");

        let store = StoreIncomplete::new(3, false).to_string();
        assert!(store.contains("3 rows"), "{store}");
        assert!(store.contains("(exit status 5)"), "{store}");
        assert!(store.contains("Resuming"), "{store}");
        let ephemeral = StoreIncomplete::new(1, true).to_string();
        assert!(ephemeral.contains("1 row "), "{ephemeral}");
        assert!(!ephemeral.contains("Resuming"), "{ephemeral}");
    }

    #[test]
    fn an_empty_report_adds_nothing_to_the_output() {
        assert_eq!(
            serde_json::to_value(RunReport::default()).unwrap(),
            serde_json::json!({})
        );
        let report = RunReport {
            stop_reason: Some("turn_limit".into()),
            actions: Some(1),
            tool_calls: Some(1),
            wall_s: Some(0.5),
            missing_files: Some(Vec::new()),
            store_degraded: None,
            unsaved_rows: None,
        };
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            serde_json::json!({
                "stop_reason": "turn_limit",
                "actions": 1,
                "tool_calls": 1,
                "wall_s": 0.5,
                "missing_files": []
            })
        );
    }
}
