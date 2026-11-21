use super::api_client::{ApiClient, AuthMethod};
use super::base::MessageStream;
use super::errors::ProviderError;
use super::retry::ProviderRetry;
use super::utils::{
    handle_response_google_compat, handle_status_openai_compat, unescape_json_values, RequestLog,
};
use crate::conversation::message::Message;

use crate::model::ModelConfig;
use crate::providers::base::{ConfigKey, ModelInfo, Provider, ProviderMetadata, ProviderUsage};
use crate::providers::formats::google::{
    create_request, get_usage, response_to_message_for_model,
    response_to_streaming_message_for_model, uses_gemini4_response_history,
};
use anyhow::Result;
use async_stream::try_stream;
use async_trait::async_trait;
use futures::TryStreamExt;
use rmcp::model::Tool;
use serde_json::Value;
use std::io;
use tokio::pin;
use tokio_stream::StreamExt;
use tokio_util::codec::{FramedRead, LinesCodec};
use tokio_util::io::StreamReader;

pub const GOOGLE_API_HOST: &str = "https://generativelanguage.googleapis.com";
// Google's current starting recommendation balances agentic capability and
// latency: https://aistudio.google.com/docs/latest-model (November 2026).
pub const GOOGLE_DEFAULT_MODEL: &str = "gemini-4-hydrogen";
// Flash-Lite remains Google's budget option for high-volume routing.
pub const GOOGLE_DEFAULT_FAST_MODEL: &str = "gemini-3.5-flash-lite";
// Text replies and image input only; Live, TTS, video and embedding models
// need different APIs. https://aistudio.google.com/docs/models
pub const GOOGLE_KNOWN_MODELS: &[&str] = &[
    "gemini-4-hydrogen",
    "gemini-4-argon",
    "gemini-4-carbon",
    // Gemini 4 Neon: offered, not the default. $3/$15 per MTok, implicit
    // caching only (https://aistudio.google.com/docs/pricing, 2026-11-19).
    "gemini-4-neon",
    // Gemini 3.x models (all GA except the two -preview ids)
    "gemini-3.8-flash",
    "gemini-3.7-flash",
    "gemini-3.6-flash",
    "gemini-3.5-flash",
    "gemini-3.5-flash-lite",
    "gemini-3.1-pro-preview",
    "gemini-3.1-flash-lite",
    "gemini-3-flash-preview",
    // Gemini 3.x image models
    "gemini-3.1-flash-image",
    "gemini-3-pro-image",
    // Existing-user API access only. Pro and Flash route to 3.8 Flash and
    // 3.5 Flash-Lite on December 1, 2026; Flash-Lite has no retirement date.
    // https://aistudio.google.com/docs/deprecations
    "gemini-2.5-pro",
    "gemini-2.5-flash",
    "gemini-2.5-flash-lite",
];

pub const GOOGLE_DOC_URL: &str = "https://aistudio.google.com/docs/models";

#[derive(Debug, serde::Serialize)]
pub struct GoogleProvider {
    #[serde(skip)]
    api_client: ApiClient,
    model: ModelConfig,
    #[serde(skip)]
    name: String,
}

impl GoogleProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let model = model.with_fast(GOOGLE_DEFAULT_FAST_MODEL.to_string());

        let config = crate::config::Config::global();
        let api_key: String = config.get_secret("GOOGLE_API_KEY")?;
        let host: String = config
            .get_param("GOOGLE_HOST")
            .unwrap_or_else(|_| GOOGLE_API_HOST.to_string());

        let auth = AuthMethod::ApiKey {
            header_name: "x-goog-api-key".to_string(),
            key: api_key,
        };

        let api_client =
            ApiClient::new(host, auth)?.with_header("Content-Type", "application/json")?;

        Ok(Self {
            api_client,
            model,
            name: Self::metadata().name,
        })
    }

    async fn post(&self, model_name: &str, payload: &Value) -> Result<Value, ProviderError> {
        let path = format!("v1beta/models/{}:generateContent", model_name);
        let response = self.api_client.response_post(&path, payload).await?;
        handle_response_google_compat(response).await
    }

    async fn post_stream(
        &self,
        model_name: &str,
        payload: &Value,
    ) -> Result<reqwest::Response, ProviderError> {
        let path = format!("v1beta/models/{}:streamGenerateContent?alt=sse", model_name);
        let response = self.api_client.response_post(&path, payload).await?;
        handle_status_openai_compat(response).await
    }
}

#[async_trait]
impl Provider for GoogleProvider {
    fn metadata() -> ProviderMetadata {
        // Every chat-capable Gemini model (2.5, 3.x and 4) accepts images. The
        // audio-output models (TTS, Live) do not, which is why none of them is
        // in GOOGLE_KNOWN_MODELS; the test module below holds that line.
        let models: Vec<ModelInfo> = GOOGLE_KNOWN_MODELS
            .iter()
            .map(|&name| {
                ModelInfo::new(name, ModelConfig::new_or_fail(name).context_limit()).with_vision()
            })
            .collect();

        ProviderMetadata::with_models(
            "google",
            "Google Gemini",
            "Gemini models from Google AI",
            GOOGLE_DEFAULT_MODEL,
            models,
            GOOGLE_DOC_URL,
            vec![
                ConfigKey::new("GOOGLE_API_KEY", true, true, None),
                ConfigKey::new("GOOGLE_HOST", false, false, Some(GOOGLE_API_HOST)),
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
        let payload = create_request(model_config, system, messages, tools)?;
        let mut log = RequestLog::start(model_config, &payload)?;

        let response = self
            .with_retry(|| async { self.post(&model_config.model_name, &payload).await })
            .await?;

        // Native signatures cover literal JSON values, including backslashes.
        let replay_response = if uses_gemini4_response_history(&response, &model_config.model_name)
        {
            response.clone()
        } else {
            unescape_json_values(&response)
        };
        let message = response_to_message_for_model(replay_response, &model_config.model_name)?;
        let usage = get_usage(&response)?;
        let response_model = match response.get("modelVersion") {
            Some(model_version) => model_version.as_str().unwrap_or_default().to_string(),
            None => model_config.model_name.clone(),
        };
        log.write(&response, Some(&usage))?;
        let provider_usage = ProviderUsage::new(response_model, usage);
        Ok((message, provider_usage))
    }

    async fn fetch_supported_models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        let response = self.api_client.response_get("v1beta/models").await?;
        let json: serde_json::Value = response.json().await?;
        let arr = match json.get("models").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => return Ok(None),
        };
        let mut models: Vec<String> = arr
            .iter()
            .filter_map(|m| m.get("name").and_then(|v| v.as_str()))
            .map(|name| name.split('/').next_back().unwrap_or(name).to_string())
            .collect();
        models.sort();
        Ok(Some(models))
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    async fn stream(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let payload = create_request(&self.model, system, messages, tools)?;
        let mut log = RequestLog::start(&self.model, &payload)?;

        let response = self
            .with_retry(|| async { self.post_stream(&self.model.model_name, &payload).await })
            .await
            .inspect_err(|e| {
                let _ = log.error(e);
            })?;

        let stream = response.bytes_stream().map_err(io::Error::other);
        let model_name = self.model.model_name.clone();

        Ok(Box::pin(try_stream! {
            let stream_reader = StreamReader::new(stream);
            let framed = FramedRead::new(stream_reader, LinesCodec::new())
                .map_err(anyhow::Error::from);

            let message_stream = response_to_streaming_message_for_model(framed, &model_name);
            pin!(message_stream);
            while let Some(message) = message_stream.next().await {
                let (message, usage, pending) = message.map_err(|e|
                    ProviderError::RequestFailed(format!("Stream decode error: {}", e))
                )?;
                if message.is_some() || usage.is_some() {
                    log.write(&message, usage.as_ref().map(|f| f.usage).as_ref())?;
                }
                yield (message, usage, pending);
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_advertised_models() {
        assert!(GOOGLE_KNOWN_MODELS.contains(&GOOGLE_DEFAULT_MODEL));
        assert!(GOOGLE_KNOWN_MODELS.contains(&GOOGLE_DEFAULT_FAST_MODEL));
        assert_eq!(
            GoogleProvider::metadata().default_model,
            GOOGLE_DEFAULT_MODEL
        );
    }

    #[test]
    fn sep_2026_flash_lineup_is_advertised_with_vision() {
        // metadata() sizes each model through ModelConfig::context_limit(),
        // which honours BIOROUTER_CONTEXT_LIMIT; other tests in this binary set
        // it process-wide under env_lock, so a window assertion must hold it.
        let _guard = env_lock::lock_env([
            ("BIOROUTER_CONTEXT_LIMIT", None::<&str>),
            ("BIOROUTER_PREDEFINED_MODELS", None::<&str>),
        ]);
        let metadata = GoogleProvider::metadata();
        for id in [
            "gemini-3.8-flash",
            "gemini-3.7-flash",
            "gemini-3.6-flash",
            "gemini-3.5-flash-lite",
        ] {
            let info = metadata
                .known_models
                .iter()
                .find(|m| m.name == id)
                .unwrap_or_else(|| panic!("{id} should be advertised"));
            assert_eq!(info.supports_vision, Some(true), "{id} accepts images");
            assert_eq!(info.context_limit, 1_048_576, "{id} has a 1M window");
        }
    }

    /// `metadata()` marks every entry vision-capable, so a model that cannot
    /// take an image, or cannot answer a chat at all, must never be listed.
    /// The 2.5 TTS previews produce audio, and the older image generator is
    /// not offered for agent chat.
    #[test]
    fn no_audio_output_or_retired_model_is_advertised() {
        for id in GOOGLE_KNOWN_MODELS {
            assert!(
                !id.contains("-tts") && !id.contains("-live") && !id.contains("native-audio"),
                "{id} produces audio, not a chat reply"
            );
        }
        for removed in [
            "gemini-2.5-flash-image",
            "gemini-2.5-flash-preview-tts",
            "gemini-2.5-pro-preview-tts",
        ] {
            assert!(
                !GOOGLE_KNOWN_MODELS.contains(&removed),
                "{removed} should no longer be advertised"
            );
        }
    }
}
