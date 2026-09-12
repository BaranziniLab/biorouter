use std::sync::{Arc, RwLock};

use super::{
    anthropic::AnthropicProvider,
    azure::AzureProvider,
    base::{Provider, ProviderMetadata},
    claude_code::ClaudeCodeProvider,
    codex::CodexProvider,
    databricks::DatabricksProvider,
    gcpvertexai::GcpVertexAIProvider,
    githubcopilot::GithubCopilotProvider,
    google::GoogleProvider,
    lead_worker::{LeadWorkerProvider, LeadWorkerRoutingState, PersistedProviderConfig},
    litellm::LiteLLMProvider,
    llamacpp::LlamaCppProvider,
    ollama::OllamaProvider,
    openai::OpenAiProvider,
    openrouter::OpenRouterProvider,
    provider_binding::{
        ensure_no_restore_marker, PersistedStandaloneProviderBinding, ProviderRestoreBinding,
    },
    provider_registry::ProviderRegistry,
    snowflake::SnowflakeProvider,
    tetrate::TetrateProvider,
    venice::VeniceProvider,
    versa_azure::VersaAzureProvider,
    xai::XaiProvider,
    xiaomi_mimo::XiaomiMimoProvider,
    zai::ZaiProvider,
};
#[cfg(feature = "aws-providers")]
use super::{
    bedrock::BedrockProvider, sagemaker_tgi::SageMakerTgiProvider,
    versa_bedrock::VersaBedrockProvider,
};
use crate::model::ModelConfig;
use crate::providers::base::ProviderType;
use crate::{
    config::declarative_providers::register_declarative_providers,
    providers::provider_registry::ProviderEntry,
};
use anyhow::Result;
use tokio::sync::OnceCell;

const DEFAULT_LEAD_TURNS: usize = 3;
const DEFAULT_FAILURE_THRESHOLD: usize = 2;
const DEFAULT_FALLBACK_TURNS: usize = 2;

static REGISTRY: OnceCell<RwLock<ProviderRegistry>> = OnceCell::const_new();

async fn init_registry() -> RwLock<ProviderRegistry> {
    let mut registry = ProviderRegistry::new().with_providers(register_builtin_providers);
    if let Err(e) = load_custom_providers_into_registry(&mut registry) {
        tracing::warn!("Failed to load custom providers: {}", e);
    }
    RwLock::new(registry)
}

/// Every provider compiled into this binary, and the single place they are
/// declared.
///
/// ⚠ Extracted from `init_registry` so a test can enumerate exactly this set
/// without also picking up the bundled and user-written declarative providers
/// that `load_custom_providers_into_registry` adds — see
/// `tests::every_registered_provider_is_classified_for_affiliation`, which fails
/// until a provider added here is classified against DR-26's third axis.
fn register_builtin_providers(registry: &mut ProviderRegistry) {
    registry.register::<AnthropicProvider, _>(|m| Box::pin(AnthropicProvider::from_env(m)), true);
    registry.register::<AzureProvider, _>(|m| Box::pin(AzureProvider::from_env(m)), false);
    #[cfg(feature = "aws-providers")]
    registry.register::<BedrockProvider, _>(|m| Box::pin(BedrockProvider::from_env(m)), false);
    registry
        .register::<VersaAzureProvider, _>(|m| Box::pin(VersaAzureProvider::from_env(m)), false);
    #[cfg(feature = "aws-providers")]
    registry.register::<VersaBedrockProvider, _>(
        |m| Box::pin(VersaBedrockProvider::from_env(m)),
        false,
    );
    // `preferred: false`: these drive another vendor's installed CLI on the
    // user's own subscription, which is a different kind of thing from a direct
    // metered endpoint and should not be offered beside one by default.
    registry
        .register::<ClaudeCodeProvider, _>(|m| Box::pin(ClaudeCodeProvider::from_env(m)), false);
    registry.register::<CodexProvider, _>(|m| Box::pin(CodexProvider::from_env(m)), false);
    registry.register::<DatabricksProvider, _>(|m| Box::pin(DatabricksProvider::from_env(m)), true);
    registry
        .register::<GcpVertexAIProvider, _>(|m| Box::pin(GcpVertexAIProvider::from_env(m)), false);
    registry.register::<GithubCopilotProvider, _>(
        |m| Box::pin(GithubCopilotProvider::from_env(m)),
        false,
    );
    registry.register::<GoogleProvider, _>(|m| Box::pin(GoogleProvider::from_env(m)), true);
    registry.register::<LiteLLMProvider, _>(|m| Box::pin(LiteLLMProvider::from_env(m)), false);
    registry.register::<LlamaCppProvider, _>(|m| Box::pin(LlamaCppProvider::from_env(m)), true);
    registry.register::<OllamaProvider, _>(|m| Box::pin(OllamaProvider::from_env(m)), true);
    registry.register::<OpenAiProvider, _>(|m| Box::pin(OpenAiProvider::from_env(m)), true);
    registry.register::<OpenRouterProvider, _>(|m| Box::pin(OpenRouterProvider::from_env(m)), true);
    #[cfg(feature = "aws-providers")]
    registry.register::<SageMakerTgiProvider, _>(
        |m| Box::pin(SageMakerTgiProvider::from_env(m)),
        false,
    );
    registry.register::<SnowflakeProvider, _>(|m| Box::pin(SnowflakeProvider::from_env(m)), false);
    registry.register::<TetrateProvider, _>(|m| Box::pin(TetrateProvider::from_env(m)), true);
    registry.register::<VeniceProvider, _>(|m| Box::pin(VeniceProvider::from_env(m)), false);
    registry.register::<XaiProvider, _>(|m| Box::pin(XaiProvider::from_env(m)), false);
    registry
        .register::<XiaomiMimoProvider, _>(|m| Box::pin(XiaomiMimoProvider::from_env(m)), false);
    registry.register::<ZaiProvider, _>(|m| Box::pin(ZaiProvider::from_env(m)), false);
}

/// Every built-in provider's metadata from a fresh registry — no declarative or
/// user-written providers, so a test that reads it sees the same set on every
/// machine.
#[cfg(test)]
pub(crate) fn builtin_provider_metadata() -> Vec<ProviderMetadata> {
    let mut registry = ProviderRegistry::new();
    register_builtin_providers(&mut registry);
    registry
        .all_metadata_with_types()
        .into_iter()
        .map(|(metadata, _)| metadata)
        .collect()
}

fn load_custom_providers_into_registry(registry: &mut ProviderRegistry) -> Result<()> {
    register_declarative_providers(registry)
}

async fn get_registry() -> &'static RwLock<ProviderRegistry> {
    REGISTRY.get_or_init(init_registry).await
}

pub async fn providers() -> Vec<(ProviderMetadata, ProviderType)> {
    get_registry()
        .await
        .read()
        .unwrap()
        .all_metadata_with_types()
}

pub async fn refresh_custom_providers() -> Result<()> {
    let registry = get_registry().await;
    registry.write().unwrap().remove_custom_providers();

    if let Err(e) = load_custom_providers_into_registry(&mut registry.write().unwrap()) {
        tracing::warn!("Failed to refresh custom providers: {}", e);
        return Err(e);
    }

    tracing::info!("Custom providers refreshed");
    Ok(())
}

async fn get_from_registry(name: &str) -> Result<ProviderEntry> {
    let guard = get_registry().await.read().unwrap();
    guard
        .entries
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("Unknown provider: {}", name))
        .cloned()
}

pub async fn create(name: &str, model: ModelConfig) -> Result<Arc<dyn Provider>> {
    ensure_no_restore_marker(&model)?;
    create_unpersisted(name, model).await
}

pub(crate) async fn create_from_persisted(
    name: &str,
    model: ModelConfig,
) -> Result<Arc<dyn Provider>> {
    if let Some(persisted) = PersistedStandaloneProviderBinding::from_model_config(&model)? {
        return create_provider_from_binding(persisted.into_binding(name)?, get_registry().await)
            .await;
    }
    if let Some(persisted) = PersistedProviderConfig::from_model_config(&model)? {
        return create_lead_worker_from_persisted(persisted, get_registry().await).await;
    }

    create_unpersisted(name, model).await
}

async fn create_unpersisted(name: &str, model: ModelConfig) -> Result<Arc<dyn Provider>> {
    let config = crate::config::Config::global();

    if let Ok(lead_model_name) = config.get_param::<String>("BIOROUTER_LEAD_MODEL") {
        tracing::info!("Creating lead/worker provider from environment variables");
        return create_lead_worker_from_env(name, &model, &lead_model_name).await;
    }

    let constructor = get_from_registry(name).await?.constructor.clone();
    constructor(model).await
}

pub async fn create_with_default_model(name: impl AsRef<str>) -> Result<Arc<dyn Provider>> {
    get_from_registry(name.as_ref())
        .await?
        .create_with_default_model()
        .await
}

pub async fn create_with_named_model(
    provider_name: &str,
    model_name: &str,
) -> Result<Arc<dyn Provider>> {
    let config = ModelConfig::new(model_name)?;
    create(provider_name, config).await
}

async fn create_lead_worker_from_env(
    default_provider_name: &str,
    default_model: &ModelConfig,
    lead_model_name: &str,
) -> Result<Arc<dyn Provider>> {
    let config = crate::config::Config::global();

    let lead_provider_name = config
        .get_param::<String>("BIOROUTER_LEAD_PROVIDER")
        .unwrap_or_else(|_| default_provider_name.to_string());

    let lead_turns = config
        .get_param::<usize>("BIOROUTER_LEAD_TURNS")
        .unwrap_or(DEFAULT_LEAD_TURNS);
    let failure_threshold = config
        .get_param::<usize>("BIOROUTER_LEAD_FAILURE_THRESHOLD")
        .unwrap_or(DEFAULT_FAILURE_THRESHOLD);
    let fallback_turns = config
        .get_param::<usize>("BIOROUTER_LEAD_FALLBACK_TURNS")
        .unwrap_or(DEFAULT_FALLBACK_TURNS);

    let lead_model_config = ModelConfig::new_with_context_env(
        lead_model_name.to_string(),
        Some("BIOROUTER_LEAD_CONTEXT_LIMIT"),
    )?;

    let worker_model_config = create_worker_model_config(default_model)?;

    let registry = get_registry().await;
    let lead_provider = create_provider_from_binding(
        ProviderRestoreBinding::registry(lead_provider_name, lead_model_config),
        registry,
    )
    .await?;
    let worker_provider = create_provider_from_binding(
        ProviderRestoreBinding::registry(default_provider_name.to_string(), worker_model_config),
        registry,
    )
    .await?;

    Ok(Arc::new(LeadWorkerProvider::new_with_settings_and_state(
        lead_provider,
        worker_provider,
        lead_turns,
        failure_threshold,
        fallback_turns,
        uuid::Uuid::new_v4().to_string(),
        LeadWorkerRoutingState::default(),
    )))
}

async fn create_lead_worker_from_persisted(
    persisted: PersistedProviderConfig,
    registry: &RwLock<ProviderRegistry>,
) -> Result<Arc<dyn Provider>> {
    let PersistedProviderConfig::LeadWorkerV2 {
        lead,
        worker,
        lead_turns,
        failure_threshold,
        fallback_turns,
        config_generation,
        routing_state,
    } = persisted;

    anyhow::ensure!(
        !config_generation.trim().is_empty(),
        "persisted lead/worker configuration has no generation"
    );

    anyhow::ensure!(
        routing_state.in_fallback_mode == (routing_state.fallback_remaining > 0),
        "invalid persisted lead/worker fallback state"
    );
    anyhow::ensure!(
        routing_state.fallback_remaining <= fallback_turns,
        "persisted lead/worker fallback exceeds configured fallback turns"
    );

    let lead_provider = create_provider_from_binding(lead, registry).await?;
    let worker_provider = create_provider_from_binding(worker, registry).await?;

    Ok(Arc::new(LeadWorkerProvider::new_with_settings_and_state(
        lead_provider,
        worker_provider,
        lead_turns,
        failure_threshold,
        fallback_turns,
        config_generation,
        routing_state,
    )))
}

async fn create_provider_from_binding(
    binding: ProviderRestoreBinding,
    registry: &RwLock<ProviderRegistry>,
) -> Result<Arc<dyn Provider>> {
    binding.validate()?;
    match binding {
        ProviderRestoreBinding::Registry {
            provider_name,
            model,
        } => {
            let constructor = registry
                .read()
                .unwrap()
                .entries
                .get(&provider_name)
                .ok_or_else(|| anyhow::anyhow!("Unknown provider: {provider_name}"))?
                .constructor
                .clone();
            constructor(model).await
        }
        ProviderRestoreBinding::VersaAzure {
            model,
            endpoint,
            deployment,
            api_version,
            credential_source,
        } => Ok(Arc::new(VersaAzureProvider::from_resolved(
            model,
            endpoint,
            deployment,
            api_version,
            credential_source,
        )?)),
        #[cfg(feature = "aws-providers")]
        ProviderRestoreBinding::VersaBedrock {
            model,
            endpoint,
            region,
            retry,
            operation_timeout_secs,
        } => Ok(Arc::new(
            VersaBedrockProvider::from_resolved(
                model,
                endpoint,
                region,
                retry,
                operation_timeout_secs,
            )
            .await?,
        )),
        #[cfg(not(feature = "aws-providers"))]
        ProviderRestoreBinding::VersaBedrock { .. } => {
            anyhow::bail!("Versa Bedrock support is not available in this build")
        }
        ProviderRestoreBinding::Codex { model, command } => {
            Ok(Arc::new(CodexProvider::from_resolved(model, command)?))
        }
        ProviderRestoreBinding::ClaudeCode { model, command } => {
            Ok(Arc::new(ClaudeCodeProvider::from_resolved(model, command)?))
        }
    }
}

fn create_worker_model_config(default_model: &ModelConfig) -> Result<ModelConfig> {
    let mut worker_config =
        crate::providers::lead_worker::model_config_without_restore_marker(default_model.clone());

    let global_config = crate::config::Config::global();

    if let Ok(limit) = global_config.get_param::<usize>("BIOROUTER_WORKER_CONTEXT_LIMIT") {
        worker_config = worker_config.with_context_limit(Some(limit));
    } else if let Ok(limit) = global_config.get_param::<usize>("BIOROUTER_CONTEXT_LIMIT") {
        worker_config = worker_config.with_context_limit(Some(limit));
    }

    Ok(worker_config)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::conversation::message::Message;
    use crate::providers::base::{ProviderUsage, Usage};
    use crate::providers::errors::ProviderError;
    use rmcp::model::Tool;

    macro_rules! fake_provider {
        ($type_name:ident, $provider_name:literal) => {
            struct $type_name {
                model: ModelConfig,
            }

            #[async_trait::async_trait]
            impl Provider for $type_name {
                fn metadata() -> ProviderMetadata {
                    ProviderMetadata::new(
                        $provider_name,
                        $provider_name,
                        "test-only provider",
                        "test-model",
                        vec!["test-model"],
                        "",
                        vec![],
                    )
                }

                fn get_name(&self) -> &str {
                    $provider_name
                }

                async fn complete_with_model(
                    &self,
                    _model_config: &ModelConfig,
                    _system: &str,
                    _messages: &[Message],
                    _tools: &[Tool],
                ) -> Result<(Message, ProviderUsage), ProviderError> {
                    Ok((
                        Message::assistant().with_text(format!("served by {}", $provider_name)),
                        ProviderUsage::new(self.model.model_name.clone(), Usage::default()),
                    ))
                }

                fn get_model_config(&self) -> ModelConfig {
                    self.model.clone()
                }
            }
        };
    }

    fake_provider!(FakeVersaAzureProvider, "versa_azure");
    fake_provider!(FakeVersaBedrockProvider, "versa_bedrock");
    fake_provider!(FakeCodexProvider, "codex");
    fake_provider!(FakeClaudeCodeProvider, "claude_code");

    fn restore_test_model(model_name: &str) -> ModelConfig {
        ModelConfig {
            model_name: model_name.to_string(),
            context_limit: Some(32_000),
            temperature: None,
            max_tokens: None,
            toolshim: false,
            toolshim_model: None,
            fast_model: None,
            request_params: None,
            reasoning_effort: None,
        }
    }

    fn restore_test_registry() -> RwLock<ProviderRegistry> {
        RwLock::new(ProviderRegistry::new().with_providers(|registry| {
            registry.register::<FakeVersaAzureProvider, _>(
                |model| Box::pin(async move { Ok(FakeVersaAzureProvider { model }) }),
                false,
            );
            registry.register::<FakeVersaBedrockProvider, _>(
                |model| Box::pin(async move { Ok(FakeVersaBedrockProvider { model }) }),
                false,
            );
            registry.register::<FakeCodexProvider, _>(
                |model| Box::pin(async move { Ok(FakeCodexProvider { model }) }),
                false,
            );
            registry.register::<FakeClaudeCodeProvider, _>(
                |model| Box::pin(async move { Ok(FakeClaudeCodeProvider { model }) }),
                false,
            );
        }))
    }

    fn restore_test_binding(provider_name: &str, model_name: &str) -> ProviderRestoreBinding {
        ProviderRestoreBinding::registry(provider_name.to_string(), restore_test_model(model_name))
    }

    async fn cold_restore(
        provider: &Arc<dyn Provider>,
        registry: &RwLock<ProviderRegistry>,
    ) -> Arc<dyn Provider> {
        let stored_json = serde_json::to_string(&provider.get_model_config()).unwrap();
        let cold_model: ModelConfig = serde_json::from_str(&stored_json).unwrap();
        let persisted = PersistedProviderConfig::from_model_config(&cold_model)
            .unwrap()
            .expect("composite marker must survive the session model-config round trip");
        create_lead_worker_from_persisted(persisted, registry)
            .await
            .unwrap()
    }

    fn routing_state(provider: &Arc<dyn Provider>) -> LeadWorkerRoutingState {
        let persisted = PersistedProviderConfig::from_model_config(&provider.get_model_config())
            .unwrap()
            .expect("composite provider must publish its routing snapshot");
        let PersistedProviderConfig::LeadWorkerV2 { routing_state, .. } = persisted;
        routing_state
    }

    fn config_generation(provider: &Arc<dyn Provider>) -> String {
        let persisted = PersistedProviderConfig::from_model_config(&provider.get_model_config())
            .unwrap()
            .expect("composite provider must publish its binding generation");
        let PersistedProviderConfig::LeadWorkerV2 {
            config_generation, ..
        } = persisted;
        config_generation
    }

    /// Every built-in provider whose **instances** can carry an affiliation
    /// (DR-26, Task 46), with the predicate that decides it. Nothing here is a
    /// name-keyed assignment: the name selects which predicate runs, the
    /// predicate reads what the instance actually resolved.
    #[allow(unused_mut)]
    fn affiliated_providers() -> Vec<(&'static str, &'static str)> {
        let mut rows = vec![
            (
                "llamacpp",
                "Local: the managed sidecar, or `self_hosted_affiliation` on LLAMACPP_EXTERNAL_HOST",
            ),
            (
                "ollama",
                "Local: `self_hosted_affiliation` on the resolved OLLAMA_HOST",
            ),
            (
                "versa_azure",
                "Institution(ucsf): `ucsf_gateway_affiliation` on the resolved VERSA_AZURE_ENDPOINT",
            ),
        ];
        #[cfg(feature = "aws-providers")]
        rows.push((
            "versa_bedrock",
            "Institution(ucsf): `ucsf_gateway_affiliation` on the resolved bedrock endpoint",
        ));
        rows
    }

    /// Every other built-in provider, each with the one-line reason affiliation
    /// does not apply to it. These are not `None` by omission — the trait
    /// default *is* `None`, asserted in `affiliation_tests`, so a provider that
    /// says nothing gets less reach rather than more. This table records that the
    /// silence was a decision.
    #[allow(unused_mut)]
    fn unaffiliated_providers() -> Vec<(&'static str, &'static str)> {
        let mut rows = vec![
            ("anthropic", "public: hosted by an AI company"),
            (
                "azure_openai",
                "public: a large cloud, whatever endpoint it is pointed at",
            ),
            (
                "claude_code",
                "public: hosted by an AI company. The `claude` CLI runs locally but the \
                 inference does not, so this is NOT Local — that value is the most permissive \
                 in the model and would hand a public round-trip the private-extension grant",
            ),
            (
                "codex",
                "public: hosted by an AI company. Same reasoning as claude_code — a local \
                 subprocess is not local inference",
            ),
            ("databricks", "public: a large cloud"),
            ("gcp_vertex_ai", "public: a large cloud"),
            ("github_copilot", "public: hosted by a software company"),
            ("google", "public: hosted by an AI company"),
            (
                "litellm",
                "public: an arbitrary proxy, and it makes no loopback claim",
            ),
            ("openai", "public: hosted by an AI company"),
            ("openrouter", "public: a model marketplace"),
            ("snowflake", "public: a large cloud"),
            ("tetrate", "public: a hosted gateway"),
            ("venice", "public: a hosted inference service"),
            ("xai", "public: hosted by an AI company"),
            ("xiaomi_mimo", "public: hosted by an AI company"),
            ("zai", "public: hosted by an AI company"),
        ];
        #[cfg(feature = "aws-providers")]
        {
            rows.push(("aws_bedrock", "public: a large cloud"));
            rows.push((
                "sagemaker_tgi",
                "public: an AWS-hosted endpoint, not this machine",
            ));
        }
        rows
    }

    /// Every built-in provider that ships `ProviderTier::Private` (Task 5), with
    /// the one-line reason. Like the affiliation table above, nothing here is a
    /// name-keyed assignment — the reason names the predicate or the endpoint the
    /// provider's own module states its tier from.
    ///
    /// ⚠ **`pub(crate)` so `tier_tests` reads THIS table rather than keeping a
    /// second one** (Task 56 Step 4). Two lists of which providers are Private is
    /// two things to update and one to forget, and the forgotten one is the guard
    /// — it would go on passing while naming a provider that no longer exists, or
    /// silently stop covering one that does.
    #[allow(unused_mut)]
    pub(crate) fn private_tier_providers() -> Vec<(&'static str, &'static str)> {
        let mut rows = vec![
            (
                "llamacpp",
                "private: the bundled sidecar runs on this machine; a LLAMACPP_EXTERNAL_HOST off it demotes the instance",
            ),
            (
                "ollama",
                "private: a self-hosted server on this machine; an OLLAMA_HOST off it demotes the instance",
            ),
            (
                "versa_azure",
                "private: the UCSF Versa gateway, under a signed agreement; an endpoint off the gateway demotes the instance",
            ),
        ];
        #[cfg(feature = "aws-providers")]
        rows.push((
            "versa_bedrock",
            "private: the UCSF Versa gateway's bedrock route; an endpoint off the gateway demotes the instance",
        ));
        rows
    }

    /// Every other built-in provider, each with the one-line reason it is Public.
    ///
    /// These are not Public by omission — the default *is* Public, pinned at all
    /// four levels by `tier_tests::a_provider_that_declares_no_tier_is_public`,
    /// so a provider that says nothing gets less reach rather than more. ⚠ This
    /// table therefore **records** a decision; it does not make one. A row here
    /// changes nothing about how the provider behaves, which is exactly why it is
    /// worth writing: without it, "nobody ever decided" and "we decided Public"
    /// are indistinguishable.
    #[allow(unused_mut)]
    fn public_tier_providers() -> Vec<(&'static str, &'static str)> {
        let mut rows = vec![
            ("anthropic", "public: general commercial endpoint"),
            (
                "azure_openai",
                "public: a large cloud. ⚠ azure.rs ships the UCSF gateway as \
                 AZURE_OPENAI_ENDPOINT's default, so this one *looks* institutional \
                 and is not; only versa_azure carries the agreement",
            ),
            (
                "claude_code",
                "public: a subprocess wrapper around the user's own `claude` CLI; the \
                 transcript still leaves the machine for Anthropic, and a consumer \
                 subscription carries no BAA to receive clinical data",
            ),
            (
                "codex",
                "public: a subprocess wrapper around the user's own `codex` CLI; the \
                 transcript still leaves the machine for OpenAI",
            ),
            ("databricks", "public: general commercial endpoint"),
            ("gcp_vertex_ai", "public: general commercial endpoint"),
            ("github_copilot", "public: general commercial endpoint"),
            ("google", "public: general commercial endpoint"),
            (
                "litellm",
                "public: an arbitrary proxy, and it makes no loopback claim",
            ),
            ("openai", "public: general commercial endpoint"),
            (
                "openrouter",
                "public: a model marketplace, upstream unknown",
            ),
            ("snowflake", "public: general commercial endpoint"),
            ("tetrate", "public: a hosted gateway"),
            ("venice", "public: a hosted inference service"),
            ("xai", "public: general commercial endpoint"),
            ("xiaomi_mimo", "public: general commercial endpoint"),
            ("zai", "public: general commercial endpoint"),
        ];
        #[cfg(feature = "aws-providers")]
        {
            rows.push(("aws_bedrock", "public: a large cloud"));
            rows.push((
                "sagemaker_tgi",
                "public: an AWS-hosted endpoint, not this machine",
            ));
        }
        rows
    }

    /// Task 53 Step 2: a new provider's tier cannot be left undecided.
    ///
    /// Affiliation has had a completeness census since Task 46; tier had none,
    /// so the fail-safe default silently absorbed every provider nobody thought
    /// about. That is a **bookkeeping** gap rather than a security hole — the
    /// default is Public, which is least-permission, and
    /// `tier_tests::the_private_set_is_a_table_of_reviewed_decisions` is what stops
    /// a new provider from claiming Private. This test closes the other half:
    /// adding a `registry.register::<…>` line above now fails until someone
    /// writes down which tier that provider ships at and why.
    ///
    /// ⚠ Same `aws-providers` caveat as the affiliation census: the feature is
    /// default-on, and `--no-default-features` legitimately registers three
    /// fewer providers, which is why both tables are `cfg`-gated in step with the
    /// registrations.
    #[test]
    fn every_registered_provider_is_classified_for_tier() {
        use crate::privacy::ProviderTier;

        let registered = registered_builtin_tiers();
        let private = private_tier_providers();
        let public = public_tier_providers();

        for (name, shipped) in &registered {
            let in_private = private.iter().any(|(n, _why)| n == name);
            let in_public = public.iter().any(|(n, _why)| n == name);
            assert!(
                in_private ^ in_public,
                "{name} is in neither tier table, or in both; decide it \
                 (may a private session be bound to this provider's models?). \
                 A row reading `public: general commercial endpoint` is a \
                 complete and correct answer."
            );

            // The table records the decision; `P::metadata()` *is* the decision.
            // Without this, a row could claim Public for a provider shipping
            // Private and the census would still be "complete".
            let recorded = if in_private {
                ProviderTier::Private
            } else {
                ProviderTier::Public
            };
            assert_eq!(
                *shipped, recorded,
                "{name} ships {shipped:?} but is filed under {recorded:?}: \
                 fix the table, not the provider (changing a tier is Task 5's \
                 `the_private_set_is_a_table_of_reviewed_decisions`, which needs \
                 an operator ruling)"
            );
        }

        // ...and the tables name nothing that is not registered, so a provider
        // deleted from the factory does not leave a stale classification behind
        // to make the count look right.
        for (name, _why) in private.iter().chain(public.iter()) {
            assert!(
                registered.iter().any(|(r, _)| r == name),
                "{name} is classified but not registered"
            );
        }

        // The count closes the loop: without it, adding a provider to BOTH the
        // registry and one table while deleting another table's row still passes
        // the loops above.
        let names: Vec<&String> = registered.iter().map(|(n, _)| n).collect();
        assert_eq!(
            registered.len(),
            private.len() + public.len(),
            "registered: {names:?}"
        );

        // Every reason is a real reason. An empty string would satisfy the
        // tables while recording nothing.
        for (name, why) in private.iter().chain(public.iter()) {
            assert!(!why.is_empty(), "{name} has no stated reason");
        }
    }

    /// The names `register_builtin_providers` actually registers, in this build's
    /// feature configuration.
    ///
    /// ⚠ Built from a **fresh** registry rather than the process-wide one:
    /// `init_registry` also loads the bundled and user-written declarative
    /// providers, so the live registry contains whatever `custom_providers/`
    /// happens to hold on the machine running the test.
    fn registered_builtin_names() -> Vec<String> {
        let mut registry = ProviderRegistry::new();
        register_builtin_providers(&mut registry);
        let mut names: Vec<String> = registry.entries.keys().cloned().collect();
        names.sort();
        names
    }

    /// The same fresh-registry set, carrying the tier each provider's own
    /// `metadata()` shipped — the value `GET /config/providers` serves and the
    /// tier census checks its table against.
    fn registered_builtin_tiers() -> Vec<(String, crate::privacy::ProviderTier)> {
        let mut registry = ProviderRegistry::new();
        register_builtin_providers(&mut registry);
        let mut rows: Vec<(String, crate::privacy::ProviderTier)> = registry
            .all_metadata_with_types()
            .into_iter()
            .map(|(metadata, _)| (metadata.name, metadata.tier))
            .collect();
        // By name only. `ProviderTier` is deliberately not `Ord` — ordering it
        // would make `max` spellable, and `max` over a capability is always a
        // bug (see `privacy::ProviderTier`) — so the tuple is not `Ord` either.
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }

    /// Task 46 Step 3: a new provider cannot be forgotten.
    ///
    /// Not a rule someone must remember — the same mechanism Task 18A built for
    /// `CAPABILITY_CONFIG_KEYS`. Adding a `registry.register::<…>` line above
    /// fails this test until someone decides whether that provider's instances
    /// carry an affiliation, and writes down why. ⚠ It lives here, directly
    /// beneath the registration list, rather than beside the rest of Task 46's
    /// tests in `affiliation_tests.rs`, because here is where the person adding
    /// a provider is already looking.
    ///
    /// ⚠ The `aws-providers` feature is **default-on**. Running the suite with
    /// `--no-default-features` compiles neither `versa_bedrock` nor the two AWS
    /// public providers, and this test then legitimately sees a shorter list —
    /// which is why both tables are `cfg`-gated in step with the registrations
    /// rather than being flat constants.
    #[test]
    fn every_registered_provider_is_classified_for_affiliation() {
        let registered = registered_builtin_names();
        let affiliated = affiliated_providers();
        let unaffiliated = unaffiliated_providers();

        for name in &registered {
            let is_affiliated = affiliated.iter().any(|(n, _why)| n == name);
            let is_not = unaffiliated.iter().any(|(n, _why)| n == name);
            assert!(
                is_affiliated ^ is_not,
                "{name} is in neither affiliation table, or in both; classify it \
                 (DR-26: does an instance of it carry Local, an Institution, or nothing?)"
            );
        }

        // ...and the tables name nothing that is not registered, so a provider
        // deleted from the factory does not leave a stale classification behind
        // to make the count look right.
        for (name, _why) in affiliated.iter().chain(unaffiliated.iter()) {
            assert!(
                registered.iter().any(|r| r == name),
                "{name} is classified but not registered"
            );
        }

        // The count closes the loop: without it, adding a provider to BOTH the
        // registry and one table while deleting another table's row still
        // passes the two loops above.
        assert_eq!(
            registered.len(),
            affiliated.len() + unaffiliated.len(),
            "registered: {registered:?}"
        );

        // Every reason is a real reason. An empty string would satisfy the
        // tables while recording nothing.
        for (name, why) in affiliated.iter().chain(unaffiliated.iter()) {
            assert!(!why.is_empty(), "{name} has no stated reason");
        }
    }

    /// Task 56 Step 3 — **referential integrity, which is what replaced a
    /// count.**
    ///
    /// This slot used to hold `this_build_knows_exactly_one_institution`: every
    /// affiliated provider had to be `Local` or `Institution(ucsf)`, because
    /// `composite_affiliation` had no representable answer for a lead/worker pair
    /// spanning two institutions and fell back to a sentinel id. The set-valued
    /// model affiliation removed that constraint, so the pin has nothing left to
    /// force.
    ///
    /// ⚠ **A count was the wrong shape of guard anyway, and it is worth saying
    /// why rather than just deleting it.** "There should be one institution" says
    /// nothing about whether the one is the *right* one, and its only possible
    /// repair is deletion — the day a second institution is genuinely added the
    /// assertion is simply wrong, so the person adding it removes the gate and
    /// learns nothing. What replaces it scales to any number of institutions and
    /// catches the error people actually make: a typo'd or invented institution
    /// id, which produces a real, silent constraint — an id in no allowlist
    /// mismatches every connector, and one that collides with a published id
    /// clears flows nobody approved.
    ///
    /// The other half of the integrity — an institution in the map that nothing
    /// references — is checked where "referenced" is defined, in
    /// `landing/scripts/build-registry.mjs`'s
    /// `assertInstitutionsAreNamedByACard`, which can see the catalog's cards.
    /// Here we can see the providers.
    ///
    /// ⚠ **What this does NOT check, said plainly.** The rows are prose written
    /// beside each provider, so this holds the TABLE to the registry, not the
    /// provider's `affiliation()` to its row: a provider whose row says
    /// `Institution(ucsf)` while its implementation returns something else
    /// passes here. That tie is per provider and lives with the provider —
    /// `versa_azure::tests::affiliation_follows_the_endpoint_this_instance_resolved`
    /// and its `versa_bedrock` twin assert the real function against the real
    /// resolved endpoint — while the sibling census above keeps the table and
    /// the built-in provider list covering each other.
    #[test]
    fn every_institution_a_provider_claims_is_published_by_the_registry() {
        // Both floors, because this test's whole shape is "for each row" and a
        // table with no institutional row satisfies it while checking nothing.
        // A cardinality gate is what it replaced; a vacuous one would be worse.
        let rows = affiliated_providers();
        assert!(
            !rows.is_empty(),
            "the affiliation table is empty, so this test resolves no institution at all"
        );
        let mut resolved = 0usize;

        for (name, why) in rows {
            let Some(rest) = why.strip_prefix("Institution(") else {
                assert!(
                    why.starts_with("Local:"),
                    "{name}'s affiliation row must begin `Local:` or `Institution(<id>):` so the \
                     institution it claims can be checked against the registry; got {why:?}"
                );
                continue;
            };
            let (id, _) = rest.split_once("):").unwrap_or_else(|| {
                panic!("{name}'s affiliation row opens `Institution(` and never closes it: {why:?}")
            });
            assert!(
                crate::privacy::affiliation::institution_display_name(
                    crate::privacy::affiliation::InstitutionId::new(id)
                )
                .is_some(),
                "{name} claims institution {id:?}, which the registry snapshot does not publish \
                 (privacy::registry_private::INSTITUTIONS). Either it is a typo, in which case \
                 it silently mismatches every connector, because an unpublished id is in no \
                 allowlist, or the institution is real and belongs in INSTITUTIONS in \
                 landing/scripts/build-registry.mjs, which is the one place an institution is \
                 declared."
            );
            resolved += 1;
        }

        assert!(
            resolved > 0,
            "no row named an institution, so every assertion above was skipped and this test \
             proved nothing about the registry. If the build genuinely ships no institutional \
             provider, that is a change to what DR-26's third axis governs and belongs in the \
             table's own doc, not in a silently green test."
        );
    }

    #[test_case::test_case(None, None, None, DEFAULT_LEAD_TURNS, DEFAULT_FAILURE_THRESHOLD, DEFAULT_FALLBACK_TURNS ; "defaults")]
    #[test_case::test_case(Some("7"), Some("4"), Some("3"), 7, 4, 3 ; "custom")]
    #[tokio::test]
    async fn test_create_lead_worker_provider(
        lead_turns: Option<&str>,
        failure_threshold: Option<&str>,
        fallback_turns: Option<&str>,
        expected_turns: usize,
        expected_failure: usize,
        expected_fallback: usize,
    ) {
        let _guard = env_lock::lock_env([
            ("BIOROUTER_LEAD_MODEL", Some("gpt-4o")),
            ("BIOROUTER_LEAD_PROVIDER", None),
            ("BIOROUTER_LEAD_TURNS", lead_turns),
            ("BIOROUTER_LEAD_FAILURE_THRESHOLD", failure_threshold),
            ("BIOROUTER_LEAD_FALLBACK_TURNS", fallback_turns),
            ("OPENAI_API_KEY", Some("fake-openai-no-keyring")),
        ]);

        let provider = create("openai", ModelConfig::new_or_fail("gpt-4o-mini"))
            .await
            .unwrap();
        let lw = provider.as_lead_worker().unwrap();
        let (lead, worker) = lw.get_model_info();
        assert_eq!(lead, "gpt-4o");
        assert_eq!(worker, "gpt-4o-mini");
        assert_eq!(
            lw.get_settings(),
            (expected_turns, expected_failure, expected_fallback)
        );
    }

    #[tokio::test]
    async fn persisted_lead_worker_restores_codex_and_claude_workers_without_their_clis() {
        let registry = restore_test_registry();

        for (worker_provider, worker_model) in [
            ("codex", "gpt-5.6-codex"),
            ("claude_code", "claude-sonnet-4-6"),
        ] {
            let original = create_lead_worker_from_persisted(
                PersistedProviderConfig::LeadWorkerV2 {
                    lead: restore_test_binding("versa_azure", "gpt-5.2"),
                    worker: restore_test_binding(worker_provider, worker_model),
                    lead_turns: 4,
                    failure_threshold: 3,
                    fallback_turns: 2,
                    config_generation: format!("static-{worker_provider}"),
                    routing_state: LeadWorkerRoutingState::default(),
                },
                &registry,
            )
            .await
            .unwrap();

            let stored_json = serde_json::to_string(&original.get_model_config()).unwrap();
            let stored_value: serde_json::Value = serde_json::from_str(&stored_json).unwrap();
            assert_eq!(
                stored_value["request_params"]["__biorouter_provider_restore"]["type"],
                "lead_worker_v2"
            );
            let cold_model: ModelConfig = serde_json::from_str(&stored_json).unwrap();
            let persisted = PersistedProviderConfig::from_model_config(&cold_model)
                .unwrap()
                .expect("composite marker must survive the session model-config round trip");
            let restored = create_lead_worker_from_persisted(persisted, &registry)
                .await
                .unwrap();
            let restored_config =
                PersistedProviderConfig::from_model_config(&restored.get_model_config())
                    .unwrap()
                    .unwrap();
            let PersistedProviderConfig::LeadWorkerV2 {
                lead,
                worker,
                config_generation,
                ..
            } = restored_config;
            assert_eq!(lead.provider_name(), "versa_azure");
            assert_eq!(worker.provider_name(), worker_provider);
            assert_eq!(config_generation, format!("static-{worker_provider}"));

            let restored = restored.as_lead_worker().unwrap();
            assert_eq!(
                restored.get_model_info(),
                ("gpt-5.2".into(), worker_model.into())
            );
            assert_eq!(restored.get_settings(), (4, 3, 2));
        }
    }

    /// A delegated child starts from the parent's live routing snapshot but is
    /// a distinct binding from then on. Each provider combination is built from
    /// this test's local registry, so neither coding-agent CLI nor either Versa
    /// endpoint is discovered or contacted.
    #[tokio::test]
    async fn delegated_versa_composites_advance_and_cold_restore_per_session() {
        let registry = restore_test_registry();

        for (lead_provider, lead_model) in [
            ("versa_azure", "gpt-5.2"),
            ("versa_bedrock", "anthropic.claude-sonnet-4-6"),
        ] {
            for (worker_provider, worker_model) in [
                ("codex", "gpt-5.6-codex"),
                ("claude_code", "claude-sonnet-4-6"),
            ] {
                let parent = create_lead_worker_from_persisted(
                    PersistedProviderConfig::LeadWorkerV2 {
                        lead: restore_test_binding(lead_provider, lead_model),
                        worker: restore_test_binding(worker_provider, worker_model),
                        lead_turns: 2,
                        failure_threshold: 2,
                        fallback_turns: 2,
                        config_generation: format!("parent-{lead_provider}-{worker_provider}"),
                        routing_state: LeadWorkerRoutingState {
                            turn_count: 3,
                            failure_count: 1,
                            in_fallback_mode: false,
                            fallback_remaining: 0,
                        },
                    },
                    &registry,
                )
                .await
                .unwrap();
                assert_eq!(parent.get_name(), lead_provider);
                let parent_generation = config_generation(&parent);
                let child_model = crate::providers::lead_worker::model_config_for_session_fork(
                    &parent.get_model_config(),
                )
                .unwrap()
                .expect("a composite snapshot is forkable");
                let child_persisted = PersistedProviderConfig::from_model_config(&child_model)
                    .unwrap()
                    .unwrap();
                let child = create_lead_worker_from_persisted(child_persisted, &registry)
                    .await
                    .unwrap();
                let child_generation = config_generation(&child);

                assert!(!Arc::ptr_eq(&parent, &child));
                assert_eq!(child.get_name(), lead_provider);
                assert_ne!(parent_generation, child_generation);
                assert_eq!(routing_state(&parent), routing_state(&child));

                for expected_turn in [4, 5] {
                    let (_, usage) = child.complete("system", &[], &[]).await.unwrap();
                    assert_eq!(usage.provider.as_deref(), Some(worker_provider));
                    assert_eq!(usage.model, worker_model);
                    assert_eq!(routing_state(&child).turn_count, expected_turn);
                    assert_eq!(routing_state(&parent).turn_count, 3);
                }

                parent.complete("system", &[], &[]).await.unwrap();
                assert_eq!(routing_state(&parent).turn_count, 4);
                assert_eq!(routing_state(&child).turn_count, 5);

                let restored_parent = cold_restore(&parent, &registry).await;
                let restored_child = cold_restore(&child, &registry).await;
                assert_eq!(restored_parent.get_name(), lead_provider);
                assert_eq!(restored_child.get_name(), lead_provider);
                assert_eq!(config_generation(&restored_parent), parent_generation);
                assert_eq!(config_generation(&restored_child), child_generation);
                assert_eq!(routing_state(&restored_parent).turn_count, 4);
                assert_eq!(routing_state(&restored_child).turn_count, 5);

                restored_child.complete("system", &[], &[]).await.unwrap();
                assert_eq!(routing_state(&restored_child).turn_count, 6);
                assert_eq!(routing_state(&restored_parent).turn_count, 4);
            }
        }
    }

    #[tokio::test]
    async fn a_mid_worker_cold_restore_stays_on_codex_and_claude_workers_without_their_clis() {
        let registry = restore_test_registry();

        for (worker_provider, worker_model) in [
            ("codex", "gpt-5.6-codex"),
            ("claude_code", "claude-sonnet-4-6"),
        ] {
            let original = create_lead_worker_from_persisted(
                PersistedProviderConfig::LeadWorkerV2 {
                    lead: restore_test_binding("versa_azure", "gpt-5.2"),
                    worker: restore_test_binding(worker_provider, worker_model),
                    lead_turns: 2,
                    failure_threshold: 2,
                    fallback_turns: 2,
                    config_generation: format!("worker-{worker_provider}"),
                    routing_state: LeadWorkerRoutingState {
                        turn_count: 5,
                        failure_count: 1,
                        in_fallback_mode: false,
                        fallback_remaining: 0,
                    },
                },
                &registry,
            )
            .await
            .unwrap();

            let restored = cold_restore(&original, &registry).await;
            assert_eq!(
                routing_state(&restored),
                LeadWorkerRoutingState {
                    turn_count: 5,
                    failure_count: 1,
                    in_fallback_mode: false,
                    fallback_remaining: 0,
                }
            );
            let (_, usage) = restored.complete("system", &[], &[]).await.unwrap();
            assert_eq!(usage.model, worker_model);
            assert_eq!(usage.provider.as_deref(), Some(worker_provider));
            assert_eq!(routing_state(&restored).turn_count, 6);
        }
    }

    #[tokio::test]
    async fn a_mid_fallback_cold_restore_finishes_lead_turns_then_returns_to_each_worker() {
        let registry = restore_test_registry();

        for (worker_provider, worker_model) in [
            ("codex", "gpt-5.6-codex"),
            ("claude_code", "claude-sonnet-4-6"),
        ] {
            let original = create_lead_worker_from_persisted(
                PersistedProviderConfig::LeadWorkerV2 {
                    lead: restore_test_binding("versa_azure", "gpt-5.2"),
                    worker: restore_test_binding(worker_provider, worker_model),
                    lead_turns: 2,
                    failure_threshold: 2,
                    fallback_turns: 3,
                    config_generation: format!("fallback-{worker_provider}"),
                    routing_state: LeadWorkerRoutingState {
                        turn_count: 7,
                        failure_count: 0,
                        in_fallback_mode: true,
                        fallback_remaining: 2,
                    },
                },
                &registry,
            )
            .await
            .unwrap();

            let restored = cold_restore(&original, &registry).await;
            let (_, first_usage) = restored.complete("system", &[], &[]).await.unwrap();
            assert_eq!(first_usage.model, "gpt-5.2");
            assert_eq!(first_usage.provider.as_deref(), Some("versa_azure"));
            assert_eq!(
                routing_state(&restored),
                LeadWorkerRoutingState {
                    turn_count: 8,
                    failure_count: 0,
                    in_fallback_mode: true,
                    fallback_remaining: 1,
                }
            );

            let restored = cold_restore(&restored, &registry).await;
            let (_, second_usage) = restored.complete("system", &[], &[]).await.unwrap();
            assert_eq!(second_usage.model, "gpt-5.2");
            assert_eq!(second_usage.provider.as_deref(), Some("versa_azure"));
            assert!(!routing_state(&restored).in_fallback_mode);

            let (_, worker_usage) = restored.complete("system", &[], &[]).await.unwrap();
            assert_eq!(worker_usage.model, worker_model);
            assert_eq!(worker_usage.provider.as_deref(), Some(worker_provider));
        }
    }

    #[test]
    fn unknown_persisted_provider_version_is_not_silently_treated_as_the_lead() {
        let mut model = restore_test_model("gpt-5.2");
        model.request_params = Some(std::collections::HashMap::from([(
            "__biorouter_provider_restore".to_string(),
            serde_json::json!({ "type": "lead_worker_v3" }),
        )]));

        let error = PersistedProviderConfig::from_model_config(&model).unwrap_err();
        assert!(error
            .to_string()
            .contains("invalid persisted provider configuration"));
    }

    #[tokio::test]
    async fn untrusted_azure_restore_markers_cannot_rebind_credentials_to_an_endpoint() {
        let binding = ProviderRestoreBinding::VersaAzure {
            model: restore_test_model("credential-capture"),
            endpoint: crate::providers::provider_binding::SecretFreeEndpoint::new(
                "https://credential-capture.invalid/exfiltrate".into(),
            )
            .unwrap(),
            deployment: "credential-capture".into(),
            api_version: "2025-04-01-preview".into(),
            credential_source:
                crate::providers::provider_binding::VersaAzureCredentialSource::ApiKey,
        };
        let standalone = PersistedStandaloneProviderBinding::new(binding)
            .unwrap()
            .to_model_config()
            .unwrap();
        let mut composite = restore_test_model("credential-capture");
        composite.request_params = Some(std::collections::HashMap::from([(
            crate::providers::provider_binding::RESTORE_CONFIG_KEY.into(),
            serde_json::json!({"type": "lead_worker_v2"}),
        )]));

        for model in [standalone, composite] {
            let error = match create("versa_azure", model).await {
                Ok(_) => panic!("external request parameters entered the restore path"),
                Err(error) => error,
            };
            assert!(
                error
                    .to_string()
                    .contains("reserved for trusted session state"),
                "unexpected error: {error}"
            );
            assert!(
                !error.to_string().contains("credential-capture.invalid"),
                "the rejected endpoint leaked into the public error: {error}"
            );
        }
    }

    #[tokio::test]
    async fn test_create_regular_provider_without_lead_config() {
        let _guard = env_lock::lock_env([
            ("BIOROUTER_LEAD_MODEL", None),
            ("BIOROUTER_LEAD_PROVIDER", None),
            ("BIOROUTER_LEAD_TURNS", None),
            ("BIOROUTER_LEAD_FAILURE_THRESHOLD", None),
            ("BIOROUTER_LEAD_FALLBACK_TURNS", None),
            ("OPENAI_API_KEY", Some("fake-openai-no-keyring")),
        ]);

        let provider = create("openai", ModelConfig::new_or_fail("gpt-4o-mini"))
            .await
            .unwrap();
        assert!(provider.as_lead_worker().is_none());
        assert_eq!(provider.get_model_config().model_name, "gpt-4o-mini");
    }

    #[test_case::test_case(None, None, 16_000 ; "no overrides uses default")]
    #[test_case::test_case(Some("32000"), None, 32_000 ; "worker limit overrides default")]
    #[test_case::test_case(Some("32000"), Some("64000"), 32_000 ; "worker limit takes priority over global")]
    fn test_worker_model_context_limit(
        worker_limit: Option<&str>,
        global_limit: Option<&str>,
        expected_limit: usize,
    ) {
        let _guard = env_lock::lock_env([
            ("BIOROUTER_WORKER_CONTEXT_LIMIT", worker_limit),
            ("BIOROUTER_CONTEXT_LIMIT", global_limit),
        ]);

        let default_model =
            ModelConfig::new_or_fail("gpt-3.5-turbo").with_context_limit(Some(16_000));

        let result = create_worker_model_config(&default_model).unwrap();
        assert_eq!(result.context_limit, Some(expected_limit));
    }

    #[test]
    fn worker_model_preserves_every_field_except_the_restore_marker_and_context_override() {
        let _guard = env_lock::lock_env([
            ("BIOROUTER_WORKER_CONTEXT_LIMIT", Some("64000")),
            ("BIOROUTER_CONTEXT_LIMIT", None),
        ]);
        let mut default_model = restore_test_model("worker-model");
        default_model.temperature = Some(0.4);
        default_model.max_tokens = Some(4_096);
        default_model.toolshim = true;
        default_model.toolshim_model = Some("tool-model".into());
        default_model.fast_model = Some("fast-model".into());
        default_model.reasoning_effort = Some(crate::agents::effort::ReasoningEffort::Deep);
        default_model.request_params = Some(std::collections::HashMap::from([
            ("keep".into(), serde_json::json!({"nested": true})),
            (
                crate::providers::provider_binding::RESTORE_CONFIG_KEY.into(),
                serde_json::json!({"type": "old"}),
            ),
        ]));

        let worker = create_worker_model_config(&default_model).unwrap();
        assert_eq!(worker.context_limit, Some(64_000));
        assert_eq!(worker.temperature, default_model.temperature);
        assert_eq!(worker.max_tokens, default_model.max_tokens);
        assert_eq!(worker.toolshim, default_model.toolshim);
        assert_eq!(worker.toolshim_model, default_model.toolshim_model);
        assert_eq!(worker.fast_model, default_model.fast_model);
        assert_eq!(worker.reasoning_effort, default_model.reasoning_effort);
        assert_eq!(
            worker.request_params,
            Some(std::collections::HashMap::from([(
                "keep".into(),
                serde_json::json!({"nested": true}),
            )]))
        );
    }

    #[tokio::test]
    async fn test_openai_compatible_providers_config_keys() {
        let providers_list = providers().await;
        let cases = vec![
            ("openai", "OPENAI_API_KEY"),
            ("groq", "GROQ_API_KEY"),
            ("mistral", "MISTRAL_API_KEY"),
            ("custom_deepseek", "DEEPSEEK_API_KEY"),
            ("xiaomi_mimo", "XIAOMI_MIMO_API_KEY"),
            ("zai", "ZAI_API_KEY"),
        ];
        for (name, expected_key) in cases {
            if let Some((meta, _)) = providers_list.iter().find(|(m, _)| m.name == name) {
                assert!(
                    !meta.config_keys.is_empty(),
                    "{name} provider should have config keys"
                );
                assert_eq!(
                    meta.config_keys[0].name, expected_key,
                    "First config key for {name} should be {expected_key}, got {}",
                    meta.config_keys[0].name
                );
                assert!(
                    meta.config_keys[0].required,
                    "{expected_key} should be required"
                );
                assert!(
                    meta.config_keys[0].secret,
                    "{expected_key} should be secret"
                );
            } else {
                // Provider not registered; skip test for this provider
                continue;
            }
        }
    }
}
