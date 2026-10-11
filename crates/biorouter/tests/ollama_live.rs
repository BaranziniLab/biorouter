//! Live checks for the Ollama provider against a real Ollama server.
//!
//! They need a running Ollama with the supported models pulled, so they are
//! `#[ignore]`d (CI has no Ollama, and runs no ignored test). Run them one at a
//! time so model loads and context allocations cannot overlap:
//!
//! ```sh
//! cargo test -p biorouter --test ollama_live -- --ignored --test-threads=1
//! ```
//!
//! `OLLAMA_HOST` picks the server, as it does for the provider.
//! `OLLAMA_TEST_MODELS` (comma-separated) replaces the default list, which is
//! the two models BioRouter supports on Ollama, `qwen3.8:27b` and `gemma4:31b`.
//! `OLLAMA_TEST_LEAKING_MODEL` names the model the text-recovery check runs
//! against (default `qwen2.5-coder:7b`, which must be pulled for it).
//! The provider reads its settings through `Config::global()`, so point
//! `BIOROUTER_PATH_ROOT` at a scratch directory when you do not want it to read
//! your own `config.yaml`.

use biorouter::conversation::message::{Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::providers::base::Provider;
use biorouter::providers::ollama::{OllamaProvider, OLLAMA_RECOMMENDED_MODELS};
use biorouter::providers::provider_test::test_provider_configuration;

fn selected_models(list: Option<&str>) -> Vec<String> {
    let models: Vec<String> = match list {
        Some(list) => list
            .split(',')
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(str::to_string)
            .collect(),
        None => OLLAMA_RECOMMENDED_MODELS
            .iter()
            .map(|model| model.tag.to_string())
            .collect(),
    };
    assert!(
        !models.is_empty(),
        "OLLAMA_TEST_MODELS must select at least one model"
    );
    models
}

fn test_models() -> Vec<String> {
    selected_models(std::env::var("OLLAMA_TEST_MODELS").ok().as_deref())
}

#[test]
#[should_panic(expected = "OLLAMA_TEST_MODELS must select at least one model")]
fn empty_model_selection_is_rejected() {
    selected_models(Some(" , , "));
}

/// `biorouter configure` saves a model only if this probe passes: one
/// completion with a 50-token budget and a weather tool. Ollama thinks when
/// `think` is omitted, so on both supported models the probe used to spend its
/// 50 tokens reasoning, come back with no answer, and fail the provider's
/// answered-or-error guard — `configure` then restored the previous provider
/// and the supported default could not be chosen.
#[tokio::test]
#[ignore = "needs a local Ollama with the supported models pulled"]
async fn the_configure_probe_passes_on_each_supported_model() {
    for model in test_models() {
        test_provider_configuration("ollama", &model, false, None)
            .await
            .unwrap_or_else(|error| panic!("the configure probe failed for {model}: {error:#}"));
    }
}

/// A blocking completion — the call compaction, knowledge macros and session
/// titles make — streams and comes back as one message with the server's
/// usage, the whole prompt counted.
#[tokio::test]
#[ignore = "needs a local Ollama with the supported models pulled"]
async fn a_blocking_completion_comes_back_as_one_answered_message() {
    for model in test_models() {
        let provider = OllamaProvider::from_env(ModelConfig::new(&model).unwrap())
            .await
            .unwrap();
        let (message, usage) = provider
            .complete(
                "You are terse.",
                &[Message::user().with_text("Reply with exactly the word: ready")],
                &[],
            )
            .await
            .unwrap_or_else(|error| panic!("{model}: {error}"));
        let text = message.as_concat_text();
        assert!(text.to_lowercase().contains("ready"), "{model}: {text:?}");
        let prompt = usage.usage.input_tokens.unwrap_or(0)
            + usage.usage.cache_read_input_tokens.unwrap_or(0);
        assert!(prompt > 10, "{model}: the prompt was counted: {usage:?}");
        assert!(
            provider.get_model_config().context_limit.is_some(),
            "{model}: the window chosen for the request is the one reported"
        );
    }
}

/// The bug report's model writes every tool call as JSON TEXT through Ollama
/// (qwen2.5-coder:7b: 0 of 12 calls structured on Ollama 0.40.2). The provider
/// must hand the agent a tool call, recovered from that text, and never the
/// JSON as an answer.
#[tokio::test]
#[ignore = "needs a local Ollama with qwen2.5-coder:7b (or OLLAMA_TEST_LEAKING_MODEL) pulled"]
async fn a_call_written_as_text_reaches_the_agent_as_a_tool_call() {
    use biorouter::providers::formats::text_tool_calls::is_recovered;
    use futures::StreamExt;
    use rmcp::model::Tool;

    let model = std::env::var("OLLAMA_TEST_LEAKING_MODEL")
        .unwrap_or_else(|_| "qwen2.5-coder:7b".to_string());
    let shell = Tool::new(
        "developer__shell".to_string(),
        "Run a shell command on the user's machine and return stdout/stderr.".to_string(),
        rmcp::model::object(serde_json::json!({
            "type": "object",
            "properties": {"command": {"type": "string", "description": "The command to run"}},
            "required": ["command"]
        })),
    );
    let provider = OllamaProvider::from_env(ModelConfig::new(&model).unwrap())
        .await
        .unwrap();
    let mut stream = provider
        .stream(
            "You are Biorouter. Use the provided tools when they help.",
            &[Message::user().with_text("List the files in /tmp using the shell tool.")],
            std::slice::from_ref(&shell),
        )
        .await
        .unwrap_or_else(|error| panic!("{model}: {error}"));
    let mut text = String::new();
    let mut calls = Vec::new();
    while let Some(item) = stream.next().await {
        let (message, _, _) = item.unwrap_or_else(|error| panic!("{model}: {error}"));
        for content in message.into_iter().flat_map(|message| message.content) {
            match content {
                MessageContent::Text(delta) => text.push_str(&delta.text),
                MessageContent::ToolRequest(request) => calls.push(request),
                _ => {}
            }
        }
    }
    assert!(
        !text.contains("\"name\""),
        "{model}: the JSON reached the chat: {text}"
    );
    let call = calls
        .first()
        .unwrap_or_else(|| panic!("{model}: no tool call; it answered {text:?}"));
    let params = call.tool_call.as_ref().expect("a valid call");
    assert_eq!(params.name, "developer__shell", "{model}");
    // Structured or recovered, both are fine; say which, for the record.
    eprintln!("{model}: recovered from text = {}", is_recovered(call));
}
