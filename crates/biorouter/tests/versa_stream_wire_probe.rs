#![cfg(feature = "aws-providers")]

//! Manual, metadata-only probe. Never executes a returned tool.
//!
//! Requires both `--ignored` and `BIOROUTER_RUN_VERSA_STREAM_PROBE=1`.
//! Set `BIOROUTER_VERSA_STREAM_PROBE_OUTPUT` to a NEW file directly under `/tmp`.
//! Optional: `BIOROUTER_VERSA_STREAM_PROBE_MODEL` (a Claude Bedrock model id),
//! `BIOROUTER_VERSA_STREAM_PROBE_CASE=todo|python_sqlite|python_sqlite_short`.
//! `BIOROUTER_VERSA_STREAM_PROBE_TRANSPORT=auto|http1` changes only HTTP transport.
//! The normal profile supplies credentials and `BIOROUTER_MAX_TOKENS`; this probe
//! does not override that budget. Only the synthetic prompts below can leave
//! the process. Response bodies and signed headers stay in memory.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use aws_sdk_bedrockruntime::config::Credentials;
use aws_sdk_bedrockruntime::{types as bedrock, Client};
use aws_smithy_http_client::test_util::{capture_request, CaptureRequestReceiver};
use aws_smithy_runtime_api::http::Response as SmithyResponse;
use aws_smithy_types::body::SdkBody;
use biorouter::config::Config;
use biorouter::model::ModelConfig;
use biorouter::providers::formats::bedrock::{bedrock_inference_config, to_bedrock_tool_config};
use biorouter::providers::versa_bedrock::{
    VERSA_BEDROCK_DEFAULT_ENDPOINT, VERSA_BEDROCK_DEFAULT_MODEL, VERSA_BEDROCK_DEFAULT_REGION,
};
use futures::StreamExt;
use rmcp::model::Tool;
use serde_json::{json, Value};

const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_ERROR_BODY_BYTES: usize = 16 * 1024;
const MAX_FRAME_BYTES: usize = 1024 * 1024;
const MAX_FRAMES: usize = 32_768;
const ASYNC_DEADLINE: Duration = Duration::from_secs(90);
const HARD_DEADLINE: Duration = Duration::from_secs(95);

type ProbeResult<T> = Result<T, &'static str>;

#[derive(Clone, Copy)]
enum ProbeCase {
    Todo,
    PythonSqlite,
    PythonSqliteShort,
}

impl ProbeCase {
    fn load() -> ProbeResult<Self> {
        match std::env::var("BIOROUTER_VERSA_STREAM_PROBE_CASE")
            .unwrap_or_else(|_| "todo".into())
            .as_str()
        {
            "todo" => Ok(Self::Todo),
            "python_sqlite" => Ok(Self::PythonSqlite),
            "python_sqlite_short" => Ok(Self::PythonSqliteShort),
            _ => Err("unsupported_probe_case"),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::PythonSqlite => "python_sqlite",
            Self::PythonSqliteShort => "python_sqlite_short",
        }
    }

    fn prompt(self) -> &'static str {
        match self {
            Self::Todo => {
                "This is a synthetic protocol test. First write a short two-sentence plan for \
                 checking a fictional inventory. Then call todo__todo_write exactly once with \
                 a Markdown checklist containing five specific inventory checks. Do not request \
                 any other tool, access real data, or include personal information."
            }
            Self::PythonSqlite => {
                "This is a synthetic protocol test, not permission to execute code. First write \
                 a short two-sentence plan. Then call developer__text_editor exactly once with \
                 command=create, path=/tmp/versa_probe_synthetic_inventory.py, and file_text \
                 containing a complete, meaningful 180-to-260-line Python 3 program. Use only \
                 the standard library. Implement a SQLite inventory CLI with items and adjustments \
                 tables, foreign keys, indexes, transactional add/adjust operations, argparse \
                 subcommands init/add/list/adjust/audit, parameterized SQL, input validation, \
                 helpful errors, and a --self-test mode with deterministic unittest cases using \
                 an in-memory database. Include the full source, not placeholders or repeated \
                 filler. Do not read any real files or data. No tool will actually be executed."
            }
            Self::PythonSqliteShort => {
                "This is a synthetic protocol test, not permission to execute code. First write \
                 a short two-sentence plan. Then call developer__text_editor exactly once with \
                 command=create, path=/tmp/versa_probe_synthetic_inventory.py, and file_text \
                 containing a complete, meaningful 10-to-15-line Python 3 program. Use only \
                 the standard library. Open an in-memory SQLite database, create an inventory \
                 table, insert two fictional items using parameterized SQL, print the items \
                 in deterministic order, and close the database. Include the full source, not \
                 placeholders or repeated filler. Do not read any real files or data. No tool \
                 will actually be executed."
            }
        }
    }

    fn tool(self) -> Tool {
        let (name, description, schema) = match self {
            Self::Todo => (
                "todo__todo_write",
                "Record a synthetic Markdown checklist. This diagnostic tool is never executed.",
                json!({"type":"object","properties":{"content":{"type":"string"}},
                    "required":["content"],"additionalProperties":false}),
            ),
            Self::PythonSqlite | Self::PythonSqliteShort => (
                "developer__text_editor",
                "Submit a synthetic file creation. This diagnostic tool is never executed.",
                json!({"type":"object","properties":{
                    "command":{"type":"string","enum":["create"]},
                    "path":{"type":"string"},"file_text":{"type":"string"}},
                    "required":["command","path","file_text"],"additionalProperties":false}),
            ),
        };
        Tool::new(name, description, schema.as_object().unwrap().clone())
    }
}

struct Settings {
    endpoint: String,
    region: String,
    model: ModelConfig,
    case: ProbeCase,
    http1_only: bool,
}

fn transport_http1_only(mode: &str) -> ProbeResult<bool> {
    match mode {
        "auto" => Ok(false),
        "http1" => Ok(true),
        _ => Err("unsupported_probe_transport"),
    }
}

impl Settings {
    fn load() -> ProbeResult<Self> {
        let config = Config::global();
        // Versa's own keys only, as `VersaBedrockProvider::from_env` reads them:
        // the `AWS_*` ones belong to the public Amazon Bedrock card.
        let endpoint = configured_string(config, "VERSA_BEDROCK_ENDPOINT")
            .unwrap_or_else(|| VERSA_BEDROCK_DEFAULT_ENDPOINT.into());
        validate_endpoint(&endpoint)?;
        let region = configured_string(config, "VERSA_BEDROCK_REGION")
            .unwrap_or_else(|| VERSA_BEDROCK_DEFAULT_REGION.into());
        if region.len() > 32
            || !region
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err("invalid_region");
        }
        let model = nonempty_env("BIOROUTER_VERSA_STREAM_PROBE_MODEL")
            .unwrap_or_else(|| VERSA_BEDROCK_DEFAULT_MODEL.into());
        if !model.starts_with("us.anthropic.claude-")
            || model.len() > 128
            || !model
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-.:".contains(&byte))
        {
            return Err("invalid_claude_model_id");
        }
        Ok(Self {
            endpoint,
            region,
            model: ModelConfig::new(&model).map_err(|_| "invalid_model_configuration")?,
            case: ProbeCase::load()?,
            http1_only: transport_http1_only(
                &std::env::var("BIOROUTER_VERSA_STREAM_PROBE_TRANSPORT")
                    .unwrap_or_else(|_| "auto".into()),
            )?,
        })
    }
}

fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn configured_string(config: &Config, name: &str) -> Option<String> {
    config
        .get_param::<String>(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| nonempty_env(name))
}

fn validate_endpoint(endpoint: &str) -> ProbeResult<reqwest::Url> {
    let url = reqwest::Url::parse(endpoint).map_err(|_| "invalid_endpoint_url")?;
    if url.scheme() != "https"
        || url.host_str() != Some("unified-api.ucsf.edu")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path().trim_end_matches('/') != "/general/awsai"
    {
        return Err("endpoint_is_not_the_approved_ucsf_bedrock_gateway");
    }
    Ok(url)
}

fn reserve_output() -> ProbeResult<(PathBuf, File)> {
    let path = PathBuf::from(
        std::env::var("BIOROUTER_VERSA_STREAM_PROBE_OUTPUT").map_err(|_| "output_path_required")?,
    );
    if path.parent() != Some(Path::new("/tmp")) || path.file_name().is_none() {
        return Err("output_must_be_a_new_file_directly_under_tmp");
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&path)
        .map_err(|_| "output_create_new_failed")?;
    Ok((path, file))
}

fn save_report(file: &mut File, report: &Value) -> ProbeResult<()> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "output_seek_failed")?;
    file.set_len(0).map_err(|_| "output_truncate_failed")?;
    serde_json::to_writer_pretty(&mut *file, report).map_err(|_| "output_write_failed")?;
    file.write_all(b"\n").map_err(|_| "output_write_failed")?;
    file.sync_all().map_err(|_| "output_sync_failed")
}

#[derive(Default)]
struct Block {
    started: bool,
    closed: bool,
    starts: usize,
    stops: usize,
    input: String,
    tool: Option<&'static str>,
}

#[derive(Default)]
struct Audit {
    counts: BTreeMap<&'static str, usize>,
    blocks: BTreeMap<i64, Block>,
    unindexed_events: usize,
    stop_reason: Option<&'static str>,
    usage: Value,
    events: Vec<Value>,
}

impl Audit {
    fn record(&mut self, tag: &'static str, payload: &Value, mut event: Value) {
        *self.counts.entry(tag).or_default() += 1;
        let raw_index = payload.get("contentBlockIndex");
        let index = raw_index.and_then(Value::as_i64);
        let index_state = match raw_index {
            None => "missing",
            Some(Value::Null) => "null",
            Some(_) if index.is_some() => "integer",
            Some(_) => "invalid_type",
        };
        event["tag"] = json!(tag);
        if tag.starts_with("contentBlock") {
            event["index_state"] = json!(index_state);
            event["index"] = json!(index);
            if index.is_none() {
                self.unindexed_events += 1;
            }
        }
        match tag {
            "contentBlockStart" => {
                if let Some(tool) = payload
                    .pointer("/start/toolUse")
                    .filter(|tool| tool.is_object())
                {
                    event["tool"] = json!(safe_tool(tool.get("name").and_then(Value::as_str)));
                    if let Some(index) = index {
                        let block = self.blocks.entry(index).or_default();
                        block.started = true;
                        block.starts += 1;
                        block.tool = Some(safe_tool(tool.get("name").and_then(Value::as_str)));
                    }
                }
            }
            "contentBlockDelta" => {
                if let Some(input) = payload
                    .pointer("/delta/toolUse/input")
                    .and_then(Value::as_str)
                {
                    event["input_delta_bytes"] = json!(input.len());
                    if let Some(index) = index {
                        self.blocks.entry(index).or_default().input.push_str(input);
                    }
                }
                event["delta_kind"] = json!(if payload.pointer("/delta/toolUse").is_some() {
                    "toolUse"
                } else if payload.pointer("/delta/text").is_some() {
                    "text"
                } else if payload.pointer("/delta/reasoningContent").is_some() {
                    "reasoningContent"
                } else {
                    "unknown"
                });
            }
            "contentBlockStop" => {
                if let Some(block) = index.and_then(|index| self.blocks.get_mut(&index)) {
                    block.closed = true;
                    block.stops += 1;
                }
            }
            "messageStop" => {
                self.stop_reason = Some(safe_stop_reason(
                    payload.get("stopReason").and_then(Value::as_str),
                ));
                event["stop_reason"] = json!(self.stop_reason);
            }
            "metadata" => {
                self.usage = json!({
                    "input_tokens": payload.pointer("/usage/inputTokens").and_then(Value::as_u64),
                    "output_tokens": payload.pointer("/usage/outputTokens").and_then(Value::as_u64),
                    "total_tokens": payload.pointer("/usage/totalTokens").and_then(Value::as_u64),
                    "latency_ms": payload.pointer("/metrics/latencyMs").and_then(Value::as_u64)
                });
            }
            _ => {}
        }
        self.events.push(event);
    }

    fn summary(&self) -> Value {
        let blocks = self
            .blocks
            .iter()
            .map(|(index, block)| {
                let parsed = serde_json::from_str::<Value>(&block.input);
                let json_status = match &parsed {
                    Ok(Value::Object(_)) => "valid_object",
                    Ok(_) => "valid_non_object",
                    Err(_) if block.input.is_empty() => "empty",
                    Err(error) if error.is_eof() => "incomplete_json",
                    Err(_) => "invalid_json",
                };
                json!({"index":index,"tool":block.tool,"started":block.started,
                "closed":block.closed,"start_count":block.starts,"stop_count":block.stops,
                "input_bytes":block.input.len(),"input_json_status":json_status})
            })
            .collect::<Vec<_>>();
        json!({"event_counts":self.counts,"events":self.events,"blocks":blocks,
            "unindexed_events":self.unindexed_events,"message_stop_reason":self.stop_reason,
            "usage":self.usage})
    }
}

fn safe_tool(name: Option<&str>) -> &'static str {
    match name {
        Some("todo__todo_write") => "todo__todo_write",
        Some("developer__text_editor") => "developer__text_editor",
        _ => "unexpected_tool",
    }
}

fn safe_stop_reason(reason: Option<&str>) -> &'static str {
    match reason {
        Some("end_turn") => "end_turn",
        Some("tool_use") => "tool_use",
        Some("max_tokens") => "max_tokens",
        Some("stop_sequence") => "stop_sequence",
        Some("guardrail_intervened") => "guardrail_intervened",
        Some("content_filtered") => "content_filtered",
        Some("model_context_window_exceeded") => "model_context_window_exceeded",
        Some("malformed_model_output") => "malformed_model_output",
        _ => "unknown",
    }
}

fn safe_event_tag(tag: &str) -> &'static str {
    match tag {
        "messageStart" => "messageStart",
        "contentBlockStart" => "contentBlockStart",
        "contentBlockDelta" => "contentBlockDelta",
        "contentBlockStop" => "contentBlockStop",
        "messageStop" => "messageStop",
        "metadata" => "metadata",
        "internalServerException" => "internalServerException",
        "modelStreamErrorException" => "modelStreamErrorException",
        "modelTimeoutException" => "modelTimeoutException",
        "validationException" => "validationException",
        "throttlingException" => "throttlingException",
        "serviceUnavailableException" => "serviceUnavailableException",
        _ => "unknown",
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0_u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

fn be_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes[..4].try_into().unwrap())
}

fn control_headers(mut bytes: &[u8]) -> ProbeResult<(&'static str, &'static str)> {
    let mut message_type = "missing";
    let mut event_type = "unknown";
    while !bytes.is_empty() {
        let name_len = usize::from(bytes[0]);
        if name_len == 0 || bytes.len() < name_len + 2 {
            return Err("invalid_event_header");
        }
        let name = &bytes[1..1 + name_len];
        let kind = bytes[name_len + 1];
        bytes = &bytes[name_len + 2..];
        let (prefix, len) = match kind {
            0 | 1 => (0, 0),
            2 => (0, 1),
            3 => (0, 2),
            4 => (0, 4),
            5 | 8 => (0, 8),
            6 | 7 if bytes.len() >= 2 => (2, usize::from(u16::from_be_bytes([bytes[0], bytes[1]]))),
            9 => (0, 16),
            _ => return Err("invalid_event_header_type"),
        };
        if bytes.len() < prefix + len {
            return Err("truncated_event_header");
        }
        if kind == 7 {
            let value = std::str::from_utf8(&bytes[prefix..prefix + len])
                .map_err(|_| "invalid_event_header_encoding")?;
            match name {
                b":message-type" => {
                    message_type = match value {
                        "event" => "event",
                        "exception" => "exception",
                        "error" => "error",
                        _ => "unknown",
                    };
                }
                b":event-type" | b":exception-type" => event_type = safe_event_tag(value),
                _ => {}
            }
        }
        bytes = &bytes[prefix + len..];
    }
    Ok((message_type, event_type))
}

#[derive(Default)]
struct ProbeState {
    body: Vec<u8>,
    parsed_bytes: usize,
    audit: Audit,
    http: Value,
    raw_end: &'static str,
    sdk: Value,
}

impl ProbeState {
    fn parse_available(&mut self) -> ProbeResult<()> {
        loop {
            let remaining = &self.body[self.parsed_bytes..];
            if remaining.len() < 12 {
                return Ok(());
            }
            let total = be_u32(remaining) as usize;
            let headers = be_u32(&remaining[4..]) as usize;
            if !(16..=MAX_FRAME_BYTES).contains(&total) || headers > total - 16 {
                return Err("invalid_frame_length");
            }
            if crc32(&remaining[..8]) != be_u32(&remaining[8..]) {
                return Err("prelude_crc_mismatch");
            }
            if remaining.len() < total {
                return Ok(());
            }
            if crc32(&remaining[..total - 4]) != be_u32(&remaining[total - 4..]) {
                return Err("message_crc_mismatch");
            }
            if self.audit.events.len() >= MAX_FRAMES {
                return Err("frame_count_cap");
            }
            let (message_type, tag) = control_headers(&remaining[12..12 + headers])?;
            let payload: Value = serde_json::from_slice(&remaining[12 + headers..total - 4])
                .map_err(|_| "invalid_event_payload_json")?;
            self.audit.record(
                tag,
                &payload,
                json!({"offset":self.parsed_bytes,
                "frame_bytes":total,"header_bytes":headers,"message_type":message_type,
                "prelude_crc_valid":true,"message_crc_valid":true}),
            );
            self.parsed_bytes += total;
        }
    }

    fn summary(&self) -> Value {
        let remaining = &self.body[self.parsed_bytes..];
        let trailing_frame = if remaining.len() >= 12 {
            json!({"offset":self.parsed_bytes,"available_bytes":remaining.len(),
                "declared_frame_bytes":be_u32(remaining),
                "declared_header_bytes":be_u32(&remaining[4..]),
                "prelude_crc_valid":crc32(&remaining[..8]) == be_u32(&remaining[8..])})
        } else if !remaining.is_empty() {
            json!({"offset":self.parsed_bytes,"available_bytes":remaining.len(),
                "prelude_complete":false})
        } else {
            Value::Null
        };
        json!({"http":self.http,"raw_end":self.raw_end,"response_bytes":self.body.len(),
            "parsed_bytes":self.parsed_bytes,"trailing_bytes":self.body.len()-self.parsed_bytes,
            "trailing_frame":trailing_frame,
            "raw":self.audit.summary(),"sdk_decode_of_same_bytes":self.sdk})
    }
}

async fn captured_client(
    settings: &Settings,
    credentials: Credentials,
    response: Option<SmithyResponse<SdkBody>>,
) -> ProbeResult<(Client, CaptureRequestReceiver)> {
    let response = response
        .map(|response| {
            response
                .try_into_http1x()
                .map_err(|_| "invalid_replay_response")
        })
        .transpose()?;
    let (http, captured) = capture_request(response);
    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .credentials_provider(credentials)
        .region(aws_config::Region::new(settings.region.clone()))
        .endpoint_url(settings.endpoint.clone())
        .retry_config(aws_config::retry::RetryConfig::disabled())
        .http_client(http)
        .load()
        .await;
    Ok((Client::new(&config), captured))
}

fn request(
    client: &Client,
    settings: &Settings,
) -> ProbeResult<
    aws_sdk_bedrockruntime::operation::converse_stream::builders::ConverseStreamFluentBuilder,
> {
    let message = bedrock::Message::builder()
        .role(bedrock::ConversationRole::User)
        .content(bedrock::ContentBlock::Text(settings.case.prompt().into()))
        .build()
        .map_err(|_| "synthetic_message_build_failed")?;
    Ok(client
        .converse_stream()
        .model_id(settings.model.model_name.clone())
        .system(bedrock::SystemContentBlock::Text(
            "You are testing a tool-call transport using synthetic data. Follow the requested tool schema. No tool will be executed.".into(),
        ))
        .messages(message)
        .inference_config(bedrock_inference_config(&settings.model))
        .tool_config(to_bedrock_tool_config(&[settings.case.tool()]).map_err(|_| "synthetic_tool_build_failed")?))
}

fn sdk_error_class<E, R>(
    error: &aws_smithy_runtime_api::client::result::SdkError<E, R>,
) -> &'static str {
    use aws_smithy_runtime_api::client::result::SdkError;
    match error {
        SdkError::ConstructionFailure(_) => "construction_failure",
        SdkError::TimeoutError(_) => "timeout",
        SdkError::DispatchFailure(_) => "dispatch_failure",
        SdkError::ResponseError(_) => "response_decode_error",
        SdkError::ServiceError(_) => "service_error",
        _ => "unknown_sdk_error",
    }
}

async fn sdk_replay(
    settings: &Settings,
    credentials: Credentials,
    body: Vec<u8>,
) -> ProbeResult<Value> {
    let mut response = SmithyResponse::new(200.try_into().unwrap(), SdkBody::from(body));
    response
        .headers_mut()
        .insert("content-type", "application/vnd.amazon.eventstream");
    let (client, _captured) = captured_client(settings, credentials, Some(response)).await?;
    let mut response = request(&client, settings)?
        .send()
        .await
        .map_err(|error| sdk_error_class(&error))?;
    let mut audit = Audit::default();
    let end = loop {
        let event = match response.stream.recv().await {
            Ok(Some(event)) => event,
            Ok(None) => break "clean_eof",
            Err(error) => break sdk_error_class(&error),
        };
        if audit.events.len() >= MAX_FRAMES {
            break "frame_count_cap";
        }
        let (tag, payload) = match event {
            bedrock::ConverseStreamOutput::MessageStart(_) => ("messageStart", json!({})),
            bedrock::ConverseStreamOutput::ContentBlockStart(event) => {
                let tool = match event.start {
                    Some(bedrock::ContentBlockStart::ToolUse(tool)) => json!({"name":tool.name}),
                    _ => Value::Null,
                };
                (
                    "contentBlockStart",
                    json!({"contentBlockIndex":event.content_block_index,"start":{"toolUse":tool}}),
                )
            }
            bedrock::ConverseStreamOutput::ContentBlockDelta(event) => {
                let delta = match event.delta {
                    Some(bedrock::ContentBlockDelta::ToolUse(delta)) => {
                        json!({"toolUse":{"input":delta.input}})
                    }
                    Some(bedrock::ContentBlockDelta::Text(_)) => json!({"text":null}),
                    Some(bedrock::ContentBlockDelta::ReasoningContent(_)) => {
                        json!({"reasoningContent":null})
                    }
                    _ => json!({}),
                };
                (
                    "contentBlockDelta",
                    json!({"contentBlockIndex":event.content_block_index,"delta":delta}),
                )
            }
            bedrock::ConverseStreamOutput::ContentBlockStop(event) => (
                "contentBlockStop",
                json!({"contentBlockIndex":event.content_block_index}),
            ),
            bedrock::ConverseStreamOutput::MessageStop(event) => (
                "messageStop",
                json!({"stopReason":event.stop_reason.as_str()}),
            ),
            bedrock::ConverseStreamOutput::Metadata(event) => {
                let usage = event.usage.map(|usage| json!({"inputTokens":usage.input_tokens,"outputTokens":usage.output_tokens,"totalTokens":usage.total_tokens}));
                ("metadata", json!({"usage":usage}))
            }
            _ => ("unknown", json!({})),
        };
        audit.record(tag, &payload, json!({}));
    };
    Ok(json!({"end":end,"audit":audit.summary()}))
}

fn request_error_class(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "http_timeout"
    } else if error.is_connect() {
        "http_connect_error"
    } else if error.is_body() {
        "http_body_error"
    } else {
        "http_request_error"
    }
}

fn safe_error_code(value: &str) -> Option<&'static str> {
    let code = value.split(':').next()?.rsplit('#').next()?;
    match code {
        "InternalServerException" | "internalServerException" => Some("InternalServerException"),
        "InternalServerError" | "internal_server_error" => Some("InternalServerError"),
        "ModelStreamErrorException" | "modelStreamErrorException" => {
            Some("ModelStreamErrorException")
        }
        "ModelTimeoutException" | "modelTimeoutException" => Some("ModelTimeoutException"),
        "ModelErrorException" | "modelErrorException" => Some("ModelErrorException"),
        "ValidationException" | "validationException" => Some("ValidationException"),
        "ThrottlingException" | "throttlingException" => Some("ThrottlingException"),
        "ServiceUnavailableException" | "serviceUnavailableException" => {
            Some("ServiceUnavailableException")
        }
        "AccessDeniedException" | "accessDeniedException" => Some("AccessDeniedException"),
        "ResourceNotFoundException" | "resourceNotFoundException" => {
            Some("ResourceNotFoundException")
        }
        _ => None,
    }
}

fn safe_correlation_ids(headers: &reqwest::header::HeaderMap) -> Value {
    let mut ids = serde_json::Map::new();
    for name in [
        "x-amzn-requestid",
        "x-amz-request-id",
        "x-request-id",
        "x-correlation-id",
        "apim-request-id",
    ] {
        if let Some(header) = headers.get(name) {
            let id = header
                .to_str()
                .ok()
                .filter(|value| matches!(value.len(), 32 | 36))
                .and_then(|value| uuid::Uuid::parse_str(value).ok());
            ids.insert(
                name.into(),
                match id {
                    Some(id) => json!({"state":"uuid","value":id.hyphenated().to_string()}),
                    None => json!({"state":"present_unrecognized"}),
                },
            );
        }
    }
    Value::Object(ids)
}

fn project_http_error(headers: &reqwest::header::HeaderMap, body: &[u8], end: &str) -> Value {
    let capped = body.len() >= MAX_ERROR_BODY_BYTES;
    let body = &body[..body.len().min(MAX_ERROR_BODY_BYTES)];
    let parsed = serde_json::from_slice::<Value>(body).ok();
    let candidates = [
        headers
            .get("x-amzn-errortype")
            .and_then(|value| value.to_str().ok()),
        parsed
            .as_ref()
            .and_then(|value| value.get("__type"))
            .and_then(Value::as_str),
        parsed
            .as_ref()
            .and_then(|value| value.get("code"))
            .and_then(Value::as_str),
        parsed
            .as_ref()
            .and_then(|value| value.pointer("/error/code"))
            .and_then(Value::as_str),
    ];
    let code = candidates
        .iter()
        .flatten()
        .find_map(|value| safe_error_code(value));
    let original_status = parsed
        .as_ref()
        .and_then(|value| {
            value
                .get("originalStatusCode")
                .or_else(|| value.pointer("/error/originalStatusCode"))
        })
        .and_then(Value::as_u64)
        .filter(|status| (100..=599).contains(status));
    let body_end = if capped {
        "capped"
    } else if end == "complete" {
        "complete"
    } else {
        "body_read_error"
    };
    json!({"body_bytes_read":body.len(),"body_end":body_end,"json_valid":parsed.is_some(),
        "error_code":code,"error_code_state":if code.is_some() { "recognized" }
            else if candidates.iter().any(Option::is_some) { "unrecognized" } else { "missing" },
        "original_status_code":original_status})
}

async fn read_capped_error_body(
    reader: impl tokio::io::AsyncRead + Unpin,
) -> (Vec<u8>, &'static str) {
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    let result = reader
        .take(MAX_ERROR_BODY_BYTES as u64)
        .read_to_end(&mut bytes)
        .await;
    let end = if result.is_err() {
        "body_read_error"
    } else if bytes.len() == MAX_ERROR_BODY_BYTES {
        "capped"
    } else {
        "complete"
    };
    (bytes, end)
}

async fn http_error_diagnostic(response: reqwest::Response) -> Value {
    let headers = response.headers().clone();
    let stream = response
        .bytes_stream()
        .map(|chunk| chunk.map_err(std::io::Error::other));
    let (body, end) = read_capped_error_body(tokio_util::io::StreamReader::new(stream)).await;
    project_http_error(&headers, &body, end)
}

async fn run_probe(settings: &Settings, state: &mut ProbeState) -> ProbeResult<()> {
    let config = Config::global();
    let access: String = config
        .get_secret("VERSA_BEDROCK_ACCESS_KEY_ID")
        .map_err(|_| "credential_read_failed")?;
    let secret: String = config
        .get_secret("VERSA_BEDROCK_SECRET_ACCESS_KEY")
        .map_err(|_| "credential_read_failed")?;
    if access.trim().is_empty() || secret.trim().is_empty() {
        return Err("empty_credentials");
    }
    let credentials = Credentials::new(access, secret, None, None, "VersaWireProbe");
    let (client, captured) = captured_client(settings, credentials.clone(), None).await?;
    let _ = request(&client, settings)?.send().await;
    let signed = captured.expect_request();
    let url = reqwest::Url::parse(signed.uri()).map_err(|_| "invalid_signed_request_url")?;
    if url.scheme() != "https"
        || url.host_str() != Some("unified-api.ucsf.edu")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().starts_with("/general/awsai/model/")
        || !url.path().ends_with("/converse-stream")
        || signed.method() != "POST"
        || !signed.headers().contains_key("authorization")
    {
        return Err("signed_request_target_or_auth_invalid");
    }
    let body = signed
        .body()
        .bytes()
        .ok_or("signed_request_body_not_buffered")?
        .to_vec();
    let mut http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .gzip(false)
        .brotli(false)
        .deflate(false)
        .zstd(false)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(80));
    if settings.http1_only {
        http = http.http1_only();
    }
    let http = http.build().map_err(|_| "http_client_build_failed")?;
    let mut replay = http.post(url).body(body);
    for (name, value) in signed.headers().iter() {
        replay = replay.header(name, value);
    }
    let response = replay
        .send()
        .await
        .map_err(|error| request_error_class(&error))?;
    let status = response.status().as_u16();
    let content_type = match response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
    {
        Some("application/vnd.amazon.eventstream") => "application/vnd.amazon.eventstream",
        Some("application/json") => "application/json",
        Some(_) => "other",
        None => "missing",
    };
    let version = match response.version() {
        reqwest::Version::HTTP_11 => "http/1.1",
        reqwest::Version::HTTP_2 => "h2",
        _ => "other",
    };
    state.http = json!({"status":status,"content_type":content_type,"version":version,
        "correlation_ids":safe_correlation_ids(response.headers())});
    if status != 200 || content_type != "application/vnd.amazon.eventstream" {
        if status != 200 {
            state.http["error_diagnostic"] = http_error_diagnostic(response).await;
        }
        return Err("unexpected_http_status_or_content_type");
    }
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| request_error_class(&error))?;
        if state.body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err("response_byte_cap");
        }
        state.body.extend_from_slice(&chunk);
        state.parse_available()?;
    }
    state.raw_end = if state.parsed_bytes == state.body.len() {
        "clean_eof"
    } else {
        "partial_frame_eof"
    };
    state.sdk = sdk_replay(settings, credentials, state.body.clone()).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "manual live UCSF request; requires explicit opt-in and a new metadata-only /tmp output"]
async fn manual_versa_stream_wire_probe() {
    assert_eq!(
        std::env::var("BIOROUTER_RUN_VERSA_STREAM_PROBE").as_deref(),
        Ok("1"),
        "set BIOROUTER_RUN_VERSA_STREAM_PROBE=1 explicitly"
    );
    let (_path, mut output) = reserve_output().unwrap_or_else(|class| panic!("{class}"));
    save_report(
        &mut output,
        &json!({"schema":"versa_stream_wire_probe.v1","outcome":"running"}),
    )
    .unwrap();
    let finished = Arc::new(AtomicBool::new(false));
    let watchdog = Arc::clone(&finished);
    std::thread::spawn(move || {
        std::thread::sleep(HARD_DEADLINE);
        if !watchdog.load(Ordering::Acquire) {
            eprintln!("Versa wire probe hard deadline reached; no credential or response content was logged");
            std::process::exit(124);
        }
    });
    let start = Instant::now();
    let settings = Settings::load().unwrap_or_else(|class| panic!("{class}"));
    let mut state = ProbeState::default();
    let result = tokio::time::timeout(ASYNC_DEADLINE, run_probe(&settings, &mut state)).await;
    let outcome = match result {
        Ok(Ok(())) => "capture_complete",
        Ok(Err(class)) => class,
        Err(_) => "probe_timeout",
    };
    if state.raw_end.is_empty() {
        state.raw_end = outcome;
    }
    let report = json!({"schema":"versa_stream_wire_probe.v1","outcome":outcome,
        "model":settings.model.model_name,"case":settings.case.name(),
        "requested_transport":if settings.http1_only { "http1" } else { "auto" },
        "configured_max_tokens":settings.model.max_tokens,
        "effective_max_tokens":bedrock_inference_config(&settings.model).max_tokens(),
        "elapsed_ms":start.elapsed().as_millis(),"hard_deadline_secs":HARD_DEADLINE.as_secs(),
        "response_byte_cap":MAX_RESPONSE_BYTES,"observations":state.summary()});
    save_report(&mut output, &report).unwrap_or_else(|class| panic!("{class}"));
    finished.store(true, Ordering::Release);
    assert_eq!(
        outcome, "capture_complete",
        "probe could not finish; inspect metadata-only output"
    );
}

#[cfg(test)]
mod offline_controls {
    use super::*;

    // Fixed independently with Python's stdlib zlib.crc32, not the parser CRC.
    const GOLDEN_STOP_FRAME: &[u8] = b"\x00\x00\x00\x5c\x00\x00\x00\x35\xce\x72\x39\x80\
        \x0d:message-type\x07\x00\x05event\
        \x0b:event-type\x07\x00\x10contentBlockStop\
        {\"contentBlockIndex\":3}\x05\x39\x33\xd6";
    const SENTINEL: &str = "WIRE_PROBE_SYNTHETIC_SENTINEL";

    #[test]
    fn short_sqlite_case_preserves_the_editor_tool_contract() {
        assert_eq!(
            serde_json::to_value(ProbeCase::PythonSqliteShort.tool()).unwrap(),
            serde_json::to_value(ProbeCase::PythonSqlite.tool()).unwrap(),
        );
        assert_eq!(ProbeCase::PythonSqliteShort.name(), "python_sqlite_short");
        assert!(ProbeCase::PythonSqliteShort
            .prompt()
            .contains("10-to-15-line"));
        assert!(ProbeCase::PythonSqlite.prompt().contains("180-to-260-line"));
        assert!(ProbeCase::Todo
            .prompt()
            .contains("five specific inventory checks"));
    }

    #[test]
    fn http_error_projection_retains_only_allowlisted_codes_and_numeric_status() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("authorization", SENTINEL.parse().unwrap());
        headers.insert("set-cookie", SENTINEL.parse().unwrap());
        let body = serde_json::to_vec(&json!({
            "__type":"com.amazonaws.bedrockruntime#ModelStreamErrorException",
            "originalStatusCode":500,"message":SENTINEL,"originalMessage":SENTINEL,
            "detail":SENTINEL,"arguments":{"secret":SENTINEL}
        }))
        .unwrap();
        let report = project_http_error(&headers, &body, "complete");
        assert_eq!(report["error_code"], "ModelStreamErrorException");
        assert_eq!(report["original_status_code"], 500);
        assert_eq!(report["body_end"], "complete");
        assert_eq!(report["json_valid"], true);
        assert!(!serde_json::to_string(&report).unwrap().contains(SENTINEL));
        assert_eq!(safe_correlation_ids(&headers), json!({}));
    }

    #[test]
    fn unknown_error_codes_and_body_end_labels_are_not_echoed() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-amzn-errortype", SENTINEL.parse().unwrap());
        let body = serde_json::to_vec(&json!({"error":{"code":SENTINEL,
            "originalStatusCode":999,"message":SENTINEL}}))
        .unwrap();
        let report = project_http_error(&headers, &body, SENTINEL);
        assert_eq!(report["error_code"], Value::Null);
        assert_eq!(report["error_code_state"], "unrecognized");
        assert_eq!(report["original_status_code"], Value::Null);
        assert_eq!(report["body_end"], "body_read_error");
        assert!(!serde_json::to_string(&report).unwrap().contains(SENTINEL));
        headers.insert(
            "x-amzn-errortype",
            "ValidationException:private-suffix".parse().unwrap(),
        );
        assert_eq!(
            project_http_error(&headers, b"not json", "complete")["error_code"],
            "ValidationException"
        );
    }

    #[test]
    fn correlation_headers_require_canonical_uuid_shapes() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "x-amzn-requestid",
            "12345678-1234-5678-9abc-123456789abc".parse().unwrap(),
        );
        headers.insert(
            "apim-request-id",
            "12345678123456789ABC123456789ABC".parse().unwrap(),
        );
        headers.insert("x-request-id", SENTINEL.parse().unwrap());
        headers.insert(
            "x-correlation-id",
            "{12345678-1234-5678-9abc-123456789abc}".parse().unwrap(),
        );
        headers.insert("authorization", SENTINEL.parse().unwrap());
        let report = safe_correlation_ids(&headers);
        for name in ["x-amzn-requestid", "apim-request-id"] {
            assert_eq!(
                report[name]["value"],
                "12345678-1234-5678-9abc-123456789abc"
            );
        }
        for name in ["x-request-id", "x-correlation-id"] {
            assert_eq!(report[name], json!({"state":"present_unrecognized"}));
        }
        assert!(report.get("authorization").is_none());
        assert!(!serde_json::to_string(&report).unwrap().contains(SENTINEL));
    }

    #[tokio::test]
    async fn error_body_reader_and_projection_enforce_sixteen_kib_limit() {
        let small = b"{\"code\":\"InternalServerError\"}";
        let (captured, end) = read_capped_error_body(small.as_slice()).await;
        assert_eq!(captured, small.to_vec());
        assert_eq!(end, "complete");
        let oversized = vec![b'x'; MAX_ERROR_BODY_BYTES + 17];
        let mut unread = oversized.as_slice();
        let (captured, end) = read_capped_error_body(&mut unread).await;
        assert_eq!(captured.len(), MAX_ERROR_BODY_BYTES);
        assert_eq!(
            unread.len(),
            17,
            "the reader must not consume beyond the cap"
        );
        assert_eq!(end, "capped");
        let report = project_http_error(&reqwest::header::HeaderMap::new(), &oversized, "complete");
        assert_eq!(report["body_bytes_read"], MAX_ERROR_BODY_BYTES);
        assert_eq!(report["body_end"], "capped");
        assert_eq!(report["json_valid"], false);
    }

    #[test]
    fn transport_mode_has_an_explicit_allowlist() {
        assert_eq!(transport_http1_only("auto"), Ok(false));
        assert_eq!(transport_http1_only("http1"), Ok(true));
        for value in ["", "h2", "HTTP1", "https://elsewhere.invalid"] {
            assert_eq!(
                transport_http1_only(value),
                Err("unsupported_probe_transport")
            );
        }
    }

    // The fixture writer uses the MSB-first polynomial formulation, independently
    // of the reader's reflected, LSB-first implementation and fixed golden bytes.
    fn fixture_crc(bytes: &[u8]) -> u32 {
        let mut crc = u32::MAX;
        for byte in bytes {
            crc ^= u32::from(byte.reverse_bits()) << 24;
            for _ in 0..8 {
                let top_bit = crc & 0x8000_0000 != 0;
                crc <<= 1;
                if top_bit {
                    crc ^= 0x04c1_1db7;
                }
            }
        }
        !crc.reverse_bits()
    }

    fn frame(tag: &str, payload: &[u8]) -> Vec<u8> {
        let mut headers = b"\x0d:message-type\x07\x00\x05event\x0b:event-type\x07".to_vec();
        headers.extend_from_slice(&u16::try_from(tag.len()).unwrap().to_be_bytes());
        headers.extend_from_slice(tag.as_bytes());
        let total = u32::try_from(16 + headers.len() + payload.len()).unwrap();
        let mut bytes = total.to_be_bytes().to_vec();
        bytes.extend_from_slice(&u32::try_from(headers.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(&fixture_crc(&bytes).to_be_bytes());
        bytes.extend_from_slice(&headers);
        bytes.extend_from_slice(payload);
        bytes.extend_from_slice(&fixture_crc(&bytes).to_be_bytes());
        bytes
    }

    fn event(tag: &str, payload: Value) -> Vec<u8> {
        frame(tag, &serde_json::to_vec(&payload).unwrap())
    }

    fn tool_start(index: i64) -> Vec<u8> {
        event(
            "contentBlockStart",
            json!({"contentBlockIndex":index,
            "start":{"toolUse":{"toolUseId":"synthetic","name":"developer__text_editor"}}}),
        )
    }

    fn tool_delta(index: i64, input: &str) -> Vec<u8> {
        event(
            "contentBlockDelta",
            json!({"contentBlockIndex":index,
            "delta":{"toolUse":{"input":input}}}),
        )
    }

    fn parsed(bytes: &[u8]) -> ProbeState {
        let mut state = ProbeState {
            body: bytes.to_vec(),
            ..ProbeState::default()
        };
        state.parse_available().unwrap();
        state
    }

    #[test]
    fn crc_and_frame_match_independent_goldens() {
        for crc in [crc32 as fn(&[u8]) -> u32, fixture_crc] {
            assert_eq!(crc(b""), 0);
            assert_eq!(crc(b"123456789"), 0xcbf4_3926);
            assert_eq!(crc(&GOLDEN_STOP_FRAME[..8]), 0xce72_3980);
            assert_eq!(crc(&GOLDEN_STOP_FRAME[..88]), 0x0539_33d6);
        }
        assert_eq!(
            frame("contentBlockStop", b"{\"contentBlockIndex\":3}"),
            GOLDEN_STOP_FRAME
        );
        let state = parsed(GOLDEN_STOP_FRAME);
        assert_eq!(state.parsed_bytes, 92);
        assert_eq!(state.audit.events.len(), 1);
        assert_eq!(state.audit.events[0]["index"], 3);
        assert_eq!(state.audit.events[0]["tag"], "contentBlockStop");
    }

    #[test]
    fn every_byte_and_every_split_preserve_frames_and_utf8() {
        let body = [
            tool_start(3),
            tool_delta(3, "{\"path\":\"/tmp/"),
            tool_delta(3, "分析.py\"}"),
            GOLDEN_STOP_FRAME.to_vec(),
            event("messageStop", json!({"stopReason":"tool_use"})),
        ]
        .concat();
        let expected = parsed(&body).summary();
        assert_eq!(expected["raw"]["blocks"][0]["closed"], true);
        assert_eq!(
            expected["raw"]["blocks"][0]["input_json_status"],
            "valid_object"
        );
        assert_eq!(expected["raw"]["message_stop_reason"], "tool_use");
        let mut incremental = ProbeState::default();
        for byte in &body {
            incremental.body.push(*byte);
            incremental.parse_available().unwrap();
        }
        assert_eq!(incremental.summary(), expected);
        for split in 0..=body.len() {
            let mut state = parsed(&body[..split]);
            state.body.extend_from_slice(&body[split..]);
            state.parse_available().unwrap();
            assert_eq!(state.summary(), expected, "split at byte {split}");
        }
    }

    #[test]
    fn prelude_and_message_crc_corruption_are_distinct() {
        for (offset, expected) in [(8, "prelude_crc_mismatch"), (91, "message_crc_mismatch")] {
            let mut state = ProbeState {
                body: GOLDEN_STOP_FRAME.to_vec(),
                ..ProbeState::default()
            };
            state.body[offset] ^= 1;
            assert_eq!(state.parse_available(), Err(expected));
            assert_eq!(state.parsed_bytes, 0);
            assert!(state.audit.events.is_empty());
        }
    }

    #[test]
    fn every_truncation_and_exact_twelve_byte_prelude_remain_visible() {
        for cut in 1..GOLDEN_STOP_FRAME.len() {
            let state = parsed(&GOLDEN_STOP_FRAME[..cut]);
            let report = state.summary();
            assert_eq!(report["parsed_bytes"], 0);
            assert_eq!(report["trailing_bytes"], cut);
            assert!(state.audit.events.is_empty());
            assert!(!report["trailing_frame"].is_null());
        }
        let body = [GOLDEN_STOP_FRAME, &GOLDEN_STOP_FRAME[..12]].concat();
        let state = parsed(&body);
        let report = state.summary();
        assert_eq!(state.audit.events.len(), 1);
        assert_eq!(report["parsed_bytes"], 92);
        assert_eq!(report["trailing_bytes"], 12);
        assert_eq!(report["trailing_frame"]["offset"], 92);
        assert_eq!(report["trailing_frame"]["declared_frame_bytes"], 92);
        assert_eq!(report["trailing_frame"]["prelude_crc_valid"], true);
    }

    #[test]
    fn missing_null_and_invalid_indices_never_default_to_zero() {
        for (index, expected_state, expected_index) in [
            (None, "missing", None),
            (Some(Value::Null), "null", None),
            (Some(json!("3")), "invalid_type", None),
            (Some(json!(0)), "integer", Some(0)),
            (Some(json!(1)), "integer", Some(1)),
            (Some(json!(3)), "integer", Some(3)),
        ] {
            let mut payload = json!({"start":{"toolUse":{
                "toolUseId":"synthetic","name":"todo__todo_write"}}});
            if let Some(index) = index {
                payload["contentBlockIndex"] = index;
            }
            let state = parsed(&event("contentBlockStart", payload));
            assert_eq!(state.audit.events[0]["index_state"], expected_state);
            assert_eq!(state.audit.events[0]["index"], json!(expected_index));
            assert_eq!(
                state.audit.unindexed_events,
                usize::from(expected_index.is_none())
            );
            assert_eq!(
                state.audit.blocks.len(),
                usize::from(expected_index.is_some())
            );
            if let Some(index) = expected_index {
                assert!(state.audit.blocks.contains_key(&index));
            }
        }
    }

    #[test]
    fn wrong_stop_index_does_not_close_a_valid_json_block() {
        let body = [
            tool_start(3),
            tool_delta(3, "{}"),
            event("contentBlockStop", json!({"contentBlockIndex":0})),
        ]
        .concat();
        let report = parsed(&body).summary();
        assert_eq!(report["raw"]["blocks"][0]["index"], 3);
        assert_eq!(
            report["raw"]["blocks"][0]["input_json_status"],
            "valid_object"
        );
        assert_eq!(report["raw"]["blocks"][0]["closed"], false);
        assert_eq!(report["raw"]["events"][2]["index"], 0);
    }

    #[test]
    fn json_validity_and_protocol_closure_are_independent() {
        for (input, expected) in [
            ("{}", "valid_object"),
            ("[]", "valid_non_object"),
            ("{", "incomplete_json"),
            ("invalid", "invalid_json"),
            ("", "empty"),
        ] {
            for closed in [false, true] {
                let mut body = [tool_start(3), tool_delta(3, input)].concat();
                if closed {
                    body.extend_from_slice(GOLDEN_STOP_FRAME);
                }
                let report = parsed(&body).summary();
                assert_eq!(report["raw"]["blocks"][0]["closed"], closed);
                assert_eq!(report["raw"]["blocks"][0]["input_json_status"], expected);
            }
        }
    }

    #[test]
    fn serialized_summary_never_contains_argument_or_unknown_header_content() {
        let input = json!({"secret":SENTINEL}).to_string();
        let body = [
            event(
                "contentBlockStart",
                json!({"contentBlockIndex":3,
                "start":{"toolUse":{"toolUseId":SENTINEL,"name":SENTINEL}}}),
            ),
            tool_delta(3, &input),
            GOLDEN_STOP_FRAME.to_vec(),
            event(SENTINEL, json!({"unrecognized_payload":SENTINEL})),
            event("messageStop", json!({"stopReason":SENTINEL})),
            event(
                "metadata",
                json!({"usage":{"inputTokens":12,"outputTokens":8,
                "totalTokens":20,"unrecognized":SENTINEL},"other":SENTINEL}),
            ),
        ]
        .concat();
        let report = parsed(&body).summary();
        let serialized = serde_json::to_string(&report).unwrap();
        assert!(!serialized.contains(SENTINEL));
        assert!(!serialized.contains(&input));
        assert_eq!(report["raw"]["blocks"][0]["tool"], "unexpected_tool");
        assert_eq!(report["raw"]["blocks"][0]["input_bytes"], input.len());
        assert_eq!(report["raw"]["message_stop_reason"], "unknown");
        assert_eq!(report["raw"]["usage"]["total_tokens"], 20);
    }

    #[test]
    fn malformed_header_lengths_and_types_fail_without_panicking() {
        assert_eq!(control_headers(&[0, 7]), Err("invalid_event_header"));
        assert_eq!(control_headers(b"\x03x"), Err("invalid_event_header"));
        assert_eq!(
            control_headers(b"\x01x\x63"),
            Err("invalid_event_header_type")
        );
        assert_eq!(
            control_headers(b"\x01x\x07\x00\x02x"),
            Err("truncated_event_header")
        );
    }
}
