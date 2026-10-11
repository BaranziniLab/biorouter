//! Ollama's native chat wire format, `POST /api/chat`.
//!
//! ## Why not the OpenAI-compatible endpoint
//!
//! The provider used `/v1/chat/completions` until 2026-10. That endpoint has no
//! way to set the context window: Ollama builds its options from a fixed list
//! (`stop`, `max_tokens`, `temperature`, `seed`, the two penalties, `top_p`)
//! and silently ignores everything else, `options.num_ctx` included. On a
//! machine where Ollama's default window is 4096 tokens (every CPU-only or
//! low-VRAM machine, and every Intel Mac) it then cut BioRouter's ~31-40k-token
//! prompt to the first 4 tokens plus the last 2046, with an HTTP 200 and one
//! WARN line in the server log. The model saw only the tail of the
//! alphabetically sorted tool list and wrote invented tool calls as JSON text,
//! which is the bug report this module exists to fix. Measured on Ollama
//! 0.40.2: `/v1` reported `prompt_tokens` 2050 with and without
//! `options.num_ctx`; `/api/chat` with `options.num_ctx` processed all 15,950.
//!
//! The native endpoint takes `options.num_ctx`, `think`, `keep_alive` and
//! `truncate`. With `truncate: false` an over-window prompt is an HTTP 400
//! (`exceed_context_size_error`) instead of a silent cut, which
//! [`map_error`] turns into [`ProviderError::ContextLengthExceeded`] so the
//! agent's overflow recovery runs.
//!
//! ## Shapes, as measured on Ollama 0.40.2
//!
//! - A tool call arrives as ONE complete chunk, never as argument fragments:
//!   `message.tool_calls: [{id, function: {index, name, arguments: {..}}}]`,
//!   with `arguments` a JSON object (not a string, as on `/v1`).
//! - Thinking streams in `message.thinking`; the answer in `message.content`.
//! - The last line is `{done: true, done_reason, prompt_eval_count,
//!   prompt_eval_cached_count, eval_count, ..}`. `prompt_eval_count` is the
//!   WHOLE prompt, cached tokens included (202 with 201 cached on a repeat), so
//!   fresh input is the difference. `done_reason` is `stop` even when the turn
//!   ended in tool calls, and `length` at `num_predict`.
//! - An error before the first byte is an HTTP status with `{"error": "<s>"}`;
//!   after streaming began it is a final NDJSON line `{"error": "<s>"}` on a
//!   200. The string can itself be JSON — llama-server's error object, which
//!   Ollama forwards as text.

use crate::agents::extension_manager::MOIM_OPEN_TAG;
use crate::conversation::message::{Message, MessageContent};
use crate::providers::base::{PendingToolCall, ProviderStreamItem, ProviderUsage, Usage};
use crate::providers::errors::ProviderError;
use crate::providers::formats::audience;
// Thinking is re-emitted in batches rather than per token: every streamed item
// is pushed onto the turn's message, and one `Thinking` block per token would
// leave hundreds of blocks on one row. The cadence is imported from the
// OpenAI-compatible decoder rather than restated, so the two local providers
// (Ollama here, Llama Server there) can never hand thinking on at different
// rates; see that constant for what a person actually sees.
use crate::providers::formats::openai::{
    format_tools, validate_tool_schemas, THINKING_FLUSH_CHARS, THINKING_FLUSH_INTERVAL,
};
use crate::providers::utils::{
    detect_image_path, is_valid_function_name, load_image_file, safely_parse_json,
    sanitize_function_name,
};
use anyhow::anyhow;
use async_stream::try_stream;
use futures::Stream;
use reqwest::StatusCode;
use rmcp::model::{
    object, CallToolRequestParams, ErrorCode, ErrorData, RawContent, ResourceContents, Role, Tool,
};
use serde_json::{json, Map, Value};
use std::borrow::Cow;
use std::collections::HashMap;
use std::time::Instant;

/// The native chat route, relative to the configured host.
pub const OLLAMA_CHAT_PATH: &str = "api/chat";

/// What to send as `think`. Ollama takes a bool, or one of the level strings a
/// model declares in `/api/show`'s `thinking.values`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Think {
    Off,
    On,
    Level(String),
}

impl Think {
    pub fn to_json(&self) -> Value {
        match self {
            Think::Off => json!(false),
            Think::On => json!(true),
            Think::Level(level) => json!(level),
        }
    }

    /// Read a `think` value as Ollama spells it (`false`, `true`, `"low"`, …),
    /// also accepting the strings a config file or an environment variable
    /// produces for the two booleans.
    pub fn from_json(value: &Value) -> Option<Self> {
        match value {
            Value::Bool(false) => Some(Think::Off),
            Value::Bool(true) => Some(Think::On),
            Value::String(text) => {
                let text = text.trim();
                match text.to_ascii_lowercase().as_str() {
                    "" => None,
                    "false" | "off" | "no" | "none" => Some(Think::Off),
                    "true" | "on" | "yes" => Some(Think::On),
                    _ => Some(Think::Level(text.to_string())),
                }
            }
            _ => None,
        }
    }

    /// Rank of a level among the ones Ollama's renderers know, so the deepest
    /// level a model declares can be picked without a per-model table.
    fn rank(&self) -> u8 {
        match self {
            Think::Off => 0,
            Think::On => 1,
            Think::Level(level) => match level.as_str() {
                "minimal" => 1,
                "low" => 2,
                "medium" => 3,
                "high" => 4,
                "xhigh" => 5,
                "max" => 6,
                _ => 1,
            },
        }
    }
}

/// What `POST /api/show` says about one model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelFacts {
    /// `capabilities`, e.g. `["completion", "vision", "tools", "thinking"]`.
    /// Empty when the server did not say (Ollama before ~0.6).
    pub capabilities: Vec<String>,
    /// `model_info[<general.architecture>.context_length]`: the window the model
    /// was trained for. The key is NOT fixed — the MLX build of qwen3.8 reports
    /// `qwen3_5.context_length` and the GGUF build `qwen35.context_length` — so
    /// it is read through `general.architecture` rather than hard-coded.
    pub trained_context: Option<usize>,
    /// `thinking.values`: the `think` values this model accepts. qwen3.8 is
    /// `[false, "low", "medium", "xhigh"]` (its template RAISES on `high`);
    /// gemma4 and qwen3 are `[false, true]`; gpt-oss is `["low", "medium",
    /// "high"]`, with no `false`. Empty when the server did not say — which is
    /// also what deepseek-r1:8b reports, a model that thinks whatever `think`
    /// says (Ollama 0.40.2).
    pub think_values: Vec<Think>,
    /// The model's chat template shows the tool list only next to the LAST
    /// user message (`{{- if and $.Tools $last }}`, llama3.1/3.2) or only when
    /// a user message is among the last two (`le (len (slice $.Messages
    /// $index)) 2`, mistral-small3.2 and mistral-nemo). After the first tool
    /// result such a model no longer sees any tool definition: measured,
    /// llama3.1 then answered from memory with a fabricated result and
    /// mistral-small invented tool names its parser swallowed (R8). Read from
    /// the template's literal text, so it never fires for a template that
    /// merely resembles these.
    pub tools_only_near_last_user: bool,
    /// The template renders an assistant turn's tool calls only when the turn
    /// has no text (`{{- if .Content }} .. {{- else if .ToolCalls }}`:
    /// mistral-small3.2, mistral-nemo, qwen2.5-coder, command-r7b, hermes3).
    /// Sent a turn with both, it renders the text and DROPS the calls, so the
    /// model sees tool results for calls it never made (R7).
    pub drops_calls_after_text: bool,
    /// `remote_host` / `remote_model`: Ollama serves this model by forwarding
    /// to another host (an Ollama cloud model, possibly renamed with `ollama
    /// cp`), so a request for it leaves this machine.
    pub remote: bool,
    /// The origin of `remote_host` (`https://ollama.com` for
    /// `https://ollama.com:443`), when it names an http(s) URL: where a
    /// request for this model really goes.
    pub remote_origin: Option<String>,
}

impl ModelFacts {
    pub fn from_show(show: &Value) -> Self {
        let capabilities = show
            .get("capabilities")
            .and_then(Value::as_array)
            .map(|caps| {
                caps.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let model_info = show.get("model_info");
        let trained_context = model_info
            .and_then(|info| info.get("general.architecture"))
            .and_then(Value::as_str)
            .and_then(|arch| {
                model_info?
                    .get(format!("{arch}.context_length"))
                    .and_then(Value::as_u64)
            })
            .and_then(|n| usize::try_from(n).ok())
            .filter(|&n| n > 0);
        let think_values = show
            .pointer("/thinking/values")
            .and_then(Value::as_array)
            .map(|values| values.iter().filter_map(Think::from_json).collect())
            .unwrap_or_default();
        let template = show
            .get("template")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let tools_only_near_last_user = [
            "if and $.Tools $last",
            "(le (len (slice $.Messages $index)) 2) $.Tools",
            "$.Tools (le (len (slice $.Messages $i)) 2)",
        ]
        .iter()
        .any(|pattern| template.contains(pattern));
        let drops_calls_after_text = template.contains("else if .ToolCalls");
        let remote = ["remote_host", "remote_model"].iter().any(|key| {
            show.get(key)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
        });
        let remote_origin = show
            .get("remote_host")
            .and_then(Value::as_str)
            .and_then(|host| crate::providers::base::computer_use_destination_origin(host.trim()));
        Self {
            capabilities,
            trained_context,
            think_values,
            tools_only_near_last_user,
            drops_calls_after_text,
            remote,
            remote_origin,
        }
    }

    pub fn has(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }

    /// Whether `think` may be sent at all. Sending it to a model without the
    /// `thinking` capability is an HTTP 400 (`"<model>" does not support
    /// thinking`), so an unknown capability list counts as no.
    pub fn supports_thinking(&self) -> bool {
        self.has("thinking")
    }

    pub fn supports_vision(&self) -> bool {
        self.has("vision")
    }

    /// Whether the model may be offered for tool-calling chats. `None` when the
    /// server reported no capabilities, so the caller can keep it listed.
    pub fn supports_tools(&self) -> Option<bool> {
        (!self.capabilities.is_empty()).then(|| self.has("tools"))
    }

    fn levels_known(&self) -> bool {
        !self.think_values.is_empty()
    }

    /// Whether the model may think even when `think: false` is sent, so its
    /// output budget must leave room for reasoning (R6): a thinking model that
    /// declares no `thinking.values` (deepseek-r1:8b returned 239-3219
    /// characters of thinking with `think: false`), or one whose values do not
    /// include `false` (gpt-oss, whose `think: false` still returned 46-357).
    pub fn thinks_regardless(&self) -> bool {
        self.supports_thinking()
            && (!self.levels_known() || !self.think_values.contains(&Think::Off))
    }

    fn is_boolean_thinker(&self) -> bool {
        self.think_values
            .iter()
            .all(|value| matches!(value, Think::Off | Think::On))
    }

    /// The deepest `think` value this model declares: `xhigh` for qwen3.8,
    /// `true` for a model that only takes booleans.
    pub fn deepest_think(&self) -> Think {
        self.think_values
            .iter()
            .filter(|value| **value != Think::Off)
            .max_by_key(|value| value.rank())
            .cloned()
            .unwrap_or(Think::On)
    }

    /// `requested`, made acceptable to this model, or `None` if it cannot be.
    ///
    /// `Off` is accepted by every thinking model. A level is accepted only if
    /// the model declares it — qwen3.8's renderer raises on anything outside
    /// `low|medium|xhigh`, so `high` must never be forwarded — except that a
    /// boolean-only model takes any level as `true` (gemma4 collapses every
    /// truthy value to the same `<|think|>`). `On` sent to a level model is
    /// that model's own default.
    ///
    /// `Off` sent to a model that declares levels but not `false` (gpt-oss) is
    /// its LOWEST level: Ollama ignores `false` for it, and `low` is the least
    /// reasoning it can be asked for.
    pub fn accept(&self, requested: &Think) -> Option<Think> {
        if !self.supports_thinking() {
            return None;
        }
        match requested {
            Think::Off if self.levels_known() && !self.think_values.contains(&Think::Off) => self
                .think_values
                .iter()
                .filter(|value| matches!(value, Think::Level(_)))
                .min_by_key(|value| value.rank())
                .cloned()
                .or(Some(Think::Off)),
            Think::Off => Some(Think::Off),
            Think::On => Some(Think::On),
            Think::Level(_) if !self.levels_known() || self.is_boolean_thinker() => Some(Think::On),
            Think::Level(_) => self
                .think_values
                .iter()
                .any(|value| value == requested)
                .then(|| requested.clone()),
        }
    }
}

/// Everything one `/api/chat` request needs beyond the conversation itself.
pub struct ChatRequest<'a> {
    pub model: &'a str,
    pub system: &'a str,
    pub messages: &'a [Message],
    pub tools: &'a [Tool],
    pub stream: bool,
    /// Sent on EVERY request for a model, and always the same number: on the
    /// llama.cpp runner a different `num_ctx` reloads the model (measured).
    pub num_ctx: usize,
    /// Only when the model config pins one. Omitting it lets the Modelfile's
    /// recommended sampling apply (qwen3.8: top_k 20, top_p 0.95; gemma4: top_k
    /// 64, top_p 0.95) — `/v1` forced 1.0/1.0 on every request instead.
    pub temperature: Option<f32>,
    pub num_predict: Option<i32>,
    pub think: Option<Think>,
    pub keep_alive: Option<Value>,
    /// Whether the model reads images. Without it an image becomes a sentence
    /// saying one was there, rather than an `images` array the model ignores.
    pub vision: bool,
    /// Send tool-call turns with no text ([`ModelFacts::drops_calls_after_text`]).
    pub calls_without_text: bool,
}

/// Build the `/api/chat` body.
pub fn create_request(request: &ChatRequest<'_>) -> anyhow::Result<Value> {
    let mut messages = vec![json!({"role": "system", "content": request.system})];
    messages.extend(format_messages(
        request.messages,
        request.vision,
        request.calls_without_text,
    ));
    // A conversation with no user turn at all: Qwen's chat template raises "No
    // user query found in messages", and no model answers well without one.
    if !messages
        .iter()
        .any(|message| message.get("role").and_then(Value::as_str) == Some("user"))
    {
        messages.push(json!({"role": "user", "content": "Continue."}));
    }

    let mut options = Map::new();
    options.insert("num_ctx".to_string(), json!(request.num_ctx));
    if let Some(temperature) = request.temperature {
        options.insert("temperature".to_string(), json!(temperature));
    }
    if let Some(num_predict) = request.num_predict {
        options.insert("num_predict".to_string(), json!(num_predict));
    }

    let mut payload = json!({
        "model": request.model,
        "messages": messages,
        "stream": request.stream,
        "options": Value::Object(options),
        // Refuse instead of cutting. Ollama before 0.12.6 ignores the field,
        // which is why the provider also checks the reported prompt size.
        "truncate": false,
    });
    let mut tools = format_tools(request.tools)?;
    validate_tool_schemas(&mut tools);
    if !tools.is_empty() {
        payload["tools"] = json!(tools);
    }
    if let Some(think) = &request.think {
        payload["think"] = think.to_json();
    }
    if let Some(keep_alive) = &request.keep_alive {
        payload["keep_alive"] = keep_alive.clone();
    }
    Ok(payload)
}

/// Index of the last message that is a real user query: a user turn that is not
/// a tool result and carries something other than BioRouter's own `<info-msg>`
/// block. Thinking is replayed only for assistant turns after it.
fn last_user_query(messages: &[&Message]) -> Option<usize> {
    messages.iter().rposition(|message| {
        message.role == Role::User
            && !message.is_tool_response()
            && message.content.iter().any(|content| match content {
                MessageContent::Text(text) => {
                    let text = text.text.trim_start();
                    !text.is_empty() && !text.starts_with(MOIM_OPEN_TAG)
                }
                MessageContent::Image(_) => true,
                _ => false,
            })
    })
}

fn image_note() -> String {
    "[An image was attached here, but this model cannot read images.]".to_string()
}

/// Convert BioRouter messages to `/api/chat` messages (without the system one).
///
/// - Tool results become `{role: "tool", tool_name, tool_call_id, content}`, in
///   the order the calls were made, and BEFORE any text the same user turn
///   carries (BioRouter merges its `<info-msg>` block into the tool-result
///   turn; putting it first would separate a call from its result).
/// - EVERY `role: "tool"` message names its tool (R4): native `/api/chat` does
///   not derive the name from the id, and gpt-oss's template rendered a
///   nameless result as `functions. to=assistant`. A result whose call is not
///   in the history (compacted away, or never recorded) has no name to give,
///   so it is sent as user text instead of as a nameless tool message.
/// - Results follow the order of their calls (R5): the mistral, granite,
///   llama3.1 and ministral templates drop the id and pair results with calls
///   by POSITION, so a result sent out of order is read as another call's.
/// - `calls_without_text` sends a tool-call turn with no text, for templates
///   that otherwise render the text and drop the calls (R7).
/// - Assistant tool calls carry `arguments` as a JSON object.
/// - Consecutive assistant rows are merged into one message. BioRouter persists
///   one turn as a thinking row followed by one row per tool call; sending them
///   as separate assistant turns would render as several turns the model never
///   produced.
/// - Thinking is replayed in `thinking` only for assistant turns after the last
///   real user query — the current tool loop. That is what Qwen's own chat
///   template keeps; Ollama's qwen3.8 renderer renders whatever it is given
///   (verified with `_debug_render_only`), and its gemma4 renderer drops it.
#[allow(clippy::too_many_lines)]
pub fn format_messages(messages: &[Message], vision: bool, calls_without_text: bool) -> Vec<Value> {
    let visible: Vec<&Message> = messages.iter().filter(|m| m.is_agent_visible()).collect();
    let replay_thinking_after = last_user_query(&visible);
    let mut tool_names: HashMap<String, String> = HashMap::new();
    let mut out: Vec<Value> = Vec::new();

    for (index, message) in visible.iter().enumerate() {
        match message.role {
            Role::Assistant => {
                let mut texts: Vec<String> = Vec::new();
                let mut thinking: Vec<String> = Vec::new();
                let mut tool_calls: Vec<Value> = Vec::new();
                let replay = replay_thinking_after.is_none_or(|boundary| index > boundary);
                for content in &message.content {
                    match content {
                        MessageContent::Text(text) if !text.text.is_empty() => {
                            texts.push(text.text.clone());
                        }
                        MessageContent::Thinking(block) if replay && !block.thinking.is_empty() => {
                            thinking.push(block.thinking.clone());
                        }
                        MessageContent::ToolRequest(request) => {
                            if let Ok(call) = &request.tool_call {
                                let name = sanitize_function_name(&call.name);
                                tool_names.insert(request.id.clone(), name.clone());
                                tool_calls.push(tool_call_json(&request.id, &name, call));
                            }
                        }
                        MessageContent::FrontendToolRequest(request) => {
                            if let Ok(call) = &request.tool_call {
                                let name = sanitize_function_name(&call.name);
                                tool_names.insert(request.id.clone(), name.clone());
                                tool_calls.push(tool_call_json(&request.id, &name, call));
                            }
                        }
                        _ => {}
                    }
                }
                if texts.is_empty() && thinking.is_empty() && tool_calls.is_empty() {
                    continue;
                }
                let previous_is_assistant = out
                    .last()
                    .and_then(|m| m.get("role"))
                    .and_then(Value::as_str)
                    == Some("assistant");
                if previous_is_assistant {
                    let previous = out.last_mut().expect("checked above");
                    append_text(previous, "content", &texts.join("\n"));
                    append_text(previous, "thinking", &thinking.join(""));
                    if !tool_calls.is_empty() {
                        let calls = previous
                            .as_object_mut()
                            .expect("built as an object")
                            .entry("tool_calls")
                            .or_insert_with(|| json!([]));
                        if let Some(calls) = calls.as_array_mut() {
                            calls.extend(tool_calls);
                        }
                    }
                } else {
                    let mut converted = json!({"role": "assistant", "content": texts.join("\n")});
                    if !thinking.is_empty() {
                        converted["thinking"] = json!(thinking.join(""));
                    }
                    if !tool_calls.is_empty() {
                        converted["tool_calls"] = json!(tool_calls);
                    }
                    out.push(converted);
                }
            }
            Role::User => {
                let mut tool_messages: Vec<Value> = Vec::new();
                let mut followups: Vec<Value> = Vec::new();
                let mut texts: Vec<String> = Vec::new();
                let mut images: Vec<String> = Vec::new();
                for content in &message.content {
                    match content {
                        MessageContent::Text(text) if !text.text.is_empty() => {
                            texts.push(text.text.clone());
                            if vision {
                                if let Some(path) = detect_image_path(&text.text) {
                                    if let Ok(image) = load_image_file(path) {
                                        images.push(image.data.clone());
                                    }
                                }
                            }
                        }
                        MessageContent::Image(image) => {
                            if vision {
                                images.push(image.data.clone());
                            } else {
                                texts.push(image_note());
                            }
                        }
                        MessageContent::ToolResponse(response) => {
                            let (text, tool_images) = tool_result_text(response, vision);
                            match tool_names.get(&response.id) {
                                Some(name) => tool_messages.push(json!({
                                    "role": "tool",
                                    "tool_name": name,
                                    "tool_call_id": response.id,
                                    "content": text,
                                })),
                                None => texts
                                    .push(format!("The result of an earlier tool call:\n{text}")),
                            }
                            if !tool_images.is_empty() {
                                followups.push(json!({
                                    "role": "user",
                                    "content": "The images the tool result above refers to.",
                                    "images": tool_images,
                                }));
                            }
                        }
                        _ => {}
                    }
                }
                out.extend(tool_messages);
                out.extend(followups);
                if !texts.is_empty() || !images.is_empty() {
                    let mut converted = json!({"role": "user", "content": texts.join("\n")});
                    if !images.is_empty() {
                        converted["images"] = json!(images);
                    }
                    out.push(converted);
                }
            }
        }
    }
    order_tool_results_by_call(&mut out);
    if calls_without_text {
        for message in &mut out {
            let has_calls = message
                .get("tool_calls")
                .and_then(Value::as_array)
                .is_some_and(|calls| !calls.is_empty());
            if has_calls && message.get("role").and_then(Value::as_str) == Some("assistant") {
                message["content"] = json!("");
            }
        }
    }
    out
}

/// Stable-sort every run of consecutive `role: "tool"` messages into the order
/// of the calls in the assistant message before the run (R5). BioRouter stores
/// one result per row, and parallel calls can finish in any order.
fn order_tool_results_by_call(out: &mut [Value]) {
    let mut call_order: HashMap<String, usize> = HashMap::new();
    let mut index = 0;
    while index < out.len() {
        let role = out[index]
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if role == "assistant" {
            call_order = out[index]
                .get("tool_calls")
                .and_then(Value::as_array)
                .map(|calls| {
                    calls
                        .iter()
                        .enumerate()
                        .filter_map(|(position, call)| {
                            call.get("id")
                                .and_then(Value::as_str)
                                .map(|id| (id.to_string(), position))
                        })
                        .collect()
                })
                .unwrap_or_default();
            index += 1;
            continue;
        }
        if role != "tool" {
            index += 1;
            continue;
        }
        let start = index;
        while index < out.len() && out[index].get("role").and_then(Value::as_str) == Some("tool") {
            index += 1;
        }
        out[start..index].sort_by_key(|message| {
            message
                .get("tool_call_id")
                .and_then(Value::as_str)
                .and_then(|id| call_order.get(id).copied())
                .unwrap_or(usize::MAX)
        });
    }
}

fn append_text(message: &mut Value, key: &str, addition: &str) {
    if addition.is_empty() {
        return;
    }
    let existing = message.get(key).and_then(Value::as_str).unwrap_or_default();
    let joined = if existing.is_empty() {
        addition.to_string()
    } else if key == "thinking" {
        format!("{existing}{addition}")
    } else {
        format!("{existing}\n{addition}")
    };
    message[key] = json!(joined);
}

fn tool_call_json(id: &str, name: &str, call: &CallToolRequestParams) -> Value {
    let arguments = call
        .arguments
        .as_ref()
        .map(|args| Value::Object(args.clone()))
        .unwrap_or_else(|| json!({}));
    json!({
        "id": id,
        "function": {"name": name, "arguments": arguments},
    })
}

/// A tool result as one string, plus the base64 images it carried (only when
/// the model reads images). Only blocks addressed to the model are sent; see
/// `formats::audience`.
fn tool_result_text(
    response: &crate::conversation::message::ToolResponse,
    vision: bool,
) -> (String, Vec<String>) {
    match &response.tool_result {
        Ok(result) => {
            let mut parts = Vec::new();
            let mut images = Vec::new();
            for content in result.content.iter().filter(|c| audience::is_for_model(c)) {
                match &content.raw {
                    RawContent::Image(image) => {
                        if vision {
                            parts.push(
                                "This tool result included an image, sent in the next message."
                                    .to_string(),
                            );
                            images.push(image.data.clone());
                        } else {
                            parts.push(
                                "This tool result included an image, which this model cannot read."
                                    .to_string(),
                            );
                        }
                    }
                    RawContent::Resource(resource) => {
                        if let ResourceContents::TextResourceContents { text, .. } =
                            &resource.resource
                        {
                            parts.push(text.clone());
                        }
                    }
                    _ => {
                        if let Some(text) = audience::flattened_text(content) {
                            parts.push(text);
                        }
                    }
                }
            }
            (parts.join("\n"), images)
        }
        Err(error) => (
            format!("The tool call returned the following error:\n{error}"),
            Vec::new(),
        ),
    }
}

/// Bytes of TEXT in a request body: every string except image payloads (the
/// native `images` arrays and `/v1` `data:` URLs). Used to tell whether the
/// prompt size a server reports can possibly be the whole request — a base64
/// image is many bytes and few tokens, so it must not count.
pub fn text_bytes(payload: &Value) -> usize {
    fn walk(value: &Value) -> usize {
        match value {
            Value::String(text) if text.starts_with("data:") => 0,
            Value::String(text) => text.len(),
            Value::Array(items) => items.iter().map(walk).sum(),
            Value::Object(map) => map
                .iter()
                .map(|(key, value)| {
                    if key == "images" {
                        0
                    } else {
                        key.len() + walk(value)
                    }
                })
                .sum(),
            _ => 4,
        }
    }
    walk(payload)
}

/// Usage from a final (`done: true`) chunk or a non-streamed response.
///
/// `prompt_eval_count` includes the cached prompt tokens, so — to keep
/// [`Usage`]'s buckets disjoint — fresh input is the difference and the cached
/// count goes to `cache_read_input_tokens`. `total_tokens` is the whole prompt
/// plus the output: the window occupancy the context gauge and compaction read.
pub fn get_usage(chunk: &Value) -> Option<Usage> {
    let count = |key: &str| {
        chunk
            .get(key)
            .and_then(Value::as_i64)
            .and_then(|n| i32::try_from(n).ok())
    };
    let prompt = count("prompt_eval_count");
    let output = count("eval_count");
    if prompt.is_none() && output.is_none() {
        return None;
    }
    let cached = count("prompt_eval_cached_count")
        .filter(|&cached| cached > 0)
        .map(|cached| cached.min(prompt.unwrap_or(cached)));
    let input = match (prompt, cached) {
        (Some(prompt), Some(cached)) => Some(prompt - cached),
        (prompt, _) => prompt,
    };
    let total = match (prompt, output) {
        (Some(prompt), Some(output)) => Some(prompt + output),
        (prompt, output) => prompt.or(output),
    };
    Some(Usage::new(input, output, total).with_cache(cached, None))
}

/// Routes inline think tags in `content` to thinking as the deltas arrive —
/// servers and models without a thinking parser leave the reasoning inline —
/// holding back only as much text as could be the start of a tag split across
/// two deltas (R13):
///
/// - a leading `<think>` block is thinking; an unterminated one is all
///   reasoning (the reply hit its limit);
/// - a leading `>` (or any tail of `<think>`) on a line of its own is the rest
///   of a think tag the template already opened — deepseek's `>\n\n` — and is
///   dropped. A markdown quote (`> text`) has text after the `>` and is kept;
/// - only in a reply that OPENED that way — with a think block or a tag's
///   tail — a `<think>` later in the text opens another block and a stray
///   `</think>` is dropped. deepseek-r1:8b's follow-up came back as `>\n\nThe
///   files in /tmp are alpha.txt and beta.csv.\n</think><think>\nOkay, the
///   user ...` — the answer, then a closing tag, then 300 tokens of reasoning.
///   Every capture with a tag mid-text (seven, all deepseek-r1) opened with
///   the `>` tail. Any other reply's tags are literal text: an answer about
///   `re.sub(r"<think>.*?</think>", "", s)` used to lose `.*?` into a hidden
///   thinking block and show `re.sub(r"", "", s)` — and that corrupted text
///   was what got stored and replayed to the model. Keying it on the model's
///   `thinking` capability instead would not have helped: qwen3.8:27b, the
///   default, has it and leaves no tags in its content.
#[derive(Default)]
struct InlineThinkSplitter {
    state: SplitterState,
    pending: String,
    /// Drop whitespace before the first answer character after a think block.
    skip_leading_ws: bool,
    /// The reply opened with a think block or a tag's tail, so tags later in
    /// the text are reasoning markup too; otherwise they are the answer's.
    tags_are_markup: bool,
}

#[derive(Default, PartialEq)]
enum SplitterState {
    #[default]
    Start,
    InThink,
    Passthrough,
}

const OPEN_THINK: &str = "<think>";
const CLOSE_THINK: &str = "</think>";

/// Tails of `<think>`, longest first: what is left of a think tag whose start
/// was part of the rendered prompt.
const THINK_TAG_TAILS: &[&str] = &["think>", "hink>", "ink>", "nk>", "k>", ">"];

enum TagTail<'a> {
    /// Not a tag tail: the text is an answer.
    No,
    /// Could be one; the next character decides.
    Undecided,
    /// A tail on a line of its own: this is the text after it.
    Strip(&'a str),
}

fn leading_tag_tail(text: &str) -> TagTail<'_> {
    for tail in THINK_TAG_TAILS {
        if let Some(after) = text.strip_prefix(tail) {
            if after.is_empty() {
                return TagTail::Undecided;
            }
            if after.starts_with('\n') || after.starts_with("\r\n") {
                return TagTail::Strip(after);
            }
            return TagTail::No;
        }
    }
    TagTail::No
}

/// The length of the longest tail of `text` that could be the start of one of
/// `tags`. The tags are ASCII, so it is always a character boundary.
fn tag_prefix_tail(text: &str, tags: &[&str]) -> usize {
    tags.iter()
        .flat_map(|tag| (1..tag.len()).filter_map(move |n| tag.get(..n)))
        .filter(|prefix| text.ends_with(prefix))
        .map(str::len)
        .max()
        .unwrap_or(0)
}

impl InlineThinkSplitter {
    /// Returns `(thinking, text)` ready to emit.
    fn push(&mut self, delta: &str) -> (String, String) {
        self.pending.push_str(delta);
        let mut thinking = String::new();
        let mut text = String::new();
        loop {
            match self.state {
                SplitterState::Start => {
                    let trimmed = self.pending.trim_start();
                    if let Some(rest) = trimmed.strip_prefix(OPEN_THINK) {
                        self.pending = rest.to_string();
                        self.state = SplitterState::InThink;
                        self.tags_are_markup = true;
                        continue;
                    }
                    if OPEN_THINK.starts_with(trimmed) {
                        // Could still become `<think>` (or is only whitespace).
                        return (thinking, text);
                    }
                    match leading_tag_tail(trimmed) {
                        TagTail::Undecided => return (thinking, text),
                        TagTail::Strip(after) => {
                            self.pending = after.to_string();
                            self.skip_leading_ws = true;
                            self.tags_are_markup = true;
                            self.state = SplitterState::Passthrough;
                        }
                        TagTail::No => self.state = SplitterState::Passthrough,
                    }
                }
                SplitterState::InThink => {
                    if let Some((reasoning, answer)) = self.pending.split_once(CLOSE_THINK) {
                        thinking.push_str(reasoning);
                        self.pending = answer.to_string();
                        self.skip_leading_ws = true;
                        self.state = SplitterState::Passthrough;
                        continue;
                    }
                    let keep = tag_prefix_tail(&self.pending, &[CLOSE_THINK]);
                    let split = self.pending.len() - keep;
                    thinking.extend(self.pending.drain(..split));
                    return (thinking, text);
                }
                SplitterState::Passthrough => {
                    if self.skip_leading_ws {
                        let trimmed = self.pending.trim_start();
                        if trimmed.is_empty() {
                            self.pending.clear();
                            return (thinking, text);
                        }
                        self.pending = trimmed.to_string();
                        self.skip_leading_ws = false;
                    }
                    if !self.tags_are_markup {
                        // Literal text, tags and all; nothing to hold back.
                        text.push_str(&self.pending);
                        self.pending.clear();
                        return (thinking, text);
                    }
                    let open = self.pending.find(OPEN_THINK).map(|at| (at, OPEN_THINK));
                    let close = self.pending.find(CLOSE_THINK).map(|at| (at, CLOSE_THINK));
                    let first = match (open, close) {
                        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
                        (a, b) => a.or(b),
                    };
                    match first {
                        Some((at, tag)) => {
                            let (before, from_tag) = self.pending.split_at(at);
                            text.push_str(before);
                            self.pending =
                                from_tag.strip_prefix(tag).unwrap_or(from_tag).to_string();
                            if tag == OPEN_THINK {
                                self.state = SplitterState::InThink;
                            }
                            // A stray `</think>` is simply dropped.
                        }
                        None => {
                            let keep = tag_prefix_tail(&self.pending, &[OPEN_THINK, CLOSE_THINK]);
                            let split = self.pending.len() - keep;
                            text.extend(self.pending.drain(..split));
                            return (thinking, text);
                        }
                    }
                }
            }
        }
    }

    fn finish(&mut self) -> (String, String) {
        let pending = std::mem::take(&mut self.pending);
        match self.state {
            SplitterState::InThink => (pending, String::new()),
            _ if self.skip_leading_ws => (String::new(), pending.trim_start().to_string()),
            _ => (String::new(), pending),
        }
    }
}

fn assistant_message(id: &str, content: Vec<MessageContent>) -> Message {
    Message::new(Role::Assistant, chrono::Utc::now().timestamp(), content).with_id(id)
}

/// Parse one tool call from `message.tool_calls`. Arguments are an object on
/// the native endpoint; a string is accepted too, for servers that mimic it.
/// Ollama before ~0.12 sent no `id`, so one is generated.
fn parse_tool_call(call: &Value) -> MessageContent {
    let id = call
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("call_{}", uuid::Uuid::new_v4().simple()));
    let function = call.get("function").cloned().unwrap_or(Value::Null);
    let name = function
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if !is_valid_function_name(&name) {
        return MessageContent::tool_request(
            id,
            Err(ErrorData::new(
                ErrorCode::INVALID_REQUEST,
                format!(
                    "The provided function name '{name}' had invalid characters, it must match this regex [a-zA-Z0-9_-]+"
                ),
                None,
            )),
        );
    }
    let arguments = match function.get("arguments") {
        None | Some(Value::Null) => Ok(json!({})),
        Some(Value::Object(map)) => Ok(Value::Object(map.clone())),
        Some(Value::String(text)) if text.trim().is_empty() => Ok(json!({})),
        Some(Value::String(text)) => safely_parse_json(text).map_err(|error| {
            ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("Could not interpret tool use parameters for id {id}: {error}"),
                Some(json!({"biorouterToolCallFailure": "invalid_arguments"})),
            )
        }),
        Some(_) => Err(ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            "Tool arguments must be a JSON object; the call was not executed.",
            Some(json!({"biorouterToolCallFailure": "invalid_arguments"})),
        )),
    }
    .and_then(|value| {
        if value.is_object() {
            Ok(value)
        } else {
            Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                "Tool arguments must be a JSON object; the call was not executed.",
                Some(json!({"biorouterToolCallFailure": "invalid_arguments"})),
            ))
        }
    });
    MessageContent::tool_request(
        id,
        arguments.map(|params| CallToolRequestParams {
            task: None,
            name: Cow::Owned(name),
            arguments: Some(object(params)),
            meta: None,
        }),
    )
}

fn finish_reason(done_reason: Option<&str>, called_tools: bool) -> Option<String> {
    match done_reason {
        Some("stop") | None if called_tools => Some("tool_calls".to_string()),
        Some(reason) => Some(reason.to_string()),
        None => None,
    }
}

/// What the decoder needs beyond the lines themselves.
#[derive(Clone, Debug)]
pub struct DecodeOptions {
    pub model_name: String,
    /// The setting an overflow error names (see [`map_error`]).
    pub window_setting: &'static str,
    /// The id every message of this reply carries. Fixed by the caller so the
    /// items of a corrective retry fold into the same assistant message.
    /// `None` mints one.
    pub message_id: Option<String>,
}

impl DecodeOptions {
    pub fn new(model_name: impl Into<String>, window_setting: &'static str) -> Self {
        Self {
            model_name: model_name.into(),
            window_setting,
            message_id: None,
        }
    }
}

/// The model called a tool this request did not offer, and Ollama ended the
/// turn over it (R18): `{"error": "tool 'get_weather' not found"}`, as an HTTP
/// 500 body or as the last line of a 200 stream after the reply's text
/// (ministral-3:8b, Ollama 0.40.2). A model mistake, not a server failure, and
/// NOT a missing model: the old "model ... not found" test read `tool
/// 'model_info' not found` as "Ollama model not found".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownToolCalled {
    pub name: String,
    /// Whether reply text had already streamed before the error.
    pub after_text: bool,
}

impl std::fmt::Display for UnknownToolCalled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "The model called a tool named '{}', which is not one of the tools this chat offers, \
             so nothing was run (Ollama: tool '{}' not found).",
            self.name, self.name
        )?;
        if self.after_text {
            write!(f, " The reply above stopped there and is incomplete.")?;
        }
        Ok(())
    }
}

impl std::error::Error for UnknownToolCalled {}

/// The tool name in Ollama's `tool '<name>' not found`, or `None`.
pub fn unknown_tool_name(text: &str) -> Option<String> {
    let rest = text.trim().strip_prefix("tool '")?;
    let name = rest.strip_suffix("' not found")?;
    (!name.is_empty() && !name.contains('\n')).then(|| name.to_string())
}

/// Decode an `/api/chat` NDJSON stream into BioRouter stream items.
///
/// - Text deltas are yielded as they arrive (empty deltas are skipped — Ollama
///   sends one per thinking token, and each used to become an empty message).
/// - Thinking is yielded to the agent as `MessageContent::Thinking`, batched
///   (see [`THINKING_FLUSH_INTERVAL`]); the agent holds the batches and shows
///   one block per phase (`agents::reply_parts::HeldThinking`). It is never
///   sent back as answer text. Think tags inside `content` are routed there too
///   (R13).
/// - Tool calls are ACCUMULATED across chunks (R9: ministral, mistral-small,
///   llama3.1 and granite send N calls as N chunks and a done chunk with none;
///   gpt-oss sends its call IN the done chunk), announced with a
///   [`PendingToolCall`] the moment they arrive, and yielded together, in one
///   message, with the final usage — so the agent dispatches a multi-call turn
///   in parallel, and so the provider can check the prompt size before
///   anything executes.
/// - A `{"error": …}` line ends the stream with the mapped error, which names
///   `window_setting` if it is a window overflow (see [`map_error`]), and says
///   the reply above is incomplete when text had streamed (R17). A
///   `tool '<X>' not found` line is an [`UnknownToolCalled`] error (R18).
/// - A body that ends with neither `done: true` nor an error line is a
///   transport failure, never an empty answer.
///
/// Every request streams, the provider's blocking calls included: they collect
/// this stream rather than ask for one complete body, so a long generation is
/// never one long silence.
pub fn response_to_streaming_message<S>(
    lines: S,
    model_name: String,
    window_setting: &'static str,
) -> impl Stream<Item = anyhow::Result<ProviderStreamItem>> + 'static
where
    S: Stream<Item = anyhow::Result<String>> + Unpin + Send + 'static,
{
    response_to_streaming_message_with(lines, DecodeOptions::new(model_name, window_setting))
}

/// [`response_to_streaming_message`], with every option.
#[allow(clippy::too_many_lines)]
pub fn response_to_streaming_message_with<S>(
    mut lines: S,
    options: DecodeOptions,
) -> impl Stream<Item = anyhow::Result<ProviderStreamItem>> + 'static
where
    S: Stream<Item = anyhow::Result<String>> + Unpin + Send + 'static,
{
    try_stream! {
        use futures::StreamExt;

        let window_setting = options.window_setting;
        let message_id = options
            .message_id
            .clone()
            .unwrap_or_else(|| format!("ollama_{}", uuid::Uuid::new_v4().simple()));
        let mut splitter = InlineThinkSplitter::default();
        let mut thinking_buffer = String::new();
        let mut last_thinking_flush = Instant::now();
        let mut tool_calls: Vec<MessageContent> = Vec::new();
        let mut model = options.model_name;
        let mut finished = false;
        let mut text_streamed = false;

        while let Some(line) = lines.next().await {
            let line = line?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let chunk: Value = serde_json::from_str(line).map_err(|_| {
                // The chunk text can be model output; keep it out of the error.
                anyhow!("Ollama sent a stream line that is not JSON")
            })?;
            if let Some(error) = chunk.get("error") {
                let text = error_value_text(error);
                if let Some(name) = unknown_tool_name(&text) {
                    Err(anyhow::Error::new(UnknownToolCalled { name, after_text: text_streamed }))?;
                }
                let mapped = map_error(None, &text, &model, None, window_setting);
                Err(anyhow::Error::new(if text_streamed {
                    mark_incomplete(mapped)
                } else {
                    mapped
                }))?;
            }
            if let Some(name) = chunk.get("model").and_then(Value::as_str).filter(|m| !m.is_empty()) {
                model = name.to_string();
            }
            let message = chunk.get("message");

            let mut thinking_delta = message
                .and_then(|m| m.get("thinking"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let (inline_thinking, text_delta) = splitter.push(
                message
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            );
            thinking_delta.push_str(&inline_thinking);
            thinking_buffer.push_str(&thinking_delta);

            let has_more = !text_delta.is_empty()
                || message.and_then(|m| m.get("tool_calls")).is_some()
                || chunk.get("done").and_then(Value::as_bool) == Some(true);
            if !thinking_buffer.is_empty()
                && (has_more
                    || thinking_buffer.len() >= THINKING_FLUSH_CHARS
                    || last_thinking_flush.elapsed() >= THINKING_FLUSH_INTERVAL)
            {
                let batch = std::mem::take(&mut thinking_buffer);
                last_thinking_flush = Instant::now();
                yield (
                    Some(assistant_message(&message_id, vec![MessageContent::thinking(batch, "")])),
                    None,
                    None,
                );
            }

            if !text_delta.is_empty() {
                text_streamed |= !text_delta.trim().is_empty();
                yield (
                    Some(assistant_message(&message_id, vec![MessageContent::text(text_delta)])),
                    None,
                    None,
                );
            }

            if let Some(calls) = message.and_then(|m| m.get("tool_calls")).and_then(Value::as_array) {
                for call in calls {
                    let parsed = parse_tool_call(call);
                    if let MessageContent::ToolRequest(request) = &parsed {
                        let name = call
                            .pointer("/function/name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        yield (
                            None,
                            None,
                            Some(PendingToolCall {
                                id: request.id.clone(),
                                name,
                                partial_args: None,
                            }),
                        );
                    }
                    tool_calls.push(parsed);
                }
            }

            if chunk.get("done").and_then(Value::as_bool) == Some(true) {
                let (inline_thinking, text_tail) = splitter.finish();
                if !inline_thinking.is_empty() {
                    yield (
                        Some(assistant_message(&message_id, vec![MessageContent::thinking(inline_thinking, "")])),
                        None,
                        None,
                    );
                }
                if !text_tail.is_empty() {
                    yield (
                        Some(assistant_message(&message_id, vec![MessageContent::text(text_tail)])),
                        None,
                        None,
                    );
                }
                let usage = ProviderUsage {
                    model: model.clone(),
                    provider: None,
                    usage: get_usage(&chunk).unwrap_or_default(),
                    finish_reason: finish_reason(
                        chunk.get("done_reason").and_then(Value::as_str),
                        !tool_calls.is_empty(),
                    ),
                };
                let calls = (!tool_calls.is_empty())
                    .then(|| assistant_message(&message_id, std::mem::take(&mut tool_calls)));
                yield (calls, Some(usage), None);
                finished = true;
                break;
            }
        }

        if !finished {
            // The body ended without `done: true`: the server went away mid-turn.
            // Held tool calls are dropped — a call that was never confirmed
            // complete must not execute.
            Err(anyhow::Error::new(ProviderError::RequestFailed(
                "Ollama's response ended before the turn was complete; no tool call from it was run. Retry the message.".to_string(),
            )))?;
        }
    }
}

/// Say that the reply already shown stopped at this error (R17): the user has
/// read part of an answer and must not mistake it for the whole.
pub fn mark_incomplete(error: ProviderError) -> ProviderError {
    const NOTE: &str = " The reply above stopped there and is incomplete.";
    match error {
        ProviderError::RequestFailed(text) => ProviderError::RequestFailed(format!("{text}{NOTE}")),
        ProviderError::ServerError(text) => ProviderError::ServerError(format!("{text}{NOTE}")),
        ProviderError::ExecutionError(text) => {
            ProviderError::ExecutionError(format!("{text}{NOTE}"))
        }
        ProviderError::ContextLengthExceeded(text) => {
            ProviderError::ContextLengthExceeded(format!("{text}{NOTE}"))
        }
        other => other,
    }
}

/// The text of an `error` field: a string as-is, anything else serialized.
pub fn error_value_text(error: &Value) -> String {
    match error {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// The error text of an HTTP error body: `{"error": "<s>"}` (Ollama), or the
/// raw body when it is something else.
pub fn error_text_from_body(body: &str) -> String {
    match serde_json::from_str::<Value>(body) {
        Ok(value) => value
            .get("error")
            .map(error_value_text)
            .unwrap_or_else(|| body.trim().to_string()),
        Err(_) => body.trim().to_string(),
    }
}

/// Whether a 404 body is Ollama saying the MODEL is missing, as opposed to a
/// server that has no `/api/chat` route at all (which gets the `/v1` fallback).
pub fn is_model_not_found(body: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return false;
    };
    value
        .get("error")
        .and_then(Value::as_str)
        .map(|text| unknown_tool_name(text).is_none() && says_model_not_found(text))
        .unwrap_or(false)
}

/// Ollama's "model ... not found". Never a TOOL that was not found (R18):
/// `tool 'model_info' not found` mentions a model and is not about one.
fn says_model_not_found(text: &str) -> bool {
    if unknown_tool_name(text).is_some() {
        return false;
    }
    let text = text.to_ascii_lowercase();
    text.contains("model") && text.contains("not found")
}

/// A window overflow, from llama-server's error object (which Ollama forwards
/// as a JSON string), or from the sentences Ollama and its MLX runner use.
struct Overflow {
    prompt_tokens: Option<u64>,
    window: Option<u64>,
    detail: String,
}

fn parse_overflow(text: &str) -> Option<Overflow> {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        let error = value.get("error").unwrap_or(&value);
        let detail = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or(text)
            .to_string();
        if error.get("type").and_then(Value::as_str) == Some("exceed_context_size_error") {
            return Some(Overflow {
                prompt_tokens: error.get("n_prompt_tokens").and_then(Value::as_u64),
                window: error.get("n_ctx").and_then(Value::as_u64),
                detail,
            });
        }
        return parse_overflow_sentence(&detail);
    }
    parse_overflow_sentence(text)
}

fn parse_overflow_sentence(text: &str) -> Option<Overflow> {
    let lower = text.to_ascii_lowercase();
    let is_overflow = lower.contains("exceeds the available context size")
        || lower.contains("exceeds the model's maximum context length")
        || lower.contains("prompt is longer than the context length");
    if !is_overflow {
        return None;
    }
    // "request (28946 tokens) exceeds the available context size (2048 tokens)"
    let numbers: Vec<u64> = regex::Regex::new(r"\((\d+) tokens\)")
        .ok()
        .map(|re| {
            re.captures_iter(text)
                .filter_map(|caps| caps[1].parse().ok())
                .collect()
        })
        .unwrap_or_default();
    Some(Overflow {
        prompt_tokens: numbers.first().copied(),
        window: numbers.get(1).copied(),
        detail: text.to_string(),
    })
}

/// Map an Ollama error to a [`ProviderError`]. `status` is `None` for an error
/// that arrived inside a 200 stream. The server's own words are always kept:
/// the user is the administrator of their local Ollama, and "out of memory" or
/// "model not found" is something they can act on.
///
/// `window_setting` is the setting that changes the window this provider asks
/// for — `OLLAMA_NUM_CTX` for the built-in provider, `BIOROUTER_CONTEXT_LIMIT`
/// for an Ollama-engine custom provider, which does not read `OLLAMA_NUM_CTX`.
/// Naming the wrong one sent a user to raise a setting that changed nothing.
pub fn map_error(
    status: Option<StatusCode>,
    text: &str,
    model: &str,
    requested_window: Option<usize>,
    window_setting: &str,
) -> ProviderError {
    let text = text.trim();
    if let Some(overflow) = parse_overflow(text) {
        let size = match (overflow.prompt_tokens, overflow.window) {
            // The canonical sentence, whatever the server's wording: the agent
            // reads both numbers back out of it (`utils::context_overflow_sizes`)
            // to tell an overflow compaction can fix from one caused by
            // BioRouter's instructions and tools alone, which it cannot.
            (Some(prompt), Some(window)) => format!(
                "{} (the window Ollama is running it with)",
                crate::providers::utils::context_overflow_message(prompt, window)
            ),
            (Some(prompt), None) => format!("the request is {prompt} tokens"),
            _ => "the request does not fit the window Ollama is running it with".to_string(),
        };
        let asked = requested_window
            .map(|n| format!(" (BioRouter asked for {n})"))
            .unwrap_or_default();
        let subject = if model.is_empty() {
            "the model".to_string()
        } else {
            model.to_string()
        };
        return ProviderError::ContextLengthExceeded(format!(
            "The conversation does not fit {subject}'s context window: {size}. \
             BioRouter's instructions and tool definitions alone take tens of thousands of tokens. \
             Raise {window_setting}{asked}, or turn off extensions you are not using to shrink \
             BioRouter's instructions and tool list. Ollama said: {}",
            overflow.detail
        ));
    }

    if let Some(name) = unknown_tool_name(text) {
        // R18: the model's mistake. Not retried here; the provider gives the
        // model one corrective retry before it ever gets this far.
        return ProviderError::RequestFailed(
            UnknownToolCalled {
                name,
                after_text: false,
            }
            .to_string(),
        );
    }
    let lower = text.to_ascii_lowercase();
    if lower.contains("does not support tools") {
        // R19: a capability answer, not a fault, so it is not retried. Seen
        // when `/api/show` could not be read first: gemma3:4b answered
        // `{"error":"registry.ollama.ai/library/gemma3:4b does not support
        // tools"}` (and the same in OpenAI's error shape on `/v1`).
        return no_tools_error(model, Some(text));
    }
    if lower.contains("token repeat limit") {
        return ProviderError::RequestFailed(format!(
            "Ollama stopped the reply because the model kept repeating itself ({text}). Retry, \
             or rephrase the request."
        ));
    }
    if says_model_not_found(text) {
        let name = if model.is_empty() { "<model>" } else { model };
        return ProviderError::RequestFailed(format!(
            "Ollama model not found: {name} is not downloaded on this Ollama server. \
             Run: ollama pull {name}  (Ollama said: {text})"
        ));
    }
    if lower.contains("more system memory") || lower.contains("out of memory") {
        // Not a transient fault: retrying the same load on the same machine
        // fails the same way, so this must not read as a server error.
        return ProviderError::ExecutionError(format!(
            "Ollama could not load the model with the memory available: {text}. \
             Close other applications, lower {window_setting}, or choose a smaller model."
        ));
    }

    match status {
        Some(StatusCode::UNAUTHORIZED) | Some(StatusCode::FORBIDDEN) => {
            ProviderError::Authentication(format!("Ollama refused the request: {text}"))
        }
        Some(StatusCode::TOO_MANY_REQUESTS) => ProviderError::RateLimitExceeded {
            details: format!("Ollama is busy: {text}"),
            retry_delay: None,
        },
        Some(status) if status.is_server_error() => {
            ProviderError::ServerError(format!("Ollama server error ({status}): {text}"))
        }
        Some(status) => {
            ProviderError::RequestFailed(format!("Ollama rejected the request ({status}): {text}"))
        }
        None => ProviderError::RequestFailed(format!("Ollama reported an error: {text}")),
    }
}

/// The actionable error for a model without tool support (R19), sent before
/// anything is asked of the model when `/api/show` says so, or mapped from
/// Ollama's own 400. Not transient: a retry gets the same answer.
pub fn no_tools_error(model: &str, server_said: Option<&str>) -> ProviderError {
    let subject = if model.is_empty() {
        "This model"
    } else {
        model
    };
    let said = server_said
        .map(|text| format!(" (Ollama said: {text})"))
        .unwrap_or_default();
    ProviderError::RequestFailed(format!(
        "{subject} cannot call tools: Ollama reports no tool support for it{said}, and \
         Biorouter's agent works through tool calls. Choose a model that supports tools \
         (qwen3.8:27b and gemma4:31b are the supported local models), or set \
         BIOROUTER_TOOLSHIM=true to have tool calls read from this model's text."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::TryStreamExt;
    use rmcp::model::CallToolResult;

    fn call(name: &str, args: Value) -> CallToolRequestParams {
        CallToolRequestParams {
            task: None,
            name: name.to_string().into(),
            arguments: Some(object(args)),
            meta: None,
        }
    }

    fn request<'a>(messages: &'a [Message], tools: &'a [Tool]) -> ChatRequest<'a> {
        ChatRequest {
            model: "qwen3.8:27b",
            system: "SYS",
            messages,
            tools,
            stream: true,
            num_ctx: 65_536,
            temperature: None,
            num_predict: None,
            think: None,
            keep_alive: Some(json!("30m")),
            vision: false,
            calls_without_text: false,
        }
    }

    #[test]
    fn the_request_carries_the_window_and_refuses_truncation_without_default_sampling() {
        let messages = vec![Message::user().with_text("hello")];
        let payload = create_request(&request(&messages, &[])).unwrap();
        assert_eq!(payload["options"]["num_ctx"], 65_536);
        assert_eq!(payload["truncate"], false);
        assert_eq!(payload["keep_alive"], "30m");
        assert_eq!(payload["stream"], true);
        // No sampling defaults: the Modelfile's recommended values must apply.
        assert!(payload["options"].get("temperature").is_none());
        assert!(payload["options"].get("top_p").is_none());
        assert!(payload["options"].get("num_predict").is_none());
        assert!(payload.get("think").is_none());
        assert!(payload.get("tools").is_none());
        // Exactly one system message, at index 0.
        let roles: Vec<&str> = payload["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["system", "user"]);
    }

    #[test]
    fn pinned_sampling_and_think_are_forwarded() {
        let messages = vec![Message::user().with_text("hello")];
        let mut req = request(&messages, &[]);
        req.temperature = Some(0.0);
        req.num_predict = Some(64);
        req.think = Some(Think::Level("xhigh".to_string()));
        let payload = create_request(&req).unwrap();
        assert_eq!(payload["options"]["temperature"], 0.0);
        assert_eq!(payload["options"]["num_predict"], 64);
        assert_eq!(payload["think"], "xhigh");
    }

    #[test]
    fn a_conversation_without_a_user_turn_gets_one() {
        let messages = vec![Message::assistant().with_text("hi")];
        let payload = create_request(&request(&messages, &[])).unwrap();
        let last = payload["messages"].as_array().unwrap().last().unwrap();
        assert_eq!(last["role"], "user");
    }

    #[test]
    fn tool_history_uses_object_arguments_tool_name_and_call_id() {
        let messages = vec![
            Message::user().with_text("read it"),
            Message::assistant().with_tool_request(
                "call_1",
                Ok(call("developer__read_file", json!({"path": "/x"}))),
            ),
            Message::user().with_tool_response(
                "call_1",
                Ok(CallToolResult {
                    content: vec![rmcp::model::Content::text("alpha\nbeta")],
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
            ),
        ];
        let out = format_messages(&messages, false, false);
        assert_eq!(out[1]["role"], "assistant");
        assert_eq!(
            out[1]["tool_calls"][0]["function"]["arguments"],
            json!({"path": "/x"}),
            "arguments must be an object on the native endpoint"
        );
        assert_eq!(out[1]["tool_calls"][0]["id"], "call_1");
        assert_eq!(out[2]["role"], "tool");
        assert_eq!(out[2]["tool_name"], "developer__read_file");
        assert_eq!(out[2]["tool_call_id"], "call_1");
        assert_eq!(out[2]["content"], "alpha\nbeta");
    }

    #[test]
    fn info_msg_merged_into_a_tool_result_turn_follows_the_result() {
        let messages = vec![
            Message::user().with_text("go"),
            Message::assistant().with_tool_request("c1", Ok(call("shell", json!({})))),
            Message::user()
                .with_tool_response(
                    "c1",
                    Ok(CallToolResult {
                        content: vec![rmcp::model::Content::text("done")],
                        structured_content: None,
                        is_error: Some(false),
                        meta: None,
                    }),
                )
                .with_text(format!("{MOIM_OPEN_TAG}\nAutomatic context</info-msg>")),
        ];
        let out = format_messages(&messages, false, false);
        let roles: Vec<&str> = out.iter().map(|m| m["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["user", "assistant", "tool", "user"]);
    }

    #[test]
    fn thinking_is_replayed_only_inside_the_current_tool_loop_and_rows_merge() {
        let messages = vec![
            Message::user().with_text("first question"),
            Message::assistant()
                .with_thinking("OLD-THOUGHT", "")
                .with_text("first answer"),
            Message::user().with_text("second question"),
            // BioRouter persists one turn as a thinking row then a tool row.
            Message::assistant().with_thinking("NEW-THOUGHT", ""),
            Message::assistant()
                .with_tool_request("c1", Ok(call("shell", json!({"command": "ls"})))),
            Message::user().with_tool_response(
                "c1",
                Ok(CallToolResult {
                    content: vec![rmcp::model::Content::text("a b")],
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
            ),
            // The info-msg block is not a user query.
            Message::user().with_text(format!("{MOIM_OPEN_TAG}\nctx</info-msg>")),
        ];
        let out = format_messages(&messages, false, false);
        assert!(
            out[1].get("thinking").is_none(),
            "history before the last query drops thinking"
        );
        assert_eq!(out[1]["content"], "first answer");
        let current = &out[3];
        assert_eq!(current["role"], "assistant");
        assert_eq!(current["thinking"], "NEW-THOUGHT");
        assert_eq!(current["tool_calls"][0]["function"]["name"], "shell");
        assert_eq!(
            out.iter().filter(|m| m["role"] == "assistant").count(),
            2,
            "the thinking row and the tool row are one assistant turn"
        );
    }

    #[test]
    fn tool_result_blocks_reach_the_model_by_audience() {
        let messages = vec![
            Message::assistant().with_tool_request("call-1", Ok(call("t", json!({})))),
            Message::user().with_tool_response(
                "call-1",
                Ok(CallToolResult {
                    content: audience::every_audience_case(),
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
            ),
        ];
        let out = format_messages(&messages, false, false);
        let sent = out[1]["content"].as_str().unwrap();
        assert_eq!(sent, audience::MODEL_VISIBLE.join("\n"));
        for withheld in audience::MODEL_HIDDEN {
            assert!(!sent.contains(withheld), "{withheld} reached the model");
        }

        let view = vec![
            Message::assistant().with_tool_request("call-2", Ok(call("t", json!({})))),
            Message::user().with_tool_response(
                "call-2",
                Ok(CallToolResult {
                    content: audience::text_editor_view_result(),
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
            ),
        ];
        let out = format_messages(&view, false, false);
        assert_eq!(out[1]["content"], audience::VIEW_FOR_MODEL);
    }

    #[test]
    fn images_go_in_images_only_for_a_vision_model() {
        let messages = vec![Message::user()
            .with_text("what is this")
            .with_image("aGVsbG8=", "image/png")];
        let with_vision = format_messages(&messages, true, false);
        assert_eq!(with_vision[0]["images"], json!(["aGVsbG8="]));
        let without = format_messages(&messages, false, false);
        assert!(without[0].get("images").is_none());
        assert!(without[0]["content"]
            .as_str()
            .unwrap()
            .contains("cannot read images"));
    }

    #[test]
    fn text_bytes_ignore_image_payloads() {
        let image = "A".repeat(100_000);
        let payload = json!({
            "messages": [
                {"role": "user", "content": "hi", "images": [image.clone()]},
                {"role": "user", "content": [{"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{image}")}}]},
            ]
        });
        assert!(text_bytes(&payload) < 200, "{}", text_bytes(&payload));
    }

    #[test]
    fn usage_splits_cached_tokens_out_of_the_prompt() {
        let usage = get_usage(&json!({
            "done": true,
            "prompt_eval_count": 202,
            "prompt_eval_cached_count": 201,
            "eval_count": 14,
        }))
        .unwrap();
        assert_eq!(usage.input_tokens, Some(1));
        assert_eq!(usage.cache_read_input_tokens, Some(201));
        assert_eq!(usage.output_tokens, Some(14));
        assert_eq!(usage.total_tokens, Some(216));

        let uncached = get_usage(
            &json!({"prompt_eval_count": 360, "prompt_eval_cached_count": 0, "eval_count": 61}),
        )
        .unwrap();
        assert_eq!(uncached.input_tokens, Some(360));
        assert_eq!(uncached.cache_read_input_tokens, None);
    }

    #[test]
    fn show_facts_read_the_context_key_through_the_architecture() {
        let show = json!({
            "capabilities": ["completion", "vision", "tools", "thinking"],
            "model_info": {"general.architecture": "qwen35", "qwen35.context_length": 262144},
            "thinking": {"values": [false, "low", "medium", "xhigh"], "default": "medium"},
        });
        let facts = ModelFacts::from_show(&show);
        assert_eq!(facts.trained_context, Some(262_144));
        assert!(facts.supports_thinking() && facts.supports_vision());
        assert_eq!(facts.supports_tools(), Some(true));
        assert_eq!(facts.deepest_think(), Think::Level("xhigh".into()));
        // `high` raises in qwen3.8's template; it must never be forwarded.
        assert_eq!(facts.accept(&Think::Level("high".into())), None);
        assert_eq!(
            facts.accept(&Think::Level("low".into())),
            Some(Think::Level("low".into()))
        );
        assert_eq!(facts.accept(&Think::Off), Some(Think::Off));

        let gemma = ModelFacts::from_show(&json!({
            "capabilities": ["completion", "tools", "thinking"],
            "model_info": {"general.architecture": "gemma4", "gemma4.context_length": 262144},
            "thinking": {"values": [false, true], "default": true},
        }));
        assert_eq!(gemma.deepest_think(), Think::On);
        assert_eq!(gemma.accept(&Think::Level("xhigh".into())), Some(Think::On));

        let plain = ModelFacts::from_show(&json!({"capabilities": ["completion"]}));
        assert_eq!(
            plain.accept(&Think::Off),
            None,
            "never send think to a non-thinking model"
        );
        assert_eq!(plain.supports_tools(), Some(false));
        assert_eq!(ModelFacts::from_show(&json!({})).supports_tools(), None);
    }

    fn lines(body: &str) -> impl Stream<Item = anyhow::Result<String>> + Unpin + Send + 'static {
        futures::stream::iter(
            body.lines()
                .map(|line| Ok(line.to_string()))
                .collect::<Vec<_>>(),
        )
    }

    async fn decode(body: &str) -> anyhow::Result<Vec<ProviderStreamItem>> {
        Box::pin(response_to_streaming_message(
            lines(body),
            "m".to_string(),
            "OLLAMA_NUM_CTX",
        ))
        .try_collect()
        .await
    }

    fn texts(items: &[ProviderStreamItem]) -> String {
        items
            .iter()
            .filter_map(|(m, _, _)| m.as_ref())
            .map(Message::as_concat_text)
            .collect()
    }

    fn thinking(items: &[ProviderStreamItem]) -> String {
        items
            .iter()
            .filter_map(|(m, _, _)| m.as_ref())
            .flat_map(|m| m.content.iter())
            .filter_map(|c| match c {
                MessageContent::Thinking(t) => Some(t.thinking.clone()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn text_and_thinking_stream_separately_and_empty_deltas_are_skipped() {
        let body = concat!(
            r#"{"model":"qwen3.8:27b","message":{"role":"assistant","content":"","thinking":"Let me "},"done":false}"#,
            "\n",
            r#"{"model":"qwen3.8:27b","message":{"role":"assistant","content":"","thinking":"think."},"done":false}"#,
            "\n",
            r#"{"model":"qwen3.8:27b","message":{"role":"assistant","content":""},"done":false}"#,
            "\n",
            r#"{"model":"qwen3.8:27b","message":{"role":"assistant","content":"Hello"},"done":false}"#,
            "\n",
            r#"{"model":"qwen3.8:27b","message":{"role":"assistant","content":" world"},"done":false}"#,
            "\n",
            r#"{"model":"qwen3.8:27b","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":30913,"prompt_eval_cached_count":0,"eval_count":12}"#,
            "\n",
        );
        let items = decode(body).await.unwrap();
        assert_eq!(texts(&items), "Hello world");
        assert_eq!(thinking(&items), "Let me think.");
        assert!(
            items
                .iter()
                .filter_map(|(m, _, _)| m.as_ref())
                .all(|m| !m.content.is_empty()),
            "no empty messages"
        );
        assert!(
            !items
                .iter()
                .filter_map(|(m, _, _)| m.as_ref())
                .any(|m| m.as_concat_text().contains("think.")),
            "thinking never becomes answer text"
        );
        let usage = items.iter().find_map(|(_, u, _)| u.clone()).unwrap();
        assert_eq!(usage.usage.input_tokens, Some(30_913));
        assert_eq!(usage.finish_reason.as_deref(), Some("stop"));
        assert_eq!(usage.model, "qwen3.8:27b");
        // Every message of one response shares an id, so the agent merges them.
        let ids: std::collections::HashSet<_> = items
            .iter()
            .filter_map(|(m, _, _)| m.as_ref().and_then(|m| m.id.clone()))
            .collect();
        assert_eq!(ids.len(), 1);
    }

    #[tokio::test]
    async fn tool_calls_arrive_complete_and_are_yielded_together_with_usage() {
        let body = concat!(
            r#"{"model":"m","message":{"role":"assistant","content":"","thinking":"plan"},"done":false}"#,
            "\n",
            r#"{"model":"m","message":{"role":"assistant","content":"","tool_calls":[{"id":"call_a","function":{"index":0,"name":"developer__shell","arguments":{"command":"ls"}}}]},"done":false}"#,
            "\n",
            r#"{"model":"m","message":{"role":"assistant","content":"","tool_calls":[{"function":{"index":1,"name":"todo__write","arguments":{"items":["x"]}}}]},"done":false}"#,
            "\n",
            r#"{"model":"m","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":10,"eval_count":5}"#,
            "\n",
        );
        let items = decode(body).await.unwrap();
        let pending: Vec<_> = items.iter().filter_map(|(_, _, p)| p.clone()).collect();
        assert_eq!(pending.len(), 2, "each call is announced as it arrives");
        let (message, usage, _) = items
            .iter()
            .find(|(m, _, _)| m.as_ref().is_some_and(Message::is_tool_call))
            .unwrap();
        assert!(usage.is_some(), "calls are held until the final usage");
        let requests: Vec<_> = message
            .as_ref()
            .unwrap()
            .content
            .iter()
            .filter_map(MessageContent::as_tool_request)
            .collect();
        assert_eq!(requests.len(), 2, "both calls in one message");
        assert_eq!(requests[0].id, "call_a");
        let first = requests[0].tool_call.as_ref().unwrap();
        assert_eq!(first.name, "developer__shell");
        assert_eq!(first.arguments.as_ref().unwrap()["command"], "ls");
        assert!(
            requests[1].id.starts_with("call_"),
            "a missing id is generated"
        );
        assert_eq!(
            usage.as_ref().unwrap().finish_reason.as_deref(),
            Some("tool_calls")
        );
    }

    #[tokio::test]
    async fn length_is_reported_as_the_finish_reason() {
        let body = concat!(
            r#"{"model":"m","message":{"role":"assistant","content":"partial"},"done":false}"#,
            "\n",
            r#"{"model":"m","message":{"role":"assistant","content":""},"done":true,"done_reason":"length","prompt_eval_count":10,"eval_count":64}"#,
            "\n",
        );
        let items = decode(body).await.unwrap();
        let usage = items.iter().find_map(|(_, u, _)| u.clone()).unwrap();
        assert_eq!(usage.finish_reason.as_deref(), Some("length"));
    }

    #[tokio::test]
    async fn a_mid_stream_error_line_ends_the_stream_with_the_servers_words() {
        let body = concat!(
            r#"{"model":"m","message":{"role":"assistant","content":"par"},"done":false}"#,
            "\n",
            r#"{"error":"llama runner process has terminated: signal: killed"}"#,
            "\n",
        );
        let error = decode(body).await.unwrap_err();
        let error = error.downcast::<ProviderError>().unwrap();
        assert!(error
            .to_string()
            .contains("llama runner process has terminated"));
    }

    #[tokio::test]
    async fn a_stream_cut_before_done_never_releases_held_tool_calls() {
        let body = concat!(
            r#"{"model":"m","message":{"role":"assistant","content":"","tool_calls":[{"id":"c","function":{"name":"shell","arguments":{}}}]},"done":false}"#,
            "\n",
        );
        let error = decode(body).await.unwrap_err();
        assert!(error
            .to_string()
            .contains("ended before the turn was complete"));
    }

    #[tokio::test]
    async fn inline_think_tags_are_routed_to_thinking_across_split_deltas() {
        let body = concat!(
            r#"{"model":"m","message":{"role":"assistant","content":"<thi"},"done":false}"#,
            "\n",
            r#"{"model":"m","message":{"role":"assistant","content":"nk>reason"},"done":false}"#,
            "\n",
            r#"{"model":"m","message":{"role":"assistant","content":"ing</th"},"done":false}"#,
            "\n",
            r#"{"model":"m","message":{"role":"assistant","content":"ink>\n\nAnswer"},"done":false}"#,
            "\n",
            r#"{"model":"m","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop"}"#,
            "\n",
        );
        let items = decode(body).await.unwrap();
        assert_eq!(texts(&items), "Answer");
        assert_eq!(thinking(&items), "reasoning");
    }

    /// A reply that did not open with a think block or a tag's tail owns its
    /// tags: an answer ABOUT think tags keeps them, split across deltas or
    /// not, and nothing of it becomes thinking.
    #[tokio::test]
    async fn think_tags_in_an_ordinary_answer_are_literal_text() {
        let answer = "Strip them with `re.sub(r\"<think>.*?</think>\", \"\", s)`, or split on \
                      `</think>` and keep the last part.";
        let line = |content: &str, done: bool| {
            let mut line = json!({"model": "m", "message": {"role": "assistant", "content": content}, "done": done});
            if done {
                line["done_reason"] = json!("stop");
            }
            format!("{line}\n")
        };
        let whole = format!("{}{}", line(answer, false), line("", true));
        let mut split = String::new();
        for piece in answer.as_bytes().chunks(3) {
            split.push_str(&line(std::str::from_utf8(piece).unwrap(), false));
        }
        split.push_str(&line("", true));
        for body in [whole, split] {
            let items = decode(&body).await.unwrap();
            assert_eq!(texts(&items), answer);
            assert_eq!(thinking(&items), "");
        }

        // deepseek-r1:8b's measured follow-up opened with the `>` tail of a
        // think tag its template had opened: there the tags ARE markup.
        let deepseek = concat!(
            ">\n\nThe files are alpha.txt and beta.csv.\n</think><think>\nOkay, the user ",
            "asked about files."
        );
        let items = decode(&format!("{}{}", line(deepseek, false), line("", true)))
            .await
            .unwrap();
        assert_eq!(texts(&items), "The files are alpha.txt and beta.csv.\n");
        assert_eq!(thinking(&items), "\nOkay, the user asked about files.");
    }

    /// The verbatim body Ollama 0.40.2 returned for a 12,025-token prompt at a
    /// 2048 window with `truncate: false`, streaming and not.
    const OVERFLOW_BODY: &str = r#"{"error":"{\"error\":{\"code\":400,\"message\":\"request (12025 tokens) exceeds the available context size (2048 tokens), try increasing it\",\"type\":\"exceed_context_size_error\",\"n_prompt_tokens\":12025,\"n_ctx\":2048}}"}"#;

    #[test]
    fn the_overflow_400_is_context_length_exceeded_naming_both_numbers_and_the_fixes() {
        let error = map_error(
            Some(StatusCode::BAD_REQUEST),
            &error_text_from_body(OVERFLOW_BODY),
            "qwen3:8b",
            Some(2048),
            "OLLAMA_NUM_CTX",
        );
        let ProviderError::ContextLengthExceeded(text) = &error else {
            panic!("expected ContextLengthExceeded, got {error:?}");
        };
        assert!(text.contains("12025 tokens"), "{text}");
        assert!(text.contains("2048 tokens"), "{text}");
        assert!(text.contains("OLLAMA_NUM_CTX"), "{text}");
        assert!(text.contains("extensions"), "{text}");
        assert_eq!(
            crate::providers::utils::context_overflow_sizes(text),
            Some((12025, 2048)),
            "the agent must be able to read both sizes back: {text}"
        );

        // The same sentence without the JSON wrapper (another server, or a
        // future Ollama that stops forwarding llama-server's object).
        let plain = map_error(
            Some(StatusCode::BAD_REQUEST),
            "request (28946 tokens) exceeds the available context size (4096 tokens), try increasing it",
            "m",
            None,
            "OLLAMA_NUM_CTX",
        );
        assert!(
            matches!(plain, ProviderError::ContextLengthExceeded(ref t) if t.contains("28946") && t.contains("4096"))
        );
    }

    /// A custom Ollama-engine provider does not read `OLLAMA_NUM_CTX`, so its
    /// errors must name the setting it does honour, not one that changes
    /// nothing for it.
    #[test]
    fn overflow_and_memory_errors_name_the_providers_own_window_setting() {
        let overflow = map_error(
            Some(StatusCode::BAD_REQUEST),
            &error_text_from_body(OVERFLOW_BODY),
            "qwen3:8b",
            Some(65_536),
            "BIOROUTER_CONTEXT_LIMIT",
        )
        .to_string();
        assert!(
            overflow.contains("Raise BIOROUTER_CONTEXT_LIMIT"),
            "{overflow}"
        );
        assert!(!overflow.contains("OLLAMA_NUM_CTX"), "{overflow}");
        let memory = map_error(
            Some(StatusCode::INTERNAL_SERVER_ERROR),
            "model requires more system memory (40.1 GiB) than is available (12.0 GiB)",
            "gemma4:31b",
            None,
            "BIOROUTER_CONTEXT_LIMIT",
        )
        .to_string();
        assert!(memory.contains("lower BIOROUTER_CONTEXT_LIMIT"), "{memory}");
        assert!(!memory.contains("OLLAMA_NUM_CTX"), "{memory}");
    }

    #[test]
    fn a_missing_model_says_how_to_pull_it_and_is_not_a_route_404() {
        let body = r#"{"error":"model 'qwen3.8:27b' not found"}"#;
        assert!(is_model_not_found(body));
        assert!(!is_model_not_found("404 page not found"));
        assert!(!is_model_not_found(
            r#"{"error":{"code":404,"message":"File Not Found","type":"not_found_error"}}"#
        ));
        assert!(!is_model_not_found(r#"{"detail":"Not Found"}"#));
        let error = map_error(
            Some(StatusCode::NOT_FOUND),
            &error_text_from_body(body),
            "qwen3.8:27b",
            None,
            "OLLAMA_NUM_CTX",
        );
        let text = error.to_string();
        assert!(text.contains("ollama pull qwen3.8:27b"), "{text}");
        assert_eq!(
            error.kind(),
            crate::providers::errors::ProviderErrorKind::ModelUnavailable
        );
    }

    #[test]
    fn server_text_is_surfaced_and_memory_errors_are_not_retried() {
        let error = map_error(
            Some(StatusCode::INTERNAL_SERVER_ERROR),
            "model requires more system memory (40.1 GiB) than is available (12.0 GiB)",
            "gemma4:31b",
            None,
            "OLLAMA_NUM_CTX",
        );
        assert!(error.to_string().contains("40.1 GiB"));
        assert!(!error.kind().is_transient(), "{error:?}");

        let busy = map_error(
            Some(StatusCode::SERVICE_UNAVAILABLE),
            "server busy",
            "m",
            None,
            "OLLAMA_NUM_CTX",
        );
        assert!(busy.to_string().contains("server busy"));
        assert!(busy.kind().is_transient());
    }

    fn tool_result(id: &str, text: &str) -> Message {
        Message::user().with_tool_response(
            id,
            Ok(CallToolResult {
                content: vec![rmcp::model::Content::text(text)],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        )
    }

    /// R5: the mistral, granite, llama3.1 and ministral templates drop the id
    /// and pair results with calls by POSITION, so parallel results stored in
    /// the order they finished are sent in the order the calls were made.
    #[test]
    fn tool_results_follow_the_order_of_their_calls() {
        let messages = vec![
            Message::user().with_text("both"),
            Message::assistant()
                .with_tool_request("c1", Ok(call("developer__shell", json!({"command": "ls"}))))
                .with_tool_request(
                    "c2",
                    Ok(call("knowledge__kb_search", json!({"query": "q"}))),
                ),
            // The second call finished first.
            tool_result("c2", "found"),
            tool_result("c1", "a b"),
        ];
        let out = format_messages(&messages, false, false);
        let ids: Vec<&str> = out
            .iter()
            .filter(|m| m["role"] == "tool")
            .map(|m| m["tool_call_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["c1", "c2"]);
        let names: Vec<&str> = out
            .iter()
            .filter(|m| m["role"] == "tool")
            .map(|m| m["tool_name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["developer__shell", "knowledge__kb_search"]);
    }

    /// R4: a `role: "tool"` message always names its tool; gpt-oss rendered a
    /// nameless one as `functions. to=assistant`. A result whose call is not in
    /// the history has no name to give, so it goes as user text instead.
    #[test]
    fn a_result_without_its_call_is_sent_as_text_never_as_a_nameless_tool_message() {
        let messages = vec![
            Message::user().with_text("go"),
            tool_result("compacted-away", "old output"),
        ];
        let out = format_messages(&messages, false, false);
        assert!(out.iter().all(|m| m["role"] != "tool"), "{out:?}");
        assert!(out
            .iter()
            .any(|m| m["role"] == "user" && m["content"].as_str().unwrap().contains("old output")));
    }

    /// R7: a template that renders an assistant turn's text and then its calls
    /// only `else if .ToolCalls` drops the calls when both are present, so for
    /// such a model a tool-call turn is sent with no text.
    #[test]
    fn calls_without_text_empties_the_text_of_tool_call_turns_only() {
        let messages = vec![
            Message::user().with_text("go"),
            Message::assistant()
                .with_text("I will list the files.")
                .with_tool_request("c1", Ok(call("developer__shell", json!({"command": "ls"})))),
            tool_result("c1", "a b"),
            Message::assistant().with_text("Two files."),
        ];
        let kept = format_messages(&messages, false, false);
        assert_eq!(kept[1]["content"], "I will list the files.");
        let emptied = format_messages(&messages, false, true);
        assert_eq!(emptied[1]["content"], "");
        assert_eq!(
            emptied[1]["tool_calls"][0]["function"]["name"],
            "developer__shell"
        );
        assert_eq!(
            emptied[3]["content"], "Two files.",
            "a text-only turn keeps its text"
        );
    }

    /// The template facts are read from the template's literal text, checked
    /// against the templates Ollama 0.40.2 ships: llama3.1/3.2 and
    /// mistral-small3.2/mistral-nemo hide the tools after a tool result (R8);
    /// ministral-3 and devstral show them at the last USER message, which a
    /// tool result does not move, so they are not flagged.
    #[test]
    fn template_facts_match_the_shipped_templates_and_nothing_else() {
        let facts = |template: &str| ModelFacts::from_show(&json!({"template": template}));
        for hides in [
            "{{- $last := eq (len (slice $.Messages $i)) 1 }}\n{{- if and $.Tools $last }}",
            "{{- if and (le (len (slice $.Messages $index)) 2) $.Tools }}[AVAILABLE_TOOLS]",
            "{{- if and $.Tools (le (len (slice $.Messages $i)) 2) }}[AVAILABLE_TOOLS]",
        ] {
            assert!(facts(hides).tools_only_near_last_user, "{hides}");
        }
        for shows in [
            "{{- if and (eq $lastUserIndex $index) $.Tools }}[AVAILABLE_TOOLS]{{ $.Tools }}",
            "{{- if .Tools }}\n# Tools\n{{- range .Tools }}",
            "{{ .Prompt }}",
            "",
        ] {
            assert!(!facts(shows).tools_only_near_last_user, "{shows}");
        }
        assert!(
            facts("{{- if .Content }} {{ .Content }}{{- else if .ToolCalls }}[TOOL_CALLS]")
                .drops_calls_after_text
        );
        assert!(!facts("{{ .Content }}{{ range .ToolCalls }}").drops_calls_after_text);

        assert!(ModelFacts::from_show(&json!({"remote_host": "https://ollama.com:443"})).remote);
        assert!(ModelFacts::from_show(&json!({"remote_model": "gpt-oss:120b"})).remote);
        assert!(!ModelFacts::from_show(&json!({"remote_host": "  "})).remote);
        assert!(!ModelFacts::from_show(&json!({"capabilities": ["tools"]})).remote);
        assert_eq!(
            ModelFacts::from_show(&json!({"remote_host": "https://ollama.com:443"}))
                .remote_origin
                .as_deref(),
            Some("https://ollama.com")
        );
        assert_eq!(
            ModelFacts::from_show(&json!({"remote_model": "gpt-oss:120b"})).remote_origin,
            None
        );
    }

    /// R6, the two thinking models that do not take `false`: gpt-oss declares
    /// levels without `false` (Ollama ignores `false` and thinks anyway), so
    /// "off" is its lowest level; deepseek-r1 declares no levels and thinks
    /// whatever `think` says, so its output budget must leave room for it.
    #[test]
    fn thinking_off_maps_to_what_each_model_can_actually_do() {
        let gpt_oss = ModelFacts::from_show(&json!({
            "capabilities": ["completion", "tools", "thinking"],
            "thinking": {"values": ["low", "medium", "high"]},
        }));
        assert_eq!(
            gpt_oss.accept(&Think::Off),
            Some(Think::Level("low".into()))
        );
        assert_eq!(
            gpt_oss.accept(&Think::Level("high".into())),
            Some(Think::Level("high".into()))
        );
        assert!(gpt_oss.thinks_regardless());

        let deepseek = ModelFacts::from_show(&json!({"capabilities": ["completion", "thinking"]}));
        assert_eq!(deepseek.accept(&Think::Off), Some(Think::Off));
        assert!(deepseek.thinks_regardless());

        let qwen = ModelFacts::from_show(&json!({
            "capabilities": ["completion", "tools", "thinking"],
            "thinking": {"values": [false, "low", "medium", "xhigh"]},
        }));
        assert!(!qwen.thinks_regardless());
    }

    /// R18: Ollama's `tool 'X' not found` is a model mistake. It never reads as
    /// a missing MODEL — `tool 'model_info' not found` mentions one — and the
    /// message names the tool.
    #[test]
    fn a_tool_that_does_not_exist_is_never_a_missing_model() {
        assert_eq!(
            unknown_tool_name("tool 'get_weather' not found").as_deref(),
            Some("get_weather")
        );
        assert_eq!(unknown_tool_name("model 'x' not found"), None);
        let body = r#"{"error":"tool 'model_info' not found"}"#;
        assert!(!is_model_not_found(body));
        let error = map_error(
            Some(StatusCode::INTERNAL_SERVER_ERROR),
            &error_text_from_body(body),
            "ministral-3:8b",
            None,
            "OLLAMA_NUM_CTX",
        );
        let text = error.to_string();
        assert!(text.contains("tool named 'model_info'"), "{text}");
        assert!(!text.contains("ollama pull"), "{text}");
        assert!(!error.kind().is_transient(), "{error:?}");
    }

    /// R17(d): granite4's runner died over an overflow with an HTTP 500 whose
    /// text ends in `GGML_ASSERT(buffer) failed`. The sentence inside it names
    /// both sizes, so it is an overflow, not a server error to retry.
    #[test]
    fn a_runner_crash_over_an_overflow_reads_as_one() {
        let body = r#"{"error":"an error was encountered while running the model: error: task id = 0, error: request (13099 tokens) exceeds the available context size (2048 tokens), try increasing it\nGGML_ASSERT(buffer) failed"}"#;
        let error = map_error(
            Some(StatusCode::INTERNAL_SERVER_ERROR),
            &error_text_from_body(body),
            "granite4:tiny-h",
            Some(2048),
            "OLLAMA_NUM_CTX",
        );
        let ProviderError::ContextLengthExceeded(text) = &error else {
            panic!("expected ContextLengthExceeded, got {error:?}");
        };
        assert_eq!(
            crate::providers::utils::context_overflow_sizes(text),
            Some((13099, 2048)),
            "{text}"
        );
    }

    /// R19: the 400 a model without tools answers is a capability answer,
    /// in both of Ollama's shapes, and never transient.
    #[test]
    fn does_not_support_tools_is_an_actionable_refusal_not_a_fault() {
        for body in [
            r#"{"error":"registry.ollama.ai/library/gemma3:4b does not support tools"}"#,
            r#"{"error":{"message":"registry.ollama.ai/library/gemma3:4b does not support tools","type":"invalid_request_error","param":null,"code":null}}"#,
        ] {
            let error = map_error(
                Some(StatusCode::BAD_REQUEST),
                &error_text_from_body(body),
                "gemma3:4b",
                None,
                "OLLAMA_NUM_CTX",
            );
            let text = error.to_string();
            assert!(text.contains("gemma3:4b cannot call tools"), "{text}");
            assert!(text.contains("BIOROUTER_TOOLSHIM"), "{text}");
            assert!(!error.kind().is_transient(), "{error:?}");
        }
    }

    /// R17(b) + R18: ministral-3:8b streamed a sentence, then Ollama ended the
    /// 200 stream with `{"error":"tool 'get_weather' not found"}` and no done
    /// line. The text is delivered, and the error says which tool and that
    /// the reply above is incomplete.
    #[tokio::test]
    async fn an_unknown_tool_after_text_keeps_the_text_and_names_the_tool() {
        let body = concat!(
            r#"{"model":"m","message":{"role":"assistant","content":"I am about to fetch the weather."},"done":false}"#,
            "\n",
            r#"{"error":"tool 'get_weather' not found"}"#,
            "\n",
        );
        let mut decoded = Box::pin(response_to_streaming_message(
            lines(body),
            "m".to_string(),
            "OLLAMA_NUM_CTX",
        ));
        use futures::StreamExt;
        let first = decoded.next().await.unwrap().unwrap();
        assert_eq!(
            first.0.unwrap().as_concat_text(),
            "I am about to fetch the weather."
        );
        let error = decoded.next().await.unwrap().unwrap_err();
        let called = error.downcast::<UnknownToolCalled>().unwrap();
        assert_eq!(called.name, "get_weather");
        assert!(called.after_text);
        assert!(called.to_string().contains("incomplete"), "{called}");
    }
}
