use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::api_client::{ApiClient, AuthMethod};
use super::base::{ConfigKey, Provider, ProviderMetadata, ProviderUsage};
use super::errors::ProviderError;
use super::formats::snowflake::{create_request, get_usage, response_to_message};
use super::retry::ProviderRetry;
use super::utils::{get_model, map_http_error_to_provider_error, ImageFormat, RequestLog};
use crate::config::ConfigError;
use crate::conversation::message::Message;

use crate::model::ModelConfig;
use rmcp::model::Tool;

// Verified against Snowflake's Cortex AI regional-availability page
// (2026-09-25). Removed on 2026-09-25: claude-4-sonnet — Snowflake moved it to
// legacy on 2026-08-12 (only accounts that had already used it can call it; a
// new account's call fails) and ends it on 2026-10-14. Removed earlier:
// claude-4-opus, claude-3-7-sonnet, claude-3-5-sonnet. Not listed:
// claude-fable-5 / -5-1, which Snowflake offers only as a private preview to
// accounts it has enabled.
//
// The default stays on Sonnet 4.6, which Snowflake serves in more regions
// than any newer Claude, so a new account's first chat works wherever it is.
// It is also listed FIRST, ahead of the newer models: the UI auto-selects
// `known_models[0]` when a user switches providers (SwitchModelModal), not
// this constant, and the regional-coverage argument holds for that pick too.
pub const SNOWFLAKE_DEFAULT_MODEL: &str = "claude-sonnet-4-6";
pub const SNOWFLAKE_KNOWN_MODELS: &[&str] = &[
    // Claude Sonnet 4.6 (1M context) — the default, see above.
    SNOWFLAKE_DEFAULT_MODEL,
    // Claude 5 series (1M context). Snowflake caps Sonnet 5's output at 64K,
    // half of what the Claude API allows.
    "claude-sonnet-5",
    "claude-opus-5",
    // Claude Opus 5.5 is a PUBLIC PREVIEW on Snowflake, which calls previews
    // "not suitable for production workloads" — listed for users who want it,
    // never the default. Its thinking blocks are bound to the conversation
    // prefix, but this provider never replays thinking (see
    // `formats::snowflake::format_messages`), so there is nothing to strip.
    "claude-opus-5-5",
    // Claude 4.6+ series (1M context)
    "claude-opus-4-8",
    "claude-opus-4-7",
    "claude-opus-4-6",
    // Claude 4.5 series (200K context)
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

    #[allow(clippy::too_many_lines)]
    async fn post(&self, payload: &Value) -> Result<Value, ProviderError> {
        let response = self
            .api_client
            .response_post("api/v2/cortex/inference:complete", payload)
            .await?;

        let status = response.status();
        let payload_text: String = response.text().await.ok().unwrap_or_default();

        if status.is_success() {
            if let Ok(payload) = serde_json::from_str::<Value>(&payload_text) {
                if payload.get("code").is_some() {
                    let code = payload
                        .get("code")
                        .and_then(|c| c.as_str())
                        .unwrap_or("Unknown code");
                    let message = payload
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("Unknown message");
                    return Err(ProviderError::RequestFailed(format!(
                        "{} - {}",
                        code, message
                    )));
                }
            }
        }

        let lines = payload_text.lines().collect::<Vec<_>>();

        let mut text = String::new();
        let mut tool_name = String::new();
        let mut tool_input = String::new();
        let mut tool_use_id = String::new();
        for line in lines.iter() {
            if line.is_empty() {
                continue;
            }

            let json_str = match line.strip_prefix("data: ") {
                Some(s) => s,
                None => continue,
            };

            if let Ok(json_line) = serde_json::from_str::<Value>(json_str) {
                let choices = match json_line.get("choices").and_then(|c| c.as_array()) {
                    Some(choices) => choices,
                    None => {
                        continue;
                    }
                };

                let choice = match choices.first() {
                    Some(choice) => choice,
                    None => {
                        continue;
                    }
                };

                let delta = match choice.get("delta") {
                    Some(delta) => delta,
                    None => {
                        continue;
                    }
                };

                // Track if we found text in content_list to avoid duplication
                let mut found_text_in_content_list = false;

                // Handle content_list array first
                if let Some(content_list) = delta.get("content_list").and_then(|cl| cl.as_array()) {
                    for content_item in content_list {
                        match content_item.get("type").and_then(|t| t.as_str()) {
                            Some("text") => {
                                if let Some(text_content) =
                                    content_item.get("text").and_then(|t| t.as_str())
                                {
                                    text.push_str(text_content);
                                    found_text_in_content_list = true;
                                }
                            }
                            Some("tool_use") => {
                                if let Some(tool_id) =
                                    content_item.get("tool_use_id").and_then(|id| id.as_str())
                                {
                                    tool_use_id.push_str(tool_id);
                                }
                                if let Some(name) =
                                    content_item.get("name").and_then(|n| n.as_str())
                                {
                                    tool_name.push_str(name);
                                }
                                if let Some(input) =
                                    content_item.get("input").and_then(|i| i.as_str())
                                {
                                    tool_input.push_str(input);
                                }
                            }
                            _ => {
                                // Handle content items without explicit type but with tool information
                                if let Some(name) =
                                    content_item.get("name").and_then(|n| n.as_str())
                                {
                                    tool_name.push_str(name);
                                }
                                if let Some(tool_id) =
                                    content_item.get("tool_use_id").and_then(|id| id.as_str())
                                {
                                    tool_use_id.push_str(tool_id);
                                }
                                if let Some(input) =
                                    content_item.get("input").and_then(|i| i.as_str())
                                {
                                    tool_input.push_str(input);
                                }
                            }
                        }
                    }
                }

                // Handle direct content field (for text) only if we didn't find text in content_list
                if !found_text_in_content_list {
                    if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                        text.push_str(content);
                    }
                }
            }
        }

        // Build the appropriate response structure
        let mut content_list = Vec::new();

        // Add text content if available
        if !text.is_empty() {
            content_list.push(json!({
                "type": "text",
                "text": text
            }));
        }

        // Add tool use content only if we have complete tool information
        if !tool_use_id.is_empty() && !tool_name.is_empty() {
            // Parse tool input as JSON if it's not empty
            let parsed_input = if tool_input.is_empty() {
                json!({})
            } else {
                serde_json::from_str::<Value>(&tool_input)
                    .unwrap_or_else(|_| json!({"raw_input": tool_input}))
            };

            content_list.push(json!({
                "type": "tool_use",
                "tool_use_id": tool_use_id,
                "name": tool_name,
                "input": parsed_input
            }));
        }

        // Ensure we always have at least some content
        if content_list.is_empty() {
            content_list.push(json!({
                "type": "text",
                "text": ""
            }));
        }

        let answer_payload = json!({
            "role": "assistant",
            "content": text,
            "content_list": content_list
        });

        if status.is_success() {
            Ok(answer_payload)
        } else {
            let error_json = serde_json::from_str::<Value>(&payload_text).ok();
            Err(map_http_error_to_provider_error(status, error_json))
        }
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
        let payload = create_request(model_config, system, messages, tools)?;

        let mut log = RequestLog::start(&self.model, &payload)?;

        let response = self
            .with_retry(|| async {
                let payload_clone = payload.clone();
                self.post(&payload_clone).await
            })
            .await?;

        let message = response_to_message(&response)?;
        let usage = get_usage(&response)?;
        let response_model = get_model(&response);

        log.write(&response, Some(&usage))?;

        Ok((message, ProviderUsage::new(response_model, usage)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The UI auto-selects `known_models[0]` on a provider switch, so the
    /// widest-coverage default must be the first entry, not merely declared.
    #[test]
    fn the_default_is_the_first_entry() {
        let metadata = SnowflakeProvider::metadata();
        assert_eq!(metadata.default_model, "claude-sonnet-4-6");
        assert_eq!(metadata.known_models[0].name, metadata.default_model);
    }

    #[test]
    fn catalog_tracks_snowflakes_lifecycle() {
        // Legacy since 2026-08-12 and a new account's call fails; EOL 2026-10-14.
        assert!(!SNOWFLAKE_KNOWN_MODELS.contains(&"claude-4-sonnet"));
        for model in ["claude-sonnet-5", "claude-opus-5", "claude-opus-5-5"] {
            assert!(SNOWFLAKE_KNOWN_MODELS.contains(&model), "{model}");
        }
        // Opus 5.5 is a public preview on Snowflake: offered, never the default.
        assert_ne!(SNOWFLAKE_DEFAULT_MODEL, "claude-opus-5-5");
        // Fable is a private preview there; not offered.
        assert!(!SNOWFLAKE_KNOWN_MODELS
            .iter()
            .any(|model| model.contains("fable")));
    }
}
