use super::api_client::{ApiClient, AuthMethod};
use super::errors::ProviderError;
use super::retry::ProviderRetry;
use super::utils::{
    get_model, handle_response_openai_compat, handle_status_openai_compat, stream_openai_compat,
    RequestLog,
};
use crate::conversation::message::{Message, MessageContent};
use crate::model::ModelConfig;
use crate::providers::base::{
    ConfigKey, MessageStream, ModelInfo, Provider, ProviderMetadata, ProviderUsage, Usage,
};
use crate::providers::formats::openai::{create_request, get_usage, response_to_message};
use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::Tool;
use serde_json::Value;

// Xiaomi's MiMo model family, served through an OpenAI-compatible API.
//
// Endpoint and auth were verified live (June 2026) against a real MiMo key:
// the default below is the Singapore "Token Plan" host, which answered
// `GET /v1/models` and `POST /v1/chat/completions` (Bearer auth, OpenAI wire
// format) with HTTP 200 and a real `mimo-v2.5` completion. MiMo serves keys
// per region/plan, so operators on a different tier set `XIAOMI_MIMO_HOST`:
//   - Pay-as-you-go (`sk-` keys): https://api.xiaomimimo.com/v1
//   - Token Plan (`tp-` keys):    https://token-plan-{cn,sgp,ams}.xiaomimimo.com/v1
// The wire format (OpenAI-compatible chat/completions + Bearer auth) follows
// the same shape every other OpenAI-compatible provider here uses.
pub const XIAOMI_MIMO_API_HOST: &str = "https://token-plan-sgp.xiaomimimo.com/v1";
pub const XIAOMI_MIMO_DEFAULT_MODEL: &str = "mimo-v2.5";
pub const XIAOMI_MIMO_KNOWN_MODELS: &[&str] = &[
    // MiMo v2.5 family (~1M context)
    "mimo-v2.5",
    "mimo-v2.5-pro",
    // MiMo v2 family (~256k context)
    "mimo-v2-pro",
    "mimo-v2-omni",
];

pub const XIAOMI_MIMO_DOC_URL: &str = "https://github.com/XiaomiMiMo/MiMo";

/// Whether a MiMo model accepts image input. Only the multimodal "omni" models
/// do on the served endpoints; the text models 404 on images
/// ("No endpoints found that support image input"). Override is intentional and
/// conservative — an unknown/custom model name is treated as text-only so we
/// never feed images to an endpoint that will reject them.
pub fn model_supports_vision(name: &str) -> bool {
    name.to_ascii_lowercase().contains("omni")
}

const IMAGE_OMITTED_PLACEHOLDER: &str = "[image omitted: the active MiMo model does not accept image input. Describe what you need in text, or switch to a vision-capable model such as mimo-v2-omni.]";

/// Defensive: replace any image content with a text placeholder so a text-only
/// model never *receives* image input — even if a tool (e.g. the developer
/// `image_processor` or a screenshot) put an image in the conversation. Without
/// this the served endpoint rejects the request with
/// `404: No endpoints found that support image input`, which strands the agent.
fn strip_image_content(messages: &[Message]) -> Vec<Message> {
    use rmcp::model::{Content, RawContent};
    messages
        .iter()
        .map(|msg| {
            let mut out = msg.clone();
            out.content = msg
                .content
                .iter()
                .map(|c| match c {
                    MessageContent::Image(_) => MessageContent::text(IMAGE_OMITTED_PLACEHOLDER),
                    MessageContent::ToolResponse(tr) => {
                        let mut new_tr = tr.clone();
                        if let Ok(result) = &mut new_tr.tool_result {
                            for ct in result.content.iter_mut() {
                                if matches!(&ct.raw, RawContent::Image(_)) {
                                    *ct = Content::text(IMAGE_OMITTED_PLACEHOLDER);
                                }
                            }
                        }
                        MessageContent::ToolResponse(new_tr)
                    }
                    other => other.clone(),
                })
                .collect();
            out
        })
        .collect()
}

#[derive(serde::Serialize)]
pub struct XiaomiMimoProvider {
    #[serde(skip)]
    api_client: ApiClient,
    model: ModelConfig,
    supports_streaming: bool,
    #[serde(skip)]
    name: String,
}

impl XiaomiMimoProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let config = crate::config::Config::global();
        let api_key: String = config.get_secret("XIAOMI_MIMO_API_KEY")?;
        let host: String = config
            .get_param("XIAOMI_MIMO_HOST")
            .unwrap_or_else(|_| XIAOMI_MIMO_API_HOST.to_string());

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
impl Provider for XiaomiMimoProvider {
    fn metadata() -> ProviderMetadata {
        // Only the multimodal "omni" MiMo models actually accept image input on
        // the served endpoints. The text models (mimo-v2.5 / mimo-v2.5-pro /
        // mimo-v2-pro) return `404: No endpoints found that support image input`
        // when sent an image — so declaring them vision-capable made the harness
        // and UI feed them screenshots (e.g. via the developer image_processor /
        // Biorouter Copilot screen_capture), which then 404'd and got the agent
        // stuck. We therefore declare `.with_vision()` ONLY for vision-capable
        // models (name contains "omni"); see `model_supports_vision`.
        let models = XIAOMI_MIMO_KNOWN_MODELS
            .iter()
            .map(|&name| {
                let info = ModelInfo::new(name, ModelConfig::new_or_fail(name).context_limit());
                if model_supports_vision(name) {
                    info.with_vision()
                } else {
                    info
                }
            })
            .collect();
        ProviderMetadata::with_models(
            "xiaomi_mimo",
            "Xiaomi MiMo",
            "Xiaomi MiMo models via an OpenAI-compatible API. Set XIAOMI_MIMO_HOST to your MiMo endpoint.",
            XIAOMI_MIMO_DEFAULT_MODEL,
            models,
            XIAOMI_MIMO_DOC_URL,
            vec![
                ConfigKey::new("XIAOMI_MIMO_API_KEY", true, true, None),
                ConfigKey::new("XIAOMI_MIMO_HOST", false, false, Some(XIAOMI_MIMO_API_HOST)),
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
        let stripped = (!model_supports_vision(&model_config.model_name))
            .then(|| strip_image_content(messages));
        let messages = stripped.as_deref().unwrap_or(messages);
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
        let stripped =
            (!model_supports_vision(&self.model.model_name)).then(|| strip_image_content(messages));
        let messages = stripped.as_deref().unwrap_or(messages);
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

    #[test]
    fn test_metadata_structure() {
        let metadata = XiaomiMimoProvider::metadata();

        assert_eq!(metadata.name, "xiaomi_mimo");
        assert_eq!(metadata.default_model, "mimo-v2.5");
        assert!(metadata.known_models.iter().any(|m| m.name == "mimo-v2.5"));
        assert!(!metadata.known_models.is_empty());

        assert_eq!(metadata.config_keys.len(), 2);
        assert_eq!(metadata.config_keys[0].name, "XIAOMI_MIMO_API_KEY");
        assert_eq!(metadata.config_keys[1].name, "XIAOMI_MIMO_HOST");
    }

    #[test]
    fn test_only_omni_models_declare_vision() {
        // The text models must NOT advertise vision (they 404 on image input);
        // only the multimodal "omni" model does.
        assert!(model_supports_vision("mimo-v2-omni"));
        assert!(!model_supports_vision("mimo-v2.5"));
        assert!(!model_supports_vision("mimo-v2.5-pro"));
        assert!(!model_supports_vision("mimo-v2-pro"));
        // an unknown/custom model is conservatively text-only
        assert!(!model_supports_vision("some-custom-model"));

        let metadata = XiaomiMimoProvider::metadata();
        for m in &metadata.known_models {
            let expected = m.name.contains("omni");
            assert_eq!(
                m.supports_vision,
                if expected { Some(true) } else { None },
                "vision flag wrong for {}",
                m.name
            );
        }
    }

    #[test]
    fn test_strip_image_content_replaces_images_with_text() {
        use rmcp::model::{CallToolResult, Content};
        let msgs = vec![
            Message::user().with_image("BASE64DATA", "image/png"),
            Message::user().with_tool_response(
                "call_1",
                Ok(CallToolResult {
                    content: vec![
                        Content::text("preview generated"),
                        Content::image("BASE64DATA", "image/png"),
                    ],
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
            ),
        ];
        let stripped = strip_image_content(&msgs);
        // No image content remains anywhere.
        for m in &stripped {
            for c in &m.content {
                assert!(
                    !matches!(c, MessageContent::Image(_)),
                    "top-level image should be stripped"
                );
                if let MessageContent::ToolResponse(tr) = c {
                    if let Ok(r) = &tr.tool_result {
                        for ct in &r.content {
                            assert!(
                                !matches!(&ct.raw, rmcp::model::RawContent::Image(_)),
                                "tool-result image should be stripped"
                            );
                        }
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn test_registered_in_factory() {
        let all = crate::providers::providers().await;
        assert!(
            all.iter().any(|(m, _)| m.name == "xiaomi_mimo"),
            "xiaomi_mimo provider must be registered in the factory registry"
        );
    }
}
