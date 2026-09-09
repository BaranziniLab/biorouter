use std::collections::HashMap;

use super::base::{ConfigKey, ModelInfo, Provider, ProviderMetadata, ProviderUsage};
use super::errors::ProviderError;
use super::retry::{ProviderRetry, RetryConfig};
use crate::conversation::message::Message;
use crate::model::ModelConfig;
use crate::providers::utils::RequestLog;
use anyhow::Result;
use async_trait::async_trait;
use aws_sdk_bedrockruntime::config::ProvideCredentials;
use aws_sdk_bedrockruntime::{types as bedrock, Client};
use rmcp::model::Tool;
use serde_json::Value;

use aws_sdk_bedrockruntime::operation::converse_stream::ConverseStreamOutput as ConverseStreamResponse;

use super::base::MessageStream;
// Import the migrated helper functions from providers/formats/bedrock.rs
use super::formats::bedrock::{
    bedrock_blocking_inference_config, bedrock_inference_config, bedrock_message_stream,
    classify_bedrock_converse_error, classify_bedrock_converse_stream_error, from_bedrock_message,
    from_bedrock_usage, map_bedrock_stop_reason, to_bedrock_message, to_bedrock_tool_config,
};

pub const BEDROCK_DOC_LINK: &str =
    "https://docs.aws.amazon.com/bedrock/latest/userguide/models-supported.html";

pub const BEDROCK_DEFAULT_MODEL: &str = "us.anthropic.claude-sonnet-4-6";
// Verified against AWS Bedrock model cards + lifecycle page (June 2026).
// Short-form IDs (e.g. claude-sonnet-4-6) are cross-region inference profiles.
// Versioned IDs (v1:0 suffix) are the full inference-profile ARN names.
// Removed: claude-sonnet-4-20250514 (Bedrock Legacy since Apr 2026, retired
// by Anthropic Jun 15, 2026) and claude-opus-4-1 (deprecated, retires Aug
// 2026).
pub const BEDROCK_KNOWN_MODELS: &[&str] = &[
    // Claude Sonnet 5 / Opus 4.8 — newest public Bedrock Claude models
    "us.anthropic.claude-sonnet-5",
    "us.anthropic.claude-opus-4-8",
    // Claude Sonnet 4.6 — latest, preferred default (1M context)
    "us.anthropic.claude-sonnet-4-6",
    // Claude Opus 4.6 (1M context)
    "us.anthropic.claude-opus-4-6-v1",
    // Claude Opus 4.5 (Nov 2025 versioned ID, 200K)
    "us.anthropic.claude-opus-4-5-20251101-v1:0",
    // Claude Sonnet 4.5 (Sep 2025 versioned ID, 200K)
    "us.anthropic.claude-sonnet-4-5-20250929-v1:0",
    // Claude Haiku 4.5 (Oct 2025 versioned ID, 200K)
    "us.anthropic.claude-haiku-4-5-20251001-v1:0",
];

pub const BEDROCK_DEFAULT_MAX_RETRIES: usize = 6;
pub const BEDROCK_DEFAULT_INITIAL_RETRY_INTERVAL_MS: u64 = 2000;
pub const BEDROCK_DEFAULT_BACKOFF_MULTIPLIER: f64 = 2.0;
pub const BEDROCK_DEFAULT_MAX_RETRY_INTERVAL_MS: u64 = 120_000;

#[derive(Debug, serde::Serialize)]
pub struct BedrockProvider {
    #[serde(skip)]
    client: Client,
    model: ModelConfig,
    #[serde(skip)]
    retry_config: RetryConfig,
    #[serde(skip)]
    name: String,
}

impl BedrockProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let config = crate::config::Config::global();

        // Attempt to load config and secrets to get AWS_ prefixed keys
        // to re-export them into the environment for aws_config to use as fallback
        let set_aws_env_vars = |res: Result<HashMap<String, Value>, _>| {
            if let Ok(map) = res {
                map.into_iter()
                    .filter(|(key, _)| key.starts_with("AWS_"))
                    .filter_map(|(key, value)| value.as_str().map(|s| (key, s.to_string())))
                    .for_each(|(key, s)| std::env::set_var(key, s));
            }
        };

        set_aws_env_vars(config.all_values());
        set_aws_env_vars(config.all_secrets());

        // Normalize AWS_ENDPOINT_URL_BEDROCK → AWS_ENDPOINT_URL_BEDROCK_RUNTIME.
        // The AWS SDK for Rust reads the service-specific key AWS_ENDPOINT_URL_BEDROCK_RUNTIME,
        // but users (and older configs) often set the shorter AWS_ENDPOINT_URL_BEDROCK.
        // Accept either: if only the short form is set, promote it to the correct key.
        if std::env::var("AWS_ENDPOINT_URL_BEDROCK_RUNTIME").is_err() {
            if let Ok(url) = std::env::var("AWS_ENDPOINT_URL_BEDROCK") {
                std::env::set_var("AWS_ENDPOINT_URL_BEDROCK_RUNTIME", url);
            }
        }

        // Use load_defaults() which supports AWS SSO, profiles, and environment variables
        let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest());

        if let Ok(profile_name) = config.get_param::<String>("AWS_PROFILE") {
            if !profile_name.is_empty() {
                loader = loader.profile_name(&profile_name);
            }
        }

        // Check for AWS_REGION configuration
        if let Ok(region) = config.get_param::<String>("AWS_REGION") {
            if !region.is_empty() {
                loader = loader.region(aws_config::Region::new(region));
            }
        }

        // Bound a hung/stalled endpoint so a turn can't wait forever (see
        // `bedrock_timeout_config`). Without this, an endpoint that accepts the
        // connection but never answers freezes the agent with no error.
        if let Some(timeout_config) = super::formats::bedrock::bedrock_timeout_config(config) {
            loader = loader.timeout_config(timeout_config);
        }

        let sdk_config = loader.load().await;

        // Validate credentials or return error back up
        sdk_config
            .credentials_provider()
            .ok_or_else(|| anyhow::anyhow!("No AWS credentials provider configured"))?
            .provide_credentials()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to load AWS credentials: {}. Make sure to run 'aws sso login --profile <your-profile>' if using SSO", e))?;

        let client = Client::new(&sdk_config);
        let retry_config = Self::load_retry_config(config);

        Ok(Self {
            client,
            model,
            retry_config,
            name: Self::metadata().name,
        })
    }

    fn load_retry_config(config: &crate::config::Config) -> RetryConfig {
        let max_retries = config
            .get_param::<usize>("BEDROCK_MAX_RETRIES")
            .unwrap_or(BEDROCK_DEFAULT_MAX_RETRIES);

        let initial_interval_ms = config
            .get_param::<u64>("BEDROCK_INITIAL_RETRY_INTERVAL_MS")
            .unwrap_or(BEDROCK_DEFAULT_INITIAL_RETRY_INTERVAL_MS);

        let backoff_multiplier = config
            .get_param::<f64>("BEDROCK_BACKOFF_MULTIPLIER")
            .unwrap_or(BEDROCK_DEFAULT_BACKOFF_MULTIPLIER);

        let max_interval_ms = config
            .get_param::<u64>("BEDROCK_MAX_RETRY_INTERVAL_MS")
            .unwrap_or(BEDROCK_DEFAULT_MAX_RETRY_INTERVAL_MS);

        RetryConfig {
            max_retries,
            initial_interval_ms,
            backoff_multiplier,
            max_interval_ms,
        }
    }

    async fn converse(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(bedrock::Message, Option<bedrock::TokenUsage>, String), ProviderError> {
        let model_name = &model_config.model_name;

        let mut request = self
            .client
            .converse()
            .system(bedrock::SystemContentBlock::Text(system.to_string()))
            .model_id(model_name.to_string())
            .inference_config(bedrock_blocking_inference_config(model_config))
            .set_messages(Some(
                messages
                    .iter()
                    .filter(|m| m.is_agent_visible())
                    .map(to_bedrock_message)
                    .collect::<Result<_>>()?,
            ));

        if !tools.is_empty() {
            request = request.tool_config(to_bedrock_tool_config(tools)?);
        }

        let response = request
            .send()
            .await
            .map_err(classify_bedrock_converse_error)?;

        let finish_reason = map_bedrock_stop_reason(&response.stop_reason);
        match response.output {
            Some(bedrock::ConverseOutput::Message(message)) => {
                Ok((message, response.usage, finish_reason))
            }
            _ => Err(ProviderError::RequestFailed(
                "No output from Bedrock".to_string(),
            )),
        }
    }

    /// Open a `ConverseStream` response. Mirrors [`Self::converse`] exactly —
    /// same system prompt, messages and tool config — so the streaming and
    /// blocking paths cannot drift in what they send.
    async fn converse_stream(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<ConverseStreamResponse, ProviderError> {
        let mut request = self
            .client
            .converse_stream()
            .system(bedrock::SystemContentBlock::Text(system.to_string()))
            .model_id(model_config.model_name.clone())
            .inference_config(bedrock_inference_config(model_config))
            .set_messages(Some(
                messages
                    .iter()
                    .filter(|m| m.is_agent_visible())
                    .map(to_bedrock_message)
                    .collect::<Result<_>>()?,
            ));

        if !tools.is_empty() {
            request = request.tool_config(to_bedrock_tool_config(tools)?);
        }

        request
            .send()
            .await
            .map_err(classify_bedrock_converse_stream_error)
    }
}

#[async_trait]
impl Provider for BedrockProvider {
    fn metadata() -> ProviderMetadata {
        // All listed Bedrock models are Claude variants (Sonnet 4.x, Opus 4.x), all vision-capable.
        let models: Vec<ModelInfo> = BEDROCK_KNOWN_MODELS
            .iter()
            .map(|&name| {
                ModelInfo::new(name, ModelConfig::new_or_fail(name).context_limit()).with_vision()
            })
            .collect();

        ProviderMetadata::with_models(
            "aws_bedrock",
            "Amazon Bedrock",
            "Run models through Amazon Bedrock.",
            BEDROCK_DEFAULT_MODEL,
            models,
            BEDROCK_DOC_LINK,
            vec![
                ConfigKey::new("AWS_PROFILE", true, false, Some("default")),
                ConfigKey::new("AWS_REGION", true, false, Some("us-west-2")),
            ],
        )
        .with_unlisted_models()
    }

    fn get_name(&self) -> &str {
        &self.name
    }

    fn retry_config(&self) -> RetryConfig {
        self.retry_config.clone()
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
        let model_name = model_config.model_name.clone();

        let debug_payload = serde_json::json!({
            "system": system,
            "messages": messages,
            "tools": tools
        });
        let mut log = RequestLog::start(&self.model, &debug_payload)?;

        let (bedrock_message, bedrock_usage, finish_reason) = self
            .with_retry(|| self.converse(model_config, system, messages, tools))
            .await
            .inspect_err(|e| {
                let _ = log.error(e);
            })?;

        let usage = bedrock_usage
            .as_ref()
            .map(from_bedrock_usage)
            .unwrap_or_default();

        let message = from_bedrock_message(&bedrock_message)?;

        log.write(
            &serde_json::to_value(&message).unwrap_or_default(),
            Some(&usage),
        )?;

        let mut provider_usage = ProviderUsage::new(model_name.to_string(), usage);
        provider_usage.finish_reason = Some(finish_reason);
        Ok((message, provider_usage))
    }

    /// Stream a turn via Bedrock `ConverseStream`.
    ///
    /// Only opening the stream is retried (via `with_retry`, so the existing
    /// Bedrock retry budget and error classification are preserved). Once events
    /// start arriving, a failure is terminal: partial output has already reached
    /// the agent and replaying the request would duplicate it.
    async fn stream(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let model_name = self.model.model_name.clone();

        let debug_payload = serde_json::json!({
            "system": system,
            "messages": messages,
            "tools": tools,
            "stream": true
        });
        let mut log = RequestLog::start(&self.model, &debug_payload)?;

        let response = self
            .with_retry(|| self.converse_stream(&self.model, system, messages, tools))
            .await
            .inspect_err(|e| {
                let _ = log.error(e);
            })?;

        Ok(bedrock_message_stream(response, model_name, log))
    }

    fn supports_streaming(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::extension::ExtensionConfig;
    use crate::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
    use crate::config::permission::PermissionManager;
    use crate::config::BioRouterMode;
    use crate::conversation::message::MessageContent;
    use crate::providers::formats::bedrock::BedrockStreamDecoder;
    use crate::session::session_manager::SessionType;
    use crate::session::SessionManager;
    use aws_sdk_bedrockruntime::config::Credentials;
    use aws_smithy_http_client::test_util::capture_request;
    use futures::StreamExt;
    use rmcp::model::CallToolRequestParams;
    use rmcp::object;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    async fn capturing_provider() -> (
        BedrockProvider,
        aws_smithy_http_client::test_util::CaptureRequestReceiver,
    ) {
        let (http_client, captured) = capture_request(None);
        let sdk_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .credentials_provider(Credentials::new(
                "test-access-key",
                "test-secret-key",
                None,
                None,
                "BedrockWireTest",
            ))
            .region(aws_config::Region::new("us-west-2"))
            .endpoint_url("https://bedrock.invalid")
            .http_client(http_client)
            .load()
            .await;
        (
            BedrockProvider {
                client: Client::new(&sdk_config),
                model: ModelConfig::new_or_fail(BEDROCK_DEFAULT_MODEL),
                retry_config: RetryConfig::new(0, 0, 1.0, 0),
                name: "aws_bedrock".to_string(),
            },
            captured,
        )
    }

    fn captured_json(
        captured: aws_smithy_http_client::test_util::CaptureRequestReceiver,
    ) -> serde_json::Value {
        let request = captured.expect_request();
        serde_json::from_slice(request.body().bytes().expect("buffered request body"))
            .expect("JSON request body")
    }

    struct SignedToolReplayProvider {
        calls: Mutex<Vec<Vec<Message>>>,
    }

    struct ReplacementCaptureProvider {
        calls: Mutex<Vec<Vec<Message>>>,
    }

    struct DecoderDrivenReplayProvider {
        calls: Mutex<Vec<Vec<Message>>>,
    }

    struct SignedFrontendReplayProvider {
        calls: Mutex<Vec<Vec<Message>>>,
    }

    impl DecoderDrivenReplayProvider {
        fn reasoning_delta(
            index: i32,
            delta: bedrock::ReasoningContentBlockDelta,
        ) -> bedrock::ConverseStreamOutput {
            bedrock::ConverseStreamOutput::ContentBlockDelta(
                bedrock::ContentBlockDeltaEvent::builder()
                    .content_block_index(index)
                    .delta(bedrock::ContentBlockDelta::ReasoningContent(delta))
                    .build()
                    .unwrap(),
            )
        }

        fn text_delta(index: i32, text: &str) -> bedrock::ConverseStreamOutput {
            bedrock::ConverseStreamOutput::ContentBlockDelta(
                bedrock::ContentBlockDeltaEvent::builder()
                    .content_block_index(index)
                    .delta(bedrock::ContentBlockDelta::Text(text.to_string()))
                    .build()
                    .unwrap(),
            )
        }

        fn tool_start(index: i32, id: &str) -> bedrock::ConverseStreamOutput {
            bedrock::ConverseStreamOutput::ContentBlockStart(
                bedrock::ContentBlockStartEvent::builder()
                    .content_block_index(index)
                    .start(bedrock::ContentBlockStart::ToolUse(
                        bedrock::ToolUseBlockStart::builder()
                            .tool_use_id(id)
                            .name("developer__shell")
                            .build()
                            .unwrap(),
                    ))
                    .build()
                    .unwrap(),
            )
        }

        fn tool_delta(index: i32, input: &str) -> bedrock::ConverseStreamOutput {
            bedrock::ConverseStreamOutput::ContentBlockDelta(
                bedrock::ContentBlockDeltaEvent::builder()
                    .content_block_index(index)
                    .delta(bedrock::ContentBlockDelta::ToolUse(
                        bedrock::ToolUseBlockDelta::builder()
                            .input(input)
                            .build()
                            .unwrap(),
                    ))
                    .build()
                    .unwrap(),
            )
        }

        fn block_stop(index: i32) -> bedrock::ConverseStreamOutput {
            bedrock::ConverseStreamOutput::ContentBlockStop(
                bedrock::ContentBlockStopEvent::builder()
                    .content_block_index(index)
                    .build()
                    .unwrap(),
            )
        }

        fn first_stream() -> MessageStream {
            let events = vec![
                Self::reasoning_delta(
                    0,
                    bedrock::ReasoningContentBlockDelta::Text("stream-think-a".to_string()),
                ),
                Self::reasoning_delta(
                    0,
                    bedrock::ReasoningContentBlockDelta::Text("-b".to_string()),
                ),
                Self::reasoning_delta(
                    0,
                    bedrock::ReasoningContentBlockDelta::Signature("stream-signature".to_string()),
                ),
                Self::block_stop(0),
                Self::reasoning_delta(
                    1,
                    bedrock::ReasoningContentBlockDelta::RedactedContent(
                        aws_smithy_types::Blob::new(b"opaque-stream-reasoning"),
                    ),
                ),
                Self::block_stop(1),
                Self::text_delta(2, "split "),
                Self::text_delta(2, "text  "),
                Self::block_stop(2),
                Self::tool_start(3, "stream-tool-a"),
                Self::tool_delta(3, r#"{"command":"printf stream-a"}"#),
                Self::block_stop(3),
                Self::tool_start(4, "stream-tool-b"),
                Self::tool_delta(4, r#"{"command":"printf stream-b"}"#),
                Self::block_stop(4),
                bedrock::ConverseStreamOutput::MessageStop(
                    bedrock::MessageStopEvent::builder()
                        .stop_reason(bedrock::StopReason::ToolUse)
                        .build()
                        .unwrap(),
                ),
            ];
            let mut decoder = BedrockStreamDecoder::new(BEDROCK_DEFAULT_MODEL);
            let mut decoded = events
                .iter()
                .flat_map(|event| decoder.on_event(event))
                .collect::<Vec<_>>();
            decoded.extend(decoder.finish());
            let items = decoded
                .into_iter()
                .map(|(message, usage)| Ok((message, usage, None)));
            Box::pin(futures::stream::iter(items.collect::<Vec<_>>()))
        }
    }

    #[async_trait]
    impl Provider for DecoderDrivenReplayProvider {
        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system_prompt: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            Err(ProviderError::NotImplemented(
                "decoder test provider is streaming-only".to_string(),
            ))
        }

        async fn stream(
            &self,
            _system: &str,
            messages: &[Message],
            _tools: &[Tool],
        ) -> Result<MessageStream, ProviderError> {
            let call = {
                let mut calls = self.calls.lock().unwrap();
                let call = calls.len();
                calls.push(messages.to_vec());
                call
            };
            if call == 0 {
                return Ok(Self::first_stream());
            }
            let mut usage = ProviderUsage::new(
                BEDROCK_DEFAULT_MODEL.to_string(),
                crate::providers::base::Usage::new(Some(10), Some(5), Some(15)),
            );
            usage.finish_reason = Some("stop".to_string());
            Ok(crate::providers::base::stream_from_single_message(
                Message::assistant().with_text("stream replay done"),
                usage,
            ))
        }

        fn supports_streaming(&self) -> bool {
            true
        }

        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new_or_fail(BEDROCK_DEFAULT_MODEL)
        }

        fn metadata() -> ProviderMetadata {
            BedrockProvider::metadata()
        }

        fn get_name(&self) -> &str {
            "aws_bedrock"
        }
    }

    #[async_trait]
    impl Provider for ReplacementCaptureProvider {
        async fn complete(
            &self,
            _system_prompt: &str,
            messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            self.calls.lock().unwrap().push(messages.to_vec());
            let mut usage = ProviderUsage::new(
                BEDROCK_DEFAULT_MODEL.to_string(),
                crate::providers::base::Usage::new(Some(10), Some(5), Some(15)),
            );
            usage.finish_reason = Some("stop".to_string());
            Ok((Message::assistant().with_text("replacement done"), usage))
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            system_prompt: &str,
            messages: &[Message],
            tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            self.complete(system_prompt, messages, tools).await
        }

        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new_or_fail(BEDROCK_DEFAULT_MODEL)
        }

        fn metadata() -> ProviderMetadata {
            BedrockProvider::metadata()
        }

        fn get_name(&self) -> &str {
            "replacement-provider"
        }
    }

    #[async_trait]
    impl Provider for SignedFrontendReplayProvider {
        async fn complete(
            &self,
            _system_prompt: &str,
            messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            let call = {
                let mut calls = self.calls.lock().unwrap();
                let call = calls.len();
                calls.push(messages.to_vec());
                call
            };
            let mut usage = ProviderUsage::new(
                BEDROCK_DEFAULT_MODEL.to_string(),
                crate::providers::base::Usage::new(Some(10), Some(5), Some(15)),
            );
            if call == 0 {
                usage.finish_reason = Some("tool_calls".to_string());
                Ok((
                    Message::assistant()
                        .with_thinking("frontend reasoning", "frontend-signature")
                        .with_tool_request(
                            "frontend-call",
                            Ok(CallToolRequestParams {
                                task: None,
                                meta: None,
                                name: "frontend__pick".into(),
                                arguments: Some(object!({"count": "7"})),
                            }),
                        ),
                    usage,
                ))
            } else {
                usage.finish_reason = Some("stop".to_string());
                Ok((
                    Message::assistant()
                        .with_thinking("done", "frontend-final-signature")
                        .with_text("done"),
                    usage,
                ))
            }
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            system_prompt: &str,
            messages: &[Message],
            tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            self.complete(system_prompt, messages, tools).await
        }

        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new_or_fail(BEDROCK_DEFAULT_MODEL)
        }

        fn metadata() -> ProviderMetadata {
            BedrockProvider::metadata()
        }

        fn get_name(&self) -> &str {
            "aws_bedrock"
        }
    }

    impl SignedToolReplayProvider {
        fn first_response() -> Message {
            Message::assistant()
                .with_thinking("signed reasoning", "signature-one")
                .with_text("answer with trailing whitespace  ")
                .with_tool_request(
                    "tool-a",
                    Ok(CallToolRequestParams {
                        task: None,
                        meta: None,
                        name: "developer__shell".into(),
                        arguments: Some(object!({"command": 7})),
                    }),
                )
                .with_tool_request(
                    "tool-b",
                    Ok(CallToolRequestParams {
                        task: None,
                        meta: None,
                        name: "developer__shell".into(),
                        arguments: Some(object!({"command": "provider-original"})),
                    }),
                )
        }

        fn final_response() -> Message {
            Message::assistant()
                .with_thinking("final reasoning", "signature-two")
                .with_text("done")
        }
    }

    #[async_trait]
    impl Provider for SignedToolReplayProvider {
        async fn complete(
            &self,
            _system_prompt: &str,
            messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            let call = {
                let mut calls = self.calls.lock().unwrap();
                let call = calls.len();
                calls.push(messages.to_vec());
                call
            };
            let mut usage = ProviderUsage::new(
                BEDROCK_DEFAULT_MODEL.to_string(),
                crate::providers::base::Usage::new(Some(10), Some(5), Some(15)),
            );
            if call == 0 {
                usage.finish_reason = Some("tool_calls".to_string());
                Ok((Self::first_response(), usage))
            } else {
                usage.finish_reason = Some("stop".to_string());
                Ok((Self::final_response(), usage))
            }
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            system_prompt: &str,
            messages: &[Message],
            tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            self.complete(system_prompt, messages, tools).await
        }

        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new_or_fail(BEDROCK_DEFAULT_MODEL)
        }

        fn metadata() -> ProviderMetadata {
            BedrockProvider::metadata()
        }

        fn get_name(&self) -> &str {
            "aws_bedrock"
        }
    }

    fn assert_inference_wire(
        captured: aws_smithy_http_client::test_util::CaptureRequestReceiver,
        expected_tokens: i64,
        expected_temperature: Option<f64>,
    ) {
        let request = captured.expect_request();
        let body = request.body().bytes().expect("buffered request body");
        let json: serde_json::Value = serde_json::from_slice(body).expect("JSON request body");
        assert_eq!(json["inferenceConfig"]["maxTokens"], expected_tokens);
        assert_eq!(
            json["inferenceConfig"]["temperature"],
            expected_temperature.map_or(serde_json::Value::Null, serde_json::Value::from)
        );
    }

    #[tokio::test]
    async fn converse_sends_configured_inference_fields_on_the_wire() {
        let (provider, captured) = capturing_provider().await;
        let config = ModelConfig::new_or_fail("us.anthropic.claude-sonnet-4-6")
            .with_max_tokens(Some(34_567))
            .with_temperature(Some(0.25));
        let _ = provider
            .converse(
                &config,
                "system",
                &[Message::user().with_text("hello")],
                &[],
            )
            .await;
        assert_inference_wire(captured, 21_333, Some(0.25));
    }

    #[tokio::test]
    async fn converse_uses_transport_safe_default_on_the_wire() {
        let (provider, captured) = capturing_provider().await;
        let config = ModelConfig::new_or_fail("us.anthropic.claude-sonnet-4-6");
        let _ = provider
            .converse(
                &config,
                "system",
                &[Message::user().with_text("hello")],
                &[],
            )
            .await;
        assert_inference_wire(captured, 21_333, None);
    }

    #[tokio::test]
    async fn converse_stream_sends_configured_inference_fields_on_the_wire() {
        let (provider, captured) = capturing_provider().await;
        let config = ModelConfig::new_or_fail("us.anthropic.claude-sonnet-4-6")
            .with_max_tokens(Some(23_456))
            .with_temperature(Some(0.5));
        let _ = provider
            .converse_stream(
                &config,
                "system",
                &[Message::user().with_text("hello")],
                &[],
            )
            .await;
        assert_inference_wire(captured, 23_456, Some(0.5));
    }

    #[tokio::test]
    async fn converse_stream_keeps_large_model_default_on_the_wire() {
        let (provider, captured) = capturing_provider().await;
        let config = ModelConfig::new_or_fail("us.anthropic.claude-sonnet-4-6");
        let _ = provider
            .converse_stream(
                &config,
                "system",
                &[Message::user().with_text("hello")],
                &[],
            )
            .await;
        assert_inference_wire(captured, 64_000, None);
    }

    #[tokio::test]
    async fn signed_frontend_coercion_replays_original_and_audits_executed_arguments() {
        let work = TempDir::new().unwrap();
        let data = TempDir::new().unwrap();
        let permissions = TempDir::new().unwrap();
        let manager = Arc::new(SessionManager::new(data.path().to_path_buf()));
        let agent = Agent::with_config(AgentConfig::new(
            manager.clone(),
            Arc::new(PermissionManager::new(permissions.path().to_path_buf())),
            None,
            BioRouterMode::Auto,
        ));
        agent
            .add_extension(ExtensionConfig::Frontend {
                name: "frontend".to_string(),
                description: "frontend".to_string(),
                tools: vec![Tool::new(
                    "frontend__pick".to_string(),
                    "pick".to_string(),
                    object!({
                        "type": "object",
                        "properties": {"count": {"type": "integer"}}
                    }),
                )],
                instructions: None,
                bundled: None,
                available_tools: vec![],
            })
            .await
            .unwrap();
        let session = manager
            .create_session(
                work.path().to_path_buf(),
                "frontend signed".to_string(),
                SessionType::Hidden,
            )
            .await
            .unwrap();
        let provider = Arc::new(SignedFrontendReplayProvider {
            calls: Mutex::new(Vec::new()),
        });
        agent
            .update_provider(provider.clone(), &session.id)
            .await
            .unwrap();
        let stream = agent
            .reply(
                Message::user().with_text("pick"),
                SessionConfig {
                    id: session.id.clone(),
                    schedule_id: None,
                    max_turns: Some(5),
                    max_tool_calls: Some(5),
                    budget: None,
                    retry_config: None,
                    reasoning_effort: None,
                },
                None,
            )
            .await
            .unwrap();
        tokio::pin!(stream);
        let mut saw_coerced_frontend_request = false;
        while let Some(event) = stream.next().await {
            let AgentEvent::Message(message) = event.unwrap() else {
                continue;
            };
            for content in &message.content {
                let MessageContent::FrontendToolRequest(request) = content else {
                    continue;
                };
                let call = request.tool_call.as_ref().unwrap();
                assert_eq!(call.arguments.as_ref().unwrap()["count"], 7);
                saw_coerced_frontend_request = true;
                agent
                    .handle_tool_result(
                        request.id.clone(),
                        Ok(rmcp::model::CallToolResult {
                            content: vec![rmcp::model::Content::text("picked")],
                            structured_content: None,
                            is_error: Some(false),
                            meta: None,
                        }),
                    )
                    .await;
            }
        }
        assert!(saw_coerced_frontend_request);
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        let replayed = calls[1]
            .iter()
            .flat_map(|message| message.content.iter())
            .find_map(|content| match content {
                MessageContent::ToolRequest(request) if request.id == "frontend-call" => {
                    Some(request.tool_call.as_ref().unwrap())
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(replayed.arguments.as_ref().unwrap()["count"], "7");
        let audit = calls[1]
            .iter()
            .flat_map(|message| message.content.iter())
            .find_map(|content| match content {
                MessageContent::ToolResponse(response) => response
                    .tool_result
                    .as_ref()
                    .ok()
                    .and_then(|result| result.meta.as_ref())
                    .and_then(|meta| meta.0.get("biorouterToolExecution")),
                _ => None,
            })
            .unwrap();
        assert_eq!(audit["providerAuthored"]["arguments"]["count"], "7");
        assert_eq!(audit["actuallyExecuted"]["arguments"]["count"], 7);
    }

    #[tokio::test]
    async fn decoder_stream_round_trips_exact_signed_block_sequence_through_agent_and_wire() {
        let work = TempDir::new().unwrap();
        let data = TempDir::new().unwrap();
        let permissions = TempDir::new().unwrap();
        let manager = Arc::new(SessionManager::new(data.path().to_path_buf()));
        let agent = Agent::with_config(AgentConfig::new(
            manager.clone(),
            Arc::new(PermissionManager::new(permissions.path().to_path_buf())),
            None,
            BioRouterMode::Auto,
        ));
        agent
            .add_extension(ExtensionConfig::Builtin {
                name: "developer".to_string(),
                description: "developer".to_string(),
                display_name: Some("Developer".to_string()),
                timeout: Some(30),
                bundled: Some(true),
                available_tools: vec![],
            })
            .await
            .unwrap();
        let session = manager
            .create_session(
                work.path().to_path_buf(),
                "decoder replay".to_string(),
                SessionType::Hidden,
            )
            .await
            .unwrap();
        let scripted = Arc::new(DecoderDrivenReplayProvider {
            calls: Mutex::new(Vec::new()),
        });
        agent
            .update_provider(scripted.clone(), &session.id)
            .await
            .unwrap();
        let stream = agent
            .reply(
                Message::user().with_text("run streamed tools"),
                SessionConfig {
                    id: session.id.clone(),
                    schedule_id: None,
                    max_turns: Some(6),
                    max_tool_calls: Some(6),
                    budget: None,
                    retry_config: None,
                    reasoning_effort: None,
                },
                None,
            )
            .await
            .unwrap();
        tokio::pin!(stream);
        while let Some(event) = stream.next().await {
            assert!(!matches!(event.unwrap(), AgentEvent::TurnAborted { .. }));
        }

        let calls = scripted.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 2);
        let signed_index = calls[0].len();
        let signed = &calls[1][signed_index];
        assert_eq!(signed.content.len(), 5);
        let MessageContent::Thinking(thinking) = &signed.content[0] else {
            panic!("reasoning must remain first");
        };
        assert_eq!(thinking.thinking, "stream-think-a-b");
        assert_eq!(thinking.signature, "stream-signature");
        let MessageContent::RedactedThinking(redacted) = &signed.content[1] else {
            panic!("redacted reasoning must remain second");
        };
        assert_eq!(redacted.data, "b3BhcXVlLXN0cmVhbS1yZWFzb25pbmc=");
        let MessageContent::Text(text) = &signed.content[2] else {
            panic!("split text deltas must reconstruct one text block");
        };
        assert_eq!(text.text, "split text  ");
        let tool_ids = signed.content[3..]
            .iter()
            .map(|content| match content {
                MessageContent::ToolRequest(request) => request.id.as_str(),
                _ => panic!("tools must remain after the signed content"),
            })
            .collect::<Vec<_>>();
        assert_eq!(tool_ids, ["stream-tool-a", "stream-tool-b"]);
        assert_eq!(
            calls[1][signed_index + 1]
                .content
                .iter()
                .filter(|content| matches!(content, MessageContent::ToolResponse(_)))
                .count(),
            2
        );

        let stored = manager
            .get_session(&session.id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        let durable_signed = stored
            .iter()
            .find(|message| {
                message.content.iter().any(
                    |content| matches!(content, MessageContent::Thinking(value) if value.signature == "stream-signature"),
                )
            })
            .unwrap();
        assert_eq!(
            serde_json::to_value(&durable_signed.content).unwrap(),
            serde_json::to_value(&signed.content).unwrap()
        );

        let (wire_provider, captured) = capturing_provider().await;
        let _ = wire_provider
            .converse(
                &ModelConfig::new_or_fail(BEDROCK_DEFAULT_MODEL),
                "system",
                &calls[1],
                &[],
            )
            .await;
        let wire = captured_json(captured);
        let signed_wire = wire["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| {
                message["content"].as_array().is_some_and(|blocks| {
                    blocks.iter().any(|block| {
                        block["reasoningContent"]["reasoningText"]["signature"]
                            == "stream-signature"
                    })
                })
            })
            .unwrap();
        let blocks = signed_wire["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 5);
        assert_eq!(
            blocks[0]["reasoningContent"]["reasoningText"]["text"],
            "stream-think-a-b"
        );
        assert_eq!(
            blocks[1]["reasoningContent"]["redactedContent"],
            "b3BhcXVlLXN0cmVhbS1yZWFzb25pbmc="
        );
        assert_eq!(blocks[2]["text"], "split text  ");
        assert_eq!(blocks[3]["toolUse"]["toolUseId"], "stream-tool-a");
        assert_eq!(blocks[4]["toolUse"]["toolUseId"], "stream-tool-b");
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn signed_multi_tool_replay_is_exact_in_reply_and_omitted_after_reload() {
        // The project-hooks decision is stated on this agent's config below.
        // It used to be `set_var("BIOROUTER_ALLOW_PROJECT_HOOKS", "1")` with no
        // matching remove, so the flag stood for every agent constructed in
        // the binary afterwards — and an unkeyed `#[serial]` fenced it against
        // only the other unkeyed tests.
        let work = TempDir::new().unwrap();
        let data = TempDir::new().unwrap();
        let permissions = TempDir::new().unwrap();
        std::fs::create_dir_all(work.path().join(".biorouter")).unwrap();
        // The hook body is emitted by the platform shell — `sh -c` on unix,
        // `cmd.exe /D /S /C` on Windows (see hooks/command_runner.rs). A single
        // quote is a QUOTING character in sh and a LITERAL in cmd, so one
        // command string cannot serve both: the sh form echoes its apostrophes
        // into the JSON under cmd and the parse fails, taking `additionalContext`
        // with it. cmd's `echo` preserves double quotes (the runner passes its
        // /C payload verbatim for exactly this reason), so the unquoted form is
        // the correct one there.
        //
        // This is the only hook-driven test that lives in the LIB rather than in
        // crates/biorouter/tests/, and Windows CI runs `cargo test --workspace
        // --lib --bins` — so it is the only one Windows actually executes. Keep
        // any new hook command here portable, or it will pass locally on macOS
        // and fail only on Windows CI.
        // Same split as `json_stdout_command` in hooks/command_runner.rs's tests,
        // which is the existing precedent for emitting JSON from a hook on both
        // platforms.
        fn echo_json(json: &str) -> String {
            if cfg!(target_os = "windows") {
                format!("echo {json}")
            } else {
                format!("echo '{json}'")
            }
        }
        let hooks_file = serde_json::json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "developer__shell",
                    "hooks": [{
                        "type": "command",
                        "command": echo_json(
                            r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","updatedInput":{"command":"printf rewritten-by-hook"}}}"#
                        ),
                    }],
                }],
                "PostToolUse": [{
                    "matcher": "developer__shell",
                    "hooks": [{
                        "type": "command",
                        "command": echo_json(
                            r#"{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"post-tool-audit-context"}}"#
                        ),
                    }],
                }],
            }
        });
        // YAML is a superset of JSON and the loader is serde_yaml, so writing
        // JSON here sidesteps a second layer of quoting inside the file itself.
        std::fs::write(
            work.path().join(".biorouter/hooks.yaml"),
            serde_json::to_string_pretty(&hooks_file).unwrap(),
        )
        .unwrap();

        let manager = Arc::new(SessionManager::new(data.path().to_path_buf()));
        let agent = Agent::with_config(
            AgentConfig::new(
                manager.clone(),
                Arc::new(PermissionManager::new(permissions.path().to_path_buf())),
                None,
                BioRouterMode::Auto,
            )
            .with_project_hooks(true),
        );
        agent
            .add_extension(ExtensionConfig::Builtin {
                name: "developer".to_string(),
                description: "developer".to_string(),
                display_name: Some("Developer".to_string()),
                timeout: Some(30),
                bundled: Some(true),
                available_tools: vec![],
            })
            .await
            .unwrap();
        let session = manager
            .create_session(
                work.path().to_path_buf(),
                "signed replay".to_string(),
                SessionType::Hidden,
            )
            .await
            .unwrap();
        let scripted = Arc::new(SignedToolReplayProvider {
            calls: Mutex::new(Vec::new()),
        });
        let replacement = Arc::new(ReplacementCaptureProvider {
            calls: Mutex::new(Vec::new()),
        });
        agent
            .update_provider(scripted.clone(), &session.id)
            .await
            .unwrap();
        let stream = agent
            .reply(
                Message::user().with_text("run both"),
                SessionConfig {
                    id: session.id.clone(),
                    schedule_id: None,
                    max_turns: Some(8),
                    max_tool_calls: Some(8),
                    budget: None,
                    retry_config: None,
                    reasoning_effort: None,
                },
                None,
            )
            .await
            .unwrap();
        tokio::pin!(stream);
        let mut provider_swapped = false;
        while let Some(event) = stream.next().await {
            let event = event.unwrap();
            assert!(!matches!(event, AgentEvent::TurnAborted { .. }));
            if !provider_swapped
                && matches!(
                    &event,
                    AgentEvent::Message(message)
                        if message.content.iter().any(
                            |content| matches!(content, MessageContent::Thinking(thinking) if thinking.signature == "signature-one")
                        )
                )
            {
                agent
                    .update_provider(replacement.clone(), &session.id)
                    .await
                    .unwrap();
                provider_swapped = true;
            }
        }
        assert!(provider_swapped);
        assert!(replacement.calls.lock().unwrap().is_empty());

        let calls = scripted.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 2);
        let first_input = &calls[0];
        let second_input = &calls[1];
        assert_eq!(
            serde_json::to_vec(&second_input[..first_input.len()]).unwrap(),
            serde_json::to_vec(first_input).unwrap(),
            "active MOIM/provider prefix changed between signed calls"
        );
        let signed_index = first_input.len();
        assert_eq!(
            serde_json::to_value(&second_input[signed_index].content).unwrap(),
            serde_json::to_value(&SignedToolReplayProvider::first_response().content).unwrap()
        );
        let continuation = &second_input[signed_index + 1];
        assert_eq!(
            continuation
                .content
                .iter()
                .filter(|content| matches!(content, MessageContent::ToolResponse(_)))
                .count(),
            2,
            "two tool results must be one canonical Bedrock user message"
        );
        assert!(continuation
            .as_concat_text()
            .contains("post-tool-audit-context"));
        assert_eq!(
            second_input.len(),
            signed_index + 2,
            "hook context must be folded into the tool-result user message"
        );

        let stored = manager
            .get_session(&session.id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        let audits = stored
            .iter()
            .flat_map(|message| message.content.iter())
            .filter_map(|content| match content {
                MessageContent::ToolResponse(response) => response
                    .tool_result
                    .as_ref()
                    .ok()
                    .and_then(|result| result.meta.as_ref())
                    .and_then(|meta| meta.0.get("biorouterToolExecution")),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(audits.len(), 2);
        assert!(audits.iter().all(|audit| {
            audit["actuallyExecuted"]["arguments"]["command"] == "printf rewritten-by-hook"
        }));
        assert!(audits.iter().any(|audit| {
            audit["providerAuthored"]["arguments"]["command"] == serde_json::json!(7)
        }));
        assert!(stored.iter().flat_map(|message| &message.content).any(
            |content| matches!(content, MessageContent::Thinking(thinking) if thinking.signature == "signature-one")
        ));
        assert!(stored
            .iter()
            .any(|message| message.as_concat_text() == "answer with trailing whitespace  "));

        let replacement_stream = agent
            .reply(
                Message::user().with_text("use the replacement now"),
                SessionConfig {
                    id: session.id.clone(),
                    schedule_id: None,
                    max_turns: Some(2),
                    max_tool_calls: None,
                    budget: None,
                    retry_config: None,
                    reasoning_effort: None,
                },
                None,
            )
            .await
            .unwrap();
        tokio::pin!(replacement_stream);
        while let Some(event) = replacement_stream.next().await {
            event.unwrap();
        }
        // Read everything out under the guard and release it at the block edge.
        // An explicit `drop` does not satisfy `clippy::await_holding_lock`, and
        // this is a std `Mutex` in an async test — holding it across the awaits
        // below is the exact shape that deadlocked the retry path.
        let (replacement_call_count, replacement_had_reasoning, replacement_input) = {
            let replacement_calls = replacement.calls.lock().unwrap();
            let count = replacement_calls.len();
            let had_reasoning = replacement_calls.first().is_some_and(|call| {
                call.iter()
                    .flat_map(|message| &message.content)
                    .any(|content| {
                        matches!(
                            content,
                            MessageContent::Thinking(_) | MessageContent::RedactedThinking(_)
                        )
                    })
            });
            let input = replacement_calls
                .first()
                .map(|call| call.iter().map(Message::as_concat_text).collect::<String>())
                .unwrap_or_default();
            (count, had_reasoning, input)
        };
        assert_eq!(replacement_call_count, 1);
        assert!(!replacement_had_reasoning);
        assert!(replacement_input.contains("use the replacement now"));
        assert!(replacement_input.contains("<info-msg>"));

        let cold_manager = Arc::new(SessionManager::new(data.path().to_path_buf()));
        let cold_agent = Agent::with_config(AgentConfig::new(
            cold_manager.clone(),
            Arc::new(PermissionManager::new(permissions.path().to_path_buf())),
            None,
            BioRouterMode::Auto,
        ));
        let (bedrock, cold_capture) = capturing_provider().await;
        cold_agent
            .update_provider(Arc::new(bedrock), &session.id)
            .await
            .unwrap();
        let cold_stream = cold_agent
            .reply(
                Message::user().with_text("cold reload"),
                SessionConfig {
                    id: session.id.clone(),
                    schedule_id: None,
                    max_turns: Some(2),
                    max_tool_calls: None,
                    budget: None,
                    retry_config: None,
                    reasoning_effort: None,
                },
                None,
            )
            .await
            .unwrap();
        tokio::pin!(cold_stream);
        while let Some(event) = cold_stream.next().await {
            match event {
                Err(_) | Ok(AgentEvent::TurnAborted { .. }) => break,
                Ok(AgentEvent::Message(message))
                    if message.as_concat_text().contains("Model call failed") =>
                {
                    break;
                }
                Ok(_) => {}
            }
        }
        let cold_wire = captured_json(cold_capture);
        let cold_wire_text = serde_json::to_string(&cold_wire).unwrap();
        assert!(!cold_wire_text.contains("reasoningContent"));
        assert!(!cold_wire_text.contains("signature-one"));
        assert!(!cold_wire_text.contains("signature-two"));
        assert!(cold_wire_text.contains("answer with trailing whitespace"));
        assert!(cold_wire_text.contains("cold reload"));
        assert!(cold_wire_text.contains("<info-msg>"));

        let cold_stored = cold_manager
            .get_session(&session.id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        assert!(cold_stored.iter().flat_map(|message| &message.content).any(
            |content| matches!(content, MessageContent::Thinking(thinking) if thinking.signature == "signature-one")
        ));
    }
}
