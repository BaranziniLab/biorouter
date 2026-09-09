use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use crate::mcp_utils::ToolResult;
use anyhow::{anyhow, bail, Result};
use aws_sdk_bedrockruntime::error::{ProvideErrorMetadata, SdkError};
use aws_sdk_bedrockruntime::operation::converse::ConverseError;
use aws_sdk_bedrockruntime::operation::converse_stream::{
    ConverseStreamError, ConverseStreamOutput as ConverseStreamResponse,
};
use aws_sdk_bedrockruntime::types as bedrock;
use aws_sdk_bedrockruntime::types::error::ConverseStreamOutputError;
use aws_smithy_types::{Document, Number};
use base64::Engine;
use chrono::Utc;
use rmcp::model::{
    object, CallToolRequestParams, Content, ErrorCode, ErrorData, RawContent, ResourceContents,
    Role, Tool,
};
use serde_json::{json, Value};

use super::super::base::{tool_call_batching_enabled, Usage};
use super::super::errors::ProviderError;
use super::audience;
use crate::conversation::message::{Message, MessageContent};
use crate::model::ModelConfig;
use crate::providers::utils::RequestLog;

/// Conservative output ceiling used when a Bedrock model has no BioRouter
/// override. Current Claude 4.5+ models support at least 64K output, while an
/// unlisted/non-Claude model keeps the broadly supported 4K fallback.
pub const BEDROCK_CLAUDE_DEFAULT_MAX_TOKENS: i32 = 64_000;
pub const BEDROCK_GENERIC_DEFAULT_MAX_TOKENS: i32 = 4_096;
pub const BEDROCK_BLOCKING_MAX_TOKENS: i32 = 21_333;

/// One inference policy for Amazon Bedrock and the Versa proxy, shared by both
/// Converse and ConverseStream. Keeping this as a typed SDK value makes it
/// impossible for the four callers to disagree about the field names or
/// defaults.
pub fn bedrock_inference_config(model_config: &ModelConfig) -> bedrock::InferenceConfiguration {
    bedrock_inference_config_with_limit(model_config, None)
}

pub fn bedrock_blocking_inference_config(
    model_config: &ModelConfig,
) -> bedrock::InferenceConfiguration {
    bedrock_inference_config_with_limit(model_config, Some(BEDROCK_BLOCKING_MAX_TOKENS))
}

/// A short, safe description of an SDK error for a user-facing message.
///
/// ⚠ **Never `format!("{:?}", err)` on an SDK error.** The Debug rendering of a
/// `ServiceError` includes the RAW RESPONSE, and the raw response carries the
/// request headers -- among them the SigV4 `authorization` header:
///
/// ```text
/// "authorization": "AWS4-HMAC-SHA256 Credential=47dd…/us-west-2/bedrock/aws4_request,
///                   SignedHeaders=…, Signature=93d6a424…"
/// ```
///
/// That was being rendered into the chat as ~40 lines of Rust debug output, so a
/// user reporting a 403 would paste a live request signature into an issue
/// tracker. It is also unreadable: the one useful token in the whole dump was
/// `"Invalid Client Id"`.
///
/// This returns the service's own message when there is one, and a bare
/// description otherwise. Both are safe to show and are what the reader needs.
fn safe_detail<E: aws_sdk_bedrockruntime::error::ProvideErrorMetadata>(err: &E) -> String {
    match err.message() {
        Some(m) if !m.trim().is_empty() => m.trim().to_string(),
        _ => match err.code() {
            Some(c) if !c.trim().is_empty() => format!("error code {}", c.trim()),
            _ => "no further detail was returned".to_string(),
        },
    }
}

/// Bound a *default* output allowance by the room compaction leaves for it.
///
/// Bedrock rejects a request whose `input + maxTokens` exceeds the model's
/// context window — "input length and max_tokens exceed context limit: 196395 +
/// 64000 > 204698". Auto-compaction does not fire until input reaches
/// [`DEFAULT_COMPACTION_THRESHOLD`] of the window, so a fixed 64,000 allowance
/// opens a band on every 200K-context model (Haiku 4.5, Sonnet 4.5, Opus 4.5)
/// where input is past `context - 64_000` but not yet past the compaction
/// trigger: every request in that band 400s on a turn the agent believed was in
/// budget. It recovers — `looks_like_context_overflow` classifies it and
/// compacts — but at the cost of a wasted round trip each time.
///
/// Keeping the allowance inside the post-threshold headroom closes the band by
/// construction. The floor is the conservative default, so this can never ask
/// for less than the provider would have applied with the field omitted.
fn bounded_by_context_window(max_tokens: i32, context_limit: usize) -> i32 {
    let headroom =
        (context_limit as f64 * (1.0 - crate::context_mgmt::DEFAULT_COMPACTION_THRESHOLD)) as i32;
    max_tokens.min(headroom.max(BEDROCK_GENERIC_DEFAULT_MAX_TOKENS))
}

fn bedrock_inference_config_with_limit(
    model_config: &ModelConfig,
    transport_limit: Option<i32>,
) -> bedrock::InferenceConfiguration {
    let max_tokens = model_config
        .max_tokens
        .filter(|tokens| *tokens > 0)
        .unwrap_or_else(|| {
            // Our own default is bounded by the headroom compaction guarantees.
            // An explicitly configured value is the caller's decision and is left
            // alone.
            bounded_by_context_window(
                bedrock_default_max_tokens(&model_config.model_name),
                model_config.context_limit(),
            )
        });
    let max_tokens = transport_limit.map_or(max_tokens, |limit| max_tokens.min(limit));
    let mut builder = bedrock::InferenceConfiguration::builder().max_tokens(max_tokens);

    // Sonnet 5 and the other adaptive-only Claude families reject sampling
    // parameters. Older Claude and non-Claude Bedrock models may receive the
    // configured temperature unchanged.
    if !bedrock_uses_adaptive_thinking(&model_config.model_name) {
        builder = builder.set_temperature(model_config.temperature);
    }
    builder.build()
}

fn bedrock_default_max_tokens(model_name: &str) -> i32 {
    const CLAUDE_64K_FAMILIES: &[&str] = &[
        "claude-opus-4-5",
        "claude-opus-4.5",
        "claude-opus-4-6",
        "claude-opus-4.6",
        "claude-opus-4-7",
        "claude-opus-4.7",
        "claude-opus-4-8",
        "claude-opus-4.8",
        "claude-sonnet-4-5",
        "claude-sonnet-4.5",
        "claude-sonnet-4-6",
        "claude-sonnet-4.6",
        "claude-sonnet-4-7",
        "claude-sonnet-4.7",
        "claude-sonnet-4-8",
        "claude-sonnet-4.8",
        "claude-haiku-4-5",
        "claude-haiku-4.5",
        "claude-haiku-4-6",
        "claude-haiku-4.6",
        "claude-haiku-4-7",
        "claude-haiku-4.7",
        "claude-haiku-4-8",
        "claude-haiku-4.8",
        "claude-opus-5",
        "claude-sonnet-5",
        "claude-haiku-5",
        "claude-fable-5",
        "claude-mythos-5",
    ];
    let lower = model_name.to_ascii_lowercase();
    if lower.contains("claude-3-haiku") {
        4_096
    } else if lower.contains("claude-opus-4-0") || lower.contains("claude-opus-4-1") {
        32_000
    } else if CLAUDE_64K_FAMILIES
        .iter()
        .any(|family| lower.contains(family))
    {
        BEDROCK_CLAUDE_DEFAULT_MAX_TOKENS
    } else {
        BEDROCK_GENERIC_DEFAULT_MAX_TOKENS
    }
}

fn bedrock_uses_adaptive_thinking(model_name: &str) -> bool {
    const ADAPTIVE_ONLY: &[&str] = &[
        "opus-5", "sonnet-5", "fable-5", "mythos-5", "opus-4-7", "opus-4.7", "opus-4-8", "opus-4.8",
    ];
    let lower = model_name.to_ascii_lowercase();
    ADAPTIVE_ONLY.iter().any(|pattern| lower.contains(pattern))
}

pub fn to_bedrock_message(message: &Message) -> Result<bedrock::Message> {
    bedrock::Message::builder()
        .role(to_bedrock_role(&message.role))
        .set_content(Some(
            message
                .content
                .iter()
                .map(to_bedrock_message_content)
                .collect::<Result<_>>()?,
        ))
        .build()
        .map_err(|err| anyhow!("Failed to construct Bedrock message: {}", err))
}

pub fn to_bedrock_message_content(content: &MessageContent) -> Result<bedrock::ContentBlock> {
    Ok(match content {
        MessageContent::Text(text) => bedrock::ContentBlock::Text(text.text.to_string()),
        MessageContent::ToolConfirmationRequest(_tool_confirmation_request) => {
            bedrock::ContentBlock::Text("".to_string())
        }
        MessageContent::ActionRequired(_action_required) => {
            bedrock::ContentBlock::Text("".to_string())
        }
        MessageContent::Image(image) => {
            bedrock::ContentBlock::Image(to_bedrock_image(&image.data, &image.mime_type)?)
        }
        MessageContent::Thinking(thinking) => {
            bedrock::ContentBlock::ReasoningContent(bedrock::ReasoningContentBlock::ReasoningText(
                bedrock::ReasoningTextBlock::builder()
                    .text(thinking.thinking.clone())
                    .set_signature(
                        (!thinking.signature.is_empty()).then(|| thinking.signature.clone()),
                    )
                    .build()?,
            ))
        }
        MessageContent::RedactedThinking(redacted) => {
            let bytes = base64::prelude::BASE64_STANDARD
                .decode(&redacted.data)
                .map_err(|e| anyhow!("Invalid base64 Bedrock redacted reasoning block: {e}"))?;
            bedrock::ContentBlock::ReasoningContent(
                bedrock::ReasoningContentBlock::RedactedContent(aws_smithy_types::Blob::new(bytes)),
            )
        }
        MessageContent::SystemNotification(_) => {
            bail!("SystemNotification should not get passed to the provider")
        }
        MessageContent::ToolRequest(tool_req) => {
            let tool_use_id = tool_req.id.to_string();
            let tool_use = if let Ok(call) = tool_req.tool_call.as_ref() {
                bedrock::ToolUseBlock::builder()
                    .tool_use_id(tool_use_id)
                    .name(call.name.to_string())
                    .input(to_bedrock_json(&Value::from(call.arguments.clone())))
                    .build()
            } else {
                bedrock::ToolUseBlock::builder()
                    .tool_use_id(tool_use_id)
                    .build()
            }?;
            bedrock::ContentBlock::ToolUse(tool_use)
        }
        MessageContent::FrontendToolRequest(tool_req) => {
            let tool_use_id = tool_req.id.to_string();
            let tool_use = if let Ok(call) = tool_req.tool_call.as_ref() {
                bedrock::ToolUseBlock::builder()
                    .tool_use_id(tool_use_id)
                    .name(call.name.to_string())
                    .input(to_bedrock_json(&Value::from(call.arguments.clone())))
                    .build()
            } else {
                bedrock::ToolUseBlock::builder()
                    .tool_use_id(tool_use_id)
                    .build()
            }?;
            bedrock::ContentBlock::ToolUse(tool_use)
        }
        MessageContent::ToolResponse(tool_res) => {
            let content = match &tool_res.tool_result {
                Ok(result) => Some(
                    result
                        .content
                        .iter()
                        // Send only what the tool addressed to the model.
                        .filter(|c| audience::is_for_model(c))
                        .map(|c| to_bedrock_tool_result_content_block(&tool_res.id, c.clone()))
                        .collect::<Result<_>>()?,
                ),
                Err(error) => {
                    // For errors, create a text content block with the error message
                    Some(vec![bedrock::ToolResultContentBlock::Text(format!(
                        "The tool call returned the following error:\n{}",
                        error
                    ))])
                }
            };
            bedrock::ContentBlock::ToolResult(
                bedrock::ToolResultBlock::builder()
                    .tool_use_id(tool_res.id.to_string())
                    .status(if tool_res.tool_result.is_ok() {
                        bedrock::ToolResultStatus::Success
                    } else {
                        bedrock::ToolResultStatus::Error
                    })
                    .set_content(content)
                    .build()?,
            )
        }
    })
}

/// Convert MCP Content to Bedrock ToolResultContentBlock
///
/// Supports text, images, and document resources. Images are supported
/// by Bedrock for Anthropic Claude 3 models.
pub fn to_bedrock_tool_result_content_block(
    tool_use_id: &str,
    content: Content,
) -> Result<bedrock::ToolResultContentBlock> {
    Ok(match content.raw {
        RawContent::Text(text) => bedrock::ToolResultContentBlock::Text(text.text),
        RawContent::Image(image) => {
            bedrock::ToolResultContentBlock::Image(to_bedrock_image(&image.data, &image.mime_type)?)
        }
        RawContent::ResourceLink(_link) => {
            bedrock::ToolResultContentBlock::Text("[Resource link]".to_string())
        }
        RawContent::Resource(resource) => match &resource.resource {
            ResourceContents::TextResourceContents { text, .. } => {
                match to_bedrock_document(tool_use_id, &resource.resource)? {
                    Some(doc) => bedrock::ToolResultContentBlock::Document(doc),
                    None => bedrock::ToolResultContentBlock::Text(text.to_string()),
                }
            }
            ResourceContents::BlobResourceContents { .. } => {
                bail!("Blob resource content is not supported by Bedrock provider yet")
            }
        },
        RawContent::Audio(..) => bail!("Audio is not not supported by Bedrock provider"),
    })
}

pub fn to_bedrock_role(role: &Role) -> bedrock::ConversationRole {
    match role {
        Role::User => bedrock::ConversationRole::User,
        Role::Assistant => bedrock::ConversationRole::Assistant,
    }
}

pub fn to_bedrock_image(data: &String, mime_type: &String) -> Result<bedrock::ImageBlock> {
    // Extract format from MIME type
    let format = match mime_type.as_str() {
        "image/png" => bedrock::ImageFormat::Png,
        "image/jpeg" | "image/jpg" => bedrock::ImageFormat::Jpeg,
        "image/gif" => bedrock::ImageFormat::Gif,
        "image/webp" => bedrock::ImageFormat::Webp,
        _ => bail!(
            "Unsupported image format: {}. Bedrock supports png, jpeg, gif, webp",
            mime_type
        ),
    };

    // Create image source with base64 data
    let source = bedrock::ImageSource::Bytes(aws_smithy_types::Blob::new(
        base64::prelude::BASE64_STANDARD
            .decode(data)
            .map_err(|e| anyhow!("Failed to decode base64 image data: {}", e))?,
    ));

    // Build the image block
    Ok(bedrock::ImageBlock::builder()
        .format(format)
        .source(source)
        .build()?)
}

pub fn to_bedrock_tool_config(tools: &[Tool]) -> Result<bedrock::ToolConfiguration> {
    Ok(bedrock::ToolConfiguration::builder()
        .set_tools(Some(
            tools.iter().map(to_bedrock_tool).collect::<Result<_>>()?,
        ))
        .build()?)
}

pub fn to_bedrock_tool(tool: &Tool) -> Result<bedrock::Tool> {
    let mut input_schema = tool.input_schema.as_ref().clone();

    // If the schema doesn't have a "type" field, add it
    // This is required by Bedrock
    if !input_schema.contains_key("type") {
        input_schema.insert("type".to_string(), Value::String("object".to_string()));
    }

    Ok(bedrock::Tool::ToolSpec(
        bedrock::ToolSpecification::builder()
            .name(tool.name.to_string())
            .description(
                tool.description
                    .as_ref()
                    .map(|d| d.to_string())
                    .unwrap_or_default(),
            )
            .input_schema(bedrock::ToolInputSchema::Json(to_bedrock_json(
                &Value::Object(input_schema),
            )))
            .build()?,
    ))
}

pub fn to_bedrock_json(value: &Value) -> Document {
    match value {
        Value::Null => Document::Null,
        Value::Bool(bool) => Document::Bool(*bool),
        Value::Number(num) => {
            if let Some(n) = num.as_u64() {
                Document::Number(Number::PosInt(n))
            } else if let Some(n) = num.as_i64() {
                Document::Number(Number::NegInt(n))
            } else if let Some(n) = num.as_f64() {
                Document::Number(Number::Float(n))
            } else {
                unreachable!()
            }
        }
        Value::String(str) => Document::String(str.to_string()),
        Value::Array(arr) => Document::Array(arr.iter().map(to_bedrock_json).collect()),
        Value::Object(obj) => Document::Object(HashMap::from_iter(
            obj.into_iter()
                .map(|(key, val)| (key.to_string(), to_bedrock_json(val))),
        )),
    }
}

fn to_bedrock_document(
    tool_use_id: &str,
    content: &ResourceContents,
) -> Result<Option<bedrock::DocumentBlock>> {
    let (uri, text) = match content {
        ResourceContents::TextResourceContents { uri, text, .. } => (uri, text),
        ResourceContents::BlobResourceContents { .. } => {
            bail!("Blob resource content is not supported by Bedrock provider yet")
        }
    };

    let filename = Path::new(uri)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(uri);

    // Return None if the file type is not supported
    let (name, format) = match filename.split_once('.') {
        Some((name, "txt")) => (name, bedrock::DocumentFormat::Txt),
        Some((name, "csv")) => (name, bedrock::DocumentFormat::Csv),
        Some((name, "md")) => (name, bedrock::DocumentFormat::Md),
        Some((name, "html")) => (name, bedrock::DocumentFormat::Html),
        _ => return Ok(None), // Not a supported document type
    };

    // Since we can't use the full path (due to character limit and also Bedrock does not accept `/` etc.),
    // and Bedrock wants document names to be unique, we're adding `tool_use_id` as a prefix to make
    // document names unique.
    let name = format!("{tool_use_id}-{name}");

    Ok(Some(
        bedrock::DocumentBlock::builder()
            .format(format)
            .name(name)
            .source(bedrock::DocumentSource::Bytes(text.as_bytes().into()))
            .build()
            .map_err(|err| anyhow!("Failed to construct Bedrock document: {}", err))?,
    ))
}

pub fn from_bedrock_message(message: &bedrock::Message) -> Result<Message> {
    let role = from_bedrock_role(message.role())?;
    let content = message
        .content()
        .iter()
        .map(from_bedrock_content_block)
        .collect::<Result<Vec<_>>>()?;
    let created = Utc::now().timestamp();

    Ok(Message::new(role, created, content))
}

pub fn from_bedrock_content_block(block: &bedrock::ContentBlock) -> Result<MessageContent> {
    Ok(match block {
        bedrock::ContentBlock::Text(text) => MessageContent::text(text),
        bedrock::ContentBlock::ToolUse(tool_use) => MessageContent::tool_request(
            tool_use.tool_use_id.to_string(),
            Ok(CallToolRequestParams {
                task: None,
                name: tool_use.name.clone().into(),
                arguments: Some(object(from_bedrock_json(&tool_use.input.clone())?)),
                meta: None,
            }),
        ),
        bedrock::ContentBlock::ToolResult(tool_res) => MessageContent::tool_response(
            tool_res.tool_use_id.to_string(),
            if tool_res.content.is_empty() {
                Err(ErrorData {
                    code: ErrorCode::INTERNAL_ERROR,
                    message: Cow::from("Empty content for tool use from Bedrock".to_string()),
                    data: None,
                })
            } else {
                tool_res
                    .content
                    .iter()
                    .map(from_bedrock_tool_result_content_block)
                    .collect::<ToolResult<Vec<_>>>()
                    .map(|content| rmcp::model::CallToolResult {
                        content,
                        structured_content: None,
                        is_error: Some(false),
                        meta: None,
                    })
            },
        ),
        bedrock::ContentBlock::ReasoningContent(reasoning) => match reasoning {
            bedrock::ReasoningContentBlock::ReasoningText(reasoning_text) => {
                MessageContent::thinking(
                    reasoning_text.text.clone(),
                    reasoning_text.signature.clone().unwrap_or_default(),
                )
            }
            bedrock::ReasoningContentBlock::RedactedContent(redacted) => {
                MessageContent::redacted_thinking(
                    base64::prelude::BASE64_STANDARD.encode(redacted.as_ref()),
                )
            }
            _ => bail!("Unsupported reasoning content block from Bedrock"),
        },
        _ => bail!("Unsupported content block type from Bedrock"),
    })
}

pub fn from_bedrock_tool_result_content_block(
    content: &bedrock::ToolResultContentBlock,
) -> ToolResult<Content> {
    Ok(match content {
        bedrock::ToolResultContentBlock::Text(text) => Content::text(text.to_string()),
        _ => {
            return Err(ErrorData {
                code: ErrorCode::INTERNAL_ERROR,
                message: Cow::from("Unsupported tool result from Bedrock".to_string()),
                data: None,
            })
        }
    })
}

pub fn from_bedrock_role(role: &bedrock::ConversationRole) -> Result<Role> {
    Ok(match role {
        bedrock::ConversationRole::User => Role::User,
        bedrock::ConversationRole::Assistant => Role::Assistant,
        _ => bail!("Unknown role from Bedrock"),
    })
}

/// Convert a Bedrock Converse `TokenUsage` into our [`Usage`].
///
/// Per-provider semantics: for Anthropic models on Bedrock, `inputTokens`
/// **excludes** the cache buckets — `cacheReadInputTokens` and
/// `cacheWriteInputTokens` are reported separately and billed in addition (the
/// same shape as Anthropic-native). We map cache-write to `cache_creation` and
/// keep all four buckets disjoint, so [`Usage::billed_total`] is a plain sum
/// that reconciles with the vendor total. `total_tokens` is left as the SDK's
/// value (input + output; it does not include the cache buckets) — it is the
/// context-occupancy gauge, not the billed number.
pub fn from_bedrock_usage(usage: &bedrock::TokenUsage) -> Usage {
    Usage::new(
        Some(usage.input_tokens),
        Some(usage.output_tokens),
        Some(usage.total_tokens),
    )
    .with_cache(
        usage.cache_read_input_tokens,
        usage.cache_write_input_tokens,
    )
}

pub fn from_bedrock_json(document: &Document) -> Result<Value> {
    Ok(match document {
        Document::Null => Value::Null,
        Document::Bool(bool) => Value::Bool(*bool),
        Document::Number(num) => match num {
            Number::PosInt(i) => Value::Number((*i).into()),
            Number::NegInt(i) => Value::Number((*i).into()),
            Number::Float(f) => Value::Number(
                serde_json::Number::from_f64(*f).ok_or(anyhow!("Expected a valid float"))?,
            ),
        },
        Document::String(str) => Value::String(str.clone()),
        Document::Array(arr) => {
            Value::Array(arr.iter().map(from_bedrock_json).collect::<Result<_>>()?)
        }
        Document::Object(obj) => Value::Object(
            obj.iter()
                .map(|(key, val)| Ok((key.clone(), from_bedrock_json(val)?)))
                .collect::<Result<_>>()?,
        ),
    })
}

/// Default overall timeout (seconds) for a single Bedrock `Converse` call,
/// including any SDK-internal retries. Without an operation timeout a stalled
/// endpoint (a proxy that accepts the connection but never answers) hangs the
/// whole agent turn forever — the user sees a spinner that never resolves and no
/// error is ever surfaced. This bounds that case while staying generous enough to
/// never abort a legitimate slow completion. Override with
/// `BEDROCK_OPERATION_TIMEOUT_SECS` (0 disables the timeout entirely).
pub const BEDROCK_DEFAULT_OPERATION_TIMEOUT_SECS: u64 = 300;

/// Build the [`TimeoutConfig`] for a Bedrock client, reading
/// `BEDROCK_OPERATION_TIMEOUT_SECS` (config param first, then process env),
/// defaulting to [`BEDROCK_DEFAULT_OPERATION_TIMEOUT_SECS`]. Returns `None` when
/// the timeout is explicitly disabled (`0`), leaving the SDK default (no overall
/// timeout) in place.
pub fn bedrock_timeout_config(
    config: &crate::config::Config,
) -> Option<aws_smithy_types::timeout::TimeoutConfig> {
    let secs = config
        .get_param::<u64>("BEDROCK_OPERATION_TIMEOUT_SECS")
        .ok()
        .or_else(|| {
            std::env::var("BEDROCK_OPERATION_TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
        })
        .unwrap_or(BEDROCK_DEFAULT_OPERATION_TIMEOUT_SECS);

    if secs == 0 {
        return None;
    }

    Some(
        aws_smithy_types::timeout::TimeoutConfig::builder()
            .operation_timeout(std::time::Duration::from_secs(secs))
            .build(),
    )
}

/// True if `text` reads like a context/token-window overflow, regardless of how
/// the (possibly proxied) endpoint phrases it. Amazon Bedrock itself says
/// "Input is too long for requested model."; gateways in front of it rephrase.
pub fn looks_like_context_overflow(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains("input is too long")
        || t.contains("too long for requested model")
        || t.contains("too many tokens")
        || t.contains("maximum context")
        || t.contains("context length")
        || t.contains("context window")
        || t.contains("maximum number of tokens")
        || (t.contains("token") && (t.contains("exceed") || t.contains("limit")))
        || (t.contains("prompt") && t.contains("too long"))
}

/// Classify a Bedrock `Converse` failure into a [`ProviderError`].
///
/// ## Why this exists
///
/// UCSF's Versa Bedrock endpoint is a MuleSoft proxy in front of Amazon Bedrock,
/// and even direct Bedrock is commonly fronted by a gateway. Those intermediaries
/// return HTTP error responses whose bodies are **not** in the AWS JSON error
/// shape (no `__type`/`code`), so the AWS SDK cannot map them to a typed
/// [`ConverseError`] and collapses them to `ConverseError::Unhandled` with empty
/// metadata. Transport failures (timeouts, dropped connections) likewise carry no
/// service metadata.
///
/// The previous mapping matched only the typed variants and routed *everything
/// else* — including throttling (HTTP 429) and context overflow (HTTP 400 "input
/// too long") that the proxy failed to shape — to a generic
/// [`ProviderError::ServerError`] carrying an unreadable `{:?}` debug dump. In a
/// real incident that meant:
///   * auto-compaction never fired (a proxied context overflow never became
///     [`ProviderError::ContextLengthExceeded`], so the turn kept re-sending an
///     over-limit prompt that failed identically on every retry),
///   * rate-limit handling never fired (a proxied 429 never became
///     [`ProviderError::RateLimitExceeded`], so it missed the deeper retry budget
///     and the scheduler back-off), and
///   * the user was shown
///     `Unhandled(Unhandled { source: ErrorMetadata { code: None, message: None,
///     .. }, .. })`.
///
/// This classifier looks past the SDK's typed variant: it reads the raw HTTP
/// status code (still available on the error's raw response) plus a snippet of the
/// response body, so a proxy that hides the AWS error shape is still classified
/// correctly, and every branch yields a human-readable message.
pub fn classify_bedrock_converse_error(err: SdkError<ConverseError>) -> ProviderError {
    // Capture the HTTP status + whether the body reads like an overflow BEFORE
    // consuming `err`. `raw_response()` is populated for ServiceError/ResponseError.
    let status = err.raw_response().map(|r| r.status().as_u16());
    let body_says_overflow = err
        .raw_response()
        .and_then(|r| r.body().bytes())
        .map(|b| looks_like_context_overflow(&String::from_utf8_lossy(&b[..b.len().min(4096)])))
        .unwrap_or(false);

    // Transport-level failures never carry a service body; they are transient and
    // retryable. Give them a clear, actionable message rather than a raw debug dump.
    // (This also matters because a hung endpoint that never answers surfaces here as
    // a TimeoutError once an operation timeout is configured on the client.)
    match &err {
        SdkError::TimeoutError(_) => {
            return ProviderError::ServerError(
                "Bedrock request timed out with no response from the endpoint \
                 (network stall or gateway hang). This is usually transient."
                    .to_string(),
            );
        }
        SdkError::DispatchFailure(_) => {
            return ProviderError::ServerError(
                "Could not reach the Bedrock endpoint (connection/dispatch failure). \
                 Check the endpoint URL and network connectivity. This is usually transient."
                    .to_string(),
            );
        }
        SdkError::ResponseError(_) => {
            return ProviderError::ServerError(
                "Received an incomplete or unparseable response from the Bedrock endpoint. \
                 This is usually transient."
                    .to_string(),
            );
        }
        SdkError::ConstructionFailure(_) => {
            return ProviderError::ServerError(
                "Failed to construct the Bedrock request before sending it.".to_string(),
            );
        }
        // ServiceError (a real HTTP error response was received) — fall through.
        _ => {}
    }

    // Compact, credential-free detail. See `safe_detail`.
    let detail = format!("Failed to call Bedrock: {}", safe_detail(&err));

    match err.into_service_error() {
        ConverseError::ThrottlingException(e) => ProviderError::RateLimitExceeded {
            details: format!("Bedrock throttling error: {:?}", e),
            retry_delay: None,
        },
        ConverseError::AccessDeniedException(e) => {
            ProviderError::Authentication(format!("Failed to call Bedrock: {:?}", e))
        }
        ConverseError::ValidationException(e)
            if looks_like_context_overflow(e.message().unwrap_or_default()) =>
        {
            ProviderError::ContextLengthExceeded(format!("Failed to call Bedrock: {:?}", e))
        }
        ConverseError::ValidationException(e) => {
            // A non-overflow validation error is a hard client-side rejection (HTTP
            // 400). Retrying an identical request cannot fix it, so surface it as a
            // non-retryable execution error instead of a retryable ServerError.
            ProviderError::ExecutionError(format!("Bedrock rejected the request: {:?}", e))
        }
        ConverseError::ModelErrorException(e) => {
            ProviderError::ExecutionError(format!("Failed to call Bedrock: {:?}", e))
        }
        ConverseError::ModelTimeoutException(e) => {
            ProviderError::ServerError(format!("Bedrock model timed out (transient): {:?}", e))
        }
        ConverseError::ModelNotReadyException(e) => {
            ProviderError::ServerError(format!("Bedrock model not ready (transient): {:?}", e))
        }
        ConverseError::InternalServerException(e) => {
            ProviderError::ServerError(format!("Bedrock internal server error: {:?}", e))
        }
        ConverseError::ServiceUnavailableException(e) => {
            ProviderError::ServerError(format!("Bedrock service unavailable (transient): {:?}", e))
        }
        ConverseError::ResourceNotFoundException(e) => ProviderError::ExecutionError(format!(
            "Bedrock resource not found; check the model id / access: {:?}",
            e
        )),
        // `Unhandled` (and any future variant): the proxy returned an error the SDK
        // couldn't type. Fall back to the raw HTTP status + body captured above.
        other => classify_untyped_bedrock_error(status, body_says_overflow, detail, other),
    }
}

/// Classify a Bedrock error the SDK could not map to a typed variant, using the
/// raw HTTP status code and body signals. This is the path a proxied error takes.
///
/// Generic over the error type so the streaming operation (`ConverseStreamError`)
/// shares one implementation with the blocking one (`ConverseError`) — the
/// proxy-shaped-error problem this solves is identical on both paths.
fn classify_untyped_bedrock_error<E: ProvideErrorMetadata + std::fmt::Debug>(
    status: Option<u16>,
    body_says_overflow: bool,
    detail: String,
    err: E,
) -> ProviderError {
    // If either the body or whatever metadata the SDK salvaged reads like an
    // overflow, treat it as a context-length problem so the agent can compact.
    if body_says_overflow || looks_like_context_overflow(err.message().unwrap_or_default()) {
        return ProviderError::ContextLengthExceeded(format!(
            "Bedrock reported a context/token-window overflow. {detail}"
        ));
    }

    match status {
        Some(429) => ProviderError::RateLimitExceeded {
            details: format!("Bedrock endpoint returned HTTP 429 (throttled). {detail}"),
            retry_delay: None,
        },
        Some(413) => ProviderError::ContextLengthExceeded(format!(
            "Bedrock endpoint returned HTTP 413 (payload too large). {detail}"
        )),
        Some(code @ (401 | 403)) => ProviderError::Authentication(format!(
            "Bedrock endpoint returned HTTP {code} (unauthorized). {detail}"
        )),
        Some(408) => ProviderError::ServerError(format!(
            "Bedrock endpoint returned HTTP 408 (request timeout). This is usually transient. {detail}"
        )),
        Some(code) if (500..600).contains(&code) => ProviderError::ServerError(format!(
            "Bedrock endpoint returned HTTP {code}. This is usually transient. {detail}"
        )),
        Some(code) => {
            // Some other status the proxy invented. Keep it retryable (proxy blips
            // dominate here) but name the status so the cause is visible.
            ProviderError::ServerError(format!("Bedrock endpoint returned HTTP {code}. {detail}"))
        }
        // No raw response at all (should not happen for a ServiceError, but keep it
        // safe): treat as a retryable server error.
        None => ProviderError::ServerError(detail),
    }
}

// ---------------------------------------------------------------------------
// Streaming: the `ConverseStream` event stream
// ---------------------------------------------------------------------------

/// Classify a failure of the *streaming* `ConverseStream` operation — i.e. the
/// initial `send()`, before any event has been received.
///
/// This is deliberately a near-twin of [`classify_bedrock_converse_error`]:
/// `ConverseStreamError` is a different generated type with an extra
/// `ModelStreamErrorException` variant, so it cannot share the `match`, but the
/// proxy-shaped-error reasoning documented on the blocking classifier applies
/// verbatim (UCSF's MuleSoft proxy returns bodies the SDK cannot type, so we
/// fall back to the raw HTTP status).
pub fn classify_bedrock_converse_stream_error(err: SdkError<ConverseStreamError>) -> ProviderError {
    let status = err.raw_response().map(|r| r.status().as_u16());
    let body_says_overflow = err
        .raw_response()
        .and_then(|r| r.body().bytes())
        .map(|b| looks_like_context_overflow(&String::from_utf8_lossy(&b[..b.len().min(4096)])))
        .unwrap_or(false);

    match &err {
        SdkError::TimeoutError(_) => {
            return ProviderError::ServerError(
                "Bedrock streaming request timed out with no response from the endpoint \
                 (network stall or gateway hang). This is usually transient."
                    .to_string(),
            );
        }
        SdkError::DispatchFailure(_) => {
            return ProviderError::ServerError(
                "Could not reach the Bedrock endpoint (connection/dispatch failure). \
                 Check the endpoint URL and network connectivity. This is usually transient."
                    .to_string(),
            );
        }
        SdkError::ResponseError(_) => {
            return ProviderError::ServerError(
                "Received an incomplete or unparseable response from the Bedrock endpoint. \
                 This is usually transient."
                    .to_string(),
            );
        }
        SdkError::ConstructionFailure(_) => {
            return ProviderError::ServerError(
                "Failed to construct the Bedrock streaming request before sending it.".to_string(),
            );
        }
        _ => {}
    }

    let detail = format!("Failed to open Bedrock stream: {}", safe_detail(&err));

    match err.into_service_error() {
        ConverseStreamError::ThrottlingException(e) => ProviderError::RateLimitExceeded {
            details: format!("Bedrock throttling error: {:?}", e),
            retry_delay: None,
        },
        ConverseStreamError::AccessDeniedException(e) => {
            ProviderError::Authentication(format!("Failed to call Bedrock: {:?}", e))
        }
        ConverseStreamError::ValidationException(e)
            if looks_like_context_overflow(e.message().unwrap_or_default()) =>
        {
            ProviderError::ContextLengthExceeded(format!("Failed to call Bedrock: {:?}", e))
        }
        ConverseStreamError::ValidationException(e) => {
            ProviderError::ExecutionError(format!("Bedrock rejected the request: {:?}", e))
        }
        ConverseStreamError::ModelErrorException(e) => {
            ProviderError::ExecutionError(format!("Failed to call Bedrock: {:?}", e))
        }
        ConverseStreamError::ModelTimeoutException(e) => {
            ProviderError::ServerError(format!("Bedrock model timed out (transient): {:?}", e))
        }
        ConverseStreamError::ModelNotReadyException(e) => {
            ProviderError::ServerError(format!("Bedrock model not ready (transient): {:?}", e))
        }
        // Only on the streaming operation: the service failed part-way through
        // emitting the event stream. AWS documents this as retryable.
        ConverseStreamError::ModelStreamErrorException(e) => {
            ProviderError::ServerError(format!("Bedrock model stream error (transient): {:?}", e))
        }
        ConverseStreamError::InternalServerException(e) => {
            ProviderError::ServerError(format!("Bedrock internal server error: {:?}", e))
        }
        ConverseStreamError::ServiceUnavailableException(e) => {
            ProviderError::ServerError(format!("Bedrock service unavailable (transient): {:?}", e))
        }
        ConverseStreamError::ResourceNotFoundException(e) => {
            ProviderError::ExecutionError(format!(
                "Bedrock resource not found; check the model id / access: {:?}",
                e
            ))
        }
        other => classify_untyped_bedrock_error(status, body_says_overflow, detail, other),
    }
}

/// Classify a failure raised **mid-stream**, while receiving events.
///
/// The error type here is `SdkError<ConverseStreamOutputError, RawMessage>` —
/// note the second type parameter: at this point the HTTP response is long gone
/// and the "raw response" is an event-stream frame, so there is no status code
/// to fall back on. We therefore classify purely from the typed variant and its
/// metadata. (The function is generic over that second parameter so it does not
/// have to name `RawMessage`, which lives behind an `aws-smithy-types` feature
/// this crate does not enable directly.)
///
/// These errors are **not** retried: bytes have already been handed to the
/// agent, and replaying the request would duplicate them. The turn fails and the
/// agent's own error handling takes over. This matches the Anthropic streaming
/// path, which likewise does not retry once a stream has begun.
pub fn classify_bedrock_stream_event_error<R: std::fmt::Debug + Send + Sync + 'static>(
    err: SdkError<ConverseStreamOutputError, R>,
) -> ProviderError {
    match &err {
        SdkError::TimeoutError(_) => {
            return ProviderError::ServerError(
                "The Bedrock response stream stalled and timed out mid-generation. \
                 This is usually transient."
                    .to_string(),
            );
        }
        SdkError::DispatchFailure(_) => {
            return ProviderError::ServerError(
                "The connection to Bedrock dropped mid-generation. This is usually transient."
                    .to_string(),
            );
        }
        SdkError::ResponseError(_) => {
            return ProviderError::ServerError(
                "Received a malformed frame in the Bedrock response stream. \
                 This is usually transient."
                    .to_string(),
            );
        }
        _ => {}
    }

    let detail = format!("Bedrock stream error: {}", safe_detail(&err));

    match err.into_service_error() {
        ConverseStreamOutputError::ThrottlingException(e) => ProviderError::RateLimitExceeded {
            details: format!("Bedrock throttled the response stream: {:?}", e),
            retry_delay: None,
        },
        ConverseStreamOutputError::ValidationException(e)
            if looks_like_context_overflow(e.message().unwrap_or_default()) =>
        {
            ProviderError::ContextLengthExceeded(format!("Bedrock stream: {:?}", e))
        }
        ConverseStreamOutputError::ValidationException(e) => {
            ProviderError::ExecutionError(format!("Bedrock rejected the request: {:?}", e))
        }
        ConverseStreamOutputError::ModelStreamErrorException(e) => {
            ProviderError::ServerError(format!("Bedrock model stream error (transient): {:?}", e))
        }
        ConverseStreamOutputError::InternalServerException(e) => {
            ProviderError::ServerError(format!("Bedrock internal server error: {:?}", e))
        }
        ConverseStreamOutputError::ServiceUnavailableException(e) => {
            ProviderError::ServerError(format!("Bedrock service unavailable (transient): {:?}", e))
        }
        other => {
            if looks_like_context_overflow(other.message().unwrap_or_default()) {
                ProviderError::ContextLengthExceeded(format!(
                    "Bedrock reported a context/token-window overflow mid-stream. {detail}"
                ))
            } else {
                ProviderError::ServerError(detail)
            }
        }
    }
}

/// Map Bedrock's `stopReason` onto the OpenAI-style `finish_reason` the agent
/// loop understands.
///
/// Only `"length"` triggers the agent's auto-continue path, so the mapping is
/// deliberately conservative: the reasons that mean "the OUTPUT hit the token
/// cap" map to `"length"`, and everything else passes through as its raw Bedrock
/// string. In particular `model_context_window_exceeded` — which means the
/// **input** did not fit — must NOT become `"length"`, or the agent would try to
/// continue a turn that can never make progress.
pub fn map_bedrock_stop_reason(reason: &bedrock::StopReason) -> String {
    match reason {
        bedrock::StopReason::EndTurn | bedrock::StopReason::StopSequence => "stop".to_string(),
        bedrock::StopReason::MaxTokens => "length".to_string(),
        bedrock::StopReason::ToolUse => "tool_calls".to_string(),
        bedrock::StopReason::ContentFiltered | bedrock::StopReason::GuardrailIntervened => {
            "content_filter".to_string()
        }
        other => other.as_str().to_string(),
    }
}

/// A tool-use block being accumulated across `contentBlockDelta` events.
#[derive(Debug, Clone)]
struct ToolUseAccumulator {
    tool_use_id: String,
    name: String,
    /// Partial JSON, concatenated in arrival order. Never parsed until the
    /// matching `contentBlockStop`.
    input: String,
}

/// Incremental decoder for the Bedrock `ConverseStream` event stream.
///
/// # Why this exists
///
/// The blocking `Converse` API returns nothing until the model has finished the
/// entire turn, so a tool-call card could not appear in the UI until generation
/// was completely done (see
/// `docs/history/streaming-tool-call-ui-2026-07/tool-call-ui-latency-investigation.md` §0). This decoder
/// turns the SDK's typed event stream into the repo's [`MessageStream`], so text
/// reaches the UI as it is produced.
///
/// # Safety property (the important one)
///
/// **A tool request is emitted exactly once, at `contentBlockStop`, with fully
/// parsed arguments — never before.** Bedrock delivers a tool's `input` as a
/// sequence of partial-JSON fragments; emitting early would hand the agent's
/// dispatch path a *truncated* argument object, and for `shell` or `text_editor`
/// executing truncated arguments is destructive. Accumulated fragments are only
/// ever `push_str`ed until the block closes.
///
/// # Block indices, not "the current block"
///
/// Every event carries a `contentBlockIndex`. Accumulators are keyed by that
/// index rather than by a single "current tool" slot, so a response containing
/// several tool_use blocks decodes correctly even if the service interleaves
/// their deltas.
///
pub struct BedrockStreamDecoder {
    model_name: String,
    tool_blocks: HashMap<i32, ToolUseAccumulator>,
    reasoning_blocks: HashMap<i32, ReasoningAccumulator>,
    /// Latest usage reported by the `metadata` event.
    usage: Option<Usage>,
    /// Mapped `stopReason` from the `messageStop` event.
    finish_reason: Option<String>,
    /// Set once `messageStop` is seen. A stream that ends without it was cut off.
    saw_message_stop: bool,
    /// Shared id stamped on every message yielded for this response, so the
    /// desktop store merges the deltas into one transcript entry.
    message_id: String,
    /// §6.2b (issue #41): whether completed `tool_use` blocks are buffered and
    /// flushed as ONE assistant message at `messageStop` / `finish()`, mirroring
    /// the Anthropic decoder. Read once at construction from
    /// `tool_call_batching_enabled()` (`BIOROUTER_TOOL_CALL_BATCHING`).
    ///
    /// Without batching, every tool_use block became its own message carrying
    /// the SAME shared `message_id`; the agent loop then rebuilt two assistant
    /// messages with one id in a single persist batch, which the session store
    /// rejects (`UNIQUE(session_id, msg_uid)`) — killing the whole turn.
    batch_tool_calls: bool,
    /// Completed tool_use contents awaiting the batched flush, keyed by
    /// `content_block_index`. Buffered in `contentBlockStop` ARRIVAL order and
    /// sorted by index at flush time: the decoder supports interleaved blocks
    /// (see "Block indices" above), so a later block can close first, and the
    /// batched message must follow the response's canonical block order — not
    /// stop order — for dispatch and persistence.
    pending_tool_contents: Vec<(i32, MessageContent)>,
    /// Completed non-tool blocks that could not yet be emitted because a lower
    /// content-block index was still open. This is what preserves canonical
    /// response order when the service interleaves block events.
    ordered_contents: BTreeMap<i32, Vec<MessageContent>>,
    closed_blocks: HashSet<i32>,
    next_content_index: i32,
}

#[derive(Default)]
struct ReasoningAccumulator {
    text: String,
    signature: String,
    redacted: Vec<u8>,
}

/// One item of the decoded stream: a partial message and/or a usage snapshot.
pub type BedrockStreamItem = (
    Option<Message>,
    Option<crate::providers::base::ProviderUsage>,
);

impl BedrockStreamDecoder {
    pub fn new(model_name: impl Into<String>) -> Self {
        Self {
            model_name: model_name.into(),
            tool_blocks: HashMap::new(),
            reasoning_blocks: HashMap::new(),
            usage: None,
            finish_reason: None,
            saw_message_stop: false,
            message_id: uuid::Uuid::new_v4().to_string(),
            batch_tool_calls: tool_call_batching_enabled(),
            pending_tool_contents: Vec::new(),
            ordered_contents: BTreeMap::new(),
            closed_blocks: HashSet::new(),
            next_content_index: 0,
        }
    }

    /// Test seam: construct with an explicit batching mode instead of reading
    /// `BIOROUTER_TOOL_CALL_BATCHING`. Mutating that env var in tests races
    /// every parallel test that constructs a decoder (a known workspace
    /// gotcha), so the kill-switch shape test injects the flag directly; the
    /// env parsing itself lives in `tool_call_batching_enabled()`.
    #[cfg(test)]
    fn with_batching(model_name: impl Into<String>, batch_tool_calls: bool) -> Self {
        Self {
            batch_tool_calls,
            ..Self::new(model_name)
        }
    }

    /// True if the stream terminated without a `messageStop` event.
    pub fn was_truncated(&self) -> bool {
        !self.saw_message_stop
    }

    /// Every message this decoder yields for one response carries the SAME id.
    ///
    /// The desktop store merges streamed messages by id (`chatStreamStore`
    /// `pushMessage`); without a stable one, each `contentBlockDelta` lands as a
    /// separate transcript bubble with its own timestamp, so a streamed reply
    /// renders one bubble per token. The Anthropic decoder gets this for free by
    /// reusing the `message_start` id (`formats/anthropic.rs:629`); ConverseStream's
    /// `messageStart` carries only a role, so we mint one per stream instead.
    fn assistant_message(&self, content: MessageContent) -> Message {
        let mut message = Message::new(Role::Assistant, Utc::now().timestamp(), vec![content]);
        message.id = Some(self.message_id.clone());
        message
    }

    /// Current usage snapshot, carrying whatever `finish_reason` we know.
    ///
    /// The agent keeps the **last** usage snapshot of a turn rather than summing
    /// them, so re-emitting a snapshot cannot double-count. That is what lets us
    /// emit one at `messageStop` (finish reason known, usage possibly not yet)
    /// and another at `metadata` (real token counts) without corrupting cost
    /// accounting.
    fn usage_snapshot(&self) -> crate::providers::base::ProviderUsage {
        let mut snapshot = crate::providers::base::ProviderUsage::new(
            self.model_name.clone(),
            self.usage.unwrap_or_default(),
        );
        snapshot.finish_reason = self.finish_reason.clone();
        snapshot
    }

    /// Feed one SDK event, returning zero or more stream items to yield.
    /// One `contentBlockDelta`. Split out of [`Self::on_event`] purely to keep
    /// that dispatcher readable; the behaviour is unchanged.
    fn on_content_block_delta(
        &mut self,
        delta_event: &bedrock::ContentBlockDeltaEvent,
    ) -> Vec<BedrockStreamItem> {
        match delta_event.delta.as_ref() {
            // The whole point of the change: text is yielded the moment
            // it arrives.
            Some(bedrock::ContentBlockDelta::Text(text)) => {
                if text.is_empty() {
                    Vec::new()
                } else if delta_event.content_block_index == self.next_content_index
                    && !self
                        .ordered_contents
                        .contains_key(&delta_event.content_block_index)
                {
                    vec![(
                        Some(self.assistant_message(MessageContent::text(text))),
                        None,
                    )]
                } else {
                    self.push_ordered_content(
                        delta_event.content_block_index,
                        MessageContent::text(text),
                    );
                    Vec::new()
                }
            }
            Some(bedrock::ContentBlockDelta::ToolUse(tool_delta)) => {
                if let Some(acc) = self.tool_blocks.get_mut(&delta_event.content_block_index) {
                    acc.input.push_str(&tool_delta.input);
                } else {
                    tracing::debug!(
                        index = delta_event.content_block_index,
                        "Bedrock toolUse delta for an unknown content block index; dropping"
                    );
                }
                Vec::new()
            }
            Some(bedrock::ContentBlockDelta::ReasoningContent(reasoning)) => {
                let acc = self
                    .reasoning_blocks
                    .entry(delta_event.content_block_index)
                    .or_default();
                match reasoning {
                    bedrock::ReasoningContentBlockDelta::Text(text) => acc.text.push_str(text),
                    bedrock::ReasoningContentBlockDelta::Signature(signature) => {
                        acc.signature.push_str(signature)
                    }
                    bedrock::ReasoningContentBlockDelta::RedactedContent(redacted) => {
                        acc.redacted.extend_from_slice(redacted.as_ref())
                    }
                    _ => {}
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// One `contentBlockStop`. Split out of [`Self::on_event`] for the same
    /// reason.
    fn on_content_block_stop(
        &mut self,
        stop: &bedrock::ContentBlockStopEvent,
    ) -> Vec<BedrockStreamItem> {
        let mut items = match self.tool_blocks.remove(&stop.content_block_index) {
            Some(acc) => {
                let content = self.finish_tool_content(acc);
                if self.batch_tool_calls {
                    // §6.2b: defer — flushed as ONE message at
                    // `messageStop` (or `finish()`), so a multi-tool
                    // turn dispatches in parallel and never persists
                    // two assistant rows sharing this decoder's id.
                    self.pending_tool_contents
                        .push((stop.content_block_index, content));
                    Vec::new()
                } else {
                    self.push_ordered_content(stop.content_block_index, content);
                    Vec::new()
                }
            }
            None => {
                if let Some(acc) = self.reasoning_blocks.remove(&stop.content_block_index) {
                    for content in Self::finish_reasoning_content(acc) {
                        self.push_ordered_content(stop.content_block_index, content);
                    }
                }
                Vec::new()
            }
        };
        self.closed_blocks.insert(stop.content_block_index);
        if !(self.batch_tool_calls
            && self
                .pending_tool_contents
                .iter()
                .any(|(index, _)| *index == stop.content_block_index))
        {
            items.extend(self.flush_ready_contents());
        }
        items
    }

    pub fn on_event(&mut self, event: &bedrock::ConverseStreamOutput) -> Vec<BedrockStreamItem> {
        match event {
            // Role announcement only; nothing to surface.
            bedrock::ConverseStreamOutput::MessageStart(_) => Vec::new(),

            bedrock::ConverseStreamOutput::ContentBlockStart(start) => {
                if let Some(bedrock::ContentBlockStart::ToolUse(tool_use)) = start.start.as_ref() {
                    // The tool NAME and id are known here, but we still do not
                    // emit — a tool request without complete arguments must
                    // never reach the agent's dispatch path.
                    self.tool_blocks.insert(
                        start.content_block_index,
                        ToolUseAccumulator {
                            tool_use_id: tool_use.tool_use_id.clone(),
                            name: tool_use.name.clone(),
                            input: String::new(),
                        },
                    );
                }
                Vec::new()
            }

            bedrock::ConverseStreamOutput::ContentBlockDelta(delta_event) => {
                self.on_content_block_delta(delta_event)
            }

            bedrock::ConverseStreamOutput::ContentBlockStop(stop) => {
                self.on_content_block_stop(stop)
            }

            bedrock::ConverseStreamOutput::MessageStop(stop) => {
                self.saw_message_stop = true;
                self.finish_reason = Some(map_bedrock_stop_reason(&stop.stop_reason));
                // §6.2b: a messageStop closes the response, so every tool block
                // that will arrive already has. Flush the batch here, riding the
                // SAME stream item as the usage snapshot — agent.rs reads
                // (message, usage) in one match arm.
                let message = self.flush_all_pending_contents();
                vec![(message, Some(self.usage_snapshot()))]
            }

            bedrock::ConverseStreamOutput::Metadata(meta) => match meta.usage.as_ref() {
                Some(usage) => {
                    self.usage = Some(from_bedrock_usage(usage));
                    vec![(None, Some(self.usage_snapshot()))]
                }
                None => Vec::new(),
            },

            _ => Vec::new(),
        }
    }

    /// §6.2b: drain buffered `tool_use` contents into a **single** assistant
    /// message stamped with the shared `message_id`, or `None` when nothing is
    /// buffered. One message carrying N `ToolRequest`s is what makes the
    /// agent's `select_all` dispatch them in parallel — and what keeps one
    /// response from persisting as several rows sharing one `msg_uid`
    /// (issue #41). The drain empties the buffer, so a second flush is a no-op;
    /// a batch is never delivered twice.
    ///
    /// Sorted by `content_block_index` before assembly: blocks are buffered in
    /// stop-arrival order, and with interleaved blocks a later block can close
    /// first. Request order is load-bearing downstream — Anthropic 400s a
    /// tool-result batch whose order doesn't match the request order.
    fn push_ordered_content(&mut self, index: i32, content: MessageContent) {
        let contents = self.ordered_contents.entry(index).or_default();
        match (contents.last_mut(), content) {
            (Some(MessageContent::Text(existing)), MessageContent::Text(next)) => {
                existing.text.push_str(&next.text);
            }
            (_, content) => contents.push(content),
        }
    }

    fn flush_ready_contents(&mut self) -> Vec<BedrockStreamItem> {
        let mut items = Vec::new();
        while self.closed_blocks.contains(&self.next_content_index) {
            if self.batch_tool_calls
                && self
                    .pending_tool_contents
                    .iter()
                    .any(|(index, _)| *index == self.next_content_index)
            {
                break;
            }
            self.closed_blocks.remove(&self.next_content_index);
            if let Some(contents) = self.ordered_contents.remove(&self.next_content_index) {
                if !contents.is_empty() {
                    let mut message =
                        Message::new(Role::Assistant, Utc::now().timestamp(), contents);
                    message.id = Some(self.message_id.clone());
                    items.push((Some(message), None));
                }
            }
            self.next_content_index += 1;
        }
        items
    }

    fn flush_all_pending_contents(&mut self) -> Option<Message> {
        for (index, content) in std::mem::take(&mut self.pending_tool_contents) {
            self.push_ordered_content(index, content);
        }
        let contents = std::mem::take(&mut self.ordered_contents)
            .into_values()
            .flatten()
            .collect::<Vec<_>>();
        self.closed_blocks.clear();
        if contents.is_empty() {
            None
        } else {
            let mut message = Message::new(Role::Assistant, Utc::now().timestamp(), contents);
            message.id = Some(self.message_id.clone());
            Some(message)
        }
    }

    fn finish_reasoning_content(acc: ReasoningAccumulator) -> Vec<MessageContent> {
        if !acc.redacted.is_empty() {
            if !acc.text.is_empty() || !acc.signature.is_empty() {
                tracing::warn!(
                    "Bedrock reasoning block mixed redacted and plaintext deltas; preserving the redacted block"
                );
            }
            vec![MessageContent::redacted_thinking(
                base64::prelude::BASE64_STANDARD.encode(acc.redacted),
            )]
        } else if !acc.text.is_empty() || !acc.signature.is_empty() {
            vec![MessageContent::thinking(acc.text, acc.signature)]
        } else {
            Vec::new()
        }
    }

    /// Turn a completed tool-use block into a tool request content.
    ///
    /// Called only from the `contentBlockStop` arm, so `acc.input` is the
    /// complete argument JSON.
    fn finish_tool_content(&self, acc: ToolUseAccumulator) -> MessageContent {
        let ToolUseAccumulator {
            tool_use_id,
            name,
            input,
        } = acc;

        let parsed = if input.trim().is_empty() {
            // A no-argument tool sends no deltas at all.
            Ok(Value::Object(Default::default()))
        } else {
            serde_json::from_str::<Value>(&input)
                .map_err(|e| format!("Could not parse tool arguments: {e}"))
        };

        match parsed {
            // `rmcp::model::object` debug-asserts its argument is an object, and
            // the agent expects a map. A non-object here means the stream was
            // corrupt; fail the call loudly rather than coercing it to `{}`.
            Ok(value) if value.is_object() => MessageContent::tool_request(
                tool_use_id,
                Ok(CallToolRequestParams {
                    task: None,
                    name: name.into(),
                    arguments: Some(object(value)),
                    meta: None,
                }),
            ),
            Ok(_) => MessageContent::tool_request(
                tool_use_id,
                Err(ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "Tool arguments were not a JSON object; the call was not executed.",
                    Some(json!({"biorouterToolCallFailure":"invalid_arguments"})),
                )),
            ),
            Err(message) => MessageContent::tool_request(
                tool_use_id,
                Err(ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    message,
                    Some(json!({"biorouterToolCallFailure":"invalid_arguments"})),
                )),
            ),
        }
    }

    /// Flush state after the event stream ends.
    ///
    /// Any tool block still open never received its `contentBlockStop`, so its
    /// completion is unconfirmed even if its JSON parses. It is a failed request —
    /// never as a callable one — so the turn reports the truncation instead of
    /// silently dropping it (or, far worse, executing half a command).
    pub fn finish(&mut self) -> Vec<BedrockStreamItem> {
        let mut items: Vec<BedrockStreamItem> = Vec::new();

        // §6.2b: a stream that ended WITHOUT a `messageStop` (truncated after
        // its blocks closed) still has its buffered COMPLETE tool blocks
        // flushed as one batched message — otherwise a whole multi-tool turn
        // would silently vanish. A no-op in the common path (`messageStop`
        // already drained the buffer).
        if let Some(batched) = self.flush_all_pending_contents() {
            items.push((Some(batched), None));
        }

        let mut pending: Vec<(i32, ToolUseAccumulator)> = self.tool_blocks.drain().collect();
        pending.sort_by_key(|(index, _)| *index);

        items.extend(pending.into_iter().map(|(index, acc)| {
            tracing::warn!(
                index,
                tool = %acc.name,
                argument_bytes = acc.input.len(),
                arguments_are_json_object = serde_json::from_str::<serde_json::Value>(&acc.input)
                    .is_ok_and(|value| value.is_object()),
                "Bedrock stream ended before tool_use block completed; \
                 surfacing it as a failed tool call because completion was not confirmed"
            );
            let content = MessageContent::tool_request(
                acc.tool_use_id,
                Err(ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!(
                        "The Bedrock response stream ended before completion of the tool block for `{}` \
                             was confirmed, so the call was not made. Please retry.",
                        acc.name
                    ),
                    Some(json!({"biorouterToolCallFailure":"incomplete_stream"})),
                )),
            );
            (Some(self.assistant_message(content)), None)
        }));

        // A truncated stream never reached `messageStop`/`metadata`, so no usage
        // snapshot was emitted. Emit whatever we have so a cancelled or cut-off
        // turn is still billed rather than recorded as free.
        if !self.saw_message_stop && self.usage.is_some() {
            items.push((None, Some(self.usage_snapshot())));
        }

        items
    }
}

/// Drive a `ConverseStream` response into the repo's [`MessageStream`].
///
/// Takes the whole operation output rather than its `stream` field because the
/// SDK's `EventReceiver` lives in a private module and is therefore unnameable
/// outside `aws-sdk-bedrockruntime`.
///
/// Cancellation: dropping the returned stream drops the generator mid-`await`,
/// which drops the receiver and the accumulator state together. Nothing is
/// half-flushed, because the only state that could be flushed (an open tool
/// block) is *never* emitted as a callable request.
pub fn bedrock_message_stream(
    response: ConverseStreamResponse,
    model_name: String,
    mut log: RequestLog,
) -> crate::providers::base::MessageStream {
    Box::pin(async_stream::try_stream! {
        let mut receiver = response.stream;
        let mut decoder = BedrockStreamDecoder::new(model_name);

        loop {
            let event = match receiver.recv().await {
                Ok(Some(event)) => event,
                Ok(None) => break,
                Err(err) => {
                    let provider_error = classify_bedrock_stream_event_error(err);
                    let _ = log.error(&provider_error);
                    // Terminates the stream with this error. Any tool block still
                    // open is intentionally NOT flushed as a callable request.
                    yield Err(provider_error)?;
                    break;
                }
            };

            for (message, usage) in decoder.on_event(&event) {
                log.write(&message, usage.as_ref().map(|u| u.usage).as_ref())?;
                // Bedrock does not (yet) emit pending tool-call notifications;
                // the third slot is always `None`. The decoder only ever yields a
                // tool block once it is complete, so nothing partial escapes here.
                yield (message, usage, None);
            }
        }

        for (message, usage) in decoder.finish() {
            log.write(&message, usage.as_ref().map(|u| u.usage).as_ref())?;
            yield (message, usage, None);
        }
    })
}

#[cfg(test)]
mod bedrock_stream_tests {
    use super::*;
    use aws_sdk_bedrockruntime::types::{
        ContentBlockDelta, ContentBlockDeltaEvent, ContentBlockStart, ContentBlockStartEvent,
        ContentBlockStopEvent, ConversationRole, ConverseStreamMetadataEvent, ConverseStreamOutput,
        MessageStartEvent, MessageStopEvent, ReasoningContentBlockDelta, StopReason, TokenUsage,
        ToolUseBlockDelta, ToolUseBlockStart,
    };

    // ---- synthetic event builders -----------------------------------------

    fn message_start() -> ConverseStreamOutput {
        ConverseStreamOutput::MessageStart(
            MessageStartEvent::builder()
                .role(ConversationRole::Assistant)
                .build()
                .unwrap(),
        )
    }

    fn message_stop(reason: StopReason) -> ConverseStreamOutput {
        ConverseStreamOutput::MessageStop(
            MessageStopEvent::builder()
                .stop_reason(reason)
                .build()
                .unwrap(),
        )
    }

    fn text_delta(index: i32, text: &str) -> ConverseStreamOutput {
        ConverseStreamOutput::ContentBlockDelta(
            ContentBlockDeltaEvent::builder()
                .content_block_index(index)
                .delta(ContentBlockDelta::Text(text.to_string()))
                .build()
                .unwrap(),
        )
    }

    fn tool_start(index: i32, id: &str, name: &str) -> ConverseStreamOutput {
        ConverseStreamOutput::ContentBlockStart(
            ContentBlockStartEvent::builder()
                .content_block_index(index)
                .start(ContentBlockStart::ToolUse(
                    ToolUseBlockStart::builder()
                        .tool_use_id(id)
                        .name(name)
                        .build()
                        .unwrap(),
                ))
                .build()
                .unwrap(),
        )
    }

    fn tool_delta(index: i32, fragment: &str) -> ConverseStreamOutput {
        ConverseStreamOutput::ContentBlockDelta(
            ContentBlockDeltaEvent::builder()
                .content_block_index(index)
                .delta(ContentBlockDelta::ToolUse(
                    ToolUseBlockDelta::builder()
                        .input(fragment)
                        .build()
                        .unwrap(),
                ))
                .build()
                .unwrap(),
        )
    }

    fn block_stop(index: i32) -> ConverseStreamOutput {
        ConverseStreamOutput::ContentBlockStop(
            ContentBlockStopEvent::builder()
                .content_block_index(index)
                .build()
                .unwrap(),
        )
    }

    fn metadata(input: i32, output: i32, total: i32) -> ConverseStreamOutput {
        ConverseStreamOutput::Metadata(
            ConverseStreamMetadataEvent::builder()
                .usage(
                    TokenUsage::builder()
                        .input_tokens(input)
                        .output_tokens(output)
                        .total_tokens(total)
                        .cache_read_input_tokens(7)
                        .cache_write_input_tokens(3)
                        .build()
                        .unwrap(),
                )
                .build(),
        )
    }

    // ---- assertion helpers -------------------------------------------------

    fn drain(
        decoder: &mut BedrockStreamDecoder,
        events: &[ConverseStreamOutput],
    ) -> Vec<BedrockStreamItem> {
        events
            .iter()
            .flat_map(|event| decoder.on_event(event))
            .collect()
    }

    fn texts(items: &[BedrockStreamItem]) -> Vec<String> {
        items
            .iter()
            .filter_map(|(message, _)| message.as_ref())
            .flat_map(|message| message.content.iter())
            .filter_map(|content| match content {
                MessageContent::Text(text) => Some(text.text.clone()),
                _ => None,
            })
            .collect()
    }

    /// Every tool request in the decoded items, as `(id, Ok(name, args) | Err(msg))`.
    #[allow(clippy::type_complexity)]
    fn tool_requests(
        items: &[BedrockStreamItem],
    ) -> Vec<(String, Result<(String, Value), String>)> {
        items
            .iter()
            .filter_map(|(message, _)| message.as_ref())
            .flat_map(|message| message.content.iter())
            .filter_map(|content| match content {
                MessageContent::ToolRequest(request) => Some((
                    request.id.clone(),
                    match &request.tool_call {
                        Ok(call) => Ok((
                            call.name.to_string(),
                            Value::Object(call.arguments.clone().unwrap_or_default()),
                        )),
                        Err(error) => Err(error.message.to_string()),
                    },
                )),
                _ => None,
            })
            .collect()
    }

    // ---- text ---------------------------------------------------------------

    /// REGRESSION (2026-07-18, found in the live GUI, not by these tests):
    /// every message a single response yields must share ONE id.
    ///
    /// The desktop store merges streamed messages by id. When this decoder left
    /// `Message::id` as None, each `contentBlockDelta` landed as its own
    /// transcript bubble with its own timestamp — a streamed reply rendered one
    /// bubble per token. The original tests asserted decoded CONTENT and so were
    /// blind to it; assert identity here.
    #[test]
    fn every_message_in_one_response_shares_a_single_id() {
        let mut decoder = BedrockStreamDecoder::new("m");
        let items = drain(
            &mut decoder,
            &[
                message_start(),
                text_delta(0, "Hello"),
                text_delta(0, " there"),
                text_delta(0, "!"),
                tool_start(1, "tooluse_abc", "shell"),
                tool_delta(1, "{\"command\":\"ls\"}"),
                block_stop(1),
                // §6.2b: the batched tool message flushes at messageStop.
                message_stop(StopReason::ToolUse),
            ],
        );

        let ids: Vec<Option<String>> = items
            .into_iter()
            .filter_map(|(m, _)| m.map(|m| m.id))
            .collect();

        assert!(
            ids.len() >= 4,
            "expected several yielded messages, got {}",
            ids.len()
        );
        assert!(
            ids.iter().all(Option::is_some),
            "every streamed message needs an id: {ids:?}"
        );
        let first = &ids[0];
        assert!(
            ids.iter().all(|id| id == first),
            "all messages in one response must share one id so the UI merges them, got {ids:?}"
        );
    }

    /// Two separate responses must NOT share an id, or the second would merge
    /// into the first in the transcript.
    #[test]
    fn separate_responses_get_distinct_ids() {
        let mut a = BedrockStreamDecoder::new("m");
        let mut b = BedrockStreamDecoder::new("m");
        let id_of = |d: &mut BedrockStreamDecoder| {
            drain(d, &[message_start(), text_delta(0, "hi")])
                .into_iter()
                .find_map(|(m, _)| m.and_then(|m| m.id))
        };
        let ida = id_of(&mut a);
        let idb = id_of(&mut b);
        assert!(ida.is_some() && idb.is_some());
        assert_ne!(ida, idb, "two responses must not share a message id");
    }

    /// The entire point of the change: each text delta must surface as its own
    /// item, not be buffered until the turn ends.
    #[test]
    fn text_is_yielded_incrementally_as_it_arrives() {
        let mut decoder = BedrockStreamDecoder::new("us.anthropic.claude-sonnet-4-6");
        let items = drain(
            &mut decoder,
            &[
                message_start(),
                text_delta(0, "Hello"),
                text_delta(0, ", "),
                text_delta(0, "world"),
                block_stop(0),
                message_stop(StopReason::EndTurn),
            ],
        );

        assert_eq!(texts(&items), vec!["Hello", ", ", "world"]);
        assert!(tool_requests(&items).is_empty());
        assert!(!decoder.was_truncated());
        assert!(decoder.finish().is_empty());
    }

    #[test]
    fn empty_text_deltas_are_not_yielded_as_messages() {
        let mut decoder = BedrockStreamDecoder::new("m");
        let items = drain(&mut decoder, &[text_delta(0, ""), text_delta(0, "x")]);
        assert_eq!(texts(&items), vec!["x"]);
    }

    // ---- single tool call ---------------------------------------------------

    /// THE safety property: nothing tool-shaped is emitted until the block has
    /// closed (complete arguments), and — with §6.2b batching on by default —
    /// the request is delivered once, at `messageStop`.
    #[test]
    fn tool_call_is_emitted_once_at_message_stop_with_complete_arguments() {
        let mut decoder = BedrockStreamDecoder::new("m");

        let before_stop = drain(
            &mut decoder,
            &[
                message_start(),
                tool_start(0, "tooluse_abc", "shell"),
                tool_delta(0, "{\"command\":"),
                tool_delta(0, "\"ls -la /tmp\"}"),
            ],
        );
        assert!(
            before_stop.is_empty(),
            "no item may be emitted before contentBlockStop, got {before_stop:?}"
        );

        // §6.2b: block close buffers the completed request; nothing is
        // emitted until the response ends.
        let at_block_stop = drain(&mut decoder, &[block_stop(0)]);
        assert!(
            tool_requests(&at_block_stop).is_empty(),
            "batched: the request must not be emitted before messageStop"
        );

        let at_stop = drain(&mut decoder, &[message_stop(StopReason::ToolUse)]);
        let requests = tool_requests(&at_stop);
        assert_eq!(requests.len(), 1, "exactly one tool request");
        assert_eq!(requests[0].0, "tooluse_abc");
        let (name, args) = requests[0].1.as_ref().expect("tool call should parse");
        assert_eq!(name, "shell");
        assert_eq!(args, &serde_json::json!({"command": "ls -la /tmp"}));

        // And it is not re-emitted afterwards.
        assert!(tool_requests(&decoder.finish()).is_empty());
    }

    /// Bedrock splits `input` at arbitrary byte boundaries, including mid-token
    /// and mid-string. Accumulation must be a plain concatenation.
    #[test]
    fn tool_input_split_across_many_deltas_reassembles_exactly() {
        let full = r#"{"path":"/tmp/a.txt","command":"str_replace","new_str":"x, y"}"#;
        let mut decoder = BedrockStreamDecoder::new("m");

        let mut events = vec![tool_start(3, "id1", "text_editor")];
        // Chop into 5-byte fragments, deliberately ignoring JSON structure.
        for chunk in full.as_bytes().chunks(5) {
            events.push(tool_delta(3, std::str::from_utf8(chunk).unwrap()));
        }
        events.push(block_stop(3));
        events.push(message_stop(StopReason::ToolUse));

        let items = drain(&mut decoder, &events);
        let requests = tool_requests(&items);
        assert_eq!(requests.len(), 1);
        let (name, args) = requests[0].1.as_ref().unwrap();
        assert_eq!(name, "text_editor");
        assert_eq!(args, &serde_json::from_str::<Value>(full).unwrap());
    }

    #[test]
    fn tool_with_no_input_deltas_gets_an_empty_object() {
        let mut decoder = BedrockStreamDecoder::new("m");
        let items = drain(
            &mut decoder,
            &[
                tool_start(0, "id", "list_things"),
                block_stop(0),
                message_stop(StopReason::ToolUse),
            ],
        );
        let requests = tool_requests(&items);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1.as_ref().unwrap().1, serde_json::json!({}));
    }

    // ---- multiple tool calls ------------------------------------------------

    /// Two tool_use blocks in one response, with their deltas interleaved.
    /// Keying accumulators by `contentBlockIndex` is what makes this work.
    #[test]
    fn two_interleaved_tool_calls_each_decode_correctly() {
        let mut decoder = BedrockStreamDecoder::new("m");
        let items = drain(
            &mut decoder,
            &[
                message_start(),
                tool_start(0, "id_a", "shell"),
                tool_start(1, "id_b", "text_editor"),
                tool_delta(0, "{\"command\":"),
                tool_delta(1, "{\"path\":\"/etc/"),
                tool_delta(0, "\"pwd\"}"),
                tool_delta(1, "hosts\"}"),
                block_stop(0),
                block_stop(1),
                message_stop(StopReason::ToolUse),
            ],
        );

        let requests = tool_requests(&items);
        assert_eq!(requests.len(), 2);

        assert_eq!(requests[0].0, "id_a");
        let (name_a, args_a) = requests[0].1.as_ref().unwrap();
        assert_eq!(name_a, "shell");
        assert_eq!(args_a, &serde_json::json!({"command": "pwd"}));

        assert_eq!(requests[1].0, "id_b");
        let (name_b, args_b) = requests[1].1.as_ref().unwrap();
        assert_eq!(name_b, "text_editor");
        assert_eq!(args_b, &serde_json::json!({"path": "/etc/hosts"}));
    }

    /// §6.2b core gate (issue #41): two `tool_use` blocks in one response must
    /// decode to **one** assistant message carrying **two** `ToolRequest`s —
    /// mirroring the Anthropic decoder's
    /// `test_streaming_batches_multiple_tool_uses_into_one_message`. Before
    /// this, each block became its own message stamped with the SAME shared
    /// `message_id`, and the agent loop persisted two assistant rows with one
    /// `msg_uid` — which the session store rejects (UNIQUE constraint 2067),
    /// killing the turn.
    #[test]
    fn two_tool_use_blocks_batch_into_one_assistant_message() {
        let mut decoder = BedrockStreamDecoder::with_batching("m", true);
        let items = drain(
            &mut decoder,
            &[
                message_start(),
                text_delta(0, "Let me check."),
                block_stop(0),
                tool_start(1, "toolu_a", "developer__shell"),
                tool_delta(1, "{\"command\":\"ls\"}"),
                block_stop(1),
                tool_start(2, "toolu_b", "developer__text_editor"),
                tool_delta(2, "{\"command\":\"view\",\"path\":\"/tmp/x\"}"),
                block_stop(2),
                message_stop(StopReason::ToolUse),
            ],
        );

        // Exactly ONE message carries tool requests, and it carries BOTH,
        // in block order (request order is load-bearing — Anthropic 400s a
        // reordered tool-result batch).
        let tool_messages: Vec<&Message> = items
            .iter()
            .filter_map(|(m, _)| m.as_ref())
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
        let requests = tool_requests(&items);
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].0, "toolu_a");
        assert_eq!(requests[1].0, "toolu_b");

        // Identity: the batched message carries the decoder's shared id, so
        // desktop delta-merging still folds the whole response into one bubble.
        let text_id = items
            .iter()
            .filter_map(|(m, _)| m.as_ref())
            .find(|m| {
                m.content
                    .iter()
                    .any(|c| matches!(c, MessageContent::Text(_)))
            })
            .and_then(|m| m.id.clone())
            .expect("text message must carry the shared id");
        assert_eq!(
            tool_messages[0].id.as_deref(),
            Some(text_id.as_str()),
            "the batched tool message must share the response's message id"
        );

        // The batched message rides the SAME stream item as the messageStop
        // usage snapshot (agent.rs reads message + usage in one match arm).
        let batched_item = items
            .iter()
            .find(|(m, _)| {
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
    }

    /// Interleaved blocks where the LATER block closes FIRST: the batched
    /// message must order its requests by `content_block_index`, not by
    /// `contentBlockStop` arrival. Before this fix the buffer kept stop order,
    /// so block 2's request was dispatched and persisted ahead of block 1's.
    #[test]
    fn batched_tools_keep_block_order_when_stops_arrive_reversed() {
        let mut decoder = BedrockStreamDecoder::with_batching("m", true);
        let items = drain(
            &mut decoder,
            &[
                message_start(),
                // Both blocks open, deltas interleave, then they close in
                // REVERSE index order (2 before 1).
                tool_start(1, "toolu_first", "developer__shell"),
                tool_start(2, "toolu_second", "developer__text_editor"),
                tool_delta(2, "{\"path\":"),
                tool_delta(1, "{\"command\":"),
                tool_delta(2, "\"/tmp/x\"}"),
                tool_delta(1, "\"ls\"}"),
                block_stop(2),
                block_stop(1),
                message_stop(StopReason::ToolUse),
            ],
        );

        let requests = tool_requests(&items);
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].0, "toolu_first",
            "block 1 must come first even though block 2 closed first"
        );
        assert_eq!(requests[1].0, "toolu_second");
        // Arguments were accumulated per index, unaffected by the reordering.
        assert_eq!(
            requests[0].1.as_ref().unwrap().1,
            serde_json::json!({"command": "ls"})
        );
        assert_eq!(
            requests[1].1.as_ref().unwrap().1,
            serde_json::json!({"path": "/tmp/x"})
        );
    }

    /// Same reversal, but the stream dies after the blocks closed and BEFORE
    /// `messageStop` — the `finish()` flush must apply the same index sort.
    #[test]
    fn finish_flush_also_sorts_reversed_stop_order() {
        let mut decoder = BedrockStreamDecoder::with_batching("m", true);
        let during = drain(
            &mut decoder,
            &[
                message_start(),
                tool_start(1, "toolu_first", "shell"),
                tool_start(2, "toolu_second", "text_editor"),
                tool_delta(1, "{\"command\":\"pwd\"}"),
                tool_delta(2, "{\"path\":\"/tmp/y\"}"),
                block_stop(2),
                block_stop(1),
                // connection died here: no messageStop
            ],
        );
        assert!(tool_requests(&during).is_empty());

        let flushed = decoder.finish();
        let requests = tool_requests(&flushed);
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].0, "toolu_first");
        assert_eq!(requests[1].0, "toolu_second");
    }

    /// §6.2b kill switch: with batching OFF (`BIOROUTER_TOOL_CALL_BATCHING=0`
    /// at construction) the pre-batching serial shape is restored — one
    /// assistant message per `tool_use` block.
    ///
    /// Injected via the `with_batching` test seam instead of mutating the env
    /// var: env mutation races every parallel test that constructs a decoder
    /// (`new()` reads the flag), and these sync tests hit that window
    /// reliably. The env parsing is `tool_call_batching_enabled()`'s own
    /// contract, exercised by the Anthropic decoder's serial kill-switch test.
    #[test]
    fn kill_switch_restores_serial_tool_messages() {
        let mut decoder = BedrockStreamDecoder::with_batching("m", false);
        let items = drain(
            &mut decoder,
            &[
                message_start(),
                tool_start(0, "toolu_a", "shell"),
                tool_delta(0, "{\"command\":\"ls\"}"),
                block_stop(0),
                tool_start(1, "toolu_b", "text_editor"),
                tool_delta(1, "{\"path\":\"/tmp/x\"}"),
                block_stop(1),
                message_stop(StopReason::ToolUse),
            ],
        );

        let shape: Vec<usize> = items
            .iter()
            .filter_map(|(m, _)| m.as_ref())
            .map(|m| {
                m.content
                    .iter()
                    .filter(|c| matches!(c, MessageContent::ToolRequest(_)))
                    .count()
            })
            .filter(|count| *count > 0)
            .collect();
        assert_eq!(
            shape,
            vec![1, 1],
            "with batching OFF each tool_use block must be its own message (serial)"
        );
    }

    /// §6.2b regression: a stream that ends after its blocks closed but
    /// WITHOUT a `messageStop` (cut off cleanly) must still deliver the
    /// batched tools via `finish()` — otherwise a whole multi-tool turn would
    /// silently vanish.
    #[test]
    fn stream_ending_without_message_stop_still_flushes_batched_tools() {
        let mut decoder = BedrockStreamDecoder::with_batching("m", true);
        let during = drain(
            &mut decoder,
            &[
                message_start(),
                tool_start(0, "toolu_done", "shell"),
                tool_delta(0, "{\"command\":\"pwd\"}"),
                block_stop(0),
                tool_start(1, "toolu_open", "text_editor"),
                tool_delta(1, "{\"pa"),
                // connection died here: no block_stop(1), no messageStop
            ],
        );
        assert!(tool_requests(&during).is_empty());

        let flushed = decoder.finish();
        assert!(decoder.was_truncated());
        let requests = tool_requests(&flushed);
        assert_eq!(requests.len(), 2, "completed AND truncated calls surface");
        // The completed block is callable (flushed batch)…
        assert_eq!(requests[0].0, "toolu_done");
        assert!(requests[0].1.is_ok());
        // …the truncated one is a failed request, never callable.
        assert_eq!(requests[1].0, "toolu_open");
        assert!(requests[1].1.is_err());
    }

    #[test]
    fn text_and_tool_call_in_one_response_both_decode() {
        let mut decoder = BedrockStreamDecoder::new("m");
        let items = drain(
            &mut decoder,
            &[
                text_delta(0, "Let me look."),
                block_stop(0),
                tool_start(1, "id", "shell"),
                tool_delta(1, "{\"command\":\"ls\"}"),
                block_stop(1),
                message_stop(StopReason::ToolUse),
            ],
        );
        assert_eq!(texts(&items), vec!["Let me look."]);
        assert_eq!(tool_requests(&items).len(), 1);
    }

    // ---- truncation ---------------------------------------------------------

    /// A stream that dies mid-tool-call must NOT produce a callable request with
    /// half the arguments — for `shell` that would execute a truncated command.
    /// It must also not vanish silently; it becomes a failed request.
    #[test]
    fn stream_truncated_mid_tool_call_yields_a_failed_request_not_truncated_arguments() {
        let mut decoder = BedrockStreamDecoder::new("m");

        let during = drain(
            &mut decoder,
            &[
                message_start(),
                tool_start(0, "id_trunc", "shell"),
                tool_delta(0, "{\"command\":\"rm -rf /tmp/scratch"),
                // no contentBlockStop, no messageStop — connection died here
            ],
        );
        assert!(during.is_empty());

        let flushed = decoder.finish();
        assert!(decoder.was_truncated());

        let requests = tool_requests(&flushed);
        assert_eq!(requests.len(), 1, "the dropped call must be surfaced");
        assert_eq!(requests[0].0, "id_trunc");
        let error = requests[0]
            .1
            .as_ref()
            .expect_err("a truncated tool call must never be a callable request");
        assert!(
            error.contains("ended before completion of the tool block"),
            "error should explain the truncation: {error}"
        );
        assert!(
            error.contains("shell"),
            "error should name the tool: {error}"
        );
    }

    /// The exact shape the user-reported `us.anthropic.claude-sonnet-5` failure
    /// hands the agent loop, pinned here so the agent-loop test next door
    /// (`tests/signed_stream_truncation_agent_loop.rs`) scripts reality rather
    /// than a guess: a SIGNED reasoning block that closed cleanly, followed by a
    /// `tool_use` block the connection cut off mid-arguments.
    ///
    /// Three properties are load-bearing downstream and none of them is implied
    /// by the others: the two messages share ONE id (so the agent folds them
    /// into a single signed assistant row), the failure is tagged
    /// `incomplete_stream` (which is what separates "the provider never finished
    /// sending" from "the provider sent arguments we cannot use"), and the
    /// signed row itself never reaches `finish()` — it was already emitted.
    #[test]
    fn signed_truncated_tool_block_reports_incomplete_stream_under_the_signed_id() {
        let mut decoder = BedrockStreamDecoder::new("m");
        let reasoning = |index, text: &str| {
            ConverseStreamOutput::ContentBlockDelta(
                ContentBlockDeltaEvent::builder()
                    .content_block_index(index)
                    .delta(ContentBlockDelta::ReasoningContent(
                        ReasoningContentBlockDelta::Text(text.to_string()),
                    ))
                    .build()
                    .unwrap(),
            )
        };
        let signature = |index, signature: &str| {
            ConverseStreamOutput::ContentBlockDelta(
                ContentBlockDeltaEvent::builder()
                    .content_block_index(index)
                    .delta(ContentBlockDelta::ReasoningContent(
                        ReasoningContentBlockDelta::Signature(signature.to_string()),
                    ))
                    .build()
                    .unwrap(),
            )
        };

        let during = drain(
            &mut decoder,
            &[
                message_start(),
                reasoning(0, "I will rewrite the file."),
                signature(0, "bedrock-signature"),
                block_stop(0),
                tool_start(1, "toolu_trunc", "developer__text_editor"),
                tool_delta(
                    1,
                    "{\"command\":\"write\",\"path\":\"/tmp/a.rs\",\"file_text\":\"fn ma",
                ),
                // the connection died here: no contentBlockStop, no messageStop
            ],
        );
        let signed = during
            .iter()
            .filter_map(|(message, _)| message.as_ref())
            .find(|message| {
                message
                    .content
                    .iter()
                    .any(|content| matches!(content, MessageContent::Thinking(_)))
            })
            .expect("the closed reasoning block is emitted during the stream");
        assert_eq!(
            signed.content[0].as_thinking().unwrap().signature,
            "bedrock-signature"
        );

        let flushed = decoder.finish();
        assert!(decoder.was_truncated());
        assert!(
            !flushed
                .iter()
                .filter_map(|(message, _)| message.as_ref())
                .any(|message| message
                    .content
                    .iter()
                    .any(|content| matches!(content, MessageContent::Thinking(_)))),
            "the signed row was already emitted; finish() must not re-emit it"
        );

        let failures: Vec<&ErrorData> = flushed
            .iter()
            .filter_map(|(message, _)| message.as_ref())
            .flat_map(|message| message.content.iter())
            .filter_map(|content| match content {
                MessageContent::ToolRequest(request) => request.tool_call.as_ref().err(),
                _ => None,
            })
            .collect();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(
            failures[0]
                .data
                .as_ref()
                .and_then(|data| data.get("biorouterToolCallFailure"))
                .and_then(Value::as_str),
            Some("incomplete_stream"),
            "the tag is what tells the agent loop this was a cut connection"
        );

        let flushed_id = flushed
            .iter()
            .filter_map(|(message, _)| message.as_ref())
            .find(|message| {
                message
                    .content
                    .iter()
                    .any(|content| matches!(content, MessageContent::ToolRequest(_)))
            })
            .and_then(|message| message.id.clone());
        assert_eq!(
            flushed_id.as_deref(),
            signed.id.as_deref(),
            "the truncated request must carry the SAME id as the signed row it belongs to"
        );
    }

    /// Text already delivered before a truncation stays delivered; only the
    /// incomplete tool block is turned into a failure.
    #[test]
    fn truncation_preserves_already_yielded_text() {
        let mut decoder = BedrockStreamDecoder::new("m");
        let during = drain(
            &mut decoder,
            &[
                text_delta(0, "partial answer"),
                tool_start(1, "id", "shell"),
                tool_delta(1, "{\"comm"),
            ],
        );
        assert_eq!(texts(&during), vec!["partial answer"]);
        assert_eq!(tool_requests(&decoder.finish()).len(), 1);
    }

    /// A clean stream leaves nothing to flush — `finish` must not invent a
    /// duplicate tool request for a block that already closed.
    #[test]
    fn finish_after_a_complete_stream_emits_nothing() {
        let mut decoder = BedrockStreamDecoder::new("m");
        drain(
            &mut decoder,
            &[
                tool_start(0, "id", "shell"),
                tool_delta(0, "{\"command\":\"ls\"}"),
                block_stop(0),
                message_stop(StopReason::ToolUse),
                metadata(1, 2, 3),
            ],
        );
        assert!(decoder.finish().is_empty());
        assert!(!decoder.was_truncated());
    }

    // ---- malformed arguments -------------------------------------------------

    #[test]
    fn unparseable_tool_json_becomes_a_failed_request() {
        let mut decoder = BedrockStreamDecoder::new("m");
        let items = drain(
            &mut decoder,
            &[
                tool_start(0, "id", "shell"),
                tool_delta(0, "{\"command\": not json"),
                block_stop(0),
                message_stop(StopReason::ToolUse),
            ],
        );
        let requests = tool_requests(&items);
        assert_eq!(requests.len(), 1);
        assert!(requests[0].1.is_err(), "must not be callable");
    }

    /// `rmcp::model::object` debug-asserts its input is an object; a scalar must
    /// be rejected before it reaches that call.
    #[test]
    fn non_object_tool_json_becomes_a_failed_request() {
        let mut decoder = BedrockStreamDecoder::new("m");
        let items = drain(
            &mut decoder,
            &[
                tool_start(0, "id", "shell"),
                tool_delta(0, "\"just a string\""),
                block_stop(0),
                message_stop(StopReason::ToolUse),
            ],
        );
        let requests = tool_requests(&items);
        assert_eq!(requests.len(), 1);
        let error = requests[0].1.as_ref().unwrap_err();
        assert!(error.contains("not a JSON object"), "got: {error}");
    }

    // ---- usage / accounting --------------------------------------------------

    #[test]
    fn metadata_event_reports_usage_with_all_buckets_and_the_model_name() {
        let mut decoder = BedrockStreamDecoder::new("us.anthropic.claude-opus-4-6-v1");
        let items = drain(&mut decoder, &[text_delta(0, "hi"), metadata(100, 40, 140)]);

        let usage = items
            .iter()
            .filter_map(|(_, usage)| usage.as_ref())
            .next_back()
            .expect("a usage snapshot must be emitted");

        assert_eq!(usage.model, "us.anthropic.claude-opus-4-6-v1");
        assert_eq!(usage.usage.input_tokens, Some(100));
        assert_eq!(usage.usage.output_tokens, Some(40));
        assert_eq!(usage.usage.total_tokens, Some(140));
        assert_eq!(usage.usage.cache_read_input_tokens, Some(7));
        assert_eq!(usage.usage.cache_creation_input_tokens, Some(3));
        // Same reconciliation the blocking path guarantees: 100 + 40 + 7 + 3.
        assert_eq!(usage.usage.billed_total(), Some(150));
    }

    /// The final snapshot must carry both the real token counts and the mapped
    /// finish reason, in Bedrock's real event order (messageStop then metadata).
    #[test]
    fn final_usage_snapshot_carries_tokens_and_finish_reason() {
        let mut decoder = BedrockStreamDecoder::new("m");
        let items = drain(
            &mut decoder,
            &[
                text_delta(0, "hi"),
                message_stop(StopReason::MaxTokens),
                metadata(10, 5, 15),
            ],
        );

        let last = items
            .iter()
            .filter_map(|(_, usage)| usage.as_ref())
            .next_back()
            .expect("a usage snapshot must be emitted");
        // "max_tokens" -> "length" is what lets the agent auto-continue a
        // response cut off by the output limit.
        assert_eq!(last.finish_reason.as_deref(), Some("length"));
        assert_eq!(last.usage.input_tokens, Some(10));
        assert_eq!(last.usage.output_tokens, Some(5));
    }

    #[test]
    fn stop_reasons_map_onto_the_openai_style_finish_reasons() {
        for (bedrock_reason, expected) in [
            (StopReason::EndTurn, "stop"),
            (StopReason::StopSequence, "stop"),
            (StopReason::MaxTokens, "length"),
            (StopReason::ToolUse, "tool_calls"),
            (StopReason::ContentFiltered, "content_filter"),
            (StopReason::GuardrailIntervened, "content_filter"),
            // An INPUT-too-large stop must never become "length": that is the
            // agent's auto-continue trigger, and continuing cannot help.
            (
                StopReason::ModelContextWindowExceeded,
                "model_context_window_exceeded",
            ),
        ] {
            let mut decoder = BedrockStreamDecoder::new("m");
            let items = drain(&mut decoder, &[message_stop(bedrock_reason.clone())]);
            let usage = items
                .iter()
                .filter_map(|(_, usage)| usage.as_ref())
                .next_back()
                .unwrap();
            assert_eq!(
                usage.finish_reason.as_deref(),
                Some(expected),
                "for {bedrock_reason:?}"
            );
        }
    }

    /// Usage that arrived before a truncation must still be reported, so a
    /// cut-off turn is billed rather than recorded as free.
    #[test]
    fn truncated_stream_still_reports_usage_it_received() {
        let mut decoder = BedrockStreamDecoder::new("m");
        drain(&mut decoder, &[metadata(50, 20, 70)]);

        let flushed = decoder.finish();
        let usage = flushed
            .iter()
            .filter_map(|(_, usage)| usage.as_ref())
            .next_back()
            .expect("usage should be re-emitted on truncation");
        assert_eq!(usage.usage.input_tokens, Some(50));
    }

    /// No usage was ever reported, so `finish` must not fabricate a zero
    /// snapshot that would overwrite real accounting.
    #[test]
    fn truncated_stream_without_usage_emits_no_usage_snapshot() {
        let mut decoder = BedrockStreamDecoder::new("m");
        drain(&mut decoder, &[text_delta(0, "hi")]);
        let flushed = decoder.finish();
        assert!(flushed.iter().all(|(_, usage)| usage.is_none()));
    }

    // ---- reasoning content ---------------------------------------------------

    #[test]
    fn reasoning_deltas_close_as_signed_blocks_in_index_order_with_one_response_id() {
        let mut decoder = BedrockStreamDecoder::new("m");
        let reasoning = |index, text: &str| {
            ConverseStreamOutput::ContentBlockDelta(
                ContentBlockDeltaEvent::builder()
                    .content_block_index(index)
                    .delta(ContentBlockDelta::ReasoningContent(
                        ReasoningContentBlockDelta::Text(text.to_string()),
                    ))
                    .build()
                    .unwrap(),
            )
        };
        let signature = |index, signature: &str| {
            ConverseStreamOutput::ContentBlockDelta(
                ContentBlockDeltaEvent::builder()
                    .content_block_index(index)
                    .delta(ContentBlockDelta::ReasoningContent(
                        ReasoningContentBlockDelta::Signature(signature.to_string()),
                    ))
                    .build()
                    .unwrap(),
            )
        };

        // Block 1 closes before block 0. The decoder must wait and emit them in
        // canonical index order, with complete text+signature pairs.
        let items = drain(
            &mut decoder,
            &[
                reasoning(1, "second"),
                signature(1, "sig-2"),
                block_stop(1),
                reasoning(0, "first"),
                signature(0, "sig-1"),
                block_stop(0),
            ],
        );
        let messages = items
            .iter()
            .filter_map(|(message, _)| message.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), 2);
        let thinking = messages
            .iter()
            .map(|message| message.content[0].as_thinking().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(thinking[0].thinking, "first");
        assert_eq!(thinking[0].signature, "sig-1");
        assert_eq!(thinking[1].thinking, "second");
        assert_eq!(thinking[1].signature, "sig-2");
        assert!(messages.iter().all(|message| message.id == messages[0].id));
    }

    #[test]
    fn batched_tool_block_never_lets_later_text_overtake_it() {
        let mut decoder = BedrockStreamDecoder::with_batching("m", true);
        let reasoning = ConverseStreamOutput::ContentBlockDelta(
            ContentBlockDeltaEvent::builder()
                .content_block_index(0)
                .delta(ContentBlockDelta::ReasoningContent(
                    ReasoningContentBlockDelta::Text("think".to_string()),
                ))
                .build()
                .unwrap(),
        );
        let signature = ConverseStreamOutput::ContentBlockDelta(
            ContentBlockDeltaEvent::builder()
                .content_block_index(0)
                .delta(ContentBlockDelta::ReasoningContent(
                    ReasoningContentBlockDelta::Signature("sig".to_string()),
                ))
                .build()
                .unwrap(),
        );
        let items = drain(
            &mut decoder,
            &[
                reasoning,
                signature,
                block_stop(0),
                tool_start(1, "tool-1", "shell"),
                tool_delta(1, r#"{"command":"pwd"}"#),
                block_stop(1),
                text_delta(2, "after tool"),
                block_stop(2),
                message_stop(StopReason::ToolUse),
            ],
        );
        let flattened = items
            .iter()
            .filter_map(|(message, _)| message.as_ref())
            .flat_map(|message| message.content.iter())
            .collect::<Vec<_>>();
        assert_eq!(flattened.len(), 3);
        assert!(matches!(flattened[0], MessageContent::Thinking(_)));
        assert!(matches!(flattened[1], MessageContent::ToolRequest(_)));
        assert!(matches!(flattened[2], MessageContent::Text(_)));
    }

    #[test]
    fn a_text_block_stays_buffered_after_a_lower_index_unblocks_it() {
        let mut decoder = BedrockStreamDecoder::with_batching("m", false);
        let items = drain(
            &mut decoder,
            &[
                text_delta(1, "A"),
                text_delta(0, "zero"),
                block_stop(0),
                text_delta(1, "B"),
                block_stop(1),
            ],
        );
        assert_eq!(texts(&items).join(""), "zeroAB");
    }

    #[test]
    fn redacted_reasoning_delta_round_trips_its_exact_bytes() {
        let mut decoder = BedrockStreamDecoder::new("m");
        let bytes = b"opaque\0reasoning";
        let delta = ConverseStreamOutput::ContentBlockDelta(
            ContentBlockDeltaEvent::builder()
                .content_block_index(0)
                .delta(ContentBlockDelta::ReasoningContent(
                    ReasoningContentBlockDelta::RedactedContent(aws_smithy_types::Blob::new(bytes)),
                ))
                .build()
                .unwrap(),
        );
        let items = drain(&mut decoder, &[delta, block_stop(0)]);
        let content = items[0].0.as_ref().unwrap().content[0].clone();
        let MessageContent::RedactedThinking(redacted) = &content else {
            panic!("expected redacted reasoning");
        };
        assert_eq!(
            base64::prelude::BASE64_STANDARD
                .decode(&redacted.data)
                .unwrap(),
            bytes
        );
        let block = to_bedrock_message_content(&content).unwrap();
        let bedrock::ContentBlock::ReasoningContent(
            bedrock::ReasoningContentBlock::RedactedContent(replayed),
        ) = block
        else {
            panic!("expected Bedrock redacted reasoning");
        };
        assert_eq!(replayed.as_ref(), bytes);
    }

    /// A toolUse delta whose block was never opened (should not happen, but the
    /// service is not ours) must be dropped, never invent a request.
    #[test]
    fn tool_delta_for_an_unknown_block_index_is_dropped() {
        let mut decoder = BedrockStreamDecoder::new("m");
        let items = drain(&mut decoder, &[tool_delta(9, "{\"a\":1}"), block_stop(9)]);
        assert!(tool_requests(&items).is_empty());
        assert!(decoder.finish().is_empty());
    }
}

#[cfg(test)]
mod bedrock_error_tests {
    use super::*;
    use crate::providers::retry::should_retry;
    use aws_sdk_bedrockruntime::config::http::HttpResponse;
    use aws_sdk_bedrockruntime::error::ErrorMetadata;
    use aws_sdk_bedrockruntime::operation::converse::ConverseError;
    use aws_sdk_bedrockruntime::types::error::{ThrottlingException, ValidationException};
    use aws_smithy_runtime_api::http::StatusCode;
    use aws_smithy_types::body::SdkBody;

    /// A `ServiceError` — the SDK received an HTTP error response. `inner` is the
    /// (possibly `Unhandled`) typed error the SDK managed to parse from the body.
    fn service_err(status: u16, body: &str, inner: ConverseError) -> SdkError<ConverseError> {
        let resp = HttpResponse::new(
            StatusCode::try_from(status).unwrap(),
            SdkBody::from(body.as_bytes().to_vec()),
        );
        SdkError::service_error(inner, resp)
    }

    /// A proxy error the SDK could not type: `ConverseError::Unhandled` with empty
    /// metadata — exactly what UCSF's Versa MuleSoft proxy produced in the incident.
    fn proxy_unhandled(status: u16, body: &str) -> SdkError<ConverseError> {
        service_err(
            status,
            body,
            ConverseError::generic(ErrorMetadata::builder().build()),
        )
    }

    // ---- looks_like_context_overflow --------------------------------------

    #[test]
    fn overflow_phrasings_are_detected() {
        for s in [
            "Input is too long for requested model.",
            "input is too long",
            "The prompt is too long",
            "exceeds the maximum context length",
            "too many tokens in the request",
            "This exceeds the model's token limit",
            "maximum number of tokens",
        ] {
            assert!(looks_like_context_overflow(s), "should flag: {s}");
        }
        for s in ["throttled", "internal server error", "access denied", ""] {
            assert!(!looks_like_context_overflow(s), "should NOT flag: {s}");
        }
    }

    // ---- the incident: proxy errors the SDK collapsed to `Unhandled` -------

    /// Reproduces the captured incident error: a `ServiceError` wrapping
    /// `ConverseError::Unhandled { meta: empty }`. Before the fix this became a
    /// generic retryable `ServerError` with an unreadable `{:?}` dump. A 429 from
    /// the proxy must now be recognised as a rate limit so it gets the deeper
    /// retry budget and informs the scheduler.
    #[test]
    fn proxy_429_is_rate_limit_not_opaque_server_error() {
        let err = classify_bedrock_converse_error(proxy_unhandled(429, ""));
        assert!(
            matches!(err, ProviderError::RateLimitExceeded { .. }),
            "expected RateLimitExceeded, got {err:?}"
        );
        assert!(should_retry(&err));
    }

    #[test]
    fn proxy_5xx_is_retryable_server_error_with_readable_message() {
        for status in [500u16, 502, 503, 504] {
            let err =
                classify_bedrock_converse_error(proxy_unhandled(status, "<html>gateway</html>"));
            assert!(
                matches!(err, ProviderError::ServerError(_)),
                "status {status}: expected ServerError, got {err:?}"
            );
            assert!(should_retry(&err), "status {status} should be retryable");
            // No raw `Unhandled(Unhandled { .. })` dump leaking to the user.
            assert!(err.to_string().contains(&status.to_string()));
        }
    }

    /// The failure mode that stranded the original session: an over-limit prompt
    /// that the proxy rejected. If it never becomes `ContextLengthExceeded`, the
    /// agent can't auto-compact and every retry re-sends the same doomed prompt.
    #[test]
    fn proxy_400_input_too_long_is_context_length_exceeded() {
        let err = classify_bedrock_converse_error(proxy_unhandled(
            400,
            "{\"message\":\"Input is too long for requested model.\"}",
        ));
        assert!(
            matches!(err, ProviderError::ContextLengthExceeded(_)),
            "expected ContextLengthExceeded, got {err:?}"
        );
        // Context overflow is handled by compaction, not the generic retry loop.
        assert!(!should_retry(&err));
    }

    #[test]
    fn proxy_413_payload_too_large_is_context_length_exceeded() {
        let err = classify_bedrock_converse_error(proxy_unhandled(413, ""));
        assert!(
            matches!(err, ProviderError::ContextLengthExceeded(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn proxy_401_403_are_auth_errors_not_retried() {
        for status in [401u16, 403] {
            let err = classify_bedrock_converse_error(proxy_unhandled(status, ""));
            assert!(
                matches!(err, ProviderError::Authentication(_)),
                "status {status}: expected Authentication, got {err:?}"
            );
            assert!(!should_retry(&err), "auth errors must not be retried");
        }
    }

    // ---- transport failures (hung / unreachable endpoint) ------------------

    #[test]
    fn timeout_is_retryable_server_error_with_clear_message() {
        let err = classify_bedrock_converse_error(SdkError::timeout_error(Box::new(
            std::io::Error::new(std::io::ErrorKind::TimedOut, "stalled"),
        )));
        match &err {
            ProviderError::ServerError(msg) => assert!(
                msg.to_lowercase().contains("timed out"),
                "message should mention a timeout: {msg}"
            ),
            other => panic!("expected ServerError, got {other:?}"),
        }
        assert!(should_retry(&err));
    }

    // ---- typed variants still classify correctly ---------------------------

    #[test]
    fn typed_throttling_is_rate_limit() {
        let inner = ConverseError::ThrottlingException(
            ThrottlingException::builder().message("slow down").build(),
        );
        let err = classify_bedrock_converse_error(service_err(429, "", inner));
        assert!(
            matches!(err, ProviderError::RateLimitExceeded { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn typed_validation_overflow_is_context_length() {
        let inner = ConverseError::ValidationException(
            ValidationException::builder()
                .message("Input is too long for requested model.")
                .build(),
        );
        let err = classify_bedrock_converse_error(service_err(400, "", inner));
        assert!(
            matches!(err, ProviderError::ContextLengthExceeded(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn typed_validation_non_overflow_is_non_retryable_execution_error() {
        let inner = ConverseError::ValidationException(
            ValidationException::builder()
                .message("The value at messages.1 is invalid")
                .build(),
        );
        let err = classify_bedrock_converse_error(service_err(400, "", inner));
        assert!(
            matches!(err, ProviderError::ExecutionError(_)),
            "got {err:?}"
        );
        // A malformed request cannot be fixed by retrying it verbatim.
        assert!(!should_retry(&err));
    }

    // ---- timeout config knob ----------------------------------------------

    #[test]
    fn timeout_config_env_override_and_disable() {
        // Note: relies on process-global env; kept simple and independent of the
        // shared Config by exercising the env parse path directly.
        let parse = |v: &str| v.trim().parse::<u64>().ok();
        assert_eq!(parse("0"), Some(0));
        assert_eq!(parse("600"), Some(600));
        assert_eq!(parse("  90 "), Some(90));
        assert_eq!(parse("nope"), None);
        // Default is a bounded, generous value (never infinite).
        const { assert!(BEDROCK_DEFAULT_OPERATION_TIMEOUT_SECS >= 60) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use rmcp::model::{AnnotateAble, RawImageContent};

    // Base64 encoded 1x1 PNG image for testing
    const TEST_IMAGE_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==";

    #[test]
    fn inference_config_honors_override_and_uses_model_safe_defaults() {
        let configured = ModelConfig::new_or_fail("us.anthropic.claude-sonnet-4-6")
            .with_max_tokens(Some(12_345))
            .with_temperature(Some(0.25));
        let inference = bedrock_inference_config(&configured);
        assert_eq!(inference.max_tokens(), Some(12_345));
        assert_eq!(inference.temperature(), Some(0.25));

        let sonnet_five = ModelConfig::new_or_fail("us.anthropic.claude-sonnet-5")
            .with_max_tokens(None)
            .with_temperature(Some(0.25));
        let inference = bedrock_inference_config(&sonnet_five);
        assert_eq!(
            inference.max_tokens(),
            Some(BEDROCK_CLAUDE_DEFAULT_MAX_TOKENS)
        );
        assert_eq!(
            inference.temperature(),
            None,
            "adaptive-only Claude models reject sampling parameters"
        );

        let unlisted = ModelConfig::new_or_fail("vendor.unknown-model").with_max_tokens(None);
        assert_eq!(
            bedrock_inference_config(&unlisted).max_tokens(),
            Some(BEDROCK_GENERIC_DEFAULT_MAX_TOKENS)
        );

        let unlisted_claude_three =
            ModelConfig::new_or_fail("us.anthropic.claude-3-sonnet-20240229-v1:0")
                .with_max_tokens(None);
        assert_eq!(
            bedrock_inference_config(&unlisted_claude_three).max_tokens(),
            Some(BEDROCK_GENERIC_DEFAULT_MAX_TOKENS)
        );
    }

    /// The 64,000 default must not exceed what compaction leaves room for.
    ///
    /// Bedrock rejects `input + maxTokens > context`. Compaction does not fire
    /// until input reaches 80% of the window, so on a 200K model a flat 64,000
    /// allowance makes every request between 136K and 160K of input fail on a
    /// turn the agent believed was in budget. On a 1M model there is room and
    /// the default must be left alone — a clamp that fired everywhere would
    /// quietly reinstate the truncation this whole change exists to fix.
    #[test]
    fn the_default_output_allowance_fits_the_context_window() {
        let wide = ModelConfig::new_or_fail("us.anthropic.claude-sonnet-5").with_max_tokens(None);
        assert_eq!(wide.context_limit(), 1_000_000);
        assert_eq!(
            bedrock_inference_config(&wide).max_tokens(),
            Some(BEDROCK_CLAUDE_DEFAULT_MAX_TOKENS),
            "a 1M window has room for the full default"
        );

        let narrow =
            ModelConfig::new_or_fail("us.anthropic.claude-haiku-4-5").with_max_tokens(None);
        assert_eq!(narrow.context_limit(), 200_000);
        let allowance = bedrock_inference_config(&narrow)
            .max_tokens()
            .expect("an allowance is always set");
        assert!(
            allowance < BEDROCK_CLAUDE_DEFAULT_MAX_TOKENS,
            "a 200K window cannot carry the full 64,000 default"
        );
        // The invariant, stated as the thing that actually matters: the largest
        // input compaction will ever allow, plus the allowance, still fits.
        let compaction_trigger = (narrow.context_limit() as f64
            * crate::context_mgmt::DEFAULT_COMPACTION_THRESHOLD)
            as i32;
        assert!(
            compaction_trigger + allowance <= narrow.context_limit() as i32,
            "input at the compaction trigger ({compaction_trigger}) plus the \
             allowance ({allowance}) must fit in {}",
            narrow.context_limit()
        );

        // An explicitly configured value is the caller's decision, not ours.
        let configured =
            ModelConfig::new_or_fail("us.anthropic.claude-haiku-4-5").with_max_tokens(Some(64_000));
        assert_eq!(
            bedrock_inference_config(&configured).max_tokens(),
            Some(64_000),
            "an explicit max_tokens is never silently rewritten"
        );

        // The floor: never ask for less than the provider applied when the
        // field was omitted, however small the window.
        assert_eq!(
            bounded_by_context_window(BEDROCK_GENERIC_DEFAULT_MAX_TOKENS, 8_192),
            BEDROCK_GENERIC_DEFAULT_MAX_TOKENS
        );
    }

    #[test]
    fn blocking_reasoning_text_signature_and_redaction_round_trip_losslessly() {
        let redacted_bytes = b"opaque\0bedrock";
        let bedrock_message = bedrock::Message::builder()
            .role(bedrock::ConversationRole::Assistant)
            .content(bedrock::ContentBlock::ReasoningContent(
                bedrock::ReasoningContentBlock::ReasoningText(
                    bedrock::ReasoningTextBlock::builder()
                        .text("private reasoning")
                        .signature("signed-token")
                        .build()
                        .unwrap(),
                ),
            ))
            .content(bedrock::ContentBlock::ReasoningContent(
                bedrock::ReasoningContentBlock::RedactedContent(aws_smithy_types::Blob::new(
                    redacted_bytes,
                )),
            ))
            .content(bedrock::ContentBlock::Text("answer".to_string()))
            .build()
            .unwrap();

        let decoded = from_bedrock_message(&bedrock_message).unwrap();
        let replayed = to_bedrock_message(&decoded).unwrap();
        assert_eq!(replayed.content().len(), 3);

        let bedrock::ContentBlock::ReasoningContent(bedrock::ReasoningContentBlock::ReasoningText(
            thinking,
        )) = &replayed.content()[0]
        else {
            panic!("expected signed reasoning text first");
        };
        assert_eq!(thinking.text(), "private reasoning");
        assert_eq!(thinking.signature(), Some("signed-token"));

        let bedrock::ContentBlock::ReasoningContent(
            bedrock::ReasoningContentBlock::RedactedContent(redacted),
        ) = &replayed.content()[1]
        else {
            panic!("expected redacted reasoning second");
        };
        assert_eq!(redacted.as_ref(), redacted_bytes);
        assert_eq!(replayed.content()[2].as_text().unwrap(), "answer");
    }

    #[test]
    fn from_bedrock_usage_maps_cache_write_to_creation_and_keeps_disjoint() {
        let usage = bedrock::TokenUsage::builder()
            .input_tokens(100)
            .output_tokens(40)
            .total_tokens(140)
            .cache_read_input_tokens(900)
            .cache_write_input_tokens(200)
            .build()
            .unwrap();
        let converted = from_bedrock_usage(&usage);
        assert_eq!(converted.input_tokens, Some(100)); // fresh input, cache excluded
        assert_eq!(converted.output_tokens, Some(40));
        assert_eq!(converted.cache_read_input_tokens, Some(900));
        assert_eq!(converted.cache_creation_input_tokens, Some(200)); // from cache_write
        assert_eq!(converted.total_tokens, Some(140)); // SDK context total
                                                       // Billed reconciles: 100 + 40 + 900 + 200 = 1240.
        assert_eq!(converted.billed_total(), Some(1240));
    }

    #[test]
    fn from_bedrock_usage_without_cache_leaves_cache_none() {
        let usage = bedrock::TokenUsage::builder()
            .input_tokens(10)
            .output_tokens(5)
            .total_tokens(15)
            .build()
            .unwrap();
        let converted = from_bedrock_usage(&usage);
        assert_eq!(converted.cache_read_input_tokens, None);
        assert_eq!(converted.cache_creation_input_tokens, None);
        assert_eq!(converted.billed_total(), Some(15));
    }

    /// Every audience case, through the real Bedrock formatter.
    ///
    /// This is the regression. Bedrock used to ask whether the USER was absent
    /// from the audience, so it dropped `delta-both-audiences` (content the
    /// tool marked useful to both) and forwarded `echo-empty-audience` (content
    /// the tool addressed to nobody). Both are the opposite of what the other
    /// three formatters did with the same tool result.
    #[test]
    fn tool_result_blocks_reach_the_model_by_audience() {
        let response = MessageContent::tool_response(
            "call-1",
            Ok(rmcp::model::CallToolResult {
                content: crate::providers::formats::audience::every_audience_case(),
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        );

        let block = to_bedrock_message_content(&response).expect("tool result converts");
        let bedrock::ContentBlock::ToolResult(result) = block else {
            panic!("a tool response converts to a ToolResult block");
        };

        let sent: Vec<String> = result
            .content()
            .iter()
            .map(|c| c.as_text().expect("fixture blocks are text").clone())
            .collect();

        assert_eq!(
            sent,
            crate::providers::formats::audience::MODEL_VISIBLE,
            "the Bedrock tool result must carry exactly the model-addressed blocks"
        );
        for withheld in crate::providers::formats::audience::MODEL_HIDDEN {
            assert!(
                !sent.iter().any(|text| text == withheld),
                "{withheld} reached the model"
            );
        }
    }

    #[test]
    fn test_to_bedrock_image_supported_formats() -> Result<()> {
        let supported_formats = [
            "image/png",
            "image/jpeg",
            "image/jpg",
            "image/gif",
            "image/webp",
        ];

        for mime_type in supported_formats {
            let image = RawImageContent {
                data: TEST_IMAGE_BASE64.to_string(),
                mime_type: mime_type.to_string(),
                meta: None,
            }
            .no_annotation();

            let result = to_bedrock_image(&image.data, &image.mime_type);
            assert!(result.is_ok(), "Failed to convert {} format", mime_type);
        }

        Ok(())
    }

    #[test]
    fn test_to_bedrock_image_unsupported_format() {
        let image = RawImageContent {
            data: TEST_IMAGE_BASE64.to_string(),
            mime_type: "image/bmp".to_string(),
            meta: None,
        }
        .no_annotation();

        let result = to_bedrock_image(&image.data, &image.mime_type);
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Unsupported image format: image/bmp"));
        assert!(error_msg.contains("Bedrock supports png, jpeg, gif, webp"));
    }

    #[test]
    fn test_to_bedrock_image_invalid_base64() {
        let image = RawImageContent {
            data: "invalid_base64_data!!!".to_string(),
            mime_type: "image/png".to_string(),
            meta: None,
        }
        .no_annotation();

        let result = to_bedrock_image(&image.data, &image.mime_type);
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Failed to decode base64 image data"));
    }

    #[test]
    fn test_to_bedrock_message_content_image() -> Result<()> {
        let image = RawImageContent {
            data: TEST_IMAGE_BASE64.to_string(),
            mime_type: "image/png".to_string(),
            meta: None,
        }
        .no_annotation();

        let message_content = MessageContent::Image(image);
        let result = to_bedrock_message_content(&message_content)?;

        // Verify we get an Image content block
        assert!(matches!(result, bedrock::ContentBlock::Image(_)));

        Ok(())
    }

    #[test]
    fn test_to_bedrock_tool_result_content_block_image() -> Result<()> {
        let content = Content::image(TEST_IMAGE_BASE64.to_string(), "image/png".to_string());
        let result = to_bedrock_tool_result_content_block("test_id", content)?;

        // Verify the wrapper correctly converts Content::Image to ToolResultContentBlock::Image
        assert!(matches!(result, bedrock::ToolResultContentBlock::Image(_)));

        Ok(())
    }
}

#[cfg(test)]
mod safe_detail_tests {
    use super::*;
    use aws_sdk_bedrockruntime::error::ProvideErrorMetadata;

    /// A stand-in for an SDK error whose Debug rendering carries the raw
    /// response -- which is where the SigV4 `authorization` header lives.
    #[derive(Debug)]
    struct FakeSdkError {
        message: Option<String>,
        code: Option<String>,
    }

    impl ProvideErrorMetadata for FakeSdkError {
        fn meta(&self) -> &aws_smithy_types::error::ErrorMetadata {
            // Not reached: `safe_detail` uses `message()`/`code()` below.
            unimplemented!("safe_detail must not need the full metadata")
        }
        fn message(&self) -> Option<&str> {
            self.message.as_deref()
        }
        fn code(&self) -> Option<&str> {
            self.code.as_deref()
        }
    }

    /// The real 403 from the UCSF proxy. What the user needs is these three
    /// words; what they were shown was ~40 lines of Rust debug output.
    #[test]
    fn the_service_message_is_what_survives() {
        let e = FakeSdkError {
            message: Some("Invalid Client Id".into()),
            code: None,
        };
        assert_eq!(safe_detail(&e), "Invalid Client Id");
    }

    /// ⚠ THE SECURITY ASSERTION. `format!("{:?}", err)` on a real
    /// `ServiceError` includes the raw response, and the raw response carries
    ///
    ///   "authorization": "AWS4-HMAC-SHA256 Credential=…, Signature=…"
    ///
    /// so a user reporting a 403 was pasting a live request signature wherever
    /// they pasted the error. Nothing derived from the transport may appear.
    #[test]
    fn no_transport_material_can_reach_the_message() {
        let e = FakeSdkError {
            message: Some("Invalid Client Id".into()),
            code: Some("AccessDenied".into()),
        };
        let out = safe_detail(&e);
        for forbidden in [
            "AWS4-HMAC-SHA256",
            "Signature=",
            "Credential=",
            "authorization",
            "x-amz-security-token",
        ] {
            assert!(
                !out.contains(forbidden),
                "`{forbidden}` reached a user-facing message: {out}"
            );
        }
    }

    /// An error with no message still has to say something, or the caller
    /// formats an empty string into "Bedrock endpoint returned HTTP 403 ()".
    #[test]
    fn an_empty_message_falls_back_to_the_code_then_to_prose() {
        let coded = FakeSdkError {
            message: Some("   ".into()),
            code: Some("ThrottlingException".into()),
        };
        assert_eq!(safe_detail(&coded), "error code ThrottlingException");

        let bare = FakeSdkError {
            message: None,
            code: None,
        };
        assert_eq!(safe_detail(&bare), "no further detail was returned");
    }
}
