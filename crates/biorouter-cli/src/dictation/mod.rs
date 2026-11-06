//! The terminal's dictation: `biorouter transcribe` (PR-12), and from PR-25 `biorouter
//! dictate` and the front ends' capture (`docs/dictation/surfaces.md`, "The terminal").
//!
//! The exit codes and the `--format json` error shape are shared by `transcribe` and
//! `dictate`, so they live here: 0 ok; 1 a failure, a failed transcription included; 2
//! declined to start; 3 nothing heard; 4 cancelled by the person; 5 `--copy` could not
//! reach a clipboard and printed the text instead. The sentences they print are in
//! [`messages`], as the message catalog places the CLI's copy.

#[cfg(feature = "dictation")]
pub mod decode;
pub mod messages;

use serde::Serialize;

/// `transcribe --format json`'s schema (and `dictate --format json`'s, from PR-25).
pub const SCHEMA: u32 = 1;

/// The exit codes of `dictate` and `transcribe`.
pub mod exit {
    pub const OK: i32 = 0;
    /// A failure, a failed transcription included (MSG-ENGINE-FAILED-CLI's first
    /// sentence on stderr).
    pub const FAILED: i32 = 1;
    /// Declined to start: no engine, unsupported processor, policy off, no model, a
    /// language the model cannot take.
    pub const DECLINED: i32 = 2;
    /// Nothing heard: no speech, an echo only, too short, or exact zeros.
    pub const NOTHING_HEARD: i32 = 3;
    /// Cancelled by the person.
    pub const CANCELLED: i32 = 4;
    /// `dictate --copy` printed the text because no clipboard was reachable (PR-25).
    pub const CLIPBOARD_FALLBACK: i32 = 5;
}

/// How a `transcribe` or `dictate` run ended, other than with text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Transcribed,
    /// The text was printed because no clipboard was reachable (PR-25).
    ClipboardFallback,
    NotCompiled,
    UnsupportedCpu,
    Policy,
    NoModel,
    NotInstalled,
    UnknownModel,
    Language,
    NoSpeech,
    TooShort,
    TooLong,
    UnsupportedFormat,
    NotFound,
    Unreadable,
    /// `--output` could not be written.
    Unwritable,
    EngineFailed,
    Cancelled,
}

impl Outcome {
    /// The process exit code.
    pub fn exit_code(self) -> i32 {
        match self {
            Outcome::Transcribed => exit::OK,
            Outcome::ClipboardFallback => exit::CLIPBOARD_FALLBACK,
            Outcome::NotCompiled
            | Outcome::UnsupportedCpu
            | Outcome::Policy
            | Outcome::NoModel
            | Outcome::NotInstalled
            | Outcome::UnknownModel
            | Outcome::Language => exit::DECLINED,
            Outcome::NoSpeech | Outcome::TooShort => exit::NOTHING_HEARD,
            Outcome::Cancelled => exit::CANCELLED,
            Outcome::TooLong
            | Outcome::UnsupportedFormat
            | Outcome::NotFound
            | Outcome::Unreadable
            | Outcome::Unwritable
            | Outcome::EngineFailed => exit::FAILED,
        }
    }

    /// The JSON `error_code`, in the words of the HTTP routes where they share one.
    pub fn error_code(self) -> &'static str {
        match self {
            Outcome::Transcribed => "ok",
            Outcome::ClipboardFallback => "clipboard_unavailable",
            Outcome::NotCompiled => "not_compiled",
            Outcome::UnsupportedCpu => "unsupported_cpu",
            Outcome::Policy => "policy",
            Outcome::NoModel => "no_model",
            Outcome::NotInstalled => "not_installed",
            Outcome::UnknownModel => "unknown_model",
            Outcome::Language => "language",
            Outcome::NoSpeech => "no_speech",
            Outcome::TooShort => "too_short",
            Outcome::TooLong => "too_long",
            Outcome::UnsupportedFormat => "unsupported_format",
            Outcome::NotFound => "not_found",
            Outcome::Unreadable => "unreadable",
            Outcome::Unwritable => "unwritable",
            Outcome::EngineFailed => "engine_failed",
            Outcome::Cancelled => "cancelled",
        }
    }
}

/// `--format json` when a run ends without text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct JsonError {
    pub schema: u32,
    pub error_code: &'static str,
    pub message: String,
}

impl JsonError {
    pub fn new(outcome: Outcome, message: impl Into<String>) -> JsonError {
        JsonError {
            schema: SCHEMA,
            error_code: outcome.error_code(),
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every exit code from 0 to 5 is what some outcome ends with, and each outcome has
    /// one stable `error_code`.
    #[test]
    fn every_exit_code_is_reachable() {
        let all = [
            Outcome::Transcribed,
            Outcome::ClipboardFallback,
            Outcome::NotCompiled,
            Outcome::UnsupportedCpu,
            Outcome::Policy,
            Outcome::NoModel,
            Outcome::NotInstalled,
            Outcome::UnknownModel,
            Outcome::Language,
            Outcome::NoSpeech,
            Outcome::TooShort,
            Outcome::TooLong,
            Outcome::UnsupportedFormat,
            Outcome::NotFound,
            Outcome::Unreadable,
            Outcome::Unwritable,
            Outcome::EngineFailed,
            Outcome::Cancelled,
        ];
        let table: Vec<(Outcome, i32, &str)> = all
            .iter()
            .map(|o| (*o, o.exit_code(), o.error_code()))
            .collect();
        for code in 0..=5 {
            assert!(
                table.iter().any(|(_, c, _)| *c == code),
                "exit {code} is unreachable"
            );
        }
        assert!(table.iter().all(|(_, c, _)| (0..=5).contains(c)));
        let mut codes: Vec<&str> = table.iter().map(|(_, _, e)| *e).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), all.len(), "error codes must be distinct");
        assert_eq!(Outcome::NoSpeech.exit_code(), 3);
        assert_eq!(Outcome::TooShort.exit_code(), 3);
        assert_eq!(Outcome::Cancelled.exit_code(), 4);
        assert_eq!(Outcome::NoModel.exit_code(), 2);
        assert_eq!(Outcome::EngineFailed.exit_code(), 1);
        assert_eq!(
            Outcome::UnsupportedFormat.error_code(),
            "unsupported_format"
        );
    }

    #[test]
    fn the_json_error_is_schema_1() {
        let error = JsonError::new(Outcome::NoSpeech, messages::MSG_NO_SPEECH);
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            serde_json::json!({"schema": 1, "error_code": "no_speech", "message": messages::MSG_NO_SPEECH})
        );
    }
}
