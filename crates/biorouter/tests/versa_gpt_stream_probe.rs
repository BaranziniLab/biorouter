//! Opt-in synthetic provider/decoder probe; returned tools are never executed.
//! Requires BIOROUTER_RUN_VERSA_GPT_PROBE=1, BIOROUTER_VERSA_GPT_PROBE_MODEL,
//! matching VERSA_AZURE_DEPLOYMENT_NAME, AZURE_OPENAI_ENDPOINT, and a NEW
//! BIOROUTER_VERSA_GPT_PROBE_OUTPUT directly under /tmp. CASE defaults to todo;
//! BIOROUTER_VERSA_GPT_PROBE_CASE=python_sqlite requests a medium file instead.
//! gpt-5.6 is an unverified requested deployment, not a claim of availability.

use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use biorouter::conversation::message::{Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::providers::base::{Provider, ProviderStreamItem};
use biorouter::providers::versa_azure::VersaAzureProvider;
use futures::StreamExt;
use rmcp::model::Tool;
use serde::Serialize;
use serde_json::{json, Value};

type ProbeResult<T> = Result<T, &'static str>;
const HARD_SECONDS: u64 = 95;

fn approved_model(model: &str, deployment: &str) -> ProbeResult<&'static str> {
    if model != deployment {
        return Err("model_deployment_mismatch");
    }
    match model {
        "gpt-5.6" => Ok("gpt-5.6"),
        "gpt-5.5-2026-04-24" => Ok("gpt-5.5-2026-04-24"),
        _ => Err("unsupported_probe_model"),
    }
}

fn approved_endpoint(endpoint: &str) -> ProbeResult<()> {
    let url = reqwest::Url::parse(endpoint).map_err(|_| "invalid_endpoint")?;
    if url.scheme() != "https"
        || url.host_str() != Some("unified-api.ucsf.edu")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path().trim_end_matches('/') != "/general"
    {
        return Err("unapproved_endpoint");
    }
    Ok(())
}

fn safe_tag(value: Option<&str>) -> &'static str {
    match value {
        Some("stop") => "stop",
        Some("tool_calls") => "tool_calls",
        Some("function_call") => "function_call",
        Some("length") => "length",
        Some("content_filter") => "content_filter",
        Some("invalid_arguments") => "invalid_arguments",
        Some("incomplete_stream") => "incomplete_stream",
        Some("todo__todo_write") => "todo__todo_write",
        Some("developer__text_editor") => "developer__text_editor",
        Some("gpt-5.6") => "gpt-5.6",
        Some("gpt-5.5") => "gpt-5.5",
        Some("gpt-5.5-2026-04-24") => "gpt-5.5-2026-04-24",
        None => "missing",
        _ => "other",
    }
}

fn synthetic_case(case: &str) -> ProbeResult<(&'static str, Tool)> {
    let (prompt, name, schema) = match case {
        "todo" => (
            "Synthetic transport test: write a two-sentence plan, then call todo__todo_write once with a Markdown checklist of five fictional inventory checks. Use no personal data. No tool will be executed.",
            "todo__todo_write",
            json!({"type":"object","properties":{"content":{"type":"string"}},"required":["content"],"additionalProperties":false}),
        ),
        "python_sqlite" => (
            "Synthetic transport test: write a two-sentence plan, then call developer__text_editor once with command=create, path=/tmp/versa_gpt_synthetic_inventory.py, and file_text containing a meaningful complete 180-to-260-line stdlib Python SQLite inventory CLI. Include items and adjustments tables, foreign keys, transactional add/adjust, argparse init/add/list/adjust/audit, parameterized SQL, validation, helpful errors, and deterministic in-memory unittest self-tests. No placeholders, repeated filler, real data, file access, or execution. No tool will actually run.",
            "developer__text_editor",
            json!({"type":"object","properties":{"command":{"type":"string","enum":["create"]},"path":{"type":"string"},"file_text":{"type":"string"}},"required":["command","path","file_text"],"additionalProperties":false}),
        ),
        _ => return Err("unsupported_probe_case"),
    };
    Ok((
        prompt,
        Tool::new(
            name,
            "Synthetic diagnostic tool; never executed.",
            schema.as_object().unwrap().clone(),
        ),
    ))
}

#[derive(Default, Serialize)]
struct Summary {
    items: usize,
    pending_updates: usize,
    largest_pending_arguments_bytes: usize,
    text_bytes: usize,
    tool_calls: Vec<Value>,
    usage: Value,
}

impl Summary {
    fn observe(&mut self, (message, usage, pending): ProviderStreamItem) {
        self.items += 1;
        if let Some(pending) = pending {
            self.pending_updates += 1;
            self.largest_pending_arguments_bytes = self
                .largest_pending_arguments_bytes
                .max(pending.partial_args.as_ref().map_or(0, String::len));
        }
        if let Some(usage) = usage {
            self.usage = json!({"model":safe_tag(Some(&usage.model)),
                "finish_reason":safe_tag(usage.finish_reason.as_deref()),
                "input_tokens":usage.usage.input_tokens,"output_tokens":usage.usage.output_tokens,
                "total_tokens":usage.usage.total_tokens});
        }
        for content in message.into_iter().flat_map(|message| message.content) {
            match content {
                MessageContent::Text(text) => self.text_bytes += text.text.len(),
                MessageContent::ToolRequest(request) => self.tool_calls.push(match request.tool_call {
                    Ok(call) => json!({"state":"accepted","tool":safe_tag(Some(&call.name)),
                        "arguments_object":call.arguments.is_some(),
                        "argument_bytes":serde_json::to_vec(&call.arguments).map_or(0, |bytes| bytes.len())}),
                    Err(error) => json!({"state":"failed","failure":safe_tag(error.data.as_ref()
                        .and_then(|data| data.get("biorouterToolCallFailure")).and_then(Value::as_str))}),
                }),
                _ => {}
            }
        }
    }
}

async fn probe(model: ModelConfig, case: &str, summary: &mut Summary) -> ProbeResult<()> {
    let provider = VersaAzureProvider::from_env(model.clone())
        .await
        .map_err(|_| "provider_initialization_failed")?;
    let binding =
        serde_json::to_value(provider.restore_binding()).map_err(|_| "binding_unavailable")?;
    approved_endpoint(
        binding["endpoint"]
            .as_str()
            .ok_or("binding_endpoint_missing")?,
    )?;
    if binding["deployment"] != model.model_name || binding["credential_source"] != "api_key" {
        return Err("resolved_deployment_or_auth_source_mismatch");
    }
    let (prompt, tool) = synthetic_case(case)?;
    let mut stream = provider
        .stream(
            "Follow the synthetic task and the exact tool schema. Never access real data.",
            &[Message::user().with_text(prompt)],
            &[tool],
        )
        .await
        .map_err(|error| error.kind().wire_code())?;
    while let Some(item) = stream.next().await {
        summary.observe(item.map_err(|error| error.kind().wire_code())?);
        if summary.items > 4096
            || summary.text_bytes > 8 * 1024 * 1024
            || summary.largest_pending_arguments_bytes > 8 * 1024 * 1024
        {
            return Err("decoded_output_cap");
        }
    }
    Ok(())
}

fn required(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("required probe environment variable missing: {name}"))
}

#[tokio::test]
#[ignore = "manual UCSF request; explicit opt-in, matching model/deployment and new output required"]
async fn manual_versa_gpt_stream_probe() {
    assert_eq!(
        std::env::var("BIOROUTER_RUN_VERSA_GPT_PROBE").as_deref(),
        Ok("1")
    );
    let model = approved_model(
        &required("BIOROUTER_VERSA_GPT_PROBE_MODEL"),
        &required("VERSA_AZURE_DEPLOYMENT_NAME"),
    )
    .unwrap_or_else(|class| panic!("{class}"));
    approved_endpoint(&required("AZURE_OPENAI_ENDPOINT")).unwrap_or_else(|class| panic!("{class}"));
    let case = std::env::var("BIOROUTER_VERSA_GPT_PROBE_CASE").unwrap_or_else(|_| "todo".into());
    synthetic_case(&case).unwrap_or_else(|class| panic!("{class}"));
    let path = required("BIOROUTER_VERSA_GPT_PROBE_OUTPUT");
    assert_eq!(
        Path::new(&path).parent(),
        Some(Path::new("/tmp")),
        "output must be directly under /tmp"
    );
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options
        .open(path)
        .unwrap_or_else(|_| panic!("output_create_new_failed"));
    output.write_all(b"{\"outcome\":\"running\"}\n").unwrap();
    let finished = Arc::new(AtomicBool::new(false));
    let watchdog = Arc::clone(&finished);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(HARD_SECONDS));
        if !watchdog.load(Ordering::Acquire) {
            std::process::exit(124);
        }
    });
    let start = Instant::now();
    let config = ModelConfig::new(model).unwrap_or_else(|_| panic!("invalid_model_configuration"));
    let mut summary = Summary::default();
    let outcome = match tokio::time::timeout(
        Duration::from_secs(90),
        probe(config.clone(), &case, &mut summary),
    )
    .await
    {
        Ok(Ok(())) => "decoded_stream_eof",
        Ok(Err(class)) => class,
        Err(_) => "probe_timeout",
    };
    let report = json!({"schema":"versa_gpt_stream_probe.v1","model":model,"deployment":model,
        "case":case,"configured_max_tokens":config.max_tokens,"outcome":outcome,
        "elapsed_ms":start.elapsed().as_millis(),"hard_deadline_secs":HARD_SECONDS,"summary":summary});
    output.seek(SeekFrom::Start(0)).unwrap();
    output.set_len(0).unwrap();
    serde_json::to_writer_pretty(&mut output, &report).unwrap();
    output.sync_all().unwrap();
    finished.store(true, Ordering::Release);
    assert_eq!(
        outcome, "decoded_stream_eof",
        "inspect metadata-only probe output"
    );
}

#[test]
fn offline_probe_validation_rejects_route_and_model_drift() {
    assert!(approved_model("gpt-5.6", "gpt-5.6").is_ok());
    assert!(approved_model("gpt-5.6", "gpt-5.5-2026-04-24").is_err());
    assert!(approved_model("other", "other").is_err());
    assert!(approved_endpoint("https://unified-api.ucsf.edu/general").is_ok());
    for endpoint in [
        "http://unified-api.ucsf.edu/general",
        "https://elsewhere.invalid/general",
        "https://secret@unified-api.ucsf.edu/general",
        "https://unified-api.ucsf.edu/general?key=secret",
    ] {
        assert!(approved_endpoint(endpoint).is_err());
    }
    assert!(synthetic_case("arbitrary_prompt").is_err());
}

#[test]
fn offline_probe_summary_excludes_provider_content_and_errors() {
    use biorouter::providers::base::{PendingToolCall, ProviderUsage, Usage};
    use rmcp::model::{CallToolRequestParams, ErrorCode, ErrorData};
    const SECRET: &str = "SYNTHETIC_SECRET_MUST_NOT_APPEAR";
    let mut summary = Summary::default();
    summary.observe((
        Some(
            Message::assistant()
                .with_text(SECRET)
                .with_tool_request(
                    SECRET,
                    Err(ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        SECRET,
                        Some(json!({"biorouterToolCallFailure":SECRET,"raw":SECRET})),
                    )),
                )
                .with_tool_request(
                    "accepted",
                    Ok(CallToolRequestParams {
                        task: None,
                        name: SECRET.into(),
                        arguments: Some(json!({"secret":SECRET}).as_object().unwrap().clone()),
                        meta: None,
                    }),
                ),
        ),
        Some(ProviderUsage::new(SECRET.into(), Usage::default())),
        Some(PendingToolCall {
            id: SECRET.into(),
            name: SECRET.into(),
            partial_args: Some(SECRET.into()),
        }),
    ));
    let encoded = serde_json::to_string(&summary).unwrap();
    assert!(!encoded.contains(SECRET));
    assert_eq!(summary.tool_calls[0]["failure"], "other");
    assert_eq!(summary.tool_calls[1]["tool"], "other");
    assert_eq!(summary.tool_calls[1]["arguments_object"], true);
    assert_eq!(summary.usage["model"], "other");
}
