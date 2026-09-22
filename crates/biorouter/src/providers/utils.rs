use super::base::{MessageStream, Usage};
use super::errors::GoogleErrorCode;
use crate::config::paths::Paths;
use crate::model::ModelConfig;
use crate::privacy::ProviderTier;
use crate::providers::errors::ProviderError;
use crate::providers::formats::openai::response_to_streaming_message;
use anyhow::{anyhow, Result};
use async_stream::try_stream;
use base64::Engine;
use futures::TryStreamExt;
use regex::Regex;
use reqwest::{Response, StatusCode};
use rmcp::model::{AnnotateAble, ImageContent, RawImageContent};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt::Display;
use std::fs::File;
use std::io;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::pin;
use tokio_stream::StreamExt;
use tokio_util::codec::{FramedRead, LinesCodec};
use tokio_util::io::StreamReader;
use uuid::Uuid;

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub enum ImageFormat {
    OpenAi,
    Anthropic,
}

/// Convert an image content into an image json based on format
pub fn convert_image(image: &ImageContent, image_format: &ImageFormat) -> Value {
    match image_format {
        ImageFormat::OpenAi => json!({
            "type": "image_url",
            "image_url": {
                "url": format!("data:{};base64,{}", image.mime_type, image.data)
            }
        }),
        ImageFormat::Anthropic => json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": image.mime_type,
                "data": image.data,
            }
        }),
    }
}

pub fn filter_extensions_from_system_prompt(system: &str) -> String {
    if let Some(tool_state_start) = system.find("# Current Tool State") {
        let Some(tool_state) = system.get(tool_state_start..) else {
            return system.to_string();
        };
        if let Some(next_section_pos) = tool_state.find("\n# Working on Tasks") {
            let Some(before) = system.get(..tool_state_start) else {
                return system.to_string();
            };
            let Some(after) = tool_state.get(next_section_pos..) else {
                return system.to_string();
            };
            return format!("{}{}", before.trim_end(), after);
        }
        return system
            .get(..tool_state_start)
            .map(|before| before.trim_end().to_string())
            .unwrap_or_else(|| system.to_string());
    }

    // Accept prompts generated before capabilities and extensions gained
    // separate authoritative sections.
    let Some(extensions_start) = system.find("# Extensions") else {
        return system.to_string();
    };

    let Some(after_extensions) = system.get(extensions_start + 1..) else {
        return system.to_string();
    };

    if let Some(next_section_pos) = after_extensions.find("\n# ") {
        let Some(before) = system.get(..extensions_start) else {
            return system.to_string();
        };
        let Some(after) = system.get(extensions_start + next_section_pos + 1..) else {
            return system.to_string();
        };
        format!("{}{}", before.trim_end(), after)
    } else {
        system
            .get(..extensions_start)
            .map(|s| s.trim_end().to_string())
            .unwrap_or_else(|| system.to_string())
    }
}

fn check_context_length_exceeded(text: &str) -> bool {
    let check_phrases = [
        "context_length_exceeded",
        "maximum context length",
        "prompt is too long",
        "prompt too long",
        "input is too long",
        "input too long",
    ];
    let text_lower = text.to_lowercase();
    check_phrases
        .iter()
        .any(|phrase| text_lower.contains(phrase))
        || (text_lower.contains("exceed")
            && ["context length", "context window", "context limit"]
                .iter()
                .any(|subject| text_lower.contains(subject)))
}

fn format_server_error_message(status_code: StatusCode, payload: Option<&Value>) -> String {
    match payload {
        Some(Value::Null) | None => format!(
            "HTTP {}: No response body received from server",
            status_code.as_u16()
        ),
        Some(p) => format!("HTTP {}: {}", status_code.as_u16(), p),
    }
}

pub fn map_http_error_to_provider_error(
    status: StatusCode,
    payload: Option<Value>,
) -> ProviderError {
    let message = payload.as_ref().and_then(|p| {
        p.get("error")
            .and_then(|e| e.get("message"))
            .or_else(|| p.get("message"))
            .and_then(Value::as_str)
    });
    let extract_message = || -> String {
        message
            .map(String::from)
            .unwrap_or_else(|| payload.as_ref().map(|p| p.to_string()).unwrap_or_default())
    };

    let error = match status {
        StatusCode::OK => unreachable!("Should not call this function with OK status"),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProviderError::Authentication(format!(
            "Authentication failed. Status: {}. Response: {}",
            status,
            extract_message()
        )),
        StatusCode::NOT_FOUND => {
            ProviderError::RequestFailed(format!("Resource not found (404): {}", extract_message()))
        }
        StatusCode::PAYLOAD_TOO_LARGE => ProviderError::ContextLengthExceeded(extract_message()),
        StatusCode::BAD_REQUEST => {
            let payload_str = extract_message();
            let has_context_code = payload.as_ref().is_some_and(|payload| {
                payload.pointer("/error/code").and_then(Value::as_str)
                    == Some("context_length_exceeded")
                    || payload.get("code").and_then(Value::as_str)
                        == Some("context_length_exceeded")
            });
            if has_context_code || message.is_some_and(check_context_length_exceeded) {
                ProviderError::ContextLengthExceeded(payload_str)
            } else {
                ProviderError::RequestFailed(format!("Bad request (400): {}", payload_str))
            }
        }
        StatusCode::TOO_MANY_REQUESTS => ProviderError::RateLimitExceeded {
            details: extract_message(),
            retry_delay: None,
        },
        _ if status.is_server_error() => {
            ProviderError::ServerError(format!("Server error ({}): {}", status, extract_message()))
        }
        _ => ProviderError::RequestFailed(format!(
            "Request failed with status {}: {}",
            status,
            extract_message()
        )),
    };

    if !status.is_success() {
        tracing::warn!(
            status = status.as_u16(),
            error_type = error.telemetry_type(),
            "Provider request failed"
        );
    }

    error
}

pub async fn handle_status_openai_compat(response: Response) -> Result<Response, ProviderError> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let payload = serde_json::from_str::<Value>(&body).ok();
        return Err(map_http_error_to_provider_error(status, payload));
    }
    Ok(response)
}

pub async fn handle_response_openai_compat(response: Response) -> Result<Value, ProviderError> {
    let response = handle_status_openai_compat(response).await?;

    response.json::<Value>().await.map_err(|e| {
        ProviderError::RequestFailed(format!("Response body is not valid JSON: {}", e))
    })
}

/// The Azure OpenAI chat-completions path, shared by every Azure-shaped
/// provider (`azure`, `versa_azure`) and by both their blocking and streaming
/// paths.
///
/// One string in one place on purpose: `supports_streaming()` is hardcoded true
/// on those providers, so there is no fallback to `complete()` if the path is
/// wrong — a drift here 404s every streaming turn outright rather than
/// degrading. Duplicating the `format!` per provider meant a change made in the
/// tested copy could silently miss the untested one.
pub fn azure_chat_completions_path(deployment_name: &str, api_version: &str) -> String {
    format!(
        "openai/deployments/{}/chat/completions?api-version={}",
        deployment_name, api_version
    )
}

pub fn stream_openai_compat(
    response: Response,
    mut log: RequestLog,
) -> Result<MessageStream, ProviderError> {
    let stream = response.bytes_stream().map_err(io::Error::other);

    Ok(Box::pin(try_stream! {
        let stream_reader = StreamReader::new(stream);
        let framed = FramedRead::new(stream_reader, LinesCodec::new())
            .map_err(anyhow::Error::from);

        let message_stream = response_to_streaming_message(framed);
        pin!(message_stream);
        while let Some(message) = message_stream.next().await {
            let (message, usage, pending) = message.map_err(|error| {
                error.downcast::<ProviderError>().unwrap_or_else(|error|
                    ProviderError::RequestFailed(format!("Stream decode error: {}", error))
                )
            })?;
            log.write(&message, usage.as_ref().map(|f| f.usage).as_ref())?;
            yield (message, usage, pending);
        }
    }))
}

pub fn is_google_model(payload: &Value) -> bool {
    payload
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_lowercase()
        .contains("google")
}

/// Extracts `StatusCode` from response status or payload error code.
/// This function first checks the status code of the response. If the status is successful (2xx),
/// it then checks the payload for any error codes and maps them to appropriate `StatusCode`.
/// If the status is not successful (e.g., 4xx or 5xx), the original status code is returned.
fn get_google_final_status(status: StatusCode, payload: Option<&Value>) -> StatusCode {
    // If the status is successful, check for an error in the payload
    if status.is_success() {
        if let Some(payload) = payload {
            if let Some(error) = payload.get("error") {
                if let Some(code) = error.get("code").and_then(|c| c.as_u64()) {
                    if let Some(google_error) = GoogleErrorCode::from_code(code) {
                        return google_error.to_status_code();
                    }
                }
            }
        }
    }
    status
}

fn parse_google_retry_delay(payload: &Value) -> Option<Duration> {
    payload
        .get("error")
        .and_then(|error| error.get("details"))
        .and_then(|details| details.as_array())
        .and_then(|details_array| {
            details_array.iter().find_map(|detail| {
                if detail
                    .get("@type")
                    .and_then(|t| t.as_str())
                    .is_some_and(|s| s.ends_with("RetryInfo"))
                {
                    detail
                        .get("retryDelay")
                        .and_then(|delay| delay.as_str())
                        .and_then(|s| s.strip_suffix('s'))
                        .and_then(|num| num.parse::<u64>().ok())
                        .map(Duration::from_secs)
                } else {
                    None
                }
            })
        })
}

/// Handle response from Google Gemini API-compatible endpoints.
///
/// Processes HTTP responses, handling specific statuses and parsing the payload
/// for error messages. Diagnostics retain only the response status.
///
/// ### References
/// - Error Codes: https://ai.google.dev/gemini-api/docs/troubleshooting?lang=python
///
/// ### Arguments
/// - `response`: The HTTP response to process.
///
/// ### Returns
/// - `Ok(Value)`: Parsed JSON on success.
/// - `Err(ProviderError)`: Describes the failure reason.
pub async fn handle_response_google_compat(response: Response) -> Result<Value, ProviderError> {
    let status = response.status();
    let payload: Option<Value> = response.json().await.ok();
    let final_status = get_google_final_status(status, payload.as_ref());

    match final_status {
        StatusCode::OK =>  payload.ok_or_else( || ProviderError::RequestFailed("Response body is not valid JSON".to_string()) ),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            Err(ProviderError::Authentication(format!("Authentication failed. Please ensure your API keys are valid and have the required permissions. \
                Status: {}. Response: {:?}", final_status, payload )))
        }
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND => {
            let mut error_msg = "Unknown error".to_string();
            if let Some(payload) = &payload {
                if let Some(error) = payload.get("error") {
                    error_msg = error.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error").to_string();
                    let error_status = error.get("status").and_then(|s| s.as_str()).unwrap_or("Unknown status");
                    if error_status == "INVALID_ARGUMENT" && error_msg.to_lowercase().contains("exceeds") {
                        return Err(ProviderError::ContextLengthExceeded(error_msg.to_string()));
                    }
                }
            }
            tracing::debug!(status = final_status.as_u16(), "Provider request failed");
            Err(ProviderError::RequestFailed(format!("Request failed with status: {}. Message: {}", final_status, error_msg)))
        }
        StatusCode::TOO_MANY_REQUESTS => {
            let retry_delay = payload.as_ref().and_then(parse_google_retry_delay);
            Err(ProviderError::RateLimitExceeded {
                details: format!("{:?}", payload),
                retry_delay,
            })
        }
        _ if final_status.is_server_error() => Err(ProviderError::ServerError(
            format_server_error_message(final_status, payload.as_ref()),
        )),
        _ => {
            tracing::debug!(status = final_status.as_u16(), "Provider request failed");
            Err(ProviderError::RequestFailed(format!("Request failed with status: {}", final_status)))
        }
    }
}

pub fn sanitize_function_name(name: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"[^a-zA-Z0-9_-]").unwrap());
    re.replace_all(name, "_").to_string()
}

pub fn is_valid_function_name(name: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^[a-zA-Z0-9_-]+$").unwrap());
    re.is_match(name)
}

/// Extract the model name from a JSON object. Common with most providers to have this top level attribute.
pub fn get_model(data: &Value) -> String {
    if let Some(model) = data.get("model") {
        if let Some(model_str) = model.as_str() {
            model_str.to_string()
        } else {
            "Unknown".to_string()
        }
    } else {
        "Unknown".to_string()
    }
}

/// Check if a file is actually an image by examining its magic bytes
fn is_image_file(path: &Path) -> bool {
    if let Ok(mut file) = std::fs::File::open(path) {
        let mut buffer = [0u8; 8]; // Large enough for most image magic numbers
        if file.read(&mut buffer).is_ok() {
            // Check magic numbers for common image formats
            return match &buffer[0..4] {
                // PNG: 89 50 4E 47
                [0x89, 0x50, 0x4E, 0x47] => true,
                // JPEG: FF D8 FF
                [0xFF, 0xD8, 0xFF, _] => true,
                // GIF: 47 49 46 38
                [0x47, 0x49, 0x46, 0x38] => true,
                _ => false,
            };
        }
    }
    false
}

/// Detect if a string contains a path to an image file
pub fn detect_image_path(text: &str) -> Option<&str> {
    // Basic image file extension check
    let extensions = [".png", ".jpg", ".jpeg"];

    // Find any word that ends with an image extension
    for word in text.split_whitespace() {
        if extensions
            .iter()
            .any(|ext| word.to_lowercase().ends_with(ext))
        {
            let path = Path::new(word);
            // Check if it's an absolute path and file exists
            if path.is_absolute() && path.is_file() {
                // Verify it's actually an image file
                if is_image_file(path) {
                    return Some(word);
                }
            }
        }
    }
    None
}

/// Convert a local image file to base64 encoded ImageContent
pub fn load_image_file(path: &str) -> Result<ImageContent, ProviderError> {
    let path = Path::new(path);

    // Verify it's an image before proceeding
    if !is_image_file(path) {
        return Err(ProviderError::RequestFailed(
            "File is not a valid image".to_string(),
        ));
    }

    // Read the file
    let bytes = std::fs::read(path)
        .map_err(|e| ProviderError::RequestFailed(format!("Failed to read image file: {}", e)))?;

    // Detect mime type from extension
    let mime_type = match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => match ext.to_lowercase().as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            _ => {
                return Err(ProviderError::RequestFailed(
                    "Unsupported image format".to_string(),
                ))
            }
        },
        None => {
            return Err(ProviderError::RequestFailed(
                "Unknown image format".to_string(),
            ))
        }
    };

    // Convert to base64
    let data = base64::prelude::BASE64_STANDARD.encode(&bytes);

    Ok(RawImageContent {
        mime_type: mime_type.to_string(),
        data,
        meta: None,
    }
    .no_annotation())
}

pub fn unescape_json_values(value: &Value) -> Value {
    let mut cloned = value.clone();
    unescape_json_values_in_place(&mut cloned);
    cloned
}

fn unescape_json_values_in_place(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for v in map.values_mut() {
                unescape_json_values_in_place(v);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                unescape_json_values_in_place(v);
            }
        }
        Value::String(s) => {
            if s.contains('\\') {
                *s = s
                    .replace("\\\\n", "\n")
                    .replace("\\\\t", "\t")
                    .replace("\\\\r", "\r")
                    .replace("\\\\\"", "\"")
                    .replace("\\n", "\n")
                    .replace("\\t", "\t")
                    .replace("\\r", "\r")
                    .replace("\\\"", "\"");
            }
        }
        _ => {}
    }
}

/// What a [`RequestLog`] may write about the exchange it is logging.
///
/// The privacy question this answers is not "is the log file protected?" — it
/// is "what leaves the machine?". `<state>/logs/llm_request.*.jsonl` is zipped
/// into the diagnostics bundle a user emails to support believing they sent a
/// bug report, so a prompt written here is a prompt disclosed to a third party.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadPolicy {
    /// The prompt, the response and every streamed chunk are written verbatim.
    Full,
    /// Only metadata: the model config, the session, timings, token usage and
    /// redacted errors. No prompt, completion, tool arguments, or upstream error text.
    MetadataOnly,
}

impl PayloadPolicy {
    /// The ONE place a provider tier becomes a logging decision.
    ///
    /// Private-tier means the model is reached without the content leaving the
    /// user's machine or their institution's boundary; writing that content
    /// into a bundle destined for a public issue tracker undoes exactly that.
    pub const fn for_tier(tier: ProviderTier) -> Self {
        if tier.is_private() {
            Self::MetadataOnly
        } else {
            Self::Full
        }
    }

    pub const fn is_metadata_only(self) -> bool {
        matches!(self, Self::MetadataOnly)
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::MetadataOnly => "metadata-only",
        }
    }
}

/// Per-request debug log of the LLM exchange (`<state>/logs/llm_request.N.jsonl`).
///
/// BR-57: this is written on **every** request and, while streaming, once per
/// decoded chunk — interleaved between token batches on the async runtime. To
/// keep blocking file I/O off that runtime the lines are buffered in memory
/// ([`write`](Self::write) does no syscalls) and the whole exchange is flushed
/// and rotated in a single `spawn_blocking` at [`finish`](Self::finish) time.
///
/// ⚠ **Every log records the session it belongs to, and a log that cannot name
/// one is unattributable.** `generate_diagnostics` ships only the logs whose
/// header names the session the user asked to report; see
/// `crate::session::diagnostics`. That is why the header line is written even
/// under [`PayloadPolicy::MetadataOnly`] — dropping it would not make the file
/// private, it would make the file unfilterable.
pub struct RequestLog {
    /// Buffered JSONL lines (each already serialized, no trailing newline).
    /// `None` once the log has been flushed, so `finish` is idempotent.
    lines: Option<Vec<String>>,
    temp_path: PathBuf,
    /// Decided once, at [`start_with_tier`](Self::start_with_tier), and never
    /// re-read: a policy sampled again per chunk could change mid-stream if the
    /// bound provider were swapped, and half a redacted transcript is a leaked
    /// transcript.
    policy: PayloadPolicy,
    started: Instant,
    /// Response chunks suppressed by [`PayloadPolicy::MetadataOnly`], and their
    /// combined serialized size. Kept so the log still says *how much* came
    /// back, which is what a truncation or a runaway-response report needs.
    redacted_chunks: u64,
    redacted_bytes: u64,
}

pub const LOGS_TO_KEEP: usize = 10;

impl RequestLog {
    /// Open a log for a request whose provider tier is **not known here**.
    ///
    /// ⚠ This resolves to [`ProviderTier::Private`], i.e. metadata only, and
    /// that is deliberate. `RequestLog` is constructed from inside a provider's
    /// `complete`/`stream` body, which hands it a [`ModelConfig`] and nothing
    /// else — no `&dyn Provider`, so no `tier()`. A tier this constructor
    /// guessed (from the configured provider name, say) would be wrong in the
    /// leaking direction the moment a session binds a provider other than the
    /// global default, which is the ordinary case for a private chat. Guessing
    /// Public to keep the debug log rich is the bug this fixes; guessing Private
    /// costs prompt text in a debug file and can never disclose a private
    /// transcript.
    ///
    /// A provider that knows its own tier should call
    /// [`start_with_tier`](Self::start_with_tier) with `self.tier()` — that is
    /// the seam, and it lives in the provider modules, not here.
    pub fn start<Payload>(model_config: &ModelConfig, payload: &Payload) -> Result<Self>
    where
        Payload: Serialize,
    {
        Self::start_with_tier(ProviderTier::Private, model_config, payload)
    }

    /// Open a log for a request whose provider tier **is** known.
    pub fn start_with_tier<Payload>(
        tier: ProviderTier,
        model_config: &ModelConfig,
        payload: &Payload,
    ) -> Result<Self>
    where
        Payload: Serialize,
    {
        Self::start_in(&Paths::in_state_dir("logs"), tier, model_config, payload)
    }

    /// [`start_with_tier`](Self::start_with_tier), writing under `logs_dir`.
    ///
    /// Every production log goes through `start_with_tier`, i.e. the state
    /// dir as `Paths` says it is when the request starts. This seam exists for
    /// the tests below that read a log back: they pass a directory of their
    /// own instead of pointing `BIOROUTER_PATH_ROOT` at one, because the
    /// provider tests in this binary open logs through `start` without the env
    /// lock, and one that starts while such a test holds the variable writes
    /// into that test's directory and rotates its `llm_request.0.jsonl` out
    /// from under it. Forced (a probe opening one inside
    /// `a_public_tier_log_still_writes_the_exchange`'s window), the old shape
    /// failed 10 of 10 on `contents.contains("hello there")`.
    fn start_in<Payload>(
        logs_dir: &Path,
        tier: ProviderTier,
        model_config: &ModelConfig,
        payload: &Payload,
    ) -> Result<Self>
    where
        Payload: Serialize,
    {
        let request_id = Uuid::new_v4();
        let temp_name = format!("llm_request.{request_id}.jsonl");
        let temp_path = logs_dir.join(PathBuf::from(temp_name));

        let policy = PayloadPolicy::for_tier(tier);
        let encoded = serde_json::to_string(payload)?;

        // `model_config` is model name / limits / sampling knobs — no user
        // content — so it is metadata under either policy.
        let mut header = serde_json::json!({
            "session_id": crate::session_context::current_session_id(),
            "provider_tier": tier,
            "payloads": policy.as_str(),
            "model_config": model_config,
            "input_bytes": encoded.len(),
        });
        if !policy.is_metadata_only() {
            header["input"] = serde_json::from_str(&encoded)?;
        }

        // Buffer the opening line in memory — no file is opened on the async
        // runtime; the disk work is deferred to `finish` (`spawn_blocking`).
        let lines = vec![serde_json::to_string(&header)?];

        Ok(Self {
            lines: Some(lines),
            temp_path,
            policy,
            started: Instant::now(),
            redacted_chunks: 0,
            redacted_bytes: 0,
        })
    }

    /// The policy this log was opened under. Exposed so a test can assert on
    /// the decision as well as on its output.
    pub fn policy(&self) -> PayloadPolicy {
        self.policy
    }

    fn write_json(&mut self, line: &serde_json::Value) -> Result<()> {
        let lines = self
            .lines
            .as_mut()
            .ok_or_else(|| anyhow!("logger is finished"))?;
        lines.push(serde_json::to_string(line)?);
        Ok(())
    }

    pub fn error<E>(&mut self, error: E) -> Result<()>
    where
        E: Display,
    {
        if self.policy.is_metadata_only() {
            return self.write_json(&json!({"error": "[redacted]", "error_redacted": true}));
        }
        self.write_json(&serde_json::json!({
            "error": format!("{}", error),
        }))
    }

    pub fn provider_error(&mut self, error: &ProviderError) -> Result<()> {
        if self.policy.is_metadata_only() {
            return self.write_json(&json!({
                "error": "[redacted]",
                "error_redacted": true,
                "error_type": error.telemetry_type(),
            }));
        }
        self.error(error)
    }

    pub fn write<Payload>(&mut self, data: &Payload, usage: Option<&Usage>) -> Result<()>
    where
        Payload: Serialize,
    {
        if self.policy.is_metadata_only() {
            self.redacted_chunks += 1;
            self.redacted_bytes += serde_json::to_string(data)?.len() as u64;
            // Token counts are metadata and are what a usage report needs; the
            // chunks that carry no usage carry only content, so they vanish.
            if usage.is_some() {
                return self.write_json(&serde_json::json!({ "usage": usage }));
            }
            return Ok(());
        }

        self.write_json(&serde_json::json!({
            "data": data,
            "usage": usage,
        }))
    }

    fn finish(&mut self) -> Result<()> {
        // Timings are metadata under either policy, and under
        // `MetadataOnly` this trailer is what keeps the log diagnostic:
        // it still says how long the call took and how much came back.
        if self.lines.is_some() {
            let trailer = serde_json::json!({
                "elapsed_ms": self.started.elapsed().as_millis() as u64,
                "payloads": self.policy.as_str(),
                "redacted_response_chunks": self.redacted_chunks,
                "redacted_response_bytes": self.redacted_bytes,
            });
            self.write_json(&trailer)?;
        }

        let Some(lines) = self.lines.take() else {
            return Ok(());
        };
        let temp_path = std::mem::take(&mut self.temp_path);
        match tokio::runtime::Handle::try_current() {
            // On a runtime (the streaming/agent path): move the open + write +
            // rotate off the async worker. Detached — the turn does not wait on
            // it; a normally-dropped runtime still drains in-flight blocking
            // tasks, so shutdown flushes the last request's log.
            Ok(handle) => {
                handle.spawn_blocking(move || {
                    if let Err(e) = flush_request_log(lines, temp_path) {
                        tracing::debug!("failed to flush LLM request log: {e}");
                    }
                });
                Ok(())
            }
            // No runtime in scope (sync context / unit tests): flush inline.
            Err(_) => flush_request_log(lines, temp_path),
        }
    }
}

/// Write the buffered request log to its temp file, then rotate the numbered
/// logs so the newest becomes `llm_request.0.jsonl` and only [`LOGS_TO_KEEP`]
/// are retained. This is entirely blocking file I/O and must run off the async
/// runtime (via `spawn_blocking`) — see [`RequestLog`].
fn flush_request_log(lines: Vec<String>, temp_path: PathBuf) -> Result<()> {
    let logs_dir = temp_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| Paths::in_state_dir("logs"));
    // `start` no longer opens a file, so ensure the directory exists here.
    let _ = std::fs::create_dir_all(&logs_dir);

    let mut writer = BufWriter::new(
        File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_path)?,
    );
    for line in &lines {
        writeln!(writer, "{line}")?;
    }
    writer.flush()?;
    drop(writer);

    let log_path = |i| logs_dir.join(format!("llm_request.{}.jsonl", i));
    for i in (0..LOGS_TO_KEEP - 1).rev() {
        let _ = std::fs::rename(log_path(i), log_path(i + 1));
    }
    std::fs::rename(&temp_path, log_path(0))?;
    Ok(())
}

impl Drop for RequestLog {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let _ = self.finish();
    }
}

/// Safely parse a JSON string that may contain doubly-encoded or malformed JSON.
/// This function first attempts to parse the input string as-is. If that fails,
/// it applies control character escaping and tries again.
///
/// This approach preserves valid JSON like `{"key1": "value1",\n"key2": "value"}`
/// (which contains a literal \n but is perfectly valid JSON) while still fixing
/// broken JSON like `{"key1": "value1\n","key2": "value"}` (which contains an
/// unescaped newline character).
pub fn safely_parse_json(s: &str) -> Result<serde_json::Value, serde_json::Error> {
    // First, try parsing the string as-is
    match serde_json::from_str(s) {
        Ok(value) => Ok(value),
        Err(_) => {
            // If that fails, try with control character escaping
            let escaped = json_escape_control_chars_in_string(s);
            serde_json::from_str(&escaped)
        }
    }
}

/// Helper to escape control characters in a string that is supposed to be a JSON document.
/// This function iterates through the input string `s` and replaces any literal
/// control characters (U+0000 to U+001F) with their JSON-escaped equivalents
/// (e.g., '\n' becomes "\\n", '\u0001' becomes "\\u0001").
///
/// It does NOT escape quotes (") or backslashes (\) because it assumes `s` is a
/// full JSON document, and these characters might be structural (e.g., object delimiters,
/// existing valid escape sequences). The goal is to fix common LLM errors where
/// control characters are emitted raw into what should be JSON string values,
/// making the overall JSON structure unparsable.
///
/// If the input string `s` has other JSON syntax errors (e.g., an unescaped quote
/// *within* a string value like `{"key": "string with " quote"}`), this function
/// will not fix them. It specifically targets unescaped control characters.
pub fn json_escape_control_chars_in_string(s: &str) -> String {
    let mut r = String::with_capacity(s.len()); // Pre-allocate for efficiency
    for c in s.chars() {
        match c {
            // ASCII Control characters (U+0000 to U+001F)
            '\u{0000}'..='\u{001F}' => {
                match c {
                    '\u{0008}' => r.push_str("\\b"), // Backspace
                    '\u{000C}' => r.push_str("\\f"), // Form feed
                    '\n' => r.push_str("\\n"),       // Line feed
                    '\r' => r.push_str("\\r"),       // Carriage return
                    '\t' => r.push_str("\\t"),       // Tab
                    // Other control characters (e.g., NUL, SOH, VT, etc.)
                    // that don't have a specific short escape sequence.
                    _ => {
                        r.push_str(&format!("\\u{:04x}", c as u32));
                    }
                }
            }
            // Other characters are passed through.
            // This includes quotes (") and backslashes (\). If these are part of the
            // JSON structure (e.g. {"key": "value"}) or part of an already correctly
            // escaped sequence within a string value (e.g. "string with \\\" quote"),
            // they are preserved as is. This function does not attempt to fix
            // malformed quote or backslash usage *within* string values if the LLM
            // generates them incorrectly (e.g. {"key": "unescaped " quote in string"}).
            _ => r.push(c),
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn http_context_classifier_keeps_tool_array_limits_as_request_failures() {
        let message = "Invalid 'tools': array too long. Expected at most 128 items, but got 129.";
        let error = map_http_error_to_provider_error(
            StatusCode::BAD_REQUEST,
            Some(json!({"error": {
                "message": message,
                "code": "array_above_max_length",
                "param": "tools"
            }})),
        );
        assert_eq!(
            error,
            ProviderError::RequestFailed(format!("Bad request (400): {message}"))
        );
    }

    #[test]
    fn http_context_classifier_keeps_tool_description_limits_as_request_failures() {
        let mut failures = Vec::new();
        for message in [
            "Invalid 'tools[0].function.description': string too long. Expected at most 1024 characters, but got 1200.",
            "Invalid 'tools[0].function.description': input length exceeds 1024 characters.",
        ] {
            let error = map_http_error_to_provider_error(
                StatusCode::BAD_REQUEST,
                Some(json!({"error": {
                    "message": message,
                    "code": "string_above_max_length",
                    "param": "tools[0].function.description"
                }})),
            );
            if error != ProviderError::RequestFailed(format!("Bad request (400): {message}")) {
                failures.push(format!("{message}: {error:?}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn http_context_classifier_keeps_output_parameter_errors_as_request_failures() {
        let mut failures = Vec::new();
        for (code, param, message) in [
            (
                "unsupported_parameter",
                "max_tokens",
                "Unsupported parameter: 'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead.",
            ),
            (
                "invalid_value",
                "max_tokens",
                "Invalid 'max_tokens': integer must be greater than or equal to 1.",
            ),
            (
                "invalid_value",
                "max_tokens",
                "max_tokens exceeds the maximum allowed output token count of 16384.",
            ),
            (
                "invalid_value",
                "max_completion_tokens",
                "max_completion_tokens exceeds the maximum allowed output token count of 128000.",
            ),
        ] {
            let error = map_http_error_to_provider_error(
                StatusCode::BAD_REQUEST,
                Some(json!({"error": {
                    "message": message,
                    "code": code,
                    "param": param
                }})),
            );
            if error != ProviderError::RequestFailed(format!("Bad request (400): {message}")) {
                failures.push(format!("parameter: {param}; code: {code}; error: {error:?}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn http_context_classifier_preserves_explicit_context_messages() {
        let mut failures = Vec::new();
        for message in [
            "This model's maximum context length is 8192 tokens. Your messages resulted in 9000 tokens.",
            "Request exceeds the model's context window.",
            "Input length exceeds the context limit of 8192 tokens.",
            "Prompt is too long: 9000 tokens > 8192 maximum.",
            "Input is too long for requested model.",
            "input is too long",
            "The prompt is too long",
            "Request exceeds the maximum context length.",
            "context_length_exceeded",
        ] {
            let error = map_http_error_to_provider_error(
                StatusCode::BAD_REQUEST,
                Some(json!({"error": {"message": message}})),
            );
            if error != ProviderError::ContextLengthExceeded(message.to_string()) {
                failures.push(format!("{message}: {error:?}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn http_context_classifier_rejects_non_overflow_context_mentions() {
        let mut failures = Vec::new();
        for message in [
            "Invalid context length: must be positive.",
            "Unsupported parameter: context_length.",
        ] {
            let error = map_http_error_to_provider_error(
                StatusCode::BAD_REQUEST,
                Some(json!({"error": {
                    "message": message,
                    "code": "invalid_request_error"
                }})),
            );
            if error != ProviderError::RequestFailed(format!("Bad request (400): {message}")) {
                failures.push(format!("{message}: {error:?}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn http_context_classifier_recognizes_structured_context_codes() {
        let message = "Request rejected.";
        let mut failures = Vec::new();
        for payload in [
            json!({"error": {
                "code": "context_length_exceeded",
                "message": message
            }}),
            json!({
                "code": "context_length_exceeded",
                "message": message
            }),
        ] {
            let error = map_http_error_to_provider_error(StatusCode::BAD_REQUEST, Some(payload));
            if error != ProviderError::ContextLengthExceeded(message.to_string()) {
                failures.push(format!("{error:?}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn http_context_classifier_malformed_payloads_are_not_context_signals() {
        let mut failures = Vec::new();
        for (label, payload) in [
            ("absent payload", None),
            ("null payload", Some(Value::Null)),
            ("empty payload", Some(json!({}))),
            ("empty code", Some(json!({"error": {"code": ""}}))),
            ("null code", Some(json!({"error": {"code": null}}))),
            ("numeric code", Some(json!({"error": {"code": 42}}))),
            (
                "array code",
                Some(json!({"error": {"code": ["context_length_exceeded"]}})),
            ),
            (
                "object code",
                Some(json!({"error": {"code": {"detail": "context_length_exceeded"}}})),
            ),
            (
                "non-exact string code",
                Some(json!({"error": {"code": "not_context_length_exceeded"}})),
            ),
            (
                "top-level array code",
                Some(json!({"code": ["context_length_exceeded"]})),
            ),
            (
                "unrelated detail",
                Some(json!({"error": {"detail": "context_length_exceeded"}})),
            ),
            (
                "object message",
                Some(json!({"error": {"message": {"detail": "context_length_exceeded"}}})),
            ),
            (
                "array message",
                Some(json!({"message": ["context_length_exceeded"]})),
            ),
            (
                "nested null message retains precedence",
                Some(json!({
                    "error": {"message": null},
                    "message": "context_length_exceeded"
                })),
            ),
            ("string payload", Some(json!("context_length_exceeded"))),
            (
                "array payload",
                Some(json!([{"code": "context_length_exceeded"}])),
            ),
        ] {
            let details = payload.as_ref().map(Value::to_string).unwrap_or_default();
            let error = map_http_error_to_provider_error(StatusCode::BAD_REQUEST, payload);
            if error != ProviderError::RequestFailed(format!("Bad request (400): {details}")) {
                failures.push(format!("{label}: {error:?}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn http_context_classifier_context_codes_without_messages_remain_context() {
        let mut failures = Vec::new();
        for payload in [
            json!({"error": {"code": "context_length_exceeded"}}),
            json!({"code": "context_length_exceeded"}),
            json!({"error": {"code": "context_length_exceeded", "message": null}}),
            json!({"code": "context_length_exceeded", "message": {"unexpected": true}}),
        ] {
            let details = payload.to_string();
            let error = map_http_error_to_provider_error(StatusCode::BAD_REQUEST, Some(payload));
            if error != ProviderError::ContextLengthExceeded(details.clone()) {
                failures.push(format!("{details}: {error:?}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn http_context_classifier_preserves_non_bad_request_status_precedence() {
        let message = "context length exceeded";
        let mut failures = Vec::new();
        for (status, expected) in [
            (
                StatusCode::UNAUTHORIZED,
                ProviderError::Authentication(format!(
                    "Authentication failed. Status: {}. Response: {message}",
                    StatusCode::UNAUTHORIZED
                )),
            ),
            (
                StatusCode::FORBIDDEN,
                ProviderError::Authentication(format!(
                    "Authentication failed. Status: {}. Response: {message}",
                    StatusCode::FORBIDDEN
                )),
            ),
            (
                StatusCode::TOO_MANY_REQUESTS,
                ProviderError::RateLimitExceeded {
                    details: message.to_string(),
                    retry_delay: None,
                },
            ),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                ProviderError::ServerError(format!(
                    "Server error ({}): {message}",
                    StatusCode::INTERNAL_SERVER_ERROR
                )),
            ),
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                ProviderError::ContextLengthExceeded(message.to_string()),
            ),
            (
                StatusCode::NOT_FOUND,
                ProviderError::RequestFailed(format!("Resource not found (404): {message}")),
            ),
        ] {
            let error = map_http_error_to_provider_error(
                status,
                Some(json!({"error": {
                    "code": "context_length_exceeded",
                    "message": message
                }})),
            );
            if error != expected {
                failures.push(format!("status: {status}; error: {error:?}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[tokio::test]
    async fn http_context_classifier_openai_wrappers_preserve_error_categories() {
        let mut failures = Vec::new();
        for (code, message, expected) in [
            (
                "array_above_max_length",
                "Invalid 'tools': array too long. Expected at most 128 items, but got 129.",
                ProviderError::RequestFailed(
                    "Bad request (400): Invalid 'tools': array too long. Expected at most 128 items, but got 129.".into(),
                ),
            ),
            (
                "context_length_exceeded",
                "This model's maximum context length is 8192 tokens. Your messages resulted in 9000 tokens.",
                ProviderError::ContextLengthExceeded(
                    "This model's maximum context length is 8192 tokens. Your messages resulted in 9000 tokens.".into(),
                ),
            ),
        ] {
            for streaming in [false, true] {
                let response = axum::http::Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .header("content-type", "application/json")
                    .body(json!({"error": {"code": code, "message": message}}).to_string())
                    .unwrap();
                let response = reqwest::Response::from(response);
                let error = if streaming {
                    handle_status_openai_compat(response).await.err()
                } else {
                    handle_response_openai_compat(response).await.err()
                };
                if error.as_ref() != Some(&expected) {
                    failures.push(format!("streaming: {streaming}; code: {code}; error: {error:?}"));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn filter_extensions_removes_the_complete_current_tool_state_slice() {
        let prompt = "Base\n\n# Current Tool State\n\n# Enabled Capabilities\n\n## developer\n\ndev\n\n# Loaded Extensions\n\n## custom\n\next\n\n# Working on Tasks\n\nwork";
        let filtered = filter_extensions_from_system_prompt(prompt);

        assert_eq!(filtered, "Base\n# Working on Tasks\n\nwork");
        assert!(!filtered.contains("developer"));
        assert!(!filtered.contains("custom"));
    }

    #[test]
    fn filter_extensions_preserves_unicode_around_the_tool_state_slice() {
        let prompt = "Résumé 🧬\n\n# Current Tool State\n\n## developer\n\ndev\n\n# Working on Tasks\n\nMéditation";

        assert_eq!(
            filter_extensions_from_system_prompt(prompt),
            "Résumé 🧬\n# Working on Tasks\n\nMéditation"
        );
    }

    #[test]
    fn filter_extensions_still_accepts_the_legacy_prompt_section() {
        let prompt = "Base\n\n# Extensions\n\n## custom\n\next\n\n# Working on Tasks\n\nwork";
        assert_eq!(
            filter_extensions_from_system_prompt(prompt),
            "Base\n# Working on Tasks\n\nwork"
        );
    }

    // BR-57: the log buffers its lines in memory and only touches the disk when
    // it is finished (inline here, since there is no tokio runtime; on the
    // streaming path the same flush runs in `spawn_blocking`).
    /// A log opened with an explicit policy, writing into `dir` instead of the
    /// real state directory. Only the tests below build one this way; every
    /// production path goes through `start`/`start_with_tier`.
    fn test_log(
        dir: &std::path::Path,
        policy: PayloadPolicy,
        header: serde_json::Value,
    ) -> RequestLog {
        RequestLog {
            lines: Some(vec![header.to_string()]),
            temp_path: dir.join("llm_request.abc.jsonl"),
            policy,
            started: Instant::now(),
            redacted_chunks: 0,
            redacted_bytes: 0,
        }
    }

    #[test]
    fn test_request_log_buffers_then_flushes_on_finish() {
        let dir = tempfile::tempdir().unwrap();
        let temp_path = dir.path().join("llm_request.abc.jsonl");
        let mut log = test_log(dir.path(), PayloadPolicy::Full, json!({"input": "hi"}));

        // `write`/`error` are pure in-memory: nothing is written to disk yet.
        log.write(&json!({"delta": "one"}), None).unwrap();
        log.error("boom").unwrap();
        assert_eq!(log.lines.as_ref().unwrap().len(), 3);
        assert!(!temp_path.exists());
        assert!(!dir.path().join("llm_request.0.jsonl").exists());

        // Finishing flushes every buffered line and rotates into slot 0.
        log.finish().unwrap();
        let slot0 = dir.path().join("llm_request.0.jsonl");
        let contents = std::fs::read_to_string(&slot0).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        // Three buffered lines plus the timing trailer `finish` appends.
        assert_eq!(lines.len(), 4);
        assert!(lines[0].contains("\"input\":\"hi\""));
        assert!(lines[1].contains("\"delta\":\"one\""));
        assert!(lines[2].contains("\"error\":\"boom\""));
        assert!(lines[3].contains("\"elapsed_ms\""));
        // The temp file was consumed by the rename.
        assert!(!temp_path.exists());

        // A second finish (e.g. from Drop) is a no-op.
        log.finish().unwrap();
        assert!(log.lines.is_none());
    }

    #[test]
    fn private_error_logging_redacts_upstream_echoes() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = test_log(dir.path(), PayloadPolicy::MetadataOnly, json!({}));
        log.error(ProviderError::RequestFailed(
            "SYNTHETIC_SECRET_SENTINEL and echoed tool arguments".into(),
        ))
        .unwrap();
        log.finish().unwrap();

        let contents = std::fs::read_to_string(dir.path().join("llm_request.0.jsonl")).unwrap();
        assert!(!contents.contains("SYNTHETIC_SECRET_SENTINEL"));
        assert!(!contents.contains("echoed tool arguments"));
        assert!(contents.contains("\"error_redacted\":true"));
    }

    #[test]
    fn private_error_logging_never_formats_display() {
        struct NeverFormat;
        impl Display for NeverFormat {
            fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                panic!("private upstream errors must not be formatted");
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let mut log = test_log(dir.path(), PayloadPolicy::MetadataOnly, json!({}));
        log.error(NeverFormat).unwrap();
        log.finish().unwrap();
    }

    #[test]
    fn private_error_logging_preserves_only_trusted_provider_class() {
        let error = ProviderError::RequestFailed("429 SYNTHETIC_SECRET_SENTINEL".into());
        for policy in [PayloadPolicy::MetadataOnly, PayloadPolicy::Full] {
            let dir = tempfile::tempdir().unwrap();
            let mut log = test_log(dir.path(), policy, json!({}));
            log.provider_error(&error).unwrap();
            log.finish().unwrap();
            let contents = std::fs::read_to_string(dir.path().join("llm_request.0.jsonl")).unwrap();
            if policy.is_metadata_only() {
                assert!(!contents.contains("SYNTHETIC_SECRET_SENTINEL"));
                assert!(!contents.contains("429"));
                assert!(contents.contains("\"error_type\":\"request\""));
                assert!(contents.contains("\"error_redacted\":true"));
            } else {
                assert!(contents.contains("429 SYNTHETIC_SECRET_SENTINEL"));
                assert!(!contents.contains("error_redacted"));
            }
        }
    }

    #[derive(Clone, Default)]
    struct ErrorLogCapture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for ErrorLogCapture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn error_log_subscriber(capture: ErrorLogCapture) -> impl tracing::Subscriber + Send + Sync {
        tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(move || capture.clone())
            .finish()
    }

    #[test]
    fn private_error_logging_http_trace_excludes_response_payload() {
        let capture = ErrorLogCapture::default();
        let subscriber = error_log_subscriber(capture.clone());
        let error = tracing::subscriber::with_default(subscriber, || {
            map_http_error_to_provider_error(
                StatusCode::BAD_REQUEST,
                Some(json!({"error":{"message":"SYNTHETIC_SECRET_SENTINEL"}})),
            )
        });

        assert!(error.to_string().contains("SYNTHETIC_SECRET_SENTINEL"));
        let trace = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(
            trace.contains("400"),
            "the control warning must be captured"
        );
        assert!(!trace.contains("SYNTHETIC_SECRET_SENTINEL"));
    }

    #[tokio::test]
    async fn private_error_logging_retry_traces_exclude_upstream_echoes() {
        use crate::providers::retry::{retry_operation, ProviderRetry, RetryConfig};
        use tracing::instrument::WithSubscriber;

        struct RetryOnce;
        impl ProviderRetry for RetryOnce {
            fn retry_config(&self) -> RetryConfig {
                RetryConfig::new(1, 0, 1.0, 0)
            }
        }

        let capture = ErrorLogCapture::default();
        let subscriber = error_log_subscriber(capture.clone());
        let attempts = std::sync::atomic::AtomicUsize::new(0);
        let operation = || {
            attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async {
                Err::<(), _>(ProviderError::ServerError(
                    "SYNTHETIC_SECRET_SENTINEL".into(),
                ))
            }
        };
        async {
            let error = retry_operation(&RetryConfig::new(1, 0, 1.0, 0), operation)
                .await
                .unwrap_err();
            assert!(matches!(error, ProviderError::ServerError(text) if text == "SYNTHETIC_SECRET_SENTINEL"));
            assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
            let error = RetryOnce.with_retry(operation).await.unwrap_err();
            assert!(matches!(error, ProviderError::ServerError(text) if text == "SYNTHETIC_SECRET_SENTINEL"));
            assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 4);
        }
        .with_subscriber(subscriber)
        .await;

        let trace = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert_eq!(trace.matches("Request failed").count(), 2);
        assert!(!trace.contains("SYNTHETIC_SECRET_SENTINEL"));
    }

    #[tokio::test]
    async fn private_error_logging_google_traces_exclude_response_payload() {
        use tracing::instrument::WithSubscriber;

        let capture = ErrorLogCapture::default();
        let subscriber = error_log_subscriber(capture.clone());
        async {
            for status in [400, 422] {
                let response = axum::http::Response::builder()
                    .status(status)
                    .header("content-type", "application/json")
                    .body(json!({"error":{"message":"SYNTHETIC_SECRET_SENTINEL"}}).to_string())
                    .unwrap();
                handle_response_google_compat(reqwest::Response::from(response))
                    .await
                    .unwrap_err();
            }
        }
        .with_subscriber(subscriber)
        .await;

        let trace = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(trace.contains("400"));
        assert!(trace.contains("422"));
        assert!(!trace.contains("SYNTHETIC_SECRET_SENTINEL"));
    }

    #[test]
    fn payload_policy_follows_the_provider_tier() {
        assert_eq!(
            PayloadPolicy::for_tier(ProviderTier::Private),
            PayloadPolicy::MetadataOnly
        );
        assert_eq!(
            PayloadPolicy::for_tier(ProviderTier::Public),
            PayloadPolicy::Full
        );
    }

    /// A private-tier exchange leaves model, timings, tokens and a redacted error
    /// on disk and **no transcript** — asserted on the bytes that reach the
    /// file, not on the policy value that produced them.
    #[test]
    fn a_private_tier_log_writes_no_prompt_and_no_completion() {
        let dir = tempfile::tempdir().unwrap();

        let model = ModelConfig::new("gpt-4o").unwrap();
        let prompt = json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "patient MRN 8675309 has glioblastoma"}],
        });

        let mut log =
            RequestLog::start_in(dir.path(), ProviderTier::Private, &model, &prompt).unwrap();
        assert_eq!(log.policy(), PayloadPolicy::MetadataOnly);
        log.write(&json!({"delta": "the biopsy shows"}), None)
            .unwrap();
        log.write(&json!({"delta": " an IDH-wildtype tumour"}), None)
            .unwrap();
        log.error("upstream 429").unwrap();
        log.finish().unwrap();

        let contents = std::fs::read_to_string(dir.path().join("llm_request.0.jsonl")).unwrap();

        // Not one word of the exchange survived.
        assert!(!contents.contains("8675309"), "prompt leaked: {contents}");
        assert!(
            !contents.contains("glioblastoma"),
            "prompt leaked: {contents}"
        );
        assert!(
            !contents.contains("biopsy"),
            "completion leaked: {contents}"
        );
        assert!(
            !contents.contains("IDH-wildtype"),
            "completion leaked: {contents}"
        );
        assert!(
            !contents.contains("\"input\""),
            "input key present: {contents}"
        );
        assert!(
            !contents.contains("\"data\""),
            "data key present: {contents}"
        );

        // The metadata the fix promises to keep is all there.
        assert!(contents.contains("\"provider_tier\":\"private\""));
        assert!(contents.contains("\"payloads\":\"metadata-only\""));
        assert!(contents.contains("gpt-4o"), "model name is metadata");
        assert!(contents.contains("\"elapsed_ms\""));
        assert!(contents.contains("\"redacted_response_chunks\":2"));
        assert!(!contents.contains("upstream 429"));
        assert!(contents.contains("\"error_redacted\":true"));
    }

    /// The other half of the same gate: Public keeps the debug log useful.
    #[test]
    fn a_public_tier_log_still_writes_the_exchange() {
        let dir = tempfile::tempdir().unwrap();

        let model = ModelConfig::new("gpt-4o").unwrap();
        let prompt = json!({"messages": [{"role": "user", "content": "hello there"}]});

        let mut log =
            RequestLog::start_in(dir.path(), ProviderTier::Public, &model, &prompt).unwrap();
        assert_eq!(log.policy(), PayloadPolicy::Full);
        log.write(&json!({"delta": "general kenobi"}), None)
            .unwrap();
        log.finish().unwrap();

        let contents = std::fs::read_to_string(dir.path().join("llm_request.0.jsonl")).unwrap();
        assert!(contents.contains("hello there"));
        assert!(contents.contains("general kenobi"));
        assert!(contents.contains("\"provider_tier\":\"public\""));
    }

    /// `start` is what all ~25 provider call sites reach, and none of them pass
    /// a tier. It must resolve to the safe end of the lattice.
    ///
    /// ⚠ The lines are TAKEN, not flushed: `start` resolves the ambient state
    /// dir, which this test does not own, and every other test in the binary
    /// can write to. They are the exact strings a flush writes (plus a
    /// timing trailer), so asserting on them is asserting on the file's bytes
    /// without reading a file anyone else can rotate.
    #[test]
    fn the_tierless_constructor_every_provider_uses_is_metadata_only() {
        let model = ModelConfig::new("gpt-4o").unwrap();
        let mut log = RequestLog::start(&model, &json!({"secret": "do not ship me"})).unwrap();
        assert_eq!(log.policy(), PayloadPolicy::MetadataOnly);
        let contents = log.lines.take().expect("an open log").join("\n");
        assert!(contents.contains("gpt-4o"), "not the header: {contents}");
        assert!(!contents.contains("do not ship me"), "leaked: {contents}");
    }

    /// The diagnostics filter reads this header field. If it stops being
    /// written, every log becomes unattributable and the bundle ships none of
    /// them — a silent loss, so pin it here as well as at the reader.
    #[tokio::test]
    async fn every_log_header_records_the_session_it_belongs_to() {
        let model = ModelConfig::new("gpt-4o").unwrap();

        // Taken, not flushed — see the tierless test above.
        let header = crate::session_context::with_session_id(Some("sess-42".into()), async {
            let mut log = RequestLog::start(&model, &json!({"m": 1})).unwrap();
            log.lines.take().unwrap()[0].clone()
        })
        .await;

        assert!(header.contains("\"session_id\":\"sess-42\""), "{header}");
    }

    /// Outside a scoped session the header still carries the key, with `null` —
    /// which the diagnostics filter reads as "cannot attribute, do not ship".
    #[test]
    fn a_log_opened_outside_a_session_records_a_null_session() {
        let model = ModelConfig::new("gpt-4o").unwrap();
        // Taken, not flushed — see the tierless test above.
        let mut log = RequestLog::start(&model, &json!({"m": 1})).unwrap();
        assert!(log.lines.take().unwrap()[0].contains("\"session_id\":null"));
    }

    /// `start` writes where the diagnostics bundle reads. The tests above pass
    /// their own directory to `start_in`, so this is the one place that still
    /// pins the production pairing — without it, either side could move and
    /// every bundle would silently ship no request logs.
    ///
    /// Paths only, no file: the env lock is held at the sandbox root so both
    /// resolutions see the same one, and the lines are taken so nothing is
    /// flushed into a directory the whole binary shares.
    #[test]
    fn a_request_log_lands_where_the_diagnostics_bundle_reads() {
        let _root = crate::test_sandbox::pin_sandbox_path_root();
        let model = ModelConfig::new("gpt-4o").unwrap();
        let mut log = RequestLog::start(&model, &json!({"m": 1})).unwrap();
        log.lines.take();
        assert_eq!(
            log.temp_path.parent(),
            Some(crate::session::DiagnosticsSources::resolve().logs_dir()),
            "RequestLog writes {} but the diagnostics bundle sweeps {}",
            log.temp_path.display(),
            crate::session::DiagnosticsSources::resolve()
                .logs_dir()
                .display()
        );
    }

    #[test]
    fn test_flush_request_log_rotates_numbered_logs() {
        let dir = tempfile::tempdir().unwrap();
        let logs_dir = dir.path();
        // A pre-existing newest log must shift to slot 1 when a new one lands.
        std::fs::write(logs_dir.join("llm_request.0.jsonl"), "old\n").unwrap();

        let temp_path = logs_dir.join("llm_request.new.jsonl");
        flush_request_log(vec!["fresh".to_string()], temp_path).unwrap();

        assert_eq!(
            std::fs::read_to_string(logs_dir.join("llm_request.0.jsonl")).unwrap(),
            "fresh\n"
        );
        assert_eq!(
            std::fs::read_to_string(logs_dir.join("llm_request.1.jsonl")).unwrap(),
            "old\n"
        );
    }

    #[test]
    fn test_detect_image_path() {
        // Create a temporary PNG file with valid PNG magic numbers
        let temp_dir = tempfile::tempdir().unwrap();
        let png_path = temp_dir.path().join("test.png");
        let png_data = [
            0x89, 0x50, 0x4E, 0x47, // PNG magic number
            0x0D, 0x0A, 0x1A, 0x0A, // PNG header
            0x00, 0x00, 0x00, 0x0D, // Rest of fake PNG data
        ];
        std::fs::write(&png_path, png_data).unwrap();
        let png_path_str = png_path.to_str().unwrap();

        // Create a fake PNG (wrong magic numbers)
        let fake_png_path = temp_dir.path().join("fake.png");
        std::fs::write(&fake_png_path, b"not a real png").unwrap();

        // Test with valid PNG file using absolute path
        let text = format!("Here is an image {}", png_path_str);
        assert_eq!(detect_image_path(&text), Some(png_path_str));

        // Test with non-image file that has .png extension
        let text = format!("Here is a fake image {}", fake_png_path.to_str().unwrap());
        assert_eq!(detect_image_path(&text), None);

        // Test with non-existent file
        let text = "Here is a fake.png that doesn't exist";
        assert_eq!(detect_image_path(text), None);

        // Test with non-image file
        let text = "Here is a file.txt";
        assert_eq!(detect_image_path(text), None);

        // Test with relative path (should not match)
        let text = "Here is a relative/path/image.png";
        assert_eq!(detect_image_path(text), None);
    }

    #[test]
    fn test_load_image_file() {
        // Create a temporary PNG file with valid PNG magic numbers
        let temp_dir = tempfile::tempdir().unwrap();
        let png_path = temp_dir.path().join("test.png");
        let png_data = [
            0x89, 0x50, 0x4E, 0x47, // PNG magic number
            0x0D, 0x0A, 0x1A, 0x0A, // PNG header
            0x00, 0x00, 0x00, 0x0D, // Rest of fake PNG data
        ];
        std::fs::write(&png_path, png_data).unwrap();
        let png_path_str = png_path.to_str().unwrap();

        // Create a fake PNG (wrong magic numbers)
        let fake_png_path = temp_dir.path().join("fake.png");
        std::fs::write(&fake_png_path, b"not a real png").unwrap();
        let fake_png_path_str = fake_png_path.to_str().unwrap();

        // Test loading valid PNG file
        let result = load_image_file(png_path_str);
        assert!(result.is_ok());
        let image = result.unwrap();
        assert_eq!(image.mime_type, "image/png");

        // Test loading fake PNG file
        let result = load_image_file(fake_png_path_str);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not a valid image"));

        // Test non-existent file
        let result = load_image_file("nonexistent.png");
        assert!(result.is_err());

        // Create a GIF file with valid header bytes
        let gif_path = temp_dir.path().join("test.gif");
        // Minimal GIF89a header
        let gif_data = [0x47, 0x49, 0x46, 0x38, 0x39, 0x61];
        std::fs::write(&gif_path, gif_data).unwrap();
        let gif_path_str = gif_path.to_str().unwrap();

        // Test loading unsupported GIF format
        let result = load_image_file(gif_path_str);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unsupported image format"));
    }

    #[test]
    fn test_sanitize_function_name() {
        assert_eq!(sanitize_function_name("hello-world"), "hello-world");
        assert_eq!(sanitize_function_name("hello world"), "hello_world");
        assert_eq!(sanitize_function_name("hello@world"), "hello_world");
    }

    #[test]
    fn test_is_valid_function_name() {
        assert!(is_valid_function_name("hello-world"));
        assert!(is_valid_function_name("hello_world"));
        assert!(!is_valid_function_name("hello world"));
        assert!(!is_valid_function_name("hello@world"));
    }

    #[test]
    fn unescape_json_values_with_object() {
        let value = json!({"text": "Hello\\nWorld"});
        let unescaped_value = unescape_json_values(&value);
        assert_eq!(unescaped_value, json!({"text": "Hello\nWorld"}));
    }

    #[test]
    fn unescape_json_values_with_array() {
        let value = json!(["Hello\\nWorld", "Goodbye\\tWorld"]);
        let unescaped_value = unescape_json_values(&value);
        assert_eq!(unescaped_value, json!(["Hello\nWorld", "Goodbye\tWorld"]));
    }

    #[test]
    fn unescape_json_values_with_string() {
        let value = json!("Hello\\nWorld");
        let unescaped_value = unescape_json_values(&value);
        assert_eq!(unescaped_value, json!("Hello\nWorld"));
    }

    #[test]
    fn unescape_json_values_with_mixed_content() {
        let value = json!({
            "text": "Hello\\nWorld\\\\n!",
            "array": ["Goodbye\\tWorld", "See you\\rlater"],
            "nested": {
                "inner_text": "Inner\\\"Quote\\\""
            }
        });
        let unescaped_value = unescape_json_values(&value);
        assert_eq!(
            unescaped_value,
            json!({
                "text": "Hello\nWorld\n!",
                "array": ["Goodbye\tWorld", "See you\rlater"],
                "nested": {
                    "inner_text": "Inner\"Quote\""
                }
            })
        );
    }

    #[test]
    fn unescape_json_values_with_no_escapes() {
        let value = json!({"text": "Hello World"});
        let unescaped_value = unescape_json_values(&value);
        assert_eq!(unescaped_value, json!({"text": "Hello World"}));
    }

    #[test]
    fn test_is_google_model() {
        // Define the test cases as a vector of tuples
        let test_cases = vec![
            // (input, expected_result)
            (json!({ "model": "google_gemini" }), true),
            (json!({ "model": "microsoft_bing" }), false),
            (json!({ "model": "" }), false),
            (json!({}), false),
            (json!({ "model": "Google_XYZ" }), true),
            (json!({ "model": "google_abc" }), true),
        ];

        // Iterate through each test case and assert the result
        for (payload, expected_result) in test_cases {
            assert_eq!(is_google_model(&payload), expected_result);
        }
    }

    #[test]
    fn test_get_google_final_status_success() {
        let status = StatusCode::OK;
        let payload = json!({});
        let result = get_google_final_status(status, Some(&payload));
        assert_eq!(result, StatusCode::OK);
    }

    #[test]
    fn test_get_google_final_status_with_error_code() {
        // Test error code mappings for different payload error codes
        let test_cases = vec![
            // (error code, status, expected status code)
            (200, None, StatusCode::OK),
            (429, Some(StatusCode::OK), StatusCode::TOO_MANY_REQUESTS),
            (400, Some(StatusCode::OK), StatusCode::BAD_REQUEST),
            (401, Some(StatusCode::OK), StatusCode::UNAUTHORIZED),
            (403, Some(StatusCode::OK), StatusCode::FORBIDDEN),
            (404, Some(StatusCode::OK), StatusCode::NOT_FOUND),
            (500, Some(StatusCode::OK), StatusCode::INTERNAL_SERVER_ERROR),
            (503, Some(StatusCode::OK), StatusCode::SERVICE_UNAVAILABLE),
            (999, Some(StatusCode::OK), StatusCode::INTERNAL_SERVER_ERROR),
            (500, Some(StatusCode::BAD_REQUEST), StatusCode::BAD_REQUEST),
            (
                404,
                Some(StatusCode::INTERNAL_SERVER_ERROR),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];

        for (error_code, status, expected_status) in test_cases {
            let payload = if let Some(_status) = status {
                json!({
                    "error": {
                        "code": error_code,
                        "message": "Error message"
                    }
                })
            } else {
                json!({})
            };

            let result = get_google_final_status(status.unwrap_or(StatusCode::OK), Some(&payload));
            assert_eq!(result, expected_status);
        }
    }

    #[test]
    fn test_safely_parse_json() {
        // Test valid JSON that should parse without escaping (contains proper escape sequence)
        let valid_json = r#"{"key1": "value1","key2": "value2"}"#;
        let result = safely_parse_json(valid_json).unwrap();
        assert_eq!(result["key1"], "value1");
        assert_eq!(result["key2"], "value2");

        // Test JSON with actual unescaped newlines that needs escaping
        let invalid_json = "{\"key1\": \"value1\n\",\"key2\": \"value2\"}";
        let result = safely_parse_json(invalid_json).unwrap();
        assert_eq!(result["key1"], "value1\n");
        assert_eq!(result["key2"], "value2");

        // Test already valid JSON - should parse on first try
        let good_json = r#"{"test": "value"}"#;
        let result = safely_parse_json(good_json).unwrap();
        assert_eq!(result["test"], "value");

        // Test completely invalid JSON that can't be fixed
        let broken_json = r#"{"key": "unclosed_string"#;
        assert!(safely_parse_json(broken_json).is_err());

        // Test empty object
        let empty_json = "{}";
        let result = safely_parse_json(empty_json).unwrap();
        assert!(result.as_object().unwrap().is_empty());

        // Test JSON with escaped newlines (valid JSON) - should parse on first try
        let escaped_json = r#"{"key": "value with\nnewline"}"#;
        let result = safely_parse_json(escaped_json).unwrap();
        assert_eq!(result["key"], "value with\nnewline");
    }

    #[test]
    fn test_json_escape_control_chars_in_string() {
        // Test basic control character escaping
        assert_eq!(
            json_escape_control_chars_in_string("Hello\nWorld"),
            "Hello\\nWorld"
        );
        assert_eq!(
            json_escape_control_chars_in_string("Hello\tWorld"),
            "Hello\\tWorld"
        );
        assert_eq!(
            json_escape_control_chars_in_string("Hello\rWorld"),
            "Hello\\rWorld"
        );

        // Test multiple control characters
        assert_eq!(
            json_escape_control_chars_in_string("Hello\n\tWorld\r"),
            "Hello\\n\\tWorld\\r"
        );

        // Test that quotes and backslashes are preserved (not escaped)
        assert_eq!(
            json_escape_control_chars_in_string("Hello \"World\""),
            "Hello \"World\""
        );
        assert_eq!(
            json_escape_control_chars_in_string("Hello\\World"),
            "Hello\\World"
        );

        // Test JSON-like string with control characters
        assert_eq!(
            json_escape_control_chars_in_string("{\"message\": \"Hello\nWorld\"}"),
            "{\"message\": \"Hello\\nWorld\"}"
        );

        // Test no changes for normal strings
        assert_eq!(
            json_escape_control_chars_in_string("Hello World"),
            "Hello World"
        );

        // Test other control characters get unicode escapes
        assert_eq!(
            json_escape_control_chars_in_string("Hello\u{0001}World"),
            "Hello\\u0001World"
        );
    }

    #[test]
    fn test_parse_google_retry_delay() {
        let payload = json!({
            "error": {
                "details": [
                    {
                        "@type": "type.googleapis.com/google.rpc.RetryInfo",
                        "retryDelay": "42s"
                    }
                ]
            }
        });
        assert_eq!(
            parse_google_retry_delay(&payload),
            Some(Duration::from_secs(42))
        );
    }
}
