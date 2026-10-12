//! Live integration tests for the Llama Server provider + managed sidecar.
//!
//! These exercise a real llama-server process with a real model, so they are
//! `#[ignore]`d by default. Run them explicitly, one at a time (they share the
//! process-wide sidecar):
//!
//! ```sh
//! BIOROUTER_LLAMACPP_BIN=ui/desktop/src/bin/llamacpp/llama-server \
//!   cargo test -p biorouter --test llamacpp_integration -- --ignored --test-threads=1
//! ```
//!
//! They run the catalog's default model, Qwen3.8 27B: the first run downloads
//! its ~19 GB GGUF from Hugging Face into the shared model store (or reuses an
//! `ollama pull qwen3.8:27b-q4_K_M`), and later runs start from the file on
//! disk. `LLAMACPP_TEST_MODEL` points them at another catalog name or raw
//! `owner/repo:QUANT` spec — `gemma4-31b`, or a small GGUF when only the
//! protocol is under test.
//!
//! Beyond the round trips, these pin the failures the 2026-10 investigation of
//! the "local model writes JSON instead of calling tools" report found, none of
//! which a terse prompt and one tool could reach: a real agent bootstrap is
//! ~31-40k tokens, two requests can arrive at once, an over-long prompt must be
//! a context error compaction can act on, and the server must not answer an
//! unauthenticated caller.

use std::time::{Duration, Instant};

use biorouter::agents::effort::ReasoningEffort;
use biorouter::conversation::message::{Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::providers::base::{Provider, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::providers::llamacpp::{resolve_chat_model_source, LlamaCppProvider, DEFAULT_MODEL};
use biorouter::providers::llamacpp_sidecar::{current_api_key, global, SidecarState};
use futures::StreamExt;
use rmcp::{model::Tool, object};

fn test_model() -> String {
    let model = std::env::var("LLAMACPP_TEST_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    selected_model(&model)
}

fn selected_model(model: &str) -> String {
    let model = model.trim().to_string();
    assert!(!model.is_empty(), "LLAMACPP_TEST_MODEL must name a model");
    let (resolved, _) = resolve_chat_model_source(&model)
        .unwrap_or_else(|error| panic!("invalid LLAMACPP_TEST_MODEL {model:?}: {error}"));
    assert_eq!(
        resolved, model,
        "LLAMACPP_TEST_MODEL must select the model actually served, not a retired alias"
    );
    model
}

#[test]
#[should_panic(expected = "LLAMACPP_TEST_MODEL must name a model")]
fn empty_model_selection_is_rejected() {
    selected_model("   ");
}

#[test]
#[should_panic(expected = "invalid LLAMACPP_TEST_MODEL")]
fn unknown_model_selection_is_rejected() {
    selected_model("nonexistent-test-model");
}

#[test]
#[should_panic(expected = "must select the model actually served")]
fn retired_model_selection_is_rejected() {
    selected_model("gemma4-12b");
}

fn weather_tool() -> Tool {
    Tool::new(
        "get_weather".to_string(),
        "Get current temperature for a given location.".to_string(),
        object!({
            "type": "object",
            "required": ["location"],
            "properties": {
                "location": {"type": "string"}
            }
        }),
    )
}

/// Thirty plausible tools, so the schema block resembles Biorouter's own.
fn tool_roster() -> Vec<Tool> {
    let mut tools: Vec<Tool> = (0..30)
        .map(|i| {
            Tool::new(
                format!("lab_records_tool_{i:02}"),
                format!(
                    "Look up laboratory record category {i} for a patient cohort. Returns the \
                     matching rows with units, reference ranges and the collection timestamp; \
                     use it only when the user asks about category {i}."
                ),
                object!({
                    "type": "object",
                    "required": ["cohort_id"],
                    "properties": {
                        "cohort_id": {"type": "string", "description": "The cohort identifier"},
                        "limit": {"type": "integer", "description": "Maximum rows to return"},
                        "since": {"type": "string", "description": "ISO-8601 start date"}
                    }
                }),
            )
        })
        .collect();
    tools.push(weather_tool());
    tools
}

/// A system prompt of roughly `tokens` tokens: instructions, then reference
/// material the model has to carry without being derailed by it.
fn bootstrap_system_prompt(tokens: usize) -> String {
    let mut prompt = String::from(
        "You are Biorouter, an AI agent. Always use the provided tools to answer questions \
         that need data; never describe a tool call in text instead of making it.\n\n\
         # Reference notes\n",
    );
    let paragraph = "The clinic's supply tracker records each delivery with a batch number, \
        the receiving nurse, the storage room and the expiry date. Items below their reorder \
        threshold are flagged in the weekly summary, and controlled substances additionally \
        require a second signature at intake and a reconciliation at the end of each shift. ";
    // ~70 tokens per paragraph with the Qwen and Gemma tokenizers.
    let mut i = 0;
    while prompt.len() < tokens * 4 {
        prompt.push_str(&format!("\n## Note {i}\n{paragraph}"));
        i += 1;
    }
    prompt
}

async fn provider_with(max_tokens: i32) -> LlamaCppProvider {
    let model = ModelConfig::new(&test_model())
        .unwrap()
        .with_max_tokens(Some(max_tokens));
    LlamaCppProvider::from_env(model).await.unwrap()
}

async fn provider() -> LlamaCppProvider {
    provider_with(200).await
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

#[tokio::test]
#[ignore = "spawns a real llama-server and may download a model"]
async fn chat_completion_roundtrip() {
    let provider = provider().await;
    let messages = vec![Message::user().with_text("Reply with exactly the word: pong")];
    let (message, usage) = provider
        .complete("You are a terse assistant.", &messages, &[])
        .await
        .expect("completion should succeed");

    let text = message.as_concat_text();
    assert!(
        text.to_lowercase().contains("pong"),
        "expected pong, got: {text:?}"
    );
    assert!(
        usage.usage.total_tokens.unwrap_or(0) > 0,
        "expected token usage to be reported"
    );
    println!("completion: {text}");
}

#[tokio::test]
#[ignore = "spawns a real llama-server and may download a model"]
async fn streaming_completion_works() {
    let provider = provider().await;
    let messages = vec![Message::user().with_text("Count from 1 to 5, digits only.")];
    let mut stream = provider
        .stream("You are a terse assistant.", &messages, &[])
        .await
        .expect("stream should start");

    let mut chunks = 0;
    let mut text = String::new();
    while let Some(item) = stream.next().await {
        let (message, _usage, _pending) = item.expect("stream item should be ok");
        if let Some(message) = message {
            text.push_str(&message.as_concat_text());
        }
        chunks += 1;
    }
    assert!(
        chunks > 1,
        "expected multiple streamed chunks, got {chunks}"
    );
    assert!(!text.is_empty(), "expected streamed text");
    println!("streamed {chunks} chunks: {text}");
}

#[tokio::test]
#[ignore = "spawns a real llama-server and may download a model"]
async fn tool_calling_emits_tool_request() {
    let provider = provider().await;
    let messages = vec![Message::user()
        .with_text("What is the weather in Paris? Use the get_weather tool to find out.")];
    let (message, _usage) = provider
        .complete(
            "You are an AI agent. Always use the provided tools to answer questions.",
            &messages,
            &[weather_tool()],
        )
        .await
        .expect("tool-call completion should succeed");

    assert!(
        has_paris_weather_request(&message),
        "expected a valid get_weather tool call for Paris, got: {:?}",
        message.content
    );
}

/// A prompt the size of Biorouter's real first turn (~31k tokens with the
/// built-in extensions) fits the default window, and the model still makes a
/// STRUCTURED tool call from the end of it. The 32k tier refused this prompt
/// outright; a silently truncated one is what made models write tool calls as
/// JSON text.
#[tokio::test]
#[ignore = "spawns a real llama-server and may download a model"]
async fn a_bootstrap_sized_prompt_fits_and_still_calls_tools() {
    let provider = provider_with(400).await;
    let system = bootstrap_system_prompt(30_000);
    let messages =
        vec![Message::user()
            .with_text("What is the weather in Paris right now? Use the right tool.")];
    let started = Instant::now();
    let (message, usage) = provider
        .complete(&system, &messages, &tool_roster())
        .await
        .expect("a bootstrap-sized prompt must fit the default window");
    let input = prompt_tokens(&usage.usage);
    println!(
        "prompt {input} tokens, answered in {:.1}s, window {:?}",
        started.elapsed().as_secs_f64(),
        provider.get_model_config().context_limit
    );
    assert!(input > 25_000, "the prompt was only {input} tokens");
    assert!(
        has_paris_weather_request(&message),
        "expected a valid get_weather tool call for Paris, got: {:?}",
        message.content
    );
    let window = provider.get_model_config().context_limit.unwrap_or(0);
    assert!(
        window >= 65_536,
        "the reported window ({window}) must be the server's, and never below 64k"
    );
}

/// LC-8. The provider asks for prefill progress on every stream, so a long
/// cold prefill keeps the client's idle timeout fed. Pins both halves: the
/// pinned build really sends `prompt_progress` frames while it prefills, and
/// a stream carrying them still decodes to the answer alone.
#[tokio::test]
#[ignore = "spawns a real llama-server and may download a model"]
async fn a_long_prefill_streams_progress_and_still_answers() {
    let provider = provider().await;
    let system = bootstrap_system_prompt(6_000);
    let mut stream = provider
        .stream(
            &system,
            &[Message::user()
                .with_text("In one word: what does the tracker record besides the nurse?")],
            &[],
        )
        .await
        .expect("stream should start");
    let mut text = String::new();
    while let Some(item) = stream.next().await {
        let (message, _usage, _pending) = item.expect("progress frames must not break the stream");
        if let Some(message) = message {
            text.push_str(&message.as_concat_text());
        }
    }
    assert!(!text.trim().is_empty(), "expected an answer");

    // The wire contract the provider relies on, without the decoder in between.
    let port = global().status().await.port.expect("a running sidecar");
    let key = current_api_key().expect("a key for the running sidecar");
    let body = serde_json::json!({
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": "Reply with one word."}
        ],
        "max_tokens": 4,
        "stream": true,
        "return_progress": true,
        "cache_prompt": false,
        "chat_template_kwargs": {"enable_thinking": false}
    });
    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let wire = response.text().await.unwrap();
    let frames = wire
        .lines()
        .filter(|line| line.contains("\"prompt_progress\""))
        .count();
    println!("answer {text:?}; {frames} prompt_progress frames for a ~6k-token prefill");
    assert!(frames >= 2, "expected prefill progress frames:\n{wire}");
}

/// llama-server's default of 4 slots sharing one KV cache made two
/// concurrent agent requests both fail with HTTP 500 "Context size has been
/// exceeded". With one slot the second queues behind the first.
#[tokio::test]
#[ignore = "spawns a real llama-server and may download a model"]
async fn concurrent_requests_queue_instead_of_failing() {
    let provider = provider_with(64).await;
    let system = bootstrap_system_prompt(20_000);
    let ask = |question: &'static str| {
        let provider = &provider;
        let system = &system;
        async move {
            provider
                .complete(system, &[Message::user().with_text(question)], &[])
                .await
        }
    };
    let (a, b) = tokio::join!(
        ask("In one word: what colour is the sky on a clear day?"),
        ask("In one word: what colour is fresh grass?")
    );
    let a = a.expect("the first concurrent request must succeed");
    let b = b.expect("the second concurrent request must queue and succeed");
    println!(
        "a: {:?} ({:?} in) / b: {:?} ({:?} in)",
        a.0.as_concat_text(),
        a.1.usage.input_tokens,
        b.0.as_concat_text(),
        b.1.usage.input_tokens
    );
}

/// The test above uses 64-token answers, so its queue wait is seconds and it
/// cannot see this: llama-server sends a queued request NOTHING — not even
/// headers — until its one slot frees up, and the client's idle bound runs
/// from the moment a request is sent. A request queued behind a generation
/// longer than the bound used to time out on a healthy server (and be re-sent
/// into the same queue). It now waits for the slot in-process, before it is
/// sent. The idle bound is pinned to 20 s so a long reply outlasts it in well
/// under a minute; at the default 600 s the same failure needs a 32 GiB
/// machine's ten-minute prefill.
#[tokio::test]
#[ignore = "spawns a real llama-server and may download a model"]
async fn a_request_queued_behind_a_long_generation_does_not_time_out() {
    const IDLE_SECS: u64 = 20;
    // Read when each provider's client is built, i.e. on its first request,
    // so it is held for the whole test.
    let _guard = env_lock::lock_env([
        ("BIOROUTER_HTTP_READ_TIMEOUT_SECS", Some("20")),
        ("LLAMACPP_PARALLEL", Some("1")),
        ("LLAMACPP_EXTRA_ARGS", Some("")),
    ]);
    global().stop().await;
    let long = provider_with(2500).await;
    let short = provider_with(16).await;
    let terse = "You are a terse assistant.";
    // Start the server first, so start-up is not part of anything measured.
    short
        .complete(terse, &[Message::user().with_text("Reply with OK.")], &[])
        .await
        .expect("warm-up");

    let mut stream = long
        .stream(
            terse,
            &[Message::user().with_text(
                "Write every whole number from 1 to 1000 in order, separated by single spaces, \
                 and nothing else.",
            )],
            &[],
        )
        .await
        .expect("the long reply starts");
    let started = Instant::now();
    let reader = async {
        let mut chunks = 0;
        while let Some(item) = stream.next().await {
            item.expect("the long reply streams to the end");
            chunks += 1;
        }
        (chunks, started.elapsed())
    };
    let asker = async {
        // Well inside the long reply, which holds the server's only slot.
        tokio::time::sleep(Duration::from_secs(2)).await;
        let asked = Instant::now();
        let answer = short
            .complete(
                terse,
                &[Message::user().with_text("In one word: what colour is fresh grass?")],
                &[],
            )
            .await;
        (answer, asked.elapsed())
    };
    let ((chunks, long_took), (answer, short_waited)) = tokio::join!(reader, asker);
    println!(
        "long reply: {chunks} chunks in {:.1}s; queued request answered after {:.1}s",
        long_took.as_secs_f64(),
        short_waited.as_secs_f64()
    );
    assert!(
        long_took > Duration::from_secs(IDLE_SECS + 5),
        "the long reply ({long_took:?}) must outlast the {IDLE_SECS}s idle bound, or this \
         proves nothing"
    );
    let (message, _) = answer.expect("the queued request must wait for the slot, not time out");
    assert!(!message.as_concat_text().trim().is_empty());
    assert!(
        short_waited > Duration::from_secs(IDLE_SECS),
        "it really did wait longer than the idle bound: {short_waited:?}"
    );
}

/// A thinking turn shows its reasoning (as one block, once the reasoning phase
/// ends), at both levels the provider sends. Normal now names `medium` (the template's own default is
/// `xhigh`, which made Normal and Deep one request), so this also pins that
/// both values are ones Qwen3.8's template takes: anything else is HTTP 500.
#[tokio::test]
#[ignore = "spawns a real llama-server and may download a model"]
async fn a_thinking_turn_streams_its_reasoning_at_either_level() {
    let _guard = env_lock::lock_env([("LLAMACPP_ENABLE_THINKING", Some("true"))]);
    for effort in [ReasoningEffort::Normal, ReasoningEffort::Deep] {
        let model = ModelConfig::new(&test_model())
            .unwrap()
            .with_max_tokens(Some(3000))
            .with_reasoning_effort(Some(effort));
        let provider = LlamaCppProvider::from_env(model).await.unwrap();
        let started = Instant::now();
        let mut stream = provider
            .stream(
                "You are a careful assistant.",
                &[Message::user().with_text("Is 91 prime? Answer yes or no.")],
                &[],
            )
            .await
            .unwrap_or_else(|e| panic!("{effort:?}: the template must take its level: {e}"));
        let mut thinking = String::new();
        let mut first_thought_at = None;
        let mut answer = String::new();
        while let Some(item) = stream.next().await {
            let (message, _usage, _pending) = item.expect("stream item should be ok");
            for content in message.iter().flat_map(|m| m.content.iter()) {
                match content {
                    MessageContent::Thinking(t) => {
                        first_thought_at.get_or_insert(started.elapsed());
                        thinking.push_str(&t.thinking);
                    }
                    MessageContent::Text(t) => answer.push_str(&t.text),
                    _ => {}
                }
            }
        }
        println!(
            "{effort:?}: {} chars of thinking (first shown after {first_thought_at:?}), \
             answer {answer:?}, {:.1}s",
            thinking.len(),
            started.elapsed().as_secs_f64()
        );
        assert!(
            !thinking.trim().is_empty(),
            "{effort:?}: the reasoning is shown"
        );
        assert!(
            answer.to_lowercase().contains("no"),
            "{effort:?}: {answer:?}"
        );
    }
}

/// The sidecar is loopback-only AND keyed: a web page the user opens can reach
/// loopback, and llama-server reflects any `Origin` in CORS.
#[tokio::test]
#[ignore = "spawns a real llama-server and may download a model"]
async fn the_sidecar_refuses_unauthenticated_requests() {
    let provider = provider().await;
    provider
        .complete(
            "You are a terse assistant.",
            &[Message::user().with_text("Reply with OK.")],
            &[],
        )
        .await
        .expect("the provider authenticates itself");
    let port = global().status().await.port.expect("a running sidecar");
    let client = reqwest::Client::new();
    let url = |path: &str| format!("http://127.0.0.1:{port}/{path}");
    let body =
        serde_json::json!({"messages": [{"role": "user", "content": "hi"}], "max_tokens": 1});

    let anonymous = client
        .post(url("v1/chat/completions"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(anonymous.status(), 401);
    let wrong = client
        .post(url("v1/chat/completions"))
        .bearer_auth("not-the-key")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401);
    for path in ["props", "v1/models"] {
        let status = client.get(url(path)).send().await.unwrap().status();
        assert_eq!(status, 401, "/{path} must need the key");
    }
    // `/health` stays public: it is how readiness is polled.
    assert_eq!(
        client.get(url("health")).send().await.unwrap().status(),
        200
    );
    let key = current_api_key().expect("a key for the running sidecar");
    let props = client
        .get(url("props"))
        .bearer_auth(key)
        .send()
        .await
        .unwrap();
    assert_eq!(props.status(), 200);
}

/// An over-window prompt is a 400 `exceed_context_size_error`, and it must
/// reach the agent loop as `ContextLengthExceeded` — the error compaction acts
/// on — not as a plain bad request that ends the turn.
#[tokio::test]
#[ignore = "spawns a real llama-server and may download a model"]
async fn an_over_window_prompt_is_a_context_error() {
    let _guard = env_lock::lock_env([("LLAMACPP_CONTEXT_SIZE", Some("8192"))]);
    let provider = provider().await;
    let result = provider
        .complete(
            &bootstrap_system_prompt(12_000),
            &[Message::user().with_text("Summarise the notes.")],
            &[],
        )
        .await;
    // Restart at the default window for whatever runs next.
    global().stop().await;
    match result {
        Err(ProviderError::ContextLengthExceeded(message)) => println!("context error: {message}"),
        other => panic!("expected ContextLengthExceeded, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "spawns a real llama-server and may download a model"]
async fn switching_models_restarts_sidecar() {
    let (name, source) = resolve_chat_model_source(&test_model()).unwrap();
    let port_a = global().ensure(&name, &source).await.unwrap();
    global()
        .wait_ready(Duration::from_secs(3600))
        .await
        .unwrap();

    // Re-ensuring the same model must keep the same process/port.
    let port_b = global().ensure(&name, &source).await.unwrap();
    assert_eq!(port_a, port_b);

    let status = global().status().await;
    assert_eq!(status.state, SidecarState::Ready);
    assert_eq!(status.model.as_deref(), Some(name.as_str()));
    let window = status.context_size.expect("the live window is read back");
    assert!([65_536, 131_072].contains(&window), "{window}");

    global().stop().await;
    let status = global().status().await;
    assert_eq!(status.state, SidecarState::Stopped);
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
