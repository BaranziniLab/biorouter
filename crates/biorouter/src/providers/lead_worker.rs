use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::ops::Deref;
use std::sync::{Arc, Mutex};

use super::base::{
    LeadWorkerProviderTrait, MessageStream, Provider, ProviderMetadata, ProviderSteerReceiver,
    ProviderUsage,
};
use super::errors::ProviderError;
use super::provider_binding::{
    model_without_restore_marker as strip_restore_marker, ProviderRestoreBinding,
    RESTORE_CONFIG_KEY,
};
use crate::conversation::message::{Message, MessageContent};
use crate::model::ModelConfig;
use crate::privacy::affiliation::ModelAffiliation;
use crate::privacy::ProviderTier;
use rmcp::model::Tool;
use rmcp::model::{Content, RawContent};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LeadWorkerRoutingState {
    #[serde(default)]
    pub(crate) turn_count: usize,
    #[serde(default)]
    pub(crate) failure_count: usize,
    #[serde(default)]
    pub(crate) in_fallback_mode: bool,
    #[serde(default)]
    pub(crate) fallback_remaining: usize,
}

/// A tagged restore recipe. The version is part of the variant name so an older
/// binary rejects a shape it cannot reproduce instead of silently using the lead
/// for both halves.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum PersistedProviderConfig {
    LeadWorkerV2 {
        lead: ProviderRestoreBinding,
        worker: ProviderRestoreBinding,
        lead_turns: usize,
        failure_threshold: usize,
        fallback_turns: usize,
        config_generation: String,
        #[serde(default)]
        routing_state: LeadWorkerRoutingState,
    },
}

impl PersistedProviderConfig {
    pub(crate) fn from_model_config(model: &ModelConfig) -> Result<Option<Self>> {
        let Some(value) = model
            .request_params
            .as_ref()
            .and_then(|params| params.get(RESTORE_CONFIG_KEY))
        else {
            return Ok(None);
        };

        serde_json::from_value(value.clone())
            .context("invalid persisted provider configuration")
            .and_then(|persisted: Self| {
                let Self::LeadWorkerV2 { lead, worker, .. } = &persisted;
                lead.validate()?;
                worker.validate()?;
                Ok(persisted)
            })
            .map(Some)
    }

    fn with_routing_state(mut self, state: LeadWorkerRoutingState) -> Self {
        let Self::LeadWorkerV2 { routing_state, .. } = &mut self;
        *routing_state = state;
        self
    }

    fn with_temperature(mut self, temperature: f32) -> Self {
        let Self::LeadWorkerV2 { lead, worker, .. } = &mut self;
        lead.model_mut().temperature = Some(temperature);
        worker.model_mut().temperature = Some(temperature);
        self
    }

    fn for_new_session(mut self) -> Self {
        let Self::LeadWorkerV2 {
            config_generation, ..
        } = &mut self;
        *config_generation = uuid::Uuid::new_v4().to_string();
        self
    }

    fn to_model_config(&self) -> ModelConfig {
        let Self::LeadWorkerV2 { lead, .. } = self;
        let mut model = lead.model().clone();
        model
            .request_params
            .get_or_insert_with(Default::default)
            .insert(
                RESTORE_CONFIG_KEY.to_string(),
                serde_json::to_value(self)
                    .expect("lead/worker restore configuration must serialize"),
            );
        model
    }
}

pub(crate) fn model_config_without_restore_marker(model: ModelConfig) -> ModelConfig {
    strip_restore_marker(model)
}

pub(crate) fn model_config_with_composite_temperature(
    mut model: ModelConfig,
    temperature: f32,
) -> Result<ModelConfig> {
    let persisted = PersistedProviderConfig::from_model_config(&model)?;
    model.temperature = Some(temperature);
    Ok(match persisted {
        Some(persisted) => persisted.with_temperature(temperature).to_model_config(),
        None => model,
    })
}

/// Copy a composite's current routing snapshot into a distinct session binding.
///
/// The provider factory reconstructs both component providers from this recipe,
/// so the new session gets its own provider-local state as well as its own
/// lead/worker counters. A fresh generation keeps each session's conditional
/// snapshot writes scoped to the binding that its row actually describes.
pub(crate) fn model_config_for_session_fork(model: &ModelConfig) -> Result<Option<ModelConfig>> {
    Ok(PersistedProviderConfig::from_model_config(model)?
        .map(|persisted| persisted.for_new_session().to_model_config()))
}

/// A provider that switches between a lead model and a worker model based on turn count
/// and can fallback to lead model on consecutive failures
///
/// `Clone` shares the routing-state lock, so every clone observes and updates
/// the same counters and fallback flags. The restore recipe is immutable and
/// contains only provider/model settings plus those non-secret counters.
#[derive(Clone)]
pub struct LeadWorkerProvider {
    lead_provider: Arc<dyn Provider>,
    worker_provider: Arc<dyn Provider>,
    persisted_provider_config: PersistedProviderConfig,
    lead_turns: usize,
    max_failures_before_fallback: usize,
    fallback_turns: usize,
    routing_state: Arc<Mutex<LeadWorkerRoutingState>>,
}

impl LeadWorkerProvider {
    /// Create a new LeadWorkerProvider
    ///
    /// # Arguments
    /// * `lead_provider` - The provider to use for the initial turns
    /// * `worker_provider` - The provider to use after lead_turns
    /// * `lead_turns` - Number of turns to use the lead provider (default: 3)
    pub fn new(
        lead_provider: Arc<dyn Provider>,
        worker_provider: Arc<dyn Provider>,
        lead_turns: Option<usize>,
    ) -> Self {
        Self::new_with_settings(
            lead_provider,
            worker_provider,
            lead_turns.unwrap_or(3),
            2,
            2,
        )
    }

    /// Create a new LeadWorkerProvider with custom settings
    ///
    /// # Arguments
    /// * `lead_provider` - The provider to use for the initial turns
    /// * `worker_provider` - The provider to use after lead_turns
    /// * `lead_turns` - Number of turns to use the lead provider
    /// * `failure_threshold` - Number of consecutive failures before fallback
    /// * `fallback_turns` - Number of turns to use lead model in fallback mode
    pub fn new_with_settings(
        lead_provider: Arc<dyn Provider>,
        worker_provider: Arc<dyn Provider>,
        lead_turns: usize,
        failure_threshold: usize,
        fallback_turns: usize,
    ) -> Self {
        Self::new_with_settings_and_state(
            lead_provider,
            worker_provider,
            lead_turns,
            failure_threshold,
            fallback_turns,
            uuid::Uuid::new_v4().to_string(),
            LeadWorkerRoutingState::default(),
        )
    }

    pub(crate) fn new_with_settings_and_state(
        lead_provider: Arc<dyn Provider>,
        worker_provider: Arc<dyn Provider>,
        lead_turns: usize,
        failure_threshold: usize,
        fallback_turns: usize,
        config_generation: String,
        routing_state: LeadWorkerRoutingState,
    ) -> Self {
        let persisted_provider_config = PersistedProviderConfig::LeadWorkerV2 {
            lead: lead_provider.restore_binding(),
            worker: worker_provider.restore_binding(),
            lead_turns,
            failure_threshold,
            fallback_turns,
            config_generation,
            routing_state,
        };

        Self {
            lead_provider,
            worker_provider,
            persisted_provider_config,
            lead_turns,
            max_failures_before_fallback: failure_threshold,
            fallback_turns,
            routing_state: Arc::new(Mutex::new(routing_state)),
        }
    }

    /// Reset the turn counter and failure tracking (useful for new conversations)
    pub async fn reset_turn_count(&self) {
        *self.routing_state.lock().unwrap() = LeadWorkerRoutingState::default();
    }

    /// Get the current turn count
    pub async fn get_turn_count(&self) -> usize {
        self.routing_state.lock().unwrap().turn_count
    }

    /// Get the current failure count
    pub async fn get_failure_count(&self) -> usize {
        self.routing_state.lock().unwrap().failure_count
    }

    /// Check if currently in fallback mode
    pub async fn is_in_fallback_mode(&self) -> bool {
        self.routing_state.lock().unwrap().in_fallback_mode
    }

    fn routing_state(&self) -> LeadWorkerRoutingState {
        *self.routing_state.lock().unwrap()
    }

    /// Get the currently active provider based on turn count and fallback state
    fn get_active_provider(&self) -> Arc<dyn Provider> {
        let state = self.routing_state();

        // Use lead provider if we're in initial turns OR in fallback mode
        if state.turn_count < self.lead_turns || state.in_fallback_mode {
            Arc::clone(&self.lead_provider)
        } else {
            Arc::clone(&self.worker_provider)
        }
    }

    /// Handle the result of a completion attempt and update failure tracking
    async fn handle_completion_result(
        &self,
        result: &Result<(Message, ProviderUsage), ProviderError>,
    ) {
        match result {
            Ok((message, _usage)) => {
                // Check for task-level failures in the response
                let has_task_failure = self.detect_task_failures(message).await;
                let mut state = self.routing_state.lock().unwrap();

                if has_task_failure {
                    // Task failure detected - increment failure count
                    state.failure_count += 1;

                    tracing::warn!(
                        "Task failure detected in response (failure count: {})",
                        state.failure_count
                    );

                    // Check if we should trigger fallback
                    if state.turn_count >= self.lead_turns
                        && !state.in_fallback_mode
                        && state.failure_count >= self.max_failures_before_fallback
                    {
                        state.in_fallback_mode = self.fallback_turns > 0;
                        state.fallback_remaining = self.fallback_turns;
                        state.failure_count = 0;

                        tracing::warn!(
                            "🔄 SWITCHING TO LEAD MODEL: Entering fallback mode after {} consecutive task failures - using lead model for {} turns",
                            self.max_failures_before_fallback,
                            self.fallback_turns
                        );
                    }
                } else {
                    // Success - reset failure count and handle fallback mode
                    state.failure_count = 0;

                    if state.in_fallback_mode {
                        state.fallback_remaining = state.fallback_remaining.saturating_sub(1);
                        if state.fallback_remaining == 0 {
                            state.in_fallback_mode = false;
                            tracing::info!("✅ SWITCHING BACK TO WORKER MODEL: Exiting fallback mode - worker model resumed");
                        }
                    }
                }

                // Increment turn count on any completion (success or task failure)
                state.turn_count += 1;
            }
            Err(_) => {
                // Technical failure - just log and let it bubble up
                // For technical failures (API/LLM issues), we don't want to second-guess
                // the model choice - just let the default model handle it
                tracing::warn!(
                    "Technical failure detected - API/LLM issue, will use default model"
                );

                // Don't increment turn count or failure tracking for technical failures
                // as these are temporary infrastructure issues, not model capability issues
            }
        }
    }

    /// Detect task-level failures in the model's response
    async fn detect_task_failures(&self, message: &Message) -> bool {
        let mut failure_indicators = 0;

        for content in &message.content {
            match content {
                MessageContent::ToolRequest(tool_request) => {
                    // Check if tool request itself failed (malformed, etc.)
                    if tool_request.tool_call.is_err() {
                        failure_indicators += 1;
                        tracing::debug!(
                            "Failed tool request detected: {:?}",
                            tool_request.tool_call
                        );
                    }
                }
                MessageContent::ToolResponse(tool_response) => {
                    // Check if tool execution failed
                    if let Err(tool_error) = &tool_response.tool_result {
                        failure_indicators += 1;
                        tracing::debug!("Tool execution failure detected: {:?}", tool_error);
                    } else if let Ok(result) = &tool_response.tool_result {
                        // Check tool output for error indicators
                        if self.contains_error_indicators(&result.content) {
                            failure_indicators += 1;
                            tracing::debug!("Tool output contains error indicators");
                        }
                    }
                }
                MessageContent::Text(text_content) => {
                    // Check for user correction patterns or error acknowledgments
                    if self.contains_user_correction_patterns(&text_content.text) {
                        failure_indicators += 1;
                        tracing::debug!("User correction pattern detected in text");
                    }
                }
                _ => {}
            }
        }

        // Consider it a failure if we have multiple failure indicators
        failure_indicators >= 1
    }

    /// Check if tool output contains error indicators
    fn contains_error_indicators(&self, contents: &[Content]) -> bool {
        for content in contents {
            if let RawContent::Text(text_content) = content.deref() {
                let text_lower = text_content.text.to_lowercase();

                // Common error patterns in tool outputs
                if text_lower.contains("error:")
                    || text_lower.contains("failed:")
                    || text_lower.contains("exception:")
                    || text_lower.contains("traceback")
                    || text_lower.contains("syntax error")
                    || text_lower.contains("permission denied")
                    || text_lower.contains("file not found")
                    || text_lower.contains("command not found")
                    || text_lower.contains("compilation failed")
                    || text_lower.contains("test failed")
                    || text_lower.contains("assertion failed")
                {
                    return true;
                }
            }
        }
        false
    }

    /// Check for user correction patterns in text
    fn contains_user_correction_patterns(&self, text: &str) -> bool {
        let text_lower = text.to_lowercase();

        // Patterns indicating user is correcting or expressing dissatisfaction
        text_lower.contains("that's wrong")
            || text_lower.contains("that's not right")
            || text_lower.contains("that doesn't work")
            || text_lower.contains("try again")
            || text_lower.contains("let me correct")
            || text_lower.contains("actually, ")
            || text_lower.contains("no, that's")
            || text_lower.contains("that's incorrect")
            || text_lower.contains("fix this")
            || text_lower.contains("this is broken")
            || text_lower.contains("this doesn't")
            || text_lower.starts_with("no,")
            || text_lower.starts_with("wrong")
            || text_lower.starts_with("incorrect")
    }

    async fn stream_from_active(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
        steering: Option<ProviderSteerReceiver>,
    ) -> Result<MessageStream, ProviderError> {
        let provider = self.get_active_provider();
        super::base::set_current_model(&provider.get_model_config().model_name);

        let inner = match steering {
            Some(steering) if provider.supports_live_steering() => {
                provider
                    .stream_with_steering(system, messages, tools, steering)
                    .await?
            }
            Some(_) | None => provider.stream(system, messages, tools).await?,
        };
        let accounting = self.clone();

        let stream = async_stream::try_stream! {
            let mut content: Vec<MessageContent> = Vec::new();
            let mut usage: Option<ProviderUsage> = None;
            futures::pin_mut!(inner);

            while let Some(item) = futures::StreamExt::next(&mut inner).await {
                match item {
                    Ok((message, item_usage, pending)) => {
                        if let Some(message) = &message {
                            content.extend(message.content.iter().cloned());
                        }
                        if let Some(item_usage) = &item_usage {
                            usage = Some(item_usage.clone());
                        }
                        yield (message, item_usage, pending);
                    }
                    Err(e) => {
                        accounting
                            .handle_completion_result(&Err(ProviderError::RequestFailed(
                                e.to_string(),
                            )))
                            .await;
                        Err(e)?;
                        return;
                    }
                }
            }

            let message = Message::new(
                rmcp::model::Role::Assistant,
                chrono::Utc::now().timestamp(),
                content,
            );
            let usage = usage.unwrap_or_else(|| {
                ProviderUsage::new(provider.get_model_config().model_name, Default::default())
            });
            accounting.handle_completion_result(&Ok((message, usage))).await;
        };

        Ok(Box::pin(stream))
    }
}

impl LeadWorkerProviderTrait for LeadWorkerProvider {
    /// Get information about the lead and worker models for logging
    fn get_model_info(&self) -> (String, String) {
        let lead_model = self.lead_provider.get_model_config().model_name;
        let worker_model = self.worker_provider.get_model_config().model_name;
        (lead_model, worker_model)
    }

    /// Get the currently active model name
    fn get_active_model(&self) -> String {
        // Read from the global store which was set during complete()
        use super::base::get_current_model;
        get_current_model().unwrap_or_else(|| {
            // Fallback to lead model if no current model is set
            self.lead_provider.get_model_config().model_name
        })
    }

    /// Get (lead_turns, failure_threshold, fallback_turns)
    fn get_settings(&self) -> (usize, usize, usize) {
        (
            self.lead_turns,
            self.max_failures_before_fallback,
            self.fallback_turns,
        )
    }

    fn get_config_generation(&self) -> &str {
        let PersistedProviderConfig::LeadWorkerV2 {
            config_generation, ..
        } = &self.persisted_provider_config;
        config_generation
    }
}

#[async_trait]
impl Provider for LeadWorkerProvider {
    fn metadata() -> ProviderMetadata {
        // This is a wrapper provider, so we return minimal metadata
        ProviderMetadata::new(
            "lead_worker",
            "Lead/Worker Provider",
            "A provider that switches between lead and worker models based on turn count",
            "",     // No default model as this is determined by the wrapped providers
            vec![], // No known models as this depends on wrapped providers
            "",     // No doc link
            vec![], // No config keys as configuration is done through wrapped providers
        )
    }

    fn get_name(&self) -> &str {
        // Return the lead provider's name as the default
        self.lead_provider.get_name()
    }

    fn computer_use_destination(&self) -> Option<String> {
        let lead = self.lead_provider.computer_use_destination();
        let worker = self.worker_provider.computer_use_destination();
        if lead.is_none() && worker.is_none() {
            return None;
        }
        Some(format!(
            "lead: {}; worker: {}",
            lead.as_deref().unwrap_or("destination not reported"),
            worker.as_deref().unwrap_or("destination not reported")
        ))
    }

    fn computer_use_destination_identity(&self) -> Option<String> {
        let lead = self.lead_provider.computer_use_destination_identity();
        let worker = self.worker_provider.computer_use_destination_identity();
        if lead.is_none() && worker.is_none() {
            return None;
        }
        Some(super::base::computer_use_destination_digest(&format!(
            "{lead:?}\0{worker:?}"
        )))
    }

    /// The composite override. `get_name()` above answers for the lead alone, so
    /// anything keyed on it would badge a private-lead/public-worker pair
    /// Private — while the worker sees the whole transcript.
    fn tier(&self) -> ProviderTier {
        ProviderTier::least(self.lead_provider.tier(), self.worker_provider.tier())
    }

    /// The same override on DR-26's third axis, and it must exist for the same
    /// reason: the transcript reaches **both** endpoints, so whose agreements
    /// cover the pair is what both halves agree on, never the lead's alone.
    ///
    /// ⚠ Leaving this on the trait default is not a missing nicety — it produces
    /// the one combination DR-26's vocabulary says cannot exist, tier `Private`
    /// with affiliation `None`, where `None` is specified to mean *"a public
    /// model; the tier gates already hold, so affiliation never applies"*. A gate
    /// that short-circuits on it skips the cross-affiliation check for a private
    /// composite: fail-**open**, in exactly the case DR-26 exists to catch.
    ///
    /// ⚠ The affiliation census in `factory.rs` structurally cannot catch that:
    /// it enumerates what `register_builtin_providers` registers, and this
    /// composite is never registered — `factory::create` constructs it directly
    /// whenever `BIOROUTER_LEAD_MODEL` is set. The fold itself, and why `Local`
    /// is its identity, is documented on `providers::composite_affiliation`.
    fn affiliation(&self) -> Option<ModelAffiliation> {
        crate::providers::composite_affiliation(
            self.lead_provider.affiliation(),
            self.worker_provider.affiliation(),
        )
    }

    /// The same conservative fold on the tool-calling capability, and for the
    /// same reason `tier` is folded: a turn lands on the lead *or* the worker,
    /// so the pair can only drive a Biorouter-run tool loop if **both** halves
    /// can. Leaving it on the trait default would advertise the capability of
    /// whichever half happened to be asked, and a run that fell through to the
    /// other one would come back with no tool calls and look like a model that
    /// had nothing to do.
    fn supports_tool_calls(&self) -> bool {
        self.lead_provider.supports_tool_calls() && self.worker_provider.supports_tool_calls()
    }

    /// The third override that exists because `get_name()` answers for the lead.
    ///
    /// **Either** half, and the asymmetry with `tier`'s `least` and
    /// `affiliation`'s union is the point rather than an inconsistency. Those two
    /// fold *constraints*, so the pair must be no more permissive than its most
    /// restrictive half. This folds a *need*: the bridge is the only channel by
    /// which either child can use a Biorouter tool at all, so a half that needs
    /// one and does not get it runs tool-less — the exact failure the bridge was
    /// built to remove.
    ///
    /// Asking the active half instead was considered and rejected.
    /// `get_active_provider` depends on the turn count and the fallback state,
    /// both of which advance *inside* `complete_with_model`; the grant is issued
    /// before that call and its lease dropped after it, so a bridge decided from
    /// the pre-call state would be right only until the pair switched, and wrong
    /// silently. Whether this turn is a lead turn is also not knowable to
    /// `issue_tool_bridge`, which holds a `dyn Provider` and no turn counter.
    ///
    /// What an unused grant costs is one nonce in a process-global map for the
    /// length of one provider call, revoked when the lease drops. What a missing
    /// one costs is a child agent that can read nothing and change nothing, and
    /// reports the filesystem as read-only. The trade is not close.
    fn uses_tool_bridge(&self) -> bool {
        self.lead_provider.uses_tool_bridge() || self.worker_provider.uses_tool_bridge()
    }

    fn uses_tool_bridge_for_tool_surface(&self) -> bool {
        self.get_active_provider()
            .uses_tool_bridge_for_tool_surface()
    }

    /// Streaming is forwarded, or a lead/worker pair silently loses it.
    ///
    /// `Provider`'s defaults are `supports_streaming() == false` and a `stream()`
    /// that returns `NotImplemented`. A wrapper that does not override them
    /// therefore reports "cannot stream" no matter what it wraps, and the agent
    /// takes the blocking branch — so pairing two streaming providers would
    /// silently turn streaming off, with nothing failing to say so.
    ///
    /// **Both halves must agree.** The answer is `&&`, not `||`, because this is
    /// answered once per turn while the *active* provider can change between
    /// turns: claiming support because the lead has it would send a turn served
    /// by a non-streaming worker down `stream()`, straight into the trait's
    /// `NotImplemented`.
    fn supports_streaming(&self) -> bool {
        self.lead_provider.supports_streaming() && self.worker_provider.supports_streaming()
    }

    /// Live steering is selected per turn inside [`Self::stream_from_active`].
    /// Advertising the union lets a capable active half receive the channel;
    /// when the selected half is not capable, its receiver is dropped and the
    /// agent's existing acknowledgement fallback defers the steer to the next
    /// loop boundary.
    fn supports_live_steering(&self) -> bool {
        self.lead_provider.supports_live_steering() || self.worker_provider.supports_live_steering()
    }

    /// Restart steering is a property of the provider selected for this exact
    /// turn. Unlike live steering, there is no receiver for
    /// `stream_from_active` to route or reject: the agent itself drops the outer
    /// stream. Ask the same routing snapshot that `stream_from_active` asks so a
    /// mixed pair restarts only when its active half explicitly supports it.
    fn supports_restart_steering(&self) -> bool {
        self.get_active_provider().supports_restart_steering()
    }

    /// Stream from the active provider **and keep the rotation accounting**.
    ///
    /// ⚠ The accounting is the whole difficulty here, and omitting it silently
    /// disables the feature. `turn_count` is incremented in exactly one place —
    /// [`Self::handle_completion_result`] — which used to be reached only from
    /// `complete_with_model`. A `stream()` that just forwarded would leave
    /// `turn_count` at 0 forever, so `count < self.lead_turns` would always hold
    /// and **the worker model would never be used**; task-failure detection and
    /// the fallback-to-lead behaviour would never run either. Nothing would
    /// fail — the pair would simply, quietly, stop being a lead/worker pair.
    ///
    /// So the returned stream accumulates what it forwards and settles the turn
    /// when it ends, exactly as the blocking path settles it when the call
    /// returns. `self` is cloned into the stream, and the clone shares the same
    /// `Arc` counters, so it is the same accounting and not a copy of it.
    async fn stream(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        self.stream_from_active(system, messages, tools, None).await
    }

    async fn stream_with_steering(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
        steering: ProviderSteerReceiver,
    ) -> Result<MessageStream, ProviderError> {
        self.stream_from_active(system, messages, tools, Some(steering))
            .await
    }

    fn get_model_config(&self) -> ModelConfig {
        self.persisted_provider_config
            .clone()
            .with_routing_state(self.routing_state())
            .to_model_config()
    }

    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        // Get the active provider
        let provider = self.get_active_provider();

        // Log which provider is being used
        let state = self.routing_state();

        let provider_type = if state.turn_count < self.lead_turns {
            "lead (initial)"
        } else if state.in_fallback_mode {
            "lead (fallback)"
        } else {
            "worker"
        };

        // Get the active model name and update the global store
        let active_model_name = if state.turn_count < self.lead_turns || state.in_fallback_mode {
            self.lead_provider.get_model_config().model_name.clone()
        } else {
            self.worker_provider.get_model_config().model_name.clone()
        };

        // Update the global current model store
        super::base::set_current_model(&active_model_name);

        if state.in_fallback_mode {
            tracing::info!(
                "🔄 Using {} provider for turn {} (FALLBACK MODE: {} turns remaining) - Model: {}",
                provider_type,
                state.turn_count + 1,
                state.fallback_remaining,
                active_model_name
            );
        } else {
            tracing::info!(
                "Using {} provider for turn {} (lead_turns: {}) - Model: {}",
                provider_type,
                state.turn_count + 1,
                self.lead_turns,
                active_model_name
            );
        }

        let result = provider.complete(system, messages, tools).await;
        let selected_provider_name = provider.get_name().to_string();

        // For technical failures, try with default model (lead provider) instead.
        // Keep the concrete successful child provider on ProviderUsage so the
        // accounting ledger does not attribute worker spend to the wrapper's
        // lead-provider get_name().
        let (mut final_result, serving_provider_name) = match result {
            Err(original_error) => {
                tracing::warn!("Technical failure with {} provider, retrying with default model (lead provider)", provider_type);

                let default_result = self.lead_provider.complete(system, messages, tools).await;

                match default_result {
                    Ok(value) => {
                        tracing::info!(
                            "✅ Default model (lead provider) succeeded after technical failure"
                        );
                        (Ok(value), self.lead_provider.get_name().to_string())
                    }
                    Err(_) => {
                        tracing::error!("❌ Default model (lead provider) also failed - returning original error");
                        (Err(original_error), selected_provider_name)
                    }
                }
            }
            Ok(value) => (Ok(value), selected_provider_name),
        };

        if let Ok((_, usage)) = &mut final_result {
            usage.provider = Some(serving_provider_name);
        }

        // Handle the result and update tracking (only for successful completions)
        self.handle_completion_result(&final_result).await;

        final_result
    }

    async fn fetch_supported_models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        // Combine models from both providers
        let lead_models = self.lead_provider.fetch_supported_models().await?;
        let worker_models = self.worker_provider.fetch_supported_models().await?;

        match (lead_models, worker_models) {
            (Some(lead), Some(worker)) => {
                let mut all_models = lead;
                all_models.extend(worker);
                all_models.sort();
                all_models.dedup();
                Ok(Some(all_models))
            }
            (Some(models), None) | (None, Some(models)) => Ok(Some(models)),
            (None, None) => Ok(None),
        }
    }

    fn supports_embeddings(&self) -> bool {
        // Support embeddings if either provider supports them
        self.lead_provider.supports_embeddings() || self.worker_provider.supports_embeddings()
    }

    async fn create_embeddings(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, ProviderError> {
        // Use the lead provider for embeddings if it supports them, otherwise use worker
        if self.lead_provider.supports_embeddings() {
            self.lead_provider.create_embeddings(texts).await
        } else if self.worker_provider.supports_embeddings() {
            self.worker_provider.create_embeddings(texts).await
        } else {
            Err(ProviderError::ExecutionError(
                "Neither lead nor worker provider supports embeddings".to_string(),
            ))
        }
    }

    /// Check if this provider is a LeadWorkerProvider
    fn as_lead_worker(&self) -> Option<&dyn LeadWorkerProviderTrait> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    /// Issue #108. A turn lands on the lead *or* the worker, so the pair can
    /// drive a Biorouter-run tool loop only if BOTH halves can — the same
    /// conservative fold `tier` takes, and for the same reason. On the trait
    /// default the composite would report whichever half was asked, and a run
    /// that fell through to the other one would come back empty.
    #[test]
    fn the_composite_can_call_tools_only_when_both_halves_can() {
        use super::LeadWorkerProvider;
        use crate::conversation::message::Message;
        use crate::model::ModelConfig;
        use crate::providers::base::{Provider, ProviderMetadata, ProviderUsage};
        use crate::providers::claude_code::ClaudeCodeProvider;
        use crate::providers::errors::ProviderError;
        use crate::providers::ollama::OllamaProvider;
        use rmcp::model::Tool;
        use std::sync::Arc;

        struct ToollessProvider;

        #[async_trait::async_trait]
        impl Provider for ToollessProvider {
            fn metadata() -> ProviderMetadata {
                ProviderMetadata::empty()
            }

            fn get_name(&self) -> &str {
                "tool-less-fixture"
            }

            fn get_model_config(&self) -> ModelConfig {
                ModelConfig::new_or_fail("tool-less-model")
            }

            fn supports_tool_calls(&self) -> bool {
                false
            }

            async fn complete_with_model(
                &self,
                _model_config: &ModelConfig,
                _system: &str,
                _messages: &[Message],
                _tools: &[Tool],
            ) -> Result<(Message, ProviderUsage), ProviderError> {
                unreachable!("the capability check never calls the provider")
            }
        }

        let config = crate::config::declarative_providers::DeclarativeProviderConfig {
            name: "ingest-fixture".to_string(),
            engine: crate::config::declarative_providers::ProviderEngine::Ollama,
            display_name: "Ingest fixture".to_string(),
            description: None,
            api_key_env: "NOT_USED".to_string(),
            base_url: "http://localhost:11434".to_string(),
            models: vec![],
            headers: None,
            timeout_seconds: None,
            supports_streaming: None,
        };
        let can: Arc<dyn Provider> = Arc::new(
            OllamaProvider::from_custom_config(ModelConfig::new_or_fail("qwen3"), config)
                .expect("a declarative ollama provider must construct"),
        );
        let coding_agent: Arc<dyn Provider> = Arc::new(ClaudeCodeProvider::for_tests(
            std::path::PathBuf::from("/usr/bin/claude"),
            "claude-sonnet-4-6",
        ));
        let cannot: Arc<dyn Provider> = Arc::new(ToollessProvider);
        assert!(can.supports_tool_calls());
        assert!(coding_agent.supports_tool_calls());
        assert!(!cannot.supports_tool_calls());

        let bridged_pair = LeadWorkerProvider::new(can.clone(), coding_agent, Some(3));
        assert!(bridged_pair.supports_tool_calls());

        for (lead, worker, expected) in [
            (&can, &can, true),
            (&can, &cannot, false),
            (&cannot, &can, false),
            (&cannot, &cannot, false),
        ] {
            let composite = LeadWorkerProvider::new(Arc::clone(lead), Arc::clone(worker), Some(3));
            assert_eq!(
                composite.supports_tool_calls(),
                expected,
                "lead={} worker={}",
                lead.get_name(),
                worker.get_name()
            );
        }
    }

    use super::*;
    use crate::conversation::message::{Message, MessageContent};
    use crate::providers::base::{ProviderMetadata, ProviderUsage, Usage};
    use chrono::Utc;
    use rmcp::model::{AnnotateAble, RawTextContent, Role};

    #[derive(Clone)]
    struct MockProvider {
        name: String,
        model_config: ModelConfig,
    }

    #[async_trait]
    impl Provider for MockProvider {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::empty()
        }

        fn get_name(&self) -> &str {
            &self.name
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model_config.clone()
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            Ok((
                Message::new(
                    Role::Assistant,
                    Utc::now().timestamp(),
                    vec![MessageContent::Text(
                        RawTextContent {
                            text: format!("Response from {}", self.name),
                            meta: None,
                        }
                        .no_annotation(),
                    )],
                ),
                ProviderUsage::new(self.name.clone(), Usage::default()),
            ))
        }
    }

    /// A pair whose WORKER is a coding agent still gets a tool bridge.
    ///
    /// The worker is the half that runs most of a conversation's turns — every
    /// turn after `lead_turns` — so a pair configured `anthropic` + `codex` is a
    /// pair that is mostly Codex. Deciding from `get_name()` asked the lead and
    /// answered "no bridge", and the child then ran with no access to any of
    /// Biorouter's tools: it could not read a file, could not query SPOKE, and
    /// (measured on the Codex path) reported the filesystem as read-only.
    ///
    /// The two assertions below are one claim in two halves, and the second is
    /// what makes the first mean anything: the pair must answer yes *while*
    /// `provider_uses_bridge(get_name())` answers no. Without that line the test
    /// would still pass against the name lookup it exists to rule out.
    #[tokio::test]
    async fn a_pair_gets_a_bridge_when_either_half_needs_one() {
        let coding_agent = || {
            Arc::new(MockProvider {
                name: "codex".to_string(),
                model_config: ModelConfig::new_or_fail("gpt-5.5"),
            })
        };
        let plain = || {
            Arc::new(MockProvider {
                name: "anthropic".to_string(),
                model_config: ModelConfig::new_or_fail("claude-opus-4"),
            })
        };

        let worker_is_codex = LeadWorkerProvider::new(plain(), coding_agent(), Some(3));
        assert!(
            !crate::providers::coding_agent::bridge::provider_uses_bridge(
                worker_is_codex.get_name()
            ),
            "the premise: `get_name()` on a pair is the LEAD's name, so a name \
             lookup cannot see the worker at all"
        );
        assert!(
            worker_is_codex.uses_tool_bridge(),
            "the worker runs every turn past the lead turns; without a bridge that \
             child agent has no way to reach a single Biorouter tool"
        );
        assert!(
            !worker_is_codex.uses_tool_bridge_for_tool_surface(),
            "the ordinary lead keeps its request tools even though the worker needs a lease"
        );

        let worker_turn = LeadWorkerProvider::new_with_settings_and_state(
            plain(),
            coding_agent(),
            3,
            2,
            2,
            "worker-turn".into(),
            LeadWorkerRoutingState {
                turn_count: 3,
                ..LeadWorkerRoutingState::default()
            },
        );
        assert!(
            worker_turn.uses_tool_bridge_for_tool_surface(),
            "the active coding-agent worker receives the bridge roster"
        );

        let lead_is_codex = LeadWorkerProvider::new(coding_agent(), plain(), Some(3));
        assert!(
            lead_is_codex.uses_tool_bridge(),
            "the lead's turns need the bridge just as much"
        );

        let neither = LeadWorkerProvider::new(plain(), plain(), Some(3));
        assert!(
            !neither.uses_tool_bridge(),
            "a pair of ordinary providers receives its tools in the request; a \
             grant here would be a live capability on every turn with nothing to \
             use it"
        );
    }

    #[tokio::test]
    async fn test_lead_worker_switching() {
        let lead_provider = Arc::new(MockProvider {
            name: "lead".to_string(),
            model_config: ModelConfig::new_or_fail("lead-model"),
        });

        let worker_provider = Arc::new(MockProvider {
            name: "worker".to_string(),
            model_config: ModelConfig::new_or_fail("worker-model"),
        });

        let provider = LeadWorkerProvider::new(lead_provider, worker_provider, Some(3));

        // First three turns should use lead provider
        for i in 0..3 {
            let (_message, usage) = provider.complete("system", &[], &[]).await.unwrap();
            assert_eq!(usage.model, "lead");
            assert_eq!(usage.provider.as_deref(), Some("lead"));
            assert_eq!(provider.get_turn_count().await, i + 1);
            assert!(!provider.is_in_fallback_mode().await);
        }

        // Subsequent turns should use worker provider
        for i in 3..6 {
            let (_message, usage) = provider.complete("system", &[], &[]).await.unwrap();
            assert_eq!(usage.model, "worker");
            assert_eq!(usage.provider.as_deref(), Some("worker"));
            assert_eq!(provider.get_turn_count().await, i + 1);
            assert!(!provider.is_in_fallback_mode().await);
        }

        // Reset and verify it goes back to lead
        provider.reset_turn_count().await;
        assert_eq!(provider.get_turn_count().await, 0);
        assert_eq!(provider.get_failure_count().await, 0);
        assert!(!provider.is_in_fallback_mode().await);

        let (_message, usage) = provider.complete("system", &[], &[]).await.unwrap();
        assert_eq!(usage.model, "lead");
        assert_eq!(usage.provider.as_deref(), Some("lead"));
    }

    #[tokio::test]
    async fn test_technical_failure_retry() {
        let lead_provider = Arc::new(MockFailureProvider {
            name: "lead".to_string(),
            model_config: ModelConfig::new_or_fail("lead-model"),
            should_fail: false, // Lead provider works
        });

        let worker_provider = Arc::new(MockFailureProvider {
            name: "worker".to_string(),
            model_config: ModelConfig::new_or_fail("worker-model"),
            should_fail: true, // Worker will fail
        });

        let provider = LeadWorkerProvider::new(lead_provider, worker_provider, Some(2));

        // First two turns use lead (should succeed)
        for _i in 0..2 {
            let result = provider.complete("system", &[], &[]).await;
            assert!(result.is_ok());
            assert_eq!(result.unwrap().1.model, "lead");
            assert!(!provider.is_in_fallback_mode().await);
        }

        // Next turn uses worker (will fail, but should retry with lead and succeed)
        let result = provider.complete("system", &[], &[]).await;
        assert!(result.is_ok()); // Should succeed because lead provider is used as fallback
        let usage = result.unwrap().1;
        assert_eq!(usage.model, "lead"); // Should be lead provider
        assert_eq!(usage.provider.as_deref(), Some("lead"));
        assert_eq!(provider.get_failure_count().await, 0); // No failure tracking for technical failures
        assert!(!provider.is_in_fallback_mode().await); // Not in fallback mode

        // Another turn - should still try worker first, then retry with lead
        let result = provider.complete("system", &[], &[]).await;
        assert!(result.is_ok()); // Should succeed because lead provider is used as fallback
        assert_eq!(result.unwrap().1.model, "lead"); // Should be lead provider
        assert_eq!(provider.get_failure_count().await, 0); // Still no failure tracking
        assert!(!provider.is_in_fallback_mode().await); // Still not in fallback mode
    }

    #[tokio::test]
    async fn test_fallback_on_task_failures() {
        // Test that task failures (not technical failures) still trigger fallback mode
        // This would need a different mock that simulates task failures in successful responses
        // For now, we'll test the fallback mode functionality directly
        let lead_provider = Arc::new(MockFailureProvider {
            name: "lead".to_string(),
            model_config: ModelConfig::new_or_fail("lead-model"),
            should_fail: false,
        });

        let worker_provider = Arc::new(MockFailureProvider {
            name: "worker".to_string(),
            model_config: ModelConfig::new_or_fail("worker-model"),
            should_fail: false,
        });

        let provider = LeadWorkerProvider::new(lead_provider, worker_provider, Some(2));

        // Simulate being in fallback mode
        {
            *provider.routing_state.lock().unwrap() = LeadWorkerRoutingState {
                turn_count: 4,
                failure_count: 0,
                in_fallback_mode: true,
                fallback_remaining: 2,
            };
        }

        // Should use lead provider in fallback mode
        let result = provider.complete("system", &[], &[]).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().1.model, "lead");
        assert!(provider.is_in_fallback_mode().await);

        // One more fallback turn
        let result = provider.complete("system", &[], &[]).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().1.model, "lead");
        assert!(!provider.is_in_fallback_mode().await); // Should exit fallback mode
    }

    #[derive(Clone)]
    struct MockFailureProvider {
        name: String,
        model_config: ModelConfig,
        should_fail: bool,
    }

    #[async_trait]
    impl Provider for MockFailureProvider {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::empty()
        }

        fn get_name(&self) -> &str {
            &self.name
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model_config.clone()
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            if self.should_fail {
                Err(ProviderError::ExecutionError(
                    "Simulated failure".to_string(),
                ))
            } else {
                Ok((
                    Message::new(
                        Role::Assistant,
                        Utc::now().timestamp(),
                        vec![MessageContent::Text(
                            RawTextContent {
                                text: format!("Response from {}", self.name),
                                meta: None,
                            }
                            .no_annotation(),
                        )],
                    ),
                    ProviderUsage::new(self.name.clone(), Usage::default()),
                ))
            }
        }
    }

    /// A provider whose streaming support is whatever the test says it is.
    struct StreamingCapability {
        streams: bool,
        steers: bool,
        restarts: bool,
        model: ModelConfig,
    }

    #[async_trait]
    impl Provider for StreamingCapability {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::empty()
        }
        fn get_name(&self) -> &str {
            "capability"
        }
        fn get_model_config(&self) -> ModelConfig {
            self.model.clone()
        }
        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            Ok((
                Message::assistant().with_text("done"),
                ProviderUsage::new("m".to_string(), Usage::default()),
            ))
        }
        fn supports_streaming(&self) -> bool {
            self.streams
        }
        fn supports_live_steering(&self) -> bool {
            self.steers
        }
        fn supports_restart_steering(&self) -> bool {
            self.restarts
        }
        async fn stream(
            &self,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<MessageStream, ProviderError> {
            Ok(crate::providers::base::stream_from_single_message(
                Message::assistant().with_text("streamed"),
                ProviderUsage::new("m".to_string(), Usage::default()),
            ))
        }
        async fn stream_with_steering(
            &self,
            system: &str,
            messages: &[Message],
            tools: &[Tool],
            mut steering: ProviderSteerReceiver,
        ) -> Result<MessageStream, ProviderError> {
            if !self.steers {
                return Err(ProviderError::NotImplemented(
                    "test provider does not support live steering".to_string(),
                ));
            }
            tokio::spawn(async move {
                if let Some(request) = steering.recv().await {
                    request.acknowledge();
                }
            });
            self.stream(system, messages, tools).await
        }
    }

    fn capability(streams: bool) -> Arc<dyn Provider> {
        Arc::new(StreamingCapability {
            streams,
            steers: streams,
            restarts: false,
            model: ModelConfig::new("m").unwrap(),
        })
    }

    fn streaming_capability(steers: bool) -> Arc<dyn Provider> {
        Arc::new(StreamingCapability {
            streams: true,
            steers,
            restarts: false,
            model: ModelConfig::new("m").unwrap(),
        })
    }

    fn restart_capability(restarts: bool) -> Arc<dyn Provider> {
        Arc::new(StreamingCapability {
            streams: true,
            steers: false,
            restarts,
            model: ModelConfig::new("m").unwrap(),
        })
    }

    /// Pairing two streaming providers must keep streaming.
    ///
    /// Without an override the trait default (`false`) stands and the agent
    /// silently takes the blocking branch — the coding-agent providers would
    /// appear not to stream, with nothing failing to explain why.
    #[tokio::test]
    async fn a_pair_of_streaming_providers_still_streams() {
        let pair = LeadWorkerProvider::new(capability(true), capability(true), Some(1));
        assert!(
            pair.supports_streaming(),
            "a lead/worker pair must forward the capability it wraps"
        );

        let stream = pair.stream("SYS", &[], &[]).await;
        assert!(
            stream.is_ok(),
            "and stream() must reach the active provider"
        );
    }

    #[tokio::test]
    async fn a_pair_forwards_steering_to_the_active_provider() {
        let pair = LeadWorkerProvider::new(capability(true), capability(true), Some(1));
        assert!(pair.supports_live_steering());

        let (sender, receiver) = crate::providers::base::provider_steer_channel();
        let stream = pair
            .stream_with_steering("SYS", &[], &[], receiver)
            .await
            .expect("stream");
        let (request, acknowledged) =
            crate::providers::base::ProviderSteerRequest::new("change course");
        assert!(sender.send(request).is_ok(), "active provider is listening");
        acknowledged
            .await
            .expect("active provider retained the acknowledgement")
            .expect("active provider accepted the steer");

        futures::pin_mut!(stream);
        while futures::StreamExt::next(&mut stream).await.is_some() {}
        assert_eq!(pair.get_turn_count().await, 1);
    }

    #[tokio::test]
    async fn a_mixed_pair_steers_whichever_capable_half_is_active() {
        async fn assert_acknowledged(pair: &LeadWorkerProvider) {
            let (sender, receiver) = crate::providers::base::provider_steer_channel();
            let stream = pair
                .stream_with_steering("SYS", &[], &[], receiver)
                .await
                .expect("stream");
            let (request, acknowledged) =
                crate::providers::base::ProviderSteerRequest::new("change course");
            sender.send(request).expect("active provider is listening");
            acknowledged
                .await
                .expect("active provider retained the acknowledgement")
                .expect("active provider accepted the steer");
            futures::pin_mut!(stream);
            while futures::StreamExt::next(&mut stream).await.is_some() {}
        }

        let capable_lead = LeadWorkerProvider::new(
            streaming_capability(true),
            streaming_capability(false),
            Some(1),
        );
        assert!(capable_lead.supports_live_steering());
        assert_acknowledged(&capable_lead).await;

        let capable_worker = LeadWorkerProvider::new(
            streaming_capability(false),
            streaming_capability(true),
            Some(1),
        );
        let first = capable_worker
            .stream("SYS", &[], &[])
            .await
            .expect("lead stream");
        futures::pin_mut!(first);
        while futures::StreamExt::next(&mut first).await.is_some() {}
        assert_acknowledged(&capable_worker).await;
    }

    #[tokio::test]
    async fn a_mixed_pair_drops_the_live_channel_when_the_active_half_cannot_steer() {
        let pair = LeadWorkerProvider::new(
            streaming_capability(false),
            streaming_capability(true),
            Some(1),
        );
        assert!(pair.supports_live_steering());
        let (sender, receiver) = crate::providers::base::provider_steer_channel();
        let stream = pair
            .stream_with_steering("SYS", &[], &[], receiver)
            .await
            .expect("the non-steering half still streams normally");
        let (request, _acknowledged) =
            crate::providers::base::ProviderSteerRequest::new("defer me");
        assert!(
            sender.send(request).is_err(),
            "a non-steering active half must close the channel so the agent falls back"
        );
        futures::pin_mut!(stream);
        while futures::StreamExt::next(&mut stream).await.is_some() {}
    }

    #[tokio::test]
    async fn restart_steering_tracks_the_active_half_without_changing_rotation() {
        let pair = LeadWorkerProvider::new(
            restart_capability(true),
            streaming_capability(true),
            Some(1),
        );
        assert!(pair.supports_live_steering());
        assert!(
            pair.supports_restart_steering(),
            "the restart-capable lead is active"
        );

        let lead = pair.stream("SYS", &[], &[]).await.expect("lead stream");
        futures::pin_mut!(lead);
        while futures::StreamExt::next(&mut lead).await.is_some() {}

        assert_eq!(pair.get_turn_count().await, 1);
        assert!(pair.supports_live_steering());
        assert!(
            !pair.supports_restart_steering(),
            "the live-only worker must not inherit restart steering"
        );

        let inverse = LeadWorkerProvider::new(
            streaming_capability(true),
            restart_capability(true),
            Some(1),
        );
        assert!(inverse.supports_live_steering());
        assert!(!inverse.supports_restart_steering());
        let lead = inverse.stream("SYS", &[], &[]).await.expect("lead stream");
        futures::pin_mut!(lead);
        while futures::StreamExt::next(&mut lead).await.is_some() {}
        assert!(inverse.supports_live_steering());
        assert!(inverse.supports_restart_steering());
    }

    /// **The turn must still rotate on the streaming path.**
    ///
    /// This is the assertion the first version of `stream()` failed. It
    /// forwarded the active provider's stream and nothing else, so
    /// `handle_completion_result` — the only place `turn_count` is incremented —
    /// was never reached. `turn_count` stayed 0, `count < lead_turns` stayed
    /// true, and the worker model was never used again. Nothing errored; the
    /// pair just quietly stopped being a lead/worker pair.
    ///
    /// `stream.is_ok()` cannot catch that, which is exactly why this test drains
    /// the stream and then asks who would serve the next turn.
    #[tokio::test]
    async fn streaming_a_turn_advances_the_rotation() {
        let pair = LeadWorkerProvider::new(capability(true), capability(true), Some(1));
        assert_eq!(pair.get_turn_count().await, 0);

        // One streamed turn, drained to completion — the settle happens when the
        // stream ends, not when it is created.
        let stream = pair.stream("SYS", &[], &[]).await.expect("stream");
        futures::pin_mut!(stream);
        while futures::StreamExt::next(&mut stream).await.is_some() {}

        assert_eq!(
            pair.get_turn_count().await,
            1,
            "a streamed turn must advance the rotation, or the pair uses the lead \
             model forever and the worker is never reached"
        );
    }

    /// Draining only part of a stream must not settle the turn twice, and
    /// abandoning one must not advance it at all.
    #[tokio::test]
    async fn an_abandoned_stream_does_not_advance_the_rotation() {
        let pair = LeadWorkerProvider::new(capability(true), capability(true), Some(1));

        let stream = pair.stream("SYS", &[], &[]).await.expect("stream");
        drop(stream);

        assert_eq!(
            pair.get_turn_count().await,
            0,
            "a turn the user cancelled before it produced anything is not a turn \
             the model took"
        );
    }

    /// If either half cannot stream, the pair must not claim it can.
    ///
    /// The answer is `&&`, not `||`: this is answered once per turn while the
    /// active provider changes between turns, so claiming support because the
    /// lead has it would send a worker-served turn into the trait's
    /// `NotImplemented`.
    #[tokio::test]
    async fn a_pair_with_one_blocking_half_does_not_claim_streaming() {
        let lead_only = LeadWorkerProvider::new(capability(true), capability(false), Some(1));
        assert!(
            !lead_only.supports_streaming(),
            "a worker that cannot stream would hit NotImplemented on its turn"
        );
        assert!(lead_only.supports_live_steering());

        let worker_only = LeadWorkerProvider::new(capability(false), capability(true), Some(1));
        assert!(
            !worker_only.supports_streaming(),
            "and symmetrically for a non-streaming lead"
        );
        assert!(worker_only.supports_live_steering());
    }
    #[test]
    fn computer_use_composite_discloses_both_destinations_and_binds_both_routes() {
        use crate::providers::api_client::{ApiClient, AuthMethod};
        use crate::providers::openai::OpenAiProvider;
        let endpoint = |url: &str| -> Arc<dyn Provider> {
            Arc::new(OpenAiProvider::new(
                ApiClient::new(url.into(), AuthMethod::BearerToken("test".into())).unwrap(),
                ModelConfig::new_or_fail("model"),
            ))
        };
        let lead = endpoint("http://localhost:11434");
        let first = LeadWorkerProvider::new(
            lead.clone(),
            endpoint("https://remote.example/private-a"),
            None,
        );
        let second = LeadWorkerProvider::new(
            lead.clone(),
            endpoint("https://remote.example/private-b"),
            None,
        );
        assert_eq!(
            first.computer_use_destination().as_deref(),
            Some("lead: http://localhost:11434; worker: https://remote.example")
        );
        assert_eq!(
            first.computer_use_destination(),
            second.computer_use_destination()
        );
        assert_ne!(
            first.computer_use_destination_identity(),
            second.computer_use_destination_identity()
        );
        let unknown = Arc::new(MockProvider {
            name: "unknown".into(),
            model_config: ModelConfig::new_or_fail("model"),
        });
        let partial = LeadWorkerProvider::new(lead, unknown, None);
        assert_eq!(
            partial.computer_use_destination().as_deref(),
            Some("lead: http://localhost:11434; worker: destination not reported")
        );
    }
}
