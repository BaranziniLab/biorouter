use anyhow::Result;
use async_stream::try_stream;
use async_trait::async_trait;
use futures::TryStreamExt;
use reqwest::StatusCode;
use serde_json::Value;
use std::io;
use tokio::pin;
use tokio_util::io::StreamReader;

use super::api_client::{ApiClient, ApiResponse, AuthMethod};
use super::base::{ConfigKey, MessageStream, ModelInfo, Provider, ProviderMetadata, ProviderUsage};
use super::errors::ProviderError;
use super::formats::anthropic::{
    apply_thinking_block_binding, create_request, get_usage, response_to_message,
    response_to_streaming_message, THINKING_BINDING_CONTROLS_BETA,
};
use super::utils::{get_model, handle_status_openai_compat, map_http_error_to_provider_error};
use crate::config::declarative_providers::DeclarativeProviderConfig;
use crate::conversation::message::Message;
use crate::model::ModelConfig;
use crate::providers::retry::ProviderRetry;
use crate::providers::utils::RequestLog;
use rmcp::model::Tool;

// Stays on Opus 4.8 although Anthropic's models overview now tells new work
// to start with Claude Opus 5.5 (checked 2026-09-25). This file's rule is to
// promote a default only after a live smoke test through this provider, and
// no Anthropic API key was available to run one when Opus 5.5 was added.
// Two things to weigh when that test runs: Opus 5.5's default effort is
// `medium` (Opus 4.8's is `high`) and BioRouter sends no `output_config.effort`,
// so the switch would quietly lower effort; and it is a preserved-thinking
// model, which only works here because `create_request`'s payload goes
// through `apply_thinking_block_binding` below.
pub const ANTHROPIC_DEFAULT_MODEL: &str = "claude-opus-4-8";
const ANTHROPIC_DEFAULT_FAST_MODEL: &str = "claude-haiku-4-5";
// Verified against Anthropic's models overview and deprecations pages
// (2026-09-25): every entry below is Active. The list is ordered newest →
// oldest; the UI auto-selects the first entry when a user switches providers
// (SwitchModelModal), so the newest Opus sits at the top even while
// ANTHROPIC_DEFAULT_MODEL is older.
const ANTHROPIC_KNOWN_MODELS: &[&str] = &[
    // Claude Opus 5.5 (GA 2026-09-22 — 1M context, 128K output, $4/$20 per
    // MTok, cheaper than Opus 5). Thinking cannot be disabled and forced
    // tool_choice is a 400; BioRouter sends neither. Its thinking blocks are
    // bound to the conversation, see `apply_thinking_block_binding`.
    "claude-opus-5-5",
    // Claude Fable 5.1 (GA 2026-09-01 — tier above Opus, $10/$50, 1M). Same
    // preserved-thinking rules as Opus 5.5, and it needs 30-day data
    // retention: an org on zero data retention gets a 400.
    "claude-fable-5-1",
    // Claude Opus 5 (1M context, $5/$25 per MTok).
    "claude-opus-5",
    "claude-sonnet-5",
    // Claude 4.8
    "claude-opus-4-8",
    // Claude Fable 5 (Legacy on Anthropic's overview — still served,
    // superseded by Fable 5.1 at the same price).
    "claude-fable-5",
    // Claude 4.7
    "claude-opus-4-7",
    // Claude 4.6
    "claude-opus-4-6",
    "claude-sonnet-4-6",
    // Claude 4.5
    "claude-opus-4-5",
    "claude-opus-4-5-20251101",
    "claude-sonnet-4-5",
    "claude-sonnet-4-5-20250929",
    "claude-haiku-4-5",
    "claude-haiku-4-5-20251001",
];

const ANTHROPIC_DOC_URL: &str = "https://platform.claude.com/docs/en/about-claude/models/overview";
const ANTHROPIC_API_VERSION: &str = "2023-06-01";

#[derive(serde::Serialize)]
pub struct AnthropicProvider {
    #[serde(skip)]
    api_client: ApiClient,
    model: ModelConfig,
    supports_streaming: bool,
    name: String,
}

impl AnthropicProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let model = model.with_fast(ANTHROPIC_DEFAULT_FAST_MODEL.to_string());

        let config = crate::config::Config::global();
        let api_key: String = config.get_secret("ANTHROPIC_API_KEY")?;
        let host: String = config
            .get_param("ANTHROPIC_HOST")
            .unwrap_or_else(|_| "https://api.anthropic.com".to_string());

        let auth = AuthMethod::ApiKey {
            header_name: "x-api-key".to_string(),
            key: api_key,
        };

        let api_client =
            ApiClient::new(host, auth)?.with_header("anthropic-version", ANTHROPIC_API_VERSION)?;

        Ok(Self {
            api_client,
            model,
            supports_streaming: true,
            name: Self::metadata().name,
        })
    }

    pub fn from_custom_config(
        model: ModelConfig,
        config: DeclarativeProviderConfig,
    ) -> Result<Self> {
        let global_config = crate::config::Config::global();
        let api_key: String = global_config
            .get_secret(&config.api_key_env)
            .map_err(|_| anyhow::anyhow!("Missing API key: {}", config.api_key_env))?;

        let auth = AuthMethod::ApiKey {
            header_name: "x-api-key".to_string(),
            key: api_key,
        };

        let api_client = ApiClient::new(config.base_url, auth)?
            .with_header("anthropic-version", ANTHROPIC_API_VERSION)?;

        Ok(Self {
            api_client,
            model,
            supports_streaming: config.supports_streaming.unwrap_or(true),
            name: config.name.clone(),
        })
    }

    /// Build the request body for `model_config` and the one `anthropic-beta`
    /// value it needs, if any.
    ///
    /// The betas are decided from the payload, not from `self.model`:
    /// `complete_with_model` may be asked for a different model (the fast
    /// model), and a header chosen for one model and sent with another's body
    /// is exactly how a `block_binding` field ends up without its beta — a 400.
    fn prepare_request(
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Value, Option<String>), ProviderError> {
        let mut payload = create_request(model_config, system, messages, tools)?;
        let binding = apply_thinking_block_binding(&mut payload);
        let beta = Self::anthropic_beta(&model_config.model_name, binding);
        Ok((payload, beta))
    }

    /// Every beta a request needs, comma-separated into ONE `anthropic-beta`
    /// header. `ApiRequestBuilder::header` replaces rather than appends, so two
    /// separate `anthropic-beta` headers would silently keep only the last —
    /// which is what the claude-3-7 pair below did until 2026-09-25.
    fn anthropic_beta(model_name: &str, thinking_binding: bool) -> Option<String> {
        let mut betas: Vec<&str> = Vec::new();

        if model_name.starts_with("claude-3-7-sonnet-") {
            if std::env::var("CLAUDE_THINKING_ENABLED").is_ok() {
                betas.push("output-128k-2025-02-19");
            }
            betas.push("token-efficient-tools-2025-02-19");
        }
        if thinking_binding {
            betas.push(THINKING_BINDING_CONTROLS_BETA);
        }

        (!betas.is_empty()).then(|| betas.join(","))
    }

    async fn post(
        &self,
        payload: &Value,
        beta: Option<&str>,
    ) -> Result<ApiResponse, ProviderError> {
        let mut request = self.api_client.request("v1/messages");

        if let Some(beta) = beta {
            request = request.header("anthropic-beta", beta)?;
        }

        Ok(request.api_post(payload).await?)
    }

    /// With the binding-controls beta the response names every thinking block
    /// the API dropped (`input_transformations`). A drop is expected rather
    /// than an error here — see `apply_thinking_block_binding` — so it is only
    /// worth a debug line, for whoever is checking why a turn re-planned.
    fn log_input_transformations(response: &Value) {
        if let Some(dropped) = response
            .get("input_transformations")
            .and_then(Value::as_array)
            .filter(|dropped| !dropped.is_empty())
        {
            let detail = serde_json::to_string(dropped).unwrap_or_default();
            tracing::debug!(
                count = dropped.len(),
                "Anthropic dropped replayed thinking blocks: {detail}"
            );
        }
    }

    fn anthropic_api_call_result(response: ApiResponse) -> Result<Value, ProviderError> {
        match response.status {
            StatusCode::OK => response.payload.ok_or_else(|| {
                ProviderError::RequestFailed("Response body is not valid JSON".to_string())
            }),
            _ => {
                if response.status == StatusCode::BAD_REQUEST {
                    if let Some(error_msg) = response
                        .payload
                        .as_ref()
                        .and_then(|p| p.get("error"))
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                    {
                        let msg = error_msg.to_string();
                        if msg.to_lowercase().contains("too long")
                            || msg.to_lowercase().contains("too many")
                        {
                            return Err(ProviderError::ContextLengthExceeded(msg));
                        }
                    }
                }
                Err(map_http_error_to_provider_error(
                    response.status,
                    response.payload,
                ))
            }
        }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn metadata() -> ProviderMetadata {
        // Every current Claude model accepts image input (Anthropic's models
        // overview, 2026-09-25). If a text-only Claude ships, switch this to a
        // per-model match.
        //
        // Context windows are per-model, not a blanket 200k: Opus 5.5, Opus 5,
        // Fable 5 / 5.1, Sonnet 5 and Opus 4.6+/Sonnet 4.6 are 1M (GA — no beta
        // header), while the 4.5 tier and Haiku 4.5 remain 200k.
        // `context_window_for` is the single source of truth; don't hardcode a
        // number here.
        let models: Vec<ModelInfo> = ANTHROPIC_KNOWN_MODELS
            .iter()
            .map(|&model_name| {
                ModelInfo::new(model_name, ModelConfig::context_window_for(model_name))
                    .with_vision()
            })
            .collect();

        ProviderMetadata::with_models(
            "anthropic",
            "Anthropic",
            "Claude and other models from Anthropic",
            ANTHROPIC_DEFAULT_MODEL,
            models,
            ANTHROPIC_DOC_URL,
            vec![
                ConfigKey::new("ANTHROPIC_API_KEY", true, true, None),
                ConfigKey::new(
                    "ANTHROPIC_HOST",
                    true,
                    false,
                    Some("https://api.anthropic.com"),
                ),
            ],
        )
        .with_unlisted_models()
    }

    fn get_name(&self) -> &str {
        &self.name
    }

    fn computer_use_destination(&self) -> Option<String> {
        self.api_client.computer_use_destination("v1/messages")
    }

    fn computer_use_destination_identity(&self) -> Option<String> {
        Some(
            self.api_client
                .computer_use_destination_identity("v1/messages"),
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
        let (payload, beta) = Self::prepare_request(model_config, system, messages, tools)?;

        let response = self
            .with_retry(|| async { self.post(&payload, beta.as_deref()).await })
            .await?;

        let json_response = Self::anthropic_api_call_result(response)?;
        Self::log_input_transformations(&json_response);

        let message = response_to_message(&json_response)?;
        let usage = get_usage(&json_response)?;
        tracing::debug!("🔍 Anthropic non-streaming parsed usage: input_tokens={:?}, output_tokens={:?}, total_tokens={:?}",
                usage.input_tokens, usage.output_tokens, usage.total_tokens);

        let response_model = get_model(&json_response);
        let mut log = RequestLog::start(&self.model, &payload)?;
        log.write(&json_response, Some(&usage))?;
        let provider_usage = ProviderUsage::new(response_model, usage);
        tracing::debug!(
            "🔍 Anthropic non-streaming returning ProviderUsage: {:?}",
            provider_usage
        );
        Ok((message, provider_usage))
    }

    async fn fetch_supported_models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        let response = self.api_client.api_get("v1/models").await?;

        if response.status != StatusCode::OK {
            return Err(map_http_error_to_provider_error(
                response.status,
                response.payload,
            ));
        }

        let json = response.payload.unwrap_or_default();
        let arr = match json.get("data").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => return Ok(None),
        };

        let mut models: Vec<String> = arr
            .iter()
            .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        models.sort();
        Ok(Some(models))
    }

    async fn stream(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let (mut payload, beta) = Self::prepare_request(&self.model, system, messages, tools)?;
        payload
            .as_object_mut()
            .unwrap()
            .insert("stream".to_string(), Value::Bool(true));

        let mut request = self.api_client.request("v1/messages");
        let mut log = RequestLog::start(&self.model, &payload)?;

        if let Some(beta) = beta.as_deref() {
            request = request.header("anthropic-beta", beta)?;
        }

        let resp = request.response_post(&payload).await.inspect_err(|e| {
            let _ = log.error(e);
        })?;
        let response = handle_status_openai_compat(resp).await.inspect_err(|e| {
            let _ = log.error(e);
        })?;

        let stream = response.bytes_stream().map_err(io::Error::other);

        Ok(Box::pin(try_stream! {
            let stream_reader = StreamReader::new(stream);
            let framed = tokio_util::codec::FramedRead::new(stream_reader, tokio_util::codec::LinesCodec::new()).map_err(anyhow::Error::from);

            let message_stream = response_to_streaming_message(framed);
            pin!(message_stream);
            while let Some(message) = futures::StreamExt::next(&mut message_stream).await {
                let (message, usage, pending) = message.map_err(|e| ProviderError::RequestFailed(format!("Stream decode error: {}", e)))?;
                log.write(&message, usage.as_ref().map(|f| f.usage).as_ref())?;
                yield (message, usage, pending);
            }
        }))
    }

    fn supports_streaming(&self) -> bool {
        self.supports_streaming
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_anthropic_over_limit_response_to_context_length_error() {
        let response = ApiResponse {
            status: StatusCode::BAD_REQUEST,
            payload: Some(json!({
                "type": "error",
                "error": {
                    "type": "invalid_request_error",
                    "message": "prompt is too long: 1000001 tokens > 1000000 maximum"
                }
            })),
        };

        let error = AnthropicProvider::anthropic_api_call_result(response).unwrap_err();

        assert!(matches!(error, ProviderError::ContextLengthExceeded(_)));
    }

    // Opus 5.5 and Fable 5.1 bind each thinking block to the prefix it was
    // produced under, and BioRouter's prefix moves on every request (hourly
    // system-prompt timestamp, MOIM). Without `drop_block` a new account's
    // second request 400s. The newest model is also the one the UI selects
    // first, so this is the request a new user sends.
    #[test]
    fn preserved_thinking_models_opt_into_drop_block_with_the_beta() {
        for model in ["claude-opus-5-5", "claude-fable-5-1"] {
            let config = ModelConfig::new_or_fail(model);
            let (payload, beta) = AnthropicProvider::prepare_request(
                &config,
                "system",
                &[Message::user().with_text("hi")],
                &[],
            )
            .unwrap();

            assert_eq!(
                payload["thinking"],
                json!({
                    "type": "adaptive",
                    "block_binding": { "prefix_mismatch_behavior": "drop_block" }
                }),
                "{model}"
            );
            assert_eq!(
                beta.as_deref(),
                Some(THINKING_BINDING_CONTROLS_BETA),
                "{model}: block_binding without the beta header is a 400"
            );
        }
    }

    #[test]
    fn other_claude_models_send_neither_the_beta_nor_block_binding() {
        for model in [
            ANTHROPIC_DEFAULT_MODEL,
            "claude-opus-5",
            "claude-sonnet-5",
            "claude-fable-5",
            ANTHROPIC_DEFAULT_FAST_MODEL,
        ] {
            let config = ModelConfig::new_or_fail(model);
            let (payload, beta) = AnthropicProvider::prepare_request(
                &config,
                "system",
                &[Message::user().with_text("hi")],
                &[],
            )
            .unwrap();

            assert_eq!(beta, None, "{model}");
            assert!(
                payload
                    .get("thinking")
                    .and_then(|thinking| thinking.get("block_binding"))
                    .is_none(),
                "{model}"
            );
        }
    }

    // `ApiRequestBuilder::header` replaces a header of the same name, so each
    // beta has to be joined into one value or all but the last are lost.
    #[test]
    fn betas_share_one_comma_separated_header() {
        let beta = AnthropicProvider::anthropic_beta("claude-3-7-sonnet-20250219", true)
            .expect("two betas");
        let parts: Vec<&str> = beta.split(',').collect();
        assert!(
            parts.contains(&"token-efficient-tools-2025-02-19"),
            "{beta}"
        );
        assert!(parts.contains(&THINKING_BINDING_CONTROLS_BETA), "{beta}");
    }

    // The header is decided by the model the BODY names. `complete_with_model`
    // can be handed a different model than the provider was built with (the
    // fast model), and a beta chosen from `self.model` would then ride with
    // the wrong body — or go missing from the one that needs it.
    #[tokio::test]
    async fn the_wire_request_carries_the_beta_for_the_model_it_names() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "msg_1",
                "type": "message",
                "role": "assistant",
                "model": "claude-opus-5-5",
                "content": [{ "type": "text", "text": "ok" }],
                "stop_reason": "end_turn",
                "usage": { "input_tokens": 3, "output_tokens": 1 },
                "input_transformations": []
            })))
            .mount(&server)
            .await;

        let api_client = ApiClient::new(
            server.uri(),
            AuthMethod::ApiKey {
                header_name: "x-api-key".to_string(),
                key: "test-key".to_string(),
            },
        )
        .unwrap()
        .with_header("anthropic-version", ANTHROPIC_API_VERSION)
        .unwrap();
        // Built for Opus 4.8, asked for Opus 5.5.
        let provider = AnthropicProvider {
            api_client,
            model: ModelConfig::new_or_fail(ANTHROPIC_DEFAULT_MODEL),
            supports_streaming: true,
            name: "anthropic".to_string(),
        };

        provider
            .complete_with_model(
                &ModelConfig::new_or_fail("claude-opus-5-5"),
                "system",
                &[Message::user().with_text("hi")],
                &[],
            )
            .await
            .unwrap();

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(
            request
                .headers
                .get("anthropic-beta")
                .and_then(|value| value.to_str().ok()),
            Some(THINKING_BINDING_CONTROLS_BETA)
        );
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["model"], "claude-opus-5-5");
        assert_eq!(
            body["thinking"]["block_binding"]["prefix_mismatch_behavior"],
            "drop_block"
        );
    }
}
