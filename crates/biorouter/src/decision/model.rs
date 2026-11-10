//! The seam between the decision service and the adapters: a [`DecisionModel`]
//! answers a [`DecisionRequest`], and a [`DecisionModelSource`] lists and opens
//! them. The adapters live under `providers::decision`, so the destination-key
//! scan in `providers/destination_keys.rs` sees every host key they read.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::error::DecisionResult;
use super::types::{
    DecisionAdapter, DecisionKindTag, DecisionModelRef, DecisionRequest, ModelVerdicts,
    MAX_CHOICE_OPTIONS,
};
use crate::privacy::affiliation::ModelAffiliation;
use crate::privacy::ProviderTier;

/// What a decision model can take.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionCaps {
    pub kinds: Vec<DecisionKindTag>,
    pub max_questions: usize,
    /// The model's window. The service refuses a request it estimates above
    /// this, minus a margin, before anything is sent.
    pub context_tokens: usize,
    /// Requests in flight at once for this model.
    pub max_concurrency: usize,
    /// A body size the backend refuses above (Ollama's System One: 64 KiB),
    /// counted by [`DecisionModel::request_bytes`].
    pub max_request_bytes: Option<usize>,
    /// Runs on this machine: ranked first, and given longer for the first call,
    /// which includes loading the model.
    pub local: bool,
}

/// Remote models take this many requests at once by default.
pub const REMOTE_CONCURRENCY: usize = 8;
/// A local server shares one machine with the chat, so fewer.
pub const LOCAL_CONCURRENCY: usize = 2;

/// One model that answers decisions.
#[async_trait]
pub trait DecisionModel: Send + Sync {
    /// The reference this model answers to, with its adapter filled in.
    fn model_ref(&self) -> DecisionModelRef;
    fn display_name(&self) -> String;
    /// Declared, never inherited: Gate H reads it before every call.
    fn tier(&self) -> ProviderTier;
    fn affiliation(&self) -> Option<ModelAffiliation>;
    fn caps(&self) -> DecisionCaps;
    /// The most options one choice question may offer this model. Defaults to
    /// the shared [`MAX_CHOICE_OPTIONS`]; a model family that takes fewer
    /// says so here, and the service refuses a larger choice before sending
    /// (and as a setting that asks one is saved).
    fn max_choice_options(&self) -> usize {
        MAX_CHOICE_OPTIONS
    }
    /// The bytes `request` takes as it is sent, which
    /// [`DecisionCaps::max_request_bytes`] is a limit on. Defaults to the
    /// state's own bytes plus each question's instructions and 64 for its
    /// framing; an adapter that builds the body itself counts the encoded
    /// body, which JSON escaping and its own fields make larger.
    fn request_bytes(&self, request: &DecisionRequest) -> usize {
        request.context.byte_len()
            + request
                .questions
                .values()
                .map(|q| q.instructions.len() + 64)
                .sum::<usize>()
    }
    /// Answer every question over the request's one state. The service has
    /// already validated the request, checked privacy and checked its size.
    async fn decide(&self, request: &DecisionRequest) -> DecisionResult<ModelVerdicts>;
}

/// A decision model as the catalog lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionModelEntry {
    /// `baseten_decision` or `ollama_decision`.
    pub provider: String,
    pub model: String,
    pub display_name: String,
    pub context_tokens: usize,
    pub kinds: Vec<DecisionKindTag>,
    pub tier: ProviderTier,
    /// Not serialized: `ModelAffiliation` has no wire form. The routes report
    /// the tier, which is what a person chooses by.
    #[serde(skip)]
    pub affiliation: Option<ModelAffiliation>,
    pub adapter: DecisionAdapter,
    pub local: bool,
    /// Set when the server will not load this copy of the model: the sentence
    /// that says so and names the fix (an Ollama model downloaded in a format
    /// that Ollama no longer supports). Such a model stays listed, so a person
    /// sees it and why, and is listed after the ones that load. `None` when it
    /// loads, and whenever discovery could not tell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_refusal: Option<String>,
}

impl DecisionModelEntry {
    pub fn model_ref(&self) -> DecisionModelRef {
        DecisionModelRef::new(&self.provider, &self.model).with_adapter(self.adapter)
    }

    /// Whether `wanted` names this entry: by provider, then by model id or
    /// display name, and by adapter when one is asked for.
    pub fn answers_to(&self, wanted: &DecisionModelRef) -> bool {
        self.provider == wanted.provider
            && (self.model == wanted.model || self.display_name == wanted.model)
            && wanted.adapter.is_none_or(|adapter| adapter == self.adapter)
    }
}

/// Lists and opens decision models. The production source reads config and
/// asks the servers; tests supply their own.
#[async_trait]
pub trait DecisionModelSource: Send + Sync {
    /// Every model available now, in no particular order. Discovery errors
    /// leave a backend out rather than failing the list.
    async fn list(&self) -> Vec<DecisionModelEntry>;
    /// Open the model `wanted` names, whether or not `list` would show it.
    async fn open(&self, wanted: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>>;

    /// Why the listing may be missing models that are installed, as the
    /// sentence a person acts on (a local server that is not answering), or
    /// `None`. A refusal for "no model" says it in place of the install
    /// advice, which would be wrong for a model already on this computer.
    async fn unavailable_hint(&self) -> Option<String> {
        None
    }
}
