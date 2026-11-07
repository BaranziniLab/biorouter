//! What can go wrong with a decision, as sentences a person (or the model that
//! asked) can act on. Each variant maps to one HTTP status
//! (`biorouter-server/src/routes/decisions.rs`).
//!
//! ⚠ No variant carries a backend's error body. System One validation errors
//! can quote the submitted state, and the state can be a private chat's text
//! (PR #469's `baseten_decisions.rs` made the same choice). A short
//! `error_type` token is kept, because it says what to fix without saying what
//! was sent.

use super::types::DecisionUsage;

/// How a person gets a decision model that runs on this computer: the one
/// sentence every surface that finds none quotes (the service's no-model
/// refusals, `biorouter decide`, the `decision_models` tool).
pub const INSTALL_ADVICE: &str = "Install one on this computer with `biorouter models decision \
     pull` (tev1:0.8b, about 1.5 GB), or with Install in a chat's Monitors dialog.";

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
    /// The backend answered in a shape that cannot be trusted (502). The
    /// model answered, so the request was billed: `usage` is what it
    /// reported, when the adapter could read that much, so the failure row
    /// and the usage ledger still count it. Build it with
    /// [`DecisionError::malformed`] and add the usage with
    /// [`DecisionError::with_billed_usage`].
    #[error("{model} returned an answer that could not be read: {detail}")]
    Malformed {
        model: String,
        detail: String,
        usage: DecisionUsage,
    },
}

impl DecisionError {
    /// An answer that could not be read, with no usage yet.
    pub fn malformed(model: &str, detail: impl Into<String>) -> Self {
        Self::Malformed {
            model: model.to_string(),
            detail: detail.into(),
            usage: DecisionUsage::default(),
        }
    }

    /// The tokens the request was billed for although it failed: a
    /// [`Self::Malformed`] answer's usage, else none. A backend that refused
    /// or failed the request (non-2xx) reports no usage, and its body is
    /// never read, so a [`Self::Backend`] error bills nothing here.
    pub fn billed_usage(&self) -> DecisionUsage {
        match self {
            Self::Malformed { usage, .. } => *usage,
            _ => DecisionUsage::default(),
        }
    }

    /// The same error with `usage` added to what it was billed for. Only a
    /// [`Self::Malformed`] answer carries usage; every other variant is
    /// returned unchanged.
    pub fn with_billed_usage(self, usage: DecisionUsage) -> Self {
        match self {
            Self::Malformed {
                model,
                detail,
                usage: before,
            } => Self::Malformed {
                model,
                detail,
                usage: before + usage,
            },
            other => other,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: i64, output: i64) -> DecisionUsage {
        DecisionUsage {
            input_tokens: Some(input),
            output_tokens: Some(output),
        }
    }

    /// Only an answer that could not be read carries what it was billed for;
    /// adding usage to any other error changes nothing, and the sentence a
    /// person reads never mentions it.
    #[test]
    fn billed_usage_rides_only_malformed() {
        let malformed = DecisionError::malformed("joey-1", "no answer for question `q`");
        assert_eq!(malformed.billed_usage(), DecisionUsage::default());
        let billed = malformed.clone().with_billed_usage(usage(30, 1));
        assert_eq!(billed.billed_usage(), usage(30, 1));
        assert_eq!(
            billed.clone().with_billed_usage(usage(5, 2)).billed_usage(),
            usage(35, 3),
            "usage adds up"
        );
        assert_eq!(billed.to_string(), malformed.to_string());

        let backend = DecisionError::Backend {
            model: "joey-1".into(),
            status: Some(500),
            error_type: None,
            message: "boom".into(),
        };
        let timeout = DecisionError::Timeout {
            model: "joey-1".into(),
            seconds: 10,
        };
        for other in [backend, timeout, DecisionError::Invalid("x".into())] {
            assert_eq!(other.clone().with_billed_usage(usage(30, 1)), other);
            assert_eq!(other.billed_usage(), DecisionUsage::default());
        }
    }
}
