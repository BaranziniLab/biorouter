//! Decision-model adapters: the backends a [`crate::decision::DecisionService`]
//! sends questions to.
//!
//! Albert Lee's prototype in PR #469 (`agents/baseten_decisions.rs`,
//! `agents/exit_criteria.rs`) was the first working decision-model integration
//! in BioRouter, and the System One question builders, the answer-type check,
//! the slug catalog and the refusal to echo error bodies carry over from it.
//!
//! In order of preference, recorded on every decision:
//! 1. [`system_one`]: a decision model's own distribution, from Baseten or a
//!    local Ollama (0.35 and later).
//! 2. [`logprob`]: a chat model's first-token log-probabilities, over Ollama's
//!    `/api/chat` or an OpenAI-format endpoint.
//! 3. [`verbalized`]: a number a chat model writes, never calibrated.
//!
//! [`chat`] is the decision model bound as a chat's main model: the provider
//! the factory registers for `baseten_decision` and `ollama_decision`, and the
//! reply a decision chat gives instead of an LLM loop.
//!
//! [`install`] downloads a decision model into the Ollama on this computer
//! when a person asks (the install route and `biorouter models decision
//! pull`), so a private chat has one it may use. Nothing installs on its own.
//!
//! These live under `providers/` on purpose: the destination-key scan in
//! `providers/destination_keys.rs` reads every config key this tree reads, so
//! a new host key cannot be added here without deciding whether changing it
//! over HTTP needs the user-action proof.

pub mod catalog;
pub mod chat;
pub mod install;
pub mod logprob;
pub mod system_one;
pub mod verbalized;

pub use catalog::{ConfiguredDecisionModels, BASETEN_DECISION_PROVIDER, OLLAMA_DECISION_PROVIDER};
pub use chat::{
    is_decision_chat_provider, BasetenDecisionChatProvider, DecisionChatProvider,
    OllamaDecisionChatProvider,
};
pub use install::{
    installer, DecisionInstallSnapshot, DecisionInstaller, InstallJob, InstallOffer,
    InstallRefusal, InstallRefusalKind, InstallState, InstallableDecisionModel, OllamaPlace,
    OllamaStatus, PullProgress, PullStage, DEFAULT_INSTALL_MODEL, INSTALLABLE_DECISION_MODELS,
};
pub use logprob::LogprobAdapter;
pub use system_one::SystemOneAdapter;
pub use verbalized::VerbalizedAdapter;
