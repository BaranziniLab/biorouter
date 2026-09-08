use super::base::{ConfigKey, ModelInfo, Provider, ProviderMetadata, ProviderUsage};
use super::errors::ProviderError;
use super::retry::{ProviderRetry, RetryConfig};
use crate::conversation::message::Message;
use crate::model::ModelConfig;
use crate::privacy::ProviderTier;
use crate::providers::utils::RequestLog;
use anyhow::Result;
use async_trait::async_trait;
use aws_sdk_bedrockruntime::config::{Credentials, ProvideCredentials};
use aws_sdk_bedrockruntime::{types as bedrock, Client};
use rmcp::model::Tool;

use aws_sdk_bedrockruntime::operation::converse_stream::ConverseStreamOutput as ConverseStreamResponse;

use super::base::MessageStream;
use super::formats::bedrock::{
    bedrock_blocking_inference_config, bedrock_inference_config, bedrock_message_stream,
    classify_bedrock_converse_error, classify_bedrock_converse_stream_error, from_bedrock_message,
    from_bedrock_usage, map_bedrock_stop_reason, to_bedrock_message, to_bedrock_tool_config,
};
use super::provider_binding::{
    model_without_restore_marker, PersistedRetryConfig, ProviderRestoreBinding, SecretFreeEndpoint,
};

pub const VERSA_BEDROCK_DOC_LINK: &str = "http://biorouter.ucsf.edu/docs";
pub const VERSA_BEDROCK_DEFAULT_MODEL: &str = "us.anthropic.claude-opus-4-6-v1";
// Ordered newest → oldest. The UI auto-selects the first entry as the default
// model when switching providers. Model IDs follow the AWS Bedrock format
// documented at
// https://platform.claude.com/docs/en/about-claude/models, prefixed with the
// `us.` cross-region inference profile required by the UCSF MuleSoft proxy.
//
// Only models that UCSF's Bedrock account is entitled to invoke are listed —
// every entry below has been verified end-to-end via the Versa proxy, except
// where noted. Users can type a newer ID via the "Enter a model not listed..."
// option once UCSF enables it.
pub const VERSA_BEDROCK_KNOWN_MODELS: &[&str] = &[
    // Claude 4.8 (1M context). Added 2026-07 (issue #29). Verified live
    // through the MuleSoft proxy on 2026-07-26: the short un-suffixed form
    // below answered a real converse round-trip, while the `-v1` spelling
    // (which opus-4-6 uses) was rejected with "The provided model
    // identifier is invalid" — 4.8 and 4.6 genuinely differ in id shape on
    // this account. The default model stays on 4.6 for now.
    "us.anthropic.claude-opus-4-8",
    // Claude 4.6 (1M context)
    "us.anthropic.claude-opus-4-6-v1",
    "us.anthropic.claude-sonnet-4-6",
    // Claude 4.5 (200K context)
    "us.anthropic.claude-opus-4-5-20251101-v1:0",
    // Haiku 4.5 (200K context)
    "us.anthropic.claude-haiku-4-5-20251001-v1:0",
    // Sonnet 4 removed: Anthropic retires claude-sonnet-4-20250514 on
    // June 15, 2026 (Bedrock marked it Legacy in April 2026).
];

// UCSF MuleSoft Bedrock proxy. UCSF-issued access keys are signed against this
// endpoint instead of public AWS, so this must be set for Versa Bedrock to work.
pub const VERSA_BEDROCK_DEFAULT_ENDPOINT: &str = "https://unified-api.ucsf.edu/general/awsai";
pub const VERSA_BEDROCK_DEFAULT_REGION: &str = "us-west-2";

pub const VERSA_BEDROCK_DEFAULT_MAX_RETRIES: usize = 6;
pub const VERSA_BEDROCK_DEFAULT_INITIAL_RETRY_INTERVAL_MS: u64 = 2000;
pub const VERSA_BEDROCK_DEFAULT_BACKOFF_MULTIPLIER: f64 = 2.0;
pub const VERSA_BEDROCK_DEFAULT_MAX_RETRY_INTERVAL_MS: u64 = 120_000;

#[cfg(test)]
fn with_test_http_client(loader: aws_config::ConfigLoader) -> aws_config::ConfigLoader {
    let (http_client, _captured) = aws_smithy_http_client::test_util::capture_request(None);
    loader.http_client(http_client)
}

#[derive(Debug, serde::Serialize)]
pub struct VersaBedrockProvider {
    #[serde(skip)]
    client: Client,
    model: ModelConfig,
    #[serde(skip)]
    retry_config: RetryConfig,
    #[serde(skip)]
    name: String,
    /// The endpoint this instance resolved at construction. `tier()` reads it,
    /// never the provider's name — the last fallback in the chain below is
    /// `AWS_ENDPOINT_URL_BEDROCK_RUNTIME`, which `bedrock.rs` sets
    /// process-globally with `std::env::set_var`.
    #[serde(skip)]
    resolved_endpoint: String,
    #[serde(skip)]
    region: String,
    #[serde(skip)]
    operation_timeout_secs: Option<u64>,
}

impl VersaBedrockProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let config = crate::config::Config::global();

        // Endpoint: configurable, but always falls back to the UCSF MuleSoft proxy
        // so a fresh install with just the key + secret works out of the box.
        let endpoint_url: String = config
            .get_param::<String>("AWS_ENDPOINT_URL_BEDROCK")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                std::env::var("AWS_ENDPOINT_URL_BEDROCK")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
            })
            .or_else(|| {
                std::env::var("AWS_ENDPOINT_URL_BEDROCK_RUNTIME")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
            })
            .unwrap_or_else(|| VERSA_BEDROCK_DEFAULT_ENDPOINT.to_string());

        let region: String = config
            .get_param::<String>("AWS_REGION")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                std::env::var("AWS_REGION")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
            })
            .unwrap_or_else(|| VERSA_BEDROCK_DEFAULT_REGION.to_string());

        let retry_config = Self::load_retry_config(config);
        let operation_timeout_secs = Self::load_operation_timeout_secs(config);

        Self::from_resolved(
            model,
            SecretFreeEndpoint::new(endpoint_url)?,
            region,
            PersistedRetryConfig {
                max_retries: retry_config.max_retries,
                initial_interval_ms: retry_config.initial_interval_ms,
                backoff_multiplier: retry_config.backoff_multiplier,
                max_interval_ms: retry_config.max_interval_ms,
            },
            operation_timeout_secs,
        )
        .await
    }

    pub(crate) async fn from_resolved(
        model: ModelConfig,
        endpoint: SecretFreeEndpoint,
        region: String,
        retry: PersistedRetryConfig,
        operation_timeout_secs: Option<u64>,
    ) -> Result<Self> {
        let binding = ProviderRestoreBinding::VersaBedrock {
            model: model.clone(),
            endpoint: endpoint.clone(),
            region: region.clone(),
            retry: retry.clone(),
            operation_timeout_secs,
        };
        binding.validate()?;

        let config = crate::config::Config::global();
        // ⚠ `map_err(|_| "... is not configured")` DISCARDS the reason, and the
        // two reasons need opposite responses from the user:
        //
        //   NotFound        -> "add it in Settings" is right
        //   anything else   -> the key IS there and the store refused the read,
        //                      so "add it in Settings" sends the user to retype
        //                      a credential that is already correct
        //
        // On macOS the second happens whenever the Keychain grant no longer
        // covers the running binary -- the grant is bound to the signature, so a
        // newly installed or re-signed build asks again, and an unanswered
        // prompt fails the read.
        let read_key = |name: &str| -> anyhow::Result<String> {
            match config.get_secret::<String>(name) {
                Ok(value) => Ok(value),
                Err(crate::config::ConfigError::NotFound(_)) => Err(anyhow::anyhow!(
                    "{name} is not configured. Add it under Versa API Bedrock in Settings."
                )),
                Err(error) => Err(anyhow::anyhow!(
                    "Could not read {name} from the credential store: {error}\n\n\
                     The credential appears to be configured, so this is the store \
                     refusing the read rather than a missing key -- do NOT re-enter it. \
                     On macOS a Keychain grant is tied to the application's signature, \
                     so a newly installed or re-signed build asks for permission again; \
                     answer that prompt with \u{201c}Always Allow\u{201d}."
                )),
            }
        };
        let access_key_id: String = read_key("VERSA_BEDROCK_ACCESS_KEY_ID")?;
        let secret_access_key: String = read_key("VERSA_BEDROCK_SECRET_ACCESS_KEY")?;
        anyhow::ensure!(
            !access_key_id.trim().is_empty() && !secret_access_key.trim().is_empty(),
            "Versa Bedrock access key id / secret access key is empty"
        );

        let credentials =
            Credentials::new(access_key_id, secret_access_key, None, None, "VersaBedrock");
        let loader = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .credentials_provider(credentials)
            .region(aws_config::Region::new(region.clone()))
            .endpoint_url(endpoint.as_str());
        #[cfg(test)]
        let loader = with_test_http_client(loader);
        let mut loader = loader;
        if let Some(secs) = operation_timeout_secs {
            loader = loader.timeout_config(
                aws_smithy_types::timeout::TimeoutConfig::builder()
                    .operation_timeout(std::time::Duration::from_secs(secs))
                    .build(),
            );
        }
        let sdk_config = loader.load().await;

        sdk_config
            .credentials_provider()
            .ok_or_else(|| anyhow::anyhow!("No AWS credentials provider configured"))?
            .provide_credentials()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to load Versa Bedrock credentials: {}", e))?;

        let client = Client::new(&sdk_config);
        let retry_config = RetryConfig {
            max_retries: retry.max_retries,
            initial_interval_ms: retry.initial_interval_ms,
            backoff_multiplier: retry.backoff_multiplier,
            max_interval_ms: retry.max_interval_ms,
        };

        Ok(Self {
            client,
            model,
            retry_config,
            name: Self::metadata().name,
            resolved_endpoint: endpoint.into_string(),
            region,
            operation_timeout_secs,
        })
    }

    fn load_retry_config(config: &crate::config::Config) -> RetryConfig {
        let max_retries = config
            .get_param::<usize>("BEDROCK_MAX_RETRIES")
            .unwrap_or(VERSA_BEDROCK_DEFAULT_MAX_RETRIES);
        let initial_interval_ms = config
            .get_param::<u64>("BEDROCK_INITIAL_RETRY_INTERVAL_MS")
            .unwrap_or(VERSA_BEDROCK_DEFAULT_INITIAL_RETRY_INTERVAL_MS);
        let backoff_multiplier = config
            .get_param::<f64>("BEDROCK_BACKOFF_MULTIPLIER")
            .unwrap_or(VERSA_BEDROCK_DEFAULT_BACKOFF_MULTIPLIER);
        let max_interval_ms = config
            .get_param::<u64>("BEDROCK_MAX_RETRY_INTERVAL_MS")
            .unwrap_or(VERSA_BEDROCK_DEFAULT_MAX_RETRY_INTERVAL_MS);
        RetryConfig {
            max_retries,
            initial_interval_ms,
            backoff_multiplier,
            max_interval_ms,
        }
    }

    fn load_operation_timeout_secs(config: &crate::config::Config) -> Option<u64> {
        let secs = config
            .get_param::<u64>("BEDROCK_OPERATION_TIMEOUT_SECS")
            .ok()
            .or_else(|| {
                std::env::var("BEDROCK_OPERATION_TIMEOUT_SECS")
                    .ok()
                    .and_then(|value| value.trim().parse::<u64>().ok())
            })
            .unwrap_or(super::formats::bedrock::BEDROCK_DEFAULT_OPERATION_TIMEOUT_SECS);
        (secs != 0).then_some(secs)
    }

    async fn converse(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(bedrock::Message, Option<bedrock::TokenUsage>, String), ProviderError> {
        let model_name = &model_config.model_name;

        let mut request = self
            .client
            .converse()
            .system(bedrock::SystemContentBlock::Text(system.to_string()))
            .model_id(model_name.to_string())
            .inference_config(bedrock_blocking_inference_config(model_config))
            .set_messages(Some(
                messages
                    .iter()
                    .filter(|m| m.is_agent_visible())
                    .map(to_bedrock_message)
                    .collect::<Result<_>>()?,
            ));

        if !tools.is_empty() {
            request = request.tool_config(to_bedrock_tool_config(tools)?);
        }

        let response = request
            .send()
            .await
            .map_err(classify_bedrock_converse_error)?;

        let finish_reason = map_bedrock_stop_reason(&response.stop_reason);
        match response.output {
            Some(bedrock::ConverseOutput::Message(message)) => {
                Ok((message, response.usage, finish_reason))
            }
            _ => Err(ProviderError::RequestFailed(
                "No output from Bedrock".to_string(),
            )),
        }
    }

    /// Open a `ConverseStream` response. Mirrors [`Self::converse`] exactly —
    /// same system prompt, messages and tool config — so the streaming and
    /// blocking paths cannot drift in what they send.
    async fn converse_stream(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<ConverseStreamResponse, ProviderError> {
        let mut request = self
            .client
            .converse_stream()
            .system(bedrock::SystemContentBlock::Text(system.to_string()))
            .model_id(model_config.model_name.clone())
            .inference_config(bedrock_inference_config(model_config))
            .set_messages(Some(
                messages
                    .iter()
                    .filter(|m| m.is_agent_visible())
                    .map(to_bedrock_message)
                    .collect::<Result<_>>()?,
            ));

        if !tools.is_empty() {
            request = request.tool_config(to_bedrock_tool_config(tools)?);
        }

        request
            .send()
            .await
            .map_err(classify_bedrock_converse_stream_error)
    }
}

#[async_trait]
impl Provider for VersaBedrockProvider {
    fn metadata() -> ProviderMetadata {
        let models: Vec<ModelInfo> = VERSA_BEDROCK_KNOWN_MODELS
            .iter()
            .map(|&name| {
                ModelInfo::new(name, ModelConfig::new_or_fail(name).context_limit()).with_vision()
            })
            .collect();

        ProviderMetadata::with_models(
            "versa_bedrock",
            "Versa API Bedrock",
            "UCSF Anthropic models via Amazon Bedrock. Access key + secret only; endpoint and region are pre-configured.",
            VERSA_BEDROCK_DEFAULT_MODEL,
            models,
            VERSA_BEDROCK_DOC_LINK,
            vec![
                ConfigKey::new("VERSA_BEDROCK_ACCESS_KEY_ID", true, true, None),
                ConfigKey::new("VERSA_BEDROCK_SECRET_ACCESS_KEY", true, true, None),
                ConfigKey::new(
                    "AWS_ENDPOINT_URL_BEDROCK",
                    false,
                    false,
                    Some(VERSA_BEDROCK_DEFAULT_ENDPOINT),
                ),
                ConfigKey::new(
                    "AWS_REGION",
                    false,
                    false,
                    Some(VERSA_BEDROCK_DEFAULT_REGION),
                ),
            ],
        )
        .with_unlisted_models()
        // The shipped endpoint is the UCSF gateway, so a default install is
        // Private. An instance that resolved elsewhere says so itself, below.
        .with_tier(ProviderTier::Private)
        // The type-level half of the same claim, for the surfaces that must
        // name the institution BEFORE anything is configured — see
        // `ProviderMetadata::institutions`. `affiliation()` below is the
        // instance answer and outranks it wherever the daemon resolved one.
        .with_institution(super::UCSF_INSTITUTION)
    }

    fn get_name(&self) -> &str {
        &self.name
    }

    fn restore_binding(&self) -> ProviderRestoreBinding {
        ProviderRestoreBinding::VersaBedrock {
            model: model_without_restore_marker(self.model.clone()),
            endpoint: SecretFreeEndpoint::new(self.resolved_endpoint.clone())
                .expect("resolved Versa Bedrock endpoint must remain valid"),
            region: self.region.clone(),
            retry: PersistedRetryConfig {
                max_retries: self.retry_config.max_retries,
                initial_interval_ms: self.retry_config.initial_interval_ms,
                backoff_multiplier: self.retry_config.backoff_multiplier,
                max_interval_ms: self.retry_config.max_interval_ms,
            },
            operation_timeout_secs: self.operation_timeout_secs,
        }
    }

    fn tier(&self) -> ProviderTier {
        crate::providers::ucsf_gateway_tier(&self.resolved_endpoint)
    }

    /// DR-26: `Institution("ucsf")` — decided by the **same** resolved endpoint
    /// as the tier above, through the same host check, so the two can never
    /// disagree about a repointed instance.
    fn affiliation(&self) -> Option<crate::privacy::affiliation::ModelAffiliation> {
        crate::providers::ucsf_gateway_affiliation(&self.resolved_endpoint)
    }

    fn retry_config(&self) -> RetryConfig {
        self.retry_config.clone()
    }

    fn get_model_config(&self) -> ModelConfig {
        self.model.clone()
    }

    #[tracing::instrument(
        skip(self, model_config, system, messages, tools),
        fields(model_config, input, output, input_tokens, output_tokens, total_tokens)
    )]
    async fn complete_with_model(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let model_name = model_config.model_name.clone();

        let debug_payload = serde_json::json!({
            "system": system,
            "messages": messages,
            "tools": tools
        });
        let mut log = RequestLog::start(&self.model, &debug_payload)?;

        let (bedrock_message, bedrock_usage, finish_reason) = self
            .with_retry(|| self.converse(model_config, system, messages, tools))
            .await
            .inspect_err(|e| {
                let _ = log.error(e);
            })?;

        let usage = bedrock_usage
            .as_ref()
            .map(from_bedrock_usage)
            .unwrap_or_default();

        let message = from_bedrock_message(&bedrock_message)?;

        log.write(
            &serde_json::to_value(&message).unwrap_or_default(),
            Some(&usage),
        )?;

        let mut provider_usage = ProviderUsage::new(model_name.to_string(), usage);
        provider_usage.finish_reason = Some(finish_reason);
        Ok((message, provider_usage))
    }

    /// Stream a turn via Bedrock `ConverseStream`.
    ///
    /// Only opening the stream is retried (via `with_retry`, so the existing
    /// Versa retry budget and error classification are preserved). Once events
    /// start arriving, a failure is terminal: partial output has already reached
    /// the agent and replaying the request would duplicate it.
    async fn stream(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let model_name = self.model.model_name.clone();

        let debug_payload = serde_json::json!({
            "system": system,
            "messages": messages,
            "tools": tools,
            "stream": true
        });
        let mut log = RequestLog::start(&self.model, &debug_payload)?;

        let response = self
            .with_retry(|| self.converse_stream(&self.model, system, messages, tools))
            .await
            .inspect_err(|e| {
                let _ = log.error(e);
            })?;

        Ok(bedrock_message_stream(response, model_name, log))
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_restart_steering(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_smithy_http_client::test_util::capture_request;

    /// A provider wired the way `from_env` builds one, minus the credential and
    /// global-config lookups — `from_env` needs UCSF-issued secrets, so it
    /// cannot run here. Everything below is a pure function of
    /// `resolved_endpoint`, and the client is built through the same
    /// `aws_config` loader production uses so the struct literal cannot drift
    /// from a real one.
    async fn provider_at(endpoint: &str) -> VersaBedrockProvider {
        let sdk_config =
            with_test_http_client(aws_config::defaults(aws_config::BehaviorVersion::latest()))
                .credentials_provider(Credentials::new(
                    "test-access-key",
                    "test-secret-key",
                    None,
                    None,
                    "VersaBedrockTest",
                ))
                .region(aws_config::Region::new(VERSA_BEDROCK_DEFAULT_REGION))
                .endpoint_url(endpoint.to_string())
                .load()
                .await;

        VersaBedrockProvider {
            client: Client::new(&sdk_config),
            model: ModelConfig::new_or_fail(VERSA_BEDROCK_DEFAULT_MODEL),
            retry_config: RetryConfig::default(),
            name: "versa_bedrock".to_string(),
            resolved_endpoint: endpoint.to_string(),
            region: VERSA_BEDROCK_DEFAULT_REGION.to_string(),
            operation_timeout_secs: Some(
                crate::providers::formats::bedrock::BEDROCK_DEFAULT_OPERATION_TIMEOUT_SECS,
            ),
        }
    }

    async fn capturing_provider() -> (
        VersaBedrockProvider,
        aws_smithy_http_client::test_util::CaptureRequestReceiver,
    ) {
        let (http_client, captured) = capture_request(None);
        let endpoint = "https://versa-bedrock.invalid";
        let sdk_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .credentials_provider(Credentials::new(
                "test-access-key",
                "test-secret-key",
                None,
                None,
                "VersaBedrockWireTest",
            ))
            .region(aws_config::Region::new(VERSA_BEDROCK_DEFAULT_REGION))
            .endpoint_url(endpoint)
            .http_client(http_client)
            .load()
            .await;
        (
            VersaBedrockProvider {
                client: Client::new(&sdk_config),
                model: ModelConfig::new_or_fail(VERSA_BEDROCK_DEFAULT_MODEL),
                retry_config: RetryConfig::default(),
                name: "versa_bedrock".to_string(),
                resolved_endpoint: endpoint.to_string(),
                region: VERSA_BEDROCK_DEFAULT_REGION.to_string(),
                operation_timeout_secs: Some(
                    crate::providers::formats::bedrock::BEDROCK_DEFAULT_OPERATION_TIMEOUT_SECS,
                ),
            },
            captured,
        )
    }

    #[tokio::test]
    async fn restore_binding_keeps_route_and_transport_policy_without_credentials() {
        let endpoint = "https://versa-bedrock.invalid/exact-route";
        let provider = provider_at(endpoint).await;
        let encoded = serde_json::to_value(provider.restore_binding()).unwrap();
        assert_eq!(encoded["kind"], "versa_bedrock");
        assert_eq!(encoded["endpoint"], endpoint);
        assert_eq!(encoded["region"], VERSA_BEDROCK_DEFAULT_REGION);
        assert_eq!(
            encoded["operation_timeout_secs"],
            crate::providers::formats::bedrock::BEDROCK_DEFAULT_OPERATION_TIMEOUT_SECS
        );
        let text = encoded.to_string();
        assert!(!text.contains("test-access-key"));
        assert!(!text.contains("test-secret-key"));
    }

    #[tokio::test]
    async fn provider_advertises_restart_steering() {
        let provider = provider_at(VERSA_BEDROCK_DEFAULT_ENDPOINT).await;
        assert!(provider.supports_streaming());
        assert!(!provider.supports_live_steering());
        assert!(
            provider.supports_restart_steering(),
            "Versa Bedrock cannot inject into ConverseStream, so a queued steer must restart it"
        );
    }

    #[tokio::test]
    async fn restore_reloads_rotated_credentials_and_fails_closed_when_they_are_missing() {
        use std::collections::HashMap;

        async fn restore_with(access_key: &str, secret_key: &str) -> Result<VersaBedrockProvider> {
            crate::config::with_config_overrides(
                HashMap::from([
                    ("VERSA_BEDROCK_ACCESS_KEY_ID".into(), access_key.into()),
                    ("VERSA_BEDROCK_SECRET_ACCESS_KEY".into(), secret_key.into()),
                ]),
                VersaBedrockProvider::from_resolved(
                    ModelConfig::new_or_fail(VERSA_BEDROCK_DEFAULT_MODEL),
                    SecretFreeEndpoint::new("https://versa-bedrock.invalid/exact".into()).unwrap(),
                    VERSA_BEDROCK_DEFAULT_REGION.into(),
                    PersistedRetryConfig {
                        max_retries: 6,
                        initial_interval_ms: 2_000,
                        backoff_multiplier: 2.0,
                        max_interval_ms: 120_000,
                    },
                    Some(300),
                ),
            )
            .await
        }

        let missing = restore_with("", "").await;
        assert!(missing.is_err());

        let first = restore_with("first-access-sentinel", "first-secret-sentinel")
            .await
            .unwrap();
        let second = restore_with("second-access-sentinel", "second-secret-sentinel")
            .await
            .unwrap();
        let first = serde_json::to_value(first.restore_binding()).unwrap();
        let second = serde_json::to_value(second.restore_binding()).unwrap();
        assert_eq!(
            first, second,
            "credential rotation must not change the binding"
        );
        let encoded = second.to_string();
        assert!(!encoded.contains("second-access-sentinel"));
        assert!(!encoded.contains("second-secret-sentinel"));
    }

    fn assert_inference_wire(
        captured: aws_smithy_http_client::test_util::CaptureRequestReceiver,
        expected_tokens: i64,
        expected_temperature: Option<f64>,
    ) {
        let request = captured.expect_request();
        let body = request.body().bytes().expect("buffered request body");
        let json: serde_json::Value = serde_json::from_slice(body).expect("JSON request body");
        assert_eq!(json["inferenceConfig"]["maxTokens"], expected_tokens);
        assert_eq!(
            json["inferenceConfig"]["temperature"],
            expected_temperature.map_or(serde_json::Value::Null, serde_json::Value::from)
        );
    }

    #[tokio::test]
    async fn converse_sends_configured_inference_fields_on_the_wire() {
        let (provider, captured) = capturing_provider().await;
        let config = ModelConfig::new_or_fail("us.anthropic.claude-sonnet-4-6")
            .with_max_tokens(Some(34_567))
            .with_temperature(Some(0.25));
        let _ = provider
            .converse(
                &config,
                "system",
                &[Message::user().with_text("hello")],
                &[],
            )
            .await;
        assert_inference_wire(captured, 21_333, Some(0.25));
    }

    #[tokio::test]
    async fn converse_uses_transport_safe_default_on_the_wire() {
        let (provider, captured) = capturing_provider().await;
        let config = ModelConfig::new_or_fail("us.anthropic.claude-sonnet-4-6");
        let _ = provider
            .converse(
                &config,
                "system",
                &[Message::user().with_text("hello")],
                &[],
            )
            .await;
        assert_inference_wire(captured, 21_333, None);
    }

    #[tokio::test]
    async fn converse_stream_sends_configured_inference_fields_on_the_wire() {
        let (provider, captured) = capturing_provider().await;
        let config = ModelConfig::new_or_fail("us.anthropic.claude-sonnet-4-6")
            .with_max_tokens(Some(45_678))
            .with_temperature(Some(0.5));
        let _ = provider
            .converse_stream(
                &config,
                "system",
                &[Message::user().with_text("hello")],
                &[],
            )
            .await;
        assert_inference_wire(captured, 45_678, Some(0.5));
    }

    #[tokio::test]
    async fn converse_stream_keeps_large_model_default_on_the_wire() {
        let (provider, captured) = capturing_provider().await;
        let config = ModelConfig::new_or_fail("us.anthropic.claude-sonnet-4-6");
        let _ = provider
            .converse_stream(
                &config,
                "system",
                &[Message::user().with_text("hello")],
                &[],
            )
            .await;
        assert_inference_wire(captured, 64_000, None);
    }

    /// Task 5 rule 2, **wired** — not just the predicate behind it.
    ///
    /// `providers::ucsf_gateway_tier` is unit-tested on its own in
    /// `tier_tests.rs`, but a test of the predicate alone cannot see whether
    /// this provider calls it, or hands it the right field. Replace the body of
    /// `tier()` with an unconditional `Private` and every one of those tests
    /// still passes. This one does not — and the demotion matters most here,
    /// because the last fallback in `from_env`'s endpoint chain is
    /// `AWS_ENDPOINT_URL_BEDROCK_RUNTIME`, which `bedrock.rs` sets
    /// **process-globally** with `std::env::set_var`.
    #[tokio::test]
    async fn tier_follows_the_endpoint_this_instance_resolved() {
        let shipped = provider_at(VERSA_BEDROCK_DEFAULT_ENDPOINT).await;
        assert_eq!(shipped.tier(), ProviderTier::Private);

        let elsewhere = provider_at("https://bedrock-runtime.us-west-2.amazonaws.com").await;
        // Same name, same metadata, same everything a name-keyed rule can see.
        assert_eq!(elsewhere.get_name(), shipped.get_name());
        assert_eq!(
            VersaBedrockProvider::metadata().tier,
            ProviderTier::Private,
            "the type-level claim is still Private; only the instance demotes"
        );
        assert_eq!(elsewhere.tier(), ProviderTier::Public);
    }

    /// DR-26 (Task 46) rule, **wired** — the same argument as the tier test
    /// above, for the third axis, and it matters most here: the last fallback in
    /// `from_env`'s endpoint chain is `AWS_ENDPOINT_URL_BEDROCK_RUNTIME`, which
    /// `bedrock.rs` sets **process-globally** with `std::env::set_var`. An
    /// affiliation keyed on the provider's name would keep claiming `ucsf` for
    /// an instance that another provider's construction had already repointed at
    /// a plain AWS region.
    #[tokio::test]
    async fn affiliation_follows_the_endpoint_this_instance_resolved() {
        use crate::privacy::affiliation::{InstitutionId, ModelAffiliation};

        let shipped = provider_at(VERSA_BEDROCK_DEFAULT_ENDPOINT).await;
        assert_eq!(
            shipped.affiliation(),
            Some(ModelAffiliation::institution(InstitutionId::new("ucsf")))
        );

        let elsewhere = provider_at("https://bedrock-runtime.us-west-2.amazonaws.com").await;
        assert_eq!(elsewhere.get_name(), shipped.get_name());
        assert_eq!(
            elsewhere.affiliation(),
            None,
            "an instance that lost Private must lose `ucsf` with it"
        );
    }

    /// The **type-level** half, which is what a catalog can group by before
    /// anything is configured. `GET /config/providers` resolves the instance
    /// affiliation above only for a CONFIGURED provider, so on a fresh machine
    /// this metadata claim is the only thing that can name the institution.
    ///
    /// ⚠ It is deliberately weaker than `affiliation()`: it says where this
    /// provider SHIPS pointed, so a repointed instance still carries `ucsf`
    /// here while the test above proves `affiliation()` correctly drops it.
    /// A surface must prefer the instance answer wherever the daemon resolved
    /// one — see `ProviderMetadata::institutions`.
    #[test]
    fn the_shipped_metadata_names_the_institution_for_an_unconfigured_row() {
        let institutions = VersaBedrockProvider::metadata().institutions.clone();
        assert_eq!(institutions.len(), 1);
        assert_eq!(institutions[0].id, "ucsf");
        assert_eq!(institutions[0].display_name.as_deref(), Some("UCSF"));
    }
}
