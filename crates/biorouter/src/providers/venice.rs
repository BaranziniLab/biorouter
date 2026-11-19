use anyhow::Result;
use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};

use super::api_client::{ApiClient, AuthMethod};
use super::base::{ConfigKey, ModelInfo, Provider, ProviderMetadata, ProviderUsage};
use super::errors::ProviderError;
use super::formats::openai;
use super::retry::ProviderRetry;
use super::utils::{map_http_error_to_provider_error, ImageFormat};
use crate::conversation::message::Message;

use crate::model::ModelConfig;
use rmcp::model::Tool;

// ---------- Capability Flags ----------
#[derive(Debug)]
struct CapabilityFlags(String);

impl CapabilityFlags {
    fn from_json(value: &serde_json::Value) -> Self {
        let caps = &value["model_spec"]["capabilities"];
        let mut s = String::with_capacity(6);
        macro_rules! flag {
            ($json_key:literal, $letter:literal) => {
                if caps
                    .get($json_key)
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    s.push($letter);
                }
            };
        }
        flag!("optimizedForCode", 'c'); // code
        flag!("supportsVision", 'v'); // vision
        flag!("supportsFunctionCalling", 'f');
        flag!("supportsResponseSchema", 's');
        flag!("supportsWebSearch", 'w');
        flag!("supportsReasoning", 'r');
        CapabilityFlags(s)
    }
}

impl std::fmt::Display for CapabilityFlags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}]", self.0) // e.g. "[cvfsw]"
    }
}
// ---------- END Capability Flags ----------

// ---------- Helpers ----------
/// Return the raw model id (everything before the first space).
fn strip_flags(model: &str) -> &str {
    model.split_whitespace().next().unwrap_or(model)
}
// ---------- END Helpers ----------

pub const VENICE_DOC_URL: &str = "https://docs.venice.ai/";
pub const VENICE_DEFAULT_MODEL: &str = "deepseek-v4-1-flash-1017-uncensored";
pub const VENICE_DEFAULT_HOST: &str = "https://api.venice.ai";
pub const VENICE_DEFAULT_BASE_PATH: &str = "api/v1/chat/completions";
pub const VENICE_DEFAULT_MODELS_PATH: &str = "api/v1/models";

// Fallback models to use when API is unavailable
const FALLBACK_MODELS: [&str; 4] = [
    VENICE_DEFAULT_MODEL,
    "llama-3.2-3b",
    "llama-3.3-70b",
    "mistral-small-3-2-24b-instruct",
];

#[derive(Debug, Serialize)]
pub struct VeniceProvider {
    #[serde(skip)]
    api_client: ApiClient,
    base_path: String,
    models_path: String,
    model: ModelConfig,
    #[serde(skip)]
    name: String,
}

impl VeniceProvider {
    pub async fn from_env(mut model: ModelConfig) -> Result<Self> {
        let config = crate::config::Config::global();
        let api_key: String = config.get_secret("VENICE_API_KEY")?;
        let host: String = config
            .get_param("VENICE_HOST")
            .unwrap_or_else(|_| VENICE_DEFAULT_HOST.to_string());
        let base_path: String = config
            .get_param("VENICE_BASE_PATH")
            .unwrap_or_else(|_| VENICE_DEFAULT_BASE_PATH.to_string());
        let models_path: String = config
            .get_param("VENICE_MODELS_PATH")
            .unwrap_or_else(|_| VENICE_DEFAULT_MODELS_PATH.to_string());

        // Ensure we only keep the bare model id internally
        model.model_name = strip_flags(&model.model_name).to_string();

        let auth = AuthMethod::BearerToken(api_key);
        let api_client = ApiClient::new(host, auth)?;

        let instance = Self {
            api_client,
            base_path,
            models_path,
            model,
            name: Self::metadata().name,
        };

        Ok(instance)
    }

    async fn post(&self, path: &str, payload: &Value) -> Result<Value, ProviderError> {
        let response = self.api_client.response_post(path, payload).await?;

        let status = response.status();
        tracing::debug!("Venice response status: {}", status);

        if !status.is_success() {
            // Read response body for more details on error
            let error_body = response.text().await.unwrap_or_default();

            // Log full error response for debugging
            tracing::debug!("Full Venice error response: {}", error_body);

            // Try to parse the error response
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&error_body) {
                // Print the full JSON error for better debugging
                println!(
                    "Venice API error response: {}",
                    serde_json::to_string_pretty(&json).unwrap_or_else(|_| json.to_string())
                );

                // Check for tool support errors
                if let Some(details) = json.get("details") {
                    // Specifically look for tool support issues
                    if let Some(tools) = details.get("tools") {
                        if let Some(errors) = tools.get("_errors") {
                            if errors.to_string().contains("not supported by this model") {
                                let model_name = self.model.model_name.clone();
                                return Err(ProviderError::RequestFailed(
                                    format!("The selected model '{}' does not support tool calls. Please select a model that supports tools, such as 'llama-3.3-70b' or 'mistral-small-3-2-24b-instruct'.", model_name)
                                ));
                            }
                        }
                    }
                }

                // Check for specific error message in context.issues
                if let Some(context) = json.get("context") {
                    if let Some(issues) = context.get("issues") {
                        if let Some(issues_array) = issues.as_array() {
                            for issue in issues_array {
                                if let Some(message) = issue.get("message").and_then(|m| m.as_str())
                                {
                                    if message.contains("tools is not supported by this model") {
                                        let model_name = self.model.model_name.clone();
                                        return Err(ProviderError::RequestFailed(
                                            format!("The selected model '{}' does not support tool calls. Please select a model that supports tools, such as 'llama-3.3-70b' or 'mistral-small-3-2-24b-instruct'.", model_name)
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Use the common error mapping function
            let error_json = serde_json::from_str::<Value>(&error_body).ok();
            return Err(map_http_error_to_provider_error(status, error_json));
        }

        let response_text = response.text().await?;
        serde_json::from_str(&response_text).map_err(|e| {
            ProviderError::RequestFailed(format!(
                "Failed to parse JSON: {}\nResponse: {}",
                e, response_text
            ))
        })
    }
}

#[async_trait]
impl Provider for VeniceProvider {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::with_models(
            "venice",
            "Venice.ai",
            "Venice.ai models (Llama, DeepSeek, Mistral) with function calling",
            VENICE_DEFAULT_MODEL,
            FALLBACK_MODELS
                .iter()
                .map(|model| {
                    if *model == VENICE_DEFAULT_MODEL {
                        ModelInfo::new(*model, 1_000_000).with_vision()
                    } else {
                        ModelInfo::new(*model, ModelConfig::context_window_for(model))
                    }
                })
                .collect(),
            VENICE_DOC_URL,
            vec![
                ConfigKey::new("VENICE_API_KEY", true, true, None),
                ConfigKey::new("VENICE_HOST", true, false, Some(VENICE_DEFAULT_HOST)),
                ConfigKey::new(
                    "VENICE_BASE_PATH",
                    true,
                    false,
                    Some(VENICE_DEFAULT_BASE_PATH),
                ),
                ConfigKey::new(
                    "VENICE_MODELS_PATH",
                    true,
                    false,
                    Some(VENICE_DEFAULT_MODELS_PATH),
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

    async fn fetch_supported_models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        let response = self.api_client.response_get(&self.models_path).await?;
        let json: serde_json::Value = response.json().await?;

        let mut models = json["data"]
            .as_array()
            .ok_or_else(|| ProviderError::RequestFailed("No data field in JSON".to_string()))?
            .iter()
            .filter_map(|model| {
                let id = model["id"].as_str()?.to_owned();
                // Build flags from capabilities
                let flags = CapabilityFlags::from_json(model);
                // Only include models that support function calling (have 'f' flag)
                if flags.0.contains('f') {
                    Some(format!("{id} {flags}"))
                } else {
                    None
                }
            })
            .collect::<Vec<String>>();
        models.sort();
        Ok(Some(models))
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
        let mut config = model_config.clone();
        config.model_name = strip_flags(&config.model_name).to_string();
        let mut payload = openai::create_request(
            &config,
            system,
            messages,
            tools,
            &ImageFormat::OpenAi,
            false,
        )?;
        payload["stream"] = json!(false);
        if config.model_name == VENICE_DEFAULT_MODEL {
            if let Some(effort) = config
                .reasoning_effort
                .and_then(|effort| effort.provider_effort())
            {
                payload["reasoning_effort"] = json!(effort);
            }
        }

        let response = self
            .with_retry(|| self.post(&self.base_path, &payload))
            .await?;
        if !response["choices"][0]["message"].is_object() {
            return Err(ProviderError::RequestFailed(
                "Invalid response format: missing message".to_string(),
            ));
        }
        let mut message = openai::response_to_message(&response)?;
        openai::stamp_reasoning_provenance(&mut message, &self.name);
        let mut usage =
            ProviderUsage::new(config.model_name, openai::get_usage(&response["usage"]));
        usage.finish_reason = response["choices"][0]["finish_reason"]
            .as_str()
            .map(str::to_string);
        Ok((message, usage))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::MessageContent;
    use rmcp::model::{object, CallToolRequestParams, CallToolResult, Content};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn model_refresh_provider(server: &MockServer) -> VeniceProvider {
        let mut model = ModelConfig::new_or_fail("llama-3.3-70b")
            .with_temperature(Some(0.25))
            .with_max_tokens(Some(8_192));
        model.model_name = "deepseek-v4-1-flash-1017-uncensored".to_string();
        VeniceProvider {
            api_client: ApiClient::new(
                server.uri(),
                AuthMethod::BearerToken("synthetic-venice-key".to_string()),
            )
            .unwrap(),
            base_path: VENICE_DEFAULT_BASE_PATH.to_string(),
            models_path: VENICE_DEFAULT_MODELS_PATH.to_string(),
            model,
            name: "venice".to_string(),
        }
    }

    async fn model_refresh_server(message: Value) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": message, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18}
            })))
            .expect(1)
            .mount(&server)
            .await;
        server
    }

    async fn model_refresh_payload(server: &MockServer) -> Value {
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        serde_json::from_slice(&requests[0].body).unwrap()
    }

    #[tokio::test]
    async fn model_refresh_preserves_user_image_blocks() {
        let server =
            model_refresh_server(json!({"role": "assistant", "content": "A chart."})).await;
        let provider = model_refresh_provider(&server);
        provider
            .complete_with_model(
                &provider.model,
                "Read the chart.",
                &[Message::user()
                    .with_text("Describe the result.")
                    .with_image("aW1hZ2U=", "image/png")],
                &[],
            )
            .await
            .unwrap();

        let payload = model_refresh_payload(&server).await;
        let content = payload["messages"][1]["content"]
            .as_array()
            .expect("Venice vision input must retain typed text and image blocks");
        assert!(content.iter().any(|part| part["type"] == "text"));
        assert!(content.iter().any(|part| {
            part["type"] == "image_url"
                && part["image_url"]["url"] == "data:image/png;base64,aW1hZ2U="
        }));
    }

    #[tokio::test]
    async fn model_refresh_honors_configured_output_budget_and_temperature() {
        let server = model_refresh_server(json!({"role": "assistant", "content": "Ready."})).await;
        let provider = model_refresh_provider(&server);
        provider
            .complete_with_model(
                &provider.model,
                "",
                &[Message::user().with_text("Proceed.")],
                &[],
            )
            .await
            .unwrap();

        let payload = model_refresh_payload(&server).await;
        let budget = payload
            .get("max_completion_tokens")
            .or_else(|| payload.get("max_tokens"));
        assert_eq!(budget, Some(&json!(8_192)));
        assert_eq!(payload["temperature"], json!(0.25));
        assert_eq!(payload["model"], VENICE_DEFAULT_MODEL);
        assert!(payload.get("reasoning_effort").is_none());
    }

    #[tokio::test]
    async fn model_refresh_retains_tool_reply_text_reasoning_and_usage() {
        let server = model_refresh_server(json!({
            "role": "assistant",
            "content": "I will inspect both.",
            "reasoning_content": "Compare both sources before answering.",
            "tool_calls": [{
                "id": "lookup-1",
                "type": "function",
                "function": {"name": "lookup", "arguments": "{\"source\":1}"}
            }]
        }))
        .await;
        let provider = model_refresh_provider(&server);
        let (reply, usage) = provider
            .complete_with_model(
                &provider.model,
                "",
                &[Message::user().with_text("Compare.")],
                &[],
            )
            .await
            .unwrap();

        assert_eq!(reply.as_concat_text(), "I will inspect both.");
        let request = reply
            .content
            .iter()
            .find_map(MessageContent::as_tool_request)
            .unwrap();
        assert_eq!(request.id, "lookup-1");
        assert_eq!(
            request.metadata.as_ref().unwrap().get("reasoning_content"),
            Some(&json!("Compare both sources before answering."))
        );
        assert_eq!(usage.usage.input_tokens, Some(11));
        assert_eq!(usage.usage.output_tokens, Some(7));
        assert_eq!(usage.usage.total_tokens, Some(18));
        assert_eq!(usage.finish_reason.as_deref(), Some("stop"));
        assert_eq!(
            request.metadata.as_ref().unwrap().get("reasoning_provider"),
            Some(&json!("venice"))
        );
    }

    #[tokio::test]
    async fn model_refresh_keeps_each_tool_result_in_its_own_message() {
        let server =
            model_refresh_server(json!({"role": "assistant", "content": "Compared."})).await;
        let provider = model_refresh_provider(&server);
        let mut assistant = Message::assistant()
            .with_text("Inspecting both sources.")
            .with_tool_request(
                "lookup-1",
                Ok(CallToolRequestParams {
                    task: None,
                    name: "lookup".into(),
                    arguments: Some(object(json!({"source": 1}))),
                    meta: None,
                }),
            )
            .with_tool_request(
                "lookup-2",
                Ok(CallToolRequestParams {
                    task: None,
                    name: "lookup".into(),
                    arguments: Some(object(json!({"source": 2}))),
                    meta: None,
                }),
            );
        for content in &mut assistant.content {
            if let MessageContent::ToolRequest(request) = content {
                request.metadata = Some(object(json!({
                    "reasoning_content": "Private reasoning from the prior response.",
                    "reasoning_provider": "venice"
                })));
            }
        }
        let results = Message::user()
            .with_tool_response(
                "lookup-1",
                Ok(CallToolResult::success(vec![Content::text("first")])),
            )
            .with_tool_response(
                "lookup-2",
                Ok(CallToolResult::success(vec![Content::text("second")])),
            );
        provider
            .complete_with_model(&provider.model, "", &[assistant, results], &[])
            .await
            .unwrap();

        let payload = model_refresh_payload(&server).await;
        let assistant = payload["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["role"] == "assistant")
            .unwrap();
        assert_eq!(assistant["content"], "Inspecting both sources.");
        assert_eq!(assistant["tool_calls"].as_array().unwrap().len(), 2);
        assert!(assistant.get("reasoning_content").is_none());
        let tool_messages: Vec<_> = payload["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] == "tool")
            .collect();
        assert_eq!(tool_messages.len(), 2);
        assert_eq!(tool_messages[0]["tool_call_id"], "lookup-1");
        assert_eq!(tool_messages[1]["tool_call_id"], "lookup-2");
        assert!(tool_messages[0]["content"].to_string().contains("first"));
        assert!(tool_messages[1]["content"].to_string().contains("second"));
        assert!(tool_messages
            .iter()
            .all(|message| message.get("reasoning_content").is_none()));
    }

    #[test]
    fn test_metadata_structure() {
        let metadata = VeniceProvider::metadata();

        assert_eq!(metadata.default_model, VENICE_DEFAULT_MODEL);
        let default = metadata
            .known_models
            .iter()
            .find(|model| model.name == VENICE_DEFAULT_MODEL)
            .unwrap();
        assert_eq!(default.context_limit, 1_000_000);
        assert_eq!(default.supports_vision, Some(true));
        assert!(metadata
            .known_models
            .iter()
            .any(|model| model.name == "llama-3.3-70b"));

        assert_eq!(metadata.config_keys.len(), 4);
        assert_eq!(metadata.config_keys[0].name, "VENICE_API_KEY");
        assert_eq!(metadata.config_keys[1].name, "VENICE_HOST");
        assert_eq!(metadata.config_keys[2].name, "VENICE_BASE_PATH");
        assert_eq!(metadata.config_keys[3].name, "VENICE_MODELS_PATH");
    }

    /// Venice serves Mistral Small 3.2 at 256k (`/api/v1/models` reports
    /// availableContextTokens 256000, read 2026-09-25). The registry said 128k,
    /// which halved the gauge and compacted early for an advertised model.
    #[test]
    fn mistral_small_gets_venices_256k_window() {
        assert!(FALLBACK_MODELS.contains(&"mistral-small-3-2-24b-instruct"));
        assert_eq!(
            ModelConfig::context_window_for("mistral-small-3-2-24b-instruct"),
            256_000
        );
    }
}
