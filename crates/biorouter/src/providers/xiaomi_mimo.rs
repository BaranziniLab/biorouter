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
// MiMo V2.6 (2026-09-22) replaces V2.5 at the same prices: Xiaomi deprecated
// mimo-v2.5 and mimo-v2.5-pro with a shutdown at 2026-10-21 10:00 Beijing
// time and NO replacement routing — requests to either name fail after that
// (platform.xiaomimimo.com, read 2026-09-24). The v2-pro / v2-omni names
// expired on 2026-06-30. Flash is the default because it is the same-price
// successor to the old default and is on the Token Plan the default host
// serves ("V2.6 Flagship Model Access").
pub const XIAOMI_MIMO_DEFAULT_MODEL: &str = "mimo-v2.6-flash";
/// Default first: the desktop model switcher preselects entry 0.
pub const XIAOMI_MIMO_KNOWN_MODELS: &[&str] = &[
    // MiMo V2.6 family: 1M context, text/image/video/audio input.
    "mimo-v2.6-flash",
    "mimo-v2.6-pro",
];

pub const XIAOMI_MIMO_DOC_URL: &str = "https://github.com/XiaomiMiMo/MiMo";

/// MiMo chat models that accept image input, by exact id. Xiaomi lists the
/// input modality of both V2.6 models as "Text, Image, Video, Audio"
/// (platform.xiaomimimo.com, 2026-09-24).
///
/// An explicit list, not a name heuristic: the rule this replaced ("the name
/// contains `omni`") was right for the V2 generation, where only mimo-v2-omni
/// took images and the text models 404'd on them ("No endpoints found that
/// support image input"), but it would have marked every V2.6 model text-only
/// and stripped the images they accept.
const XIAOMI_MIMO_VISION_MODELS: &[&str] = &["mimo-v2.6-flash", "mimo-v2.6-pro"];

/// Whether a MiMo model accepts image input. Deliberately conservative: an
/// unknown or custom model name is treated as text-only, so images are never
/// fed to an endpoint that will reject them.
pub fn model_supports_vision(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    XIAOMI_MIMO_VISION_MODELS.contains(&name.as_str())
}

const IMAGE_OMITTED_PLACEHOLDER: &str = "[image omitted: the active MiMo model does not accept image input. Describe what you need in text, or switch to a vision-capable model such as mimo-v2.6-flash.]";

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
        // Declare `.with_vision()` ONLY for models that really take images. A
        // text-only MiMo model (the V2 generation's mimo-v2.5 / mimo-v2.5-pro /
        // mimo-v2-pro) returns `404: No endpoints found that support image
        // input` when sent one, so declaring it vision-capable made the harness
        // and UI feed it screenshots (e.g. via the developer image_processor /
        // Biorouter Copilot screen_capture), which then 404'd and got the agent
        // stuck. Both listed V2.6 models take images; see
        // `model_supports_vision`.
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
        assert_eq!(metadata.default_model, "mimo-v2.6-flash");
        // The model switcher preselects entry 0, so it must be the default.
        assert_eq!(metadata.known_models[0].name, metadata.default_model);
        let names: Vec<&str> = metadata
            .known_models
            .iter()
            .map(|m| m.name.as_str())
            .collect();
        assert_eq!(names, ["mimo-v2.6-flash", "mimo-v2.6-pro"]);
        // Retired upstream: v2-pro / v2-omni expired 2026-06-30, and v2.5 /
        // v2.5-pro shut down 2026-10-21 with no replacement routing.
        for retired in ["mimo-v2-pro", "mimo-v2-omni", "mimo-v2.5", "mimo-v2.5-pro"] {
            assert!(!names.contains(&retired), "{retired} must not be offered");
        }
        // Both are 1M-context, from their own registry entries rather than
        // the older `mimo-v2` pattern's 256k.
        for model in &metadata.known_models {
            assert_eq!(model.context_limit, 1_048_576, "{}", model.name);
        }

        assert_eq!(metadata.config_keys.len(), 2);
        assert_eq!(metadata.config_keys[0].name, "XIAOMI_MIMO_API_KEY");
        assert_eq!(metadata.config_keys[1].name, "XIAOMI_MIMO_HOST");
    }

    #[test]
    fn vision_is_declared_per_model_id() {
        // Both V2.6 models take images, and neither name contains "omni" —
        // the heuristic this replaced would have stripped their images.
        assert!(model_supports_vision("mimo-v2.6-flash"));
        assert!(model_supports_vision("mimo-v2.6-pro"));
        assert!(
            model_supports_vision("MiMo-V2.6-Flash"),
            "ids are case-folded"
        );
        // The V2 generation's text models 404 on image input.
        assert!(!model_supports_vision("mimo-v2.5"));
        assert!(!model_supports_vision("mimo-v2.5-pro"));
        assert!(!model_supports_vision("mimo-v2-pro"));
        // An unknown or custom model is conservatively text-only — including
        // one that merely looks multimodal.
        assert!(!model_supports_vision("some-custom-model"));
        assert!(!model_supports_vision("some-omni-model"));

        let metadata = XiaomiMimoProvider::metadata();
        for m in &metadata.known_models {
            assert_eq!(
                m.supports_vision,
                model_supports_vision(&m.name).then_some(true),
                "vision flag wrong for {}",
                m.name
            );
            assert_eq!(m.supports_vision, Some(true), "{} takes images", m.name);
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
