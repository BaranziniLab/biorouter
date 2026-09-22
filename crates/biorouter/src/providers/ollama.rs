use super::api_client::{ApiClient, AuthMethod};
use super::base::{ConfigKey, MessageStream, Provider, ProviderMetadata, ProviderUsage, Usage};
use super::errors::ProviderError;
use super::retry::ProviderRetry;
use super::utils::{
    get_model, handle_response_openai_compat, handle_status_openai_compat, stream_openai_compat,
    RequestLog,
};
use crate::config::declarative_providers::DeclarativeProviderConfig;
use crate::config::BioRouterMode;
use crate::conversation::message::Message;
use crate::conversation::Conversation;

use crate::model::ModelConfig;
use crate::privacy::ProviderTier;
use crate::providers::formats::openai::{create_request, get_usage, response_to_message};
use crate::utils::safe_truncate;
use anyhow::Result;
use async_trait::async_trait;
use regex::Regex;
use rmcp::model::Tool;
use serde_json::Value;
use std::time::Duration;
use url::Url;

pub const OLLAMA_HOST: &str = "localhost";
pub const OLLAMA_TIMEOUT: u64 = 600;
pub const OLLAMA_DEFAULT_PORT: u16 = 11434;
pub const OLLAMA_DEFAULT_MODEL: &str = "qwen3";
// Empty: with no curated list, the frontend falls back to the live `api/tags`
// endpoint via `fetch_supported_models` and lists the models the user has
// actually installed locally. Users can still write in any model name via the
// "Enter a model not listed..." option (allows_unlisted_models = true below).
pub const OLLAMA_KNOWN_MODELS: &[&str] = &[];
pub const OLLAMA_DOC_URL: &str = "https://ollama.com/library";

#[derive(serde::Serialize)]
pub struct OllamaProvider {
    #[serde(skip)]
    api_client: ApiClient,
    model: ModelConfig,
    supports_streaming: bool,
    name: String,
    /// The base URL this instance resolved at construction — the same string
    /// the API client was handed. `tier()` reads it, never the provider's name:
    /// `from_custom_config` below builds an Ollama-engine provider from a
    /// user-writable JSON file whose `base_url` can point anywhere, and whose
    /// `name` can shadow a built-in.
    #[serde(skip)]
    resolved_base_url: String,
}

impl OllamaProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let config = crate::config::Config::global();
        let host: String = config
            .get_param("OLLAMA_HOST")
            .unwrap_or_else(|_| OLLAMA_HOST.to_string());

        let timeout: Duration =
            Duration::from_secs(config.get_param("OLLAMA_TIMEOUT").unwrap_or(OLLAMA_TIMEOUT));

        let base = if host.starts_with("http://") || host.starts_with("https://") {
            host.clone()
        } else {
            format!("http://{}", host)
        };

        let mut base_url =
            Url::parse(&base).map_err(|e| anyhow::anyhow!("Invalid base URL: {e}"))?;

        let explicit_port = host.contains(':');
        let is_localhost = host == "localhost" || host == "127.0.0.1" || host == "::1";

        if base_url.port().is_none() && !explicit_port && !host.starts_with("http") && is_localhost
        {
            base_url
                .set_port(Some(OLLAMA_DEFAULT_PORT))
                .map_err(|_| anyhow::anyhow!("Failed to set default port"))?;
        }

        let auth = AuthMethod::Custom(Box::new(NoAuth));
        let resolved_base_url = base_url.to_string();
        let api_client = ApiClient::with_timeout(resolved_base_url.clone(), auth, timeout)?;

        Ok(Self {
            api_client,
            model,
            supports_streaming: true,
            name: Self::metadata().name,
            resolved_base_url,
        })
    }

    pub fn from_custom_config(
        model: ModelConfig,
        config: DeclarativeProviderConfig,
    ) -> Result<Self> {
        let timeout = Duration::from_secs(config.timeout_seconds.unwrap_or(OLLAMA_TIMEOUT));

        let base =
            if config.base_url.starts_with("http://") || config.base_url.starts_with("https://") {
                config.base_url.clone()
            } else {
                format!("http://{}", config.base_url)
            };

        let mut base_url = Url::parse(&base)
            .map_err(|e| anyhow::anyhow!("Invalid base URL '{}': {}", config.base_url, e))?;

        let explicit_default_port =
            config.base_url.ends_with(":80") || config.base_url.ends_with(":443");
        let is_https = base_url.scheme() == "https";

        if base_url.port().is_none() && !explicit_default_port && !is_https {
            base_url
                .set_port(Some(OLLAMA_DEFAULT_PORT))
                .map_err(|_| anyhow::anyhow!("Failed to set default port"))?;
        }

        let auth = AuthMethod::Custom(Box::new(NoAuth));
        let resolved_base_url = base_url.to_string();
        let api_client = ApiClient::with_timeout(resolved_base_url.clone(), auth, timeout)?;

        Ok(Self {
            api_client,
            model,
            supports_streaming: config.supports_streaming.unwrap_or(true),
            name: config.name.clone(),
            resolved_base_url,
        })
    }

    async fn post(&self, payload: &Value) -> Result<Value, ProviderError> {
        let response = self
            .api_client
            .response_post("v1/chat/completions", payload)
            .await?;
        handle_response_openai_compat(response).await
    }
}

fn require_ollama_answer(mut stream: MessageStream) -> MessageStream {
    use futures::StreamExt;
    Box::pin(async_stream::try_stream! {
        let mut answered = false;
        let mut finish_reason = None;
        while let Some(item) = stream.next().await {
            let item = item?;
            if let Some(message) = &item.0 {
                answered |= message.is_tool_call()
                    || message.content.iter().filter_map(|content| content.as_text())
                        .any(|text| !text.trim().is_empty());
            }
            if let Some(reason) = item.1.as_ref().and_then(|usage| usage.finish_reason.as_ref()) {
                finish_reason = Some(reason.clone());
            }
            yield item;
        }
        if !answered {
            let detail = if finish_reason.as_deref() == Some("length") {
                "The response reached its token limit before producing an answer. Review the model's output budget or choose another model."
            } else {
                "The model may have returned reasoning only. Retry or choose another model; no answer or tool result was produced."
            };
            Err(ProviderError::RequestFailed(format!(
                "Ollama completed without answer text or tool calls. {detail}"
            )))?;
        }
    })
}

struct NoAuth;

#[async_trait]
impl super::api_client::AuthProvider for NoAuth {
    async fn get_auth_header(&self) -> Result<(String, String)> {
        Ok(("X-No-Auth".to_string(), "true".to_string()))
    }
}

#[async_trait]
impl Provider for OllamaProvider {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::new(
            "ollama",
            "Ollama",
            "Local open source models",
            OLLAMA_DEFAULT_MODEL,
            OLLAMA_KNOWN_MODELS.to_vec(),
            OLLAMA_DOC_URL,
            vec![
                ConfigKey::new("OLLAMA_HOST", true, false, Some(OLLAMA_HOST)),
                ConfigKey::new(
                    "OLLAMA_TIMEOUT",
                    false,
                    false,
                    Some(&(OLLAMA_TIMEOUT.to_string())),
                ),
            ],
        )
        .with_unlisted_models()
        // Ollama ships pointed at this machine, so a default install is
        // Private. An instance pointed off it says so itself, below.
        .with_tier(ProviderTier::Private)
        .with_local_compute()
    }

    fn get_name(&self) -> &str {
        &self.name
    }

    fn tier(&self) -> ProviderTier {
        crate::providers::self_hosted_tier(&self.resolved_base_url)
    }

    /// DR-26: `Local` exactly while the resolved base URL is loopback. A remote
    /// `OLLAMA_HOST` is someone else's server, so it gets no affiliation at all
    /// rather than inheriting `Local`'s blanket permission over every private
    /// extension.
    fn affiliation(&self) -> Option<crate::privacy::affiliation::ModelAffiliation> {
        crate::providers::self_hosted_affiliation(&self.resolved_base_url)
    }

    fn computer_use_destination(&self) -> Option<String> {
        self.api_client
            .computer_use_destination("v1/chat/completions")
    }

    fn computer_use_destination_identity(&self) -> Option<String> {
        Some(
            self.api_client
                .computer_use_destination_identity("v1/chat/completions"),
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
        let config = crate::config::Config::global();
        let biorouter_mode = config.get_biorouter_mode().unwrap_or(BioRouterMode::Auto);
        let filtered_tools = if biorouter_mode == BioRouterMode::Chat {
            &[]
        } else {
            tools
        };

        let payload = create_request(
            model_config,
            system,
            messages,
            filtered_tools,
            &super::utils::ImageFormat::OpenAi,
            false,
        )?;

        let mut log = RequestLog::start(model_config, &payload)?;
        let response = self
            .with_retry(|| async {
                let payload_clone = payload.clone();
                self.post(&payload_clone).await
            })
            .await
            .inspect_err(|e| {
                let _ = log.error(e);
            })?;

        let message = response_to_message(&response)?;

        let usage = response.get("usage").map(get_usage).unwrap_or_else(|| {
            tracing::debug!("Failed to get usage data");
            Usage::default()
        });
        let response_model = get_model(&response);
        log.write(&response, Some(&usage))?;
        Ok((message, ProviderUsage::new(response_model, usage)))
    }

    async fn generate_session_name(
        &self,
        messages: &Conversation,
    ) -> Result<String, ProviderError> {
        let context = self.get_initial_user_messages(messages);
        let message = Message::user().with_text(self.create_session_name_prompt(&context));
        let result = self
            .complete(
                "You are a title generator. Output only the requested title of 4 words or less, with no additional text, reasoning, or explanations.",
                &[message],
                &[],
            )
            .await?;

        let mut description = result.0.as_concat_text();
        description = Self::filter_reasoning_tokens(&description);

        Ok(safe_truncate(&description, 100))
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
        let config = crate::config::Config::global();
        let biorouter_mode = config.get_biorouter_mode().unwrap_or(BioRouterMode::Auto);
        let filtered_tools = if biorouter_mode == BioRouterMode::Chat {
            &[]
        } else {
            tools
        };

        let payload = create_request(
            &self.model,
            system,
            messages,
            filtered_tools,
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
        Ok(require_ollama_answer(stream_openai_compat(response, log)?))
    }

    async fn fetch_supported_models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        let response = self
            .api_client
            .response_get("api/tags")
            .await
            .map_err(|e| ProviderError::RequestFailed(format!("Failed to fetch models: {}", e)))?;

        if !response.status().is_success() {
            return Err(ProviderError::RequestFailed(format!(
                "Failed to fetch models: HTTP {}",
                response.status()
            )));
        }

        let json_response = response.json::<Value>().await.map_err(|e| {
            ProviderError::RequestFailed(format!("Failed to parse response: {}", e))
        })?;

        let models = json_response
            .get("models")
            .and_then(|m| m.as_array())
            .ok_or_else(|| {
                ProviderError::RequestFailed("No models array in response".to_string())
            })?;

        let mut model_names: Vec<String> = models
            .iter()
            .filter_map(|model| model.get("name").and_then(|n| n.as_str()).map(String::from))
            .collect();

        model_names.sort();

        Ok(Some(model_names))
    }
}

impl OllamaProvider {
    fn filter_reasoning_tokens(text: &str) -> String {
        let mut filtered = text.to_string();

        let reasoning_patterns = [
            r"<think>.*?</think>",
            r"<thinking>.*?</thinking>",
            r"Let me think.*?\n",
            r"I need to.*?\n",
            r"First, I.*?\n",
            r"Okay, .*?\n",
            r"So, .*?\n",
            r"Well, .*?\n",
            r"Hmm, .*?\n",
            r"Actually, .*?\n",
            r"Based on.*?I think",
            r"Looking at.*?I would say",
        ];

        for pattern in reasoning_patterns {
            if let Ok(re) = Regex::new(pattern) {
                filtered = re.replace_all(&filtered, "").to_string();
            }
        }
        filtered = filtered
            .replace("<think>", "")
            .replace("</think>", "")
            .replace("<thinking>", "")
            .replace("</thinking>", "");
        filtered = filtered
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");

        filtered
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::declarative_providers::ProviderEngine;
    use crate::conversation::message::Message;
    use crate::providers::base::ProviderStreamItem;
    use futures::TryStreamExt;
    use serde_json::Value;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn config(base_url: &str) -> DeclarativeProviderConfig {
        DeclarativeProviderConfig {
            name: "ollama-test".to_string(),
            engine: ProviderEngine::Ollama,
            display_name: "Ollama test server".to_string(),
            description: None,
            api_key_env: "NOT_USED".to_string(),
            base_url: base_url.to_string(),
            models: vec![],
            headers: None,
            timeout_seconds: Some(5),
            supports_streaming: Some(true),
        }
    }

    async fn provider_for(body: &str) -> (MockServer, OllamaProvider) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let provider = OllamaProvider::from_custom_config(
            ModelConfig::new_or_fail("qwen3"),
            config(&server.uri()),
        )
        .expect("test Ollama provider should construct");
        (server, provider)
    }

    async fn assert_stream_request(server: &MockServer) {
        let requests = server
            .received_requests()
            .await
            .expect("wiremock should return received requests");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url.path(), "/v1/chat/completions");
        let payload: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(payload["stream"], true);
    }

    async fn collect_stream(
        provider: &OllamaProvider,
    ) -> Result<Vec<ProviderStreamItem>, ProviderError> {
        provider
            .stream(
                "system",
                &[Message::user().with_text("test the mocked stream")],
                &[],
            )
            .await?
            .try_collect()
            .await
    }

    #[tokio::test]
    async fn reasoning_only_stream_is_an_explicit_provider_error() {
        let body = concat!(
            "data: {\"id\":\"reasoning-only\",\"model\":\"qwen3\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"reasoning_content\":\"thinking only\",\"content\":null},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"reasoning-only\",\"model\":\"qwen3\",\"choices\":[{\"index\":0,\"delta\":{\"content\":null},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}\n\n",
            "data: [DONE]\n\n",
        );
        let (server, provider) = provider_for(body).await;

        let error = collect_stream(&provider)
            .await
            .expect_err("reasoning without an answer must fail the provider stream");
        assert!(matches!(error, ProviderError::RequestFailed(_)));
        let detail = error.to_string();
        assert!(detail.contains("completed without answer text or tool calls"));
        assert!(detail.contains("reasoning only"));
        assert_stream_request(&server).await;
    }

    #[tokio::test]
    async fn empty_stream_is_an_explicit_provider_error() {
        let (server, provider) = provider_for("data: [DONE]\n\n").await;

        let error = collect_stream(&provider)
            .await
            .expect_err("an empty provider stream must not be reported as success");
        assert!(matches!(error, ProviderError::RequestFailed(_)));
        assert!(error
            .to_string()
            .contains("completed without answer text or tool calls"));
        assert_stream_request(&server).await;
    }

    #[tokio::test]
    async fn normal_text_stream_remains_successful() {
        let body = concat!(
            "data: {\"id\":\"text\",\"model\":\"qwen3\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"ready\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"text\",\"model\":\"qwen3\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}\n\n",
            "data: [DONE]\n\n",
        );
        let (server, provider) = provider_for(body).await;

        let items = collect_stream(&provider)
            .await
            .expect("text response should remain successful");
        let text = items
            .iter()
            .filter_map(|(message, _, _)| message.as_ref())
            .map(Message::as_concat_text)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>();
        assert_eq!(text, ["ready"]);
        assert_stream_request(&server).await;
    }

    #[tokio::test]
    async fn completed_tool_call_stream_remains_successful() {
        let body = concat!(
            "data: {\"id\":\"tool\",\"model\":\"qwen3\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":null,\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"shell\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"tool\",\"model\":\"qwen3\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"command\\\":\\\"pwd\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"tool\",\"model\":\"qwen3\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"\"},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":2,\"total_tokens\":3}}\n\n",
            "data: [DONE]\n\n",
        );
        let (server, provider) = provider_for(body).await;

        let items = collect_stream(&provider)
            .await
            .expect("completed tool call should remain successful");
        let tool_request = items
            .iter()
            .filter_map(|(message, _, _)| message.as_ref())
            .flat_map(|message| message.content.iter())
            .find_map(|content| content.as_tool_request())
            .expect("stream should emit the completed tool request");
        assert_eq!(tool_request.id, "call_1");
        let call = tool_request
            .tool_call
            .as_ref()
            .expect("tool call should be valid");
        assert_eq!(call.name, "shell");
        assert_eq!(call.arguments.as_ref().unwrap()["command"], "pwd");
        assert_stream_request(&server).await;
    }
}
