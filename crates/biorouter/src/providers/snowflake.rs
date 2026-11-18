use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::api_client::{ApiClient, AuthMethod};
use super::base::{ConfigKey, Provider, ProviderMetadata, ProviderUsage};
use super::errors::ProviderError;
use super::formats::openai::{create_request, get_usage};
use super::formats::openrouter::{
    add_reasoning_details_to_request_for_provider, response_to_message_for_provider,
};
use super::retry::ProviderRetry;
use super::utils::{get_model, handle_response_openai_compat, ImageFormat, RequestLog};
use crate::config::ConfigError;
use crate::conversation::message::Message;

use crate::model::ModelConfig;
use rmcp::model::Tool;

// Cortex Inference's GA Claude catalogue (2026-11-17). Account region and
// cross-region policy remain authoritative; this provider never enables a
// cross-region override. Fable is private preview, Haiku 5.5 is public preview,
// and Gemini is not available in Cortex Inference. Explicit unlisted choices
// remain supported without treating those gated routes as universal defaults.
pub const SNOWFLAKE_DEFAULT_MODEL: &str = "claude-opus-5-5";
pub const SNOWFLAKE_KNOWN_MODELS: &[&str] = &[
    SNOWFLAKE_DEFAULT_MODEL,
    "claude-sonnet-5-5",
    "claude-sonnet-4-6",
    "claude-sonnet-5",
    "claude-opus-5",
    "claude-opus-4-8",
    "claude-opus-4-7",
    "claude-opus-4-6",
    "claude-opus-4-5",
    "claude-sonnet-4-5",
    "claude-haiku-4-5",
];

pub const SNOWFLAKE_DOC_URL: &str =
    "https://docs.snowflake.com/en/user-guide/snowflake-cortex/aisql-regional-availability";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SnowflakeAuth {
    Token(String),
}

impl SnowflakeAuth {
    pub fn token(token: String) -> Self {
        Self::Token(token)
    }
}

#[derive(Debug, serde::Serialize)]
pub struct SnowflakeProvider {
    #[serde(skip)]
    api_client: ApiClient,
    model: ModelConfig,
    image_format: ImageFormat,
    #[serde(skip)]
    name: String,
}

impl SnowflakeProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let config = crate::config::Config::global();
        let mut host: Result<String, ConfigError> = config.get_param("SNOWFLAKE_HOST");
        if host.is_err() {
            host = config.get_secret("SNOWFLAKE_HOST")
        }
        if host.is_err() {
            return Err(ConfigError::NotFound(
                "Did not find SNOWFLAKE_HOST in either config file or keyring".to_string(),
            )
            .into());
        }

        let mut host = host?;

        // Convert host to lowercase
        host = host.to_lowercase();

        // Ensure host ends with snowflakecomputing.com
        if !host.ends_with("snowflakecomputing.com") {
            host = format!("{}.snowflakecomputing.com", host);
        }

        let mut token: Result<String, ConfigError> = config.get_param("SNOWFLAKE_TOKEN");

        if token.is_err() {
            token = config.get_secret("SNOWFLAKE_TOKEN")
        }

        if token.is_err() {
            return Err(ConfigError::NotFound(
                "Did not find SNOWFLAKE_TOKEN in either config file or keyring".to_string(),
            )
            .into());
        }

        // Ensure host has https:// prefix
        let base_url = if !host.starts_with("https://") && !host.starts_with("http://") {
            format!("https://{}", host)
        } else {
            host
        };

        let auth = AuthMethod::BearerToken(token?);
        let api_client = ApiClient::new(base_url, auth)?.with_header("User-Agent", "biorouter")?;

        Ok(Self {
            api_client,
            model,
            image_format: ImageFormat::OpenAi,
            name: Self::metadata().name,
        })
    }

    async fn post(&self, payload: &Value) -> Result<Value, ProviderError> {
        let response = self
            .api_client
            .response_post("api/v2/cortex/v1/chat/completions", payload)
            .await?;
        handle_response_openai_compat(response).await
    }
}

#[async_trait]
impl Provider for SnowflakeProvider {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::new(
            "snowflake",
            "Snowflake",
            "Access the latest models using Snowflake Cortex services.",
            SNOWFLAKE_DEFAULT_MODEL,
            SNOWFLAKE_KNOWN_MODELS.to_vec(),
            SNOWFLAKE_DOC_URL,
            vec![
                ConfigKey::new("SNOWFLAKE_HOST", true, false, None),
                ConfigKey::new("SNOWFLAKE_TOKEN", true, true, None),
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
        let mut payload = create_request(
            model_config,
            system,
            messages,
            tools,
            &self.image_format,
            false,
        )?;

        // Cortex rejects the deprecated OpenAI field even for Claude models.
        if let Some(max_tokens) = payload.as_object_mut().unwrap().remove("max_tokens") {
            payload["max_completion_tokens"] = max_tokens;
        }
        add_reasoning_details_to_request_for_provider(&mut payload, messages, "snowflake");

        let mut log = RequestLog::start(model_config, &payload)?;

        let response = self
            .with_retry(|| async {
                let payload_clone = payload.clone();
                self.post(&payload_clone).await
            })
            .await?;

        let message = response_to_message_for_provider(&response, "snowflake")?;
        let usage = response.get("usage").map(get_usage).unwrap_or_default();
        let response_model = get_model(&response);

        log.write(&response, Some(&usage))?;

        Ok((message, ProviderUsage::new(response_model, usage)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The UI auto-selects `known_models[0]` on a provider switch, so the
    /// declared default must be the first entry.
    #[test]
    fn the_default_is_the_first_entry() {
        let metadata = SnowflakeProvider::metadata();
        assert_eq!(metadata.default_model, "claude-opus-5-5");
        assert_eq!(metadata.known_models[0].name, metadata.default_model);
    }

    #[test]
    fn catalog_tracks_snowflakes_lifecycle() {
        // Legacy since 2026-08-12 and a new account's call fails; EOL 2026-10-14.
        assert!(!SNOWFLAKE_KNOWN_MODELS.contains(&"claude-4-sonnet"));
        for model in [
            "claude-sonnet-5",
            "claude-opus-5",
            "claude-opus-5-5",
            "claude-sonnet-5-5",
        ] {
            assert!(SNOWFLAKE_KNOWN_MODELS.contains(&model), "{model}");
        }
        // Opus 5.5 and Sonnet 5.5 became GA on 2026-11-06.
        assert_eq!(SNOWFLAKE_DEFAULT_MODEL, "claude-opus-5-5");
        // Fable is a private preview there; not offered.
        assert!(!SNOWFLAKE_KNOWN_MODELS
            .iter()
            .any(|model| model.contains("fable")));
    }
}

#[cfg(test)]
mod hosted_refresh_tests {
    use super::*;
    use crate::conversation::message::MessageContent;
    use rmcp::model::{CallToolResult, Content};
    use serde_json::json;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    #[test]
    fn hosted_refresh_snowflake_only_offers_cortex_inference_claude_routes() {
        let metadata = SnowflakeProvider::metadata();
        assert_eq!(metadata.default_model, "claude-opus-5-5");
        assert_eq!(metadata.known_models[0].name, metadata.default_model);
        assert!(SNOWFLAKE_KNOWN_MODELS.contains(&"claude-sonnet-5-5"));
        assert!(!SNOWFLAKE_KNOWN_MODELS
            .iter()
            .any(|m| m.contains("gemini") || m.contains("fable")));
    }

    #[tokio::test]
    async fn hosted_refresh_snowflake_cortex_json_tools_round_trip_on_the_documented_route() {
        let server = MockServer::start().await;
        let call_count = Arc::new(AtomicUsize::new(0));
        let count = call_count.clone();
        Mock::given(method("POST")).respond_with(move |_: &Request| {
            let message = if count.fetch_add(1,Ordering::SeqCst)==0 {
                json!({"role":"assistant","content":null,"tool_calls":[{"id":"lookup-1","type":"function","function":{"name":"lookup","arguments":"{}"}}],"reasoning_details":[{"type":"reasoning.text","text":"lookup first","signature":"cortex-fixture-signature"}]})
            } else { json!({"role":"assistant","content":"record found"}) };
            ResponseTemplate::new(200).set_body_json(json!({"id":"cortex-fixture","model":"claude-opus-5-5","choices":[{"index":0,"message":message,"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":4,"total_tokens":16}}))
        }).mount(&server).await;
        let model = ModelConfig::new_or_fail("claude-opus-5-5")
            .with_temperature(Some(0.0))
            .with_max_tokens(Some(1024));
        let provider = SnowflakeProvider {
            api_client: ApiClient::new(
                server.uri(),
                AuthMethod::BearerToken("fixture-token".into()),
            )
            .unwrap(),
            model: model.clone(),
            image_format: ImageFormat::OpenAi,
            name: "snowflake".into(),
        };
        let tool: Tool=serde_json::from_value(json!({"name":"lookup","description":"Look up a record","inputSchema":{"type":"object","properties":{}}})).unwrap();
        let mut history = vec![Message::user().with_text("look up a record")];
        let (assistant, _) = provider
            .complete_with_model(&model, "system", &history, std::slice::from_ref(&tool))
            .await
            .expect("Cortex JSON tool response");
        assert!(
            assistant
                .content
                .iter()
                .any(|c| matches!(c, MessageContent::ToolRequest(_))),
            "the tool call must survive decoding"
        );
        history.push(assistant);
        history.push(Message::user().with_tool_response(
            "lookup-1",
            Ok(CallToolResult::success(vec![Content::text("found")])),
        ));
        let (final_message, _) = provider
            .complete_with_model(&model, "system", &history, &[tool])
            .await
            .unwrap();
        assert!(final_message
            .content
            .iter()
            .any(|c| matches!(c,MessageContent::Text(t) if t.text=="record found")));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        for request in &requests {
            assert_eq!(request.url.path(), "/api/v2/cortex/v1/chat/completions");
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            assert!(body.get("temperature").is_none());
            assert!(body.get("max_tokens").is_none());
            assert_eq!(body["max_completion_tokens"], 1024);
            assert!(
                body.get("cross_region_inference").is_none(),
                "account residency policy must stay server-side"
            );
        }
        let followup: Value = serde_json::from_slice(&requests[1].body).unwrap();
        assert_eq!(
            followup["messages"][2]["reasoning_details"],
            json!([{"type":"reasoning.text","text":"lookup first","signature":"cortex-fixture-signature"}])
        );
        assert_eq!(followup["messages"][2]["tool_calls"][0]["id"], "lookup-1");
        assert_eq!(followup["messages"][3]["tool_call_id"], "lookup-1");
        assert_eq!(followup["messages"][3]["content"], "found");
    }
}

#[cfg(test)]
mod hosted_refresh_reasoning_origin_tests {
    use super::*;
    use crate::conversation::message::MessageContent;
    use crate::providers::formats::openrouter as openrouter_format;
    use crate::providers::openrouter::hosted_refresh_reasoning_origin_test_provider;
    use futures::StreamExt;
    use rmcp::model::{CallToolResult, Content};
    use serde_json::json;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const OPENROUTER_MODEL: &str = "anthropic/claude-opus-5.5";
    const CORTEX_MODEL: &str = "claude-opus-5-5";

    fn reasoning_details(provider: &str) -> Value {
        if provider == "openrouter" {
            json!([{"type":"reasoning.encrypted","data":"openrouter-only-opaque","format":"anthropic-claude-v1","id":"reasoning-1"}])
        } else {
            json!([{"type":"reasoning.text","text":"lookup first","signature":"cortex-only-signature"}])
        }
    }

    fn response(details: &Value) -> Value {
        json!({"id":"fixture","model":"claude-opus-5-5","choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"lookup-1","type":"function","function":{"name":"lookup","arguments":"{}"}}],"reasoning_details":details},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":12,"completion_tokens":4,"total_tokens":16}})
    }

    async fn fixture(details: &Value) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response(details)))
            .mount(&server)
            .await;
        server
    }

    fn snowflake(server: &MockServer) -> SnowflakeProvider {
        SnowflakeProvider {
            api_client: ApiClient::new(server.uri(), AuthMethod::BearerToken("fixture".into()))
                .unwrap(),
            model: ModelConfig::new_or_fail(CORTEX_MODEL),
            image_format: ImageFormat::OpenAi,
            name: "snowflake".into(),
        }
    }

    fn history(assistant: Message) -> Vec<Message> {
        vec![
            Message::user().with_text("lookup"),
            assistant,
            Message::user().with_tool_response(
                "lookup-1",
                Ok(CallToolResult::success(vec![Content::text("found")])),
            ),
        ]
    }

    async fn complete(provider: &impl Provider, history: &[Message]) -> Message {
        provider
            .complete_with_model(&provider.get_model_config(), "system", history, &[])
            .await
            .unwrap()
            .0
    }

    async fn assert_followup(
        provider: &impl Provider,
        server: &MockServer,
        assistant: Message,
        expected: Option<&Value>,
    ) {
        complete(provider, &history(assistant)).await;
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests.last().unwrap().body).unwrap();
        let prior = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["role"] == "assistant")
            .unwrap();
        assert_eq!(prior["tool_calls"][0]["id"], "lookup-1");
        assert_eq!(body["messages"][3]["tool_call_id"], "lookup-1");
        match expected {
            Some(details) => assert_eq!(
                &prior["reasoning_details"], details,
                "same-provider details must survive unchanged"
            ),
            None => assert!(
                prior.get("reasoning_details").is_none(),
                "foreign or ambiguous opaque details must not cross provider origins: {prior}"
            ),
        }
    }

    #[tokio::test]
    async fn hosted_refresh_reasoning_origin_openrouter_to_snowflake_isolated() {
        let details = reasoning_details("openrouter");
        let source_server = fixture(&details).await;
        let source =
            hosted_refresh_reasoning_origin_test_provider(source_server.uri(), OPENROUTER_MODEL);
        let assistant = complete(&source, &[Message::user().with_text("lookup")]).await;
        let destination_server = fixture(&reasoning_details("snowflake")).await;
        assert_followup(
            &snowflake(&destination_server),
            &destination_server,
            assistant,
            None,
        )
        .await;
    }

    #[tokio::test]
    async fn hosted_refresh_reasoning_origin_snowflake_to_openrouter_isolated() {
        let details = reasoning_details("snowflake");
        let source_server = fixture(&details).await;
        let assistant = complete(
            &snowflake(&source_server),
            &[Message::user().with_text("lookup")],
        )
        .await;
        let destination_server = fixture(&reasoning_details("openrouter")).await;
        let destination = hosted_refresh_reasoning_origin_test_provider(
            destination_server.uri(),
            OPENROUTER_MODEL,
        );
        assert_followup(&destination, &destination_server, assistant, None).await;
    }

    #[tokio::test]
    async fn hosted_refresh_reasoning_origin_ambiguous_legacy_is_not_guessed() {
        let details = reasoning_details("openrouter");
        let mut legacy = openrouter_format::response_to_message(&response(&details)).unwrap();
        for content in &mut legacy.content {
            if let MessageContent::ToolRequest(request) = content {
                request
                    .metadata
                    .as_mut()
                    .unwrap()
                    .retain(|key, _| key == openrouter_format::REASONING_DETAILS_KEY);
            }
        }
        let openrouter_server = fixture(&details).await;
        let openrouter = hosted_refresh_reasoning_origin_test_provider(
            openrouter_server.uri(),
            OPENROUTER_MODEL,
        );
        assert_followup(&openrouter, &openrouter_server, legacy.clone(), None).await;
        let snowflake_server = fixture(&reasoning_details("snowflake")).await;
        assert_followup(
            &snowflake(&snowflake_server),
            &snowflake_server,
            legacy,
            None,
        )
        .await;
    }

    #[tokio::test]
    async fn hosted_refresh_reasoning_origin_openrouter_stream_keeps_only_its_own_replay() {
        let details = reasoning_details("openrouter");
        let source_server = MockServer::start().await;
        let first = json!({"id":"stream-fixture","model":OPENROUTER_MODEL,"choices":[{"index":0,"delta":{"role":"assistant","reasoning_details":details,"tool_calls":[{"index":0,"id":"lookup-1","type":"function","function":{"name":"lookup","arguments":"{}"}}]},"finish_reason":null}]});
        let finish = json!({"id":"stream-fixture","model":OPENROUTER_MODEL,"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":12,"completion_tokens":4,"total_tokens":16}});
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(format!(
                        "data: {first}\n\ndata: {finish}\n\ndata: [DONE]\n\n"
                    )),
            )
            .mount(&source_server)
            .await;
        let source =
            hosted_refresh_reasoning_origin_test_provider(source_server.uri(), OPENROUTER_MODEL);
        let mut stream = source
            .stream("system", &[Message::user().with_text("lookup")], &[])
            .await
            .unwrap();
        let mut assistant = None;
        while let Some(item) = stream.next().await {
            if let Some(message) = item.unwrap().0 {
                if message
                    .content
                    .iter()
                    .any(|content| matches!(content, MessageContent::ToolRequest(_)))
                {
                    assistant = Some(message);
                }
            }
        }
        let assistant = assistant.expect("streamed tool request");
        let same_server = fixture(&details).await;
        let same_provider =
            hosted_refresh_reasoning_origin_test_provider(same_server.uri(), OPENROUTER_MODEL);
        assert_followup(
            &same_provider,
            &same_server,
            assistant.clone(),
            Some(&details),
        )
        .await;
        let foreign_server = fixture(&reasoning_details("snowflake")).await;
        assert_followup(
            &snowflake(&foreign_server),
            &foreign_server,
            assistant,
            None,
        )
        .await;
    }
}
