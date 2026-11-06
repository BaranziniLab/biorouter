//! Decision models: fast models that return a typed answer with
//! probabilities instead of text. BioRouter asks them through one service,
//! [`DecisionService`], whatever asks: a monitor hook, the `decide` tool, a
//! phase gate, a decision chat, a route or the CLI.
//!
//! - [`types`]: questions, states, decisions, and the arithmetic every adapter
//!   shares (confidence, thresholds, renormalisation).
//! - [`model`]: the [`DecisionModel`] seam the adapters implement.
//! - [`service`]: model resolution, privacy, size, concurrency, deadlines,
//!   and the [`DecisionSink`] seam for the decision log.
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
pub mod log;
pub mod model;
pub mod monitor;
pub mod service;
pub mod settings;
pub mod turns;
pub mod types;
pub mod window;

pub use error::{DecisionError, DecisionResult};
pub use model::{DecisionCaps, DecisionModel, DecisionModelEntry, DecisionModelSource};
pub use service::{
    global, DecisionLogEntry, DecisionOutcome, DecisionService, DecisionSession, DecisionSink,
    DecisionUseGate, LiveClassification, ModelSource,
};
pub use types::{
    Answer, ChunkOutcome, Decision, DecisionAdapter, DecisionChunk, DecisionContext, DecisionKind,
    DecisionModelRef, DecisionOrigin, DecisionRequest, DecisionUsage, ProbabilitySource, Question,
    Threshold,
};
pub use window::{build_window, WindowSpec, WindowState};
