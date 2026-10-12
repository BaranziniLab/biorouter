//! Comprehensive live survey of EVERY curated Llama Server model.
//!
//! Unlike `llamacpp_integration.rs` (one selected model, hard assertions), this is
//! a *survey*: it walks the whole `MODEL_CATALOG`, downloads/loads each model
//! through the real Biorouter provider + managed sidecar, and runs a battery of
//! capability / correctness / speed / robustness checks, writing an
//! incrementally-updated markdown report. Individual model failures are
//! recorded so one bad model never aborts the battery. After the final report
//! is saved, any failed check fails the Rust test.
//!
//! ```sh
//! BIOROUTER_LLAMACPP_BIN=ui/desktop/src/bin/llamacpp/llama-server \
//!   cargo test -p biorouter --test llamacpp_survey -- --ignored --nocapture
//! ```
//!
//! Env knobs:
//!   LLAMACPP_SURVEY_MODELS   comma-separated catalog names to limit the run
//!                            (default: every catalog entry, in order)
//!   LLAMACPP_SURVEY_OUT      report path (default: ~/Desktop/llamacpp-survey-report.md)
//!
//! First run of each model includes a multi-GB Hugging Face download (~19 GB
//! for Qwen3.8 27B, ~18 GB for Gemma 4 31B) unless the file is already in the
//! shared model store.

use std::time::{Duration, Instant};

use biorouter::conversation::message::{Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::providers::base::{Provider, Usage};
use biorouter::providers::llamacpp::{
    resolve_model_source, CatalogEntry, LlamaCppProvider, MODEL_CATALOG,
};
use biorouter::providers::llamacpp_sidecar::{global, SidecarState};
use futures::StreamExt;
use rmcp::{model::Tool, object};

// Generous so large models on a slow link still finish DOWNLOADING within the
// window (the xet CDN here ranges from ~0.6 to ~40 MB/s; each model is ~18-19 GB).
// llama.cpp resumes partial downloads across restarts, so a kill/restart never
// loses progress. Override with LLAMACPP_SURVEY_READY_SECS.
fn ready_timeout() -> Duration {
    let secs = std::env::var("LLAMACPP_SURVEY_READY_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(28_800); // 8 hours
    Duration::from_secs(secs)
}

#[derive(Default)]
struct Check {
    pass: bool,
    detail: String,
}

impl Check {
    fn ok(detail: impl Into<String>) -> Self {
        Check {
            pass: true,
            detail: detail.into(),
        }
    }
    fn fail(detail: impl Into<String>) -> Self {
        Check {
            pass: false,
            detail: detail.into(),
        }
    }
    fn mark(&self) -> &'static str {
        if self.pass {
            "✅"
        } else {
            "❌"
        }
    }
}

#[derive(Default)]
struct ModelReport {
    name: String,
    display_name: String,
    hf_spec: String,
    download_size: String,
    native_context_limit: usize,
    allocated_context_limit: Option<usize>,
    /// Download + load time to first Ready.
    load_secs: Option<f64>,
    availability: Check,
    correctness: Check,
    thinking_off: Check,
    tool_calling: Check,
    /// A structured tool call from a ~30k-token prompt, the size of
    /// Biorouter's real first turn, with its prefill rate.
    bootstrap: Check,
    streaming: Check,
    speed_toks_per_sec: Option<f64>,
    speed: Check,
}

impl ModelReport {
    fn failed_checks(&self) -> Vec<String> {
        [
            ("availability", &self.availability),
            ("correctness", &self.correctness),
            ("thinking-off", &self.thinking_off),
            ("tool calling", &self.tool_calling),
            ("bootstrap", &self.bootstrap),
            ("streaming", &self.streaming),
            ("speed", &self.speed),
        ]
        .into_iter()
        .filter(|(_, check)| !check.pass)
        .map(|(label, check)| format!("{}: {label}: {}", self.name, check.detail))
        .collect()
    }
}

fn prompt_tokens(usage: &Usage) -> i32 {
    usage.input_tokens.unwrap_or(0) + usage.cache_read_input_tokens.unwrap_or(0)
}

fn has_paris_weather_request(message: &Message) -> bool {
    message.content.iter().any(|content| {
        let MessageContent::ToolRequest(request) = content else {
            return false;
        };
        request.tool_call.as_ref().is_ok_and(|call| {
            call.name == "get_weather"
                && call
                    .arguments
                    .as_ref()
                    .and_then(|arguments| arguments.get("location"))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|location| {
                        let city = location.split(',').next().unwrap_or_default().trim();
                        ["Paris", "Paris France", "Paris FR"]
                            .iter()
                            .any(|paris| city.eq_ignore_ascii_case(paris))
                    })
        })
    })
}

fn weather_tool() -> Tool {
    Tool::new(
        "get_weather".to_string(),
        "Get current temperature for a given location.".to_string(),
        object!({
            "type": "object",
            "required": ["location"],
            "properties": { "location": {"type": "string"} }
        }),
    )
}

async fn provider_for(name: &str, max_tokens: usize) -> anyhow::Result<LlamaCppProvider> {
    let model = ModelConfig::new(name)?.with_max_tokens(Some(max_tokens as i32));
    LlamaCppProvider::from_env(model).await
}

/// Run the full battery against one already-Ready model.
async fn run_battery(report: &mut ModelReport) {
    let provider = match provider_for(&report.name, 256).await {
        Ok(p) => p,
        Err(e) => {
            report.correctness = Check::fail(format!("provider init failed: {e}"));
            return;
        }
    };

    // 1. Correctness + 2. thinking-off (content must be non-empty; reasoning
    //    models can otherwise burn the budget on thinking tokens).
    let msgs = vec![Message::user().with_text("Reply with exactly the word: pong")];
    match provider
        .complete("You are a terse assistant.", &msgs, &[])
        .await
    {
        Ok((message, usage)) => {
            let text = message.as_concat_text();
            let low = text.to_lowercase();
            report.thinking_off = if text.trim().is_empty() {
                Check::fail("empty content (thinking-off likely NOT applied)")
            } else {
                Check::ok("non-empty content")
            };
            report.correctness = if low.contains("pong") {
                Check::ok(format!(
                    "said pong ({} out tok)",
                    usage.usage.output_tokens.unwrap_or(0)
                ))
            } else {
                Check::fail(format!("no 'pong' in: {:?}", truncate(&text, 80)))
            };
        }
        Err(e) => {
            report.correctness = Check::fail(format!("complete error: {e}"));
            report.thinking_off = Check::fail("not evaluated");
        }
    }

    // 3. Tool calling.
    let msgs = vec![Message::user()
        .with_text("What is the weather in Paris? Use the get_weather tool to find out.")];
    match provider
        .complete(
            "You are an AI agent. Always use the provided tools to answer questions.",
            &msgs,
            &[weather_tool()],
        )
        .await
    {
        Ok((message, _)) => {
            report.tool_calling = if has_paris_weather_request(&message) {
                Check::ok("emitted a valid get_weather tool request for Paris")
            } else {
                Check::fail(format!(
                    "no valid get_weather call for Paris; said: {:?}",
                    truncate(&message.as_concat_text(), 80)
                ))
            };
        }
        Err(e) => report.tool_calling = Check::fail(format!("error: {e}")),
    }

    // 3b. Bootstrap-sized prompt: the model must still make a STRUCTURED call
    //     from the end of a large prompt (a truncated prompt is what made
    //     models write tool calls as JSON text), and this times the full completion.
    let system = format!(
        "You are an AI agent. Always use the provided tools to answer questions.\n\n{}",
        "The clinic's supply tracker records each delivery with a batch number, the \
         receiving nurse, the storage room and the expiry date. Items below their reorder \
         threshold are flagged in the weekly summary. "
            .repeat(1000)
    );
    let msgs = vec![Message::user()
        .with_text("What is the weather in Paris? Use the get_weather tool to find out.")];
    let t0 = Instant::now();
    match provider.complete(&system, &msgs, &[weather_tool()]).await {
        Ok((message, usage)) => {
            let secs = t0.elapsed().as_secs_f64();
            let input = prompt_tokens(&usage.usage);
            report.bootstrap = if input <= 25_000 {
                Check::fail(format!(
                    "the prompt was only {input} tokens; expected more than 25000"
                ))
            } else if has_paris_weather_request(&message) {
                Check::ok(format!(
                    "{input} prompt tokens in {secs:.1}s (~{:.0} prompt tok/s end-to-end)",
                    input as f64 / secs
                ))
            } else {
                Check::fail(format!(
                    "{input} prompt tokens, no valid get_weather call for Paris; said: {:?}",
                    truncate(&message.as_concat_text(), 80)
                ))
            };
        }
        Err(e) => report.bootstrap = Check::fail(format!("error: {e}")),
    }

    // 4. Streaming.
    let msgs = vec![Message::user().with_text("Count from 1 to 5, digits only.")];
    match provider
        .stream("You are a terse assistant.", &msgs, &[])
        .await
    {
        Ok(mut stream) => {
            let mut chunks = 0usize;
            let mut text = String::new();
            let mut stream_err = None;
            while let Some(item) = stream.next().await {
                match item {
                    Ok((Some(m), _, _)) => {
                        text.push_str(&m.as_concat_text());
                        chunks += 1;
                    }
                    Ok((None, _, _)) => chunks += 1,
                    Err(e) => {
                        stream_err = Some(e.to_string());
                        break;
                    }
                }
            }
            report.streaming = match stream_err {
                Some(e) => Check::fail(format!("stream error: {e}")),
                None if chunks > 1 => Check::ok(format!("{chunks} chunks")),
                None => Check::fail(format!("only {chunks} chunk(s)")),
            };
        }
        Err(e) => report.streaming = Check::fail(format!("stream start error: {e}")),
    }

    // 5. Speed: generate a chunk of text, tokens/sec from reported output tokens.
    let msgs = vec![Message::user()
        .with_text("Write a single detailed paragraph (about 120 words) explaining what mitochondria do in a cell.")];
    let t0 = Instant::now();
    match provider
        .complete("You are a helpful science tutor.", &msgs, &[])
        .await
    {
        Ok((_message, usage)) => {
            let elapsed = t0.elapsed().as_secs_f64();
            let out = usage.usage.output_tokens.unwrap_or(0).max(0) as f64;
            if out > 0.0 && elapsed > 0.0 {
                let tps = out / elapsed;
                report.speed_toks_per_sec = Some(tps);
                report.speed = Check::ok(format!("{tps:.1} tok/s ({out:.0} tok in {elapsed:.1}s)"));
            } else {
                report.speed = Check::fail("no output tokens reported");
            }
        }
        Err(e) => report.speed = Check::fail(format!("error: {e}")),
    }
}

fn truncate(s: &str, n: usize) -> String {
    let s = s.trim().replace('\n', " ");
    if s.chars().count() <= n {
        s
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

fn report_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("LLAMACPP_SURVEY_OUT") {
        return std::path::PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    std::path::PathBuf::from(home).join("Desktop/llamacpp-survey-report.md")
}

fn write_report(reports: &[ModelReport], in_progress: Option<&str>, selected_count: usize) {
    let mut md = String::new();
    md.push_str("# Llama Server: Embedded llama.cpp Model Survey\n\n");
    md.push_str(
        "Engine: Biorouter managed `llama-server` sidecar (build pinned). \
         Each model tested through the real `LlamaCppProvider` (thinking off, \
         q8_0 KV, one slot, the memory-tiered 64k/128k window).\n\n",
    );
    if let Some(m) = in_progress {
        md.push_str(&format!("> ⏳ Currently testing: **{m}** …\n\n"));
    } else {
        let failed_checks: Vec<_> = reports
            .iter()
            .flat_map(ModelReport::failed_checks)
            .collect();
        let result = if reports.len() < selected_count {
            "IN PROGRESS"
        } else if !reports.is_empty() && failed_checks.is_empty() {
            "PASS"
        } else {
            "FAIL"
        };
        md.push_str(&format!(
            "Overall result: **{result}** ({} of {selected_count} selected model(s), {} failed checks).\n\n",
            reports.len(),
            failed_checks.len()
        ));
    }

    md.push_str("## Summary\n\n");
    md.push_str(
        "| Model | Size | Load(s) | Avail | Correct | Think-off | Tools | 30k-prompt tools | Stream | tok/s |\n",
    );
    md.push_str(
        "|-------|------|--------:|:-----:|:-------:|:---------:|:-----:|:----------------:|:------:|------:|\n",
    );
    for r in reports {
        md.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            r.name,
            r.download_size,
            r.load_secs
                .map(|s| format!("{s:.0}"))
                .unwrap_or_else(|| "—".into()),
            r.availability.mark(),
            r.correctness.mark(),
            r.thinking_off.mark(),
            r.tool_calling.mark(),
            r.bootstrap.mark(),
            r.streaming.mark(),
            r.speed_toks_per_sec
                .map(|t| format!("{t:.1}"))
                .unwrap_or_else(|| "—".into()),
        ));
    }

    md.push_str("\n## Details\n\n");
    for r in reports {
        md.push_str(&format!("### {} (`{}`)\n\n", r.display_name, r.name));
        md.push_str(&format!(
            "- HF spec: `{}`, {}, native context {}, allocated context {}\n",
            r.hf_spec,
            r.download_size,
            r.native_context_limit,
            r.allocated_context_limit
                .map(|limit| limit.to_string())
                .unwrap_or_else(|| "not reported".into())
        ));
        if let Some(s) = r.load_secs {
            md.push_str(&format!("- Load to Ready: {s:.1}s\n"));
        }
        md.push_str(&format!(
            "- {} Availability: {}\n",
            r.availability.mark(),
            r.availability.detail
        ));
        md.push_str(&format!(
            "- {} Correctness: {}\n",
            r.correctness.mark(),
            r.correctness.detail
        ));
        md.push_str(&format!(
            "- {} Thinking-off: {}\n",
            r.thinking_off.mark(),
            r.thinking_off.detail
        ));
        md.push_str(&format!(
            "- {} Tool calling: {}\n",
            r.tool_calling.mark(),
            r.tool_calling.detail
        ));
        md.push_str(&format!(
            "- {} Tool call from a ~30k-token prompt: {}\n",
            r.bootstrap.mark(),
            r.bootstrap.detail
        ));
        md.push_str(&format!(
            "- {} Streaming: {}\n",
            r.streaming.mark(),
            r.streaming.detail
        ));
        md.push_str(&format!(
            "- {} Speed: {}\n\n",
            r.speed.mark(),
            r.speed.detail
        ));
    }

    let path = report_path();
    std::fs::write(&path, md)
        .unwrap_or_else(|error| panic!("failed to write report to {}: {error}", path.display()));
}

fn selected_models(filter: Option<&str>) -> Vec<&'static CatalogEntry> {
    let entries: Vec<_> = match filter {
        Some(names) => names
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(|name| {
                MODEL_CATALOG
                    .iter()
                    .find(|entry| entry.name == name)
                    .unwrap_or_else(|| panic!("unknown LLAMACPP_SURVEY_MODELS model: {name:?}"))
            })
            .collect(),
        None => MODEL_CATALOG.iter().collect(),
    };
    assert!(
        !entries.is_empty(),
        "LLAMACPP_SURVEY_MODELS must select at least one model"
    );
    entries
}

#[test]
#[should_panic(expected = "LLAMACPP_SURVEY_MODELS must select at least one model")]
fn empty_model_selection_is_rejected() {
    selected_models(Some(" , , "));
}

#[test]
#[should_panic(expected = "unknown LLAMACPP_SURVEY_MODELS model")]
fn unknown_model_selection_is_rejected() {
    selected_models(Some("qwen3.8-27b, nonexistent-test-model"));
}

#[tokio::test]
#[ignore = "downloads and loads every catalog model (~37 GB); run explicitly"]
async fn survey_all_models() {
    let entries = selected_models(std::env::var("LLAMACPP_SURVEY_MODELS").ok().as_deref());
    let selected_count = entries.len();
    println!("Surveying {selected_count} model(s)");
    let mut reports: Vec<ModelReport> = Vec::new();

    for entry in entries {
        let mut report = ModelReport {
            name: entry.name.to_string(),
            display_name: entry.display_name.to_string(),
            hf_spec: entry.hf_spec.to_string(),
            download_size: entry.download_size.to_string(),
            native_context_limit: entry.context_limit,
            ..Default::default()
        };
        println!(
            "\n=== {} ({}): {} ===",
            entry.display_name, entry.name, entry.download_size
        );

        // Push a placeholder so the in-progress report shows this model.
        reports.push(std::mem::take(&mut report));
        let idx = reports.len() - 1;
        write_report(&reports, Some(entry.name), selected_count);

        // 1. Availability: ensure + wait_ready, timing the download+load.
        let source = match resolve_model_source(entry.name) {
            Ok(s) => s,
            Err(e) => {
                reports[idx].availability = Check::fail(format!("resolve_model_source: {e}"));
                write_report(&reports, None, selected_count);
                continue;
            }
        };
        let t0 = Instant::now();
        match global().ensure(entry.name, &source).await {
            Ok(_port) => match global().wait_ready(ready_timeout()).await {
                Ok(_) => {
                    let load = t0.elapsed().as_secs_f64();
                    reports[idx].load_secs = Some(load);
                    let st = global().status().await;
                    if st.state == SidecarState::Ready {
                        reports[idx].allocated_context_limit = st.context_size;
                        reports[idx].availability = Check::ok(format!("Ready in {load:.1}s"));
                        println!("  ready in {load:.1}s; running battery…");
                        run_battery(&mut reports[idx]).await;
                    } else {
                        reports[idx].availability =
                            Check::fail(format!("state={:?} detail={:?}", st.state, st.detail));
                    }
                }
                Err(e) => reports[idx].availability = Check::fail(format!("not ready: {e}")),
            },
            Err(e) => reports[idx].availability = Check::fail(format!("ensure failed: {e}")),
        }

        // Free the multi-GB model from RAM before the next one.
        global().stop().await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        write_report(&reports, None, selected_count);
        println!("  done: {}", summarize(&reports[idx]));
    }

    write_report(&reports, None, selected_count);
    println!("\nReport written to {}", report_path().display());
    let failures: Vec<_> = reports
        .iter()
        .flat_map(ModelReport::failed_checks)
        .collect();
    assert!(
        failures.is_empty(),
        "{} survey check(s) failed; report saved at {}:\n{}",
        failures.len(),
        report_path().display(),
        failures.join("\n")
    );
    println!(
        "All {} model(s) passed all seven survey checks.",
        reports.len()
    );
}

fn summarize(r: &ModelReport) -> String {
    format!(
        "avail={} correct={} think-off={} tools={} bootstrap={} stream={} speed={}",
        r.availability.mark(),
        r.correctness.mark(),
        r.thinking_off.mark(),
        r.tool_calling.mark(),
        r.bootstrap.mark(),
        r.streaming.mark(),
        r.speed_toks_per_sec
            .map(|t| format!("{t:.1}t/s"))
            .unwrap_or_else(|| "—".into()),
    )
}

#[test]
fn weather_request_requires_a_valid_call_for_paris() {
    let call = |name: &str, location: serde_json::Value| {
        Message::assistant().with_tool_request(
            "weather",
            Ok(rmcp::model::CallToolRequestParams {
                task: None,
                name: name.to_string().into(),
                arguments: Some(object!({ "location": location })),
                meta: None,
            }),
        )
    };
    for location in [
        "Paris",
        " Paris, France ",
        "PARIS FRANCE",
        "Paris, Île-de-France",
    ] {
        assert!(has_paris_weather_request(&call(
            "get_weather",
            location.into()
        )));
    }
    for location in ["London", "Parish", "London and Paris"] {
        assert!(!has_paris_weather_request(&call(
            "get_weather",
            location.into()
        )));
    }
    assert!(!has_paris_weather_request(&call(
        "another_tool",
        "Paris".into()
    )));
    assert!(!has_paris_weather_request(&call("get_weather", 42.into())));
    let missing = Message::assistant().with_tool_request(
        "weather",
        Ok(rmcp::model::CallToolRequestParams {
            task: None,
            name: "get_weather".into(),
            arguments: None,
            meta: None,
        }),
    );
    assert!(!has_paris_weather_request(&missing));
    let invalid = Message::assistant().with_tool_request(
        "weather",
        Err(rmcp::model::ErrorData::internal_error(
            "invalid tool call",
            None,
        )),
    );
    assert!(!has_paris_weather_request(&invalid));
    assert!(!has_paris_weather_request(&Message::assistant().with_text(
        r#"{"name":"get_weather","arguments":{"location":"Paris"}}"#,
    )));
}

#[test]
fn prompt_count_includes_cached_input_tokens() {
    let usage = Usage {
        input_tokens: Some(1_000),
        cache_read_input_tokens: Some(30_000),
        output_tokens: Some(50),
        ..Usage::default()
    };
    assert_eq!(prompt_tokens(&usage), 31_000);
    assert!(prompt_tokens(&usage) > 25_000);
    assert_eq!(prompt_tokens(&Usage::default()), 0);
}

#[test]
fn every_reported_check_is_required_for_survey_success() {
    fn passed_report() -> ModelReport {
        ModelReport {
            name: "fixture".into(),
            availability: Check::ok("ready"),
            correctness: Check::ok("pong"),
            thinking_off: Check::ok("answer"),
            tool_calling: Check::ok("valid weather call"),
            bootstrap: Check::ok("31000 prompt tokens"),
            streaming: Check::ok("multiple chunks"),
            speed: Check::ok("tokens reported"),
            ..ModelReport::default()
        }
    }
    assert!(passed_report().failed_checks().is_empty());
    let failures: [fn(&mut ModelReport); 7] = [
        |report| report.availability = Check::fail("not ready"),
        |report| report.correctness = Check::fail("wrong answer"),
        |report| report.thinking_off = Check::fail("empty answer"),
        |report| report.tool_calling = Check::fail("wrong tool"),
        |report| report.bootstrap = Check::fail("prompt too small"),
        |report| report.streaming = Check::fail("truncated stream"),
        |report| report.speed = Check::fail("no usage"),
    ];
    for fail_one in failures {
        let mut report = passed_report();
        fail_one(&mut report);
        assert_eq!(report.failed_checks().len(), 1);
    }
}
