//! # ToolShim Module
//!
//! The ToolShim module provides a reusable component for interpreting and augmenting LLM outputs with tool calls,
//! regardless of whether the underlying model natively supports tool/function calling.
//!
//! ## Overview
//!
//! ToolShim addresses the challenge of working with models that don't natively support tools by:
//!
//! 1. Taking the text output from any LLM
//! 2. Sending it to a separate "interpreter" model (which can be the same or different model)
//! 3. Using a model to extract tool call intentions into the appropriate format
//! 4. Converting the outputs of the interpreter model into proper tool call structs
//! 5. Augmenting the original message with the extracted tool calls
//!
//! ## Key Components
//!
//! ### ToolInterpreter Trait
//!
//! The core of ToolShim is the `ToolInterpreter` trait, which defines the interface for any model that can interpret text and extract tool calls.
//!
//! ### Implementations
//!
//! The module provides an implementation for Ollama:
//!
//! - `OllamaInterpreter`: Uses Ollama's structured output API to interpret tool calls
//!
//! ### Helper Functions
//!
//! - `augment_message_with_tool_calls`: A utility function that takes any message, extracts text content, sends it to an interpreter, and adds any detected tool calls back to the message.
//! - `interpret_tool_calls_from_text`: the same interpretation for text the
//!   caller assembled itself (the agent loop runs it ONCE, on a whole streamed
//!   reply), with every recovered call checked against the advertised tools.
//!
//! ## Deterministic first, a second model only if that fails
//!
//! Before the interpreter model is asked, the reply is read with the guarded
//! text-call grammar (`formats::text_tool_calls`), TIER1 and TIER2: the whole
//! reply is a call object (or an array of them), or short prose ends in ONE
//! call block, bare or ```` ```json ````-fenced — exactly what this module's
//! own prompt asks the model to write. Measured on Ollama 0.40.2 with
//! gemma3:4b in toolshim mode: a single call came back as a bare pretty-printed
//! object (3/3), and a step with a preamble as prose plus a fenced object
//! (6/6); both are recovered here without a second model, and without its
//! failure modes (an interpreter that maps `shell` onto `developer__shell`).
//! Only text that matches neither shape goes to the interpreter.
//!
//! ## What it will not do
//!
//! - **It is opt-in and stays that way** (`BIOROUTER_TOOLSHIM`). Both shipped
//!   local models make structured tool calls once their context is right
//!   (18/18 on Ollama, 9/9 on llama.cpp, measured 2026-10); a text-to-call
//!   interpreter running by default could execute something a model only
//!   QUOTED.
//! - **It sends text only to an Ollama on this machine, and only for a model
//!   that Ollama runs itself.** The interpreter is a raw HTTP client, not a
//!   `Provider`, so no privacy gate stands between it and `OLLAMA_HOST`: a
//!   private chat's reply would reach whatever host that names. So a
//!   non-loopback interpreter host is refused, and so is an Ollama CLOUD model
//!   (`gpt-oss:120b-cloud`), which a loopback Ollama forwards to ollama.com.
//!   Both are refused before anything is sent, by name and then by the
//!   server's own `/api/show`. What this cannot see is an Ollama that is
//!   itself a proxy for somewhere else; that is the operator's setup, not a
//!   model name.
//! - **It logs no content.** Request and response bodies are the assistant's
//!   own text; only sizes and counts are logged.
//! - **It never fails a reply the user has already been shown.** The
//!   interpreter is asked once, after the reply streamed, so a late failure
//!   keeps the reply as text (see `reply_parts::ToolshimReply`). The common
//!   misconfigurations (no Ollama, interpreter model not pulled) are caught by
//!   [`OllamaInterpreter::ensure_ready`] before the chat model is asked.

use super::errors::ProviderError;
use super::ollama::OLLAMA_HOST;
use crate::conversation::message::{Message, MessageContent};
use crate::conversation::Conversation;
use crate::model::ModelConfig;
use crate::providers::formats::audience;
use crate::providers::formats::openai::create_request;
use crate::providers::formats::text_tool_calls::{self, Tiers};
use anyhow::Result;
use reqwest::Client;
use rmcp::model::{object, CallToolRequestParams, Tool};
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

/// Default model to use for tool interpretation
pub const DEFAULT_INTERPRETER_MODEL_OLLAMA: &str = "mistral-nemo";

/// The name the interpreter is told to use for "no tool call here". Never a
/// real tool, never dispatched.
const NOOP_TOOL_NAME: &str = "noop";

/// Environment variables that affect behavior:
/// - BIOROUTER_TOOLSHIM: When set to "true" or "1", enables using the tool shim in the standard OllamaProvider (default: false)
/// - BIOROUTER_TOOLSHIM_OLLAMA_MODEL: Ollama model to use as the tool interpreter (default: DEFAULT_INTERPRETER_MODEL)
///   — read into `ModelConfig::toolshim_model`, which is what the interpreter
///   honours, so a config built in code (`with_toolshim_model`) is obeyed too.
/// A trait for models that can interpret text into structured tool call JSON format
#[async_trait::async_trait]
pub trait ToolInterpreter {
    /// Interpret potential tool calls from text and convert them to proper tool call JSON format
    async fn interpret_to_tool_calls(
        &self,
        content: &str,
        tools: &[Tool],
    ) -> Result<Vec<CallToolRequestParams>, ProviderError>;
}

/// Ollama-specific implementation of the ToolInterpreter trait
pub struct OllamaInterpreter {
    client: Client,
    base_url: String,
    /// The interpreter model: `ModelConfig::toolshim_model`, else
    /// [`DEFAULT_INTERPRETER_MODEL_OLLAMA`].
    model: String,
}

impl OllamaInterpreter {
    /// The interpreter for a chat bound to `model_config`, or a refusal when
    /// `OLLAMA_HOST` is not this machine or the interpreter model is one of
    /// Ollama's cloud models.
    ///
    /// The refusal comes BEFORE any request: the caller builds the interpreter
    /// before it asks the chat model for anything, so a misconfigured host
    /// costs the user a sentence, not a generation, and never a transcript.
    ///
    /// `OLLAMA_HOST` is read with the Ollama provider's own resolver
    /// ([`crate::providers::ollama::resolve_ollama_base_url`]). This module
    /// used to parse it itself, and the two disagreed: `http://127.0.0.1:80/`
    /// sent the interpreter to 11434 while the chat went to 80, and `::1` or a
    /// blank value was refused here while the provider served it. One setting,
    /// one server.
    pub fn for_model(model_config: &ModelConfig) -> Result<Self, ProviderError> {
        let host: String = crate::config::Config::global()
            .get_param("OLLAMA_HOST")
            .unwrap_or_else(|_| OLLAMA_HOST.to_string());
        let base_url = crate::providers::ollama::resolve_ollama_base_url(&host).map_err(|e| {
            ProviderError::ExecutionError(format!(
                "BIOROUTER_TOOLSHIM could not use OLLAMA_HOST: {e}"
            ))
        })?;
        Self::with_base_url(base_url.to_string(), model_config)
    }

    fn with_base_url(base_url: String, model_config: &ModelConfig) -> Result<Self, ProviderError> {
        if !crate::providers::is_loopback_host(&base_url) {
            return Err(ProviderError::ExecutionError(format!(
                "BIOROUTER_TOOLSHIM interprets tool calls with an Ollama on this machine only, \
                 and OLLAMA_HOST ({}) is not one, so the reply was not sent to it. Point \
                 OLLAMA_HOST at a local Ollama, or turn BIOROUTER_TOOLSHIM off.",
                crate::providers::base::computer_use_destination_origin(&base_url)
                    .unwrap_or_else(|| "a remote host".to_string())
            )));
        }
        let model = model_config
            .toolshim_model
            .clone()
            .filter(|model| !model.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_INTERPRETER_MODEL_OLLAMA.to_string());
        // A loopback host is not enough: Ollama serves a cloud model by
        // forwarding the request to ollama.com, so a private chat's reply would
        // leave the machine through the one host this check just approved.
        if crate::providers::utils::is_ollama_cloud_model(&model) {
            return Err(Self::cloud_refusal(&model));
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(600))
            .build()
            .map_err(|e| {
                ProviderError::ExecutionError(format!("Failed to create HTTP client: {e}"))
            })?;
        Ok(Self {
            client,
            base_url,
            model,
        })
    }

    fn cloud_refusal(model: &str) -> ProviderError {
        ProviderError::ExecutionError(format!(
            "BIOROUTER_TOOLSHIM interprets tool calls with a model that runs on this machine \
             only, and {model} is an Ollama cloud model, which Ollama serves from ollama.com, \
             so the reply was not sent to it. Set BIOROUTER_TOOLSHIM_OLLAMA_MODEL to a model \
             you have pulled, or turn BIOROUTER_TOOLSHIM off."
        ))
    }

    /// The model this interpreter asks.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// How long [`Self::ensure_ready`] waits. `/api/show` reads a manifest and
    /// a model's metadata from local disk; it does not load the model.
    /// Measured on Ollama 0.40.2 (2026-10): 6 ms for a pulled 27B, and a 404
    /// (`model 'mistral-nemo:latest' not found`) for one that is not pulled.
    const READY_TIMEOUT: Duration = Duration::from_secs(10);

    /// Confirm, before the chat model is asked anything, that the interpreter
    /// can answer: the Ollama is up, it has the interpreter model, and it runs
    /// that model itself.
    ///
    /// ⚠ The interpreter is asked once, AFTER the reply streamed. Without this
    /// check a missing Ollama or an unpulled `mistral-nemo` surfaced only then,
    /// minutes into a local 27B reply; the failure was recoverable, so the
    /// mistake ladder (BR-66) regenerated the whole reply and failed again, and
    /// the user waited through two generations for a failed turn. Asked here,
    /// the same mistake costs one loopback round trip and a sentence.
    ///
    /// The answer's `remote_host` / `remote_model` are what Ollama's
    /// `ShowResponse` carries for a model it only proxies, which also catches a
    /// cloud model renamed with `ollama cp`. Not measured on this project's
    /// machines (no cloud model is pulled on them), so the name rule in
    /// [`Self::with_base_url`] stays the primary check and this one is
    /// additive: an answer without those fields refuses nothing.
    ///
    /// Only the model name is sent; no conversation text.
    pub async fn ensure_ready(&self) -> Result<(), ProviderError> {
        let url = format!("{}/api/show", self.base_url.trim_end_matches('/'));
        let model = self.model.as_str();
        let origin = crate::providers::base::computer_use_destination_origin(&self.base_url)
            .unwrap_or_else(|| "this machine".to_string());
        let response = self
            .client
            .post(&url)
            .timeout(Self::READY_TIMEOUT)
            .json(&json!({ "model": model }))
            .send()
            .await
            .map_err(|_| {
                ProviderError::ExecutionError(format!(
                    "BIOROUTER_TOOLSHIM needs an Ollama at {origin} to interpret tool calls, and \
                     none answered, so the model was not asked. Start Ollama, or turn \
                     BIOROUTER_TOOLSHIM off."
                ))
            })?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(ProviderError::ExecutionError(format!(
                "BIOROUTER_TOOLSHIM interprets tool calls with {model}, and the Ollama at \
                 {origin} does not have it, so the model was not asked. Run `ollama pull \
                 {model}`, set BIOROUTER_TOOLSHIM_OLLAMA_MODEL to a model you have, or turn \
                 BIOROUTER_TOOLSHIM off."
            )));
        }
        if !status.is_success() {
            // The body is not echoed, as everywhere in this module.
            return Err(ProviderError::ExecutionError(format!(
                "BIOROUTER_TOOLSHIM could not check its interpreter model {model} with the \
                 Ollama at {origin} (HTTP {status}), so the model was not asked."
            )));
        }
        // A body that is not JSON says nothing about where the model runs; the
        // status already said the model exists.
        let shown: Value = response.json().await.unwrap_or(Value::Null);
        let proxied = ["remote_host", "remote_model"].iter().any(|key| {
            shown
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|v| !v.is_empty())
        });
        if proxied {
            return Err(Self::cloud_refusal(model));
        }
        Ok(())
    }

    fn tool_structured_ouput_format_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "tool_calls": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "The name of the tool to call"
                            },
                            "arguments": {
                                "type": "object",
                                "description": "The arguments to pass to the tool"
                            }
                        },
                        "required": ["name", "arguments"]
                    }
                }
            },
            "required": ["tool_calls"]
        })
    }

    async fn post_structured(
        &self,
        system_prompt: &str,
        format_instruction: &str,
        format_schema: Value,
        model: &str,
    ) -> Result<Value, ProviderError> {
        let base_url = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/chat", base_url);

        let mut messages = Vec::new();
        let user_message = Message::user().with_text(format_instruction);
        messages.push(user_message);

        let model_config = ModelConfig::new(model)
            .map_err(|e| ProviderError::RequestFailed(format!("Model config error: {e}")))?;

        let mut payload = create_request(
            &model_config,
            system_prompt,
            &messages,
            &[], // No tools
            &super::utils::ImageFormat::OpenAi,
            false,
        )?;

        payload["stream"] = json!(false); // needed for the /api/chat endpoint to work
        payload["format"] = format_schema;

        // Metadata only. This used to log the whole payload at INFO — the
        // assistant's reply verbatim, which in a private chat may carry PHI.
        tracing::debug!(
            model,
            request_bytes = format_instruction.len(),
            "tool interpreter request"
        );

        let response = self.client.post(&url).json(&payload).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            // The body is not echoed: an interpreter error can quote its input.
            return Err(ProviderError::RequestFailed(format!(
                "Ollama structured API returned error status {} for interpreter model {model}",
                status
            )));
        }

        let response_json: Value = response.json().await.map_err(|e| {
            ProviderError::RequestFailed(format!(
                "Failed to parse Ollama structured API response: {e}"
            ))
        })?;

        Ok(response_json)
    }

    fn process_interpreter_response(
        response: &Value,
    ) -> Result<Vec<CallToolRequestParams>, ProviderError> {
        let mut tool_calls = Vec::new();
        // Extract tool_calls array from the response
        if response.get("message").is_some() && response["message"].get("content").is_some() {
            let content = response["message"]["content"].as_str().unwrap_or_default();

            // Try to parse the content as JSON
            if let Ok(content_json) = serde_json::from_str::<Value>(content) {
                // Check for the format with tool_calls array inside an object
                if content_json.is_object() && content_json.get("tool_calls").is_some() {
                    // Process each tool call in the array
                    if let Some(tool_calls_array) = content_json["tool_calls"].as_array() {
                        for item in tool_calls_array {
                            if item.is_object()
                                && item.get("name").is_some()
                                && item.get("arguments").is_some()
                            {
                                let name = item["name"].as_str().unwrap_or_default().to_string();
                                let arguments = item["arguments"].clone();

                                // Add the tool call to our result vector
                                tool_calls.push(CallToolRequestParams {
                                    task: None,
                                    name: name.into(),
                                    arguments: Some(object(arguments)),
                                    meta: None,
                                });
                            }
                        }
                    }
                }
            }
        }

        tracing::debug!(calls = tool_calls.len(), "tool interpreter response");
        Ok(tool_calls)
    }
}

#[async_trait::async_trait]
impl ToolInterpreter for OllamaInterpreter {
    async fn interpret_to_tool_calls(
        &self,
        last_assistant_msg: &str,
        tools: &[Tool],
    ) -> Result<Vec<CallToolRequestParams>, ProviderError> {
        if tools.is_empty() {
            return Ok(vec![]);
        }

        // Create the system prompt
        let system_prompt = "If there is detectable JSON-formatted tool requests, write them into valid JSON tool calls in the following format:
{{
  \"tool_calls\": [
    {{
      \"name\": \"tool_name\",
      \"arguments\": {{
        \"param1\": \"value1\",
        \"param2\": \"value2\"
      }}
    }}
  ]
}}

Otherwise, if no JSON tool requests are provided, use the no-op tool:
{{
  \"tool_calls\": [
    {{
    \"name\": \"noop\",
      \"arguments\": {{
      }}
    }}]
}}
";

        // Create enhanced content with instruction to output tool calls as JSON
        let format_instruction = format!("{}\nRequest: {}\n\n", system_prompt, last_assistant_msg);

        // Define the JSON schema for tool call format
        let format_schema = OllamaInterpreter::tool_structured_ouput_format_schema();

        // The model the chat's own config names (`ModelConfig::toolshim_model`,
        // filled from BIOROUTER_TOOLSHIM_OLLAMA_MODEL). Reading the env var
        // here again ignored a config built in code.
        let interpreter_response = self
            .post_structured("", &format_instruction, format_schema, &self.model)
            .await?;

        // Process the interpreter response to get tool calls directly
        let tool_calls = OllamaInterpreter::process_interpreter_response(&interpreter_response)?;

        Ok(tool_calls)
    }
}

/// Creates a string containing formatted tool information
pub fn format_tool_info(tools: &[Tool]) -> String {
    let mut tool_info = String::new();
    for tool in tools {
        tool_info.push_str(&format!(
            "Tool Name: {}\nSchema: {}\nDescription: {:?}\n\n",
            tool.name,
            serde_json::to_string_pretty(&tool.input_schema).unwrap_or_default(),
            tool.description
        ));
    }
    tool_info
}

/// Convert messages containing ToolRequest/ToolResponse to text messages for toolshim mode
/// This is necessary because some providers (like Bedrock) validate that tool_use/tool_result
/// blocks can only exist when tools are defined, but in toolshim mode we pass empty tools
pub fn convert_tool_messages_to_text(messages: &[Message]) -> Conversation {
    let converted_messages: Vec<Message> = messages
        .iter()
        .map(|message| {
            let mut new_content = Vec::new();
            let mut has_tool_content = false;

            for content in &message.content {
                match content {
                    MessageContent::ToolRequest(req) => {
                        has_tool_content = true;
                        // Convert tool request to text format
                        let text = if let Ok(tool_call) = &req.tool_call {
                            format!(
                                "Using tool: {}\n{{\n  \"name\": \"{}\",\n  \"arguments\": {}\n}}",
                                tool_call.name,
                                tool_call.name,
                                serde_json::to_string_pretty(&tool_call.arguments)
                                    .unwrap_or_default()
                            )
                        } else {
                            "Tool request failed".to_string()
                        };
                        new_content.push(MessageContent::text(text));
                    }
                    MessageContent::ToolResponse(res) => {
                        has_tool_content = true;
                        // Convert tool response to text format
                        let text = match &res.tool_result {
                            Ok(result) => {
                                let text_contents: Vec<String> = result
                                    .content
                                    .iter()
                                    // Send only what the tool addressed to the
                                    // model. This is the last place the audience
                                    // survives: the block becomes plain text
                                    // here, so the provider formatter that runs
                                    // next has nothing left to filter on.
                                    .filter(|c| audience::is_for_model(c))
                                    .filter_map(audience::flattened_text)
                                    .collect();
                                format!("Tool result:\n{}", text_contents.join("\n"))
                            }
                            Err(e) => format!("Tool error: {}", e),
                        };
                        new_content.push(MessageContent::text(text));
                    }
                    _ => {
                        // Keep other content types as-is
                        new_content.push(content.clone());
                    }
                }
            }

            if has_tool_content {
                Message::new(message.role.clone(), message.created, new_content)
            } else {
                message.clone()
            }
        })
        .collect();

    Conversation::new_unvalidated(converted_messages)
}

/// Modifies the system prompt to include tool usage instructions when tool interpretation is enabled
pub fn modify_system_prompt_for_tool_json(system_prompt: &str, tools: &[Tool]) -> String {
    let tool_info = format_tool_info(tools);

    format!(
        "{}\n\n{}\n\nBreak down your task into smaller steps and do one step and tool call at a time. Do not try to use multiple tools at once. If you want to use a tool, tell the user what tool to use by specifying the tool in this JSON format\n{{\n  \"name\": \"tool_name\",\n  \"arguments\": {{\n    \"parameter1\": \"value1\",\n    \"parameter2\": \"value2\"\n }}\n}}. After you get the tool result back, consider the result and then proceed to do the next step and tool call if required.",
        system_prompt,
        tool_info
    )
}

/// The tool calls an interpreter recovers from `text`, keeping only those that
/// name a tool the model was actually offered.
///
/// ⚠ **Exact names only.** The interpreter is a second model reading the
/// first one's prose, and it reports whatever name the prose used — `shell`
/// where the tool is `developer__shell`, or a tool the chat never had. Mapping
/// a near-miss onto a real tool is exactly the guess this shim must not make:
/// the prose may only be QUOTING a call. A name that is not on the roster is
/// dropped, and only its count is logged.
pub async fn interpret_tool_calls_from_text<T: ToolInterpreter>(
    interpreter: &T,
    text: &str,
    tools: &[Tool],
) -> Result<Vec<CallToolRequestParams>, ProviderError> {
    Ok(interpret_tool_requests_from_text(interpreter, text, tools)
        .await?
        .into_iter()
        .filter_map(|content| match content {
            MessageContent::ToolRequest(request) => request.tool_call.ok(),
            _ => None,
        })
        .collect())
}

/// [`interpret_tool_calls_from_text`], as tool requests ready to append to the
/// reply: each with a fresh id, and a call recovered by the text grammar
/// carrying the `recovered_from_text` marker in its metadata.
pub async fn interpret_tool_requests_from_text<T: ToolInterpreter>(
    interpreter: &T,
    text: &str,
    tools: &[Tool],
) -> Result<Vec<MessageContent>, ProviderError> {
    if tools.is_empty() || text.trim().is_empty() {
        return Ok(Vec::new());
    }
    // Text is the tool protocol in this mode, so both tiers apply; the
    // interpreter model is only asked when the grammar finds nothing.
    if let Some(recovery) =
        text_tool_calls::recover(text, tools, Tiers::WholeMessageOrTrailingBlock)
    {
        text_tool_calls::log_recovery("toolshim", "", &recovery);
        let format = recovery.format;
        return Ok(recovery
            .calls
            .into_iter()
            .map(|call| text_tool_calls::recovered_tool_request(call, format))
            .collect());
    }
    let recovered = interpreter.interpret_to_tool_calls(text, tools).await?;
    let offered = |name: &str| tools.iter().any(|tool| tool.name == name);
    let (kept, dropped): (Vec<_>, Vec<_>) = recovered
        .into_iter()
        .filter(|call| call.name != NOOP_TOOL_NAME)
        .partition(|call| offered(&call.name));
    if !dropped.is_empty() {
        tracing::debug!(
            dropped = dropped.len(),
            kept = kept.len(),
            "tool interpreter named tools that were not offered; dropped"
        );
    }
    Ok(kept
        .into_iter()
        .map(|call| MessageContent::tool_request(Uuid::new_v4().to_string(), Ok(call)))
        .collect())
}

/// Helper function to augment a message with tool calls if any are detected
///
/// Reads ALL of the message's text (a reply assembled from a stream arrives in
/// many text items), and adds only calls naming an offered tool — see
/// [`interpret_tool_calls_from_text`].
pub async fn augment_message_with_tool_calls<T: ToolInterpreter>(
    interpreter: &T,
    message: Message,
    tools: &[Tool],
) -> Result<Message, ProviderError> {
    // If there are no tools or the message is empty, return the original message
    if tools.is_empty() {
        return Ok(message);
    }

    // A native call needs no interpreting.
    if message
        .content
        .iter()
        .any(|content| matches!(content, MessageContent::ToolRequest(_)))
    {
        return Ok(message);
    }

    let text: String = message
        .content
        .iter()
        .filter_map(|content| match content {
            MessageContent::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect();

    let tool_requests = interpret_tool_requests_from_text(interpreter, &text, tools).await?;

    let mut final_message = message;
    for request in tool_requests {
        final_message = final_message.with_content(request);
    }

    Ok(final_message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::formats::audience::{
        every_audience_case, text_editor_view_result, MODEL_HIDDEN, MODEL_VISIBLE, VIEW_FOR_MODEL,
        VIEW_FOR_USER,
    };
    use rmcp::model::{CallToolResult, Content};

    /// The single text block a tool response collapses into under toolshim.
    fn converted_text(content: Vec<Content>) -> String {
        let message = Message::user().with_tool_response(
            "call-1",
            Ok(CallToolResult {
                content,
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        );

        let converted = convert_tool_messages_to_text(&[message]);
        let messages = converted.messages();
        assert_eq!(messages.len(), 1, "one message in, one message out");
        match &messages[0].content[0] {
            MessageContent::Text(text) => text.text.clone(),
            other => panic!("the tool response should have become text, got {other:?}"),
        }
    }

    /// Every audience case, through the real toolshim conversion.
    ///
    /// This is the last place the audience is readable on a toolshim turn. The
    /// block leaves here as plain text, so the provider formatter downstream
    /// has no annotation left to filter on and a block withheld from the model
    /// has to be dropped right here or not at all.
    #[test]
    fn tool_result_blocks_reach_the_model_by_audience() {
        let text = converted_text(every_audience_case());

        assert_eq!(
            text,
            format!("Tool result:\n{}", MODEL_VISIBLE.join("\n")),
            "the converted text must carry exactly the model-addressed blocks"
        );
        for withheld in MODEL_HIDDEN {
            assert!(!text.contains(withheld), "{withheld} reached the model");
        }
    }

    fn tool(name: &str) -> Tool {
        Tool::new(
            name.to_string(),
            "a tool".to_string(),
            rmcp::model::object(json!({"type": "object", "properties": {}})),
        )
    }

    fn call(name: &str) -> CallToolRequestParams {
        CallToolRequestParams {
            task: None,
            name: name.to_string().into(),
            arguments: Some(object(json!({"command": "ls"}))),
            meta: None,
        }
    }

    /// An interpreter that answers with fixed calls and records every text it
    /// was asked about.
    struct ScriptedInterpreter {
        answer: Vec<&'static str>,
        asked: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl ToolInterpreter for ScriptedInterpreter {
        async fn interpret_to_tool_calls(
            &self,
            content: &str,
            _tools: &[Tool],
        ) -> Result<Vec<CallToolRequestParams>, ProviderError> {
            self.asked.lock().unwrap().push(content.to_string());
            Ok(self.answer.iter().map(|name| call(name)).collect())
        }
    }

    fn scripted(answer: Vec<&'static str>) -> ScriptedInterpreter {
        ScriptedInterpreter {
            answer,
            asked: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// OLL-09: a recovered call is kept only when it names an offered tool
    /// EXACTLY. The reporter's text calls named `shell` and `text_editor`,
    /// which no roster had; mapping those onto `developer__shell` would run
    /// what a model may only have quoted.
    #[tokio::test]
    async fn only_calls_naming_an_offered_tool_survive_interpretation() {
        let interpreter = scripted(vec![
            "developer__shell",
            "shell",
            "noop",
            "workspace__workspace_send_prompt",
        ]);
        let tools = [tool("developer__shell"), tool("todo__todo_write")];
        let calls = interpret_tool_calls_from_text(&interpreter, "run ls", &tools)
            .await
            .unwrap();
        let names: Vec<&str> = calls.iter().map(|c| c.name.as_ref()).collect();
        assert_eq!(names, ["developer__shell"]);
    }

    /// A reply in one of the shapes the toolshim prompt asks for is read by
    /// the text grammar, and the interpreter model is never asked: gemma3:4b's
    /// verbatim toolshim replies (Ollama 0.40.2), a bare pretty-printed call
    /// and a preamble followed by a fenced call.
    #[tokio::test]
    async fn the_text_grammar_answers_before_the_interpreter_model() {
        let shell = Tool::new(
            "developer__shell".to_string(),
            "run a command".to_string(),
            rmcp::model::object(json!({
                "type": "object",
                "properties": {"command": {"type": "string"}},
                "required": ["command"]
            })),
        );
        for text in [
            "{\n  \"name\": \"developer__shell\",\n  \"arguments\": {\n    \"command\": \"ls /tmp\"\n  }\n}",
            "Okay, let's do this. First, I'll run the `ls /tmp` command.\n\n```json\n{\n  \"name\": \"developer__shell\",\n  \"arguments\": {\n    \"command\": \"ls /tmp\"\n  }\n}\n```",
        ] {
            let interpreter = scripted(vec!["noop"]);
            let requests =
                interpret_tool_requests_from_text(&interpreter, text, std::slice::from_ref(&shell))
                    .await
                    .unwrap();
            assert!(interpreter.asked.lock().unwrap().is_empty(), "{text}");
            assert_eq!(requests.len(), 1, "{text}");
            let MessageContent::ToolRequest(request) = &requests[0] else {
                panic!("a tool request");
            };
            assert!(text_tool_calls::is_recovered(request));
            let call = request.tool_call.as_ref().unwrap();
            assert_eq!(call.name, "developer__shell");
            assert_eq!(call.arguments.as_ref().unwrap()["command"], "ls /tmp");
        }
    }

    /// Blank text, or no tools, never reaches the interpreter at all.
    #[tokio::test]
    async fn nothing_to_interpret_sends_nothing() {
        let interpreter = scripted(vec!["developer__shell"]);
        let tools = [tool("developer__shell")];
        assert!(interpret_tool_calls_from_text(&interpreter, "  \n", &tools)
            .await
            .unwrap()
            .is_empty());
        assert!(interpret_tool_calls_from_text(&interpreter, "run ls", &[])
            .await
            .unwrap()
            .is_empty());
        assert!(interpreter.asked.lock().unwrap().is_empty());
    }

    /// A reply assembled from a stream holds its text in many items; the
    /// interpreter must see all of it, once — reading only the first item was
    /// one streamed token.
    #[tokio::test]
    async fn augmentation_reads_every_text_item_once() {
        let interpreter = scripted(vec!["developer__shell"]);
        let message = Message::assistant()
            .with_text("I would run the shell ")
            .with_text("with ls here.");
        let augmented =
            augment_message_with_tool_calls(&interpreter, message, &[tool("developer__shell")])
                .await
                .unwrap();
        assert_eq!(
            *interpreter.asked.lock().unwrap(),
            ["I would run the shell with ls here."]
        );
        assert!(augmented.is_tool_call());

        // A reply that already carries a native call is left alone.
        let native = Message::assistant()
            .with_text("calling")
            .with_tool_request("call-1", Ok(call("developer__shell")));
        let interpreter = scripted(vec!["developer__shell"]);
        let untouched = augment_message_with_tool_calls(
            &interpreter,
            native.clone(),
            &[tool("developer__shell")],
        )
        .await
        .unwrap();
        assert_eq!(untouched, native);
        assert!(interpreter.asked.lock().unwrap().is_empty());
    }

    /// OLL-10: the interpreter is a raw client outside every privacy gate, so a
    /// host that is not this machine is refused before anything is sent.
    #[test]
    fn a_non_loopback_interpreter_host_is_refused() {
        let config = ModelConfig::new_or_fail("qwen3.8:27b");
        for remote in [
            "http://192.168.1.10:11434/",
            "https://ollama.example.org/",
            "http://user:secret@10.0.0.5:11434/",
        ] {
            let error = OllamaInterpreter::with_base_url(remote.to_string(), &config)
                .err()
                .unwrap_or_else(|| panic!("{remote} must be refused"));
            let text = error.to_string();
            assert!(text.contains("was not sent"), "{text}");
            assert!(!text.contains("secret"), "credentials never echoed: {text}");
        }
        for local in [
            "http://localhost:11434/",
            "http://127.0.0.1:11434/",
            "http://[::1]:11434/",
        ] {
            assert!(
                OllamaInterpreter::with_base_url(local.to_string(), &config).is_ok(),
                "{local}"
            );
        }
    }

    /// A loopback Ollama forwards a cloud model to ollama.com, so the loopback
    /// check alone would let a private chat's reply leave the machine. Refused
    /// before the interpreter is even built.
    #[test]
    fn an_ollama_cloud_interpreter_model_is_refused() {
        for cloud in ["gpt-oss:120b-cloud", "glm-4.6:cloud"] {
            let config =
                ModelConfig::new_or_fail("qwen3.8:27b").with_toolshim_model(Some(cloud.into()));
            let error =
                OllamaInterpreter::with_base_url("http://127.0.0.1:11434/".to_string(), &config)
                    .err()
                    .unwrap_or_else(|| panic!("{cloud} must be refused"));
            let text = error.to_string();
            assert!(text.contains("cloud model"), "{text}");
            assert!(text.contains("was not sent"), "{text}");
        }
    }

    /// `OLLAMA_HOST` reaches the interpreter through the Ollama provider's own
    /// resolver, so one setting names one server. This module's old parser
    /// sent `http://127.0.0.1:80/` to 11434 and refused `::1` outright.
    #[test]
    fn the_interpreter_reads_ollama_host_like_the_provider() {
        let config = ModelConfig::new_or_fail("qwen3.8:27b");
        for (host, expected) in [
            ("http://127.0.0.1:80/", "http://127.0.0.1/"),
            ("::1", "http://[::1]:11434/"),
            ("localhost", "http://localhost:11434/"),
            ("127.0.0.1:11500", "http://127.0.0.1:11500/"),
        ] {
            let _env = env_lock::lock_env([("OLLAMA_HOST", Some(host))]);
            let interpreter = OllamaInterpreter::for_model(&config)
                .unwrap_or_else(|e| panic!("{host:?} should be usable: {e}"));
            assert_eq!(interpreter.base_url, expected, "{host:?}");
            assert_eq!(
                interpreter.base_url,
                crate::providers::ollama::resolve_ollama_base_url(host)
                    .unwrap()
                    .as_str(),
                "{host:?}: the interpreter and the provider must reach one server"
            );
        }
    }

    /// `ensure_ready` against a stand-in Ollama: it sends the model name and
    /// nothing else, accepts a model the server has, and refuses one it does
    /// not have, one it only proxies, or a server that is not there — each
    /// before the chat model would be asked.
    #[tokio::test]
    async fn ensure_ready_refuses_what_would_fail_after_the_reply() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let config = ModelConfig::new_or_fail("qwen3.8:27b")
            .with_toolshim_model(Some("local-interpreter".into()));

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .and(body_json(json!({"model": "local-interpreter"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "details": {"format": "gguf", "family": "qwen3"},
                "capabilities": ["completion", "tools"]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let ready = OllamaInterpreter::with_base_url(server.uri(), &config).unwrap();
        ready.ensure_ready().await.unwrap();

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "error": "model 'local-interpreter' not found"
            })))
            .mount(&server)
            .await;
        let missing = OllamaInterpreter::with_base_url(server.uri(), &config)
            .unwrap()
            .ensure_ready()
            .await
            .unwrap_err()
            .to_string();
        assert!(
            missing.contains("ollama pull local-interpreter"),
            "{missing}"
        );

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "remote_model": "gpt-oss:120b",
                "remote_host": "https://ollama.com:443"
            })))
            .mount(&server)
            .await;
        let proxied = OllamaInterpreter::with_base_url(server.uri(), &config)
            .unwrap()
            .ensure_ready()
            .await
            .unwrap_err()
            .to_string();
        assert!(proxied.contains("cloud model"), "{proxied}");

        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let absent = OllamaInterpreter::with_base_url(format!("http://127.0.0.1:{port}/"), &config)
            .unwrap()
            .ensure_ready()
            .await
            .unwrap_err()
            .to_string();
        assert!(absent.contains("none answered"), "{absent}");
    }

    /// OLL-09: `ModelConfig::toolshim_model` is the interpreter model; unset or
    /// blank falls back to the default.
    #[test]
    fn the_interpreter_model_comes_from_the_model_config() {
        let base = "http://127.0.0.1:11434/".to_string();
        let pinned = ModelConfig::new_or_fail("qwen3.8:27b")
            .with_toolshim_model(Some("qwen3.8:27b-q4_K_M".into()));
        assert_eq!(
            OllamaInterpreter::with_base_url(base.clone(), &pinned)
                .unwrap()
                .model(),
            "qwen3.8:27b-q4_K_M"
        );
        for unset in [None, Some("   ".to_string())] {
            let config = ModelConfig::new_or_fail("qwen3.8:27b").with_toolshim_model(unset);
            assert_eq!(
                OllamaInterpreter::with_base_url(base.clone(), &config)
                    .unwrap()
                    .model(),
                DEFAULT_INTERPRETER_MODEL_OLLAMA
            );
        }
    }

    /// The documented setting, end to end: `BIOROUTER_TOOLSHIM_OLLAMA_MODEL`
    /// is read ONCE, into `ModelConfig::toolshim_model`, and that is the model
    /// the interpreter asks. The interpreter used to re-read the variable
    /// itself, so a config built in code was ignored.
    #[test]
    fn the_toolshim_model_setting_reaches_the_interpreter_through_the_model_config() {
        let _env = env_lock::lock_env([("BIOROUTER_TOOLSHIM_OLLAMA_MODEL", Some("qwen3:8b"))]);
        let config = ModelConfig::new("qwen3.8:27b").unwrap();
        assert_eq!(config.toolshim_model.as_deref(), Some("qwen3:8b"));
        let interpreter =
            OllamaInterpreter::with_base_url("http://127.0.0.1:11434/".to_string(), &config)
                .unwrap();
        assert_eq!(interpreter.model(), "qwen3:8b");
    }

    #[derive(Clone, Default)]
    struct LogCapture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for LogCapture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// End to end against a stand-in Ollama: the configured interpreter model
    /// is the one asked, and — OLL-10 — not a byte of the reply or of the
    /// interpreter's answer reaches the log, even at DEBUG.
    #[tokio::test]
    async fn the_interpreter_asks_its_model_and_logs_no_content() {
        use tracing::instrument::WithSubscriber;
        use wiremock::matchers::{body_partial_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .and(body_partial_json(json!({"model": "local-interpreter"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "message": {"content": "{\"tool_calls\":[{\"name\":\"developer__shell\",\"arguments\":{\"command\":\"echo ANSWER_SENTINEL\"}}]}"}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let config = ModelConfig::new_or_fail("qwen3.8:27b")
            .with_toolshim_model(Some("local-interpreter".into()));
        let interpreter = OllamaInterpreter::with_base_url(server.uri(), &config).unwrap();

        let capture = LogCapture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(move || writer.clone())
            .finish();
        let calls = interpret_tool_calls_from_text(
            &interpreter,
            "PATIENT_SENTINEL: run the shell",
            &[tool("developer__shell")],
        )
        .with_subscriber(subscriber)
        .await
        .unwrap();

        assert_eq!(calls.len(), 1);
        let log = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(
            log.contains("tool interpreter"),
            "the metadata line is kept: {log}"
        );
        assert!(!log.contains("PATIENT_SENTINEL"), "{log}");
        assert!(!log.contains("ANSWER_SENTINEL"), "{log}");
    }

    /// A `text_editor view` through the real toolshim conversion: the file is
    /// an embedded resource, so reading only text blocks would hand the
    /// interpreter an empty `Tool result:` for every file view.
    #[test]
    fn a_viewed_file_reaches_the_model_through_its_embedded_resource() {
        let text = converted_text(text_editor_view_result());

        assert_eq!(text, format!("Tool result:\n{VIEW_FOR_MODEL}"));
        assert!(
            !text.contains(VIEW_FOR_USER),
            "the user's rendering reached the model"
        );
    }
}
