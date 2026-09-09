use crate::conversation::message::{Message, MessageContent};
use crate::model::ModelConfig;
use crate::providers::base::{tool_call_batching_enabled, PendingToolCall, Usage};
use crate::providers::errors::ProviderError;
use crate::providers::formats::audience;
use crate::providers::utils::{convert_image, ImageFormat};
use anyhow::{anyhow, Result};
use rmcp::model::{
    object, AnnotateAble, CallToolRequestParams, ErrorCode, ErrorData, JsonObject, RawContent,
    Role, Tool,
};
use rmcp::object as json_object;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;

/// Minimum wall-clock gap between two partial-argument notifications for the
/// same tool call. Anthropic emits `input_json_delta` every few tokens; without
/// throttling a single tool call would become hundreds of SSE frames.
const PENDING_ARGS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);
/// …or this many newly accumulated argument characters, whichever is sooner.
const PENDING_ARGS_CHARS: usize = 200;

// Constants for frequently used strings in Anthropic API format
const TYPE_FIELD: &str = "type";
const CONTENT_FIELD: &str = "content";
const TEXT_TYPE: &str = "text";
const ROLE_FIELD: &str = "role";
const USER_ROLE: &str = "user";
const ASSISTANT_ROLE: &str = "assistant";
const TOOL_USE_TYPE: &str = "tool_use";
const TOOL_RESULT_TYPE: &str = "tool_result";
const THINKING_TYPE: &str = "thinking";
const REDACTED_THINKING_TYPE: &str = "redacted_thinking";
const CACHE_CONTROL_FIELD: &str = "cache_control";
const ID_FIELD: &str = "id";
const NAME_FIELD: &str = "name";
const INPUT_FIELD: &str = "input";
const TOOL_USE_ID_FIELD: &str = "tool_use_id";
const IS_ERROR_FIELD: &str = "is_error";
const SIGNATURE_FIELD: &str = "signature";
const DATA_FIELD: &str = "data";

/// Convert internal Message format to Anthropic's API message specification
#[allow(clippy::too_many_lines)]
pub fn format_messages(messages: &[Message]) -> Vec<Value> {
    let mut anthropic_messages = Vec::new();

    for message in messages.iter().filter(|m| m.is_agent_visible()) {
        let role = match message.role {
            Role::User => USER_ROLE,
            Role::Assistant => ASSISTANT_ROLE,
        };

        let mut content = Vec::new();
        for msg_content in &message.content {
            match msg_content {
                MessageContent::Text(text) => {
                    content.push(json!({
                        TYPE_FIELD: TEXT_TYPE,
                        TEXT_TYPE: text.text
                    }));
                }
                MessageContent::ToolRequest(tool_request) => {
                    match &tool_request.tool_call {
                        Ok(tool_call) => {
                            content.push(json!({
                                TYPE_FIELD: TOOL_USE_TYPE,
                                ID_FIELD: tool_request.id,
                                NAME_FIELD: tool_call.name,
                                INPUT_FIELD: tool_call.arguments
                            }));
                        }
                        Err(_tool_error) => {
                            // Skip malformed tool requests - they shouldn't be sent to Anthropic
                            // This maintains the existing behavior for ToolRequest errors
                        }
                    }
                }
                MessageContent::ToolResponse(tool_response) => match &tool_response.tool_result {
                    Ok(result) => {
                        let visible = result
                            .content
                            .iter()
                            // Send only what the tool addressed to the model.
                            .filter(|c| audience::is_for_model(c))
                            .collect::<Vec<_>>();
                        let carries_image = visible
                            .iter()
                            .any(|content| matches!(&content.raw, RawContent::Image(_)));
                        let tool_content = if carries_image {
                            Value::Array(
                                visible
                                    .iter()
                                    .filter_map(|content| {
                                        match &content.raw {
                                        RawContent::Image(image) => Some(convert_image(
                                            &image.clone().no_annotation(),
                                            &ImageFormat::Anthropic,
                                        )),
                                        _ => audience::flattened_text(content).map(|text| {
                                            json!({ TYPE_FIELD: TEXT_TYPE, TEXT_TYPE: text })
                                        }),
                                    }
                                    })
                                    .collect(),
                            )
                        } else {
                            Value::String(
                                visible
                                    .iter()
                                    .filter_map(|content| audience::flattened_text(content))
                                    .collect::<Vec<_>>()
                                    .join("\n"),
                            )
                        };

                        content.push(json!({
                            TYPE_FIELD: TOOL_RESULT_TYPE,
                            TOOL_USE_ID_FIELD: tool_response.id,
                            CONTENT_FIELD: tool_content
                        }));
                    }
                    Err(tool_error) => {
                        content.push(json!({
                            TYPE_FIELD: TOOL_RESULT_TYPE,
                            TOOL_USE_ID_FIELD: tool_response.id,
                            CONTENT_FIELD: format!("Error: {}", tool_error),
                            IS_ERROR_FIELD: true
                        }));
                    }
                },
                MessageContent::ToolConfirmationRequest(_tool_confirmation_request) => {
                    // Skip tool confirmation requests
                }
                MessageContent::ActionRequired(_action_required) => {
                    // Skip action required messages - they're for UI only
                }
                MessageContent::SystemNotification(_) => {
                    // Skip
                }
                MessageContent::Thinking(thinking) => {
                    content.push(json!({
                        TYPE_FIELD: THINKING_TYPE,
                        THINKING_TYPE: thinking.thinking,
                        SIGNATURE_FIELD: thinking.signature
                    }));
                }
                MessageContent::RedactedThinking(redacted) => {
                    content.push(json!({
                        TYPE_FIELD: REDACTED_THINKING_TYPE,
                        DATA_FIELD: redacted.data
                    }));
                }
                MessageContent::Image(image) => {
                    content.push(convert_image(image, &ImageFormat::Anthropic));
                }
                MessageContent::FrontendToolRequest(tool_request) => {
                    if let Ok(tool_call) = &tool_request.tool_call {
                        content.push(json!({
                            TYPE_FIELD: TOOL_USE_TYPE,
                            ID_FIELD: tool_request.id,
                            NAME_FIELD: tool_call.name,
                            INPUT_FIELD: tool_call.arguments
                        }));
                    }
                }
            }
        }

        // Skip messages with empty content
        if !content.is_empty() {
            anthropic_messages.push(json!({
                ROLE_FIELD: role,
                CONTENT_FIELD: content
            }));
        }
    }

    // If no messages, add a default one
    if anthropic_messages.is_empty() {
        anthropic_messages.push(json!({
            ROLE_FIELD: USER_ROLE,
            CONTENT_FIELD: [{
                TYPE_FIELD: TEXT_TYPE,
                TEXT_TYPE: "Ignore"
            }]
        }));
    }

    // Add "cache_control" to the last and second-to-last "user" messages.
    // During each turn, we mark the final message with cache_control so the conversation can be
    // incrementally cached. The second-to-last user message is also marked for caching with the
    // cache_control parameter, so that this checkpoint can read from the previous cache.
    let mut user_count = 0;
    for message in anthropic_messages.iter_mut().rev() {
        if message.get(ROLE_FIELD) == Some(&json!(USER_ROLE)) {
            if let Some(content) = message.get_mut(CONTENT_FIELD) {
                if let Some(content_array) = content.as_array_mut() {
                    if let Some(last_content) = content_array.last_mut() {
                        last_content.as_object_mut().unwrap().insert(
                            CACHE_CONTROL_FIELD.to_string(),
                            json!({ TYPE_FIELD: "ephemeral" }),
                        );
                    }
                }
            }
            user_count += 1;
            if user_count >= 2 {
                break;
            }
        }
    }

    anthropic_messages
}

fn anthropic_flavored_input_schema(input_schema: Arc<JsonObject>) -> Arc<JsonObject> {
    if input_schema.is_empty() {
        return Arc::new(json_object!({
            "type": "object",
        }));
    }
    input_schema
}

/// Convert internal Tool format to Anthropic's API tool specification
pub fn format_tools(tools: &[Tool]) -> Vec<Value> {
    let mut unique_tools = HashSet::new();
    let mut tool_specs = Vec::new();

    for tool in tools {
        if unique_tools.insert(tool.name.clone()) {
            tool_specs.push(json!({
                NAME_FIELD: tool.name,
                "description": tool.description,
                "input_schema": anthropic_flavored_input_schema(tool.input_schema.clone())
            }));
        }
    }

    // Add "cache_control" to the last tool spec, if any. This means that all tool definitions,
    // will be cached as a single prefix.
    if let Some(last_tool) = tool_specs.last_mut() {
        last_tool.as_object_mut().unwrap().insert(
            CACHE_CONTROL_FIELD.to_string(),
            json!({ TYPE_FIELD: "ephemeral" }),
        );
    }

    tool_specs
}

/// Convert system message to Anthropic's API system specification
pub fn format_system(system: &str) -> Value {
    json!([{
        TYPE_FIELD: TEXT_TYPE,
        TEXT_TYPE: system,
        CACHE_CONTROL_FIELD: { TYPE_FIELD: "ephemeral" }
    }])
}

/// Convert Anthropic's API response to internal Message format
pub fn response_to_message(response: &Value) -> Result<Message> {
    let content_blocks = response
        .get(CONTENT_FIELD)
        .and_then(|c| c.as_array())
        .ok_or_else(|| anyhow!("Invalid response format: missing content array"))?;

    let mut message = Message::assistant();

    for block in content_blocks {
        match block.get(TYPE_FIELD).and_then(|t| t.as_str()) {
            Some(TEXT_TYPE) => {
                if let Some(text) = block.get(TEXT_TYPE).and_then(|t| t.as_str()) {
                    message = message.with_text(text.to_string());
                }
            }
            Some(TOOL_USE_TYPE) => {
                let id = block
                    .get(ID_FIELD)
                    .and_then(|i| i.as_str())
                    .ok_or_else(|| anyhow!("Missing tool_use id"))?;
                let name = block
                    .get(NAME_FIELD)
                    .and_then(|n| n.as_str())
                    .ok_or_else(|| anyhow!("Missing tool_use name"))?
                    .to_string();
                let input = block
                    .get(INPUT_FIELD)
                    .ok_or_else(|| anyhow!("Missing tool_use input"))?;

                let tool_call = CallToolRequestParams {
                    task: None,
                    name: name.into(),
                    arguments: Some(object(input.clone())),
                    meta: None,
                };
                message = message.with_tool_request(id, Ok(tool_call));
            }
            Some(THINKING_TYPE) => {
                let thinking = block
                    .get(THINKING_TYPE)
                    .and_then(|t| t.as_str())
                    .ok_or_else(|| anyhow!("Missing thinking content"))?
                    .to_string();
                let signature = block
                    .get(SIGNATURE_FIELD)
                    .and_then(|s| s.as_str())
                    .ok_or_else(|| anyhow!("Missing thinking signature"))?;
                message = message.with_thinking(thinking, signature);
            }
            Some(REDACTED_THINKING_TYPE) => {
                let data = block
                    .get(DATA_FIELD)
                    .and_then(|d| d.as_str())
                    .ok_or_else(|| anyhow!("Missing redacted_thinking data"))?;
                message = message.with_redacted_thinking(data);
            }
            _ => continue,
        }
    }

    Ok(message)
}

/// Build a [`Usage`] from an Anthropic usage object, keeping fresh input and the
/// two cache buckets disjoint. `total_tokens` (context occupancy) is the sum of
/// all four buckets so the live gauge is unchanged from the folded era.
fn usage_from_fields(usage: &Value) -> Usage {
    let field = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);
    let clamp = |v: u64| v.min(i32::MAX as u64) as i32;

    let input_tokens = field("input_tokens");
    let cache_creation_tokens = field("cache_creation_input_tokens");
    let cache_read_tokens = field("cache_read_input_tokens");
    let output_tokens = field("output_tokens");

    let input_i32 = clamp(input_tokens);
    let cache_creation_i32 = clamp(cache_creation_tokens);
    let cache_read_i32 = clamp(cache_read_tokens);
    let output_i32 = clamp(output_tokens);

    // Context occupancy = fresh input + both cache buckets + output.
    let total_i32 = (i64::from(input_i32)
        + i64::from(cache_creation_i32)
        + i64::from(cache_read_i32)
        + i64::from(output_i32))
    .min(i64::from(i32::MAX)) as i32;

    Usage::new(Some(input_i32), Some(output_i32), Some(total_i32))
        .with_cache(Some(cache_read_i32), Some(cache_creation_i32))
}

/// Extract usage information from Anthropic's API response.
///
/// Per-provider semantics: Anthropic's `input_tokens` **excludes** the two
/// cache buckets — `cache_read_input_tokens` and `cache_creation_input_tokens`
/// are reported *in addition* to `input_tokens`. We keep them disjoint in
/// [`Usage`] (fresh input in `input_tokens`, cache in the two cache fields) so
/// [`Usage::billed_total`] is a plain sum. `total_tokens` is the full context
/// occupancy (fresh input + both cache buckets + output) for the live gauge.
pub fn get_usage(data: &Value) -> Result<Usage> {
    // Extract usage data if available
    if let Some(usage) = data.get("usage") {
        Ok(usage_from_fields(usage))
    } else if data.as_object().is_some() {
        // The data itself may be the usage object (message_delta events that
        // carry usage at the top level).
        let usage = usage_from_fields(data);
        if usage.billed_total().unwrap_or(0) == 0 {
            tracing::debug!("🔍 Anthropic no token data found in object");
            return Ok(Usage::new(None, None, None));
        }
        Ok(usage)
    } else {
        tracing::debug!(
            "Failed to get usage data: {}",
            ProviderError::UsageError("No usage data found in response".to_string())
        );
        // If no usage data, return None for all values
        Ok(Usage::new(None, None, None))
    }
}

/// Map Anthropic's `stop_reason` onto the OpenAI-style `finish_reason` the agent
/// loop understands. In particular `max_tokens` becomes `"length"` so a response
/// cut off by the output-length limit is auto-continued instead of ending the
/// turn silently mid-sentence; the other reasons map to their OpenAI
/// equivalents, and anything unrecognised passes through unchanged.
pub fn map_stop_reason(stop_reason: &str) -> String {
    match stop_reason {
        "max_tokens" => "length".to_string(),
        "end_turn" | "stop_sequence" => "stop".to_string(),
        "tool_use" => "tool_calls".to_string(),
        other => other.to_string(),
    }
}

fn merge_streaming_usage(initial: Usage, update: Usage) -> Usage {
    let max_bucket = |left: Option<i32>, right: Option<i32>| match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    };

    let input_tokens = max_bucket(initial.input_tokens, update.input_tokens);
    let output_tokens = max_bucket(initial.output_tokens, update.output_tokens);
    let cache_read_input_tokens = max_bucket(
        initial.cache_read_input_tokens,
        update.cache_read_input_tokens,
    );
    let cache_creation_input_tokens = max_bucket(
        initial.cache_creation_input_tokens,
        update.cache_creation_input_tokens,
    );
    let total_tokens = [
        input_tokens,
        output_tokens,
        cache_read_input_tokens,
        cache_creation_input_tokens,
    ]
    .into_iter()
    .flatten()
    .map(i64::from)
    .sum::<i64>()
    .min(i64::from(i32::MAX)) as i32;

    Usage::new(input_tokens, output_tokens, Some(total_tokens))
        .with_cache(cache_read_input_tokens, cache_creation_input_tokens)
}

/// Create a complete request payload for Anthropic's API
pub fn create_request(
    model_config: &ModelConfig,
    system: &str,
    messages: &[Message],
    tools: &[Tool],
) -> Result<Value> {
    let anthropic_messages = format_messages(messages);
    let tool_specs = format_tools(tools);
    let system_spec = format_system(system);

    // Check if we have any messages to send
    if anthropic_messages.is_empty() {
        return Err(anyhow!("No valid messages to send to Anthropic API"));
    }

    // https://platform.claude.com/docs/en/about-claude/models/overview
    // 64k output tokens works for most claude models, but not old opus:
    let max_tokens = model_config.max_tokens.unwrap_or_else(|| {
        let name = &model_config.model_name;
        if name.contains("claude-3-haiku") {
            4096
        } else if name.contains("claude-opus-4-0") || name.contains("claude-opus-4-1") {
            32000
        } else {
            64000
        }
    });
    let mut payload = json!({
        "model": model_config.model_name,
        "messages": anthropic_messages,
        "max_tokens": max_tokens,
    });

    // Add system message if present
    if !system.is_empty() {
        payload
            .as_object_mut()
            .unwrap()
            .insert("system".to_string(), json!(system_spec));
    }

    // Add tools if present
    if !tool_specs.is_empty() {
        payload
            .as_object_mut()
            .unwrap()
            .insert("tools".to_string(), json!(tool_specs));
    }

    // BR-63: `deep` effort asks for extended thinking on this turn, the same way
    // the process-wide CLAUDE_THINKING_ENABLED does — either one turns it on.
    // `quick`/`normal` leave the env behaviour exactly as it was.
    let effort_budget = model_config
        .reasoning_effort
        .and_then(|effort| effort.thinking_budget())
        .map(i32::try_from)
        .and_then(Result::ok);
    let env_thinking = std::env::var("CLAUDE_THINKING_ENABLED").is_ok();
    let is_thinking_enabled = env_thinking || effort_budget.is_some();
    let adaptive_only = uses_adaptive_thinking(&model_config.model_name);

    // Add temperature if specified. Anthropic rejects a temperature other than 1
    // when extended thinking is on, so a thinking turn sends none at all — and
    // the adaptive-only models reject the sampling parameters outright (400),
    // thinking or not, so they never receive one.
    if let Some(temp) = model_config.temperature {
        if !is_thinking_enabled && !adaptive_only {
            payload
                .as_object_mut()
                .unwrap()
                .insert("temperature".to_string(), json!(temp));
        }
    }

    if is_thinking_enabled {
        let thinking = if adaptive_only {
            // budget_tokens is removed (the API 400s) on these models;
            // `{"type": "adaptive"}` is the only on-mode. max_tokens is NOT
            // bumped here: adaptive thinking has no budget to make room for
            // (max_tokens already caps thinking + response together), and
            // inflating a configured max_tokens by a stale budget can push
            // the request past the model's output ceiling — Opus 5 caps
            // output at 128K, so a 128k config + the 16k default deep budget
            // would be a guaranteed 400.
            json!({ "type": "adaptive" })
        } else {
            // Minimum budget_tokens is 1024
            let budget_tokens: i32 = std::env::var("CLAUDE_THINKING_BUDGET")
                .ok()
                .and_then(|v| v.parse().ok())
                .or(effort_budget)
                .unwrap_or(16000);

            // The budgeted path needs the max_tokens bump: budget_tokens must
            // fit under max_tokens, so make room for the thinking on top of
            // the reply.
            payload
                .as_object_mut()
                .unwrap()
                .insert("max_tokens".to_string(), json!(max_tokens + budget_tokens));
            json!({
                "type": "enabled",
                "budget_tokens": budget_tokens
            })
        };
        payload
            .as_object_mut()
            .unwrap()
            .insert("thinking".to_string(), thinking);
    }
    Ok(payload)
}

/// Models on which `thinking: {"type": "enabled", "budget_tokens": N}` is
/// **removed** — the API rejects it with a 400 — and adaptive thinking
/// (`{"type": "adaptive"}`) is the only way to turn thinking on. These models
/// also reject the sampling parameters (`temperature`/`top_p`/`top_k`)
/// outright, thinking or not.
///
/// Per Anthropic's model docs (July 2026): Claude Opus 5, Sonnet 5,
/// Fable 5 / Mythos 5, and Opus 4.7/4.8 all removed `budget_tokens` and
/// sampling; Opus 4.6 / Sonnet 4.6 merely deprecate `budget_tokens` (still
/// functional) and still accept temperature, so they stay on the legacy path
/// below along with everything older. Dotted variants cover
/// OpenRouter/Copilot-style ids.
fn uses_adaptive_thinking(model_name: &str) -> bool {
    const ADAPTIVE_ONLY: &[&str] = &[
        "opus-5", "sonnet-5", "fable-5", "mythos-5", "opus-4-7", "opus-4.7", "opus-4-8", "opus-4.8",
    ];
    ADAPTIVE_ONLY
        .iter()
        .any(|pattern| model_name.contains(pattern))
}

/// Process streaming response from Anthropic's API
/// §6.2b: drain buffered `tool_use` blocks into a **single** assistant message
/// stamped with the streaming `message_id`, or `None` when nothing is buffered.
///
/// One message carrying N `ToolRequest`s is what makes the agent's `select_all`
/// dispatch them in parallel; emitting one message per block serialized them.
/// The `drain` empties the buffer, so a second flush (after-loop belt-and-
/// suspenders) is a no-op — a batch is never delivered twice.
fn flush_pending_tool_contents(
    pending: &mut Vec<MessageContent>,
    message_id: &Option<String>,
) -> Option<Message> {
    if pending.is_empty() {
        return None;
    }
    let content: Vec<MessageContent> = std::mem::take(pending);
    let mut message = Message::new(Role::Assistant, chrono::Utc::now().timestamp(), content);
    message.id = message_id.clone();
    Some(message)
}

fn completed_tool_content(tool_id: String, name: String, args: &str) -> MessageContent {
    let parsed = if args.trim().is_empty() {
        Ok(json!({}))
    } else {
        serde_json::from_str::<Value>(args).map_err(|error| {
            ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("Could not interpret tool use parameters: {error}"),
                Some(json!({"biorouterToolCallFailure":"invalid_arguments"})),
            )
        })
    }
    .and_then(|value| {
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

    match parsed {
        Ok(arguments) => MessageContent::tool_request(
            tool_id,
            Ok(CallToolRequestParams {
                task: None,
                name: name.into(),
                arguments: Some(object(arguments)),
                meta: None,
            }),
        ),
        Err(error) => MessageContent::tool_request(tool_id, Err(error)),
    }
}

struct StreamingToolCall {
    tool_id: String,
    name: String,
    args: String,
}

fn drain_incomplete_tool_contents(
    active: &mut std::collections::HashMap<u64, StreamingToolCall>,
    order: &std::collections::VecDeque<u64>,
    completed: &mut std::collections::HashMap<u64, MessageContent>,
) {
    for index in order {
        if let Some(tool_call) = active.remove(index) {
            completed.insert(
                *index,
                MessageContent::tool_request(
                    tool_call.tool_id,
                    Err(ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        "Tool-call stream completion was not confirmed; the call was not executed. Emit a new, complete tool call.",
                        Some(json!({"biorouterToolCallFailure":"incomplete_stream"})),
                    )),
                ),
            );
        }
    }
}

fn take_ready_tool_contents(
    order: &mut std::collections::VecDeque<u64>,
    completed: &mut std::collections::HashMap<u64, MessageContent>,
) -> Vec<MessageContent> {
    let mut ready = Vec::new();
    while let Some(index) = order.front() {
        let Some(content) = completed.remove(index) else {
            break;
        };
        order.pop_front();
        ready.push(content);
    }
    ready
}

/// Decode an Anthropic SSE stream, sampling the tool-call batching kill switch
/// once, here, at stream construction.
///
/// The sample is taken by this wrapper rather than inside the decoder because
/// the decoder is what tests drive. `tool_call_batching_enabled()` is a bare
/// `std::env::var` (`providers/base.rs`), so a test that exercised the decoder
/// read whatever another test had parked in the process environment — and
/// `env_lock` could not help, because this reader never asked for it. See
/// `docs/testing/process-global-state.md`.
pub fn response_to_streaming_message<S>(
    stream: S,
) -> impl futures::Stream<Item = anyhow::Result<crate::providers::base::ProviderStreamItem>> + 'static
where
    S: futures::Stream<Item = anyhow::Result<String>> + Unpin + Send + 'static,
{
    response_to_streaming_message_batching(stream, tool_call_batching_enabled())
}

/// [`response_to_streaming_message`] with the batching decision supplied by the
/// caller instead of resolved from the environment.
#[allow(clippy::too_many_lines)]
pub(crate) fn response_to_streaming_message_batching<S>(
    mut stream: S,
    batch_tool_calls: bool,
) -> impl futures::Stream<Item = anyhow::Result<crate::providers::base::ProviderStreamItem>> + 'static
where
    S: futures::Stream<Item = anyhow::Result<String>> + Unpin + Send + 'static,
{
    use async_stream::try_stream;
    use futures::StreamExt;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug)]
    struct StreamingEvent {
        #[serde(rename = "type")]
        event_type: String,
        #[serde(flatten)]
        data: Value,
    }

    try_stream! {
        let mut active_tool_calls: std::collections::HashMap<u64, StreamingToolCall> = std::collections::HashMap::new();
        let mut tool_call_order: std::collections::VecDeque<u64> = std::collections::VecDeque::new();
        let mut completed_tool_contents: std::collections::HashMap<u64, MessageContent> = std::collections::HashMap::new();
        let mut seen_tool_indices: HashSet<u64> = HashSet::new();
        // Extended thinking. A thinking block arrives as N `thinking_delta`
        // chunks followed by one `signature_delta`; a redacted block arrives
        // whole on `content_block_start`. Both are accumulated and yielded once,
        // at `content_block_stop` — NOT incrementally like text.
        //
        // Why not incrementally: `Conversation::push` (conversation/mod.rs:96)
        // merges same-id messages by concatenating only Text+Text and by
        // `extend`ing everything else, so per-delta thinking would land in the
        // transcript as N separate Thinking blocks of which only the last
        // carries the signature. Replaying that assistant turn back to
        // Anthropic (format_messages, :102) would emit unsigned thinking blocks
        // and be rejected. The desktop store (chatStreamStore.tsx) dedups
        // content by JSON equality and appends, so it would also render N
        // thinking bubbles. Incremental thinking needs a replace-by-id merge on
        // both sides first; that is a separate change.
        let mut current_thinking: Option<(String, String)> = None;
        let mut current_redacted: Option<String> = None;
        let mut final_usage: Option<crate::providers::base::ProviderUsage> = None;
        let mut message_id: Option<String> = None;
        // Throttle state for pending-tool-call arg updates. Anthropic sends one
        // `input_json_delta` per few tokens; emitting a notification per delta
        // would turn one tool call into hundreds of SSE frames for a UI that
        // only redraws a truncated preview. Emit at most every
        // PENDING_ARGS_INTERVAL or every PENDING_ARGS_CHARS of new argument
        // text, whichever comes first.
        let mut last_pending_emit: Option<std::time::Instant> = None;
        let mut last_pending_len: usize = 0;

        while let Some(line_result) = stream.next().await {
            let line = line_result?;

            if line.trim().is_empty() {
                continue;
            }

            let Some(data_part) = line.strip_prefix("data:").map(str::trim_start) else {
                continue;
            };

            // Handle end of stream
            if data_part.trim() == "[DONE]" {
                break;
            }

            let event: StreamingEvent = serde_json::from_str(data_part)
                .map_err(|error| anyhow!("Failed to parse Anthropic streaming event: {error}"))?;

            match event.event_type.as_str() {
                "message_start" => {
                    // Message started, we can extract initial metadata and usage if needed
                    if let Some(message_data) = event.data.get("message") {
                        // Extract message ID
                        if let Some(id) = message_data.get("id").and_then(|v| v.as_str()) {
                            message_id = Some(id.to_string());
                        }

                        if let Some(usage_data) = message_data.get("usage") {
                            let usage = get_usage(usage_data).unwrap_or_default();
                            tracing::debug!("🔍 Anthropic message_start parsed usage: input_tokens={:?}, output_tokens={:?}, total_tokens={:?}",
                                    usage.input_tokens, usage.output_tokens, usage.total_tokens);
                            let model = message_data.get("model")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            final_usage = Some(crate::providers::base::ProviderUsage::new(model, usage));
                        } else {
                            tracing::debug!("🔍 Anthropic message_start has no usage data");
                        }
                    }
                    continue;
                }
                "content_block_start" => {
                    // A new content block started
                    if let Some(content_block) = event.data.get("content_block") {
                        if content_block.get("type") == Some(&json!("tool_use")) {
                            if let (Some(index), Some(id)) = (
                                event.data.get("index").and_then(Value::as_u64),
                                content_block.get("id").and_then(Value::as_str),
                            ) {
                                if let Some(name) = content_block.get("name").and_then(|v| v.as_str()) {
                                    if seen_tool_indices.insert(index) {
                                        tool_call_order.push_back(index);
                                        active_tool_calls.insert(index, StreamingToolCall {
                                            tool_id: id.to_string(),
                                            name: name.to_string(),
                                            args: String::new(),
                                        });
                                        // The tool's name is known here; its arguments
                                        // are not (and may take seconds to generate).
                                        // Announce it now so the UI can draw a card
                                        // immediately. NOT a Message — see
                                        // `PendingToolCall`.
                                        last_pending_emit = Some(std::time::Instant::now());
                                        last_pending_len = 0;
                                        yield (None, None, Some(PendingToolCall {
                                            id: id.to_string(),
                                            name: name.to_string(),
                                            partial_args: None,
                                        }));
                                    }
                                }
                            }
                        } else if content_block.get("type") == Some(&json!(THINKING_TYPE)) {
                            let initial = content_block
                                .get(THINKING_TYPE)
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string();
                            let signature = content_block
                                .get(SIGNATURE_FIELD)
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string();
                            current_thinking = Some((initial, signature));
                        } else if content_block.get("type") == Some(&json!(REDACTED_THINKING_TYPE)) {
                            if let Some(data) = content_block.get(DATA_FIELD).and_then(|v| v.as_str()) {
                                current_redacted = Some(data.to_string());
                            }
                        }
                    }
                    continue;
                }
                "content_block_delta" => {
                    if let Some(delta) = event.data.get("delta") {
                        if delta.get("type") == Some(&json!("text_delta")) {
                            // Text content delta
                            if let Some(text) = delta.get("text").and_then(|v| v.as_str()) {

                                // Yield partial text message with the same ID from message_start
                                let mut message = Message::new(
                                    Role::Assistant,
                                    chrono::Utc::now().timestamp(),
                                    vec![MessageContent::text(text)],
                                );
                                message.id = message_id.clone();
                                yield (Some(message), None, None);
                            }
                        } else if delta.get("type") == Some(&json!("input_json_delta")) {
                            // Tool input delta
                            if let Some(tool_call) = event
                                .data
                                .get("index")
                                .and_then(Value::as_u64)
                                .and_then(|index| active_tool_calls.get_mut(&index))
                            {
                                if let Some(partial_json) = delta.get("partial_json").and_then(|v| v.as_str()) {
                                    let mut update: Option<PendingToolCall> = None;
                                    tool_call.args.push_str(partial_json);
                                    // Throttled preview update. Never per delta.
                                    let due_by_size = tool_call.args.len().saturating_sub(last_pending_len) >= PENDING_ARGS_CHARS;
                                    let due_by_time = last_pending_emit
                                        .map(|t| t.elapsed() >= PENDING_ARGS_INTERVAL)
                                        .unwrap_or(true);
                                    if due_by_size || due_by_time {
                                        update = Some(PendingToolCall {
                                            id: tool_call.tool_id.clone(),
                                            name: tool_call.name.clone(),
                                            partial_args: Some(tool_call.args.clone()),
                                        });
                                        last_pending_len = tool_call.args.len();
                                    }
                                    if let Some(update) = update {
                                        last_pending_emit = Some(std::time::Instant::now());
                                        yield (None, None, Some(update));
                                    }
                                }
                            }
                        } else if delta.get("type") == Some(&json!("thinking_delta")) {
                            // Extended-thinking text delta; buffered, not yielded.
                            if let Some(chunk) = delta.get(THINKING_TYPE).and_then(|v| v.as_str()) {
                                current_thinking
                                    .get_or_insert_with(|| (String::new(), String::new()))
                                    .0
                                    .push_str(chunk);
                            }
                        } else if delta.get("type") == Some(&json!("signature_delta")) {
                            // Closes a thinking block; without it Anthropic
                            // rejects the block when it is replayed.
                            if let Some(chunk) = delta.get(SIGNATURE_FIELD).and_then(|v| v.as_str()) {
                                current_thinking
                                    .get_or_insert_with(|| (String::new(), String::new()))
                                    .1
                                    .push_str(chunk);
                            }
                        }
                    }
                    continue;
                }
                "content_block_stop" => {
                    // A thinking block closed: emit it complete, with its signature.
                    if let Some((thinking, signature)) = current_thinking.take() {
                        if !thinking.is_empty() || !signature.is_empty() {
                            let mut message = Message::assistant().with_thinking(thinking, signature);
                            message.id = message_id.clone();
                            yield (Some(message), None, None);
                        }
                    }
                    if let Some(data) = current_redacted.take() {
                        let mut message = Message::assistant().with_redacted_thinking(data);
                        message.id = message_id.clone();
                        yield (Some(message), None, None);
                    }
                    // Content block finished
                    if let Some((index, tool_call)) = event
                        .data
                        .get("index")
                        .and_then(Value::as_u64)
                        .and_then(|index| active_tool_calls.remove(&index).map(|call| (index, call)))
                    {
                        completed_tool_contents.insert(
                            index,
                            completed_tool_content(
                                tool_call.tool_id,
                                tool_call.name,
                                &tool_call.args,
                            ),
                        );

                        if !batch_tool_calls {
                            for content in take_ready_tool_contents(
                                &mut tool_call_order,
                                &mut completed_tool_contents,
                            ) {
                                let mut message = Message::new(
                                    Role::Assistant,
                                    chrono::Utc::now().timestamp(),
                                    vec![content],
                                );
                                message.id = message_id.clone();
                                yield (Some(message), None, None);
                            }
                        }
                    }
                    continue;
                }
                "message_delta" => {
                    drain_incomplete_tool_contents(
                        &mut active_tool_calls,
                        &tool_call_order,
                        &mut completed_tool_contents,
                    );
                    let mut pending_tool_contents = take_ready_tool_contents(
                        &mut tool_call_order,
                        &mut completed_tool_contents,
                    );
                    if !batch_tool_calls {
                        for content in pending_tool_contents.drain(..) {
                            let mut message = Message::new(
                                Role::Assistant,
                                chrono::Utc::now().timestamp(),
                                vec![content],
                            );
                            message.id = message_id.clone();
                            yield (Some(message), None, None);
                        }
                    }
                    // §6.2b: a message_delta closes the response, so every tool
                    // block that will arrive already has. Flush the batch here and
                    // yield it TOGETHER with this delta's usage snapshot below —
                    // agent.rs reads (message, usage) in one match arm.
                    let batched_tool_message = batch_tool_calls
                        .then(|| flush_pending_tool_contents(&mut pending_tool_contents, &message_id))
                        .flatten();

                    // Message metadata delta (like stop_reason) and cumulative usage
                    tracing::debug!("🔍 Anthropic message_delta event data: {}", serde_json::to_string_pretty(&event.data).unwrap_or_else(|_| format!("{:?}", event.data)));

                    // Anthropic reports why generation stopped in `delta.stop_reason`
                    // (e.g. "max_tokens", "end_turn", "tool_use"). Map it onto the
                    // OpenAI-style finish_reason so the agent loop can auto-continue a
                    // response cut off by the output-length limit ("length") instead
                    // of ending the turn silently mid-sentence.
                    let finish_reason = event.data
                        .get("delta")
                        .and_then(|d| d.get("stop_reason"))
                        .and_then(|v| v.as_str())
                        .map(map_stop_reason);

                    if let Some(usage_data) = event.data.get("usage") {
                        tracing::debug!("🔍 Anthropic message_delta usage data (cumulative): {}", serde_json::to_string_pretty(usage_data).unwrap_or_else(|_| format!("{:?}", usage_data)));
                        let delta_usage = get_usage(usage_data).unwrap_or_default();
                        tracing::debug!("🔍 Anthropic message_delta parsed usage: input_tokens={:?}, output_tokens={:?}, total_tokens={:?}",
                                delta_usage.input_tokens, delta_usage.output_tokens, delta_usage.total_tokens);

                        if let Some(existing_usage) = &final_usage {
                            let model = existing_usage.model.clone();
                            let merged_usage = merge_streaming_usage(existing_usage.usage, delta_usage);
                            tracing::debug!("🔍 Anthropic MERGED usage: input_tokens={:?}, output_tokens={:?}, total_tokens={:?}",
                                    merged_usage.input_tokens, merged_usage.output_tokens, merged_usage.total_tokens);
                            final_usage = Some(crate::providers::base::ProviderUsage::new(model, merged_usage));
                        } else {
                            // No existing usage, just use delta usage
                            let model = event.data.get("model")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            final_usage = Some(crate::providers::base::ProviderUsage::new(model, delta_usage));
                            tracing::debug!("🔍 Anthropic no existing usage, using delta usage");
                        }

                    } else {
                        tracing::debug!("🔍 Anthropic message_delta event has no usage field");
                    }

                    // Attach the mapped finish_reason to the running snapshot. A
                    // message_delta usually carries stop_reason alongside usage, but
                    // surface it even if this delta had no usage field yet.
                    if let Some(reason) = finish_reason {
                        match final_usage.as_mut() {
                            Some(existing) => existing.finish_reason = Some(reason),
                            None => {
                                let model = event.data.get("model")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown")
                                    .to_string();
                                let mut usage = crate::providers::base::ProviderUsage::new(
                                    model,
                                    crate::providers::base::Usage::default(),
                                );
                                usage.finish_reason = Some(reason);
                                final_usage = Some(usage);
                            }
                        }
                    }

                    // Emit a running snapshot. Anthropic reports usage only on
                    // the terminal chunk, so a cancelled turn used to record
                    // nothing at all even though the full input and the partial
                    // output were billed. The agent keeps the LAST snapshot per
                    // turn, so re-yielding here cannot double count.
                    //
                    // §6.2b: carry the batched tool message on the same item so the
                    // agent sees message + usage together.
                    if let Some(snapshot) = final_usage.clone() {
                        yield (batched_tool_message, Some(snapshot), None);
                    } else if batched_tool_message.is_some() {
                        yield (batched_tool_message, None, None);
                    }
                    continue;
                }
                "message_stop" => {
                    // Message finished, extract final usage if available
                    if let Some(usage_data) = event.data.get("usage") {
                        tracing::debug!("🔍 Anthropic streaming usage data: {}", serde_json::to_string_pretty(usage_data).unwrap_or_else(|_| format!("{:?}", usage_data)));
                        let usage = get_usage(usage_data).unwrap_or_default();
                        tracing::debug!("🔍 Anthropic parsed usage: input_tokens={:?}, output_tokens={:?}, total_tokens={:?}",
                                usage.input_tokens, usage.output_tokens, usage.total_tokens);
                        let model = event.data.get("model")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        tracing::debug!("🔍 Anthropic final_usage created with model: {}", model);
                        // Preserve any finish_reason mapped from an earlier
                        // message_delta so the terminal snapshot still reports it.
                        let prior_finish_reason =
                            final_usage.as_ref().and_then(|u| u.finish_reason.clone());
                        let mut provider_usage =
                            crate::providers::base::ProviderUsage::new(model, usage);
                        provider_usage.finish_reason = prior_finish_reason;
                        final_usage = Some(provider_usage);
                    } else {
                        tracing::debug!("🔍 Anthropic message_stop event has no usage data");
                    }
                    break;
                }
                _ => {
                    // Unknown event type, log and continue
                    tracing::debug!("Unknown streaming event type: {}", event.event_type);
                    continue;
                }
            }
        }

        drain_incomplete_tool_contents(
            &mut active_tool_calls,
            &tool_call_order,
            &mut completed_tool_contents,
        );
        let mut pending_tool_contents = take_ready_tool_contents(
            &mut tool_call_order,
            &mut completed_tool_contents,
        );
        if !batch_tool_calls {
            for content in pending_tool_contents.drain(..) {
                let mut message = Message::new(
                    Role::Assistant,
                    chrono::Utc::now().timestamp(),
                    vec![content],
                );
                message.id = message_id.clone();
                yield (Some(message), None, None);
            }
        }

        // §6.2b: a stream that ended WITHOUT a message_delta (e.g. straight to
        // message_stop / [DONE], or truncated cleanly) still has its buffered tool
        // blocks flushed here — otherwise a whole multi-tool turn would silently
        // vanish. A no-op in the common path (message_delta already drained it).
        let batched_tool_message = batch_tool_calls
            .then(|| flush_pending_tool_contents(&mut pending_tool_contents, &message_id))
            .flatten();

        // Yield final usage information if available, together with any batched
        // tool message (agent.rs reads message + usage in one match arm).
        if let Some(usage) = final_usage {
            yield (batched_tool_message, Some(usage), None);
        } else {
            if let Some(message) = batched_tool_message {
                yield (Some(message), None, None);
            }
            tracing::debug!("🔍 Anthropic no final usage to yield");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::effort::{ReasoningEffort, DEEP_THINKING_BUDGET_TOKENS};
    use crate::conversation::message::Message;
    use rmcp::object;
    use serde_json::json;

    // BR-63: `deep` is how a user asks Anthropic for extended thinking without
    // setting a process-wide env var. The API rejects a temperature other than 1
    // alongside a thinking block, so a thinking turn must send none.
    #[test]
    fn test_create_request_deep_effort_enables_thinking() -> Result<()> {
        let mut model_config = ModelConfig::new_or_fail("claude-sonnet-4-5")
            .with_reasoning_effort(Some(ReasoningEffort::Deep));
        model_config.max_tokens = Some(8000);
        model_config.temperature = Some(0.3);

        let payload = create_request(
            &model_config,
            "system",
            &[Message::user().with_text("hi")],
            &[],
        )?;

        let thinking = payload.get("thinking").expect("thinking block");
        assert_eq!(thinking["type"], "enabled");
        assert_eq!(
            thinking["budget_tokens"],
            json!(DEEP_THINKING_BUDGET_TOKENS)
        );
        // max_tokens grows by the budget, and temperature is dropped.
        assert_eq!(
            payload["max_tokens"],
            json!(8000 + DEEP_THINKING_BUDGET_TOKENS as i32)
        );
        assert!(payload.get("temperature").is_none());
        Ok(())
    }

    #[test]
    fn test_create_request_without_deep_effort_has_no_thinking() -> Result<()> {
        let mut model_config = ModelConfig::new_or_fail("claude-sonnet-4-5")
            .with_reasoning_effort(Some(ReasoningEffort::Quick));
        model_config.temperature = Some(0.5);

        let payload = create_request(
            &model_config,
            "system",
            &[Message::user().with_text("hi")],
            &[],
        )?;

        // Quick asks for no thinking block — and without one, a configured
        // temperature is still sent as before.
        assert!(payload.get("thinking").is_none());
        assert_eq!(payload["temperature"], json!(0.5));
        Ok(())
    }

    // The modern Claude models (Opus 5 / Sonnet 5 / Fable 5 / Opus 4.7 / 4.8)
    // removed `budget_tokens` — sending `thinking: {type: "enabled", ...}`
    // returns a 400 — and reject the sampling parameters outright. A
    // deep-effort turn on them must switch to `{"type": "adaptive"}` and
    // never carry temperature; before this gate, every deep-effort turn on
    // the default model (claude-opus-4-8) failed with a 400.
    #[test]
    fn test_deep_effort_on_modern_models_uses_adaptive_thinking() -> Result<()> {
        for model in [
            "claude-opus-5",
            "claude-sonnet-5",
            "claude-fable-5",
            "claude-opus-4-7",
            "claude-opus-4-8",
            // Platform-prefixed and dotted spellings must land on the same
            // path — Versa/Bedrock and OpenRouter/Copilot ids reach this
            // code too.
            "us.anthropic.claude-opus-4-8-v1",
            "anthropic/claude-opus-4.8",
        ] {
            let mut model_config =
                ModelConfig::new_or_fail(model).with_reasoning_effort(Some(ReasoningEffort::Deep));
            model_config.max_tokens = Some(8000);
            model_config.temperature = Some(0.3);

            let payload = create_request(
                &model_config,
                "system",
                &[Message::user().with_text("hi")],
                &[],
            )?;

            let thinking = payload.get("thinking").expect("thinking block");
            assert_eq!(thinking["type"], "adaptive", "{model}");
            assert!(
                thinking.get("budget_tokens").is_none(),
                "{model}: budget_tokens is removed on this model"
            );
            // max_tokens must stay exactly as configured: adaptive thinking
            // has no budget to make room for, and inflating it can exceed
            // the model's output ceiling (128K on Opus 5).
            assert_eq!(payload["max_tokens"], json!(8000), "{model}");
            assert!(
                payload.get("temperature").is_none(),
                "{model}: sampling params are rejected on this model"
            );
        }
        Ok(())
    }

    // The 128k boundary case: Opus 5 caps output (thinking + response
    // together) at 128K. A 128k max_tokens config plus the 16k default deep
    // budget used to produce max_tokens = 144_000 — a guaranteed 400 from
    // the API. The adaptive path must send the configured value untouched.
    #[test]
    fn test_adaptive_thinking_never_exceeds_configured_max_tokens() -> Result<()> {
        let mut model_config = ModelConfig::new_or_fail("claude-opus-5")
            .with_reasoning_effort(Some(ReasoningEffort::Deep));
        model_config.max_tokens = Some(128_000);

        let payload = create_request(
            &model_config,
            "system",
            &[Message::user().with_text("hi")],
            &[],
        )?;

        assert_eq!(payload["thinking"]["type"], "adaptive");
        assert_eq!(
            payload["max_tokens"],
            json!(128_000),
            "128k config + default deep budget must not become 144k"
        );
        Ok(())
    }

    // The adaptive-thinking gate itself, across every id spelling that can
    // reach it: bare Anthropic ids, dotted OpenRouter/Copilot variants, and
    // platform-prefixed Bedrock/proxy ids — plus boundary negatives that a
    // sloppy substring match would swallow.
    #[test]
    fn test_adaptive_thinking_gate_covers_all_id_spellings() {
        let adaptive_only = [
            "claude-opus-5",
            "anthropic/claude-opus-5",
            "us.anthropic.claude-opus-5-v1:0",
            "claude-sonnet-5",
            "claude-fable-5",
            "claude-mythos-5",
            "claude-opus-4-7",
            "claude-opus-4.7",
            "claude-opus-4-8",
            "claude-opus-4.8",
            "us.anthropic.claude-opus-4-8-v1",
            "anthropic/claude-opus-4.8",
        ];
        for model in adaptive_only {
            assert!(
                uses_adaptive_thinking(model),
                "{model} must be adaptive-only"
            );
        }

        let budgeted = [
            // The 4.6 pair only deprecates budget_tokens — still budgeted.
            "claude-opus-4-6",
            "us.anthropic.claude-opus-4-6-v1",
            "claude-sonnet-4-6",
            // Older tiers and their dated/prefixed spellings.
            "claude-opus-4-5",
            "us.anthropic.claude-opus-4-5-20251101-v1:0",
            "claude-opus-4-1-20250805",
            "claude-haiku-4-5",
            "claude-sonnet-4-20250514",
            // Boundary negatives: contain "opus-4-6" / "sonnet-4" as
            // substrings but are unknown tiers — they must stay on the
            // conservative budgeted path, not match a 4-7/4-8 rule.
            "claude-opus-4-68",
            "claude-sonnet-4-62",
        ];
        for model in budgeted {
            assert!(
                !uses_adaptive_thinking(model),
                "{model} must stay on the budgeted path"
            );
        }
    }

    #[test]
    fn test_modern_models_never_send_temperature_even_without_thinking() -> Result<()> {
        let mut model_config = ModelConfig::new_or_fail("claude-opus-5")
            .with_reasoning_effort(Some(ReasoningEffort::Quick));
        model_config.temperature = Some(0.5);

        let payload = create_request(
            &model_config,
            "system",
            &[Message::user().with_text("hi")],
            &[],
        )?;

        assert!(payload.get("thinking").is_none());
        assert!(
            payload.get("temperature").is_none(),
            "temperature is rejected on Opus 5 even with thinking off"
        );
        Ok(())
    }

    // Opus 4.6 / Sonnet 4.6 only deprecate budget_tokens (still functional)
    // and still accept temperature — they stay on the legacy budgeted path,
    // as do all older models (covered by the sonnet-4-5 tests above).
    #[test]
    fn test_sonnet_4_6_keeps_budgeted_thinking() -> Result<()> {
        let model_config = ModelConfig::new_or_fail("claude-sonnet-4-6")
            .with_reasoning_effort(Some(ReasoningEffort::Deep));

        let payload = create_request(
            &model_config,
            "system",
            &[Message::user().with_text("hi")],
            &[],
        )?;

        let thinking = payload.get("thinking").expect("thinking block");
        assert_eq!(thinking["type"], "enabled");
        assert_eq!(
            thinking["budget_tokens"],
            json!(DEEP_THINKING_BUDGET_TOKENS)
        );
        Ok(())
    }

    #[test]
    fn test_parse_text_response() -> Result<()> {
        let response = json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "text",
                "text": "Hello! How can I assist you today?"
            }],
            "model": "claude-sonnet-4-20250514",
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": {
                "input_tokens": 12,
                "output_tokens": 15,
                "cache_creation_input_tokens": 12,
                "cache_read_input_tokens": 0
            }
        });

        let message = response_to_message(&response)?;
        let usage = get_usage(&response)?;

        if let MessageContent::Text(text) = &message.content[0] {
            assert_eq!(text.text, "Hello! How can I assist you today?");
        } else {
            panic!("Expected Text content");
        }

        // Fresh input and cache are disjoint now: input=12, cache_creation=12.
        assert_eq!(usage.input_tokens, Some(12));
        assert_eq!(usage.output_tokens, Some(15));
        assert_eq!(usage.cache_creation_input_tokens, Some(12));
        assert_eq!(usage.cache_read_input_tokens, Some(0));
        assert_eq!(usage.total_tokens, Some(39)); // 12 + 12 + 0 + 15 context occupancy
        assert_eq!(usage.billed_total(), Some(39)); // reconciles with the vendor total

        Ok(())
    }

    #[test]
    fn test_parse_tool_response() -> Result<()> {
        let response = json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "tool_1",
                "name": "calculator",
                "input": {
                    "expression": "2 + 2"
                }
            }],
            "model": "claude-3-sonnet-20240229",
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": {
                "input_tokens": 15,
                "output_tokens": 20,
                "cache_creation_input_tokens": 15,
                "cache_read_input_tokens": 0,
            }
        });

        let message = response_to_message(&response)?;
        let usage = get_usage(&response)?;

        if let MessageContent::ToolRequest(tool_request) = &message.content[0] {
            let tool_call = tool_request.tool_call.as_ref().unwrap();
            assert_eq!(tool_call.name, "calculator");
            assert_eq!(tool_call.arguments, Some(object!({"expression": "2 + 2"})));
        } else {
            panic!("Expected ToolRequest content");
        }

        assert_eq!(usage.input_tokens, Some(15)); // fresh input only
        assert_eq!(usage.output_tokens, Some(20));
        assert_eq!(usage.cache_creation_input_tokens, Some(15));
        assert_eq!(usage.total_tokens, Some(50)); // 15 + 15 + 0 + 20
        assert_eq!(usage.billed_total(), Some(50));

        Ok(())
    }

    #[test]
    fn test_parse_thinking_response() -> Result<()> {
        let response = json!({
            "id": "msg_456",
            "type": "message",
            "role": "assistant",
            "content": [
                {
                    "type": "thinking",
                    "thinking": "This is a step-by-step thought process...",
                    "signature": "EuYBCkQYAiJAVbJNBoH7HQiDcMwwAMhWqNyoe4G2xHRprK8ICM8gZzu16i7Se4EiEbmlKqNH1GtwcX1BMK6iLu8bxWn5wPVIFBIMnptdlVal7ZX5iNPFGgwWjX+BntcEOHky4HciMFVef7FpQeqnuiL1Xt7J4OLHZSyu4tcr809AxAbclcJ5dm1xE5gZrUO+/v60cnJM2ipQp4B8/3eHI03KSV6bZR/vMrBSYCV+aa/f5KHX2cRtLGp/Ba+3Tk/efbsg01WSduwAIbR4coVrZLnGJXNyVTFW/Be2kLy/ECZnx8cqvU3oQOg="
                },
                {
                    "type": "redacted_thinking",
                    "data": "EmwKAhgBEgy3va3pzix/LafPsn4aDFIT2Xlxh0L5L8rLVyIwxtE3rAFBa8cr3qpP"
                },
                {
                    "type": "text",
                    "text": "I've analyzed the problem and here's the solution."
                }
            ],
            "model": "claude-3-7-sonnet-20250219",
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": {
                "input_tokens": 10,
                "output_tokens": 45,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
            }
        });

        let message = response_to_message(&response)?;
        let usage = get_usage(&response)?;

        assert_eq!(message.content.len(), 3);

        if let MessageContent::Thinking(thinking) = &message.content[0] {
            assert_eq!(
                thinking.thinking,
                "This is a step-by-step thought process..."
            );
            assert!(thinking
                .signature
                .starts_with("EuYBCkQYAiJAVbJNBoH7HQiDcMwwAMhWqNyoe4G2xHRprK8ICM8g"));
        } else {
            panic!("Expected Thinking content at index 0");
        }

        if let MessageContent::RedactedThinking(redacted) = &message.content[1] {
            assert_eq!(
                redacted.data,
                "EmwKAhgBEgy3va3pzix/LafPsn4aDFIT2Xlxh0L5L8rLVyIwxtE3rAFBa8cr3qpP"
            );
        } else {
            panic!("Expected RedactedThinking content at index 1");
        }

        if let MessageContent::Text(text) = &message.content[2] {
            assert_eq!(
                text.text,
                "I've analyzed the problem and here's the solution."
            );
        } else {
            panic!("Expected Text content at index 2");
        }

        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.output_tokens, Some(45));
        assert_eq!(usage.total_tokens, Some(55));

        Ok(())
    }

    /// A native Anthropic extended-thinking stream delivers the reasoning as
    /// `thinking_delta` chunks followed by a single `signature_delta`. The
    /// decoder must surface one complete, *signed* Thinking block — dropping it
    /// is a correctness bug, not a cosmetic one: Anthropic rejects a follow-up
    /// request whose assistant turn lost its thinking blocks.
    #[tokio::test]
    async fn test_streaming_surfaces_thinking_with_signature() -> Result<()> {
        use tokio::pin;
        use tokio_stream::StreamExt;

        let lines = vec![
            r#"data: {"type":"message_start","message":{"id":"msg_think","model":"claude-sonnet-4-20250514","usage":{"input_tokens":10,"output_tokens":1}}}"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me think "}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"step by step."}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EuYBCkQYAiJAsig"}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"redacted_thinking","data":"EmwKAhgBEgy3va3p"}}"#,
            r#"data: {"type":"content_block_stop","index":1}"#,
            r#"data: {"type":"content_block_start","index":2,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"text_delta","text":"Here is the answer."}}"#,
            r#"data: {"type":"content_block_stop","index":2}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":20}}"#,
            r#"data: {"type":"message_stop"}"#,
        ];
        let response_stream = tokio_stream::iter(lines.into_iter().map(|l| Ok(l.to_string())));
        let messages = response_to_streaming_message_batching(response_stream, true);
        pin!(messages);

        let mut thinking_blocks = Vec::new();
        let mut redacted_blocks = Vec::new();
        let mut text = String::new();
        while let Some(result) = messages.next().await {
            let (message, _usage, _pending) = result?;
            let Some(message) = message else { continue };
            assert_eq!(
                message.id.as_deref(),
                Some("msg_think"),
                "streamed messages must carry the message_start id"
            );
            for content in &message.content {
                match content {
                    MessageContent::Thinking(t) => thinking_blocks.push(t.clone()),
                    MessageContent::RedactedThinking(r) => redacted_blocks.push(r.clone()),
                    MessageContent::Text(t) => text.push_str(&t.text),
                    _ => {}
                }
            }
        }

        // Exactly one Thinking block: yielded whole at content_block_stop, not
        // once per delta (see the module comment on the streaming decoder).
        assert_eq!(
            thinking_blocks.len(),
            1,
            "expected exactly one complete Thinking block, got {}",
            thinking_blocks.len()
        );
        assert_eq!(thinking_blocks[0].thinking, "Let me think step by step.");
        assert_eq!(
            thinking_blocks[0].signature, "EuYBCkQYAiJAsig",
            "the signature_delta must survive decoding"
        );

        assert_eq!(redacted_blocks.len(), 1);
        assert_eq!(redacted_blocks[0].data, "EmwKAhgBEgy3va3p");

        assert_eq!(text, "Here is the answer.");
        Ok(())
    }

    /// Collect every authoritative `ToolRequest`, grouped by the message it
    /// arrived in, from a decoded stream. `[2]` means one message carried two
    /// requests (batched); `[1, 1]` means two separate one-request messages
    /// (the pre-§6.2b serial shape).
    async fn tool_request_message_shape(lines: Vec<&'static str>, batch: bool) -> Vec<usize> {
        use tokio::pin;
        use tokio_stream::StreamExt;

        let response_stream = tokio_stream::iter(lines.into_iter().map(|l| Ok(l.to_string())));
        let messages = response_to_streaming_message_batching(response_stream, batch);
        pin!(messages);

        let mut shape = Vec::new();
        while let Some(result) = messages.next().await {
            let (message, _usage, _pending) = result.unwrap();
            let Some(message) = message else { continue };
            let count = message
                .content
                .iter()
                .filter(|c| matches!(c, MessageContent::ToolRequest(_)))
                .count();
            if count > 0 {
                shape.push(count);
            }
        }
        shape
    }

    /// A response with two `tool_use` blocks. Reused by the batching tests
    /// (with a terminal `message_delta`) and, sliced, by the no-delta
    /// regression test.
    const TWO_TOOL_USE_LINES: [&str; 8] = [
        r#"data: {"type":"message_start","message":{"id":"msg_batch","model":"claude-sonnet-4-20250514","usage":{"input_tokens":5,"output_tokens":1}}}"#,
        r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_a","name":"developer__shell"}}"#,
        r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"ls\"}"}}"#,
        r#"data: {"type":"content_block_stop","index":0}"#,
        r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_b","name":"developer__text_editor"}}"#,
        r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"view\",\"path\":\"/tmp/x\"}"}}"#,
        r#"data: {"type":"content_block_stop","index":1}"#,
        r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":20}}"#,
    ];

    /// §6.2b core gate: two `tool_use` blocks in one response must decode to
    /// **one** assistant message carrying **two** `ToolRequest`s — so the agent's
    /// `select_all` sees two futures and dispatches them in parallel. Before
    /// §6.2b the native decoder emitted one message per block (`[1, 1]`), which
    /// forced serial execution; that shape is what the kill switch restores
    /// (`test_streaming_kill_switch_restores_serial_tool_messages`).
    ///
    /// ⚠ This test states its batching input rather than inheriting it, and the
    /// argument is what makes that possible. It used to carry
    /// `#[serial(tool_call_batching_env)]`, which excluded only the two other
    /// tests holding that key while the flag was read live, unlocked, by every
    /// decoder in the binary — so it asserted `[1, 1] == [2]` whenever anything
    /// else wrote the variable. `test (ubuntu-latest)` went red on a diff
    /// nowhere near it, and 7 of 8 local runs failed.
    #[tokio::test]
    async fn test_streaming_batches_multiple_tool_uses_into_one_message() -> Result<()> {
        use tokio::pin;
        use tokio_stream::StreamExt;

        let response_stream =
            tokio_stream::iter(TWO_TOOL_USE_LINES.iter().map(|l| Ok(l.to_string())));
        let messages = response_to_streaming_message_batching(response_stream, true);
        pin!(messages);

        let mut items = Vec::new();
        while let Some(result) = messages.next().await {
            items.push(result?);
        }

        // Exactly ONE message carries tool requests, and it carries BOTH.
        let tool_messages: Vec<&Message> = items
            .iter()
            .filter_map(|(m, _, _)| m.as_ref())
            .filter(|m| {
                m.content
                    .iter()
                    .any(|c| matches!(c, MessageContent::ToolRequest(_)))
            })
            .collect();
        assert_eq!(
            tool_messages.len(),
            1,
            "two tool_use blocks must batch into ONE assistant message, not one per block"
        );

        let requests: Vec<_> = tool_messages[0]
            .content
            .iter()
            .filter_map(|c| match c {
                MessageContent::ToolRequest(r) => Some(r.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            requests.len(),
            2,
            "the single message must carry BOTH tool requests"
        );

        // Request order is preserved (block 0 then block 1). Anthropic 400s if a
        // later tool-result batch is ordered against a different sequence, so the
        // request order the decoder emits is load-bearing.
        assert_eq!(requests[0].id, "toolu_a");
        assert_eq!(requests[1].id, "toolu_b");
        assert_eq!(
            requests[0].tool_call.as_ref().unwrap().name,
            "developer__shell"
        );
        assert_eq!(
            requests[1].tool_call.as_ref().unwrap().name,
            "developer__text_editor"
        );

        // The batched message rides the SAME stream item as the terminal usage
        // snapshot (agent.rs reads message + usage in one match arm).
        let batched_item = items
            .iter()
            .find(|(m, _, _)| {
                m.as_ref().is_some_and(|m| {
                    m.content
                        .iter()
                        .any(|c| matches!(c, MessageContent::ToolRequest(_)))
                })
            })
            .unwrap();
        assert!(
            batched_item.1.is_some(),
            "the batched tool message must be yielded together with the usage snapshot"
        );
        Ok(())
    }

    /// §6.2b regression: a stream that ends after the final `content_block_stop`
    /// with **no** `message_delta` (straight to `[DONE]`) must still flush its
    /// batched tools via the after-loop flush — otherwise a whole multi-tool turn
    /// would silently vanish.
    ///
    /// ⚠ It decodes two `tool_use` blocks, so it is batching-sensitive and states
    /// its input, for the reason spelled out on
    /// `test_streaming_batches_multiple_tool_uses_into_one_message`. Any future
    /// test that decodes more than one tool block must do the same.
    #[tokio::test]
    async fn test_streaming_batches_tools_without_message_delta() -> Result<()> {
        // Drop the trailing message_delta; end with [DONE] instead.
        let mut lines: Vec<&'static str> = TWO_TOOL_USE_LINES[..7].to_vec();
        lines.push(r#"data: [DONE]"#);

        let shape = tool_request_message_shape(lines, true).await;
        assert_eq!(
            shape,
            vec![2],
            "a stream ending without a message_delta must still deliver both tools \
             batched into one message (after-loop flush)"
        );
        Ok(())
    }

    /// §6.2b kill switch: `BIOROUTER_TOOL_CALL_BATCHING=0` restores the pre-§6.2b
    /// serial shape — one assistant message per `tool_use` block (`[1, 1]`). This
    /// is the full rollback lever documented alongside `BIOROUTER_TOOL_WRITE_ORDERING`.
    ///
    /// Injects the decision rather than writing `BIOROUTER_TOOL_CALL_BATCHING`.
    /// The env write this used to make was read by every OTHER decoder in the
    /// binary — a live, unlocked `std::env::var` — so `serial_test` could not
    /// contain it. The env *parsing* is covered separately by
    /// `providers::base`'s own tests.
    #[tokio::test]
    async fn test_streaming_kill_switch_restores_serial_tool_messages() -> Result<()> {
        let shape = tool_request_message_shape(TWO_TOOL_USE_LINES.to_vec(), false).await;

        assert_eq!(
            shape,
            vec![1, 1],
            "with batching OFF each tool_use block must be its own message (serial)"
        );
        Ok(())
    }

    /// §6.1b safety: a pending tool-call notification carries the tool NAME
    /// before any argument bytes, is emitted throttled (not per delta), and is
    /// never a `Message` — so it is structurally incapable of being dispatched.
    #[tokio::test]
    async fn test_streaming_emits_pending_tool_call_before_args() -> Result<()> {
        use tokio::pin;
        use tokio_stream::StreamExt;

        // A tool_use block whose argument JSON arrives across several deltas.
        let lines = vec![
            r#"data: {"type":"message_start","message":{"id":"msg_p","model":"claude-sonnet-4-20250514","usage":{"input_tokens":5,"output_tokens":1}}}"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_pending","name":"developer__shell"}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\":"}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"rm -rf /\"}"}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":9}}"#,
        ];
        let response_stream = tokio_stream::iter(lines.into_iter().map(|l| Ok(l.to_string())));
        let messages = response_to_streaming_message_batching(response_stream, true);
        pin!(messages);

        let mut items = Vec::new();
        while let Some(result) = messages.next().await {
            items.push(result?);
        }

        // (a) The FIRST item is a pending notification, name known, no message,
        //     and no partial args yet — i.e. it precedes every argument byte.
        let (first_msg, _first_usage, first_pending) = &items[0];
        assert!(first_msg.is_none(), "pending item must not carry a Message");
        let first_pending = first_pending
            .as_ref()
            .expect("first stream item must be a pending tool-call notification");
        assert_eq!(first_pending.name, "developer__shell");
        assert_eq!(first_pending.id, "toolu_pending");
        assert!(
            first_pending.partial_args.is_none(),
            "the announcement must arrive before any argument bytes"
        );

        // (c) EVERY pending item is message-free: a partial can never reach
        //     dispatch, whatever its throttled args say.
        for (msg, _usage, pending) in &items {
            if pending.is_some() {
                assert!(
                    msg.is_none(),
                    "a pending notification must never carry a Message"
                );
            }
        }

        // (b) Exactly ONE authoritative ToolRequest, emitted at content_block_stop,
        //     with the complete, correctly-parsed arguments.
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
            1,
            "exactly one authoritative ToolRequest per tool_use block"
        );
        let call = tool_requests[0]
            .tool_call
            .as_ref()
            .expect("the authoritative request parsed cleanly");
        assert_eq!(call.name, "developer__shell");
        assert_eq!(
            call.arguments.as_ref().unwrap().get("command").unwrap(),
            "rm -rf /"
        );
        Ok(())
    }

    /// Replay gate: a thinking block decoded off the stream must round-trip back
    /// into a subsequent request body in the exact shape Anthropic accepts.
    ///
    /// This deliberately reproduces the pipeline the agent loop actually runs
    /// rather than concatenating the decoded chunks by hand:
    ///
    ///  * the fixture contains a thinking block **followed by a tool_use
    ///    block**, so the constraint that actually bites is exercised — with
    ///    extended thinking on, the assistant message that carries `tool_use`
    ///    must *begin* with a thinking block;
    ///  * the decoded chunks are fed through the real `Conversation::push`,
    ///    whose id-matching merge is the only thing that joins them;
    ///  * the tool-bearing chunk is rebuilt exactly as `agent.rs` rebuilds it,
    ///    via the real `assistant_turn_message_id`, which is what decides
    ///    whether the merge happens at all.
    ///
    /// Concatenating the chunks by hand (the previous shape of this test) hides
    /// the defect completely: it passes even when the live pipeline emits two
    /// consecutive assistant messages with `tool_use` first, which Anthropic
    /// 400-rejects.
    #[tokio::test]
    async fn test_streamed_thinking_replays_into_next_request() -> Result<()> {
        use crate::agents::agent::assistant_turn_message_id;
        use crate::conversation::Conversation;
        use tokio::pin;
        use tokio_stream::StreamExt;

        let lines = vec![
            r#"data: {"type":"message_start","message":{"id":"msg_replay","model":"claude-sonnet-4-20250514","usage":{"input_tokens":10,"output_tokens":1}}}"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Reasoning."}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"SIGabc123"}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"redacted_thinking","data":"REDACTEDdata"}}"#,
            r#"data: {"type":"content_block_stop","index":1}"#,
            r#"data: {"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_01","name":"shell"}}"#,
            r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"ls\"}"}}"#,
            r#"data: {"type":"content_block_stop","index":2}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":9}}"#,
        ];
        let response_stream = tokio_stream::iter(lines.into_iter().map(|l| Ok(l.to_string())));
        let messages = response_to_streaming_message_batching(response_stream, true);
        pin!(messages);

        // Mirror the agent loop: a chunk with no tool requests is pushed
        // verbatim; a chunk that requests tools is rebuilt as a fresh assistant
        // message stamped with `assistant_turn_message_id`.
        let mut conversation = Conversation::new_unvalidated(vec![Message::user().with_text("hi")]);
        let mut saw_tool_use = false;
        while let Some(result) = messages.next().await {
            let (message, _usage, _pending) = result?;
            let Some(message) = message else { continue };

            let tool_requests: Vec<_> = message
                .content
                .iter()
                .filter_map(|c| match c {
                    MessageContent::ToolRequest(r) => Some(r.clone()),
                    _ => None,
                })
                .collect();

            if tool_requests.is_empty() {
                conversation.push(message);
                continue;
            }

            saw_tool_use = true;
            // The loop consumes the provider id once and falls back to fresh
            // uuids after that; mirrored here so the fixture stays faithful.
            let mut turn_id = Some(assistant_turn_message_id(&message));
            for request in tool_requests {
                let id = turn_id
                    .take()
                    .unwrap_or_else(|| format!("msg_{}", uuid::Uuid::new_v4()));
                conversation.push(
                    Message::assistant()
                        .with_id(id)
                        .with_tool_request(request.id.clone(), request.tool_call.clone()),
                );
            }
        }
        assert!(saw_tool_use, "fixture must produce a tool_use block");

        let spec = format_messages(conversation.messages());

        let assistant_entries: Vec<_> = spec
            .iter()
            .filter(|m| m["role"] == ASSISTANT_ROLE)
            .collect();
        assert_eq!(
            assistant_entries.len(),
            1,
            "thinking and tool_use must land in ONE assistant message; two consecutive \
             assistant entries are rejected by Anthropic. Got: {spec:#?}"
        );

        let blocks = assistant_entries[0]["content"]
            .as_array()
            .expect("assistant content array");
        assert_eq!(
            blocks[0]["type"], "thinking",
            "with extended thinking on, the tool-bearing assistant message must OPEN with \
             a thinking block. Got: {blocks:#?}"
        );
        assert_eq!(blocks[0]["thinking"], "Reasoning.");
        assert_eq!(blocks[0]["signature"], "SIGabc123");
        assert!(
            !blocks[0]["signature"].as_str().unwrap_or("").is_empty(),
            "an unsigned thinking block is rejected by Anthropic on replay"
        );
        assert_eq!(blocks[1]["type"], "redacted_thinking");
        assert_eq!(blocks[1]["data"], "REDACTEDdata");
        assert_eq!(
            blocks[2]["type"], "tool_use",
            "the tool_use block must ride in the same message, after the thinking blocks"
        );
        Ok(())
    }

    #[test]
    fn test_message_to_anthropic_spec() {
        let messages = vec![
            Message::user().with_text("Hello"),
            Message::assistant().with_text("Hi there"),
            Message::user().with_text("How are you?"),
        ];

        let spec = format_messages(&messages);

        assert_eq!(spec.len(), 3);
        assert_eq!(spec[0]["role"], "user");
        assert_eq!(spec[0]["content"][0]["type"], "text");
        assert_eq!(spec[0]["content"][0]["text"], "Hello");
        assert_eq!(spec[1]["role"], "assistant");
        assert_eq!(spec[1]["content"][0]["text"], "Hi there");
        assert_eq!(spec[2]["role"], "user");
        assert_eq!(spec[2]["content"][0]["text"], "How are you?");
    }

    #[test]
    fn test_tools_to_anthropic_spec() {
        let tools = vec![
            Tool::new(
                "calculator",
                "Calculate mathematical expressions",
                object!({
                    "type": "object",
                    "properties": {
                        "expression": {
                            "type": "string",
                            "description": "The mathematical expression to evaluate"
                        }
                    }
                }),
            ),
            Tool::new(
                "weather",
                "Get weather information",
                object!({
                    "type": "object",
                    "properties": {
                        "location": {
                            "type": "string",
                            "description": "The location to get weather for"
                        }
                    }
                }),
            ),
        ];

        let spec = format_tools(&tools);

        assert_eq!(spec.len(), 2);
        assert_eq!(spec[0]["name"], "calculator");
        assert_eq!(spec[0]["description"], "Calculate mathematical expressions");
        assert_eq!(spec[1]["name"], "weather");
        assert_eq!(spec[1]["description"], "Get weather information");

        // Verify cache control is added to last tool
        assert!(spec[1].get("cache_control").is_some());
    }

    #[test]
    fn test_system_to_anthropic_spec() {
        let system = "You are a helpful assistant.";
        let spec = format_system(system);

        assert!(spec.is_array());
        let spec_array = spec.as_array().unwrap();
        assert_eq!(spec_array.len(), 1);
        assert_eq!(spec_array[0]["type"], "text");
        assert_eq!(spec_array[0]["text"], system);
        assert!(spec_array[0].get("cache_control").is_some());
    }

    #[test]
    fn test_cache_pricing_calculation() -> Result<()> {
        // Test realistic cache scenario: small fresh input, large cached content
        let response = json!({
            "id": "msg_cache_test",
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "text",
                "text": "Based on the cached context, here's my response."
            }],
            "model": "claude-sonnet-4-20250514",
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": {
                "input_tokens": 7,        // Small fresh input
                "output_tokens": 50,      // Output tokens
                "cache_creation_input_tokens": 10000, // Large cache creation
                "cache_read_input_tokens": 5000       // Large cache read
            }
        });

        let usage = get_usage(&response)?;

        // Cache is kept disjoint from fresh input so each can be priced at its
        // own rate (cache read 0.1x, cache write 1.25x). The four buckets:
        //   fresh input 7, output 50, cache_read 5000, cache_creation 10000.
        assert_eq!(usage.input_tokens, Some(7));
        assert_eq!(usage.output_tokens, Some(50));
        assert_eq!(usage.cache_read_input_tokens, Some(5000));
        assert_eq!(usage.cache_creation_input_tokens, Some(10000));
        // Context occupancy = 7 + 10000 + 5000 + 50 = 15057 (unchanged gauge).
        assert_eq!(usage.total_tokens, Some(15057));
        // Billed total reconciles with the vendor dashboard: same 15057 here.
        assert_eq!(usage.billed_total(), Some(15057));

        Ok(())
    }

    #[test]
    fn get_usage_without_cache_keys_leaves_cache_zero() -> Result<()> {
        // A response that omits the cache_* keys entirely (no prompt caching).
        let response = json!({
            "usage": { "input_tokens": 100, "output_tokens": 40 }
        });
        let usage = get_usage(&response)?;
        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.output_tokens, Some(40));
        assert_eq!(usage.cache_read_input_tokens, Some(0));
        assert_eq!(usage.cache_creation_input_tokens, Some(0));
        assert_eq!(usage.total_tokens, Some(140));
        assert_eq!(usage.billed_total(), Some(140));
        Ok(())
    }

    #[test]
    fn get_usage_reads_top_level_usage_object() -> Result<()> {
        // Streaming message_delta can carry usage fields at the top level.
        let delta = json!({
            "input_tokens": 0,
            "output_tokens": 25,
            "cache_read_input_tokens": 800,
            "cache_creation_input_tokens": 0
        });
        let usage = get_usage(&delta)?;
        assert_eq!(usage.input_tokens, Some(0));
        assert_eq!(usage.output_tokens, Some(25));
        assert_eq!(usage.cache_read_input_tokens, Some(800));
        assert_eq!(usage.total_tokens, Some(825));
        assert_eq!(usage.billed_total(), Some(825));
        Ok(())
    }

    #[tokio::test]
    async fn streaming_usage_preserves_message_start_cache_buckets() -> Result<()> {
        use tokio::pin;
        use tokio_stream::StreamExt;

        let lines = r#"
data: {"type":"message_start","message":{"id":"msg_cache","model":"claude-sonnet-4-20250514","usage":{"input_tokens":7,"output_tokens":0,"cache_creation_input_tokens":10000,"cache_read_input_tokens":5000}}}
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":50}}
data: [DONE]
"#;
        let response_stream = tokio_stream::iter(lines.lines().map(|line| Ok(line.to_string())));
        let messages = response_to_streaming_message_batching(response_stream, true);
        pin!(messages);

        let mut final_usage = None;
        while let Some(result) = messages.next().await {
            let (_, usage, _pending) = result?;
            if usage.is_some() {
                final_usage = usage;
            }
        }

        let usage = final_usage.expect("expected terminal usage");
        assert_eq!(usage.model, "claude-sonnet-4-20250514");
        assert_eq!(usage.usage.input_tokens, Some(7));
        assert_eq!(usage.usage.output_tokens, Some(50));
        assert_eq!(usage.usage.cache_read_input_tokens, Some(5000));
        assert_eq!(usage.usage.cache_creation_input_tokens, Some(10000));
        assert_eq!(usage.usage.total_tokens, Some(15057));
        assert_eq!(usage.usage.billed_total(), Some(15057));
        Ok(())
    }

    #[test]
    fn test_tool_error_handling_maintains_pairing() {
        use crate::conversation::message::Message;
        use rmcp::model::{ErrorCode, ErrorData};

        let messages = vec![
            Message::assistant().with_tool_request(
                "tool_1",
                Ok(CallToolRequestParams {
                    task: None,
                    name: "calculator".into(),
                    arguments: Some(object!({"expression": "2 + 2"})),
                    meta: None,
                }),
            ),
            Message::user().with_tool_response(
                "tool_1",
                Err(ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    "Tool failed".to_string(),
                    None,
                )),
            ),
        ];

        let spec = format_messages(&messages);

        assert_eq!(spec.len(), 2);

        assert_eq!(spec[0]["role"], "assistant");
        assert_eq!(spec[0]["content"][0]["type"], "tool_use");
        assert_eq!(spec[0]["content"][0]["id"], "tool_1");
        assert_eq!(spec[0]["content"][0]["name"], "calculator");

        assert_eq!(spec[1]["role"], "user");
        assert_eq!(spec[1]["content"][0]["type"], "tool_result");
        assert_eq!(spec[1]["content"][0]["tool_use_id"], "tool_1");
        assert_eq!(
            spec[1]["content"][0]["content"],
            "Error: -32603: Tool failed"
        );
        assert_eq!(spec[1]["content"][0]["is_error"], true);
    }

    /// Every audience case, through the real Anthropic formatter.
    ///
    /// Anthropic is the flagship provider and was the largest of the six that
    /// forwarded a tool's user-only blocks to the model. The joined
    /// `tool_result` must carry the three blocks the tool addressed to the
    /// model and neither of the two it did not.
    #[test]
    fn tool_result_blocks_reach_the_model_by_audience() {
        let message = Message::user().with_tool_response(
            "call-1",
            Ok(rmcp::model::CallToolResult {
                content: crate::providers::formats::audience::every_audience_case(),
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        );

        let spec = format_messages(&[message]);
        let sent = spec[0]["content"][0]["content"]
            .as_str()
            .expect("tool_result content is a string");

        assert_eq!(
            sent,
            crate::providers::formats::audience::MODEL_VISIBLE.join("\n"),
            "the Anthropic tool_result must carry exactly the model-addressed blocks"
        );
        for withheld in crate::providers::formats::audience::MODEL_HIDDEN {
            assert!(!sent.contains(withheld), "{withheld} reached the model");
        }
    }

    #[test]
    fn image_tool_results_remain_visual_for_anthropic() {
        let message = Message::user().with_tool_response(
            "call-image",
            Ok(rmcp::model::CallToolResult {
                content: vec![
                    rmcp::model::Content::text("panel capture"),
                    rmcp::model::Content::image("iVBORw0KGgo=", "image/png"),
                ],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        );

        let spec = format_messages(&[message]);
        let blocks = spec[0]["content"][0]["content"]
            .as_array()
            .expect("an image tool_result uses Anthropic content blocks");
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[1]["type"], "image");
        assert_eq!(blocks[1]["source"]["media_type"], "image/png");
    }

    /// A `text_editor view` through the real Anthropic formatter.
    ///
    /// The file reaches the assistant as an embedded resource, so a formatter
    /// that filters by audience while still reading only text blocks sends an
    /// empty `tool_result` and the model cannot see the file it just asked for.
    #[test]
    fn a_viewed_file_reaches_the_model_through_its_embedded_resource() {
        let message = Message::user().with_tool_response(
            "call-1",
            Ok(rmcp::model::CallToolResult {
                content: crate::providers::formats::audience::text_editor_view_result(),
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        );

        let spec = format_messages(&[message]);
        let sent = spec[0]["content"][0]["content"]
            .as_str()
            .expect("tool_result content is a string");

        assert_eq!(sent, crate::providers::formats::audience::VIEW_FOR_MODEL);
        assert!(
            !sent.contains(crate::providers::formats::audience::VIEW_FOR_USER),
            "the user's rendering reached the model"
        );
    }

    #[test]
    fn test_map_stop_reason() {
        assert_eq!(map_stop_reason("max_tokens"), "length");
        assert_eq!(map_stop_reason("end_turn"), "stop");
        assert_eq!(map_stop_reason("stop_sequence"), "stop");
        assert_eq!(map_stop_reason("tool_use"), "tool_calls");
        // Unknown reasons pass through unchanged.
        assert_eq!(map_stop_reason("refusal"), "refusal");
    }

    #[tokio::test]
    async fn test_streaming_maps_max_tokens_to_length() -> Result<()> {
        use tokio::pin;
        use tokio_stream::StreamExt;

        // A native Anthropic stream truncated at the output-length limit ends
        // with a message_delta whose delta.stop_reason == "max_tokens". The
        // emitted ProviderUsage must carry finish_reason == "length" so the
        // agent loop can auto-continue instead of stopping silently mid-sentence.
        let lines = vec![
            r#"data: {"type":"message_start","message":{"id":"msg_1","model":"claude-sonnet-4-20250514","usage":{"input_tokens":10,"output_tokens":1}}}"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Counting: 1, 2, 3"}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"max_tokens","stop_sequence":null},"usage":{"output_tokens":8}}"#,
            r#"data: {"type":"message_stop"}"#,
        ];
        let response_stream = tokio_stream::iter(lines.into_iter().map(|l| Ok(l.to_string())));
        let messages = response_to_streaming_message_batching(response_stream, true);
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
            "expected ProviderUsage.finish_reason == Some(\"length\") on a max_tokens stream"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_streaming_natural_stop_is_not_length() -> Result<()> {
        use tokio::pin;
        use tokio_stream::StreamExt;

        // A natural completion reports stop_reason == "end_turn"; it must NOT be
        // reported as a length-truncation, or the agent loop would auto-continue
        // a turn that finished on its own.
        let lines = vec![
            r#"data: {"type":"message_start","message":{"id":"msg_2","model":"claude-sonnet-4-20250514","usage":{"input_tokens":5,"output_tokens":1}}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":1}}"#,
            r#"data: {"type":"message_stop"}"#,
        ];
        let response_stream = tokio_stream::iter(lines.into_iter().map(|l| Ok(l.to_string())));
        let messages = response_to_streaming_message_batching(response_stream, true);
        pin!(messages);

        let mut saw_stop = false;
        while let Some(Ok((_message, usage, _pending))) = messages.next().await {
            if let Some(u) = usage {
                assert_ne!(
                    u.finish_reason.as_deref(),
                    Some("length"),
                    "natural stop must not be reported as length-truncation"
                );
                if u.finish_reason.as_deref() == Some("stop") {
                    saw_stop = true;
                }
            }
        }
        assert!(
            saw_stop,
            "expected finish_reason == Some(\"stop\") on end_turn"
        );
        Ok(())
    }
}
