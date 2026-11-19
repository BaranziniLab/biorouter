//! The status `biorouter run` exits with, and the one line it prints on
//! stderr, when the run was told to leave files behind (`--require-file`, or
//! `BIOROUTER_RUN_REQUIRE_FILES`) and ended without them.
//!
//! The done gate keeps the agent working while a required file is missing or
//! empty, up to its attempt cap. Whatever ended the turn (the model gave up
//! after the cap, an action limit, a cancel), the files are checked once more
//! after it, and a run that is still missing one exits with [`EXIT_CODE`], 4,
//! so a script can tell "it ran and did not produce what was asked" (4) from a
//! run that finished (0) and from one its supervision or a limit stopped (3,
//! [`super::supervision_stop::EXIT_CODE`]). A turn that aborted keeps its own
//! status (70 and up), because the abort is the cause.

/// The exit status of a run that ended without a required file.
pub const EXIT_CODE: u8 = 4;

/// A required file a run ended without, and why it does not count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingFile {
    /// The path as it was given.
    pub path: String,
    /// The check's own reason ("does not exist", "is empty", ...).
    pub reason: String,
}

/// A `biorouter run` that ended without one or more of its required files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliverablesMissing(String);

impl DeliverablesMissing {
    pub fn new(missing: &[MissingFile]) -> Self {
        let named = missing
            .iter()
            .map(|file| file.reason.clone())
            .collect::<Vec<_>>()
            .join("; ");
        Self(format!(
            "This run ended without its required files (exit status {EXIT_CODE}): {named}."
        ))
    }

    /// The one line `main` prints on stderr.
    pub fn sentence(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DeliverablesMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DeliverablesMissing {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_names_every_missing_file_and_the_status() {
        let line = DeliverablesMissing::new(&[
            MissingFile {
                path: "answer.txt".into(),
                reason: "expected file `answer.txt` does not exist".into(),
            },
            MissingFile {
                path: "trace.md".into(),
                reason: "expected file `trace.md` is empty".into(),
            },
        ])
        .to_string();
        assert!(line.contains("(exit status 4)"), "{line}");
        assert!(line.contains("`answer.txt` does not exist"), "{line}");
        assert!(line.contains("`trace.md` is empty"), "{line}");
        assert!(!line.contains('\n'), "one line: {line}");
    }
}
