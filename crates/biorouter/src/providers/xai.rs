use super::api_client::{ApiClient, AuthMethod};
use super::errors::ProviderError;
use super::retry::ProviderRetry;
use super::utils::{
    get_model, handle_response_openai_compat, handle_status_openai_compat, stream_openai_compat,
    RequestLog,
};
use crate::conversation::message::Message;
use crate::model::ModelConfig;
use crate::providers::base::{
    ConfigKey, MessageStream, ModelInfo, Provider, ProviderMetadata, ProviderUsage, Usage,
};
use crate::providers::formats::openai::{create_request, get_usage, response_to_message};
use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::Tool;
use serde_json::Value;
pub const XAI_API_HOST: &str = "https://api.x.ai/v1";
// Verified against docs.x.ai (Sep 25, 2026; docs last updated Sep 21). xAI
// retired the entire grok-2 / grok-3 / grok-4-0709 / grok-4-fast /
// grok-code-fast-1 lineup on (or before) May 15, 2026; retired slugs
// auto-redirect (grok-code-fast-1 is now an alias of grok-build-0.1).
//
// Default: docs.x.ai says "for everything else, including code, use Grok 4.7.
// It is the most capable model we've built". It costs more than grok-4.3
// ($2/$6 vs $1.25/$2.50 per MTok) and has half the window (500K vs 1M), so
// grok-4.3 stays listed as the cheap 1M option.
//
// Removed Sep 2026:
// - grok-4.20-multi-agent-0309: xAI documents that the multi-agent model does
//   not work with the Chat Completions API, the only API this provider calls.
// - grok-imagine-image-quality: image generation only, and xAI retires it on
//   Nov 2, 2026.
pub const XAI_DEFAULT_MODEL: &str = "grok-4.7";
pub const XAI_KNOWN_MODELS: &[&str] = &[
    // Flagship family (500K context; text + image in, function calling)
    "grok-4.7",
    "grok-4.6",
    "grok-4.5",
    // 1M context
    "grok-4.3",
    "grok-4.3-latest",
    "grok-latest",
    // grok-4.20 family (1M context)
    "grok-4.20-0309-reasoning",
    "grok-4.20-0309-non-reasoning",
    // Fast agentic coding (256K context; successor to grok-code-fast-1)
    "grok-build-0.1",
    // Image generation
    "grok-imagine-image",
];

pub const XAI_DOC_URL: &str = "https://docs.x.ai/docs/overview";

/// xAI's chat models take png/jpeg images alongside text. grok-build-0.1 and
/// Grok 4.5-4.7 are listed as "text, image -> text" on docs.x.ai (Sep 2026);
/// only the image-generation model is left out.
fn xai_model_supports_vision(name: &str) -> bool {
    const IMAGE_INPUT_PREFIXES: &[&str] = &[
        "grok-4.3",
        "grok-4.20",
        "grok-4.5",
        "grok-4.6",
        "grok-4.7",
        "grok-build-",
    ];
    let normalized = name.to_ascii_lowercase();
    normalized == "grok-latest"
        || IMAGE_INPUT_PREFIXES
            .iter()
            .any(|prefix| normalized.starts_with(prefix))
}

#[derive(serde::Serialize)]
pub struct XaiProvider {
    #[serde(skip)]
    api_client: ApiClient,
    model: ModelConfig,
    supports_streaming: bool,
    #[serde(skip)]
    name: String,
}

impl XaiProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let config = crate::config::Config::global();
        let api_key: String = config.get_secret("XAI_API_KEY")?;
        let host: String = config
            .get_param("XAI_HOST")
            .unwrap_or_else(|_| XAI_API_HOST.to_string());

        let auth = AuthMethod::BearerToken(api_key);
        let api_client = ApiClient::new(host, auth)?;

        Ok(Self {
            api_client,
            model,
            supports_streaming: true,
            name: Self::metadata().name,
        })
    }

    async fn post(&self, payload: Value) -> Result<Value, ProviderError> {
        let response = self
            .api_client
            .response_post("chat/completions", &payload)
            .await?;

        handle_response_openai_compat(response).await
    }
}

#[async_trait]
impl Provider for XaiProvider {
    fn metadata() -> ProviderMetadata {
        let models: Vec<ModelInfo> = XAI_KNOWN_MODELS
            .iter()
            .map(|&name| {
                let info = ModelInfo::new(name, ModelConfig::new_or_fail(name).context_limit());
                if xai_model_supports_vision(name) {
                    info.with_png_jpeg_image_inputs()
                } else {
                    info
                }
            })
            .collect();

        ProviderMetadata::with_models(
            "xai",
            "xAI",
            "Grok models from xAI, including reasoning and multimodal capabilities",
            XAI_DEFAULT_MODEL,
            models,
            XAI_DOC_URL,
            vec![
                ConfigKey::new("XAI_API_KEY", true, true, None),
                ConfigKey::new("XAI_HOST", false, false, Some(XAI_API_HOST)),
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

        let mut log = RequestLog::start(&self.model, &payload)?;
        let response = self.with_retry(|| self.post(payload.clone())).await?;

        let message = response_to_message(&response)?;
        let usage = response.get("usage").map(get_usage).unwrap_or_else(|| {
            tracing::debug!("Failed to get usage data");
            Usage::default()
        });
        let response_model = get_model(&response);
        log.write(&response, Some(&usage))?;
        Ok((message, ProviderUsage::new(response_model, usage)))
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
                    .response_post("chat/completions", &payload)
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
        XaiProvider::metadata()
            .known_models
            .into_iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("{name} should be advertised"))
    }

    #[test]
    fn default_is_grok_4_7_and_advertised() {
        assert_eq!(XAI_DEFAULT_MODEL, "grok-4.7");
        assert!(XAI_KNOWN_MODELS.contains(&XAI_DEFAULT_MODEL));
        assert_eq!(XaiProvider::metadata().default_model, XAI_DEFAULT_MODEL);
    }

    #[test]
    fn chat_models_take_png_and_jpeg_images() {
        for name in ["grok-4.7", "grok-4.6", "grok-4.5", "grok-build-0.1"] {
            let info = known(name);
            assert_eq!(info.supports_vision, Some(true), "{name} accepts images");
            assert_eq!(
                info.supported_input_mime_types,
                Some(vec![
                    "image/png".to_string(),
                    "image/jpeg".to_string(),
                    "image/jpg".to_string()
                ]),
                "{name} is limited to png/jpeg"
            );
        }
        assert_eq!(known("grok-4.7").context_limit, 500_000);
        assert_eq!(known("grok-imagine-image").supports_vision, None);
    }

    #[test]
    fn models_unusable_through_chat_completions_are_not_advertised() {
        for removed in ["grok-4.20-multi-agent-0309", "grok-imagine-image-quality"] {
            assert!(
                !XAI_KNOWN_MODELS.contains(&removed),
                "{removed} should no longer be advertised"
            );
        }
    }
}
