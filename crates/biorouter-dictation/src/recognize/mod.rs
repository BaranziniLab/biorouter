//! What happens to the engine's words before a person sees them (DD-48).
//!
//! The worker writes what Whisper decoded; the parent decides what of it is speech.
//! [`text`] joins the pieces and holds the whole output filter, which empties an output
//! made only of what Whisper writes over silence and noise. [`echo`] holds the checks
//! against the exact prompt that was sent (a whole echo, a trailing recital) and the
//! collapse check, and decides whether one retry is worth making.
//!
//! PR-14 adds the dictionary's half: the keys, the word list, ranking, the prompt and
//! correction.

pub mod echo;
pub mod text;

use serde::{Deserialize, Serialize};

/// Why a decode was made a second time, as the transcript's `retried.reason` reports it.
/// A collapse and a deleted start are one check (fewer words than the detected speech
/// can hold), so they share one reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryReason {
    Echo,
    Collapse,
}

impl RetryReason {
    pub fn as_str(self) -> &'static str {
        match self {
            RetryReason::Echo => "echo",
            RetryReason::Collapse => "collapse",
        }
    }
}
