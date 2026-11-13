//! DR-33: the provider instance that carries a chat's decision models into its
//! tier.
//!
//! [`CallCapability::sample`](crate::privacy::CallCapability::sample) reads one
//! lock of the provider slot and asks `(tier, affiliation)` of what it finds.
//! It sees no session id, and five samplers build a throwaway pinned slot from
//! `Agent::provider()`. The only place a value reaches all of them, and Gate
//! B's direct reads, with no signature change and no census change, is the
//! provider instance in the slot. That is where the lead/worker composite put
//! its own fold, and it is where this one goes.
//!
//! [`DecisionBoundProvider`] wraps the chat's real provider. Its tier is
//! `least(inner, axes)` and its affiliation `composite_affiliation(inner,
//! axes)`; every other method delegates to the inner provider, so the row
//! always records the real provider (`get_name`, `restore_binding`), never the
//! wrapper. `the_decision_bound_provider_delegates_every_provider_method`
//! reads the `Provider` trait out of `base.rs` and fails when the wrapper's
//! impl misses a method, because a forgotten one silently takes the trait's
//! default.
//!
//! The wrapper is installed only by `Agent::install_bound_provider`, from the
//! session row (`privacy::decision_axes::from_row`). It never crosses chats:
//! the install unwraps an incoming wrapper first.

use std::sync::Arc;

use async_trait::async_trait;
use rmcp::model::Tool;

use super::base::{
    LeadWorkerProviderTrait, MessageStream, Provider, ProviderMetadata, ProviderSteerReceiver,
    ProviderUsage,
};
use super::errors::ProviderError;
use super::provider_binding::ProviderRestoreBinding;
use super::retry::RetryConfig;
use crate::conversation::message::Message;
use crate::conversation::Conversation;
use crate::model::ModelConfig;
use crate::privacy::affiliation::ModelAffiliation;
use crate::privacy::decision_axes::{DecisionAxes, LoweredBy};
use crate::privacy::ProviderTier;

/// A chat's language model, with the tier its standing decision models give it.
pub struct DecisionBoundProvider {
    inner: Arc<dyn Provider>,
    axes: DecisionAxes,
    tier: ProviderTier,
    affiliation: Option<ModelAffiliation>,
    lowered_by: Option<LoweredBy>,
}

impl DecisionBoundProvider {
    /// The provider to install for `provider` under `axes`. An incoming wrapper
    /// is unwrapped first, so a parent's wrapper handed to a child never brings
    /// the parent's axes. Identity axes install the inner provider itself, so a
    /// chat that names no decision model holds exactly the instance it holds
    /// today.
    pub fn bind(provider: Arc<dyn Provider>, axes: DecisionAxes) -> Arc<dyn Provider> {
        let inner = unwrap_decision_bound(provider);
        if axes.is_identity() {
            return inner;
        }
        let (tier, affiliation) = axes.apply(inner.tier(), inner.affiliation());
        let lowered_by = if axes.lowers(inner.tier()) {
            axes.lowered_by().cloned()
        } else {
            None
        };
        Arc::new(Self {
            inner,
            axes,
            tier,
            affiliation,
            lowered_by,
        })
    }

    pub fn inner(&self) -> &Arc<dyn Provider> {
        &self.inner
    }

    pub fn axes(&self) -> &DecisionAxes {
        &self.axes
    }
}

/// The real provider under any number of wrappers.
pub fn unwrap_decision_bound(provider: Arc<dyn Provider>) -> Arc<dyn Provider> {
    let mut provider = provider;
    while let Some(inner) = provider.decision_bound_inner() {
        provider = inner;
    }
    provider
}

/// Whether installing `axes` around `inner` would leave `current` exactly as it
/// is: the same inner instance, the same tier, affiliation and cause. Gate B
/// re-installs only when this is false, so an unchanged chat keeps its
/// instance (and a coding-agent child keeps its provider-local session).
pub fn install_is_current(
    current: &Arc<dyn Provider>,
    inner: &Arc<dyn Provider>,
    axes: &DecisionAxes,
) -> bool {
    let current_inner = unwrap_decision_bound(Arc::clone(current));
    if !Arc::ptr_eq(&current_inner, inner) {
        return false;
    }
    if axes.is_identity() {
        return current.decision_bound_inner().is_none();
    }
    // Over one inner instance, a non-identity fold is always Public with no
    // affiliation, so the tier and the cause are the whole of what can differ.
    // (The affiliation is not compared: it follows from the tier here, and a
    // hand-rolled affiliation comparison is what
    // `compatible_is_the_only_function_that_compares_two_affiliations` exists
    // to find.)
    let (tier, _) = axes.apply(inner.tier(), inner.affiliation());
    let lowered_by = axes
        .lowers(inner.tier())
        .then(|| axes.lowered_by())
        .flatten();
    current.decision_bound_inner().is_some()
        && current.tier() == tier
        && current.decision_lowered_by() == lowered_by
}

impl std::fmt::Debug for DecisionBoundProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecisionBoundProvider")
            .field("inner", &self.inner.get_name())
            .field("tier", &self.tier)
            .field("lowered_by", &self.lowered_by)
            .finish()
    }
}

#[async_trait]
impl Provider for DecisionBoundProvider {
    fn metadata() -> ProviderMetadata {
        // A wrapper, never registered: it is built only by `bind`.
        ProviderMetadata::empty()
    }

    fn get_name(&self) -> &str {
        self.inner.get_name()
    }

    fn computer_use_destination(&self) -> Option<String> {
        self.inner.computer_use_destination()
    }

    fn computer_use_destination_identity(&self) -> Option<String> {
        self.inner.computer_use_destination_identity()
    }

    fn restore_binding(&self) -> ProviderRestoreBinding {
        self.inner.restore_binding()
    }

    fn tier(&self) -> ProviderTier {
        self.tier
    }

    fn affiliation(&self) -> Option<ModelAffiliation> {
        self.affiliation
    }

    fn decision_bound_inner(&self) -> Option<Arc<dyn Provider>> {
        Some(Arc::clone(&self.inner))
    }

    fn decision_lowered_by(&self) -> Option<&LoweredBy> {
        self.lowered_by.as_ref()
    }

    fn uses_tool_bridge(&self) -> bool {
        self.inner.uses_tool_bridge()
    }

    fn uses_tool_bridge_for_tool_surface(&self) -> bool {
        self.inner.uses_tool_bridge_for_tool_surface()
    }

    async fn complete_with_model(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.inner
            .complete_with_model(model_config, system, messages, tools)
            .await
    }

    async fn complete(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.inner.complete(system, messages, tools).await
    }

    async fn complete_fast(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.inner.complete_fast(system, messages, tools).await
    }

    fn get_model_config(&self) -> ModelConfig {
        self.inner.get_model_config()
    }

    fn retry_config(&self) -> RetryConfig {
        self.inner.retry_config()
    }

    async fn validate_readiness(&self) -> Result<(), ProviderError> {
        self.inner.validate_readiness().await
    }

    async fn fetch_supported_models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        self.inner.fetch_supported_models().await
    }

    async fn check_credentials(&self) -> Result<(), ProviderError> {
        self.inner.check_credentials().await
    }

    async fn fetch_recommended_models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        self.inner.fetch_recommended_models().await
    }

    async fn map_to_canonical_model(
        &self,
        provider_model: &str,
    ) -> Result<Option<String>, ProviderError> {
        self.inner.map_to_canonical_model(provider_model).await
    }

    fn supports_embeddings(&self) -> bool {
        self.inner.supports_embeddings()
    }

    async fn supports_cache_control(&self) -> bool {
        self.inner.supports_cache_control().await
    }

    async fn create_embeddings(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, ProviderError> {
        self.inner.create_embeddings(texts).await
    }

    fn as_lead_worker(&self) -> Option<&dyn LeadWorkerProviderTrait> {
        self.inner.as_lead_worker()
    }

    async fn stream(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        self.inner.stream(system, messages, tools).await
    }

    async fn stream_with_steering(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
        steering: ProviderSteerReceiver,
    ) -> Result<MessageStream, ProviderError> {
        self.inner
            .stream_with_steering(system, messages, tools, steering)
            .await
    }

    fn supports_streaming(&self) -> bool {
        self.inner.supports_streaming()
    }

    fn supports_live_steering(&self) -> bool {
        self.inner.supports_live_steering()
    }

    fn supports_restart_steering(&self) -> bool {
        self.inner.supports_restart_steering()
    }

    fn supports_tool_calls(&self) -> bool {
        self.inner.supports_tool_calls()
    }

    fn get_active_model_name(&self) -> String {
        self.inner.get_active_model_name()
    }

    fn get_initial_user_messages(&self, messages: &Conversation) -> Vec<String> {
        self.inner.get_initial_user_messages(messages)
    }

    async fn generate_session_name(
        &self,
        messages: &Conversation,
    ) -> Result<String, ProviderError> {
        self.inner.generate_session_name(messages).await
    }

    fn create_session_name_prompt(&self, context: &[String]) -> String {
        self.inner.create_session_name_prompt(context)
    }

    async fn configure_oauth(&self) -> Result<(), ProviderError> {
        self.inner.configure_oauth().await
    }
}

#[cfg(test)]
#[path = "decision_bound_tests.rs"]
mod tests;
