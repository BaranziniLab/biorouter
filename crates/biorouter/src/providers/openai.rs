use super::api_client::{ApiClient, AuthMethod};
use super::base::{ConfigKey, ModelInfo, Provider, ProviderMetadata, ProviderUsage, Usage};
use super::embedding::{EmbeddingCapable, EmbeddingRequest, EmbeddingResponse};
use super::errors::ProviderError;
use super::formats::openai::{
    add_reasoning_content_to_request, create_request, get_usage, model_uses_responses_api,
    response_to_message, stamp_reasoning_provenance,
};
use super::formats::openai_responses::{
    create_responses_request, get_responses_usage, responses_api_to_message, stream_responses_api,
    ResponsesApiResponse,
};
use super::retry::ProviderRetry;
use super::utils::{
    get_model, handle_response_openai_compat, handle_status_openai_compat, stream_openai_compat,
    ImageFormat,
};
use crate::config::declarative_providers::DeclarativeProviderConfig;
use crate::conversation::message::Message;
use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::StatusCode;
use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashMap;

use crate::model::ModelConfig;
use crate::providers::base::MessageStream;
use crate::providers::utils::RequestLog;
use rmcp::model::Tool;

/// GPT-6 Sol (GA 2026-09-22): the like-for-like successor of the previous
/// default `gpt-5.6` (an alias of `gpt-5.6-sol`) at half its price — $2 in /
/// $10 out per MTok against $4 / $20 — with the same 1,050,000-token window.
/// Astra is the more capable tier, at five times Sol's price.
pub const OPEN_AI_DEFAULT_MODEL: &str = "gpt-6-sol";
/// GPT-6 Luna ($0.10 / $0.50 per MTok, 1,050,000 tokens) replaced `gpt-5.4-mini`
/// ($0.75 / $4.50, 400,000 tokens): cheaper and a wider window, for the
/// background calls — titles, summaries — the fast model exists for.
pub const OPEN_AI_DEFAULT_FAST_MODEL: &str = "gpt-6-luna";
// Verified against OpenAI's model pages, deprecations page and changelog on
// 2026-09-25. Context windows are per-model pages at
// developers.openai.com/api/docs/models. Models marked [responses] route to
// /v1/responses instead of /v1/chat/completions.
//
// Not advertised, and why (developers.openai.com/api/docs/deprecations):
//   * gpt-5, gpt-5-2025-08-07, gpt-5-mini, gpt-5-nano and o3 — OpenAI
//     deprecated the only snapshot behind each (gpt-5-2025-08-07,
//     gpt-5-mini-2025-08-07, gpt-5-nano-2025-08-07, o3-2025-04-16) on
//     2026-06-11, with API shutdown on 2026-12-11. The named replacements are
//     gpt-5.6-sol (for gpt-5 and o3), gpt-5.6-terra and gpt-5.6-luna, all listed
//     below. Their `MODEL_CONTEXT_WINDOWS` entries stay, so a stored session
//     that names one still resolves its window until the shutdown.
//   * o1, o3-mini, o4-mini and gpt-4.1-nano — shutting down 2026-10-23.
//   * gpt-5.1-codex — shut down 2026-07-23.
//   * gpt-4o is NOT API-deprecated: only its gpt-4o-2024-05-13 snapshot is
//     (shutdown 2026-10-23). It is simply not offered; gpt-4.1 and gpt-4o-mini
//     cover the non-reasoning tier.
//   * There is no gpt-6-terra. Terra exists only as gpt-5.6-terra.
pub const OPEN_AI_KNOWN_MODELS: &[(&str, usize)] = &[
    // GPT-6 family [responses] — Astra GA 2026-09-03, Sol and Luna GA
    // 2026-09-22. All three: 1,050,000-token window (922,000 in), 128k max
    // output, text + image input. Tool calling needs /v1/responses: Chat
    // Completions allows function tools only with reasoning_effort `none`. Sol
    // is kept first so it stays the default the UI resolves to. Prompts over
    // 272k input tokens bill 2x input / 1.5x output for the whole request.
    ("gpt-6-sol", 1_050_000),
    ("gpt-6-astra", 1_050_000),
    ("gpt-6-luna", 1_050_000),
    // GPT-5.6 family [responses] — released 2026-07-09. All three variants
    // share a 1,050,000 context window / 128k max output. `gpt-5.6` is OpenAI's
    // documented alias for `gpt-5.6-sol`. Sol prices >272k-token prompts at 2x
    // input / 1.5x output for the whole request.
    ("gpt-5.6", 1_050_000),
    ("gpt-5.6-sol", 1_050_000),
    ("gpt-5.6-terra", 1_050_000),
    ("gpt-5.6-luna", 1_050_000),
    // GPT-5.5 family [responses] — requires /v1/responses API
    ("gpt-5.5", 1_050_000),
    ("gpt-5.5-pro", 1_050_000),
    // GPT-5.4 family [responses] — requires /v1/responses API
    ("gpt-5.4", 1_050_000),
    ("gpt-5.4-pro", 1_050_000),
    ("gpt-5.4-mini", 400_000),
    ("gpt-5.4-nano", 400_000),
    // Codex (agentic coding) [responses]
    ("gpt-5.3-codex", 400_000),
    // GPT-5.2 / 5.1 (previous generation, still active; Chat Completions)
    ("gpt-5.2", 400_000),
    ("gpt-5.1", 400_000),
    // GPT-4.1 family (non-reasoning)
    ("gpt-4.1", 1_047_576),
    ("gpt-4.1-mini", 1_047_576),
    // GPT-4o mini (active; non-reasoning)
    ("gpt-4o-mini", 128_000),
];

pub const OPEN_AI_DOC_URL: &str = "https://platform.openai.com/docs/models";

/// Built-in model-id aliases for OpenAI-compatible hosts that are retiring a
/// model name, so a user's saved config keeps working after the vendor removes
/// the old id. Keyed by the API host; returns `old id -> live id`.
///
/// DeepSeek discontinued `deepseek-chat` / `deepseek-reasoner` on 2026-07-24
/// (both had been aliases of V4-Flash since the V4 launch), and retired the
/// V4-Flash model itself on 2026-09-10: `deepseek-v4-flash` is now only
/// "temporarily routed" to V4.1-Flash, whose id is `deepseek-flash` — the name
/// DeepSeek's docs tell callers to use (api-docs.deepseek.com, read
/// 2026-09-25). Rewriting all three on the wire keeps a saved config working —
/// including a custom provider pointed at a `deepseek.com` host — and stops
/// depending on a compatibility route DeepSeek has given no end date for.
/// Mapping to Flash (not `-pro`) is faithful: V4.1-Flash has thinking on by
/// default, so `deepseek-reasoner` behaviour is preserved with no cost jump.
fn builtin_model_aliases(host: &str) -> Option<HashMap<String, String>> {
    let host = host.trim().to_ascii_lowercase();
    if host == "deepseek.com" || host == "api.deepseek.com" || host.ends_with(".deepseek.com") {
        return Some(HashMap::from([
            ("deepseek-chat".to_string(), "deepseek-flash".to_string()),
            (
                "deepseek-reasoner".to_string(),
                "deepseek-flash".to_string(),
            ),
            (
                "deepseek-v4-flash".to_string(),
                "deepseek-flash".to_string(),
            ),
        ]));
    }
    None
}

#[derive(Debug, serde::Serialize)]
pub struct OpenAiProvider {
    #[serde(skip)]
    api_client: ApiClient,
    base_path: String,
    organization: Option<String>,
    project: Option<String>,
    model: ModelConfig,
    custom_headers: Option<HashMap<String, String>>,
    supports_streaming: bool,
    name: String,
    /// `old model id -> live model id` rewrites applied just before a request is
    /// sent, so retired upstream ids keep working. See [`builtin_model_aliases`].
    model_aliases: Option<HashMap<String, String>>,
}

impl OpenAiProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let model = model.with_fast(OPEN_AI_DEFAULT_FAST_MODEL.to_string());

        let config = crate::config::Config::global();
        let secrets = config.get_secrets("OPENAI_API_KEY", &["OPENAI_CUSTOM_HEADERS"])?;
        let api_key = secrets.get("OPENAI_API_KEY").unwrap().clone();
        let host: String = config
            .get_param("OPENAI_HOST")
            .unwrap_or_else(|_| "https://api.openai.com".to_string());
        let base_path: String = config
            .get_param("OPENAI_BASE_PATH")
            .unwrap_or_else(|_| "v1/chat/completions".to_string());
        let organization: Option<String> = config.get_param("OPENAI_ORGANIZATION").ok();
        let project: Option<String> = config.get_param("OPENAI_PROJECT").ok();
        let custom_headers: Option<HashMap<String, String>> = secrets
            .get("OPENAI_CUSTOM_HEADERS")
            .cloned()
            .map(parse_custom_headers);
        let timeout_secs: u64 = config.get_param("OPENAI_TIMEOUT").unwrap_or(600);

        let auth = AuthMethod::BearerToken(api_key);
        let mut api_client =
            ApiClient::with_timeout(host, auth, std::time::Duration::from_secs(timeout_secs))?;

        if let Some(org) = &organization {
            api_client = api_client.with_header("OpenAI-Organization", org)?;
        }

        if let Some(project) = &project {
            api_client = api_client.with_header("OpenAI-Project", project)?;
        }

        if let Some(headers) = &custom_headers {
            let mut header_map = reqwest::header::HeaderMap::new();
            for (key, value) in headers {
                let header_name = reqwest::header::HeaderName::from_bytes(key.as_bytes())?;
                let header_value = reqwest::header::HeaderValue::from_str(value)?;
                header_map.insert(header_name, header_value);
            }
            api_client = api_client.with_headers(header_map)?;
        }

        Ok(Self {
            api_client,
            base_path,
            organization,
            project,
            model,
            custom_headers,
            supports_streaming: true,
            name: Self::metadata().name,
            model_aliases: None,
        })
    }

    #[doc(hidden)]
    pub fn new(api_client: ApiClient, model: ModelConfig) -> Self {
        Self {
            api_client,
            base_path: "v1/chat/completions".to_string(),
            organization: None,
            project: None,
            model,
            custom_headers: None,
            supports_streaming: true,
            name: Self::metadata().name,
            model_aliases: None,
        }
    }

    pub fn from_custom_config(
        model: ModelConfig,
        config: DeclarativeProviderConfig,
    ) -> Result<Self> {
        let global_config = crate::config::Config::global();
        let api_key: String = global_config
            .get_secret(&config.api_key_env)
            .map_err(|_e| anyhow::anyhow!("Missing API key: {}", config.api_key_env))?;

        let url = url::Url::parse(&config.base_url)
            .map_err(|e| anyhow::anyhow!("Invalid base URL '{}': {}", config.base_url, e))?;

        let model_aliases = builtin_model_aliases(url.host_str().unwrap_or(""));

        let host = if let Some(port) = url.port() {
            format!(
                "{}://{}:{}",
                url.scheme(),
                url.host_str().unwrap_or(""),
                port
            )
        } else {
            format!("{}://{}", url.scheme(), url.host_str().unwrap_or(""))
        };
        let base_path = url.path().trim_start_matches('/').to_string();
        let base_path = if base_path.is_empty() || base_path == "v1" || base_path == "v1/" {
            "v1/chat/completions".to_string()
        } else {
            base_path
        };

        let timeout_secs = config.timeout_seconds.unwrap_or(600);
        let auth = AuthMethod::BearerToken(api_key);
        let mut api_client =
            ApiClient::with_timeout(host, auth, std::time::Duration::from_secs(timeout_secs))?;

        // Add custom headers if present
        if let Some(headers) = &config.headers {
            let mut header_map = reqwest::header::HeaderMap::new();
            for (key, value) in headers {
                let header_name = reqwest::header::HeaderName::from_bytes(key.as_bytes())?;
                let header_value = reqwest::header::HeaderValue::from_str(value)?;
                header_map.insert(header_name, header_value);
            }
            api_client = api_client.with_headers(header_map)?;
        }

        Ok(Self {
            api_client,
            base_path,
            organization: None,
            project: None,
            model,
            custom_headers: config.headers,
            supports_streaming: config.supports_streaming.unwrap_or(true),
            name: config.name.clone(),
            model_aliases,
        })
    }

    /// Rewrite a retired model id to its live replacement just before sending a
    /// request. Returns the input untouched when no alias applies, so the common
    /// path allocates nothing.
    fn resolve_model<'a>(&self, model_config: &'a ModelConfig) -> Cow<'a, ModelConfig> {
        if let Some(target) = self
            .model_aliases
            .as_ref()
            .and_then(|aliases| aliases.get(&model_config.model_name))
            .filter(|target| *target != &model_config.model_name)
        {
            tracing::debug!(
                from = %model_config.model_name,
                to = %target,
                "remapping retired model id to its live replacement"
            );
            let mut remapped = model_config.clone();
            remapped.model_name = target.clone();
            return Cow::Owned(remapped);
        }
        Cow::Borrowed(model_config)
    }

    fn uses_responses_api(model_name: &str) -> bool {
        model_uses_responses_api(model_name)
    }

    async fn post(&self, payload: &Value) -> Result<Value, ProviderError> {
        let response = self
            .api_client
            .response_post(&self.base_path, payload)
            .await?;
        handle_response_openai_compat(response).await
    }

    async fn post_responses(&self, payload: &Value) -> Result<Value, ProviderError> {
        let response = self
            .api_client
            .response_post("v1/responses", payload)
            .await?;
        handle_response_openai_compat(response).await
    }
}

#[async_trait]
impl Provider for OpenAiProvider {
    fn metadata() -> ProviderMetadata {
        // Per OpenAI's published model docs, every model in the catalog accepts
        // image input. That includes gpt-5.3-codex: its model page lists "Input
        // modalities: text, image" (read 2026-09-25), so the old exclusion of
        // codex variants as text-focused was wrong for it.
        const OPEN_AI_VISION_MODELS: &[&str] = &[
            "gpt-6-sol",
            "gpt-6-astra",
            "gpt-6-luna",
            "gpt-5.6",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
            "gpt-5.5",
            "gpt-5.5-pro",
            "gpt-5.4",
            "gpt-5.4-pro",
            "gpt-5.4-mini",
            "gpt-5.4-nano",
            "gpt-5.3-codex",
            "gpt-5.1",
            "gpt-5.2",
            "gpt-4.1",
            "gpt-4.1-mini",
            "gpt-4o-mini",
        ];
        let models = OPEN_AI_KNOWN_MODELS
            .iter()
            .map(|(name, limit)| {
                let info = ModelInfo::new(*name, *limit);
                if OPEN_AI_VISION_MODELS.contains(name) {
                    info.with_vision()
                } else {
                    info
                }
            })
            .collect();
        ProviderMetadata::with_models(
            "openai",
            "OpenAI",
            "GPT-4 and other OpenAI models, including OpenAI compatible ones",
            OPEN_AI_DEFAULT_MODEL,
            models,
            OPEN_AI_DOC_URL,
            vec![
                ConfigKey::new("OPENAI_API_KEY", true, true, None),
                ConfigKey::new("OPENAI_HOST", true, false, Some("https://api.openai.com")),
                ConfigKey::new("OPENAI_BASE_PATH", true, false, Some("v1/chat/completions")),
                ConfigKey::new("OPENAI_ORGANIZATION", false, false, None),
                ConfigKey::new("OPENAI_PROJECT", false, false, None),
                ConfigKey::new("OPENAI_CUSTOM_HEADERS", false, true, None),
                ConfigKey::new("OPENAI_TIMEOUT", false, false, Some("600")),
            ],
        )
        .with_unlisted_models()
    }

    fn get_name(&self) -> &str {
        &self.name
    }

    fn computer_use_destination(&self) -> Option<String> {
        self.api_client.computer_use_destination(&self.base_path)
    }

    fn computer_use_destination_identity(&self) -> Option<String> {
        Some(
            self.api_client
                .computer_use_destination_identity(&self.base_path),
        )
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
        let resolved = self.resolve_model(model_config);
        let model_config = resolved.as_ref();
        if Self::uses_responses_api(&model_config.model_name) {
            let payload = create_responses_request(model_config, system, messages, tools)?;
            let mut log = RequestLog::start(&self.model, &payload)?;

            let json_response = self
                .with_retry(|| async {
                    let payload_clone = payload.clone();
                    self.post_responses(&payload_clone).await
                })
                .await
                .inspect_err(|e| {
                    let _ = log.error(e);
                })?;

            let responses_api_response: ResponsesApiResponse =
                serde_json::from_value(json_response.clone()).map_err(|e| {
                    ProviderError::ExecutionError(format!(
                        "Failed to parse responses API response: {}",
                        e
                    ))
                })?;

            let message = responses_api_to_message(&responses_api_response)?;
            let usage = get_responses_usage(&responses_api_response);
            let model = responses_api_response.model.clone();

            log.write(&json_response, Some(&usage))?;
            Ok((message, ProviderUsage::new(model, usage)))
        } else {
            let mut payload = create_request(
                model_config,
                system,
                messages,
                tools,
                &ImageFormat::OpenAi,
                false,
            )?;
            if replays_reasoning_content(&self.name) {
                add_reasoning_content_to_request(&mut payload, messages, &self.name);
            }

            let mut log = RequestLog::start(&self.model, &payload)?;
            let json_response = self
                .with_retry(|| async {
                    let payload_clone = payload.clone();
                    self.post(&payload_clone).await
                })
                .await
                .inspect_err(|e| {
                    let _ = log.error(e);
                })?;

            let mut message = response_to_message(&json_response)?;
            // Record which provider produced any captured reasoning_content so
            // replay (above) stays scoped to this provider across a mid-session
            // provider switch.
            stamp_reasoning_provenance(&mut message, &self.name);
            let usage = json_response
                .get("usage")
                .map(get_usage)
                .unwrap_or_else(|| {
                    tracing::debug!("Failed to get usage data");
                    Usage::default()
                });

            let model = get_model(&json_response);
            log.write(&json_response, Some(&usage))?;
            Ok((message, ProviderUsage::new(model, usage)))
        }
    }

    async fn fetch_supported_models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        let models_path = self.base_path.replace("v1/chat/completions", "v1/models");
        let response = self.api_client.response_get(&models_path).await?;
        let json = handle_response_openai_compat(response).await?;
        if let Some(err_obj) = json.get("error") {
            let msg = err_obj
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(ProviderError::Authentication(msg.to_string()));
        }

        let data = json.get("data").and_then(|v| v.as_array()).ok_or_else(|| {
            ProviderError::UsageError("Missing data field in JSON response".into())
        })?;
        let mut models: Vec<String> = data
            .iter()
            .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        models.sort();
        Ok(Some(models))
    }

    fn supports_embeddings(&self) -> bool {
        true
    }

    async fn create_embeddings(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, ProviderError> {
        EmbeddingCapable::create_embeddings(self, texts)
            .await
            .map_err(|e| ProviderError::ExecutionError(e.to_string()))
    }

    fn supports_streaming(&self) -> bool {
        self.supports_streaming
    }

    async fn stream(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let resolved = self.resolve_model(&self.model);
        let model = resolved.as_ref();
        if Self::uses_responses_api(&model.model_name) {
            let mut payload = create_responses_request(model, system, messages, tools)?;
            payload["stream"] = serde_json::Value::Bool(true);

            let mut log = RequestLog::start(model, &payload)?;

            let response = self
                .with_retry(|| async {
                    let payload_clone = payload.clone();
                    let resp = self
                        .api_client
                        .response_post("v1/responses", &payload_clone)
                        .await?;
                    handle_status_openai_compat(resp).await
                })
                .await
                .inspect_err(|e| {
                    let _ = log.error(e);
                })?;

            Ok(stream_responses_api(response, log))
        } else {
            let mut payload =
                create_request(model, system, messages, tools, &ImageFormat::OpenAi, true)?;
            if replays_reasoning_content(&self.name) {
                add_reasoning_content_to_request(&mut payload, messages, &self.name);
            }
            let mut log = RequestLog::start(model, &payload)?;

            let response = self
                .with_retry(|| async {
                    let resp = self
                        .api_client
                        .response_post(&self.base_path, &payload)
                        .await?;
                    handle_status_openai_compat(resp).await
                })
                .await
                .inspect_err(|e| {
                    let _ = log.error(e);
                })?;

            let stream = stream_openai_compat(response, log)?;
            // Stamp reasoning provenance on streamed messages too — the
            // decoder attaches captured reasoning_content to the final
            // tool-request message it yields, and replay is provenance-gated.
            let provider_name = self.name.clone();
            Ok(Box::pin(stream.map(move |item| {
                item.map(|(message, usage, pending)| {
                    let message = message.map(|mut m| {
                        stamp_reasoning_provenance(&mut m, &provider_name);
                        m
                    });
                    (message, usage, pending)
                })
            })))
        }
    }
}

/// OpenAI-compatible providers whose API REQUIRES the model's
/// `reasoning_content` to be replayed on prior assistant messages during a
/// tool-calling turn. Moonshot's Kimi docs (K2.6 quickstart, 2026-07):
/// "During multi-step tool calling, you must keep the `reasoning_content` ...
/// otherwise an error will be thrown"; K2.7-Code forces thinking on every
/// turn. Deliberately a provider-name allowlist rather than
/// metadata-presence gating: DeepSeek also *emits* `reasoning_content` but
/// its API rejects the field on input, so replay must stay opt-in.
fn replays_reasoning_content(provider_name: &str) -> bool {
    provider_name == "moonshot"
}

fn parse_custom_headers(s: String) -> HashMap<String, String> {
    s.split(',')
        .filter_map(|header| {
            let mut parts = header.splitn(2, '=');
            let key = parts.next().map(|s| s.trim().to_string())?;
            let value = parts.next().map(|s| s.trim().to_string())?;
            Some((key, value))
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod alias_tests {
    use super::*;
    use crate::providers::api_client::{ApiClient, AuthMethod};

    fn model(name: &str) -> ModelConfig {
        ModelConfig::new(name).unwrap()
    }

    // Replay of `reasoning_content` is an explicit per-provider opt-in:
    // Moonshot REQUIRES it mid tool-loop; DeepSeek (which also emits the
    // field) rejects it on input; OpenAI proper never emits it.
    #[test]
    fn only_moonshot_replays_reasoning_content() {
        assert!(replays_reasoning_content("moonshot"));
        for name in [
            "openai",
            "deepseek",
            "custom_deepseek",
            "groq",
            "mistral",
            "inception",
            "openrouter",
        ] {
            assert!(!replays_reasoning_content(name), "{name} must not replay");
        }
    }

    #[test]
    fn computer_use_disclosure_tracks_actual_request_origin_and_route_identity() {
        let mut provider = provider_for_host("https://user:secret@gateway.example:8443/private");
        assert_eq!(
            provider.computer_use_destination().as_deref(),
            Some("https://gateway.example:8443")
        );
        let before = provider.computer_use_destination_identity().unwrap();
        provider.base_path = "another-route?token=hidden".into();
        assert_eq!(
            provider.computer_use_destination().as_deref(),
            Some("https://gateway.example:8443")
        );
        assert_ne!(
            provider.computer_use_destination_identity().unwrap(),
            before
        );
        provider.base_path = "https://other.example/inference?token=hidden".into();
        assert_eq!(
            provider.computer_use_destination().as_deref(),
            Some("https://other.example")
        );
    }

    fn provider_for_host(host: &str) -> OpenAiProvider {
        let api_client = ApiClient::new(
            host.to_string(),
            AuthMethod::BearerToken("test".to_string()),
        )
        .unwrap();
        let mut p = OpenAiProvider::new(api_client, model("deepseek-chat"));
        p.model_aliases = builtin_model_aliases(
            url::Url::parse(host)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string))
                .unwrap_or_default()
                .as_str(),
        );
        p
    }

    #[test]
    fn deepseek_host_aliases_retired_ids() {
        let aliases = builtin_model_aliases("api.deepseek.com").expect("deepseek host has aliases");
        // Discontinued 2026-07-24, and V4-Flash (their old target) retired
        // 2026-09-10: all three land on V4.1-Flash's own id.
        for retired in ["deepseek-chat", "deepseek-reasoner", "deepseek-v4-flash"] {
            assert_eq!(
                aliases.get(retired).map(String::as_str),
                Some("deepseek-flash"),
                "{retired}"
            );
        }
        // No alias may point at an id that is itself retired, or a rewrite would
        // only move the failure.
        for target in aliases.values() {
            assert!(
                !aliases.contains_key(target),
                "{target} is both a target and a retired id"
            );
        }
        // V4-Pro is still served (repriced 2026-08-16), so it is not rewritten.
        assert!(!aliases.contains_key("deepseek-v4-pro"));
    }

    #[test]
    fn deepseek_host_matching_is_case_insensitive_and_covers_subdomains() {
        assert!(builtin_model_aliases("API.DeepSeek.com").is_some());
        assert!(builtin_model_aliases("eu.deepseek.com").is_some());
        assert!(builtin_model_aliases("deepseek.com").is_some());
    }

    #[test]
    fn non_deepseek_hosts_have_no_aliases() {
        assert!(builtin_model_aliases("api.openai.com").is_none());
        assert!(builtin_model_aliases("api.deepseek.com.evil.example").is_none());
        assert!(builtin_model_aliases("").is_none());
    }

    #[test]
    fn resolve_model_rewrites_retired_id_only() {
        let p = provider_for_host("https://api.deepseek.com");

        let chat = model("deepseek-chat");
        assert_eq!(p.resolve_model(&chat).model_name, "deepseek-flash");

        let reasoner = model("deepseek-reasoner");
        assert_eq!(p.resolve_model(&reasoner).model_name, "deepseek-flash");

        let v4_flash = model("deepseek-v4-flash");
        assert_eq!(p.resolve_model(&v4_flash).model_name, "deepseek-flash");

        // Live ids are passed through untouched (no allocation/rewrite).
        for live in ["deepseek-flash", "deepseek-v4-pro"] {
            let config = model(live);
            assert!(
                matches!(p.resolve_model(&config), Cow::Borrowed(_)),
                "{live} must not be rewritten"
            );
            assert_eq!(p.resolve_model(&config).model_name, live);
        }
    }

    #[test]
    fn resolve_model_is_noop_without_aliases() {
        let api_client = ApiClient::new(
            "https://api.openai.com".to_string(),
            AuthMethod::BearerToken("test".to_string()),
        )
        .unwrap();
        let p = OpenAiProvider::new(api_client, model("deepseek-chat"));
        // No alias table → the (now-retired) id is left as-is.
        let chat = model("deepseek-chat");
        assert_eq!(p.resolve_model(&chat).model_name, "deepseek-chat");
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod model_capability_tests {
    use super::*;
    use crate::providers::formats::openai::{
        model_reasoning_effort, model_supports_reasoning_effort,
    };

    /// The three GPT-5.6 variants plus the `gpt-5.6` alias, all 1,050,000 ctx.
    /// Verified against developers.openai.com/api/docs/models (July 2026).
    const GPT_5_6_IDS: &[&str] = &["gpt-5.6", "gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"];

    #[test]
    fn gpt_5_6_family_is_in_the_known_model_catalog() {
        for id in GPT_5_6_IDS {
            let entry = OPEN_AI_KNOWN_MODELS.iter().find(|(name, _)| name == id);
            let (_, limit) = entry.unwrap_or_else(|| panic!("{id} missing from known models"));
            assert_eq!(*limit, 1_050_000, "{id} should have a 1,050,000 ctx window");
        }
    }

    #[test]
    fn gpt_5_6_routes_to_the_responses_api() {
        // These reject function tools on /v1/chat/completions, same as 5.4/5.5.
        for id in GPT_5_6_IDS {
            assert!(
                OpenAiProvider::uses_responses_api(id),
                "{id} must route to /v1/responses"
            );
        }
    }

    #[test]
    fn older_chat_completions_models_are_not_rerouted() {
        // Guard the `starts_with("gpt-5.6")` prefix against over-matching.
        for id in ["gpt-5", "gpt-5-mini", "gpt-5.2", "gpt-4.1", "o3"] {
            assert!(
                !OpenAiProvider::uses_responses_api(id),
                "{id} must stay on /v1/chat/completions"
            );
        }
    }

    #[test]
    fn o4_mini_reasoning_tool_calls_use_the_responses_api() {
        for id in ["o4-mini", "o4-mini-2025-04-16"] {
            assert!(
                OpenAiProvider::uses_responses_api(id),
                "{id} must use /v1/responses so function tools and reasoning effort can be combined"
            );
        }
    }

    #[test]
    fn every_configured_openai_model_has_the_expected_reasoning_capability() {
        const NON_REASONING_MODELS: &[&str] = &["gpt-4.1", "gpt-4.1-mini", "gpt-4o-mini"];

        for (id, _) in OPEN_AI_KNOWN_MODELS {
            let expected_support = !NON_REASONING_MODELS.contains(id);
            assert_eq!(
                model_supports_reasoning_effort(id),
                expected_support,
                "unexpected reasoning capability for {id}"
            );
            let expected_quick_effort = if !expected_support {
                None
            } else if id.ends_with("-pro") {
                Some("medium")
            } else {
                Some("low")
            };
            assert_eq!(
                model_reasoning_effort(id, "low"),
                expected_quick_effort,
                "unexpected Quick effort mapping for {id}"
            );
        }

        // A saved session can still name this recently deprecated model even
        // though it is no longer offered in the new-session picker.
        assert!(model_supports_reasoning_effort("o4-mini-2025-04-16"));
    }

    #[test]
    fn every_configured_azure_openai_model_has_the_expected_reasoning_capability() {
        use crate::providers::azure::AZURE_OPENAI_KNOWN_MODELS;

        for id in AZURE_OPENAI_KNOWN_MODELS {
            let expected = !id.starts_with("gpt-4.1") && !id.starts_with("gpt-4o");
            assert_eq!(
                model_supports_reasoning_effort(id),
                expected,
                "unexpected reasoning capability for Azure OpenAI model {id}"
            );
        }
    }

    #[test]
    fn default_model_is_gpt_6_sol_and_is_a_listed_model() {
        assert_eq!(OPEN_AI_DEFAULT_MODEL, "gpt-6-sol");
        assert_eq!(
            OPEN_AI_KNOWN_MODELS.first().map(|(name, _)| *name),
            Some(OPEN_AI_DEFAULT_MODEL),
            "the default is kept first in the catalog"
        );
        assert_eq!(OPEN_AI_DEFAULT_FAST_MODEL, "gpt-6-luna");
        for id in [OPEN_AI_DEFAULT_MODEL, OPEN_AI_DEFAULT_FAST_MODEL] {
            assert!(
                OPEN_AI_KNOWN_MODELS.iter().any(|(name, _)| *name == id),
                "{id} must appear in the catalog so the UI can resolve it"
            );
        }
    }

    /// GPT-6 on the OpenAI API: Astra (GA 2026-09-03), Sol and Luna (GA
    /// 2026-09-22). There is no `gpt-6-terra`.
    const GPT_6_IDS: &[&str] = &["gpt-6-sol", "gpt-6-astra", "gpt-6-luna"];

    #[test]
    fn gpt_6_family_is_advertised_with_its_window_vision_and_responses_route() {
        let meta = OpenAiProvider::metadata();
        for id in GPT_6_IDS {
            let (_, limit) = OPEN_AI_KNOWN_MODELS
                .iter()
                .find(|(name, _)| name == id)
                .unwrap_or_else(|| panic!("{id} missing from known models"));
            assert_eq!(*limit, 1_050_000, "{id} should have a 1,050,000 ctx window");
            let model = meta
                .known_models
                .iter()
                .find(|m| m.name == *id)
                .unwrap_or_else(|| panic!("{id} missing from provider metadata"));
            assert_eq!(
                model.supports_vision,
                Some(true),
                "{id} accepts image input per OpenAI"
            );
            // Chat Completions takes GPT-6 function tools only with effort
            // `none`, which BioRouter never sends.
            assert!(
                OpenAiProvider::uses_responses_api(id),
                "{id} must route to /v1/responses"
            );
            assert!(
                model_supports_reasoning_effort(id),
                "{id} is a reasoning model"
            );
        }
        assert!(
            !OPEN_AI_KNOWN_MODELS
                .iter()
                .any(|(name, _)| *name == "gpt-6-terra"),
            "gpt-6-terra does not exist; Terra is gpt-5.6-terra"
        );
    }

    #[test]
    fn gpt_6_context_limit_resolves_from_model_config() {
        let _guard = env_lock::lock_env([
            ("BIOROUTER_CONTEXT_LIMIT", None::<&str>),
            ("BIOROUTER_PREDEFINED_MODELS", None::<&str>),
        ]);
        for id in GPT_6_IDS {
            let cfg = ModelConfig::new(id).unwrap();
            assert_eq!(cfg.context_limit(), 1_050_000, "{id} ctx limit");
        }
    }

    /// OpenAI deprecated the only snapshot behind each of these on 2026-06-11
    /// (API shutdown 2026-12-11), so a new chat is not offered them. A stored
    /// chat that names one still resolves its own window and is still shaped
    /// and routed as before, until the shutdown turns it into an API error.
    #[test]
    fn deprecated_gpt_5_and_o3_ids_are_not_advertised_but_stored_chats_still_resolve() {
        let _guard = env_lock::lock_env([
            ("BIOROUTER_CONTEXT_LIMIT", None::<&str>),
            ("BIOROUTER_PREDEFINED_MODELS", None::<&str>),
        ]);
        let meta = OpenAiProvider::metadata();
        for (id, window) in [
            ("gpt-5", 400_000),
            ("gpt-5-2025-08-07", 400_000),
            ("gpt-5-mini", 400_000),
            ("gpt-5-nano", 400_000),
            ("o3", 200_000),
        ] {
            assert!(
                !OPEN_AI_KNOWN_MODELS.iter().any(|(name, _)| *name == id),
                "{id} is deprecated (shutdown 2026-12-11) and must not be offered"
            );
            assert!(
                !meta.known_models.iter().any(|m| m.name == id),
                "{id} leaked into provider metadata"
            );
            assert!(
                meta.allows_unlisted_models,
                "a stored {id} chat stays selectable"
            );
            assert_eq!(
                ModelConfig::new(id).unwrap().context_limit(),
                window,
                "{id} lost its window"
            );
            assert!(
                model_supports_reasoning_effort(id),
                "{id} is still a reasoning model"
            );
            assert!(
                !OpenAiProvider::uses_responses_api(id),
                "{id} still answers on /v1/chat/completions"
            );
        }
    }

    #[test]
    fn gpt_5_3_codex_advertises_vision() {
        // developers.openai.com/api/docs/models/gpt-5.3-codex lists "Input
        // modalities: text, image".
        let meta = OpenAiProvider::metadata();
        let codex = meta
            .known_models
            .iter()
            .find(|m| m.name == "gpt-5.3-codex")
            .expect("gpt-5.3-codex is advertised");
        assert_eq!(codex.supports_vision, Some(true));
    }

    #[test]
    fn gpt_5_6_family_advertises_vision() {
        let meta = OpenAiProvider::metadata();
        for id in GPT_5_6_IDS {
            let model = meta
                .known_models
                .iter()
                .find(|m| m.name == *id)
                .unwrap_or_else(|| panic!("{id} missing from provider metadata"));
            assert_eq!(
                model.supports_vision,
                Some(true),
                "{id} accepts image input per OpenAI"
            );
        }
    }

    #[test]
    fn gpt_5_6_context_limit_resolves_from_model_config() {
        // Exercises the separate `contains`-based table in model.rs. `context_limit()`
        // honours BIOROUTER_CONTEXT_LIMIT, which other tests set process-wide, so take
        // the shared env lock and pin it unset rather than racing them.
        let _guard = env_lock::lock_env([
            ("BIOROUTER_CONTEXT_LIMIT", None::<&str>),
            ("BIOROUTER_PREDEFINED_MODELS", None::<&str>),
        ]);
        for id in GPT_5_6_IDS {
            let cfg = ModelConfig::new(id).unwrap();
            assert_eq!(cfg.context_limit(), 1_050_000, "{id} ctx limit");
        }
    }
}

#[async_trait]
impl EmbeddingCapable for OpenAiProvider {
    async fn create_embeddings(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        let embedding_model = std::env::var("BIOROUTER_EMBEDDING_MODEL")
            .unwrap_or_else(|_| "text-embedding-3-small".to_string());

        let request = EmbeddingRequest {
            input: texts,
            model: embedding_model,
        };

        let response = self
            .with_retry(|| async {
                let request_clone = EmbeddingRequest {
                    input: request.input.clone(),
                    model: request.model.clone(),
                };
                let request_value = serde_json::to_value(request_clone)
                    .map_err(|e| ProviderError::ExecutionError(e.to_string()))?;
                self.api_client
                    .api_post("v1/embeddings", &request_value)
                    .await
                    .map_err(|e| ProviderError::ExecutionError(e.to_string()))
            })
            .await?;

        if response.status != StatusCode::OK {
            let error_text = response
                .payload
                .as_ref()
                .and_then(|p| p.as_str())
                .unwrap_or("Unknown error");
            return Err(anyhow::anyhow!("Embedding API error: {}", error_text));
        }

        let embedding_response: EmbeddingResponse = serde_json::from_value(
            response
                .payload
                .ok_or_else(|| anyhow::anyhow!("Empty response body"))?,
        )?;

        Ok(embedding_response
            .data
            .into_iter()
            .map(|d| d.embedding)
            .collect())
    }
}
