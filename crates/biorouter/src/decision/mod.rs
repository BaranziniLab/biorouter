//! Decision models: fast models that return a typed answer with
//! probabilities instead of text. BioRouter asks them through one service,
//! [`DecisionService`], whatever asks: a monitor hook, the `decide` tool, a
//! phase gate, a decision chat, a route or the CLI.
//!
//! - [`types`]: questions, states, decisions, and the arithmetic every adapter
//!   shares (confidence, thresholds, renormalisation).
//! - [`model`]: the [`DecisionModel`] seam the adapters implement.
//! - [`service`]: model resolution, privacy, size, concurrency, deadlines,
//!   the save-time checks, and the [`DecisionSink`] seam for the decision log.
//! - [`late`]: requests still running after their caller stopped waiting, so
//!   a process can wait for them before it exits.
//! - [`save`]: storing a chat's decision settings after checking that what
//!   they turn on can run.
//! - [`window`]: the rolling conversation window a monitor asks about.
//! - [`monitor`]: monitors (decision models watching a chat while its agent
//!   works) and the per-turn tally the transcript marker is drawn from.
//!
//! The adapters themselves are in `providers::decision`.
//!
//! This builds on Albert Lee's prototype in PR #469 (Baseten System One exit
//! criteria), the first working decision-model integration in BioRouter: its
//! typed decision kinds and their validation, its noul threshold rule, its
//! System One question builders and answer-type check, its refusal to echo an
//! error body, and its use of the stall checker's tail as the evidence window
//! all carry over. Design: the decision-models design document, §2, §3 and §7.

pub mod error;
pub mod late;
pub mod log;
pub mod model;
pub mod monitor;
pub mod save;
pub mod service;
pub mod settings;
pub mod turns;
pub mod types;
pub mod window;

pub use error::{DecisionError, DecisionResult, INSTALL_ADVICE};
pub use late::settle_late_answers;
pub use model::{
    model_in_words, DecisionCaps, DecisionModel, DecisionModelEntry, DecisionModelSource,
};
pub use save::{save_chat_checked, save_checked, UnusableDecisionSettings};
pub use service::{
    chat_effect, global, ChatEffect, DecisionLogEntry, DecisionOutcome, DecisionService,
    DecisionSession, DecisionSink, DecisionUseGate, LiveClassification, ModelSource,
    SaveTierContext,
};
pub use types::{
    Answer, ChunkOutcome, Decision, DecisionAdapter, DecisionChunk, DecisionContext, DecisionKind,
    DecisionModelRef, DecisionOrigin, DecisionRequest, DecisionUsage, ProbabilitySource, Question,
    Threshold,
};
pub use window::{build_window, WindowSpec, WindowState};
