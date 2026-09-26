use crate::conversation::message::{Message, MessageContent, ToolRequest, ToolResponse};
use crate::model::ModelConfig;
use crate::providers::base::{ProviderUsage, Usage};
use crate::providers::formats::audience;
use crate::providers::formats::openai::{model_reasoning_effort, model_supports_reasoning_effort};
use anyhow::{anyhow, Error};
use async_stream::try_stream;
use chrono;
use futures::Stream;
use rmcp::model::{object, CallToolRequestParams, ErrorCode, ErrorData, RawContent, Role, Tool};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponsesApiResponse {
    pub id: String,
    pub object: String,
    pub created_at: i64,
    pub status: String,
    pub model: String,
    pub output: Vec<ResponseOutputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ResponseReasoningInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ResponseUsage>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ResponseOutputItem {
    Reasoning {
        id: String,
        /// ⚠ `Vec<Value>`, not `Vec<String>`. A reasoning summary arrives as
        /// `[{"type":"summary_text","text":…}]` — objects, not strings — so the
        /// stricter type failed the *whole response* the moment a summary was
        /// present. `#[serde(other)]` below cannot save it: the tag
        /// (`reasoning`) is one this decoder models, so serde commits to this
        /// variant and then errors on the body. Nothing reads the summary; the
        /// item is skipped wholesale by `responses_api_to_message`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<Vec<Value>>,
    },
    Message {
        id: String,
        status: String,
        role: String,
        content: Vec<ResponseContentBlock>,
    },
    FunctionCall {
        id: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
        name: String,
        arguments: String,
    },
    /// Any output-item `type` this decoder does not model.
    ///
    /// The non-streaming twin of [`ResponseOutputItemInfo::Unknown`], and it
    /// matters for the same reason: `responses_api_to_message` is called on the
    /// whole response, so one built-in tool call in `output[]` — `web_search_call`,
    /// `mcp_call`, `code_interpreter_call`, `image_generation_call`,
    /// `custom_tool_call` — turned an otherwise usable answer into a decode
    /// error. An unmodelled item contributes no content, exactly as a
    /// `Reasoning` item already did.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ResponseContentBlock {
    OutputText {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<Vec<Value>>,
    },
    ToolCall {
        id: String,
        name: String,
        input: Value,
    },
    /// Any content-block `type` this decoder does not model — `refusal` above
    /// all. The non-streaming twin of [`ContentPart::Unknown`], and closed for
    /// the same reason it was: `#[serde(other)]` on the enclosing item fires
    /// only when the *item's* tag is unknown, so a `message` item carrying a
    /// refusal block selects `Message` and then fails one level down.
    ///
    /// Like the streaming half, an unmodelled block contributes no content —
    /// a refusal reaches the user as absent text rather than as an error, and
    /// surfacing refusal text is deliberately a separate change.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseReasoningInfo {
    pub effort: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseUsage {
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub total_tokens: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens_details: Option<ResponseInputTokensDetails>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseInputTokensDetails {
    pub cached_tokens: i32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ResponsesStreamEvent {
    #[serde(rename = "response.created")]
    ResponseCreated {
        sequence_number: i32,
        response: ResponseMetadata,
    },
    #[serde(rename = "response.in_progress")]
    ResponseInProgress {
        sequence_number: i32,
        response: ResponseMetadata,
    },
    #[serde(rename = "response.output_item.added")]
    OutputItemAdded {
        sequence_number: i32,
        output_index: i32,
        item: ResponseOutputItemInfo,
    },
    #[serde(rename = "response.content_part.added")]
    ContentPartAdded {
        sequence_number: i32,
        item_id: String,
        output_index: i32,
        content_index: i32,
        part: ContentPart,
    },
    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta {
        sequence_number: i32,
        item_id: String,
        output_index: i32,
        content_index: i32,
        delta: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        logprobs: Option<Vec<Value>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        obfuscation: Option<String>,
    },
    #[serde(rename = "response.output_item.done")]
    OutputItemDone {
        sequence_number: i32,
        output_index: i32,
        item: ResponseOutputItemInfo,
    },
    #[serde(rename = "response.content_part.done")]
    ContentPartDone {
        sequence_number: i32,
        item_id: String,
        output_index: i32,
        content_index: i32,
        part: ContentPart,
    },
    #[serde(rename = "response.output_text.done")]
    OutputTextDone {
        sequence_number: i32,
        item_id: String,
        output_index: i32,
        content_index: i32,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        logprobs: Option<Vec<Value>>,
    },
    #[serde(rename = "response.completed")]
    ResponseCompleted {
        sequence_number: i32,
        response: ResponseMetadata,
    },
    /// How a stream ends when it hit a cap instead of finishing — in practice
    /// the `max_output_tokens` **we ourselves set** from
    /// `model_config.max_tokens`, and otherwise a content filter.
    ///
    /// ⚠ **Named, not left to the open arm below.** It carries the same payload
    /// `response.completed` does — the usage for the turn and the terminal
    /// `output[]` — so folding it into `Unknown` decoded the tag without
    /// aborting the turn (which is what #147 asked for) while silently
    /// discarding every token count and every output item that appeared only in
    /// this frame. The two fixes are orthogonal: the open arm keeps unmodelled
    /// tags cheap, this variant keeps a *modelled* terminal frame's data.
    ///
    /// ⚠ **Lenient where `response.completed` is strict**, and deliberately:
    /// `response` is `Option` and a body this decoder cannot read degrades to
    /// `None` rather than failing (see [`lenient_response_metadata`]). A
    /// truncation frame that cannot be parsed must cost the usage, never the
    /// answer that already streamed — that is the failure this whole variant
    /// exists to prevent. `response.completed` keeps its hard failure because
    /// there it is the *only* carrier of the response.
    #[serde(rename = "response.incomplete")]
    ResponseIncomplete {
        #[serde(default)]
        sequence_number: i32,
        #[serde(default, deserialize_with = "lenient_response_metadata")]
        response: Option<ResponseMetadata>,
    },
    #[serde(rename = "response.failed")]
    ResponseFailed { sequence_number: i32, error: Value },
    #[serde(rename = "response.function_call_arguments.delta")]
    FunctionCallArgumentsDelta {
        sequence_number: i32,
        item_id: String,
        output_index: i32,
        delta: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        obfuscation: Option<String>,
    },
    #[serde(rename = "response.function_call_arguments.done")]
    FunctionCallArgumentsDone {
        sequence_number: i32,
        item_id: String,
        output_index: i32,
        arguments: String,
    },
    #[serde(rename = "error")]
    Error { error: Value },
    /// Any `type` this decoder does not model.
    ///
    /// ⚠ **Not defensive tidiness — without it a single unmodelled tag costs
    /// the whole turn.** `parse_stream_event` is called with `?` inside the
    /// reader, [`stream_responses_api`] (the decoder `openai.rs` and `azure.rs`
    /// share) turns that error into
    /// `RequestFailed("Stream decode error: …")`, and `with_retry` wraps only
    /// the POST, never stream consumption — so nothing retries and every delta
    /// already yielded is discarded along with the response.
    ///
    /// The tag that made this concrete is `response.incomplete`, which is how
    /// the API ends a stream that hit the `max_output_tokens` **we ourselves
    /// set** from `model_config.max_tokens` — i.e. a routine, self-inflicted
    /// cap turned an otherwise complete answer into a failed request. ⚠ That
    /// tag is **no longer covered here**: it carries the turn's usage and its
    /// terminal `output[]`, so decoding it to `Unknown` cost exactly the data
    /// the report was about, and it now has its own
    /// [`ResponsesStreamEvent::ResponseIncomplete`] variant. What is left for
    /// this arm is tags that carry nothing this crate consumes —
    /// `response.refusal.delta` / `.done`, and the event types the Responses
    /// API keeps adding for built-in tools and reasoning summaries.
    ///
    /// Inert by construction: `apply_stream_event` already ends in a catch-all
    /// `_ =>` arm, so an unknown event advances nothing and yields nothing —
    /// the stream simply runs on to `[DONE]` or end of input and emits whatever
    /// terminal item it had accumulated. `response.failed` and `error` keep
    /// their own explicit arms, so a real error is still a hard failure; only
    /// tags nobody modelled become silence.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseMetadata {
    pub id: String,
    pub object: String,
    pub created_at: i64,
    pub status: String,
    pub model: String,
    pub output: Vec<ResponseOutputItemInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ResponseUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ResponseReasoningInfo>,
    /// Why a `response.incomplete` frame ended the stream. Absent on every
    /// other frame, which is why it is defaulted rather than required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incomplete_details: Option<IncompleteDetails>,
}

/// The `incomplete_details` object on a truncated response.
///
/// `reason` is optional because the mapping below must survive the API adding a
/// shape this decoder has not seen — a truncation frame is the last thing that
/// should fail to decode.
#[derive(Debug, Serialize, Deserialize)]
pub struct IncompleteDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Decode a `response` object, or `None` if it cannot be read.
///
/// ⚠ Only ever used by [`ResponsesStreamEvent::ResponseIncomplete`]. Serde's
/// own `Option` handling covers a missing or null field and nothing else — a
/// `response` object with, say, no `id` is still a hard error — so leniency
/// here has to be written out: decode to a `Value` first, then try the real
/// type and swallow the failure. The cost of a `None` is the turn's usage; the
/// cost of the error it replaces is the whole answer.
fn lenient_response_metadata<'de, D>(deserializer: D) -> Result<Option<ResponseMetadata>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| serde_json::from_value(value).ok()))
}

/// The OpenAI-style `finish_reason` a truncated response reports.
///
/// `max_output_tokens` becomes `"length"` — the one token the agent loop acts
/// on (`TRUNCATION_CONTINUATION_MESSAGE` in `agents/agent.rs`), and the same
/// mapping `formats::anthropic` and `formats::bedrock` already make for their
/// own cap. ⚠ Every other reason passes through raw: `content_filter` is not a
/// length problem, and auto-continuing one would re-ask for the content that
/// was just refused.
fn incomplete_finish_reason(details: Option<&IncompleteDetails>) -> String {
    match details.and_then(|details| details.reason.as_deref()) {
        Some("max_output_tokens") => "length".to_string(),
        Some(reason) => reason.to_string(),
        None => "incomplete".to_string(),
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ResponseOutputItemInfo {
    Reasoning {
        id: String,
        /// ⚠ `Vec<Value>` and defaulted, for the reason spelled out on
        /// [`ResponseOutputItem::Reasoning`]: a summary arrives as
        /// `[{"type":"summary_text","text":…}]`, so `Vec<String>` failed every
        /// frame carrying one — latent only because `create_responses_request`
        /// never asks for summaries. The `Unknown` arm below cannot cover it;
        /// `reasoning` is a tag this decoder models.
        #[serde(default)]
        summary: Vec<Value>,
    },
    Message {
        id: String,
        status: String,
        role: String,
        content: Vec<ContentPart>,
    },
    FunctionCall {
        id: String,
        status: String,
        call_id: String,
        name: String,
        arguments: String,
    },
    /// Any output-item `type` this decoder does not model.
    ///
    /// ⚠ The enum-level arm on [`ResponsesStreamEvent`] does not reach here,
    /// for the same reason it does not reach [`ContentPart`]: an item arrives
    /// inside `response.output_item.added` / `.done` and inside
    /// `response.completed`'s `output[]`, all tags this decoder knows, so serde
    /// commits to those variants and then fails on the nested item. Every
    /// built-in tool call — `web_search_call`, `mcp_call`,
    /// `code_interpreter_call`, `image_generation_call`, `custom_tool_call` —
    /// is one of these, which is why the built-in tools the event arm's comment
    /// cites were **not** in fact covered by it.
    ///
    /// An unknown item produces no content and no pending call, so it can
    /// neither add nor remove anything. It also reports no id
    /// (see [`output_item_id`]), so it can never confirm a modelled item.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ContentPart {
    OutputText {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<Vec<Value>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        logprobs: Option<Vec<Value>>,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    /// Any part `type` this decoder does not model — `refusal` above all.
    ///
    /// ⚠ The enum-level arm on [`ResponsesStreamEvent`] cannot cover this, and
    /// that is easy to get wrong: `#[serde(other)]` fires only when the *tag*
    /// matches no variant. A `response.content_part.added` whose `part` is
    /// `{"type":"refusal",…}` carries a tag this decoder knows, so it selects
    /// `ContentPartAdded` and then fails on the nested part — the same
    /// whole-turn loss, one level down. A refusal also reappears inside
    /// `response.completed`'s `output[].content[]`, which is `Vec<ContentPart>`.
    ///
    /// Neither consumer can lose anything to this arm. `pending_output_item`
    /// looks only for `ToolCall` parts; `process_streaming_output_items` reads
    /// `OutputText` as well (`content.push(MessageContent::text(&text))`), and
    /// an unmodelled part is neither — so it contributes nothing and removes
    /// nothing. ⚠ This paragraph read "both consumers already ignore every part
    /// that is not a `ToolCall`", which is false of the text arm; the
    /// conclusion held, the premise did not. It does mean a refusal reaches the
    /// user as absent text rather than as an error; surfacing refusal text is a
    /// separate change and is deliberately not made here.
    #[serde(other)]
    Unknown,
}

/// The text and user images of one message, as one Responses role item, or
/// `None` when it carries neither (a message holding only tool traffic).
fn role_item(message: &Message) -> Option<Value> {
    let role = match message.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };

    let mut content_items = Vec::new();
    for content in &message.content {
        match content {
            MessageContent::Text(text) if !text.text.is_empty() => {
                let content_type = if message.role == Role::Assistant {
                    "output_text"
                } else {
                    "input_text"
                };
                content_items.push(json!({
                    "type": content_type,
                    "text": text.text
                }));
            }
            MessageContent::Image(image) if message.role == Role::User => {
                // Responses API user-message image format: data URL inline.
                // Assistant messages don't carry image inputs.
                content_items.push(json!({
                    "type": "input_image",
                    "image_url": format!(
                        "data:{};base64,{}",
                        image.mime_type, image.data
                    ),
                }));
            }
            _ => {}
        }
    }

    (!content_items.is_empty()).then(|| {
        json!({
            "role": role,
            "content": content_items
        })
    })
}

fn function_call_item(request: &ToolRequest) -> Option<Value> {
    let tool_call = request.tool_call.as_ref().ok()?;
    let arguments_str = tool_call
        .arguments
        .as_ref()
        .map(|args| serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string()))
        .unwrap_or_else(|| "{}".to_string());

    tracing::debug!(
        "Replaying function_call with call_id: {}, name: {}",
        request.id,
        tool_call.name
    );
    Some(json!({
        "type": "function_call",
        "call_id": request.id,
        "name": tool_call.name,
        "arguments": arguments_str
    }))
}

/// Said in a tool's output where it returned an image, which follows as a user
/// item — the wording `formats::openai::format_messages` has always used.
const TOOL_IMAGE_PLACEHOLDER: &str =
    "This tool result included an image that is uploaded in the next message.";

/// The one `function_call_output` a tool response owes its call. Each image the
/// tool addressed to the model is replaced by [`TOOL_IMAGE_PLACEHOLDER`] and
/// pushed onto `images` as an `input_image`, for the caller to send in a user
/// item after the outputs.
///
/// ⚠ An image used to vanish here: `audience::flattened_text` has no text for
/// one, so a screenshot or a rendered figure reached the model as an empty
/// output while Chat Completions forwarded it. Only a `function_call_output`'s
/// `output` string is accepted in this position, hence the separate user item.
fn function_call_output_item(response: &ToolResponse, images: &mut Vec<Value>) -> Value {
    match &response.tool_result {
        Ok(contents) => {
            let mut text_content: Vec<String> = Vec::new();
            // Send only what the tool addressed to the model.
            for content in contents
                .content
                .iter()
                .filter(|c| audience::is_for_model(c))
            {
                if let RawContent::Image(image) = &content.raw {
                    text_content.push(TOOL_IMAGE_PLACEHOLDER.to_string());
                    images.push(json!({
                        "type": "input_image",
                        "image_url": format!("data:{};base64,{}", image.mime_type, image.data),
                    }));
                } else if let Some(text) = audience::flattened_text(content) {
                    text_content.push(text);
                }
            }

            // Emitted even when nothing survives the filter. The Err
            // arm below already knows why: a `function_call` with no
            // matching output is the "No tool output found" error,
            // and the Responses API takes an empty string happily.
            // A tool that addresses every block to the user alone
            // used to skip this push and fail the whole request one
            // turn later.
            tracing::debug!("Sending function_call_output with call_id: {}", response.id);
            json!({
                "type": "function_call_output",
                "call_id": response.id,
                "output": text_content.join("\n")
            })
        }
        Err(error_data) => {
            // Handle error responses - must send them back to the API
            // to avoid "No tool output found" errors
            tracing::debug!(
                "Sending function_call_output error with call_id: {}",
                response.id
            );
            json!({
                "type": "function_call_output",
                "call_id": response.id,
                "output": format!("Error: {}", error_data.message)
            })
        }
    }
}

/// The conversation as Responses input items, in the order it happened.
///
/// ⚠ This was three passes over the whole history — every text message, then
/// every `function_call`, then every `function_call_output` — so a multi-turn
/// tool conversation reached the model with earlier turns' calls after the
/// latest user message, each cut off from the text that announced it. The API
/// accepts any order, so nothing failed; the model read a scrambled transcript.
/// It mattered more from 2026-09-25, when Azure's GPT-5.4/5.5 moved here from
/// Chat Completions, whose `format_messages` was always chronological.
///
/// Within one message: an assistant's text precedes the calls it announced; a
/// user message answers the previous turn's calls first (outputs, then the
/// images they returned) and any text it carries was written after the tool
/// ran, so it comes last. Tool images never sit between a call and its output.
fn add_conversation_items(input_items: &mut Vec<Value>, messages: &[Message]) {
    for message in messages.iter().filter(|m| m.is_agent_visible()) {
        let mut text = role_item(message);
        let mut calls = Vec::new();
        let mut outputs = Vec::new();
        let mut tool_images = Vec::new();

        for content in &message.content {
            match content {
                MessageContent::ToolRequest(request) if message.role == Role::Assistant => {
                    calls.extend(function_call_item(request));
                }
                MessageContent::ToolResponse(response) => {
                    outputs.push(function_call_output_item(response, &mut tool_images));
                }
                _ => {}
            }
        }

        if message.role == Role::Assistant {
            input_items.extend(text.take());
        }
        input_items.extend(calls);
        input_items.extend(outputs);
        if !tool_images.is_empty() {
            input_items.push(json!({
                "role": "user",
                "content": tool_images
            }));
        }
        input_items.extend(text);
    }
}

pub fn create_responses_request(
    model_config: &ModelConfig,
    system: &str,
    messages: &[Message],
    tools: &[Tool],
) -> anyhow::Result<Value, Error> {
    let mut input_items = Vec::new();

    if !system.is_empty() {
        input_items.push(json!({
            "role": "system",
            "content": [{
                "type": "input_text",
                "text": system
            }]
        }));
    }

    add_conversation_items(&mut input_items, messages);

    let mut payload = json!({
        "model": model_config.model_name,
        "input": input_items,
        "store": false,  // Don't store responses on server (we replay history ourselves)
    });

    if !tools.is_empty() {
        let tools_spec: Vec<Value> = tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                })
            })
            .collect();

        payload
            .as_object_mut()
            .unwrap()
            .insert("tools".to_string(), json!(tools_spec));
    }

    // A reasoning model refuses `temperature` (and `top_p`, which this builder
    // never sends): GPT-6 whenever the effort is not `none`, and the GPT-5.x and
    // o-series models BioRouter routes here outright. The Chat Completions
    // builder has always dropped it for these models (`is_ox_model` in
    // `formats::openai::create_request`); this one inserted it for any model
    // with a configured temperature, so every Responses-routed turn of a chat
    // with one set would 400. Dropped rather than refused, exactly as the chat
    // builder does — the setting is a preference the model cannot honour.
    if let Some(temp) = model_config.temperature {
        if !model_supports_reasoning_effort(&model_config.model_name) {
            payload
                .as_object_mut()
                .unwrap()
                .insert("temperature".to_string(), json!(temp));
        }
    }

    if let Some(tokens) = model_config.max_tokens {
        payload
            .as_object_mut()
            .unwrap()
            .insert("max_output_tokens".to_string(), json!(tokens));
    }

    // BR-63: the Responses API takes the effort as `reasoning.effort`, not the
    // chat-completions top-level `reasoning_effort`. Omitted entirely unless the
    // turn pinned an effort, so the model's own default is untouched.
    if let Some(effort) = model_config
        .reasoning_effort
        .and_then(|effort| effort.provider_effort())
        .and_then(|effort| model_reasoning_effort(&model_config.model_name, effort))
    {
        payload
            .as_object_mut()
            .unwrap()
            .insert("reasoning".to_string(), json!({ "effort": effort }));
    }

    Ok(payload)
}

pub fn responses_api_to_message(response: &ResponsesApiResponse) -> anyhow::Result<Message> {
    let mut content = Vec::new();

    for item in &response.output {
        match item {
            ResponseOutputItem::Reasoning { .. } => {
                continue;
            }
            // An item type this decoder does not model — a built-in tool call,
            // say. Spelled out rather than folded into a `_` arm so a genuinely
            // new item type still has to be considered here.
            ResponseOutputItem::Unknown => {
                continue;
            }
            ResponseOutputItem::Message {
                content: msg_content,
                ..
            } => {
                for block in msg_content {
                    match block {
                        ResponseContentBlock::OutputText { text, .. } => {
                            if !text.is_empty() {
                                content.push(MessageContent::text(text));
                            }
                        }
                        // A block this decoder does not model — a refusal, say.
                        // Contributes nothing, exactly as its streaming twin
                        // `ContentPart::Unknown` does.
                        ResponseContentBlock::Unknown => {}
                        ResponseContentBlock::ToolCall { id, name, input } => {
                            content.push(MessageContent::tool_request(
                                id.clone(),
                                Ok(CallToolRequestParams {
                                    task: None,
                                    name: name.clone().into(),
                                    arguments: Some(object(input.clone())),
                                    meta: None,
                                }),
                            ));
                        }
                    }
                }
            }
            ResponseOutputItem::FunctionCall {
                id,
                name,
                arguments,
                ..
            } => {
                tracing::debug!("Received FunctionCall with id: {}, name: {}", id, name);
                let parsed_args = if arguments.is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(arguments).unwrap_or_else(|_| json!({}))
                };

                content.push(MessageContent::tool_request(
                    id.clone(),
                    Ok(CallToolRequestParams {
                        task: None,
                        name: name.clone().into(),
                        arguments: Some(object(parsed_args)),
                        meta: None,
                    }),
                ));
            }
        }
    }

    let mut message = Message::new(Role::Assistant, chrono::Utc::now().timestamp(), content);

    message = message.with_id(response.id.clone());

    Ok(message)
}

pub fn get_responses_usage(response: &ResponsesApiResponse) -> Usage {
    response
        .usage
        .as_ref()
        .map_or_else(Usage::default, response_usage_to_usage)
}

fn response_usage_to_usage(usage: &ResponseUsage) -> Usage {
    let cached_tokens = usage
        .input_tokens_details
        .as_ref()
        .map(|details| details.cached_tokens)
        .filter(|cached| *cached > 0)
        .map(|cached| cached.min(usage.input_tokens.max(0)));
    let input_tokens = usage.input_tokens - cached_tokens.unwrap_or(0);

    Usage::new(
        Some(input_tokens),
        Some(usage.output_tokens),
        Some(usage.total_tokens),
    )
    .with_cache(cached_tokens, None)
}

fn process_streaming_output_items(
    output_items: Vec<(ResponseOutputItemInfo, bool)>,
    is_text_response: bool,
) -> Vec<MessageContent> {
    let mut content = Vec::new();

    for (item, completion_confirmed) in output_items {
        match item {
            ResponseOutputItemInfo::Reasoning { .. } => {
                // Skip reasoning items
            }
            // An item type this decoder does not model — a built-in tool call,
            // say. Contributes no content, like a reasoning item.
            ResponseOutputItemInfo::Unknown => {}
            ResponseOutputItemInfo::Message {
                status,
                content: parts,
                ..
            } => {
                let completion_confirmed = completion_confirmed && status == "completed";
                for part in parts {
                    match part {
                        ContentPart::OutputText { text, .. } => {
                            if !text.is_empty() && !is_text_response {
                                content.push(MessageContent::text(&text));
                            }
                        }
                        ContentPart::ToolCall {
                            id,
                            name,
                            arguments,
                        } => {
                            content.push(streaming_tool_content(
                                id,
                                name,
                                &arguments,
                                completion_confirmed,
                            ));
                        }
                        // An unmodelled part (a refusal, say) contributes no
                        // content. Spelled out rather than folded into a `_`
                        // arm so a genuinely new part type still has to be
                        // considered here.
                        ContentPart::Unknown => {}
                    }
                }
            }
            ResponseOutputItemInfo::FunctionCall {
                status,
                call_id,
                name,
                arguments,
                ..
            } => {
                content.push(streaming_tool_content(
                    call_id,
                    name,
                    &arguments,
                    completion_confirmed && status == "completed",
                ));
            }
        }
    }

    content
}

fn streaming_tool_content(
    call_id: String,
    name: String,
    arguments: &str,
    completion_confirmed: bool,
) -> MessageContent {
    if !completion_confirmed {
        return MessageContent::tool_request(
            call_id,
            Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "Tool-call stream completion was not confirmed; the call was not executed. Emit a new, complete tool call.",
                Some(json!({"biorouterToolCallFailure":"incomplete_stream"})),
            )),
        );
    }

    let parsed = if arguments.trim().is_empty() {
        Ok(json!({}))
    } else {
        serde_json::from_str::<Value>(arguments).map_err(|error| {
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
            call_id,
            Ok(CallToolRequestParams {
                task: None,
                name: name.into(),
                arguments: Some(object(arguments)),
                meta: None,
            }),
        ),
        Err(error) => MessageContent::tool_request(call_id, Err(error)),
    }
}

/// The item's own id, or `""` for an item type this decoder does not model.
///
/// ⚠ The empty string is load-bearing: [`confirm_output_item`] compares this
/// against the id of whatever is already tracked at that output index, and the
/// API never issues an empty id — so an unmodelled item can never confirm a
/// modelled one. It fails in the safe direction, leaving a half-streamed tool
/// call reported as incomplete rather than promoted to callable.
fn output_item_id(item: &ResponseOutputItemInfo) -> &str {
    match item {
        ResponseOutputItemInfo::Reasoning { id, .. }
        | ResponseOutputItemInfo::Message { id, .. }
        | ResponseOutputItemInfo::FunctionCall { id, .. } => id,
        ResponseOutputItemInfo::Unknown => "",
    }
}

struct PendingFunctionCall {
    call_id: String,
    name: String,
}

struct PendingOutputItem {
    item_id: String,
    calls: BTreeMap<i32, PendingFunctionCall>,
}

enum StreamingOutputItemState {
    Pending(PendingOutputItem),
    Unconfirmed(ResponseOutputItemInfo),
    Completed(ResponseOutputItemInfo),
}

/// The mutable state one Responses stream accumulates while it is read.
///
/// Every field here is written by an event and read either by a later event or
/// by [`ResponsesStreamState::into_final_item`]; nothing in it is scratch.
#[derive(Default)]
struct ResponsesStreamState {
    response_id: Option<String>,
    model_name: Option<String>,
    final_usage: Option<ProviderUsage>,
    output_items: BTreeMap<i32, StreamingOutputItemState>,
    is_text_response: bool,
}

impl ResponsesStreamState {
    /// The single terminal item the stream owes its consumer, or `None` when it
    /// produced neither content nor usage.
    ///
    /// Content wins: a non-empty message carries whatever usage arrived, which
    /// is `None` for a stream that ended at `[DONE]` without a
    /// `response.completed`. Usage with no content is yielded on its own.
    fn into_final_item(self) -> Option<(Option<Message>, Option<ProviderUsage>)> {
        let content = final_stream_content(self.output_items, self.is_text_response);

        if !content.is_empty() {
            let mut message =
                Message::new(Role::Assistant, chrono::Utc::now().timestamp(), content);
            if let Some(id) = self.response_id {
                message = message.with_id(id);
            }
            return Some((Some(message), self.final_usage));
        }

        self.final_usage.map(|usage| (None, Some(usage)))
    }
}

/// Whether the reader keeps consuming frames after an event.
enum EventFlow {
    Continue,
    Stop,
}

/// The payload of one SSE line, or `None` for a line that carries no event.
///
/// `data:` may or may not be followed by a space, and a line with no field
/// prefix at all is handed back as-is so a provider that omits it still decodes.
fn stream_frame_payload(line: &str) -> Option<&str> {
    // Skip empty lines
    if line.trim().is_empty() {
        return None;
    }

    if let Some(data) = line.strip_prefix("data:") {
        Some(data.trim_start())
    } else if line.starts_with("event:") {
        // Skip event type lines
        None
    } else {
        // Try to parse as-is in case there's no prefix
        Some(line)
    }
}

/// Decode one SSE payload into an event.
///
/// ⚠ The error must never repeat the payload: a malformed frame can hold raw
/// model output, and this error travels all the way to the user.
fn parse_stream_event(data_line: &str) -> anyhow::Result<ResponsesStreamEvent> {
    serde_json::from_str(data_line)
        .map_err(|error| anyhow!("Failed to parse Responses stream event: {error}"))
}

/// The message yielded for one `response.output_text.delta`.
///
/// An empty delta still yields a message — the frame is a liveness signal — and
/// the response id is attached so the desktop client folds every delta into one
/// message rather than rendering a message per token.
fn text_delta_message(delta: &str, response_id: Option<&str>) -> Message {
    let mut content = Vec::new();
    if !delta.is_empty() {
        content.push(MessageContent::text(delta));
    }
    let mut message = Message::new(Role::Assistant, chrono::Utc::now().timestamp(), content);

    // Add ID so desktop client knows these deltas are part of the same message
    if let Some(id) = response_id {
        message = message.with_id(id);
    }

    message
}

/// The pending tool calls a `response.output_item.added` announces, if any.
///
/// Reasoning items and text-only messages hold nothing that could become a tool
/// call, so they register no pending state: the pending map exists only to
/// report *calls* the stream never finished.
fn pending_output_item(item: ResponseOutputItemInfo) -> Option<PendingOutputItem> {
    match item {
        ResponseOutputItemInfo::FunctionCall {
            id, call_id, name, ..
        } => Some(PendingOutputItem {
            item_id: id,
            calls: [(0, PendingFunctionCall { call_id, name })]
                .into_iter()
                .collect(),
        }),
        ResponseOutputItemInfo::Message { id, content, .. } => {
            let calls = content
                .into_iter()
                .enumerate()
                .filter_map(|part| match part {
                    (content_index, ContentPart::ToolCall { id, name, .. }) => {
                        i32::try_from(content_index).ok().map(|content_index| {
                            (content_index, PendingFunctionCall { call_id: id, name })
                        })
                    }
                    (_, ContentPart::OutputText { .. } | ContentPart::Unknown) => None,
                })
                .collect::<BTreeMap<_, _>>();
            (!calls.is_empty()).then_some(PendingOutputItem { item_id: id, calls })
        }
        // Neither a reasoning item nor an unmodelled one can hold a tool call,
        // so neither registers pending state.
        ResponseOutputItemInfo::Reasoning { .. } | ResponseOutputItemInfo::Unknown => None,
    }
}

/// Record a tool call that arrived as a later `response.content_part.added`.
///
/// ⚠ An occupied index is only extended when it is still `Pending` *and* names
/// the same item. A part belonging to some other item must not edit the entry
/// sitting at this output index, and a `Completed` entry is a confirmed
/// snapshot that nothing arriving later may touch.
fn record_tool_call_part(
    output_items: &mut BTreeMap<i32, StreamingOutputItemState>,
    output_index: i32,
    item_id: String,
    content_index: i32,
    call: PendingFunctionCall,
) {
    match output_items.entry(output_index) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(StreamingOutputItemState::Pending(PendingOutputItem {
                item_id,
                calls: [(content_index, call)].into_iter().collect(),
            }));
        }
        std::collections::btree_map::Entry::Occupied(mut entry) => {
            if let StreamingOutputItemState::Pending(pending) = entry.get_mut() {
                if pending.item_id == item_id {
                    pending.calls.entry(content_index).or_insert(call);
                }
            }
        }
    }
}

/// Store the confirmed snapshot a `response.output_item.done` carries.
///
/// ⚠ A done event confirms only the item it names. A mismatched id, or a second
/// done for an index already `Completed`, is dropped rather than overwriting —
/// otherwise an unrelated item would silently make an unfinished tool call look
/// executable.
fn confirm_output_item(
    output_items: &mut BTreeMap<i32, StreamingOutputItemState>,
    output_index: i32,
    item: ResponseOutputItemInfo,
) {
    let item_id = output_item_id(&item);
    let completion_matches = match output_items.get(&output_index) {
        None => true,
        Some(StreamingOutputItemState::Pending(pending)) => pending.item_id == item_id,
        Some(StreamingOutputItemState::Unconfirmed(existing)) => {
            output_item_id(existing) == item_id
        }
        Some(StreamingOutputItemState::Completed(_)) => false,
    };

    if completion_matches {
        output_items.insert(output_index, StreamingOutputItemState::Completed(item));
    }
}

/// Fold a terminal response frame into the output map and report its usage.
///
/// Called for `response.completed` and for `response.incomplete` alike — both
/// carry the turn's usage and its terminal `output[]`. The caller adds the
/// finish reason, which is the only thing that differs between them.
///
/// ⚠ The final response's own items are recorded as `Unconfirmed`, never
/// `Completed`, and only where nothing is tracked yet: a terminal frame
/// restates items the stream may never have finished, so it must not stand in
/// for the missing `output_item.done` that would make a partial tool call
/// callable, nor replace a snapshot an earlier done already confirmed. That
/// matters more for a truncated response, not less: its `output[]` is by
/// definition the place a half-finished tool call shows up.
fn absorb_completed_response(
    response: ResponseMetadata,
    model_name: Option<&str>,
    output_items: &mut BTreeMap<i32, StreamingOutputItemState>,
) -> ProviderUsage {
    let provider_usage = ProviderUsage {
        usage: response
            .usage
            .as_ref()
            .map_or_else(Usage::default, response_usage_to_usage),
        model: model_name.unwrap_or(&response.model).to_string(),
        provider: None,
        finish_reason: None,
    };

    for (output_index, item) in response.output.into_iter().enumerate() {
        if let Ok(output_index) = i32::try_from(output_index) {
            output_items
                .entry(output_index)
                .or_insert(StreamingOutputItemState::Unconfirmed(item));
        }
    }

    provider_usage
}

/// The content of the one terminal message, in output-index order.
///
/// ⚠ Only a `Completed` item can produce a callable tool request. A `Pending`
/// item's calls are drained as explicit incomplete-stream failures rather than
/// dropped, so a truncated stream tells the model its call did not run instead
/// of losing the call entirely.
fn final_stream_content(
    output_items: BTreeMap<i32, StreamingOutputItemState>,
    is_text_response: bool,
) -> Vec<MessageContent> {
    let mut content = Vec::new();

    for item in output_items.into_values() {
        match item {
            StreamingOutputItemState::Pending(pending) => {
                content.extend(
                    pending
                        .calls
                        .into_values()
                        .map(|call| streaming_tool_content(call.call_id, call.name, "", false)),
                );
            }
            StreamingOutputItemState::Unconfirmed(item) => {
                content.extend(process_streaming_output_items(
                    vec![(item, false)],
                    is_text_response,
                ));
            }
            StreamingOutputItemState::Completed(item) => {
                content.extend(process_streaming_output_items(
                    vec![(item, true)],
                    is_text_response,
                ));
            }
        }
    }

    content
}

/// Apply one decoded event to the stream state.
///
/// ⚠ `response.output_text.delta` is deliberately not handled here. It is the
/// only event that emits anything mid-stream, so the reader intercepts it and
/// every `yield` stays visible in one place; it reaches this function only if
/// that interception is removed, and is a no-op here as it was under the
/// catch-all arm.
fn apply_stream_event(
    event: ResponsesStreamEvent,
    state: &mut ResponsesStreamState,
) -> anyhow::Result<EventFlow> {
    match event {
        ResponsesStreamEvent::ResponseCreated { response, .. }
        | ResponsesStreamEvent::ResponseInProgress { response, .. } => {
            state.response_id = Some(response.id);
            state.model_name = Some(response.model);
        }

        ResponsesStreamEvent::OutputItemAdded {
            output_index, item, ..
        } => {
            if let Some(pending) = pending_output_item(item) {
                state
                    .output_items
                    .entry(output_index)
                    .or_insert(StreamingOutputItemState::Pending(pending));
            }
        }

        ResponsesStreamEvent::ContentPartAdded {
            item_id,
            output_index,
            content_index,
            part: ContentPart::ToolCall { id, name, .. },
            ..
        } => {
            record_tool_call_part(
                &mut state.output_items,
                output_index,
                item_id,
                content_index,
                PendingFunctionCall { call_id: id, name },
            );
        }

        ResponsesStreamEvent::OutputItemDone {
            output_index, item, ..
        } => {
            confirm_output_item(&mut state.output_items, output_index, item);
        }

        ResponsesStreamEvent::OutputTextDone { .. } => {
            // Text is already complete from deltas, this is just a summary event
        }

        ResponsesStreamEvent::ResponseCompleted { response, .. } => {
            state.final_usage = Some(absorb_completed_response(
                response,
                state.model_name.as_deref(),
                &mut state.output_items,
            ));

            return Ok(EventFlow::Stop);
        }

        // The truncated twin of the arm above, and it MUST do the same work:
        // `response.incomplete` carries this turn's usage and its terminal
        // `output[]` exactly as `response.completed` does. It is also terminal,
        // hence the same `Stop`.
        //
        // The one difference is the finish reason. A cap we set ourselves maps
        // to `"length"`, which is what makes the agent loop continue the answer
        // instead of presenting a sentence that stops mid-word.
        ResponsesStreamEvent::ResponseIncomplete { response, .. } => {
            if let Some(response) = response {
                let finish_reason = incomplete_finish_reason(response.incomplete_details.as_ref());
                let mut usage = absorb_completed_response(
                    response,
                    state.model_name.as_deref(),
                    &mut state.output_items,
                );
                usage.finish_reason = Some(finish_reason);
                state.final_usage = Some(usage);
            }

            return Ok(EventFlow::Stop);
        }

        ResponsesStreamEvent::FunctionCallArgumentsDelta { .. } => {
            // Function call arguments are being streamed, but we'll get the complete
            // arguments in the OutputItemDone event, so we can ignore deltas for now
        }

        ResponsesStreamEvent::FunctionCallArgumentsDone { .. } => {
            // Arguments are complete, will be in the OutputItemDone event
        }

        ResponsesStreamEvent::ResponseFailed { error, .. } => {
            return Err(anyhow!("Responses API failed: {:?}", error));
        }

        ResponsesStreamEvent::Error { error } => {
            return Err(anyhow!("Responses API error: {:?}", error));
        }

        _ => {
            // Ignore remaining non-tool progress events.
        }
    }

    Ok(EventFlow::Continue)
}

/// Read an SSE stream from the Responses API into provider stream items.
///
/// The reader owns all three yield points: a text delta mid-stream, and the one
/// terminal item. Everything else is state kept in [`ResponsesStreamState`] and
/// only settled once the stream ends — at `[DONE]`, at `response.completed`, or
/// at end of input — because a tool call is callable only when the stream said
/// so explicitly.
pub fn responses_api_to_streaming_message<S>(
    mut stream: S,
) -> impl Stream<Item = anyhow::Result<crate::providers::base::ProviderStreamItem>> + 'static
where
    S: Stream<Item = anyhow::Result<String>> + Unpin + Send + 'static,
{
    try_stream! {
        use futures::StreamExt;

        // ⚠ There is no `accumulated_text` here on purpose. One used to be
        // push_str'd on every delta and never read: text reaches the caller as
        // the deltas are yielded, and the terminal message is built from
        // `state.output_items`. A write-only buffer beside the real state reads
        // like a second source of truth and invites a "fix" that starts using
        // it, which would duplicate every token.
        let mut state = ResponsesStreamState::default();

        'outer: while let Some(response) = stream.next().await {
            let response_str = response?;

            let data_line = match stream_frame_payload(&response_str) {
                Some(data_line) => data_line,
                None => continue,
            };

            if data_line.trim() == "[DONE]" {
                break 'outer;
            }

            match parse_stream_event(data_line)? {
                ResponsesStreamEvent::OutputTextDelta { delta, .. } => {
                    state.is_text_response = true;

                    // Yield incremental text updates for true streaming
                    yield (
                        Some(text_delta_message(&delta, state.response_id.as_deref())),
                        None,
                        None,
                    );
                }

                event => {
                    // `?` must stand on its own here: `try_stream!` does not
                    // rewrite it inside a macro argument.
                    let flow = apply_stream_event(event, &mut state)?;
                    if matches!(flow, EventFlow::Stop) {
                        break 'outer;
                    }
                }
            }
        }

        if let Some((message, usage)) = state.into_final_item() {
            yield (message, usage, None);
        }
    }
}

/// Decode a streamed Responses API HTTP response into a provider message stream,
/// logging every item it yields.
///
/// The one decoder both Responses-routed providers use — OpenAI's
/// `v1/responses` and Azure's `openai/v1/responses` — so a fix to how a stream
/// is framed or logged cannot land in one of them and not the other. It was
/// written inline in `OpenAiProvider::stream` until the Azure provider gained a
/// Responses route (2026-09-25).
pub fn stream_responses_api(
    response: reqwest::Response,
    mut log: crate::providers::utils::RequestLog,
) -> crate::providers::base::MessageStream {
    use crate::providers::errors::ProviderError;
    use futures::{StreamExt, TryStreamExt};
    use tokio_util::codec::{FramedRead, LinesCodec};
    use tokio_util::io::StreamReader;

    let stream = response.bytes_stream().map_err(std::io::Error::other);

    Box::pin(try_stream! {
        let stream_reader = StreamReader::new(stream);
        let framed = FramedRead::new(stream_reader, LinesCodec::new()).map_err(anyhow::Error::from);

        let message_stream = responses_api_to_streaming_message(framed);
        tokio::pin!(message_stream);
        while let Some(message) = message_stream.next().await {
            let (message, usage, pending) = message.map_err(|e| ProviderError::RequestFailed(format!("Stream decode error: {}", e)))?;
            log.write(&message, usage.as_ref().map(|f| f.usage).as_ref())?;
            yield (message, usage, pending);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::effort::ReasoningEffort;
    use futures::StreamExt;
    use rmcp::object;
    use tokio::pin;

    /// The single `function_call_output` produced for one tool response.
    fn function_call_output(content: Vec<rmcp::model::Content>) -> String {
        let message = Message::user().with_tool_response(
            "call-1",
            Ok(rmcp::model::CallToolResult {
                content,
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        );

        let mut items = Vec::new();
        add_conversation_items(&mut items, &[message]);
        assert_eq!(items.len(), 1, "one output per tool response");
        items[0]["output"]
            .as_str()
            .expect("output is a string")
            .to_string()
    }

    /// Every audience case, through the real Responses API formatter.
    ///
    /// `openai.rs` filtered and this path did not, so the same OpenAI account
    /// sent the model different content depending on which API the model name
    /// routed to. The two agree now.
    #[test]
    fn tool_result_blocks_reach_the_model_by_audience() {
        let sent = function_call_output(crate::providers::formats::audience::every_audience_case());

        assert_eq!(
            sent,
            crate::providers::formats::audience::MODEL_VISIBLE.join("\n"),
            "the Responses output must carry exactly the model-addressed blocks"
        );
        for withheld in crate::providers::formats::audience::MODEL_HIDDEN {
            assert!(!sent.contains(withheld), "{withheld} reached the model");
        }
    }

    /// A `text_editor view` through the real Responses API formatter: the file
    /// arrives as an embedded resource, so reading only text blocks would send
    /// the model an empty output for every file view.
    #[test]
    fn a_viewed_file_reaches_the_model_through_its_embedded_resource() {
        let sent =
            function_call_output(crate::providers::formats::audience::text_editor_view_result());

        assert_eq!(sent, crate::providers::formats::audience::VIEW_FOR_MODEL);
        assert!(
            !sent.contains(crate::providers::formats::audience::VIEW_FOR_USER),
            "the user's rendering reached the model"
        );
    }

    /// A tool that addresses every block to the user still owes the API an
    /// output for its call. Skipping the item is what raises "No tool output
    /// found", which fails the whole next request rather than losing one block.
    #[test]
    fn a_fully_withheld_result_still_answers_its_function_call() {
        let sent =
            function_call_output(vec![rmcp::model::Content::text("for the user")
                .with_audience(vec![rmcp::model::Role::User])]);

        assert_eq!(sent, "", "the output is present and empty, not absent");
    }

    fn call(id: &str, name: &str) -> Message {
        Message::assistant()
            .with_text(format!("calling {name}"))
            .with_tool_request(
                id,
                Ok(CallToolRequestParams {
                    task: None,
                    name: name.to_string().into(),
                    arguments: Some(object!({})),
                    meta: None,
                }),
            )
    }

    fn answer(id: &str, content: Vec<rmcp::model::Content>) -> Message {
        Message::user().with_tool_response(
            id,
            Ok(rmcp::model::CallToolResult {
                content,
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        )
    }

    /// One short label per input item, so an ordering assertion reads as the
    /// transcript it describes.
    fn labels(payload: &Value) -> Vec<String> {
        payload["input"]
            .as_array()
            .expect("input is an array")
            .iter()
            .map(|item| match item["type"].as_str() {
                Some("function_call") => format!("call:{}", item["call_id"].as_str().unwrap()),
                Some("function_call_output") => {
                    format!("out:{}", item["call_id"].as_str().unwrap())
                }
                _ => {
                    let first = &item["content"][0];
                    let body = first["text"]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| first["type"].as_str().unwrap().to_string());
                    format!("{}:{body}", item["role"].as_str().unwrap())
                }
            })
            .collect()
    }

    /// A two-turn tool conversation reaches the model in the order it
    /// happened. The builder used to emit all text, then every call, then every
    /// output, which put turn one's call after turn two's question — accepted
    /// by the API, so nothing failed and the model read a scrambled transcript.
    #[test]
    fn a_multi_turn_tool_conversation_is_sent_in_order() -> anyhow::Result<()> {
        let messages = vec![
            Message::user().with_text("u1"),
            call("c1", "first"),
            answer("c1", vec![rmcp::model::Content::text("r1")]),
            Message::user().with_text("u2"),
            call("c2", "second"),
            answer("c2", vec![rmcp::model::Content::text("r2")]),
        ];
        let payload = create_responses_request(
            &ModelConfig::new_or_fail("gpt-6-sol"),
            "sys",
            &messages,
            &[],
        )?;

        assert_eq!(
            labels(&payload),
            [
                "system:sys",
                "user:u1",
                "assistant:calling first",
                "call:c1",
                "out:c1",
                "user:u2",
                "assistant:calling second",
                "call:c2",
                "out:c2",
            ]
        );
        Ok(())
    }

    /// An image a tool returns to the model (a Copilot screen capture, the
    /// developer image processor) reaches it. `flattened_text` has no text for
    /// an image, so it used to vanish into an empty output; now the output
    /// says an image follows, and the image comes as a user item after it —
    /// never between the call and its output. Chat Completions'
    /// `format_messages` has always done the same.
    #[test]
    fn a_tool_result_image_follows_its_output_as_a_user_item() -> anyhow::Result<()> {
        let messages = vec![
            Message::user().with_text("look"),
            call("c1", "screen_capture"),
            answer(
                "c1",
                vec![
                    rmcp::model::Content::text("captured"),
                    rmcp::model::Content::image("aGVsbG8=", "image/png"),
                ],
            ),
        ];
        let payload =
            create_responses_request(&ModelConfig::new_or_fail("gpt-6-sol"), "", &messages, &[])?;

        assert_eq!(
            labels(&payload),
            [
                "user:look",
                "assistant:calling screen_capture",
                "call:c1",
                "out:c1",
                "user:input_image",
            ]
        );
        let input = payload["input"].as_array().unwrap();
        assert_eq!(
            input[3]["output"],
            format!("captured\n{TOOL_IMAGE_PLACEHOLDER}"),
            "the output says an image follows"
        );
        assert_eq!(
            input[4]["content"][0]["image_url"],
            "data:image/png;base64,aGVsbG8="
        );
        Ok(())
    }

    /// An image the tool addressed to the user alone stays out, exactly like a
    /// user-only text block.
    #[test]
    fn a_user_only_tool_image_is_not_forwarded() -> anyhow::Result<()> {
        let messages = vec![
            call("c1", "render"),
            answer(
                "c1",
                vec![rmcp::model::Content::image("aGVsbG8=", "image/png")
                    .with_audience(vec![rmcp::model::Role::User])],
            ),
        ];
        let payload =
            create_responses_request(&ModelConfig::new_or_fail("gpt-6-sol"), "", &messages, &[])?;

        assert_eq!(
            labels(&payload),
            ["assistant:calling render", "call:c1", "out:c1"]
        );
        assert_eq!(payload["input"][2]["output"], "");
        Ok(())
    }

    // BR-63: the Responses API takes the effort nested under `reasoning`, not as
    // the chat-completions top-level `reasoning_effort` key.
    #[test]
    fn deep_effort_sets_reasoning_effort_high() -> anyhow::Result<()> {
        let model_config =
            ModelConfig::new_or_fail("gpt-5.5").with_reasoning_effort(Some(ReasoningEffort::Deep));

        let payload = create_responses_request(
            &model_config,
            "system",
            &[Message::user().with_text("hi")],
            &[],
        )?;

        assert_eq!(payload["reasoning"]["effort"], json!("high"));
        Ok(())
    }

    #[test]
    fn quick_effort_sets_reasoning_effort_low() -> anyhow::Result<()> {
        let model_config =
            ModelConfig::new_or_fail("gpt-5.5").with_reasoning_effort(Some(ReasoningEffort::Quick));

        let payload = create_responses_request(
            &model_config,
            "system",
            &[Message::user().with_text("hi")],
            &[],
        )?;

        assert_eq!(payload["reasoning"]["effort"], json!("low"));
        Ok(())
    }

    #[test]
    fn o4_mini_combines_function_tools_with_nested_reasoning_effort() -> anyhow::Result<()> {
        let model_config = ModelConfig::new_or_fail("o4-mini-2025-04-16")
            .with_reasoning_effort(Some(ReasoningEffort::Deep));
        let tool = Tool::new(
            "lookup_record",
            "Look up a record",
            object!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }),
        );

        let payload = create_responses_request(
            &model_config,
            "system",
            &[Message::user().with_text("Find record 42")],
            &[tool],
        )?;

        assert_eq!(payload["reasoning"]["effort"], json!("high"));
        assert_eq!(payload["tools"][0]["name"], json!("lookup_record"));
        assert!(payload.get("reasoning_effort").is_none());
        assert!(payload.get("messages").is_none());
        Ok(())
    }

    #[test]
    fn quick_effort_uses_the_minimum_supported_pro_effort() -> anyhow::Result<()> {
        for model_name in ["gpt-5.4-pro", "gpt-5.5-pro"] {
            let model_config = ModelConfig::new_or_fail(model_name)
                .with_reasoning_effort(Some(ReasoningEffort::Quick));
            let payload = create_responses_request(
                &model_config,
                "system",
                &[Message::user().with_text("hi")],
                &[],
            )?;

            assert_eq!(
                payload["reasoning"]["effort"],
                json!("medium"),
                "{model_name} does not accept low reasoning effort"
            );
        }
        Ok(())
    }

    #[test]
    fn responses_builder_omits_effort_for_non_reasoning_models() -> anyhow::Result<()> {
        let model_config =
            ModelConfig::new_or_fail("gpt-4.1").with_reasoning_effort(Some(ReasoningEffort::Deep));
        let payload = create_responses_request(
            &model_config,
            "system",
            &[Message::user().with_text("hi")],
            &[],
        )?;

        assert!(payload.get("reasoning").is_none());
        Ok(())
    }

    /// A configured temperature reaches the Responses API only for a model that
    /// can take it. GPT-6 and the GPT-5.x/o-series models routed here refuse it,
    /// so sending it failed every turn of a chat that had one set.
    #[test]
    fn temperature_is_dropped_for_reasoning_models_and_kept_otherwise() -> anyhow::Result<()> {
        for model_name in [
            "gpt-6-sol",
            "gpt-6-luna",
            "gpt-6-astra",
            "gpt-6-sol-2026-09-22",
            "gpt-5.6",
            "gpt-5.5",
            "gpt-5.4-mini",
            "o4-mini-2025-04-16",
        ] {
            let model_config = ModelConfig::new_or_fail(model_name).with_temperature(Some(0.3));
            let payload = create_responses_request(
                &model_config,
                "system",
                &[Message::user().with_text("hi")],
                &[],
            )?;
            assert!(
                payload.get("temperature").is_none(),
                "{model_name} refuses temperature"
            );
            assert!(payload.get("top_p").is_none(), "{model_name}");
        }

        // A non-reasoning model still gets the setting it asked for.
        let model_config = ModelConfig::new_or_fail("gpt-4.1").with_temperature(Some(0.3));
        let payload = create_responses_request(
            &model_config,
            "system",
            &[Message::user().with_text("hi")],
            &[],
        )?;
        let sent = payload["temperature"]
            .as_f64()
            .expect("gpt-4.1 takes a temperature");
        assert!((sent - 0.3).abs() < 1e-6, "sent {sent}");
        Ok(())
    }

    /// Dropping the temperature must not take the effort or the output cap
    /// with it — the three are independent settings on the same request.
    #[test]
    fn gpt_6_keeps_effort_and_output_cap_when_temperature_is_dropped() -> anyhow::Result<()> {
        let model_config = ModelConfig::new_or_fail("gpt-6-sol")
            .with_temperature(Some(0.7))
            .with_max_tokens(Some(2048))
            .with_reasoning_effort(Some(ReasoningEffort::Deep));
        let payload = create_responses_request(
            &model_config,
            "system",
            &[Message::user().with_text("hi")],
            &[],
        )?;
        assert!(payload.get("temperature").is_none());
        assert_eq!(payload["max_output_tokens"], json!(2048));
        assert_eq!(payload["reasoning"]["effort"], json!("high"));
        assert_eq!(payload["model"], json!("gpt-6-sol"));
        Ok(())
    }

    #[test]
    fn no_effort_leaves_the_request_untouched() -> anyhow::Result<()> {
        let model_config = ModelConfig::new_or_fail("gpt-5.5");

        let payload = create_responses_request(
            &model_config,
            "system",
            &[Message::user().with_text("hi")],
            &[],
        )?;

        assert!(payload.get("reasoning").is_none());
        Ok(())
    }

    #[test]
    fn non_streaming_usage_extracts_cached_input_tokens() {
        let response: ResponsesApiResponse = serde_json::from_value(json!({
            "id": "resp_cache",
            "object": "response",
            "created_at": 1,
            "status": "completed",
            "model": "gpt-5.4",
            "output": [],
            "usage": {
                "input_tokens": 1000,
                "output_tokens": 50,
                "total_tokens": 1050,
                "input_tokens_details": { "cached_tokens": 800 }
            }
        }))
        .expect("valid Responses API response");

        let usage = get_responses_usage(&response);
        assert_eq!(usage.input_tokens, Some(200));
        assert_eq!(usage.output_tokens, Some(50));
        assert_eq!(usage.cache_read_input_tokens, Some(800));
        assert_eq!(usage.cache_creation_input_tokens, None);
        assert_eq!(usage.total_tokens, Some(1050));
        assert_eq!(usage.billed_total(), Some(1050));
    }

    #[tokio::test]
    async fn streaming_usage_extracts_cached_input_tokens() -> anyhow::Result<()> {
        let lines = r#"
data: {"type":"response.completed","sequence_number":1,"response":{"id":"resp_cache","object":"response","created_at":1,"status":"completed","model":"gpt-5.4","output":[],"usage":{"input_tokens":1000,"output_tokens":50,"total_tokens":1050,"input_tokens_details":{"cached_tokens":800}}}}
data: [DONE]
"#;
        let response_stream = tokio_stream::iter(lines.lines().map(|line| Ok(line.to_string())));
        let messages = responses_api_to_streaming_message(response_stream);
        pin!(messages);

        let mut final_usage = None;
        while let Some(result) = messages.next().await {
            let (_, usage, _pending) = result?;
            if usage.is_some() {
                final_usage = usage;
            }
        }

        let usage = final_usage.expect("expected terminal usage");
        assert_eq!(usage.model, "gpt-5.4");
        assert_eq!(usage.usage.input_tokens, Some(200));
        assert_eq!(usage.usage.output_tokens, Some(50));
        assert_eq!(usage.usage.cache_read_input_tokens, Some(800));
        assert_eq!(usage.usage.cache_creation_input_tokens, None);
        assert_eq!(usage.usage.total_tokens, Some(1050));
        assert_eq!(usage.usage.billed_total(), Some(1050));
        Ok(())
    }

    // ── Issue #147: an unmodelled shape must not cost the turn ──────────────
    //
    // `parse_stream_event` is called with `?` inside the reader;
    // `stream_responses_api` (the decoder `openai.rs` and `azure.rs` share)
    // turns the error into `RequestFailed("Stream decode error: …")`, and
    // `with_retry` wraps only the POST, so nothing retries.
    //
    // ⚠ The fix has FOUR parts, and this comment used to name only the first —
    // which is how a half-fix reads as a whole one:
    //
    //   1. `#[serde(other)]` on `ResponsesStreamEvent`, for an unknown TAG.
    //   2. The same on the three enums that sit INSIDE a modelled tag —
    //      `ContentPart`, `ResponseOutputItemInfo`, and the non-streaming
    //      `ResponseOutputItem` / `ResponseContentBlock`. Part 1 cannot reach
    //      them: serde commits to a modelled variant and then fails one level
    //      down, which is where every built-in tool call and every refusal was
    //      landing.
    //   3. `summary: Vec<Value>` on both reasoning items. No open arm can cover
    //      this either — `reasoning` is a tag this decoder models, so only the
    //      field's type decides.
    //   4. The named `ResponseIncomplete` variant, which is not about decoding
    //      at all: the tag decoded fine as `Unknown`, and the turn's usage and
    //      terminal `output[]` were thrown away with it.
    //
    // The four controls at the end fail against the plausible over-correction
    // of making the decoder lenient about everything.

    /// Collect a whole SSE stream into (text, first error).
    async fn drain(lines: &str) -> (String, Option<String>) {
        let response_stream = tokio_stream::iter(
            lines
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| Ok(line.to_string()))
                .collect::<Vec<_>>(),
        );
        let messages = responses_api_to_streaming_message(response_stream);
        pin!(messages);

        let mut text = String::new();
        let mut error = None;
        while let Some(result) = messages.next().await {
            match result {
                Ok((Some(message), _, _)) => text.push_str(&message.as_concat_text()),
                Ok(_) => {}
                Err(err) => {
                    error = Some(err.to_string());
                    break;
                }
            }
        }
        (text, error)
    }

    /// [`drain`], plus the last usage the stream reported.
    ///
    /// Separate rather than folded in because `drain`'s callers assert on text
    /// alone, and a truncation's whole claim is about what arrives BESIDE the
    /// text — the usage item is yielded with no message at all, so a helper
    /// that only collects messages cannot see it.
    async fn drain_with_usage(lines: &str) -> (String, Option<ProviderUsage>, Option<String>) {
        let response_stream = tokio_stream::iter(
            lines
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| Ok(line.to_string()))
                .collect::<Vec<_>>(),
        );
        let messages = responses_api_to_streaming_message(response_stream);
        pin!(messages);

        let mut text = String::new();
        let mut usage = None;
        let mut error = None;
        while let Some(result) = messages.next().await {
            match result {
                Ok((message, item_usage, _)) => {
                    if let Some(message) = message {
                        text.push_str(&message.as_concat_text());
                    }
                    if item_usage.is_some() {
                        usage = item_usage;
                    }
                }
                Err(err) => {
                    error = Some(err.to_string());
                    break;
                }
            }
        }
        (text, usage, error)
    }

    /// The exact frame named in the report. `max_output_tokens` is set by
    /// `create_responses_request` from `model_config.max_tokens`, so this is
    /// the terminal frame of an ordinary capped generation — not an edge case.
    ///
    /// ⚠ This test used to assert `matches!(event, …::Unknown)` and its comment
    /// framed a named variant as the *wrong* fix. That was a false dichotomy:
    /// decoding the tag and keeping the data it carries are orthogonal, and the
    /// open arm below still covers every tag nobody modelled. The requirement
    /// this now states is the real one — the tag decodes as itself, and even a
    /// body this decoder cannot read degrades to `None` instead of failing.
    ///
    /// Catches: an enum with no arm for this tag at all (the shipped one),
    /// where these return `Err("Failed to parse Responses stream event: unknown
    /// variant …")`; and a strict `response: ResponseMetadata`, where the
    /// second payload fails.
    #[test]
    fn an_incomplete_response_decodes_as_itself() {
        let event = parse_stream_event(r#"{"type":"response.incomplete"}"#)
            .expect("response.incomplete must decode");
        assert!(
            matches!(
                event,
                ResponsesStreamEvent::ResponseIncomplete { response: None, .. }
            ),
            "a bodyless truncation frame must decode as ResponseIncomplete"
        );

        let event = parse_stream_event(
            r#"{"type":"response.incomplete","sequence_number":3,"response":{"unreadable":true}}"#,
        )
        .expect("an unreadable incomplete body must not fail the frame");
        assert!(matches!(
            event,
            ResponsesStreamEvent::ResponseIncomplete { response: None, .. }
        ));
    }

    /// The data the named variant exists to keep: a turn truncated by our own
    /// `max_output_tokens` still reports what it cost, and still tells the
    /// agent loop it was cut off.
    ///
    /// Catches: decoding `response.incomplete` to `Unknown` (the state this
    /// branch shipped in), where `apply_stream_event`'s `_ => {}` arm discards
    /// the frame, `final_usage` is never set, and the turn reports no tokens,
    /// no cost and no truncation.
    #[tokio::test]
    async fn a_truncated_turn_still_reports_its_usage_and_says_it_was_cut_off() {
        let lines = r#"
data: {"type":"response.created","sequence_number":0,"response":{"id":"resp_inc","object":"response","created_at":1,"status":"in_progress","model":"gpt-5.4","output":[]}}
data: {"type":"response.output_text.delta","sequence_number":1,"item_id":"msg_1","output_index":0,"content_index":0,"delta":"partial "}
data: {"type":"response.output_text.delta","sequence_number":2,"item_id":"msg_1","output_index":0,"content_index":0,"delta":"answer"}
data: {"type":"response.incomplete","sequence_number":3,"response":{"id":"resp_inc","object":"response","created_at":1,"status":"incomplete","model":"gpt-5.4","output":[],"usage":{"input_tokens":900,"output_tokens":128,"total_tokens":1028},"incomplete_details":{"reason":"max_output_tokens"}}}
data: [DONE]
"#;
        let (text, usage, error) = drain_with_usage(lines).await;

        assert_eq!(error, None, "a capped stream must not fail the request");
        assert_eq!(text, "partial answer");

        let usage = usage.expect("a truncated turn must still report its usage");
        assert_eq!(usage.usage.input_tokens, Some(900));
        assert_eq!(usage.usage.output_tokens, Some(128));
        assert_eq!(usage.usage.total_tokens, Some(1028));
        assert_eq!(usage.model, "gpt-5.4");
        assert_eq!(
            usage.finish_reason.as_deref(),
            Some("length"),
            "\"length\" is the one token the agent loop auto-continues on"
        );
    }

    /// The other half of what the terminal frame carries: output items that
    /// appear ONLY there. `response.completed` folds them in; the truncation
    /// frame must too, or a turn whose only content arrived in the terminal
    /// frame yields an empty message.
    ///
    /// No text deltas here on purpose — with `is_text_response` set, text from
    /// the terminal frame is suppressed as an already-streamed duplicate, and
    /// the assertion would measure the suppression instead of the merge.
    ///
    /// Catches: the same `Unknown` decode, and also a `ResponseIncomplete` arm
    /// that reads `usage` but forgets to call `absorb_completed_response`.
    #[tokio::test]
    async fn a_truncated_turns_terminal_output_items_are_not_dropped() {
        let lines = r#"
data: {"type":"response.created","sequence_number":0,"response":{"id":"resp_inc2","object":"response","created_at":1,"status":"in_progress","model":"gpt-5.4","output":[]}}
data: {"type":"response.incomplete","sequence_number":1,"response":{"id":"resp_inc2","object":"response","created_at":1,"status":"incomplete","model":"gpt-5.4","output":[{"type":"message","id":"msg_1","status":"incomplete","role":"assistant","content":[{"type":"output_text","text":"only in the terminal frame"}]}],"incomplete_details":{"reason":"max_output_tokens"}}}
data: [DONE]
"#;
        let (text, _, error) = drain_with_usage(lines).await;

        assert_eq!(error, None);
        assert_eq!(text, "only in the terminal frame");
    }

    /// A refusal is not a length problem. `"length"` makes the agent loop
    /// re-ask for the rest of the answer; doing that to a content filter asks
    /// again for the content that was just refused.
    ///
    /// Catches: mapping every `response.incomplete` to `"length"` — the
    /// plausible simplification of `incomplete_finish_reason`, which passes the
    /// two tests above.
    #[tokio::test]
    async fn a_content_filtered_truncation_is_not_reported_as_a_length_cap() {
        let lines = r#"
data: {"type":"response.incomplete","sequence_number":1,"response":{"id":"resp_cf","object":"response","created_at":1,"status":"incomplete","model":"gpt-5.4","output":[],"usage":{"input_tokens":10,"output_tokens":0,"total_tokens":10},"incomplete_details":{"reason":"content_filter"}}}
data: [DONE]
"#;
        let (_, usage, error) = drain_with_usage(lines).await;

        assert_eq!(error, None);
        assert_eq!(
            usage.expect("usage").finish_reason.as_deref(),
            Some("content_filter"),
        );
    }

    /// A built-in tool call is an output ITEM, not an event tag — so the
    /// enum-level open arm never sees it, and `web_search_call` failed the
    /// whole turn while the comment beside that arm cited built-in tools as
    /// motivating. Three separate frames carry one: `output_item.added`,
    /// `output_item.done`, and `response.completed`'s `output[]`.
    ///
    /// Catches: a closed `ResponseOutputItemInfo`, where the second frame
    /// errors with "unknown variant `web_search_call`" and the streamed text is
    /// lost with the turn.
    #[tokio::test]
    async fn a_built_in_tool_output_item_does_not_fail_the_stream() {
        let lines = r#"
data: {"type":"response.created","sequence_number":0,"response":{"id":"resp_bt","object":"response","created_at":1,"status":"in_progress","model":"gpt-5.4","output":[]}}
data: {"type":"response.output_item.added","sequence_number":1,"output_index":0,"item":{"type":"web_search_call","id":"ws_1","status":"in_progress"}}
data: {"type":"response.output_item.done","sequence_number":2,"output_index":0,"item":{"type":"web_search_call","id":"ws_1","status":"completed"}}
data: {"type":"response.output_text.delta","sequence_number":3,"item_id":"msg_1","output_index":1,"content_index":0,"delta":"searched"}
data: {"type":"response.completed","sequence_number":4,"response":{"id":"resp_bt","object":"response","created_at":1,"status":"completed","model":"gpt-5.4","output":[{"type":"web_search_call","id":"ws_1","status":"completed"},{"type":"mcp_call","id":"mcp_1","status":"completed"}],"usage":{"input_tokens":5,"output_tokens":2,"total_tokens":7}}}
data: [DONE]
"#;
        let (text, usage, error) = drain_with_usage(lines).await;

        assert_eq!(
            error, None,
            "a built-in tool item must not fail the request"
        );
        assert_eq!(text, "searched");
        assert_eq!(usage.expect("usage").usage.total_tokens, Some(7));
    }

    /// The non-streaming path (`responses_api_to_message`), which the streaming
    /// fix does not reach at all: a refusal content block and a built-in tool
    /// item both arrive inside `output[]` of an ordinary POST response.
    ///
    /// Catches: closing only the streaming enums — the whole response fails to
    /// deserialize, so the caller gets an error instead of the message the
    /// model did produce.
    #[test]
    fn a_refusal_and_a_built_in_item_do_not_fail_the_non_streaming_path() {
        let response: ResponsesApiResponse = serde_json::from_value(json!({
            "id": "resp_ns",
            "object": "response",
            "created_at": 1,
            "status": "completed",
            "model": "gpt-5.4",
            "output": [
                { "type": "web_search_call", "id": "ws_1", "status": "completed" },
                {
                    "type": "message",
                    "id": "msg_1",
                    "status": "completed",
                    "role": "assistant",
                    "content": [
                        { "type": "refusal", "refusal": "I cannot help with that." },
                        { "type": "output_text", "text": "here is what I can do" }
                    ]
                }
            ]
        }))
        .expect("a response carrying a refusal and a built-in item must decode");

        let message = responses_api_to_message(&response).expect("message");
        assert_eq!(message.as_concat_text(), "here is what I can do");
    }

    /// Latent until reasoning summaries are requested, and fatal the moment
    /// they are: the API returns `[{"type":"summary_text","text":…}]`, which a
    /// `Vec<String>` cannot hold. The tag (`reasoning`) is modelled, so no
    /// `#[serde(other)]` arm can cover this — only the field's type can.
    ///
    /// Catches: `summary: Vec<String>` / `Option<Vec<String>>` on either enum.
    #[test]
    fn a_reasoning_summary_of_objects_decodes_on_both_paths() {
        let event = parse_stream_event(
            r#"{"type":"response.output_item.done","sequence_number":1,"output_index":0,"item":{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"thought about it"}]}}"#,
        )
        .expect("a streamed reasoning summary must decode");
        assert!(matches!(
            event,
            ResponsesStreamEvent::OutputItemDone {
                item: ResponseOutputItemInfo::Reasoning { .. },
                ..
            }
        ));

        let response: ResponsesApiResponse = serde_json::from_value(json!({
            "id": "resp_rs",
            "object": "response",
            "created_at": 1,
            "status": "completed",
            "model": "gpt-5.4",
            "output": [{
                "type": "reasoning",
                "id": "rs_1",
                "summary": [{ "type": "summary_text", "text": "thought about it" }]
            }]
        }))
        .expect("a non-streaming reasoning summary must decode");
        assert!(matches!(
            response.output.first(),
            Some(ResponseOutputItem::Reasoning { .. })
        ));
    }

    /// The same for the refusal events, and for a tag nobody has invented yet.
    ///
    /// Catches: a "fix" that special-cases `response.incomplete` with its own
    /// named variant instead of an open arm — which passes the test above and
    /// leaves every other unmodelled tag fatal.
    #[test]
    fn other_unmodelled_tags_decode_too() {
        for payload in [
            r#"{"type":"response.refusal.delta","sequence_number":4,"delta":"I cannot"}"#,
            r#"{"type":"response.refusal.done","sequence_number":5,"refusal":"I cannot"}"#,
            r#"{"type":"response.output_item.added.v9000","invented":true}"#,
        ] {
            let event = parse_stream_event(payload)
                .unwrap_or_else(|err| panic!("{payload} must decode, got {err}"));
            assert!(matches!(event, ResponsesStreamEvent::Unknown), "{payload}");
        }
    }

    /// The bug as the user met it: text already streamed must survive the
    /// truncation that ended the stream.
    ///
    /// Catches: the shipped decoder, where the third frame aborts the stream
    /// and the two deltas are thrown away with it — the whole turn lost to a
    /// cap we set ourselves.
    #[tokio::test]
    async fn a_stream_truncated_at_max_output_tokens_keeps_its_text() {
        let lines = r#"
data: {"type":"response.created","sequence_number":0,"response":{"id":"resp_inc","object":"response","created_at":1,"status":"in_progress","model":"gpt-5.4","output":[]}}
data: {"type":"response.output_text.delta","sequence_number":1,"item_id":"msg_1","output_index":0,"content_index":0,"delta":"partial "}
data: {"type":"response.output_text.delta","sequence_number":2,"item_id":"msg_1","output_index":0,"content_index":0,"delta":"answer"}
data: {"type":"response.incomplete","sequence_number":3,"response":{"id":"resp_inc","object":"response","created_at":1,"status":"incomplete","model":"gpt-5.4","output":[],"incomplete_details":{"reason":"max_output_tokens"}}}
data: [DONE]
"#;
        let (text, error) = drain(lines).await;
        assert_eq!(error, None, "a capped stream must not fail the request");
        assert_eq!(text, "partial answer");
    }

    /// The nested half, which the enum-level arm CANNOT cover: a refusal
    /// arrives as a `part` inside `response.content_part.added`, a tag this
    /// decoder does know, so `#[serde(other)]` on `ResponsesStreamEvent` never
    /// fires and the failure moves one level down into `ContentPart`.
    ///
    /// Catches: fixing only `ResponsesStreamEvent` and leaving `ContentPart`
    /// closed — the exact half-fix the report warns about, which passes every
    /// test above.
    #[tokio::test]
    async fn a_refusal_content_part_does_not_fail_the_stream() {
        let lines = r#"
data: {"type":"response.created","sequence_number":0,"response":{"id":"resp_ref","object":"response","created_at":1,"status":"in_progress","model":"gpt-5.4","output":[]}}
data: {"type":"response.output_text.delta","sequence_number":1,"item_id":"msg_1","output_index":0,"content_index":0,"delta":"before"}
data: {"type":"response.content_part.added","sequence_number":2,"item_id":"msg_1","output_index":0,"content_index":1,"part":{"type":"refusal","refusal":"I cannot help with that."}}
data: {"type":"response.completed","sequence_number":3,"response":{"id":"resp_ref","object":"response","created_at":1,"status":"completed","model":"gpt-5.4","output":[{"type":"message","id":"msg_1","status":"completed","role":"assistant","content":[{"type":"refusal","refusal":"I cannot help with that."}]}]}}
data: [DONE]
"#;
        let (text, error) = drain(lines).await;
        assert_eq!(error, None, "a refusal part must not fail the request");
        assert_eq!(text, "before");
    }

    /// Control 1. `#[serde(other)]` matches on the TAG only, so a frame that is
    /// not JSON at all is still a hard error — and the error still refuses to
    /// echo the payload, which may hold raw model output bound for the user.
    ///
    /// Catches: "fixing" the decode by making `parse_stream_event` swallow
    /// every error (`unwrap_or(Unknown)`), which passes all four tests above
    /// and silently drops real corruption.
    #[test]
    fn a_malformed_frame_is_still_an_error() {
        let err = parse_stream_event(r#"{"type":"response.completed", TRUNCATED"#)
            .expect_err("invalid JSON must still fail");
        let rendered = err.to_string();
        assert!(rendered.contains("Failed to parse Responses stream event"));
        assert!(
            !rendered.contains("TRUNCATED"),
            "the payload must not be echoed"
        );
    }

    /// Control 2. A tag this decoder DOES model, with a body it cannot satisfy,
    /// must not be quietly reinterpreted as `Unknown`. `response.completed` is
    /// where usage and the final output items come from; treating a broken one
    /// as an ignorable event would drop a whole response without a word.
    ///
    /// Catches: putting the open arm on the wrong axis — e.g. adding
    /// `#[serde(untagged)]` or a `Value` fallback variant, which would absorb
    /// this instead of failing.
    #[test]
    fn a_known_tag_with_a_broken_body_is_still_an_error() {
        parse_stream_event(r#"{"type":"response.completed","sequence_number":1}"#)
            .expect_err("a response.completed with no response object must fail");
    }

    /// Control 3. The two events that *are* errors keep their explicit arms —
    /// the unknown arm must not have shadowed them into silence.
    ///
    /// Catches: declaring `Unknown` before the `error` variant in a way that
    /// captured it, or deleting those arms while adding the catch-all.
    #[tokio::test]
    async fn real_error_events_still_fail_the_stream() {
        let failed = r#"
data: {"type":"response.failed","sequence_number":1,"error":{"code":"server_error","message":"boom"}}
"#;
        let (_, error) = drain(failed).await;
        assert!(
            error
                .as_deref()
                .is_some_and(|e| e.contains("Responses API failed")),
            "response.failed must still fail, got {error:?}"
        );

        let errored = r#"
data: {"type":"error","error":{"code":"rate_limit_exceeded","message":"slow down"}}
"#;
        let (_, error) = drain(errored).await;
        assert!(
            error
                .as_deref()
                .is_some_and(|e| e.contains("Responses API error")),
            "an error frame must still fail, got {error:?}"
        );
    }

    /// Control 4. The unknown arm must not have eaten a tag that carries real
    /// state. `response.output_item.done` is what makes a tool call callable,
    /// and it is one `#[serde(rename)]` typo away from decoding as `Unknown`
    /// and silently degrading every tool call to "never ran".
    ///
    /// Catches: a rename/spelling drift on any modelled variant, which is
    /// exactly the failure the open arm now hides.
    #[tokio::test]
    async fn a_modelled_tag_is_not_swallowed_by_the_unknown_arm() {
        let lines = r#"
data: {"type":"response.created","sequence_number":0,"response":{"id":"resp_tool","object":"response","created_at":1,"status":"in_progress","model":"gpt-5.4","output":[]}}
data: {"type":"response.output_item.done","sequence_number":1,"output_index":0,"item":{"type":"function_call","id":"fc_1","status":"completed","call_id":"call_1","name":"developer__shell","arguments":"{\"command\":\"ls\"}"}}
data: {"type":"response.completed","sequence_number":2,"response":{"id":"resp_tool","object":"response","created_at":1,"status":"completed","model":"gpt-5.4","output":[]}}
data: [DONE]
"#;
        let response_stream = tokio_stream::iter(
            lines
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| Ok(line.to_string()))
                .collect::<Vec<_>>(),
        );
        let messages = responses_api_to_streaming_message(response_stream);
        pin!(messages);

        let mut saw_tool_request = false;
        while let Some(result) = messages.next().await {
            let (message, _, _) = result.expect("stream must not fail");
            if let Some(message) = message {
                saw_tool_request |= message.is_tool_call();
            }
        }
        assert!(
            saw_tool_request,
            "response.output_item.done must still produce a callable tool request"
        );
    }
}
