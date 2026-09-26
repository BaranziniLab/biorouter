use crate::conversation::message::{Message, MessageContent, ProviderMetadata};
use crate::model::ModelConfig;
use crate::providers::base::{ProviderUsage, Usage};
use crate::providers::errors::ProviderError;
use crate::providers::formats::audience;
use crate::providers::utils::{
    convert_image, detect_image_path, is_valid_function_name, load_image_file, safely_parse_json,
    sanitize_function_name, ImageFormat,
};
use anyhow::{anyhow, Error};
use async_stream::try_stream;
use chrono;
use futures::Stream;
use rmcp::model::{
    object, AnnotateAble, CallToolRequestParams, Content, ErrorCode, ErrorData, RawContent,
    ResourceContents, Role, Tool,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::borrow::Cow;
use std::ops::Deref;

/// Metadata key under which DeepSeek/Moonshot-style chain-of-thought
/// (`message.reasoning_content` / `delta.reasoning_content`) is preserved on
/// an assistant message's ToolRequest contents. Twin of
/// `formats::openrouter::REASONING_DETAILS_KEY`, for OpenAI-compatible hosts
/// that surface reasoning as a plain string instead of OpenRouter's
/// structured blocks. Capture is unconditional (an inert extra metadata key
/// for providers that ignore it); replay via
/// [`add_reasoning_content_to_request`] is opt-in per provider — Moonshot
/// REQUIRES it mid tool-loop, while e.g. DeepSeek rejects the field on input.
pub const REASONING_CONTENT_KEY: &str = "reasoning_content";

/// Metadata key recording WHICH provider captured the sibling
/// [`REASONING_CONTENT_KEY`] value. Several OpenAI-compatible hosts emit
/// `reasoning_content` (DeepSeek, Moonshot, …) and the capture below is
/// shared by all of them, so replay must be scoped to the provider that
/// produced the thinking: a session switched from DeepSeek to Moonshot must
/// NOT replay DeepSeek's hidden reasoning into Moonshot. The format decoders
/// are provider-agnostic and cannot stamp this themselves — the owning
/// provider does, via [`stamp_reasoning_provenance`] — and
/// [`add_reasoning_content_to_request`] only replays entries whose stamp
/// matches the requesting provider. Unstamped captures (from a provider that
/// never stamps, or persisted by a build predating the stamp) are dropped
/// rather than leaked across providers.
pub const REASONING_PROVIDER_KEY: &str = "reasoning_provider";

#[derive(Serialize, Deserialize, Debug, Default)]
struct DeltaToolCallFunction {
    name: Option<String>,
    #[serde(default)]
    arguments: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct DeltaToolCall {
    id: Option<String>,
    function: DeltaToolCallFunction,
    index: Option<i32>,
    r#type: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Delta {
    content: Option<String>,
    role: Option<String>,
    tool_calls: Option<Vec<DeltaToolCall>>,
    reasoning_details: Option<Vec<Value>>,
    /// DeepSeek/Moonshot-style thinking deltas (a plain string), distinct
    /// from OpenRouter's structured `reasoning_details` blocks above.
    reasoning_content: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct StreamingChoice {
    delta: Delta,
    index: Option<i32>,
    finish_reason: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct StreamingChunk {
    choices: Vec<StreamingChoice>,
    created: Option<i64>,
    id: Option<String>,
    usage: Option<Value>,
    model: Option<String>,
}

fn parse_streaming_chunk(line: &str) -> anyhow::Result<StreamingChunk> {
    let mut value: Value = serde_json::from_str(line).map_err(|error| {
        anyhow!(
            "Failed to parse streaming chunk: invalid JSON at line {} column {}",
            error.line(),
            error.column()
        )
    })?;
    if value.get("error").is_some_and(|error| !error.is_null()) {
        return Err(ProviderError::RequestFailed(
            "Provider reported an error while streaming. Contact your provider administrator."
                .into(),
        )
        .into());
    }
    if let Some(choices) = value.get_mut("choices").and_then(Value::as_array_mut) {
        for choice in choices {
            let mut annotation = false;
            for key in ["content_filter_result", "content_filter_results"] {
                if let Some(results) = choice.get(key).and_then(Value::as_object) {
                    if let Some(error) = results.get("error").filter(|error| !error.is_null()) {
                        if error.get("code").and_then(Value::as_str) == Some("content_filter_error")
                        {
                            return Err(ProviderError::ServerError(
                                "Provider content filter failed (content_filter_error). Retry the request or contact your provider administrator.".into(),
                            ).into());
                        }
                        return Err(ProviderError::RequestFailed(
                            "Provider content filter reported an error. Contact your provider administrator.".into(),
                        ).into());
                    }
                    let checks: Vec<_> = results
                        .iter()
                        .filter(|(name, _)| *name != "error")
                        .collect();
                    if checks
                        .iter()
                        .any(|(_, check)| check.get("filtered") == Some(&Value::Bool(true)))
                    {
                        return Err(ProviderError::RequestFailed(
                            "Provider safety filter blocked the response (content_filter). Revise the request or contact your provider administrator.".into(),
                        ).into());
                    }
                    annotation |= !checks.is_empty()
                        && checks.iter().all(|(_, check)| {
                            check.get("filtered").and_then(Value::as_bool).is_some()
                                || check.get("detected").and_then(Value::as_bool).is_some()
                        });
                }
            }
            if choice.get("finish_reason").and_then(Value::as_str) == Some("content_filter") {
                return Err(ProviderError::RequestFailed(
                    "Provider safety filter blocked the response (content_filter). Revise the request or contact your provider administrator.".into(),
                ).into());
            }
            // Azure asynchronous filter annotations omit delta. Only recognized
            // filter results justify an empty delta; malformed choices still fail.
            if annotation && choice.get("delta").is_none_or(Value::is_null) {
                choice["delta"] = json!({});
            }
        }
    }
    // Serde's type errors can echo provider-controlled strings, including prompt
    // data. Do not include the raw payload or deserializer message in UI errors.
    serde_json::from_value(value)
        .map_err(|_| anyhow!("Failed to parse streaming chunk: invalid chunk shape (expected choices with delta objects)"))
}

#[allow(clippy::too_many_lines)]
pub fn format_messages(messages: &[Message], image_format: &ImageFormat) -> Vec<Value> {
    let mut messages_spec = Vec::new();
    for message in messages.iter().filter(|m| m.is_agent_visible()) {
        let mut converted = json!({
            "role": message.role
        });

        let mut output = Vec::new();
        let mut content_array = Vec::new();
        let mut text_array = Vec::new();

        for content in &message.content {
            match content {
                MessageContent::Text(text) => {
                    if !text.text.is_empty() {
                        if let Some(image_path) = detect_image_path(&text.text) {
                            if let Ok(image) = load_image_file(image_path) {
                                content_array.push(json!({"type": "text", "text": text.text}));
                                content_array.push(convert_image(&image, image_format));
                            } else {
                                text_array.push(text.text.clone());
                            }
                        } else {
                            text_array.push(text.text.clone());
                        }
                    }
                }
                MessageContent::Thinking(_) => {
                    // Thinking blocks are not directly used in OpenAI format
                    continue;
                }
                MessageContent::RedactedThinking(_) => {
                    // Redacted thinking blocks are not directly used in OpenAI format
                    continue;
                }
                MessageContent::SystemNotification(_) => {
                    continue;
                }
                MessageContent::ToolRequest(request) => match &request.tool_call {
                    Ok(tool_call) => {
                        let sanitized_name = sanitize_function_name(&tool_call.name);
                        let arguments_str = match &tool_call.arguments {
                            Some(args) => {
                                serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string())
                            }
                            None => "{}".to_string(),
                        };

                        let tool_calls = converted
                            .as_object_mut()
                            .unwrap()
                            .entry("tool_calls")
                            .or_insert(json!([]));

                        tool_calls.as_array_mut().unwrap().push(json!({
                            "id": request.id,
                            "type": "function",
                            "function": {
                                "name": sanitized_name,
                                "arguments": arguments_str,
                            }
                        }));
                    }
                    Err(e) => {
                        output.push(json!({
                            "role": "tool",
                            "content": format!("Error: {}", e),
                            "tool_call_id": request.id
                        }));
                    }
                },
                MessageContent::ToolResponse(response) => {
                    match &response.tool_result {
                        Ok(result) => {
                            // Send only what the tool addressed to the model.
                            let abridged: Vec<_> = result
                                .content
                                .iter()
                                .filter(|content| audience::is_for_model(content))
                                .cloned()
                                .collect();

                            // Process all content, replacing images with placeholder text
                            let mut tool_content = Vec::new();
                            let mut image_messages = Vec::new();

                            for content in abridged {
                                match content.deref() {
                                    RawContent::Image(image) => {
                                        // Add placeholder text in the tool response
                                        tool_content.push(Content::text("This tool result included an image that is uploaded in the next message."));

                                        // Create a separate image message
                                        image_messages.push(json!({
                                            "role": "user",
                                            "content": [convert_image(&image.clone().no_annotation(), image_format)]
                                        }));
                                    }
                                    RawContent::Resource(resource) => {
                                        let text = match &resource.resource {
                                            ResourceContents::TextResourceContents {
                                                text, ..
                                            } => text.clone(),
                                            _ => String::new(),
                                        };
                                        tool_content.push(Content::text(text));
                                    }
                                    _ => {
                                        tool_content.push(content);
                                    }
                                }
                            }
                            let tool_response_content: Value = json!(tool_content
                                .iter()
                                .map(|content| match content.deref() {
                                    RawContent::Text(text) => text.text.clone(),
                                    _ => String::new(),
                                })
                                .collect::<Vec<String>>()
                                .join(" "));

                            // First add the tool response with all content
                            output.push(json!({
                                "role": "tool",
                                "content": tool_response_content,
                                "tool_call_id": response.id
                            }));
                            // Then add any image messages that need to follow
                            output.extend(image_messages);
                        }
                        Err(e) => {
                            // A tool result error is shown as output so the model can interpret the error message
                            output.push(json!({
                                "role": "tool",
                                "content": format!("The tool call returned the following error:\n{}", e),
                                "tool_call_id": response.id
                            }));
                        }
                    }
                }
                MessageContent::ToolConfirmationRequest(_) => {}
                MessageContent::ActionRequired(_) => {}
                MessageContent::Image(image) => {
                    content_array.push(convert_image(image, image_format));
                }
                MessageContent::FrontendToolRequest(request) => match &request.tool_call {
                    Ok(tool_call) => {
                        let sanitized_name = sanitize_function_name(&tool_call.name);
                        let arguments_str = match &tool_call.arguments {
                            Some(args) => {
                                serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string())
                            }
                            None => "{}".to_string(),
                        };

                        let tool_calls = converted
                            .as_object_mut()
                            .unwrap()
                            .entry("tool_calls")
                            .or_insert(json!([]));

                        tool_calls.as_array_mut().unwrap().push(json!({
                            "id": request.id,
                            "type": "function",
                            "function": {
                                "name": sanitized_name,
                                "arguments": arguments_str,
                            }
                        }));
                    }
                    Err(e) => {
                        output.push(json!({
                            "role": "tool",
                            "content": format!("Error: {}", e),
                            "tool_call_id": request.id
                        }));
                    }
                },
            }
        }

        if !content_array.is_empty() {
            converted["content"] = json!(content_array);
        } else if !text_array.is_empty() {
            converted["content"] = json!(text_array.join("\n"));
        }

        if converted.get("content").is_some() || converted.get("tool_calls").is_some() {
            output.insert(0, converted);
        }

        messages_spec.extend(output);
    }

    messages_spec
}

pub fn format_tools(tools: &[Tool]) -> anyhow::Result<Vec<Value>> {
    let mut tool_names = std::collections::HashSet::new();
    let mut result = Vec::new();

    for tool in tools {
        if !tool_names.insert(&tool.name) {
            return Err(anyhow!("Duplicate tool name: {}", tool.name));
        }

        result.push(json!({
            "type": "function",
            "function": {
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.input_schema,
            }
        }));
    }

    Ok(result)
}

/// Convert OpenAI's API response to internal Message format
pub fn response_to_message(response: &Value) -> anyhow::Result<Message> {
    let Some(original) = response
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|m| m.get("message"))
    else {
        return Ok(Message::new(
            Role::Assistant,
            chrono::Utc::now().timestamp(),
            Vec::new(),
        ));
    };

    let mut content = Vec::new();

    if let Some(text) = original.get("content") {
        if let Some(text_str) = text.as_str() {
            content.push(MessageContent::text(text_str));
        }
    }

    if let Some(tool_calls) = original.get("tool_calls") {
        if let Some(tool_calls_array) = tool_calls.as_array() {
            for tool_call in tool_calls_array {
                let id = tool_call["id"].as_str().unwrap_or_default().to_string();
                let function_name = tool_call["function"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();

                // Get the raw arguments string from the LLM.
                let arguments_str = tool_call["function"]["arguments"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();

                // If arguments_str is empty, default to an empty JSON object string.
                let arguments_str = if arguments_str.is_empty() {
                    "{}".to_string()
                } else {
                    arguments_str
                };

                if !is_valid_function_name(&function_name) {
                    let error = ErrorData {
                        code: ErrorCode::INVALID_REQUEST,
                        message: Cow::from(format!(
                            "The provided function name '{}' had invalid characters, it must match this regex [a-zA-Z0-9_-]+",
                            function_name
                        )),
                        data: None,
                    };
                    content.push(MessageContent::tool_request(id, Err(error)));
                } else {
                    match safely_parse_json(&arguments_str) {
                        Ok(params) => {
                            content.push(MessageContent::tool_request(
                                id,
                                Ok(CallToolRequestParams {
                                    task: None,
                                    name: function_name.into(),
                                    arguments: Some(object(params)),
                                    meta: None,
                                }),
                            ));
                        }
                        Err(e) => {
                            let error = ErrorData {
                                code: ErrorCode::INVALID_PARAMS,
                                message: Cow::from(format!(
                                    "Could not interpret tool use parameters for id {}: {}. Raw arguments: '{}'",
                                    id, e, arguments_str
                                )),
                                data: None,
                            };
                            content.push(MessageContent::tool_request(id, Err(error)));
                        }
                    }
                }
            }
        }
    }

    // DeepSeek/Moonshot-style thinking: OpenAI-compatible hosts surface the
    // chain of thought as `message.reasoning_content`. Keep it on the tool
    // requests' metadata so providers that require it replayed (Moonshot
    // errors without it mid tool-loop) can restore it via
    // `add_reasoning_content_to_request`; for everyone else it is an inert
    // extra key. Mirrors `formats::openrouter::response_to_message`.
    if let Some(reasoning) = original
        .get("reasoning_content")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        for item in &mut content {
            if let MessageContent::ToolRequest(req) = item {
                let mut meta = req.metadata.clone().unwrap_or_default();
                meta.insert(REASONING_CONTENT_KEY.to_string(), json!(reasoning));
                req.metadata = Some(meta);
            }
        }
    }

    Ok(Message::new(
        Role::Assistant,
        chrono::Utc::now().timestamp(),
        content,
    ))
}

/// Stamp provider provenance onto every tool request that carries captured
/// `reasoning_content`. Called by the owning provider right after
/// [`response_to_message`] (and on each streamed message) — the format
/// decoders are shared across providers and cannot know who they decoded
/// for. Requests without captured reasoning are left untouched.
pub fn stamp_reasoning_provenance(message: &mut Message, provider_name: &str) {
    for item in &mut message.content {
        if let MessageContent::ToolRequest(req) = item {
            if let Some(meta) = req.metadata.as_mut() {
                if meta.contains_key(REASONING_CONTENT_KEY) {
                    meta.insert(REASONING_PROVIDER_KEY.to_string(), json!(provider_name));
                }
            }
        }
    }
}

/// Captured reasoning for replay, gated on provenance: only reasoning the
/// SAME provider captured (see [`REASONING_PROVIDER_KEY`]) is returned.
/// Unstamped or foreign-stamped captures yield `None` so another provider's
/// hidden chain of thought is never replayed across a provider switch.
fn get_reasoning_content(
    metadata: &Option<ProviderMetadata>,
    provider_name: &str,
) -> Option<String> {
    let meta = metadata.as_ref()?;
    let captured_by = meta.get(REASONING_PROVIDER_KEY)?.as_str()?;
    if captured_by != provider_name {
        return None;
    }
    meta.get(REASONING_CONTENT_KEY)?
        .as_str()
        .map(str::to_string)
}

/// Mirrors `formats::openrouter::has_assistant_content`: the same condition
/// under which `format_messages` emits an assistant entry into the payload,
/// so the index-matched walk in [`add_reasoning_content_to_request`] stays
/// aligned with the payload it patches.
fn message_has_assistant_content(message: &Message) -> bool {
    message.content.iter().any(|c| match c {
        MessageContent::Text(t) => !t.text.is_empty(),
        MessageContent::Image(_) => true,
        MessageContent::ToolRequest(req) => req.tool_call.is_ok(),
        MessageContent::FrontendToolRequest(req) => req.tool_call.is_ok(),
        _ => false,
    })
}

/// Replay captured `reasoning_content` onto the matching assistant messages
/// of an already-built Chat Completions payload.
///
/// Moonshot's Kimi docs (K2.6 quickstart, 2026-07): "During multi-step tool
/// calling, you must keep the `reasoning_content` from the assistant message
/// in the current turn's tool call within the context, otherwise an error
/// will be thrown" — and K2.7-Code forces thinking on every turn. This is
/// the request half of that contract; the capture half lives in
/// [`response_to_message`] / [`response_to_streaming_message`].
///
/// Callers must opt in per provider (see `OpenAiProvider`): the field is a
/// vendor extension, and some hosts that EMIT it (DeepSeek) reject it on
/// input. Twin of `formats::openrouter::add_reasoning_details_to_request`.
///
/// `provider_name` is the REQUESTING provider; only reasoning stamped with
/// the same provenance (see [`stamp_reasoning_provenance`]) is replayed, so
/// a session switched between providers never carries one provider's hidden
/// reasoning into another.
pub fn add_reasoning_content_to_request(
    payload: &mut Value,
    messages: &[Message],
    provider_name: &str,
) {
    let mut assistant_reasoning: Vec<Option<String>> = messages
        .iter()
        .filter(|m| m.is_agent_visible())
        .filter(|m| m.role == Role::Assistant)
        .filter(|m| message_has_assistant_content(m))
        .map(|message| {
            message.content.iter().find_map(|c| match c {
                MessageContent::ToolRequest(req) => {
                    get_reasoning_content(&req.metadata, provider_name)
                }
                _ => None,
            })
        })
        .collect();

    if let Some(payload_messages) = payload
        .as_object_mut()
        .and_then(|obj| obj.get_mut("messages"))
        .and_then(|m| m.as_array_mut())
    {
        let mut assistant_idx = 0;
        for payload_msg in payload_messages.iter_mut() {
            if payload_msg.get("role").and_then(|r| r.as_str()) == Some("assistant") {
                if assistant_idx < assistant_reasoning.len() {
                    if let Some(reasoning) = assistant_reasoning
                        .get_mut(assistant_idx)
                        .and_then(|d| d.take())
                    {
                        if let Some(obj) = payload_msg.as_object_mut() {
                            obj.insert(REASONING_CONTENT_KEY.to_string(), json!(reasoning));
                        }
                    }
                }
                assistant_idx += 1;
            }
        }
    }
}

/// Extract usage from an OpenAI-compatible `usage` object.
///
/// Per-provider semantics: OpenAI's `prompt_tokens` **already includes** the
/// cached prompt tokens — `prompt_tokens_details.cached_tokens` is a *subset*
/// of `prompt_tokens`, not an addition. To keep [`Usage`]'s buckets disjoint
/// (so [`Usage::billed_total`] is a plain sum), we subtract the cached count
/// back out of `input_tokens` and record it in `cache_read_input_tokens`.
/// OpenAI has no separate cache-write step, so `cache_creation` stays `None`.
/// `total_tokens` (prompt + completion) is unchanged. Providers that omit
/// `cached_tokens` (Ollama, most local servers) get `input_tokens = prompt`
/// and no cache buckets.
pub fn get_usage(usage: &Value) -> Usage {
    let prompt_tokens = usage
        .get("prompt_tokens")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);

    let output_tokens = usage
        .get("completion_tokens")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);

    // `cached_tokens` is a subset of `prompt_tokens`; clamp to it so a
    // malformed response can never drive `input_tokens` negative.
    let cached_tokens = usage
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|v| v.as_i64())
        .map(|v| v as i32)
        .filter(|&c| c > 0)
        .map(|c| c.min(prompt_tokens.unwrap_or(c)));

    // Fresh (non-cached) input = prompt_tokens - cached_tokens.
    let input_tokens = match (prompt_tokens, cached_tokens) {
        (Some(prompt), Some(cached)) => Some(prompt - cached),
        (prompt, _) => prompt,
    };

    let total_tokens = usage
        .get("total_tokens")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32)
        .or_else(|| match (prompt_tokens, output_tokens) {
            (Some(prompt), Some(output)) => Some(prompt + output),
            _ => None,
        });

    Usage::new(input_tokens, output_tokens, total_tokens).with_cache(cached_tokens, None)
}

/// Validates and fixes tool schemas to ensure they have proper parameter structure.
/// If parameters exist, ensures they have properties and required fields, or removes parameters entirely.
pub fn validate_tool_schemas(tools: &mut [Value]) {
    for tool in tools.iter_mut() {
        if let Some(function) = tool.get_mut("function") {
            if let Some(parameters) = function.get_mut("parameters") {
                if parameters.is_object() {
                    ensure_valid_json_schema(parameters);
                }
            }
        }
    }
}

/// Ensures that the given JSON value follows the expected JSON Schema structure.
fn ensure_valid_json_schema(schema: &mut Value) {
    if let Some(params_obj) = schema.as_object_mut() {
        // Check if this is meant to be an object type schema
        let is_object_type = params_obj
            .get("type")
            .and_then(|t| t.as_str())
            .is_none_or(|t| t == "object"); // Default to true if no type is specified

        // Only apply full schema validation to object types
        if is_object_type {
            // Ensure required fields exist with default values
            params_obj.entry("properties").or_insert_with(|| json!({}));
            params_obj.entry("required").or_insert_with(|| json!([]));
            params_obj.entry("type").or_insert_with(|| json!("object"));

            // Recursively validate properties if it exists
            if let Some(properties) = params_obj.get_mut("properties") {
                if let Some(properties_obj) = properties.as_object_mut() {
                    for (_key, prop) in properties_obj.iter_mut() {
                        if prop.is_object()
                            && prop.get("type").and_then(|t| t.as_str()) == Some("object")
                        {
                            ensure_valid_json_schema(prop);
                        }
                    }
                }
            }
        }
    }
}

fn strip_data_prefix(line: &str) -> Option<&str> {
    line.strip_prefix("data:").map(|s| s.trim())
}

#[allow(clippy::too_many_lines)]
pub fn response_to_streaming_message<S>(
    mut stream: S,
) -> impl Stream<Item = anyhow::Result<crate::providers::base::ProviderStreamItem>> + 'static
where
    S: Stream<Item = anyhow::Result<String>> + Unpin + Send + 'static,
{
    use crate::providers::base::PendingToolCall;
    /// See the twin constants in `formats::anthropic`. This decoder is the
    /// higher-traffic one: it drains the *entire* remaining stream before
    /// yielding anything, so without pending notifications the UI shows nothing
    /// at all until generation finishes.
    const PENDING_ARGS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);
    const PENDING_ARGS_CHARS: usize = 200;

    try_stream! {
        use futures::StreamExt;

        let mut accumulated_reasoning: Vec<Value> = Vec::new();
        let mut accumulated_reasoning_content = String::new();
        // Track the most recent finish_reason across chunks. MiMo (and other
        // OpenAI-compatible hosts) send finish_reason in one chunk and the usage
        // in a later `choices: []` chunk, so we remember it and attach it to the
        // ProviderUsage we emit — letting the agent loop tell a length-truncated
        // turn ("length") apart from a natural completion ("stop").
        let mut last_finish_reason: Option<String> = None;

        'outer: while let Some(response) = stream.next().await {
            let response_str = response?;
            let line = strip_data_prefix(&response_str);

            if line == Some("[DONE]") {
                break 'outer;
            }

            if line.is_none() || line.is_some_and(|l| l.is_empty()) {
                continue
            }

            let chunk = parse_streaming_chunk(line
                .ok_or_else(|| anyhow!("unexpected stream format"))?)?;

            if !chunk.choices.is_empty() {
                if let Some(details) = &chunk.choices[0].delta.reasoning_details {
                    accumulated_reasoning.extend(details.iter().cloned());
                }
                if let Some(reasoning) = &chunk.choices[0].delta.reasoning_content {
                    accumulated_reasoning_content.push_str(reasoning);
                }
                if let Some(reason) = &chunk.choices[0].finish_reason {
                    last_finish_reason = Some(reason.clone());
                }
            }

            let usage = chunk.usage.as_ref().and_then(|u| {
                chunk.model.as_ref().map(|model| {
                    ProviderUsage {
                        usage: get_usage(u),
                        model: model.clone(),
                        provider: None,
                        finish_reason: last_finish_reason.clone(),
                    }
                })
            });

            if chunk.choices.is_empty() {
                yield (None, usage, None)
            } else if chunk.choices[0].delta.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty()) {
                let mut tool_call_data: std::collections::HashMap<i32, (String, String, String)> = std::collections::HashMap::new();
                let mut tool_usage = usage;
                let mut tool_model = chunk.model.clone();
                let mut tool_finish_reason = chunk.choices[0].finish_reason.clone();
                let mut terminal_seen = false;

                // Per-index throttle state for pending-tool-call notifications:
                // (last emit instant, arg length at last emit).
                let mut pending_throttle: std::collections::HashMap<i32, (std::time::Instant, usize)> = std::collections::HashMap::new();
                // Announcements to emit after the borrow of `chunk` ends.
                let mut pending_announcements: Vec<PendingToolCall> = Vec::new();

                if let Some(tool_calls) = &chunk.choices[0].delta.tool_calls {
                    for tool_call in tool_calls {
                        if let (Some(index), Some(id), Some(name)) = (tool_call.index, &tool_call.id, &tool_call.function.name) {
                            tool_call_data.insert(index, (id.clone(), name.clone(), tool_call.function.arguments.clone()));
                            // Name is known; arguments are not. Announce now so
                            // the UI can draw a card instead of waiting for the
                            // whole stream to drain below. NOT a Message.
                            pending_throttle.insert(index, (std::time::Instant::now(), 0));
                            pending_announcements.push(PendingToolCall {
                                id: id.clone(),
                                name: name.clone(),
                                partial_args: None,
                            });
                        }
                    }
                }

                for announcement in pending_announcements.drain(..) {
                    yield (None, None, Some(announcement));
                }

                let is_complete = tool_finish_reason.is_some();

                if !is_complete {
                    let mut done = false;
                    while !done {
                        if let Some(response_chunk) = stream.next().await {
                            let response_str = response_chunk?;
                            if let Some(line) = strip_data_prefix(&response_str) {
                                if line == "[DONE]" {
                                    terminal_seen = true;
                                    break;
                                }
                                if line.is_empty() {
                                    continue;
                                }
                                let tool_chunk = parse_streaming_chunk(line)?;

                                if let Some(model) = tool_chunk.model.as_ref().filter(|model| !model.is_empty()) {
                                    tool_model = Some(model.clone());
                                }
                                if !tool_chunk.choices.is_empty() {
                                    if let Some(details) = &tool_chunk.choices[0].delta.reasoning_details {
                                        accumulated_reasoning.extend(details.iter().cloned());
                                    }
                                    if let Some(reasoning) = &tool_chunk.choices[0].delta.reasoning_content {
                                        accumulated_reasoning_content.push_str(reasoning);
                                    }
                                    if let Some(reason) = &tool_chunk.choices[0].finish_reason {
                                        last_finish_reason = Some(reason.clone());
                                        tool_finish_reason = Some(reason.clone());
                                    }
                                    if let Some(delta_tool_calls) = &tool_chunk.choices[0].delta.tool_calls {
                                        for delta_call in delta_tool_calls {
                                            if let Some(index) = delta_call.index {
                                                if let Some((id, name, ref mut args)) = tool_call_data.get_mut(&index) {
                                                    args.push_str(&delta_call.function.arguments);
                                                    // Throttled preview update. Never per delta.
                                                    let (last_at, last_len) = pending_throttle
                                                        .get(&index)
                                                        .copied()
                                                        .unwrap_or((std::time::Instant::now() - PENDING_ARGS_INTERVAL, 0));
                                                    if args.len().saturating_sub(last_len) >= PENDING_ARGS_CHARS
                                                        || last_at.elapsed() >= PENDING_ARGS_INTERVAL
                                                    {
                                                        pending_throttle.insert(index, (std::time::Instant::now(), args.len()));
                                                        pending_announcements.push(PendingToolCall {
                                                            id: id.clone(),
                                                            name: name.clone(),
                                                            partial_args: Some(args.clone()),
                                                        });
                                                    }
                                                } else if let (Some(id), Some(name)) = (&delta_call.id, &delta_call.function.name) {
                                                    tool_call_data.insert(index, (id.clone(), name.clone(), delta_call.function.arguments.clone()));
                                                    pending_throttle.insert(index, (std::time::Instant::now(), 0));
                                                    pending_announcements.push(PendingToolCall {
                                                        id: id.clone(),
                                                        name: name.clone(),
                                                        partial_args: None,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                    if tool_chunk.choices[0].finish_reason.is_some() {
                                        done = true;
                                    }
                                }

                                if let (Some(raw_usage), Some(model)) =
                                    (tool_chunk.usage.as_ref(), tool_model.as_ref())
                                {
                                    tool_usage = Some(ProviderUsage {
                                        usage: get_usage(raw_usage),
                                        model: model.clone(),
                                        provider: None,
                                        finish_reason: last_finish_reason.clone(),
                                    });
                                }
                            }
                        } else {
                            break;
                        }

                        // Flush this iteration's pending notifications. Emitted
                        // here, inside the drain loop, so the UI sees tool cards
                        // while the rest of the stream is still arriving —
                        // otherwise nothing reaches it until the loop exits.
                        for announcement in pending_announcements.drain(..) {
                            yield (None, None, Some(announcement));
                        }
                    }
                }

                let metadata: Option<ProviderMetadata> = if !accumulated_reasoning.is_empty()
                    || !accumulated_reasoning_content.is_empty()
                {
                    let mut map = ProviderMetadata::new();
                    if !accumulated_reasoning.is_empty() {
                        map.insert("reasoning_details".to_string(), json!(accumulated_reasoning));
                    }
                    if !accumulated_reasoning_content.is_empty() {
                        map.insert(
                            REASONING_CONTENT_KEY.to_string(),
                            json!(accumulated_reasoning_content),
                        );
                    }
                    Some(map)
                } else {
                    None
                };

                let mut contents = Vec::new();
                let mut sorted_indices: Vec<_> = tool_call_data.keys().cloned().collect();
                sorted_indices.sort();
                if let Some(usage) = &mut tool_usage {
                    usage.finish_reason = tool_finish_reason.clone();
                }

                for index in sorted_indices {
                    if let Some((id, function_name, arguments)) = tool_call_data.get(&index) {
                        // Parseable JSON alone does not establish completion: a
                        // gateway can end the body before the model finishes.
                        let parsed: Result<Value, ErrorData> = if !matches!(
                            tool_finish_reason.as_deref(),
                            Some("tool_calls" | "stop" | "function_call")
                        ) {
                            Err(ErrorData::new(
                                ErrorCode::INTERNAL_ERROR,
                                "Tool-call stream completion was not confirmed; the call was not executed. Emit a new, complete tool call.",
                                Some(json!({"biorouterToolCallFailure":"incomplete_stream"})),
                            ))
                        } else if arguments.trim().is_empty() {
                            Ok(json!({}))
                        } else {
                            serde_json::from_str::<Value>(arguments).map_err(|error| ErrorData::new(
                                ErrorCode::INVALID_PARAMS,
                                format!("Could not interpret tool use parameters for id {id}: {error}"),
                                Some(json!({"biorouterToolCallFailure":"invalid_arguments"})),
                            ))
                        }.and_then(|value| {
                            if value.is_object() {
                                Ok(value)
                            } else {
                                Err(ErrorData::new(
                                    ErrorCode::INVALID_PARAMS,
                                    "Tool arguments must be a JSON object; the call was not executed.",
                                    Some(json!({"biorouterToolCallFailure":"invalid_arguments"})),
                                ))
                            }
                        });

                        let content = match parsed {
                            Ok(params) => {
                                MessageContent::tool_request_with_metadata(
                                    id.clone(),
                                    Ok(CallToolRequestParams {
                                        task: None,
                                        name: function_name.clone().into(),
                                        arguments: Some(object(params)),
                                        meta: None,
                                    }),
                                    metadata.as_ref(),
                                )
                            },
                            Err(error) => {
                                MessageContent::tool_request_with_metadata(id.clone(), Err(error), metadata.as_ref())
                            }
                        };
                        contents.push(content);
                    }
                }

                let mut msg = Message::new(
                    Role::Assistant,
                    chrono::Utc::now().timestamp(),
                    contents,
                );

                // Add ID if present
                if let Some(id) = chunk.id {
                    msg = msg.with_id(id);
                }

                yield (
                    Some(msg),
                    tool_usage,
                    None,
                );
                if terminal_seen {
                    break 'outer;
                }
            } else if chunk.choices[0].delta.content.is_some() {
                let text = chunk.choices[0].delta.content.as_ref().unwrap();
                let mut msg = Message::new(
                    Role::Assistant,
                    chrono::Utc::now().timestamp(),
                    vec![MessageContent::text(text)],
                );

                // Add ID if present
                if let Some(id) = chunk.id {
                    msg = msg.with_id(id);
                }

                yield (
                    Some(msg),
                    usage,
                    None,
                )
            } else if usage.is_some() {
                yield (None, usage, None)
            }
        }
    }
}

/// OpenAI model families that accept a configurable reasoning effort.
///
/// Keep this separate from endpoint routing: GPT-4.1/4o support tools but are
/// non-reasoning models, while some reasoning models need the Responses API to
/// combine reasoning controls with function tools.
///
/// Every model this answers `true` for is also shaped as a reasoning model by
/// both request builders: no `temperature`, `max_completion_tokens` rather than
/// `max_tokens`, and a `developer` system role.
///
/// GPT-6 (Astra GA 2026-09-03, Sol and Luna GA 2026-09-22) is a reasoning
/// family: OpenAI's model pages list `reasoning.effort`, and the family rejects
/// `temperature`/`top_p` whenever the effort is not `none`. Astra accepts no
/// `none` at all; BioRouter never sends it (Quick/Deep map to low/high), so the
/// one prefix covers all three.
pub(crate) fn model_supports_reasoning_effort(model_name: &str) -> bool {
    let model_name = model_name.to_ascii_lowercase();
    model_name.starts_with("o1")
        || model_name.starts_with("o2")
        || model_name.starts_with("o3")
        || model_name.starts_with("o4")
        || model_name.starts_with("gpt-5")
        || model_name.starts_with("gpt-6")
}

/// Return an effort accepted by the selected model.
///
/// BioRouter's Quick/Deep control maps to low/high. GPT-5 Pro only accepts
/// high, while the newer numbered Pro variants start at medium; clamp Quick to
/// each model's documented minimum instead of sending a request that will 400.
pub(crate) fn model_reasoning_effort(
    model_name: &str,
    requested: &'static str,
) -> Option<&'static str> {
    if !model_supports_reasoning_effort(model_name) {
        return None;
    }

    let model_name = model_name.to_ascii_lowercase();
    if model_name == "gpt-5-pro" || model_name.starts_with("gpt-5-pro-") {
        return Some("high");
    }

    if requested == "low"
        && (model_name.starts_with("gpt-5.2-pro")
            || model_name.starts_with("gpt-5.4-pro")
            || model_name.starts_with("gpt-5.5-pro"))
    {
        return Some("medium");
    }

    Some(requested)
}

/// Models routed through Responses by BioRouter.
///
/// `o4-mini` supports Chat Completions in isolation, but OpenAI rejects the
/// function-tools + `reasoning_effort` combination there. Responses supports
/// that combination and is also the preferred endpoint for reasoning models.
///
/// GPT-6 is in the same position, and says so on every one of its model pages
/// (developers.openai.com/api/docs/models/gpt-6-sol, read 2026-09-25): "Use the
/// Responses API for built-in tools and function calling. Chat Completions
/// supports function calling only with reasoning_effort set to none." BioRouter
/// sends tools on nearly every turn and never sends `none`, so on Chat
/// Completions every tool-bearing GPT-6 turn would be refused.
///
/// Both the OpenAI and the Azure OpenAI provider route by this one predicate.
pub(crate) fn model_uses_responses_api(model_name: &str) -> bool {
    let model_name = model_name.to_ascii_lowercase();
    model_name.starts_with("gpt-5-codex")
        || model_name.starts_with("gpt-5.1-codex")
        || model_name.starts_with("gpt-5.3-codex")
        || model_name.starts_with("gpt-5.4")
        || model_name.starts_with("gpt-5.5")
        || model_name.starts_with("gpt-5.6")
        || model_name.starts_with("gpt-6")
        || model_name.starts_with("o3-pro")
        || model_name.starts_with("o4-mini")
}

pub fn create_request(
    model_config: &ModelConfig,
    system: &str,
    messages: &[Message],
    tools: &[Tool],
    image_format: &ImageFormat,
    for_streaming: bool,
) -> anyhow::Result<Value, Error> {
    if model_config.model_name.starts_with("o1-mini") {
        return Err(anyhow!(
            "o1-mini model is not currently supported since biorouter uses tool calling and o1-mini does not support it. Please use o1 or o3 models instead."
        ));
    }

    // Endpoint routing belongs to the provider: Versa uses Chat Completions for
    // GPT-5 models that the public OpenAI provider routes through Responses.
    let is_ox_model = model_supports_reasoning_effort(&model_config.model_name);

    // Extract reasoning effort only for reasoning-capable Chat Completions models.
    let (model_name, mut reasoning_effort) = if is_ox_model {
        let parts: Vec<&str> = model_config.model_name.split('-').collect();
        let last_part = parts.last().unwrap();

        match *last_part {
            "low" | "medium" | "high" => {
                let base_name = parts[..parts.len() - 1].join("-");
                (base_name, Some(last_part.to_string()))
            }
            _ => (
                model_config.model_name.to_string(),
                Some("medium".to_string()),
            ),
        }
    } else {
        // For non-O family models, use the model name as is and no reasoning effort
        (model_config.model_name.to_string(), None)
    };

    // BR-63: an explicit per-turn effort (quick/deep) outranks the effort implied
    // by the model name, but only for models that actually accept the parameter —
    // sending `reasoning_effort` to a non-reasoning model is a 400.
    if is_ox_model {
        if let Some(effort) = model_config
            .reasoning_effort
            .and_then(|effort| effort.provider_effort())
            .and_then(|effort| model_reasoning_effort(&model_config.model_name, effort))
        {
            reasoning_effort = Some(effort.to_string());
        }
    }

    let system_message = json!({
        "role": if is_ox_model { "developer" } else { "system" },
        "content": system
    });

    let messages_spec = format_messages(messages, image_format);
    let mut tools_spec = format_tools(tools)?;

    validate_tool_schemas(&mut tools_spec);

    let mut messages_array = vec![system_message];
    messages_array.extend(messages_spec);

    let mut payload = json!({
        "model": model_name,
        "messages": messages_array
    });

    if let Some(effort) = reasoning_effort {
        payload["reasoning_effort"] = json!(effort);
    }

    if !tools_spec.is_empty() {
        payload["tools"] = json!(tools_spec);
    }

    // o1, o3 models currently don't support temperature
    if !is_ox_model {
        if let Some(temp) = model_config.temperature {
            payload["temperature"] = json!(temp);
        }
    }

    // Reasoning/GPT-5 chat-completions models use max_completion_tokens instead of max_tokens.
    if let Some(tokens) = model_config.max_tokens {
        let key = if is_ox_model {
            "max_completion_tokens"
        } else {
            "max_tokens"
        };
        payload
            .as_object_mut()
            .unwrap()
            .insert(key.to_string(), json!(tokens));
    }

    if for_streaming {
        payload["stream"] = json!(true);
        payload["stream_options"] = json!({"include_usage": true});
    }

    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::effort::ReasoningEffort;
    use crate::conversation::message::Message;
    use rmcp::model::CallToolResult;
    use rmcp::object;
    use serde_json::json;
    use tokio::pin;
    use tokio_stream::{self, StreamExt};

    // === reasoning_content passthrough (Moonshot/DeepSeek-style thinking) ===

    #[test]
    fn test_response_to_message_captures_reasoning_content() -> anyhow::Result<()> {
        let response = json!({
            "choices": [{
                "message": {
                    "content": null,
                    "reasoning_content": "I should list the files first.",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "shell", "arguments": "{\"command\": \"ls\"}"}
                    }]
                }
            }]
        });

        let message = response_to_message(&response)?;
        let req = message
            .content
            .iter()
            .find_map(|c| match c {
                MessageContent::ToolRequest(req) => Some(req),
                _ => None,
            })
            .expect("tool request decoded");
        let meta = req.metadata.as_ref().expect("reasoning captured");
        assert_eq!(
            meta.get(REASONING_CONTENT_KEY).and_then(|v| v.as_str()),
            Some("I should list the files first.")
        );

        // A response without the field must stay metadata-free — the key is
        // only ever present when the host actually emitted reasoning.
        let plain = json!({
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "call_2",
                        "type": "function",
                        "function": {"name": "shell", "arguments": "{}"}
                    }]
                }
            }]
        });
        let message = response_to_message(&plain)?;
        let req = message
            .content
            .iter()
            .find_map(|c| match c {
                MessageContent::ToolRequest(req) => Some(req),
                _ => None,
            })
            .unwrap();
        assert!(req.metadata.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_streaming_accumulates_reasoning_content_onto_tool_requests() -> anyhow::Result<()>
    {
        // Kimi K2.7-Code shape: thinking arrives as `delta.reasoning_content`
        // string chunks before (and interleaved with) the tool call. The
        // decoder must concatenate them onto the authoritative ToolRequest's
        // metadata so the provider can replay them next turn.
        let lines = r#"
data: {"model":"kimi-k2.7-code","choices":[{"delta":{"role":"assistant","reasoning_content":"Let me "},"index":0,"finish_reason":null}],"object":"chat.completion.chunk","id":"x"}
data: {"model":"kimi-k2.7-code","choices":[{"delta":{"reasoning_content":"check the files."},"index":0,"finish_reason":null}],"object":"chat.completion.chunk","id":"x"}
data: {"model":"kimi-k2.7-code","choices":[{"delta":{"content":null,"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"shell","arguments":""}}]},"index":0,"finish_reason":null}],"object":"chat.completion.chunk","id":"x"}
data: {"model":"kimi-k2.7-code","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"command\": \"ls\"}"}}]},"index":0,"finish_reason":null}],"object":"chat.completion.chunk","id":"x"}
data: {"model":"kimi-k2.7-code","choices":[{"delta":{"content":""},"index":0,"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15},"object":"chat.completion.chunk","id":"x"}
data: [DONE]
"#;
        let response_stream = tokio_stream::iter(lines.lines().map(|l| Ok(l.to_string())));
        let messages = response_to_streaming_message(response_stream);
        pin!(messages);

        let mut captured: Option<String> = None;
        while let Some(Ok((message, _usage, _pending))) = messages.next().await {
            if let Some(msg) = message {
                for content in &msg.content {
                    if let MessageContent::ToolRequest(req) = content {
                        captured = req
                            .metadata
                            .as_ref()
                            .and_then(|m| m.get(REASONING_CONTENT_KEY))
                            .and_then(|v| v.as_str())
                            .map(str::to_string);
                    }
                }
            }
        }

        assert_eq!(captured.as_deref(), Some("Let me check the files."));
        Ok(())
    }

    #[test]
    fn test_add_reasoning_content_to_request_replays_onto_matching_assistant() -> anyhow::Result<()>
    {
        let mut meta = ProviderMetadata::new();
        meta.insert(REASONING_CONTENT_KEY.to_string(), json!("thought hard"));
        meta.insert(REASONING_PROVIDER_KEY.to_string(), json!("moonshot"));
        let tool_call = CallToolRequestParams {
            task: None,
            name: "shell".into(),
            arguments: Some(object!({"command": "ls"})),
            meta: None,
        };

        let messages = vec![
            Message::user().with_text("hi"),
            Message::assistant().with_content(MessageContent::tool_request_with_metadata(
                "call_1",
                Ok(tool_call),
                Some(&meta),
            )),
            // A later assistant message with no captured reasoning must not
            // receive the field.
            Message::assistant().with_text("done"),
        ];

        let model_config = ModelConfig::new_or_fail("kimi-k2.7-code");
        let mut payload = create_request(
            &model_config,
            "system",
            &messages,
            &[],
            &ImageFormat::OpenAi,
            false,
        )?;
        add_reasoning_content_to_request(&mut payload, &messages, "moonshot");

        let payload_msgs = payload["messages"].as_array().unwrap();
        let assistant_msgs: Vec<&Value> = payload_msgs
            .iter()
            .filter(|m| m["role"] == "assistant")
            .collect();
        assert_eq!(assistant_msgs.len(), 2);
        assert_eq!(
            assistant_msgs[0]["reasoning_content"],
            json!("thought hard"),
            "the tool-calling assistant message carries its reasoning back"
        );
        assert!(
            assistant_msgs[1].get("reasoning_content").is_none(),
            "an assistant message without captured reasoning stays untouched"
        );
        // Non-assistant roles are never patched.
        for msg in payload_msgs.iter().filter(|m| m["role"] != "assistant") {
            assert!(msg.get("reasoning_content").is_none());
        }
        Ok(())
    }

    #[test]
    fn test_add_reasoning_content_is_a_noop_without_captured_metadata() -> anyhow::Result<()> {
        // The self-gating property: a history produced by a host that never
        // emits reasoning_content (OpenAI proper, Groq, ...) leaves the
        // payload byte-identical.
        let messages = vec![
            Message::user().with_text("hi"),
            Message::assistant().with_text("hello"),
        ];
        let model_config = ModelConfig::new_or_fail("gpt-4.1");
        let payload = create_request(
            &model_config,
            "system",
            &messages,
            &[],
            &ImageFormat::OpenAi,
            false,
        )?;
        let mut patched = payload.clone();
        add_reasoning_content_to_request(&mut patched, &messages, "moonshot");
        assert_eq!(patched, payload);
        Ok(())
    }

    #[test]
    fn test_add_reasoning_content_drops_cross_provider_capture() -> anyhow::Result<()> {
        // Reasoning captured by ANOTHER provider must never be replayed: a
        // session switched from DeepSeek to Moonshot would otherwise leak
        // DeepSeek's hidden chain of thought into Moonshot's context.
        let build_messages = |meta: &ProviderMetadata| {
            vec![
                Message::user().with_text("hi"),
                Message::assistant().with_content(MessageContent::tool_request_with_metadata(
                    "call_1",
                    Ok(CallToolRequestParams {
                        task: None,
                        name: "shell".into(),
                        arguments: Some(object!({"command": "ls"})),
                        meta: None,
                    }),
                    Some(meta),
                )),
            ]
        };
        let model_config = ModelConfig::new_or_fail("kimi-k2.7-code");

        // Foreign provenance: stamped by DeepSeek, requested by Moonshot.
        let mut foreign = ProviderMetadata::new();
        foreign.insert(
            REASONING_CONTENT_KEY.to_string(),
            json!("deepseek thoughts"),
        );
        foreign.insert(REASONING_PROVIDER_KEY.to_string(), json!("custom_deepseek"));
        let messages = build_messages(&foreign);
        let mut payload = create_request(
            &model_config,
            "system",
            &messages,
            &[],
            &ImageFormat::OpenAi,
            false,
        )?;
        let before = payload.clone();
        add_reasoning_content_to_request(&mut payload, &messages, "moonshot");
        assert_eq!(payload, before, "foreign-stamped reasoning must be dropped");

        // Missing provenance (captured by a provider that never stamps, or by
        // a build predating the stamp): also dropped — fail closed.
        let mut unstamped = ProviderMetadata::new();
        unstamped.insert(REASONING_CONTENT_KEY.to_string(), json!("legacy thoughts"));
        let messages = build_messages(&unstamped);
        let mut payload = create_request(
            &model_config,
            "system",
            &messages,
            &[],
            &ImageFormat::OpenAi,
            false,
        )?;
        let before = payload.clone();
        add_reasoning_content_to_request(&mut payload, &messages, "moonshot");
        assert_eq!(payload, before, "unstamped reasoning must be dropped");
        Ok(())
    }

    #[test]
    fn test_stamp_reasoning_provenance_marks_only_captured_requests() -> anyhow::Result<()> {
        let mut meta = ProviderMetadata::new();
        meta.insert(REASONING_CONTENT_KEY.to_string(), json!("thoughts"));
        let tool_call = || CallToolRequestParams {
            task: None,
            name: "shell".into(),
            arguments: Some(object!({"command": "ls"})),
            meta: None,
        };
        let mut message = Message::assistant()
            .with_content(MessageContent::tool_request_with_metadata(
                "call_1",
                Ok(tool_call()),
                Some(&meta),
            ))
            .with_content(MessageContent::tool_request("call_2", Ok(tool_call())));

        stamp_reasoning_provenance(&mut message, "moonshot");

        let requests: Vec<_> = message
            .content
            .iter()
            .filter_map(|c| match c {
                MessageContent::ToolRequest(req) => Some(req),
                _ => None,
            })
            .collect();
        assert_eq!(requests.len(), 2);
        let stamped = requests[0].metadata.as_ref().unwrap();
        assert_eq!(
            stamped.get(REASONING_PROVIDER_KEY).and_then(|v| v.as_str()),
            Some("moonshot"),
            "the captured request gains provenance"
        );
        assert!(
            requests[1].metadata.is_none(),
            "a request without captured reasoning stays metadata-free"
        );

        // And the stamped capture round-trips through the replay gate.
        let messages = vec![Message::user().with_text("hi"), message];
        let model_config = ModelConfig::new_or_fail("kimi-k2.7-code");
        let mut payload = create_request(
            &model_config,
            "system",
            &messages,
            &[],
            &ImageFormat::OpenAi,
            false,
        )?;
        add_reasoning_content_to_request(&mut payload, &messages, "moonshot");
        let assistant = payload["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["role"] == "assistant")
            .unwrap();
        assert_eq!(assistant["reasoning_content"], json!("thoughts"));
        Ok(())
    }

    #[test]
    fn get_usage_subtracts_cached_tokens_from_prompt() {
        // prompt_tokens=1000 INCLUDES 800 cached. Fresh input must be 200.
        let usage = json!({
            "prompt_tokens": 1000,
            "completion_tokens": 50,
            "total_tokens": 1050,
            "prompt_tokens_details": { "cached_tokens": 800 }
        });
        let u = get_usage(&usage);
        assert_eq!(u.input_tokens, Some(200));
        assert_eq!(u.output_tokens, Some(50));
        assert_eq!(u.cache_read_input_tokens, Some(800));
        assert_eq!(u.cache_creation_input_tokens, None); // OpenAI has no cache-write
        assert_eq!(u.total_tokens, Some(1050));
        // Billed reconciles: 200 + 50 + 800 = 1050 = prompt + completion.
        assert_eq!(u.billed_total(), Some(1050));
    }

    #[test]
    fn get_usage_without_cache_details_leaves_input_as_prompt() {
        let usage = json!({
            "prompt_tokens": 300,
            "completion_tokens": 20,
            "total_tokens": 320
        });
        let u = get_usage(&usage);
        assert_eq!(u.input_tokens, Some(300));
        assert_eq!(u.cache_read_input_tokens, None);
        assert_eq!(u.cache_creation_input_tokens, None);
        assert_eq!(u.billed_total(), Some(320));
    }

    #[test]
    fn get_usage_ignores_zero_cached_tokens() {
        // cached_tokens: 0 is not a cache hit — leave input untouched, cache None.
        let usage = json!({
            "prompt_tokens": 300,
            "completion_tokens": 20,
            "total_tokens": 320,
            "prompt_tokens_details": { "cached_tokens": 0 }
        });
        let u = get_usage(&usage);
        assert_eq!(u.input_tokens, Some(300));
        assert_eq!(u.cache_read_input_tokens, None);
    }

    #[test]
    fn get_usage_clamps_cached_tokens_to_prompt() {
        // Defensive: a malformed cached > prompt must not make input negative.
        let usage = json!({
            "prompt_tokens": 100,
            "completion_tokens": 10,
            "prompt_tokens_details": { "cached_tokens": 250 }
        });
        let u = get_usage(&usage);
        assert_eq!(u.input_tokens, Some(0));
        assert_eq!(u.cache_read_input_tokens, Some(100));
    }

    #[test]
    fn test_validate_tool_schemas() {
        // Test case 1: Empty parameters object
        // Input JSON with an incomplete parameters object
        let mut actual = vec![json!({
            "type": "function",
            "function": {
                "name": "test_func",
                "description": "test description",
                "parameters": {
                    "type": "object"
                }
            }
        })];

        // Run the function to validate and update schemas
        validate_tool_schemas(&mut actual);

        // Expected JSON after validation
        let expected = vec![json!({
            "type": "function",
            "function": {
                "name": "test_func",
                "description": "test description",
                "parameters": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            }
        })];

        // Compare entire JSON structures instead of individual fields
        assert_eq!(actual, expected);

        // Test case 2: Missing type field
        let mut tools = vec![json!({
            "type": "function",
            "function": {
                "name": "test_func",
                "description": "test description",
                "parameters": {
                    "properties": {}
                }
            }
        })];

        validate_tool_schemas(&mut tools);

        let params = tools[0]["function"]["parameters"].as_object().unwrap();
        assert_eq!(params["type"], "object");

        // Test case 3: Complete valid schema should remain unchanged
        let original_schema = json!({
            "type": "function",
            "function": {
                "name": "test_func",
                "description": "test description",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "location": {
                            "type": "string",
                            "description": "City and country"
                        }
                    },
                    "required": ["location"]
                }
            }
        });

        let mut tools = vec![original_schema.clone()];
        validate_tool_schemas(&mut tools);
        assert_eq!(tools[0], original_schema);
    }

    const OPENAI_TOOL_USE_RESPONSE: &str = r#"{
        "choices": [{
            "role": "assistant",
            "message": {
                "tool_calls": [{
                    "id": "1",
                    "function": {
                        "name": "example_fn",
                        "arguments": "{\"param\": \"value\"}"
                    }
                }]
            }
        }],
        "usage": {
            "input_tokens": 10,
            "output_tokens": 25,
            "total_tokens": 35
        }
    }"#;

    #[test]
    fn test_format_messages() -> anyhow::Result<()> {
        let message = Message::user().with_text("Hello");
        let spec = format_messages(&[message], &ImageFormat::OpenAi);

        assert_eq!(spec.len(), 1);
        assert_eq!(spec[0]["role"], "user");
        assert_eq!(spec[0]["content"], "Hello");
        Ok(())
    }

    #[test]
    fn test_format_tools() -> anyhow::Result<()> {
        let tool = Tool::new(
            "test_tool",
            "A test tool",
            object!({
                "type": "object",
                "properties": {
                    "input": {
                        "type": "string",
                        "description": "Test parameter"
                    }
                },
                "required": ["input"]
            }),
        );

        let spec = format_tools(&[tool])?;

        assert_eq!(spec.len(), 1);
        assert_eq!(spec[0]["type"], "function");
        assert_eq!(spec[0]["function"]["name"], "test_tool");
        Ok(())
    }

    #[test]
    fn test_format_messages_complex() -> anyhow::Result<()> {
        let mut messages = vec![
            Message::assistant().with_text("Hello!"),
            Message::user().with_text("How are you?"),
            Message::assistant().with_tool_request(
                "tool1",
                Ok(CallToolRequestParams {
                    task: None,
                    name: "example".into(),
                    arguments: Some(object!({"param1": "value1"})),
                    meta: None,
                }),
            ),
        ];

        // Get the ID from the tool request to use in the response
        let tool_id = if let MessageContent::ToolRequest(request) = &messages[2].content[0] {
            request.id.clone()
        } else {
            panic!("should be tool request");
        };

        messages.push(Message::user().with_tool_response(
            tool_id,
            Ok(CallToolResult {
                content: vec![Content::text("Result")],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        ));

        let spec = format_messages(&messages, &ImageFormat::OpenAi);

        assert_eq!(spec.len(), 4);
        assert_eq!(spec[0]["role"], "assistant");
        assert_eq!(spec[0]["content"], "Hello!");
        assert_eq!(spec[1]["role"], "user");
        assert_eq!(spec[1]["content"], "How are you?");
        assert_eq!(spec[2]["role"], "assistant");
        assert!(spec[2]["tool_calls"].is_array());
        assert_eq!(spec[3]["role"], "tool");
        assert_eq!(spec[3]["content"], "Result");
        assert_eq!(spec[3]["tool_call_id"], spec[2]["tool_calls"][0]["id"]);

        Ok(())
    }

    #[test]
    fn test_format_messages_multiple_content() -> anyhow::Result<()> {
        let mut messages = vec![Message::assistant().with_tool_request(
            "tool1",
            Ok(CallToolRequestParams {
                task: None,
                name: "example".into(),
                arguments: Some(object!({"param1": "value1"})),
                meta: None,
            }),
        )];

        // Get the ID from the tool request to use in the response
        let tool_id = if let MessageContent::ToolRequest(request) = &messages[0].content[0] {
            request.id.clone()
        } else {
            panic!("should be tool request");
        };

        messages.push(Message::user().with_tool_response(
            tool_id,
            Ok(CallToolResult {
                content: vec![Content::text("Result")],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        ));

        let spec = format_messages(&messages, &ImageFormat::OpenAi);

        assert_eq!(spec.len(), 2);
        assert_eq!(spec[0]["role"], "assistant");
        assert!(spec[0]["tool_calls"].is_array());
        assert_eq!(spec[1]["role"], "tool");
        assert_eq!(spec[1]["content"], "Result");
        assert_eq!(spec[1]["tool_call_id"], spec[0]["tool_calls"][0]["id"]);

        Ok(())
    }

    /// Every audience case, through the real OpenAI formatter.
    ///
    /// The joined tool message must carry the three blocks the tool addressed
    /// to the model and neither of the two it did not. `delta-both-audiences`
    /// is the one that separates this filter from "the user is not in the
    /// audience", which Bedrock used to carry and which drops it.
    #[test]
    fn tool_result_blocks_reach_the_model_by_audience() {
        let message = Message::user().with_tool_response(
            "call-1",
            Ok(CallToolResult {
                content: crate::providers::formats::audience::every_audience_case(),
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        );

        let spec = format_messages(&[message], &ImageFormat::OpenAi);
        let sent = spec[0]["content"]
            .as_str()
            .expect("tool content is a string");

        assert_eq!(
            sent,
            crate::providers::formats::audience::MODEL_VISIBLE.join(" "),
            "the OpenAI tool message must carry exactly the model-addressed blocks"
        );
        for withheld in crate::providers::formats::audience::MODEL_HIDDEN {
            assert!(!sent.contains(withheld), "{withheld} reached the model");
        }
    }

    #[test]
    fn test_format_tools_duplicate() -> anyhow::Result<()> {
        let tool1 = Tool::new(
            "test_tool",
            "Test tool",
            object!({
                "type": "object",
                "properties": {
                    "input": {
                        "type": "string",
                        "description": "Test parameter"
                    }
                },
                "required": ["input"]
            }),
        );

        let tool2 = Tool::new(
            "test_tool",
            "Test tool",
            object!({
                "type": "object",
                "properties": {
                    "input": {
                        "type": "string",
                        "description": "Test parameter"
                    }
                },
                "required": ["input"]
            }),
        );

        let result = format_tools(&[tool1, tool2]);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Duplicate tool name"));

        Ok(())
    }

    #[test]
    fn test_format_tools_empty() -> anyhow::Result<()> {
        let spec = format_tools(&[])?;
        assert!(spec.is_empty());
        Ok(())
    }

    #[test]
    fn test_format_messages_with_image_path() -> anyhow::Result<()> {
        // Create a temporary PNG file with valid PNG magic numbers
        let temp_dir = tempfile::tempdir()?;
        let png_path = temp_dir.path().join("test.png");
        let png_data = [
            0x89, 0x50, 0x4E, 0x47, // PNG magic number
            0x0D, 0x0A, 0x1A, 0x0A, // PNG header
            0x00, 0x00, 0x00, 0x0D, // Rest of fake PNG data
        ];
        std::fs::write(&png_path, png_data)?;
        let png_path_str = png_path.to_str().unwrap();

        // Create message with image path
        let message = Message::user().with_text(format!("Here is an image: {}", png_path_str));
        let spec = format_messages(&[message], &ImageFormat::OpenAi);

        assert_eq!(spec.len(), 1);
        assert_eq!(spec[0]["role"], "user");

        // Content should be an array with text and image
        let content = spec[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert!(content[0]["text"].as_str().unwrap().contains(png_path_str));
        assert_eq!(content[1]["type"], "image_url");
        assert!(content[1]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));

        Ok(())
    }

    #[test]
    fn test_response_to_message_text() -> anyhow::Result<()> {
        let response = json!({
            "choices": [{
                "role": "assistant",
                "message": {
                    "content": "Hello from John Cena!"
                }
            }],
            "usage": {
                "input_tokens": 10,
                "output_tokens": 25,
                "total_tokens": 35
            }
        });

        let message = response_to_message(&response)?;
        assert_eq!(message.content.len(), 1);
        if let MessageContent::Text(text) = &message.content[0] {
            assert_eq!(text.text, "Hello from John Cena!");
        } else {
            panic!("Expected Text content");
        }
        assert!(matches!(message.role, Role::Assistant));

        Ok(())
    }

    #[test]
    fn test_response_to_message_valid_toolrequest() -> anyhow::Result<()> {
        let response: Value = serde_json::from_str(OPENAI_TOOL_USE_RESPONSE)?;
        let message = response_to_message(&response)?;

        assert_eq!(message.content.len(), 1);
        if let MessageContent::ToolRequest(request) = &message.content[0] {
            let tool_call = request.tool_call.as_ref().unwrap();
            assert_eq!(tool_call.name, "example_fn");
            assert_eq!(tool_call.arguments, Some(object!({"param": "value"})));
        } else {
            panic!("Expected ToolRequest content");
        }

        Ok(())
    }

    #[test]
    fn test_response_to_message_invalid_func_name() -> anyhow::Result<()> {
        let mut response: Value = serde_json::from_str(OPENAI_TOOL_USE_RESPONSE)?;
        response["choices"][0]["message"]["tool_calls"][0]["function"]["name"] =
            json!("invalid fn");

        let message = response_to_message(&response)?;

        if let MessageContent::ToolRequest(request) = &message.content[0] {
            match &request.tool_call {
                Err(ErrorData {
                    code: ErrorCode::INVALID_REQUEST,
                    message: msg,
                    data: None,
                }) => {
                    assert!(msg.starts_with("The provided function name"));
                }
                _ => panic!("Expected ToolNotFound error"),
            }
        } else {
            panic!("Expected ToolRequest content");
        }

        Ok(())
    }

    #[test]
    fn test_response_to_message_json_decode_error() -> anyhow::Result<()> {
        let mut response: Value = serde_json::from_str(OPENAI_TOOL_USE_RESPONSE)?;
        response["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"] =
            json!("invalid json {");

        let message = response_to_message(&response)?;

        if let MessageContent::ToolRequest(request) = &message.content[0] {
            match &request.tool_call {
                Err(ErrorData {
                    code: ErrorCode::INVALID_PARAMS,
                    message: msg,
                    data: None,
                }) => {
                    assert!(msg.starts_with("Could not interpret tool use parameters"));
                }
                _ => panic!("Expected InvalidParameters error"),
            }
        } else {
            panic!("Expected ToolRequest content");
        }

        Ok(())
    }

    #[test]
    fn test_response_to_message_empty_argument() -> anyhow::Result<()> {
        let mut response: Value = serde_json::from_str(OPENAI_TOOL_USE_RESPONSE)?;
        response["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"] =
            serde_json::Value::String("".to_string());

        let message = response_to_message(&response)?;

        if let MessageContent::ToolRequest(request) = &message.content[0] {
            let tool_call = request.tool_call.as_ref().unwrap();
            assert_eq!(tool_call.name, "example_fn");
            assert_eq!(tool_call.arguments, Some(object!({})));
        } else {
            panic!("Expected ToolRequest content");
        }

        Ok(())
    }

    #[test]
    fn test_format_messages_tool_request_with_none_arguments() -> anyhow::Result<()> {
        // Test that tool calls with None arguments are formatted as "{}" string
        let message = Message::assistant().with_tool_request(
            "tool1",
            Ok(CallToolRequestParams {
                task: None,
                name: "test_tool".into(),
                arguments: None, // This is the key case the fix addresses
                meta: None,
            }),
        );

        let spec = format_messages(&[message], &ImageFormat::OpenAi);

        assert_eq!(spec.len(), 1);
        assert_eq!(spec[0]["role"], "assistant");
        assert!(spec[0]["tool_calls"].is_array());

        let tool_call = &spec[0]["tool_calls"][0];
        assert_eq!(tool_call["id"], "tool1");
        assert_eq!(tool_call["type"], "function");
        assert_eq!(tool_call["function"]["name"], "test_tool");
        // This should be the string "{}", not null
        assert_eq!(tool_call["function"]["arguments"], "{}");

        Ok(())
    }

    #[test]
    fn test_format_messages_tool_request_with_some_arguments() -> anyhow::Result<()> {
        // Test that tool calls with Some arguments are properly JSON-serialized
        let message = Message::assistant().with_tool_request(
            "tool1",
            Ok(CallToolRequestParams {
                task: None,
                name: "test_tool".into(),
                arguments: Some(object!({"param": "value", "number": 42})),
                meta: None,
            }),
        );

        let spec = format_messages(&[message], &ImageFormat::OpenAi);

        assert_eq!(spec.len(), 1);
        assert_eq!(spec[0]["role"], "assistant");
        assert!(spec[0]["tool_calls"].is_array());

        let tool_call = &spec[0]["tool_calls"][0];
        assert_eq!(tool_call["id"], "tool1");
        assert_eq!(tool_call["type"], "function");
        assert_eq!(tool_call["function"]["name"], "test_tool");
        // This should be a JSON string representation
        let args_str = tool_call["function"]["arguments"].as_str().unwrap();
        let parsed_args: Value = serde_json::from_str(args_str)?;
        assert_eq!(parsed_args["param"], "value");
        assert_eq!(parsed_args["number"], 42);

        Ok(())
    }

    #[test]
    fn test_format_messages_frontend_tool_request_with_none_arguments() -> anyhow::Result<()> {
        // Test that FrontendToolRequest with None arguments are formatted as "{}" string
        let message = Message::assistant().with_frontend_tool_request(
            "frontend_tool1",
            Ok(CallToolRequestParams {
                task: None,
                name: "frontend_test_tool".into(),
                arguments: None, // This is the key case the fix addresses
                meta: None,
            }),
        );

        let spec = format_messages(&[message], &ImageFormat::OpenAi);

        assert_eq!(spec.len(), 1);
        assert_eq!(spec[0]["role"], "assistant");
        assert!(spec[0]["tool_calls"].is_array());

        let tool_call = &spec[0]["tool_calls"][0];
        assert_eq!(tool_call["id"], "frontend_tool1");
        assert_eq!(tool_call["type"], "function");
        assert_eq!(tool_call["function"]["name"], "frontend_test_tool");
        // This should be the string "{}", not null
        assert_eq!(tool_call["function"]["arguments"], "{}");

        Ok(())
    }

    #[test]
    fn test_format_messages_frontend_tool_request_with_some_arguments() -> anyhow::Result<()> {
        // Test that FrontendToolRequest with Some arguments are properly JSON-serialized
        let message = Message::assistant().with_frontend_tool_request(
            "frontend_tool1",
            Ok(CallToolRequestParams {
                task: None,
                name: "frontend_test_tool".into(),
                arguments: Some(object!({"action": "click", "element": "button"})),
                meta: None,
            }),
        );

        let spec = format_messages(&[message], &ImageFormat::OpenAi);

        assert_eq!(spec.len(), 1);
        assert_eq!(spec[0]["role"], "assistant");
        assert!(spec[0]["tool_calls"].is_array());

        let tool_call = &spec[0]["tool_calls"][0];
        assert_eq!(tool_call["id"], "frontend_tool1");
        assert_eq!(tool_call["type"], "function");
        assert_eq!(tool_call["function"]["name"], "frontend_test_tool");
        // This should be a JSON string representation
        let args_str = tool_call["function"]["arguments"].as_str().unwrap();
        let parsed_args: Value = serde_json::from_str(args_str)?;
        assert_eq!(parsed_args["action"], "click");
        assert_eq!(parsed_args["element"], "button");

        Ok(())
    }

    #[test]
    fn test_format_messages_multiple_text_blocks() -> anyhow::Result<()> {
        let message = Message::user()
            .with_text("--- Resource: file:///test.md ---\n# Test\n\n---\n")
            .with_text(" What is in the file?");

        let spec = format_messages(&[message], &ImageFormat::OpenAi);

        assert_eq!(spec.len(), 1);
        assert_eq!(spec[0]["role"], "user");
        assert_eq!(
            spec[0]["content"],
            "--- Resource: file:///test.md ---\n# Test\n\n---\n\n What is in the file?"
        );
        Ok(())
    }

    #[test]
    fn test_create_request_gpt_4o() -> anyhow::Result<()> {
        // Test default medium reasoning effort for O3 model
        let model_config = ModelConfig {
            model_name: "gpt-4o".to_string(),
            context_limit: Some(4096),
            temperature: None,
            max_tokens: Some(1024),
            toolshim: false,
            toolshim_model: None,
            fast_model: None,
            request_params: None,
            reasoning_effort: None,
        };
        let request = create_request(
            &model_config,
            "system",
            &[],
            &[],
            &ImageFormat::OpenAi,
            false,
        )?;
        let obj = request.as_object().unwrap();
        let expected = json!({
            "model": "gpt-4o",
            "messages": [
                {
                    "role": "system",
                    "content": "system"
                }
            ],
            "max_tokens": 1024
        });

        for (key, value) in expected.as_object().unwrap() {
            assert_eq!(obj.get(key).unwrap(), value);
        }

        Ok(())
    }

    #[test]
    fn test_create_request_o1_default() -> anyhow::Result<()> {
        // Test default medium reasoning effort for O1 model
        let model_config = ModelConfig {
            model_name: "o1".to_string(),
            context_limit: Some(4096),
            temperature: None,
            max_tokens: Some(1024),
            toolshim: false,
            toolshim_model: None,
            fast_model: None,
            request_params: None,
            reasoning_effort: None,
        };
        let request = create_request(
            &model_config,
            "system",
            &[],
            &[],
            &ImageFormat::OpenAi,
            false,
        )?;
        let obj = request.as_object().unwrap();
        let expected = json!({
            "model": "o1",
            "messages": [
                {
                    "role": "developer",
                    "content": "system"
                }
            ],
            "reasoning_effort": "medium",
            "max_completion_tokens": 1024
        });

        for (key, value) in expected.as_object().unwrap() {
            assert_eq!(obj.get(key).unwrap(), value);
        }

        Ok(())
    }

    #[test]
    fn test_create_request_gpt_5_5_chat_uses_completion_tokens() -> anyhow::Result<()> {
        let model_config = ModelConfig {
            model_name: "gpt-5.5-2026-04-24".to_string(),
            context_limit: Some(4096),
            temperature: None,
            max_tokens: Some(1024),
            toolshim: false,
            toolshim_model: None,
            fast_model: None,
            request_params: None,
            reasoning_effort: None,
        };
        let request = create_request(
            &model_config,
            "system",
            &[],
            &[],
            &ImageFormat::OpenAi,
            false,
        )?;
        let obj = request.as_object().unwrap();
        let expected = json!({
            "model": "gpt-5.5-2026-04-24",
            "messages": [
                {
                    "role": "developer",
                    "content": "system"
                }
            ],
            "max_completion_tokens": 1024,
            "reasoning_effort": "medium"
        });

        for (key, value) in expected.as_object().unwrap() {
            assert_eq!(obj.get(key).unwrap(), value);
        }
        assert!(obj.get("max_tokens").is_none());

        Ok(())
    }

    #[test]
    fn test_create_request_o3_custom_reasoning_effort() -> anyhow::Result<()> {
        // Test custom reasoning effort for O3 model
        let model_config = ModelConfig {
            model_name: "o3-mini-high".to_string(),
            context_limit: Some(4096),
            temperature: None,
            max_tokens: Some(1024),
            toolshim: false,
            toolshim_model: None,
            fast_model: None,
            request_params: None,
            reasoning_effort: None,
        };
        let request = create_request(
            &model_config,
            "system",
            &[],
            &[],
            &ImageFormat::OpenAi,
            false,
        )?;
        let obj = request.as_object().unwrap();
        let expected = json!({
            "model": "o3-mini",
            "messages": [
                {
                    "role": "developer",
                    "content": "system"
                }
            ],
            "reasoning_effort": "high",
            "max_completion_tokens": 1024
        });

        for (key, value) in expected.as_object().unwrap() {
            assert_eq!(obj.get(key).unwrap(), value);
        }

        Ok(())
    }

    // BR-63: an explicit per-turn effort outranks the effort implied by the
    // model name, and is never sent to a model that can't take it.
    #[test]
    fn test_create_request_explicit_effort_overrides_model_name_suffix() -> anyhow::Result<()> {
        let model_config = ModelConfig::new_or_fail("o3-mini-high")
            .with_reasoning_effort(Some(ReasoningEffort::Quick));

        let request = create_request(
            &model_config,
            "system",
            &[],
            &[],
            &ImageFormat::OpenAi,
            false,
        )?;

        let obj = request.as_object().unwrap();
        assert_eq!(obj.get("model").unwrap(), "o3-mini");
        assert_eq!(obj.get("reasoning_effort").unwrap(), "low");
        Ok(())
    }

    #[test]
    fn test_create_request_deep_effort_asks_for_high_reasoning() -> anyhow::Result<()> {
        let model_config =
            ModelConfig::new_or_fail("o3-mini").with_reasoning_effort(Some(ReasoningEffort::Deep));

        let request = create_request(
            &model_config,
            "system",
            &[],
            &[],
            &ImageFormat::OpenAi,
            false,
        )?;

        assert_eq!(
            request
                .as_object()
                .unwrap()
                .get("reasoning_effort")
                .unwrap(),
            "high"
        );
        Ok(())
    }

    #[test]
    fn test_create_request_effort_not_sent_to_non_reasoning_model() -> anyhow::Result<()> {
        // gpt-4o rejects `reasoning_effort` outright — the effort must degrade
        // to a no-op on the request rather than 400 the turn.
        let model_config =
            ModelConfig::new_or_fail("gpt-4o").with_reasoning_effort(Some(ReasoningEffort::Deep));

        let request = create_request(
            &model_config,
            "system",
            &[],
            &[],
            &ImageFormat::OpenAi,
            false,
        )?;

        assert!(request
            .as_object()
            .unwrap()
            .get("reasoning_effort")
            .is_none());
        Ok(())
    }

    /// The three GPT-6 ids, bare and in the dated spelling Azure deploys them
    /// under. There is no `gpt-6-terra`: Terra exists only as `gpt-5.6-terra`.
    const GPT_6_IDS: &[&str] = &[
        "gpt-6-sol",
        "gpt-6-luna",
        "gpt-6-astra",
        "gpt-6-sol-2026-09-22",
        "gpt-6-luna-2026-09-22",
        "gpt-6-astra-2026-09-03",
    ];

    #[test]
    fn gpt_6_is_a_reasoning_model_routed_to_the_responses_api() {
        for id in GPT_6_IDS {
            assert!(
                model_supports_reasoning_effort(id),
                "{id} is a reasoning model"
            );
            assert!(
                model_uses_responses_api(id),
                "{id} calls tools only through /v1/responses"
            );
            // Quick/Deep map to low/high, both of which every GPT-6 tier
            // accepts. Astra rejects `none`, which BioRouter never sends.
            assert_eq!(model_reasoning_effort(id, "low"), Some("low"), "{id}");
            assert_eq!(model_reasoning_effort(id, "high"), Some("high"), "{id}");
        }
        // The prefix must not reach a family that only shares the digits.
        assert!(!model_uses_responses_api("openai/gpt-6-sol"));
        assert!(!model_uses_responses_api("databricks-gpt-6-sol"));
    }

    /// Chat Completions is still reachable with a GPT-6 id — a custom
    /// `OPENAI_BASE_PATH`, or a gateway that only speaks it — and there it must be
    /// shaped as the reasoning model it is: GPT-6 refuses `temperature` whenever
    /// the effort is not `none`, and refuses `max_tokens` outright.
    #[test]
    fn gpt_6_chat_completions_request_is_shaped_as_a_reasoning_model() -> anyhow::Result<()> {
        for id in ["gpt-6-sol", "gpt-6-astra-2026-09-03"] {
            let model_config = ModelConfig::new_or_fail(id)
                .with_temperature(Some(0.2))
                .with_max_tokens(Some(1024));
            let request = create_request(
                &model_config,
                "system",
                &[],
                &[],
                &ImageFormat::OpenAi,
                false,
            )?;
            let obj = request.as_object().unwrap();
            assert_eq!(obj["model"], json!(id), "the id is sent as given");
            assert_eq!(obj["messages"][0]["role"], json!("developer"), "{id}");
            assert_eq!(obj["max_completion_tokens"], json!(1024), "{id}");
            assert!(obj.get("max_tokens").is_none(), "{id}");
            assert!(obj.get("temperature").is_none(), "{id}");
            assert!(obj.get("top_p").is_none(), "{id}");
            assert_eq!(obj["reasoning_effort"], json!("medium"), "{id}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_streamed_multi_tool_response_to_messages() -> anyhow::Result<()> {
        let response_lines = r#"
data: {"model":"us.anthropic.claude-sonnet-4-20250514-v1:0","choices":[{"delta":{"role":"assistant","content":"I'll run both"},"index":0,"finish_reason":null}],"usage":{"prompt_tokens":4982,"completion_tokens":null,"total_tokens":null},"object":"chat.completion.chunk","id":"msg_bdrk_014pifLTHsNZz6Lmtw1ywgDJ","created":1753288340}
data: {"model":"us.anthropic.claude-sonnet-4-20250514-v1:0","choices":[{"delta":{"role":"assistant","content":" `ls` commands in a"},"index":0,"finish_reason":null}],"usage":{"prompt_tokens":4982,"completion_tokens":null,"total_tokens":null},"object":"chat.completion.chunk","id":"msg_bdrk_014pifLTHsNZz6Lmtw1ywgDJ","created":1753288340}
data: {"model":"us.anthropic.claude-sonnet-4-20250514-v1:0","choices":[{"delta":{"role":"assistant","content":" single turn for you -"},"index":0,"finish_reason":null}],"usage":{"prompt_tokens":4982,"completion_tokens":null,"total_tokens":null},"object":"chat.completion.chunk","id":"msg_bdrk_014pifLTHsNZz6Lmtw1ywgDJ","created":1753288340}
data: {"model":"us.anthropic.claude-sonnet-4-20250514-v1:0","choices":[{"delta":{"role":"assistant","content":" one on the current directory an"},"index":0,"finish_reason":null}],"usage":{"prompt_tokens":4982,"completion_tokens":null,"total_tokens":null},"object":"chat.completion.chunk","id":"msg_bdrk_014pifLTHsNZz6Lmtw1ywgDJ","created":1753288340}
data: {"model":"us.anthropic.claude-sonnet-4-20250514-v1:0","choices":[{"delta":{"role":"assistant","content":"d one on the `working_dir`."},"index":0,"finish_reason":null}],"usage":{"prompt_tokens":4982,"completion_tokens":null,"total_tokens":null},"object":"chat.completion.chunk","id":"msg_bdrk_014pifLTHsNZz6Lmtw1ywgDJ","created":1753288340}
data: {"model":"us.anthropic.claude-sonnet-4-20250514-v1:0","choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":1,"id":"toolu_bdrk_01RMTd7R9DzQjEEWgDwzcBsU","type":"function","function":{"name":"developer__shell","arguments":""}}]},"index":0,"finish_reason":null}],"usage":{"prompt_tokens":4982,"completion_tokens":null,"total_tokens":null},"object":"chat.completion.chunk","id":"msg_bdrk_014pifLTHsNZz6Lmtw1ywgDJ","created":1753288341}
data: {"model":"us.anthropic.claude-sonnet-4-20250514-v1:0","choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":1,"function":{"arguments":""}}]},"index":0,"finish_reason":null}],"usage":{"prompt_tokens":4982,"completion_tokens":null,"total_tokens":null},"object":"chat.completion.chunk","id":"msg_bdrk_014pifLTHsNZz6Lmtw1ywgDJ","created":1753288341}
data: {"model":"us.anthropic.claude-sonnet-4-20250514-v1:0","choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":1,"function":{"arguments":"{\""}}]},"index":0,"finish_reason":null}],"usage":{"prompt_tokens":4982,"completion_tokens":null,"total_tokens":null},"object":"chat.completion.chunk","id":"msg_bdrk_014pifLTHsNZz6Lmtw1ywgDJ","created":1753288341}
data: {"model":"us.anthropic.claude-sonnet-4-20250514-v1:0","choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":1,"function":{"arguments":"command\": \"l"}}]},"index":0,"finish_reason":null}],"usage":{"prompt_tokens":4982,"completion_tokens":null,"total_tokens":null},"object":"chat.completion.chunk","id":"msg_bdrk_014pifLTHsNZz6Lmtw1ywgDJ","created":1753288341}
data: {"model":"us.anthropic.claude-sonnet-4-20250514-v1:0","choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":1,"function":{"arguments":"s\"}"}}]},"index":0,"finish_reason":null}],"usage":{"prompt_tokens":4982,"completion_tokens":null,"total_tokens":null},"object":"chat.completion.chunk","id":"msg_bdrk_014pifLTHsNZz6Lmtw1ywgDJ","created":1753288341}
data: {"model":"us.anthropic.claude-sonnet-4-20250514-v1:0","choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":2,"id":"toolu_bdrk_016bgVTGZdpjP8ehjMWp9cWW","type":"function","function":{"name":"developer__shell","arguments":""}}]},"index":0,"finish_reason":null}],"usage":{"prompt_tokens":4982,"completion_tokens":null,"total_tokens":null},"object":"chat.completion.chunk","id":"msg_bdrk_014pifLTHsNZz6Lmtw1ywgDJ","created":1753288341}
data: {"model":"us.anthropic.claude-sonnet-4-20250514-v1:0","choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":2,"function":{"arguments":""}}]},"index":0,"finish_reason":null}],"usage":{"prompt_tokens":4982,"completion_tokens":null,"total_tokens":null},"object":"chat.completion.chunk","id":"msg_bdrk_014pifLTHsNZz6Lmtw1ywgDJ","created":1753288341}
data: {"model":"us.anthropic.claude-sonnet-4-20250514-v1:0","choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":2,"function":{"arguments":"{\""}}]},"index":0,"finish_reason":null}],"usage":{"prompt_tokens":4982,"completion_tokens":null,"total_tokens":null},"object":"chat.completion.chunk","id":"msg_bdrk_014pifLTHsNZz6Lmtw1ywgDJ","created":1753288342}
data: {"model":"us.anthropic.claude-sonnet-4-20250514-v1:0","choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":2,"function":{"arguments":"command\""}}]},"index":0,"finish_reason":null}],"usage":{"prompt_tokens":4982,"completion_tokens":null,"total_tokens":null},"object":"chat.completion.chunk","id":"msg_bdrk_014pifLTHsNZz6Lmtw1ywgDJ","created":1753288342}
data: {"model":"us.anthropic.claude-sonnet-4-20250514-v1:0","choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":2,"function":{"arguments":": \"ls wor"}}]},"index":0,"finish_reason":null}],"usage":{"prompt_tokens":4982,"completion_tokens":null,"total_tokens":null},"object":"chat.completion.chunk","id":"msg_bdrk_014pifLTHsNZz6Lmtw1ywgDJ","created":1753288342}
data: {"model":"us.anthropic.claude-sonnet-4-20250514-v1:0","choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":2,"function":{"arguments":"king_dir"}}]},"index":0,"finish_reason":null}],"usage":{"prompt_tokens":4982,"completion_tokens":null,"total_tokens":null},"object":"chat.completion.chunk","id":"msg_bdrk_014pifLTHsNZz6Lmtw1ywgDJ","created":1753288342}
data: {"model":"us.anthropic.claude-sonnet-4-20250514-v1:0","choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":2,"function":{"arguments":"\"}"}}]},"index":0,"finish_reason":null}],"usage":{"prompt_tokens":4982,"completion_tokens":null,"total_tokens":null},"object":"chat.completion.chunk","id":"msg_bdrk_014pifLTHsNZz6Lmtw1ywgDJ","created":1753288342}
data: {"model":"us.anthropic.claude-sonnet-4-20250514-v1:0","choices":[{"delta":{"role":"assistant","content":""},"index":0,"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":4982,"completion_tokens":122,"total_tokens":5104},"object":"chat.completion.chunk","id":"msg_bdrk_014pifLTHsNZz6Lmtw1ywgDJ","created":1753288342}
data: [DONE]
"#;

        let response_stream =
            tokio_stream::iter(response_lines.lines().map(|line| Ok(line.to_string())));
        let messages = response_to_streaming_message(response_stream);
        pin!(messages);

        let mut saw_tool_calls = false;
        let mut final_usage = None;
        while let Some(Ok((message, usage, _pending))) = messages.next().await {
            if usage.is_some() {
                final_usage = usage;
            }
            if let Some(msg) = message {
                println!("{:?}", msg);
                if msg.content.len() == 2 {
                    if let (MessageContent::ToolRequest(req1), MessageContent::ToolRequest(req2)) =
                        (&msg.content[0], &msg.content[1])
                    {
                        if req1.tool_call.is_ok() && req2.tool_call.is_ok() {
                            assert_eq!(req1.tool_call.as_ref().unwrap().name, "developer__shell");
                            assert_eq!(req2.tool_call.as_ref().unwrap().name, "developer__shell");
                            saw_tool_calls = true;
                        }
                    }
                }
            }
        }

        assert!(
            saw_tool_calls,
            "expected tool call message with two calls, but did not see it"
        );
        let usage = final_usage.expect("expected terminal tool-call usage");
        assert_eq!(usage.usage.input_tokens, Some(4982));
        assert_eq!(usage.usage.output_tokens, Some(122));
        assert_eq!(usage.usage.total_tokens, Some(5104));
        assert_eq!(usage.usage.billed_total(), Some(5104));
        assert_eq!(usage.finish_reason.as_deref(), Some("tool_calls"));
        Ok(())
    }

    /// §6.1b safety: the higher-traffic OpenAI decoder must announce each tool
    /// call's NAME before its arguments finish, must never wrap a pending
    /// notification in a `Message`, and must still emit exactly one authoritative
    /// ToolRequest per call. Reuses the two-`developer__shell` fixture above.
    #[tokio::test]
    async fn test_streaming_emits_pending_tool_calls_before_dispatch() -> anyhow::Result<()> {
        let response_lines = r#"
data: {"model":"m","choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":1,"id":"toolu_a","type":"function","function":{"name":"developer__shell","arguments":""}}]},"index":0,"finish_reason":null}],"object":"chat.completion.chunk","id":"x","created":1}
data: {"model":"m","choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":1,"function":{"arguments":"{\"command\": \"ls\"}"}}]},"index":0,"finish_reason":null}],"object":"chat.completion.chunk","id":"x","created":1}
data: {"model":"m","choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":2,"id":"toolu_b","type":"function","function":{"name":"developer__shell","arguments":""}}]},"index":0,"finish_reason":null}],"object":"chat.completion.chunk","id":"x","created":1}
data: {"model":"m","choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":2,"function":{"arguments":"{\"command\": \"rm -rf /\"}"}}]},"index":0,"finish_reason":null}],"object":"chat.completion.chunk","id":"x","created":1}
data: {"model":"m","choices":[{"delta":{"role":"assistant","content":""},"index":0,"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15},"object":"chat.completion.chunk","id":"x","created":1}
data: [DONE]
"#;
        let response_stream =
            tokio_stream::iter(response_lines.lines().map(|line| Ok(line.to_string())));
        let messages = response_to_streaming_message(response_stream);
        pin!(messages);

        let mut items = Vec::new();
        while let Some(result) = messages.next().await {
            items.push(result?);
        }

        // (c) Every pending notification is message-free — structurally impossible
        //     to dispatch.
        let pendings: Vec<_> = items
            .iter()
            .filter_map(|(m, _, p)| p.as_ref().map(|p| (m.is_none(), p.clone())))
            .collect();
        assert!(
            pendings.iter().all(|(msg_is_none, _)| *msg_is_none),
            "a pending notification must never carry a Message"
        );

        // (a) Both tool names are announced, and the first announcement for each
        //     call carries no args (name-before-args).
        let names: Vec<_> = pendings.iter().map(|(_, p)| p.name.clone()).collect();
        assert!(names.iter().all(|n| n == "developer__shell"));
        let first_a = pendings
            .iter()
            .find(|(_, p)| p.id == "toolu_a")
            .expect("toolu_a announced");
        assert!(
            first_a.1.partial_args.is_none(),
            "the first notification for a call must precede its argument bytes"
        );
        assert!(pendings.iter().any(|(_, p)| p.id == "toolu_b"));

        // (b) Exactly TWO authoritative ToolRequests, both parsed cleanly.
        let tool_requests: Vec<_> = items
            .iter()
            .filter_map(|(m, _, _)| m.as_ref())
            .flat_map(|m| m.content.iter())
            .filter_map(|c| match c {
                MessageContent::ToolRequest(r) => Some(r.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            tool_requests.len(),
            2,
            "exactly one authoritative ToolRequest per tool call"
        );
        assert!(tool_requests.iter().all(|r| r.tool_call.is_ok()));
        Ok(())
    }

    #[tokio::test]
    async fn test_streaming_captures_length_finish_reason() -> anyhow::Result<()> {
        // Mirrors MiMo's truncation protocol: a content chunk, then a chunk with
        // finish_reason="length" and usage:null, then a separate choices:[] chunk
        // carrying usage, then [DONE]. The emitted ProviderUsage must carry the
        // "length" finish_reason so the agent loop can auto-continue.
        let lines = r#"
data: {"model":"mimo-v2.5-pro","choices":[{"delta":{"content":"counting"},"index":0,"finish_reason":null}],"object":"chat.completion.chunk","id":"x"}
data: {"model":"mimo-v2.5-pro","choices":[{"delta":{"content":null},"index":0,"finish_reason":"length"}],"usage":null,"object":"chat.completion.chunk","id":"x"}
data: {"model":"mimo-v2.5-pro","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":8,"total_tokens":18},"object":"chat.completion.chunk","id":"x"}
data: [DONE]
"#;
        let response_stream = tokio_stream::iter(lines.lines().map(|l| Ok(l.to_string())));
        let messages = response_to_streaming_message(response_stream);
        pin!(messages);

        let mut seen_length = false;
        while let Some(Ok((_message, usage, _pending))) = messages.next().await {
            if let Some(u) = usage {
                if u.finish_reason.as_deref() == Some("length") {
                    seen_length = true;
                }
            }
        }
        assert!(
            seen_length,
            "expected ProviderUsage.finish_reason == Some(\"length\") on a truncated stream"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_streaming_natural_stop_is_not_length() -> anyhow::Result<()> {
        // A natural completion reports finish_reason="stop"; the agent loop must
        // NOT treat it as truncation.
        let lines = r#"
data: {"model":"mimo-v2.5-pro","choices":[{"delta":{"content":"hello"},"index":0,"finish_reason":null}],"object":"chat.completion.chunk","id":"y"}
data: {"model":"mimo-v2.5-pro","choices":[{"delta":{"content":null},"index":0,"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":1,"total_tokens":6},"object":"chat.completion.chunk","id":"y"}
data: [DONE]
"#;
        let response_stream = tokio_stream::iter(lines.lines().map(|l| Ok(l.to_string())));
        let messages = response_to_streaming_message(response_stream);
        pin!(messages);

        while let Some(Ok((_message, usage, _pending))) = messages.next().await {
            if let Some(u) = usage {
                assert_ne!(
                    u.finish_reason.as_deref(),
                    Some("length"),
                    "natural stop must not be reported as length-truncation"
                );
            }
        }
        Ok(())
    }
}
