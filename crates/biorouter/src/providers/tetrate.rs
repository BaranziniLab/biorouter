use super::api_client::{ApiClient, AuthMethod};
use super::base::{
    ConfigKey, MessageStream, ModelInfo, Provider, ProviderMetadata, ProviderUsage, Usage,
};
use super::errors::ProviderError;
use super::retry::ProviderRetry;
use super::utils::{
    get_model, handle_response_google_compat, handle_response_openai_compat,
    handle_status_openai_compat, is_google_model, stream_openai_compat, RequestLog,
};
use crate::config::signup_tetrate::TETRATE_DEFAULT_MODEL;
use crate::conversation::message::Message;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::model::ModelConfig;
use crate::providers::formats::openai::{create_request, get_usage, response_to_message};
use rmcp::model::Tool;

// Tetrate Agent Router Service can run many models. Its full catalog is
// public (GET https://router.tetrate.ai/api/public/models?limit=1000 needs no
// key; it also lives at github.com/tetrateio/agent-router-models), but the
// provider's authenticated /v1/models fetch is what decides what a given key
// can route, so this static list is only a fallback. Verified against the
// public catalog on Sep 25, 2026.
//
// Removed models retired upstream: claude-opus-4-1 (Anthropic retired it
// Aug 5, 2026), gpt-5 / gpt-5-mini / gpt-5-nano (OpenAI deprecated them on
// Jun 11, 2026 with removal on Dec 11, 2026; Tetrate marks them deprecated),
// claude-3-7-sonnet-latest (retired Feb 19, 2026), claude-sonnet-4-20250514
// (retired Jun 15, 2026), gemini-2.0-flash(-lite) (shut down Jun 1, 2026).
//
// GPT-6 and GPT-5.6 are deliberately NOT listed, although Tetrate routes them.
// This provider always posts to v1/chat/completions, and Tetrate catalogs
// them as `mode: responses`. OpenAI's GPT-6 Sol/Luna pages say Chat
// Completions supports function calling only with reasoning_effort=none
// (Astra does not accept none), and BioRouter's own OpenAI provider sends
// GPT-5.6 through the Responses API for the same reason. Nothing Tetrate
// publishes says its Chat Completions path translates those calls to
// Responses, so a tool call is not known to work. gpt-4.1 is `mode: responses`
// too, but it is a non-reasoning model, so the restriction does not reach it.
//
// Gemini 3 (gemini-3.8-flash, gemini-3.1-pro-preview) is deliberately NOT
// listed either, although Tetrate routes it. Tetrate's public catalog backs
// those ids with Google's OpenAI-compatible endpoint
// (generativelanguage.googleapis.com/v1beta/openai/), where Gemini 3 function
// calling requires the model's thought signatures to be sent back on every
// later turn, carried in each tool call's `extra_content`. BioRouter's OpenAI
// format (formats/openai.rs) neither keeps nor replays `extra_content`, so the
// turn after a tool call would be rejected. List them again once that format
// round-trips the signature.
//
// gemini-2.5-pro / -flash are not deprecated on the Gemini API (Google, Sep 18,
// 2026) and Tetrate still routes them.
pub const TETRATE_KNOWN_MODELS: &[&str] = &[
    "claude-opus-5-5",
    "claude-fable-5-1",
    "claude-sonnet-5",
    "claude-sonnet-4-6",
    "claude-haiku-4-5",
    "gemini-2.5-pro",
    "gemini-2.5-flash",
    "gpt-4.1",
];
pub const TETRATE_DOC_URL: &str = "https://router.tetrate.ai";

fn tetrate_model_supports_vision(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    normalized.starts_with("claude-")
        || normalized.starts_with("gemini-")
        || normalized.starts_with("gpt-4.1")
}

#[derive(serde::Serialize)]
pub struct TetrateProvider {
    #[serde(skip)]
    api_client: ApiClient,
    model: ModelConfig,
    supports_streaming: bool,
    #[serde(skip)]
    name: String,
}

impl TetrateProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let config = crate::config::Config::global();
        let api_key: String = config.get_secret("TETRATE_API_KEY")?;
        // API host for LLM endpoints (/v1/chat/completions, /v1/models)
        let host: String = config
            .get_param("TETRATE_HOST")
            .unwrap_or_else(|_| "https://api.router.tetrate.ai".to_string());

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
            .response_post("v1/chat/completions", payload)
            .await?;

        // Handle Google-compatible model responses differently
        if is_google_model(payload) {
            return handle_response_google_compat(response).await;
        }

        // For OpenAI-compatible models, parse the response body to JSON
        let response_body = handle_response_openai_compat(response)
            .await
            .map_err(|e| ProviderError::RequestFailed(format!("Failed to parse response: {e}")))?;

        let _debug = format!(
            "Tetrate Agent Router Service request with payload: {} and response: {}",
            serde_json::to_string_pretty(payload).unwrap_or_else(|_| "Invalid JSON".to_string()),
            serde_json::to_string_pretty(&response_body)
                .unwrap_or_else(|_| "Invalid JSON".to_string())
        );

        // Tetrate Agent Router Service can return errors in 200 OK responses, so we have to check for errors explicitly
        if let Some(error_obj) = response_body.get("error") {
            // If there's an error object, extract the error message and code
            let error_message = error_obj
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown Tetrate Agent Router Service error");

            let error_code = error_obj.get("code").and_then(|c| c.as_u64()).unwrap_or(0);

            // Check for context length errors in the error message
            if error_code == 400 && error_message.contains("maximum context length") {
                return Err(ProviderError::ContextLengthExceeded(
                    error_message.to_string(),
                ));
            }

            // Return appropriate error based on the error code
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

        // No error detected, return the response body
        Ok(response_body)
    }
}

#[async_trait]
impl Provider for TetrateProvider {
    fn metadata() -> ProviderMetadata {
        let models: Vec<ModelInfo> = TETRATE_KNOWN_MODELS
            .iter()
            .map(|&name| {
                let info = ModelInfo::new(name, ModelConfig::new_or_fail(name).context_limit());
                if tetrate_model_supports_vision(name) {
                    info.with_vision()
                } else {
                    info
                }
            })
            .collect();

        ProviderMetadata::with_models(
            "tetrate",
            "Tetrate Agent Router Service",
            "Enterprise router for AI models",
            TETRATE_DEFAULT_MODEL,
            models,
            TETRATE_DOC_URL,
            vec![
                ConfigKey::new("TETRATE_API_KEY", true, true, None),
                ConfigKey::new(
                    "TETRATE_HOST",
                    false,
                    false,
                    Some("https://api.router.tetrate.ai"),
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
        let payload = create_request(
            model_config,
            system,
            messages,
            tools,
            &super::utils::ImageFormat::OpenAi,
            false,
        )?;
        let mut log = RequestLog::start(model_config, &payload)?;

        // Make request
        let response = self
            .with_retry(|| async {
                let payload_clone = payload.clone();
                self.post(&payload_clone).await
            })
            .await?;

        // Parse response
        let message = response_to_message(&response)?;
        let usage = response.get("usage").map(get_usage).unwrap_or_else(|| {
            tracing::debug!("Failed to get usage data");
            Usage::default()
        });
        let model = get_model(&response);
        log.write(&response, Some(&usage))?;
        Ok((message, ProviderUsage::new(model, usage)))
    }

    async fn stream(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let payload = create_request(
            &self.model,
            system,
            messages,
            tools,
            &super::utils::ImageFormat::OpenAi,
            true,
        )?;

        let mut log = RequestLog::start(&self.model, &payload)?;

        let response = self
            .with_retry(|| async {
                let resp = self
                    .api_client
                    .response_post("v1/chat/completions", &payload)
                    .await?;
                handle_status_openai_compat(resp).await
            })
            .await
            .inspect_err(|e| {
                let _ = log.error(e);
            })?;

        stream_openai_compat(response, log)
    }

    /// Fetch supported models from Tetrate Agent Router Service API (only models with tool support)
    async fn fetch_supported_models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        // Use the existing api_client which already has authentication configured
        let response = match self.api_client.response_get("v1/models").await {
            Ok(response) => response,
            Err(e) => {
                tracing::warn!("Failed to fetch models from Tetrate Agent Router Service API: {}, falling back to manual model entry", e);
                return Ok(None);
            }
        };

        // Handle JSON parsing failures gracefully
        let json: serde_json::Value = match response.json().await {
            Ok(json) => json,
            Err(e) => {
                tracing::warn!("Failed to parse Tetrate Agent Router Service API response as JSON: {}, falling back to manual model entry", e);
                return Ok(None);
            }
        };

        // Check for error in response
        if let Some(err_obj) = json.get("error") {
            let msg = err_obj
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            tracing::warn!(
                "Tetrate Agent Router Service API returned an error: {}",
                msg
            );
            return Ok(None);
        }

        // The response format from /v1/models is expected to be OpenAI-compatible
        // It should have a "data" field with an array of model objects
        let data = json.get("data").and_then(|v| v.as_array()).ok_or_else(|| {
            ProviderError::UsageError("Missing data field in JSON response".into())
        })?;

        let mut models: Vec<String> = data
            .iter()
            .filter_map(|model| {
                // Get the model ID
                let id = model.get("id").and_then(|v| v.as_str())?;

                // Check if the model supports computer_use (which indicates tool/function support)
                // The Tetrate API uses "supports_computer_use" instead of "supported_parameters"
                let supports_computer_use = model
                    .get("supports_computer_use")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                if supports_computer_use {
                    Some(id.to_string())
                } else {
                    tracing::debug!(
                        "Model '{}' does not support computer_use (tool support), skipping",
                        id
                    );
                    None
                }
            })
            .collect();

        // If no models with tool support were found, fall back to manual entry
        if models.is_empty() {
            tracing::warn!("No models with tool support found in Tetrate Agent Router Service API response, falling back to manual model entry");
            return Ok(None);
        }

        models.sort();
        Ok(Some(models))
    }

    fn supports_streaming(&self) -> bool {
        self.supports_streaming
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_advertised() {
        assert!(TETRATE_KNOWN_MODELS.contains(&TETRATE_DEFAULT_MODEL));
        assert_eq!(
            TetrateProvider::metadata().default_model,
            TETRATE_DEFAULT_MODEL
        );
    }

    #[test]
    fn current_claude_and_gemini_models_are_advertised_with_vision() {
        let metadata = TetrateProvider::metadata();
        for id in [
            "claude-opus-5-5",
            "claude-fable-5-1",
            "claude-sonnet-5",
            "gemini-2.5-pro",
            "gemini-2.5-flash",
        ] {
            let info = metadata
                .known_models
                .iter()
                .find(|m| m.name == id)
                .unwrap_or_else(|| panic!("{id} should be advertised"));
            assert_eq!(info.supports_vision, Some(true), "{id} accepts images");
        }
    }

    /// Retired or deprecated upstream, or (GPT-6 / GPT-5.6) not known to
    /// support tool calls on the Chat Completions path this provider uses.
    #[test]
    fn retired_and_responses_only_models_are_not_advertised() {
        for id in TETRATE_KNOWN_MODELS {
            assert!(
                !id.starts_with("gpt-5") && !id.starts_with("gpt-6"),
                "{id} should not be advertised on Tetrate's chat/completions path"
            );
        }
        assert!(!TETRATE_KNOWN_MODELS.contains(&"claude-opus-4-1"));
    }

    /// Gemini 3 on Tetrate needs its thought signatures replayed through the
    /// tool call's `extra_content`, which the OpenAI format does not carry, so
    /// a multi-turn tool call would fail. It stays off the list until it does.
    #[test]
    fn gemini_3_is_not_advertised_without_thought_signature_replay() {
        for id in TETRATE_KNOWN_MODELS {
            assert!(
                !id.starts_with("gemini-3"),
                "{id} needs thought-signature replay the OpenAI format lacks"
            );
        }
    }
}
