biorouter::keep_this_test_binary_off_the_os_credential_store!();

use biorouter::conversation::message::{Message, MessageContent};
use biorouter::conversation::{fix_conversation, Conversation};
use biorouter::model::ModelConfig;
use biorouter::providers::base::Provider;
use biorouter::providers::formats::google::{
    create_request, get_thought_signature, response_to_message, response_to_streaming_message,
};
use biorouter::providers::google::{
    GoogleProvider, GOOGLE_DEFAULT_FAST_MODEL, GOOGLE_DEFAULT_MODEL, GOOGLE_KNOWN_MODELS,
};
use biorouter::providers::pricing::provider_model_pricing;
use futures::StreamExt;
use rmcp::model::{CallToolResult, Content};
use serde_json::{json, Value};

fn response(model: &str, parts: Vec<Value>) -> Value {
    json!({
        "modelVersion": model,
        "candidates": [{"content": {"role": "model", "parts": parts}}]
    })
}

fn tool_result() -> CallToolResult {
    CallToolResult {
        content: vec![Content::text("complete")],
        structured_content: None,
        is_error: Some(false),
        meta: None,
    }
}

#[test]
fn gemini4_catalogue_starts_with_the_recommended_daily_model() {
    assert_eq!(GOOGLE_DEFAULT_MODEL, "gemini-4-hydrogen");
    assert_eq!(
        &GOOGLE_KNOWN_MODELS[..3],
        &["gemini-4-hydrogen", "gemini-4-argon", "gemini-4-carbon"]
    );
    assert_eq!(
        GoogleProvider::metadata().default_model,
        GOOGLE_DEFAULT_MODEL
    );
    assert_eq!(GOOGLE_DEFAULT_FAST_MODEL, "gemini-3.5-flash-lite");
}

#[test]
fn gemini4_neon_is_offered_without_changing_the_recommended_default() {
    assert_eq!(GOOGLE_DEFAULT_MODEL, "gemini-4-hydrogen");
    assert_eq!(GOOGLE_DEFAULT_FAST_MODEL, "gemini-3.5-flash-lite");
    assert_eq!(GOOGLE_KNOWN_MODELS.first(), Some(&GOOGLE_DEFAULT_MODEL));
    assert_eq!(
        GOOGLE_KNOWN_MODELS
            .iter()
            .filter(|name| **name == "gemini-4-neon")
            .count(),
        1
    );
    let metadata = GoogleProvider::metadata();
    assert_eq!(metadata.default_model, GOOGLE_DEFAULT_MODEL);
    let neon = metadata
        .known_models
        .iter()
        .find(|model| model.name == "gemini-4-neon")
        .expect("the direct Google catalogue includes Neon");
    assert_eq!(neon.supports_vision, Some(true));
}

#[test]
fn gemini4_neon_has_its_exact_direct_context_window() {
    assert_eq!(
        ModelConfig::registry_context_window("gemini-4-neon"),
        Some(1_048_576)
    );
    assert_eq!(
        ModelConfig::new_or_fail("gemini-4-neon").context_limit(),
        1_048_576
    );
}

#[test]
fn gemini4_neon_has_its_direct_standard_and_implicit_cache_prices() {
    let pricing =
        provider_model_pricing("google", "gemini-4-neon").expect("Neon has a direct Google price");
    assert!((pricing.input_token_cost * 1e6 - 3.0).abs() < 1e-12);
    assert!((pricing.output_token_cost * 1e6 - 15.0).abs() < 1e-12);
    assert!((pricing.cache_read_cost.unwrap() * 1e6 - 0.15).abs() < 1e-12);
    assert_eq!(pricing.cache_write_cost, None);
    assert_eq!(pricing.context_length, Some(1_048_576));
}

#[test]
fn gemini4_replays_all_prior_call_signatures_without_inheritance() {
    for model in [
        "gemini-4-hydrogen",
        "gemini-4-argon",
        "gemini-4-carbon",
        "gemini-4-neon",
    ] {
        let native = response_to_message(response(
            model,
            vec![
                json!({"text": "Plan", "thought": true, "thoughtSignature": "THOUGHT"}),
                json!({"functionCall": {"id": "call-1", "name": "read", "args": {}}, "thoughtSignature": "FIRST"}),
                json!({"functionCall": {"id": "call-2", "name": "read", "args": {"path": "b"}}}),
                json!({"functionCall": {"id": "call-3", "name": "read", "args": {"path": "c"}}, "thoughtSignature": "THIRD"}),
            ],
        ))
        .unwrap();
        let first = native.content[1].as_tool_request().unwrap();
        let second = native.content[2].as_tool_request().unwrap();
        assert_eq!(first.id, "call-1", "{model} preserves the provider call ID");
        assert_eq!(get_thought_signature(&first.metadata), Some("FIRST"));
        assert_eq!(get_thought_signature(&second.metadata), None);
        let tool_response = Message::user().with_tool_response_with_metadata(
            first.id.clone(),
            Ok(tool_result()),
            first.metadata.as_ref(),
        );
        let later = response_to_message(response(
            model,
            vec![json!({"text": "Done", "thoughtSignature": "FINAL"})],
        ))
        .unwrap();
        let payload = create_request(
            &ModelConfig::new_or_fail(model),
            "system",
            &[
                Message::user().with_text("inspect"),
                native,
                tool_response,
                later,
            ],
            &[],
        )
        .unwrap();
        let parts = &payload["contents"][1]["parts"];
        assert_eq!(
            parts[0],
            json!({"text": "Plan", "thought": true, "thoughtSignature": "THOUGHT"})
        );
        assert_eq!(
            parts[1],
            json!({"functionCall": {"id": "call-1", "name": "read", "args": {}}, "thoughtSignature": "FIRST"})
        );
        assert_eq!(
            parts[2],
            json!({"functionCall": {"id": "call-2", "name": "read", "args": {"path": "b"}}})
        );
        assert_eq!(parts[3]["thoughtSignature"], "THIRD");
        let result = &payload["contents"][2]["parts"][0];
        assert_eq!(result["functionResponse"]["name"], "read");
        assert_eq!(result["functionResponse"]["id"], "call-1");
        assert!(result.get("thoughtSignature").is_none());
        assert_eq!(
            payload["contents"][3]["parts"][0],
            json!({"text": "Done", "thoughtSignature": "FINAL"})
        );
    }
}

#[test]
fn gemini4_returns_parallel_tool_results_in_original_call_order() {
    let native = response_to_message(response(
        "gemini-4-hydrogen",
        vec![
            json!({"functionCall": {"id": "first", "name": "read", "args": {}}, "thoughtSignature": "FIRST"}),
            json!({"functionCall": {"id": "second", "name": "write", "args": {}}}),
        ],
    ))
    .unwrap();
    let results = Message::user()
        .with_tool_response("second", Ok(tool_result()))
        .with_tool_response("first", Ok(tool_result()));
    let payload = create_request(
        &ModelConfig::new_or_fail("gemini-4-hydrogen"),
        "system",
        &[native, results],
        &[],
    )
    .unwrap();
    let parts = &payload["contents"][1]["parts"];
    assert_eq!(parts[0]["functionResponse"]["id"], "first");
    assert_eq!(parts[0]["functionResponse"]["name"], "read");
    assert_eq!(parts[1]["functionResponse"]["id"], "second");
    assert_eq!(parts[1]["functionResponse"]["name"], "write");
    assert!(parts[0].get("thoughtSignature").is_none());
    assert!(parts[1].get("thoughtSignature").is_none());
}

#[test]
fn gemini4_signed_text_survives_persistence_as_visible_text() {
    for text in ["Answer", ""] {
        let native = response_to_message(response(
            "gemini-4-hydrogen",
            vec![json!({"text": text, "thoughtSignature": "SIGNED-TEXT"})],
        ))
        .unwrap();
        assert!(matches!(
            native.content.first(),
            Some(MessageContent::Text(_))
        ));
        let restored: Message =
            serde_json::from_value(serde_json::to_value(native).unwrap()).unwrap();
        let payload = create_request(
            &ModelConfig::new_or_fail("gemini-4-hydrogen"),
            "system",
            &[restored, Message::user().with_text("continue")],
            &[],
        )
        .unwrap();
        assert_eq!(
            payload["contents"][0]["parts"][0],
            json!({"text": text, "thoughtSignature": "SIGNED-TEXT"})
        );
    }
}

#[test]
fn gemini4_conversation_push_preserves_signed_text_boundaries() {
    let native_parts = vec![
        json!({"text": "First ", "thoughtSignature": "FIRST"}),
        json!({"text": "second ", "thoughtSignature": "SECOND"}),
        json!({"text": "", "thoughtSignature": "EMPTY"}),
    ];
    let mut conversation = Conversation::empty();
    conversation.push(Message::user().with_text("inspect"));
    for part in &native_parts {
        let message = response_to_message(response("gemini-4-hydrogen", vec![part.clone()]))
            .unwrap()
            .with_id("signed-stream".to_string());
        conversation.push(message);
    }
    conversation.push(Message::user().with_text("continue"));
    let payload = create_request(
        &ModelConfig::new_or_fail("gemini-4-hydrogen"),
        "system",
        conversation.messages(),
        &[],
    )
    .unwrap();
    assert_eq!(payload["contents"][1]["parts"], json!(native_parts));
}

#[test]
fn gemini4_normalization_preserves_signed_text_and_empty_parts() {
    let native_parts = vec![
        json!({"text": "First ", "thoughtSignature": "FIRST"}),
        json!({"text": "second ", "thoughtSignature": "SECOND"}),
        json!({"text": "", "thoughtSignature": "EMPTY"}),
    ];
    let native = response_to_message(response("gemini-4-hydrogen", native_parts.clone())).unwrap();
    let empty = response_to_message(response(
        "gemini-4-hydrogen",
        vec![json!({"text": "", "thoughtSignature": "EMPTY-TURN"})],
    ))
    .unwrap();
    let (conversation, _) = fix_conversation(Conversation::new_unvalidated([
        Message::user().with_text("inspect"),
        native,
        Message::user().with_text("continue"),
        empty,
        Message::user().with_text("last"),
    ]));
    let payload = create_request(
        &ModelConfig::new_or_fail("gemini-4-hydrogen"),
        "system",
        conversation.messages(),
        &[],
    )
    .unwrap();
    assert_eq!(payload["contents"][1]["parts"], json!(native_parts));
    assert_eq!(
        payload["contents"][3]["parts"],
        json!([{"text": "", "thoughtSignature": "EMPTY-TURN"}])
    );
}

#[tokio::test]
async fn gemini4_streaming_preserves_each_call_signature_and_id() {
    let chunks = [
        response(
            "gemini-4-hydrogen",
            vec![json!({"text": "Working", "thoughtSignature": "TEXT"})],
        ),
        response(
            "gemini-4-hydrogen",
            vec![
                json!({"functionCall": {"id": "first", "name": "read", "args": {}}, "thoughtSignature": "CALL"}),
                json!({"functionCall": {"id": "second", "name": "read", "args": {"path": "b"}}}),
            ],
        ),
    ];
    let lines = chunks.into_iter().map(|chunk| Ok(format!("data: {chunk}")));
    let stream = response_to_streaming_message(futures::stream::iter(lines));
    futures::pin_mut!(stream);
    let mut messages = Vec::new();
    while let Some(item) = stream.next().await {
        if let Some(message) = item.unwrap().0 {
            messages.push(message);
        }
    }
    let calls: Vec<_> = messages
        .iter()
        .flat_map(|m| &m.content)
        .filter_map(MessageContent::as_tool_request)
        .collect();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].id, "first");
    assert_eq!(get_thought_signature(&calls[0].metadata), Some("CALL"));
    assert_eq!(calls[1].id, "second");
    assert_eq!(get_thought_signature(&calls[1].metadata), None);
    let payload = create_request(
        &ModelConfig::new_or_fail("gemini-4-hydrogen"),
        "system",
        &messages,
        &[],
    )
    .unwrap();
    assert_eq!(
        payload["contents"][0]["parts"][0],
        json!({"text": "Working", "thoughtSignature": "TEXT"})
    );
}

#[test]
fn gemini4_omits_ignored_sampling_parameters_and_keeps_output_budget() {
    let mut config = ModelConfig::new_or_fail("gemini-4-hydrogen");
    config.temperature = Some(0.5);
    config.max_tokens = Some(32_768);
    let payload = create_request(
        &config,
        "system",
        &[Message::user().with_text("inspect")],
        &[],
    )
    .unwrap();
    assert!(payload["generationConfig"].get("temperature").is_none());
    assert!(payload["generationConfig"].get("thinkingConfig").is_none());
    assert_eq!(payload["generationConfig"]["maxOutputTokens"], 32_768);
}

#[test]
fn legacy_gemini_keeps_signature_inheritance_and_sampling_behavior() {
    let native = response_to_message(response(
        "gemini-2.5-pro",
        vec![
            json!({"functionCall": {"name": "first", "args": {}}, "thoughtSignature": "LEGACY"}),
            json!({"functionCall": {"name": "second", "args": {}}}),
        ],
    ))
    .unwrap();
    assert_eq!(
        get_thought_signature(&native.content[1].as_tool_request().unwrap().metadata),
        Some("LEGACY")
    );
    let mut config = ModelConfig::new_or_fail("gemini-2.5-pro");
    config.temperature = Some(0.5);
    let payload = create_request(&config, "system", &[native], &[]).unwrap();
    assert_eq!(payload["generationConfig"]["temperature"], 0.5);
}
