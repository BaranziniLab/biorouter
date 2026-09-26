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

// z.ai is the international platform of Zhipu AI; it serves the GLM family of
// models through an OpenAI-compatible API (and a separate Anthropic-compatible
// surface used by Claude Code — not used here). Verified live against
// docs.z.ai (June 2026): base URL `/api/paas/v4`, Bearer-token auth; the
// chat-completion API reference still names that pay-as-you-go path
// (2026-09-25).
pub const ZAI_API_HOST: &str = "https://api.z.ai/api/paas/v4";
// GLM-5.3 (released 2026-08-18 per z.ai's release notes) is z.ai's GA
// flagship and the first entry and default of the `model` enum in its own
// chat-completion API reference (docs.z.ai, read 2026-09-25).
pub const ZAI_DEFAULT_MODEL: &str = "glm-5.3";
/// Newest first, default first: the desktop model switcher preselects entry 0.
pub const ZAI_KNOWN_MODELS: &[&str] = &[
    // GLM-5 family. 5.3 and 5.3-Flash are 1M-context and always reason.
    // glm-5-turbo is gone from z.ai's price list and model enum but has no
    // deprecation notice and its guide page is live (2026-09-25), so it stays
    // until z.ai says otherwise.
    "glm-5.3",
    "glm-5.3-flash",
    "glm-5.2",
    "glm-5.1",
    "glm-5",
    "glm-5-turbo",
    // GLM-4 family
    "glm-4.7",
    "glm-4.6",
    "glm-4.5",
    "glm-4.5-air",
];

/// The listed models that take image input. GLM-5.3-Flash is natively
/// multimodal (video, image, text and file input); GLM-5.3 itself and every
/// other listed model are text-only (docs.z.ai model pages, 2026-09-25).
const ZAI_VISION_MODELS: &[&str] = &["glm-5.3-flash"];

pub const ZAI_DOC_URL: &str = "https://docs.z.ai/guides/overview/pricing";

/// Build the chat-completions body for a z.ai request.
///
/// GLM-5.3 and GLM-5.3-Flash reason on every request: `thinking: {"type":
/// "disabled"}` makes the request fail, and `reasoning_effort` accepts only
/// low/high/max (docs.z.ai/guides/llm/glm-5.3, 2026-09-25). The shared OpenAI
/// builder sends neither field for a GLM model, which is what lets those
/// models work here at all; `zai_request_never_disables_glm_5_3_reasoning`
/// holds that in place.
fn zai_request(
    model_config: &ModelConfig,
    system: &str,
    messages: &[Message],
    tools: &[Tool],
    for_streaming: bool,
) -> Result<Value, ProviderError> {
    Ok(create_request(
        model_config,
        system,
        messages,
        tools,
        &super::utils::ImageFormat::OpenAi,
        for_streaming,
    )?)
}

#[derive(serde::Serialize)]
pub struct ZaiProvider {
    #[serde(skip)]
    api_client: ApiClient,
    model: ModelConfig,
    supports_streaming: bool,
    #[serde(skip)]
    name: String,
}

impl ZaiProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let config = crate::config::Config::global();
        let api_key: String = config.get_secret("ZAI_API_KEY")?;
        let host: String = config
            .get_param("ZAI_HOST")
            .unwrap_or_else(|_| ZAI_API_HOST.to_string());

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
impl Provider for ZaiProvider {
    fn metadata() -> ProviderMetadata {
        let models = ZAI_KNOWN_MODELS
            .iter()
            .map(|&name| {
                let info = ModelInfo::new(name, ModelConfig::new_or_fail(name).context_limit());
                if ZAI_VISION_MODELS.contains(&name) {
                    info.with_vision()
                } else {
                    info
                }
            })
            .collect();
        ProviderMetadata::with_models(
            "zai",
            "z.ai",
            "GLM models from z.ai (Zhipu AI), including the GLM-4 and GLM-5 families via an OpenAI-compatible API",
            ZAI_DEFAULT_MODEL,
            models,
            ZAI_DOC_URL,
            vec![
                ConfigKey::new("ZAI_API_KEY", true, true, None),
                ConfigKey::new("ZAI_HOST", false, false, Some(ZAI_API_HOST)),
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
        let payload = zai_request(model_config, system, messages, tools, false)?;

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
        let payload = zai_request(&self.model, system, messages, tools, true)?;
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
        let metadata = ZaiProvider::metadata();

        assert_eq!(metadata.name, "zai");
        assert_eq!(metadata.default_model, "glm-5.3");
        // The model switcher preselects entry 0, so it must be the default.
        assert_eq!(metadata.known_models[0].name, metadata.default_model);
        assert!(metadata.known_models.iter().any(|m| m.name == "glm-4.6"));

        assert_eq!(metadata.config_keys.len(), 2);
        assert_eq!(metadata.config_keys[0].name, "ZAI_API_KEY");
        assert_eq!(metadata.config_keys[1].name, "ZAI_HOST");
        // Host default points at the OpenAI-compatible base URL.
        assert_eq!(
            metadata.config_keys[1].default,
            Some(ZAI_API_HOST.to_string())
        );
    }

    #[test]
    fn glm_5_3_flash_is_the_only_listed_vision_model() {
        let metadata = ZaiProvider::metadata();
        let flash = metadata
            .known_models
            .iter()
            .find(|m| m.name == "glm-5.3-flash")
            .expect("glm-5.3-flash listed");
        assert_eq!(flash.supports_vision, Some(true));
        assert!(flash
            .supported_input_mime_types
            .as_ref()
            .is_some_and(|mimes| mimes.iter().any(|m| m == "image/png")));

        // GLM-5.3 itself is text-only; so is every other listed model.
        for model in metadata
            .known_models
            .iter()
            .filter(|m| m.name != "glm-5.3-flash")
        {
            assert_eq!(model.supports_vision, None, "{} is text-only", model.name);
        }

        // Both 5.3 models are 1M-context, from the registry, not a pattern.
        for name in ["glm-5.3", "glm-5.3-flash"] {
            let info = metadata
                .known_models
                .iter()
                .find(|m| m.name == name)
                .unwrap_or_else(|| panic!("{name} listed"));
            assert_eq!(info.context_limit, 1_048_576, "{name}");
        }
    }

    /// GLM-5.3 always reasons: `thinking: {"type": "disabled"}` fails the
    /// request, and `reasoning_effort` accepts only low/high/max — so the
    /// "medium" the OpenAI builder defaults reasoning models to would fail
    /// too. Guard the request z.ai actually receives, with and without a
    /// per-turn effort, streaming and not.
    #[test]
    fn zai_request_never_disables_glm_5_3_reasoning() {
        use crate::agents::effort::ReasoningEffort;

        for name in ["glm-5.3", "glm-5.3-flash"] {
            for effort in [
                None,
                Some(ReasoningEffort::Quick),
                Some(ReasoningEffort::Normal),
                Some(ReasoningEffort::Deep),
            ] {
                for for_streaming in [false, true] {
                    let model = ModelConfig::new_or_fail(name).with_reasoning_effort(effort);
                    let payload = zai_request(&model, "system", &[], &[], for_streaming)
                        .expect("request builds");
                    assert_eq!(payload["model"], name);
                    assert!(
                        payload.get("thinking").is_none(),
                        "{name}: thinking cannot be disabled; got {payload}"
                    );
                    if let Some(effort) = payload.get("reasoning_effort") {
                        assert!(
                            ["low", "high", "max"].contains(&effort.as_str().unwrap_or_default()),
                            "{name}: z.ai rejects reasoning_effort {effort}"
                        );
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn test_registered_in_factory() {
        let all = crate::providers::providers().await;
        assert!(
            all.iter().any(|(m, _)| m.name == "zai"),
            "zai provider must be registered in the factory registry"
        );
    }
}
