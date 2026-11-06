//! What can go wrong with a decision, as sentences a person (or the model that
//! asked) can act on. Each variant maps to one HTTP status
//! (`biorouter-server/src/routes/decisions.rs`).
//!
//! ⚠ No variant carries a backend's error body. System One validation errors
//! can quote the submitted state, and the state can be a private chat's text
//! (PR #469's `baseten_decisions.rs` made the same choice). A short
//! `error_type` token is kept, because it says what to fix without saying what
//! was sent.

/// A failed decision.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DecisionError {
    /// The request itself is malformed (400).
    #[error("{0}")]
    Invalid(String),
    /// A privacy or Crew rule refused the model before anything was sent (403).
    #[error("{0}")]
    Refused(String),
    /// The state does not fit the model's window (413). Names the chunk so the
    /// caller can split it.
    #[error(
        "{chunk} is about {tokens} tokens, which does not fit {model}'s {limit}-token window \
         (with room kept for the questions). Split it into smaller pieces and ask again."
    )]
    TooLarge {
        chunk: String,
        tokens: usize,
        limit: usize,
        model: String,
    },
    /// No decision model is configured or reachable (404 / 409).
    #[error("{0}")]
    NoModel(String),
    /// The model cannot answer this kind of question.
    #[error("{0}")]
    Unsupported(String),
    #[error("{model} did not answer within {seconds} s. Nothing was decided.")]
    Timeout { model: String, seconds: u64 },
    /// The backend refused or failed the request (502).
    #[error("{message}")]
    Backend {
        model: String,
        status: Option<u16>,
        error_type: Option<String>,
        message: String,
    },
    /// The backend answered in a shape that cannot be trusted (502).
    #[error("{model} returned an answer that could not be read: {detail}")]
    Malformed { model: String, detail: String },
}

impl DecisionError {
    pub fn malformed(model: &str, detail: impl Into<String>) -> Self {
        Self::Malformed {
            model: model.to_string(),
            detail: detail.into(),
        }
    }

    /// Whether retrying the same request could change the answer: a timeout, a
    /// rate limit or a server fault. A refusal, a size or a shape error will not.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Timeout { .. } => true,
            Self::Backend { status, .. } => {
                status.is_none_or(|s| s == 429 || (500..600).contains(&s))
            }
            _ => false,
        }
    }
}

pub type DecisionResult<T> = Result<T, DecisionError>;
