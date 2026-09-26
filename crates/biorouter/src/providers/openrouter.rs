use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::api_client::{ApiClient, AuthMethod};
use super::base::{
    ConfigKey, MessageStream, ModelInfo, Provider, ProviderMetadata, ProviderUsage, Usage,
};
use super::errors::ProviderError;
use super::retry::ProviderRetry;
use super::utils::{
    get_model, handle_response_openai_compat, handle_status_openai_compat, stream_openai_compat,
    RequestLog,
};
use crate::conversation::message::Message;

use crate::model::ModelConfig;
use crate::providers::formats::openai::{create_request, get_usage};
use crate::providers::formats::openrouter as openrouter_format;
use rmcp::model::Tool;

// Sonnet 5 is Anthropic's current Sonnet: GA on OpenRouter since Jun 30, 2026,
// cheaper than Sonnet 4.6 ($2/$10 vs $3/$15 per MTok) with the same 1M/128K
// limits. Sonnet 4.6 is still active and stays listed.
pub const OPENROUTER_DEFAULT_MODEL: &str = "anthropic/claude-sonnet-5";
pub const OPENROUTER_DEFAULT_FAST_MODEL: &str = "google/gemini-3.5-flash";
pub const OPENROUTER_MODEL_PREFIX_ANTHROPIC: &str = "anthropic";

// OpenRouter can run many models; this is a curated list of current,
// tool-capable slugs verified against the live /api/v1/models catalog
// (Sep 25, 2026): every slug below lists `tools` in its supported_parameters
// and none carries an expiration_date. x-ai/grok-code-fast-1 was removed from
// OpenRouter. There is no gpt-6-terra; Terra exists only as gpt-5.6-terra.
pub const OPENROUTER_KNOWN_MODELS: &[&str] = &[
    "anthropic/claude-opus-5.5",
    "anthropic/claude-fable-5.1",
    "anthropic/claude-opus-4.8",
    "anthropic/claude-sonnet-5",
    "anthropic/claude-sonnet-4.6",
    "anthropic/claude-haiku-4.5",
    "openai/gpt-6-astra",
    "openai/gpt-6-sol",
    "openai/gpt-6-luna",
    "openai/gpt-5.6-terra",
    "google/gemini-3.1-pro-preview",
    "google/gemini-3.8-flash",
    "google/gemini-3.5-flash",
    "x-ai/grok-4.7",
    "x-ai/grok-4.3",
    "x-ai/grok-4.20",
    "x-ai/grok-build-0.1",
    "deepseek/deepseek-v4-pro",
    "deepseek/deepseek-v4.1-flash",
    "deepseek/deepseek-v4-flash",
    "qwen/qwen3-coder-next",
    "moonshotai/kimi-k3",
    "moonshotai/kimi-k2.7-code",
    "moonshotai/kimi-k2.6",
    "z-ai/glm-5.3",
    "z-ai/glm-5.2",
    "z-ai/glm-5.1",
    "minimax/minimax-m3",
];
pub const OPENROUTER_DOC_URL: &str = "https://openrouter.ai/models";

/// xAI's chat models take png/jpeg only (docs.x.ai), so they get the narrower
/// MIME list rather than `with_vision()`'s.
fn openrouter_model_is_png_jpeg_only(normalized: &str) -> bool {
    normalized.starts_with("x-ai/grok-4") || normalized == "x-ai/grok-build-0.1"
}

/// Image input per OpenRouter's `architecture.input_modalities` (Sep 25, 2026).
/// Text-only slugs in the curated list: deepseek-v4-pro / -v4-flash,
/// qwen3-coder-next and the z-ai GLM models (glm-5.3 included).
fn openrouter_model_supports_vision(name: &str) -> bool {
    const IMAGE_INPUT_PREFIXES: &[&str] = &[
        "anthropic/claude-",
        "google/gemini-",
        "openai/gpt-6-",
        "openai/gpt-5.6-",
    ];
    const IMAGE_INPUT_SLUGS: &[&str] = &[
        "moonshotai/kimi-k3",
        "moonshotai/kimi-k2.6",
        "moonshotai/kimi-k2.7-code",
        "minimax/minimax-m3",
        "deepseek/deepseek-v4.1-flash",
    ];
    let normalized = name.to_ascii_lowercase();
    openrouter_model_is_png_jpeg_only(&normalized)
        || IMAGE_INPUT_PREFIXES
            .iter()
            .any(|prefix| normalized.starts_with(prefix))
        || IMAGE_INPUT_SLUGS.contains(&normalized.as_str())
}

fn openrouter_model_info(name: &str) -> ModelInfo {
    let info = ModelInfo::new(name, ModelConfig::new_or_fail(name).context_limit());
    let normalized = name.to_ascii_lowercase();
    if openrouter_model_is_png_jpeg_only(&normalized) {
        info.with_png_jpeg_image_inputs()
    } else if openrouter_model_supports_vision(name) {
        info.with_vision()
    } else {
        info
    }
}

#[derive(serde::Serialize)]
pub struct OpenRouterProvider {
    #[serde(skip)]
    api_client: ApiClient,
    model: ModelConfig,
    supports_streaming: bool,
    #[serde(skip)]
    name: String,
}

impl OpenRouterProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let model = model.with_fast(OPENROUTER_DEFAULT_FAST_MODEL.to_string());

        let config = crate::config::Config::global();
        let api_key: String = config.get_secret("OPENROUTER_API_KEY")?;
        let host: String = config
            .get_param("OPENROUTER_HOST")
            .unwrap_or_else(|_| "https://openrouter.ai".to_string());

        let auth = AuthMethod::BearerToken(api_key);
        let api_client = ApiClient::new(host, auth)?
            .with_header("HTTP-Referer", "https://BaranziniLab.github.io/biorouter")?
            .with_header("X-Title", "biorouter")?;

        Ok(Self {
            api_client,
            model,
            supports_streaming: true,
            name: Self::metadata().name,
        })
    }

    async fn post(&self, payload: &Value) -> Result<Value, ProviderError> {
        let response = self
            .api_client
            .response_post("api/v1/chat/completions", payload)
            .await?;

        let response_body = handle_response_openai_compat(response)
            .await
            .map_err(|e| ProviderError::RequestFailed(format!("Failed to parse response: {e}")))?;

        if let Some(error_obj) = response_body.get("error") {
            let error_message = error_obj
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown OpenRouter error");

            let error_code = error_obj.get("code").and_then(|c| c.as_u64()).unwrap_or(0);

            if error_code == 400 && error_message.contains("maximum context length") {
                return Err(ProviderError::ContextLengthExceeded(
                    error_message.to_string(),
                ));
            }

            match error_code {
                401 | 403 => return Err(ProviderError::Authentication(error_message.to_string())),
                429 => {
                    return Err(ProviderError::RateLimitExceeded {
                        details: error_message.to_string(),
                        retry_delay: None,
                    })
                }
                500 | 503 => return Err(ProviderError::ServerError(error_message.to_string())),
                _ => return Err(ProviderError::RequestFailed(error_message.to_string())),
            }
        }

        Ok(response_body)
    }
}

/// Update the request when using anthropic model.
/// For anthropic model, we can enable prompt caching to save cost. Since openrouter is the OpenAI compatible
/// endpoint, we need to modify the open ai request to have anthropic cache control field.
fn update_request_for_anthropic(original_payload: &Value) -> Value {
    let mut payload = original_payload.clone();

    if let Some(messages_spec) = payload
        .as_object_mut()
        .and_then(|obj| obj.get_mut("messages"))
        .and_then(|messages| messages.as_array_mut())
    {
        // Add "cache_control" to the last and second-to-last "user" messages.
        // During each turn, we mark the final message with cache_control so the conversation can be
        // incrementally cached. The second-to-last user message is also marked for caching with the
        // cache_control parameter, so that this checkpoint can read from the previous cache.
        let mut user_count = 0;
        for message in messages_spec.iter_mut().rev() {
            if message.get("role") == Some(&json!("user")) {
                if let Some(content) = message.get_mut("content") {
                    if let Some(content_str) = content.as_str() {
                        *content = json!([{
                            "type": "text",
                            "text": content_str,
                            "cache_control": { "type": "ephemeral" }
                        }]);
                    }
                }
                user_count += 1;
                if user_count >= 2 {
                    break;
                }
            }
        }

        // Update the system message to have cache_control field.
        if let Some(system_message) = messages_spec
            .iter_mut()
            .find(|msg| msg.get("role") == Some(&json!("system")))
        {
            if let Some(content) = system_message.get_mut("content") {
                if let Some(content_str) = content.as_str() {
                    *system_message = json!({
                        "role": "system",
                        "content": [{
                            "type": "text",
                            "text": content_str,
                            "cache_control": { "type": "ephemeral" }
                        }]
                    });
                }
            }
        }
    }

    if let Some(tools_spec) = payload
        .as_object_mut()
        .and_then(|obj| obj.get_mut("tools"))
        .and_then(|tools| tools.as_array_mut())
    {
        // Add "cache_control" to the last tool spec, if any. This means that all tool definitions,
        // will be cached as a single prefix.
        if let Some(last_tool) = tools_spec.last_mut() {
            if let Some(function) = last_tool.get_mut("function") {
                function
                    .as_object_mut()
                    .unwrap()
                    .insert("cache_control".to_string(), json!({ "type": "ephemeral" }));
            }
        }
    }
    payload
}

fn is_gemini_model(model_name: &str) -> bool {
    model_name.starts_with("google/")
}

async fn create_request_based_on_model(
    provider: &OpenRouterProvider,
    system: &str,
    messages: &[Message],
    tools: &[Tool],
) -> Result<Value> {
    let mut payload = create_request(
        &provider.model,
        system,
        messages,
        tools,
        &super::utils::ImageFormat::OpenAi,
        false,
    )?;

    if provider.supports_cache_control().await {
        payload = update_request_for_anthropic(&payload);
    }

    if is_gemini_model(&provider.model.model_name) {
        openrouter_format::add_reasoning_details_to_request(&mut payload, messages);
    }

    if let Some(obj) = payload.as_object_mut() {
        obj.insert("transforms".to_string(), json!(["middle-out"]));
    }

    Ok(payload)
}

#[async_trait]
impl Provider for OpenRouterProvider {
    fn metadata() -> ProviderMetadata {
        let models: Vec<ModelInfo> = OPENROUTER_KNOWN_MODELS
            .iter()
            .map(|&name| openrouter_model_info(name))
            .collect();

        ProviderMetadata::with_models(
            "openrouter",
            "OpenRouter",
            "Router for many model providers",
            OPENROUTER_DEFAULT_MODEL,
            models,
            OPENROUTER_DOC_URL,
            vec![
                ConfigKey::new("OPENROUTER_API_KEY", true, true, None),
                ConfigKey::new(
                    "OPENROUTER_HOST",
                    false,
                    false,
                    Some("https://openrouter.ai"),
                ),
            ],
        )
        .with_unlisted_models()
    }

    fn get_name(&self) -> &str {
        &self.name
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
        let payload = create_request_based_on_model(self, system, messages, tools).await?;
        let mut log = RequestLog::start(model_config, &payload)?;

        let response = self
            .with_retry(|| async {
                let payload_clone = payload.clone();
                self.post(&payload_clone).await
            })
            .await?;

        let response_model = get_model(&response);
        let message = if is_gemini_model(&self.model.model_name) {
            openrouter_format::response_to_message(&response)?
        } else {
            crate::providers::formats::openai::response_to_message(&response)?
        };

        let usage = response.get("usage").map(get_usage).unwrap_or_else(|| {
            tracing::debug!("Failed to get usage data");
            Usage::default()
        });
        log.write(&response, Some(&usage))?;
        Ok((message, ProviderUsage::new(response_model, usage)))
    }

    /// Fetch supported models from OpenRouter API (only models with tool support)
    async fn fetch_supported_models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        // Handle request failures gracefully
        // If the request fails, fall back to manual entry
        let response = match self.api_client.response_get("api/v1/models").await {
            Ok(response) => response,
            Err(e) => {
                tracing::warn!("Failed to fetch models from OpenRouter API: {}, falling back to manual model entry", e);
                return Ok(None);
            }
        };

        // Handle JSON parsing failures gracefully
        let json: serde_json::Value = match response.json().await {
            Ok(json) => json,
            Err(e) => {
                tracing::warn!("Failed to parse OpenRouter API response as JSON: {}, falling back to manual model entry", e);
                return Ok(None);
            }
        };

        // Check for error in response
        if let Some(err_obj) = json.get("error") {
            let msg = err_obj
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            tracing::warn!("OpenRouter API returned an error: {}", msg);
            return Ok(None);
        }

        let data = json.get("data").and_then(|v| v.as_array()).ok_or_else(|| {
            ProviderError::UsageError("Missing data field in JSON response".into())
        })?;

        let mut models: Vec<String> = data
            .iter()
            .filter_map(|model| {
                // Get the model ID
                let id = model.get("id").and_then(|v| v.as_str())?;

                // Check if the model supports tools
                let supported_params =
                    match model.get("supported_parameters").and_then(|v| v.as_array()) {
                        Some(params) => params,
                        None => {
                            // If supported_parameters is missing, skip this model (assume no tool support)
                            tracing::debug!(
                                "Model '{}' missing supported_parameters field, skipping",
                                id
                            );
                            return None;
                        }
                    };

                let has_tool_support = supported_params
                    .iter()
                    .any(|param| param.as_str() == Some("tools"));

                if has_tool_support {
                    Some(id.to_string())
                } else {
                    None
                }
            })
            .collect();

        // If no models with tool support were found, fall back to manual entry
        if models.is_empty() {
            tracing::warn!("No models with tool support found in OpenRouter API response, falling back to manual model entry");
            return Ok(None);
        }

        models.sort();
        Ok(Some(models))
    }

    async fn supports_cache_control(&self) -> bool {
        self.model
            .model_name
            .starts_with(OPENROUTER_MODEL_PREFIX_ANTHROPIC)
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
        let mut payload = create_request(
            &self.model,
            system,
            messages,
            tools,
            &super::utils::ImageFormat::OpenAi,
            true,
        )?;

        if self.supports_cache_control().await {
            payload = update_request_for_anthropic(&payload);
        }

        if is_gemini_model(&self.model.model_name) {
            openrouter_format::add_reasoning_details_to_request(&mut payload, messages);
        }

        if let Some(obj) = payload.as_object_mut() {
            obj.insert("transforms".to_string(), json!(["middle-out"]));
        }

        let mut log = RequestLog::start(&self.model, &payload)?;

        let response = self
            .with_retry(|| async {
                let resp = self
                    .api_client
                    .response_post("api/v1/chat/completions", &payload)
                    .await?;
                handle_status_openai_compat(resp).await
            })
            .await
            .inspect_err(|e| {
                let _ = log.error(e);
            })?;

        stream_openai_compat(response, log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(name: &str) -> ModelInfo {
        OpenRouterProvider::metadata()
            .known_models
            .into_iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("{name} should be advertised"))
    }

    #[test]
    fn default_is_claude_sonnet_5_and_advertised() {
        assert_eq!(OPENROUTER_DEFAULT_MODEL, "anthropic/claude-sonnet-5");
        assert!(OPENROUTER_KNOWN_MODELS.contains(&OPENROUTER_DEFAULT_MODEL));
        assert!(OPENROUTER_KNOWN_MODELS.contains(&OPENROUTER_DEFAULT_FAST_MODEL));
        assert_eq!(
            OpenRouterProvider::metadata().default_model,
            OPENROUTER_DEFAULT_MODEL
        );
    }

    #[test]
    fn image_input_follows_openrouters_input_modalities() {
        for slug in [
            "anthropic/claude-opus-5.5",
            "anthropic/claude-fable-5.1",
            "openai/gpt-6-astra",
            "openai/gpt-6-sol",
            "openai/gpt-6-luna",
            "openai/gpt-5.6-terra",
            "google/gemini-3.8-flash",
            "moonshotai/kimi-k3",
            "moonshotai/kimi-k2.6",
            "moonshotai/kimi-k2.7-code",
            "minimax/minimax-m3",
            "deepseek/deepseek-v4.1-flash",
        ] {
            assert_eq!(known(slug).supports_vision, Some(true), "{slug}");
        }
        for slug in [
            "z-ai/glm-5.3",
            "z-ai/glm-5.2",
            "deepseek/deepseek-v4-pro",
            "deepseek/deepseek-v4-flash",
            "qwen/qwen3-coder-next",
        ] {
            assert_eq!(known(slug).supports_vision, None, "{slug} is text-only");
        }
    }

    #[test]
    fn xai_slugs_are_limited_to_png_and_jpeg() {
        let png_jpeg = Some(vec![
            "image/png".to_string(),
            "image/jpeg".to_string(),
            "image/jpg".to_string(),
        ]);
        for slug in ["x-ai/grok-4.7", "x-ai/grok-4.3", "x-ai/grok-build-0.1"] {
            let info = known(slug);
            assert_eq!(info.supports_vision, Some(true), "{slug}");
            assert_eq!(info.supported_input_mime_types, png_jpeg, "{slug}");
        }
    }
}
