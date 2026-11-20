//! Why a turn ended without doing its job.
//!
//! Before this existed, a turn that *failed* and a turn that *succeeded* were
//! indistinguishable to every consumer of the agent stream. A provider 403 was
//! downgraded into an assistant chat message ("Ran into this error: …") and the
//! stream then ended **normally**, so:
//!
//!   * `biorouter run` exited **0**,
//!   * `--output-format json` reported `"status": "completed"`,
//!   * telemetry logged the failed run as a **success**,
//!   * and the desktop/app runtimes rendered a finished turn.
//!
//! The only way to recover "did the turn actually run?" was to regex the English
//! prose the model-facing layer emitted. This makes the failure a typed, terminal
//! event that every boundary — CLI exit code, JSON status, SSE frame, WS frame —
//! can check mechanically.
//!
//! Wave 4 adds [`TurnAbortCode::ToolLoop`] and [`TurnAbortCode::WorkerTimeout`]
//! through the same channel.

use serde::{Deserialize, Serialize};

use crate::providers::errors::ProviderErrorKind;

/// The reason a turn was aborted. Serialized into SSE/WebSocket error frames and
/// mapped to process exit codes by the CLI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum TurnAbortCode {
    /// The provider call failed (auth, rate limit, network, server error).
    ProviderFailure { kind: ProviderErrorKind },
    /// The session store (SQLite) failed while persisting or reading the
    /// turn's messages (#31/#41). Distinct from `ProviderFailure` so wire
    /// frames and exit codes stop blaming the provider for a local disk/db
    /// problem — the operator remediation is completely different.
    SessionStore,
    /// The model called the same tool with the same arguments until the loop
    /// guard terminated the turn (Wave 4).
    ToolLoop { tool: String },
    /// A worker profile consulted by the main agent never answered (Wave 4).
    WorkerTimeout { agent: String, elapsed_s: u64 },
    /// Automatic output-length recovery spent its per-reply budget. Partial
    /// assistant output remains persisted and visible.
    OutputRecoveryExhausted {
        continuations: u32,
        zero_progress: bool,
    },
    /// A Bedrock reasoning signature can no longer be replayed against the
    /// exact provider-visible history it authenticated. No provider call was
    /// attempted because sending a changed prefix is guaranteed to fail.
    SignedReplayInvalidated,
    /// A provider-**signed** turn's response stream ended before a tool block's
    /// arguments were complete, and nothing had run.
    ///
    /// The sibling above and this one are both "signed content we cannot send
    /// back", and the difference between them is the whole reason this variant
    /// exists. `SignedReplayInvalidated` describes history that is already on
    /// the record and cannot be replayed without mutation — there is nowhere
    /// safe to resume from, so it is terminal. This one describes a turn that
    /// left **no** record: the partial assistant message was discarded rather
    /// than persisted, so the conversation is byte-for-byte the one the
    /// provider was already called with. Re-issuing it replays an untouched
    /// prefix, which is why it is the one signed abort that is retryable.
    SignedStreamTruncated,
}

impl TurnAbortCode {
    /// Stable machine-readable code for wire frames (`{"type":"error","code":…}`).
    pub fn wire_code(&self) -> &'static str {
        match self {
            Self::ProviderFailure { .. } => "provider_failure",
            Self::SessionStore => "session_store_failure",
            Self::ToolLoop { .. } => "tool_loop",
            Self::WorkerTimeout { .. } => "worker_timeout",
            Self::OutputRecoveryExhausted { .. } => "output_recovery_exhausted",
            Self::SignedReplayInvalidated => "signed_replay_invalidated",
            Self::SignedStreamTruncated => "signed_stream_truncated",
        }
    }

    /// The process exit code a CLI run should end with.
    ///
    /// These are distinct from the generic `1` that every existing `anyhow` path
    /// already returns, so a harness can tell "the agent ran and disagreed with
    /// you" from "the agent never ran".
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::ProviderFailure { kind } if kind.is_auth() => exit::PROVIDER_AUTH,
            Self::ProviderFailure { .. } => exit::PROVIDER_FAILED,
            Self::SessionStore => exit::SESSION_STORE,
            Self::ToolLoop { .. } => exit::TOOL_LOOP,
            Self::WorkerTimeout { .. } => exit::WORKER_TIMEOUT,
            Self::OutputRecoveryExhausted { .. } => exit::OUTPUT_RECOVERY_EXHAUSTED,
            Self::SignedReplayInvalidated => exit::SIGNED_REPLAY_INVALIDATED,
            Self::SignedStreamTruncated => exit::SIGNED_STREAM_TRUNCATED,
        }
    }
}

/// The machine-checkable abort code for a raw agent error — shared by the
/// CLI's reply-construction failure path and its mid-stream `Err` branch so
/// both report identically in structured output (#31/#41).
///
/// Classified by downcast, not by string matching: an error whose chain
/// contains a [`sqlx::Error`] is a session-store failure (the only sqlx in
/// the turn path is the session SQLite store), everything else falls back to
/// the provider classification. Before this, a failed `add_message` inside
/// `Agent::reply` was reported as `provider_failure` — the wire code and the
/// process exit code blamed the provider for a local db problem.
pub fn classify_agent_error(e: &anyhow::Error) -> TurnAbortCode {
    if e.chain().any(|cause| cause.is::<sqlx::Error>()) {
        return TurnAbortCode::SessionStore;
    }
    let kind = e
        .downcast_ref::<crate::providers::errors::ProviderError>()
        .map(crate::providers::errors::ProviderError::kind)
        .unwrap_or(ProviderErrorKind::Other);
    TurnAbortCode::ProviderFailure { kind }
}

/// #549: whether `e` says the session store is out of space, the one store
/// failure a headless run may outlive
/// (`crate::session::session_manager::deferred_writes`).
///
/// Exhausted storage only: SQLite `SQLITE_FULL` (13) or `SQLITE_IOERR` (10,
/// with any of its extended codes), or an I/O error of kind
/// [`std::io::ErrorKind::StorageFull`], anywhere in the chain. A constraint
/// violation, a closed pool, a decode error, a schema fault or any other I/O
/// error is not, and still aborts the turn as
/// [`TurnAbortCode::SessionStore`]. Read from the typed error, never from its
/// text.
pub fn is_storage_exhausted(e: &anyhow::Error) -> bool {
    e.chain().any(|cause| {
        if let Some(store) = cause.downcast_ref::<sqlx::Error>() {
            return match store {
                sqlx::Error::Database(database) => database
                    .code()
                    .and_then(|code| code.parse::<i32>().ok())
                    .is_some_and(|code| matches!(code & 0xff, 10 | 13)),
                sqlx::Error::Io(io) => io.kind() == std::io::ErrorKind::StorageFull,
                _ => false,
            };
        }
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::StorageFull)
    })
}

/// A turn that ended without doing its work, as an `Error` — so it can travel
/// through the CLI's existing `anyhow::Result` plumbing without changing every
/// signature, and be recovered at the top with `downcast_ref` to pick the exit
/// code.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct TurnFailed {
    pub code: TurnAbortCode,
    pub message: String,
}

impl TurnFailed {
    pub fn new(code: TurnAbortCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn exit_code(&self) -> u8 {
        self.code.exit_code()
    }
}

/// Process exit codes for a turn that did not complete its work.
pub mod exit {
    /// The turn ran to completion.
    pub const OK: u8 = 0;
    /// Any pre-existing `anyhow` failure path. Unchanged.
    pub const GENERIC: u8 = 1;
    /// The provider call failed (network, 5xx, rate limit).
    pub const PROVIDER_FAILED: u8 = 70;
    /// The provider rejected our credentials (401/403).
    pub const PROVIDER_AUTH: u8 = 75;
    /// The model looped on one tool until the guard terminated the turn.
    pub const TOOL_LOOP: u8 = 76;
    /// A consulted worker profile never answered.
    pub const WORKER_TIMEOUT: u8 = 77;
    /// The session store (SQLite) failed while persisting the turn (#31/#41).
    pub const SESSION_STORE: u8 = 78;
    /// The provider repeatedly filled its output allowance without completing.
    pub const OUTPUT_RECOVERY_EXHAUSTED: u8 = 79;
    /// Signed provider reasoning cannot be replayed without mutation.
    pub const SIGNED_REPLAY_INVALIDATED: u8 = 80;
    /// A signed turn was cut off mid-tool-call and rolled back; retry it.
    pub const SIGNED_STREAM_TRUNCATED: u8 = 81;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #31/#41: a session-store failure inside `Agent::reply` must classify
    /// as its own abort code — wire code and exit code stop blaming the
    /// provider for a local db problem. Classified by downcast anywhere in
    /// the anyhow chain, exactly how the store's errors travel (a root
    /// `sqlx::Error` under `context(...)` wrappers).
    #[test]
    fn store_errors_classify_as_session_store_not_provider_failure() {
        let store_error =
            anyhow::Error::new(sqlx::Error::PoolClosed).context("Failed to add message to session");
        assert_eq!(
            classify_agent_error(&store_error),
            TurnAbortCode::SessionStore
        );
        assert_eq!(
            classify_agent_error(&store_error).wire_code(),
            "session_store_failure"
        );
        assert_eq!(
            classify_agent_error(&store_error).exit_code(),
            exit::SESSION_STORE
        );

        // A typed provider error keeps its kind…
        let provider_error = anyhow::Error::new(
            crate::providers::errors::ProviderError::Authentication("403".to_string()),
        );
        assert_eq!(
            classify_agent_error(&provider_error),
            TurnAbortCode::ProviderFailure {
                kind: ProviderErrorKind::Auth
            }
        );

        // …and anything else stays the conservative provider/Other fallback.
        let opaque = anyhow::anyhow!("something else entirely");
        assert_eq!(
            classify_agent_error(&opaque),
            TurnAbortCode::ProviderFailure {
                kind: ProviderErrorKind::Other
            }
        );
    }

    #[test]
    fn auth_failures_get_their_own_exit_code() {
        let auth = TurnAbortCode::ProviderFailure {
            kind: ProviderErrorKind::Auth,
        };
        assert_eq!(auth.exit_code(), exit::PROVIDER_AUTH);

        let server = TurnAbortCode::ProviderFailure {
            kind: ProviderErrorKind::Server,
        };
        assert_eq!(server.exit_code(), exit::PROVIDER_FAILED);
    }

    /// Every abort code must be nonzero — the whole point is that a failed turn
    /// stops looking like a successful one.
    #[test]
    fn no_abort_code_maps_to_success() {
        for code in [
            TurnAbortCode::ProviderFailure {
                kind: ProviderErrorKind::Network,
            },
            TurnAbortCode::ToolLoop {
                tool: "ui_describe".into(),
            },
            TurnAbortCode::WorkerTimeout {
                agent: "fine_mapper".into(),
                elapsed_s: 120,
            },
            TurnAbortCode::OutputRecoveryExhausted {
                continuations: 12,
                zero_progress: false,
            },
            TurnAbortCode::SignedReplayInvalidated,
            TurnAbortCode::SignedStreamTruncated,
        ] {
            assert_ne!(code.exit_code(), exit::OK, "{code:?} must not exit 0");
            assert!(!code.wire_code().is_empty());
        }
    }

    /// The two signed aborts are separate codes on every boundary a harness can
    /// read. Collapsing either onto the other is how "retry this" and "this
    /// chat is over" become one indistinguishable outcome again.
    #[test]
    fn the_two_signed_aborts_never_share_a_code() {
        let invalidated = TurnAbortCode::SignedReplayInvalidated;
        let truncated = TurnAbortCode::SignedStreamTruncated;
        assert_ne!(invalidated.wire_code(), truncated.wire_code());
        assert_ne!(invalidated.exit_code(), truncated.exit_code());
        assert_eq!(truncated.wire_code(), "signed_stream_truncated");

        let json = serde_json::to_value(&truncated).unwrap();
        assert_eq!(json["reason"], "signed_stream_truncated");
        let back: TurnAbortCode = serde_json::from_value(json).unwrap();
        assert_eq!(back, truncated);
    }

    /// #549: only exhausted storage counts, read from the typed error, and it
    /// is still a session-store failure to every caller that does not ask.
    #[test]
    fn only_exhausted_storage_is_storage_exhausted() {
        let full = anyhow::Error::new(sqlx::Error::Io(std::io::Error::new(
            std::io::ErrorKind::StorageFull,
            "no space",
        )))
        .context("Failed to add message to session");
        assert!(is_storage_exhausted(&full));
        assert_eq!(classify_agent_error(&full), TurnAbortCode::SessionStore);
        let bare_io = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::StorageFull,
            "no space",
        ));
        assert!(is_storage_exhausted(&bare_io));

        for other in [
            anyhow::Error::new(sqlx::Error::PoolClosed),
            anyhow::Error::new(sqlx::Error::Io(std::io::Error::other(
                "database or disk is full",
            ))),
            anyhow::Error::new(sqlx::Error::RowNotFound),
            anyhow::anyhow!("database or disk is full"),
        ] {
            assert!(!is_storage_exhausted(&other), "{other:#}");
        }
    }

    #[test]
    fn wire_codes_round_trip_through_serde() {
        let code = TurnAbortCode::ToolLoop {
            tool: "ui_describe".into(),
        };
        let json = serde_json::to_value(&code).unwrap();
        assert_eq!(json["reason"], "tool_loop");
        assert_eq!(json["tool"], "ui_describe");
        let back: TurnAbortCode = serde_json::from_value(json).unwrap();
        assert_eq!(back, code);
    }
}
