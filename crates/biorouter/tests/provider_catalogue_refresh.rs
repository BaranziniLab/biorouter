biorouter::keep_this_test_binary_off_the_os_credential_store!();

use biorouter::config::declarative_providers::register_declarative_provider;
use biorouter::config::{with_config_overrides, DeclarativeProviderConfig};
use biorouter::conversation::message::Message;
use biorouter::model::ModelConfig;
use biorouter::providers::base::{ModelInfo, Provider, ProviderMetadata, ProviderType};
use biorouter::providers::openai::OpenAiProvider;
use biorouter::providers::provider_registry::ProviderRegistry;
use biorouter::providers::zai::ZaiProvider;
use rmcp::model::{CallToolResult, Content, Tool};
use rmcp::object;
use serde_json::{json, Value};
use std::collections::HashMap;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn registered(json: &str) -> ProviderMetadata {
    let config: DeclarativeProviderConfig = serde_json::from_str(json).unwrap();
    let mut registry = ProviderRegistry::new();
    register_declarative_provider(&mut registry, config, ProviderType::Declarative);
    let mut metadata = registry.all_metadata_with_types();
    assert_eq!(metadata.len(), 1);
    metadata.pop().unwrap().0
}

fn listed<'a>(metadata: &'a ProviderMetadata, name: &str) -> &'a ModelInfo {
    metadata
        .known_models
        .iter()
        .find(|model| model.name == name)
        .unwrap_or_else(|| panic!("{} does not offer {name}", metadata.name))
}

#[test]
fn moonshot_registers_k31_pro_first_without_removing_existing_choices() {
    let metadata = registered(include_str!("../src/providers/declarative/moonshot.json"));
    assert_eq!(metadata.default_model, "kimi-k3.1-pro");
    assert_eq!(metadata.known_models[0].name, metadata.default_model);
    assert_eq!(metadata.known_models[1].name, "kimi-k3.1-pro-highspeed");

    for (name, input, output) in [
        ("kimi-k3.1-pro", 2.0, 10.0),
        ("kimi-k3.1-pro-highspeed", 4.0, 20.0),
    ] {
        let model = listed(&metadata, name);
        assert_eq!(model.context_limit, 1_000_000, "{name}");
        assert_eq!(model.supports_vision, Some(true), "{name}");
        assert_eq!(model.input_token_cost, Some(input / 1_000_000.0), "{name}");
        assert_eq!(
            model.output_token_cost,
            Some(output / 1_000_000.0),
            "{name}"
        );
        assert_eq!(model.currency.as_deref(), Some("$"), "{name}");
        assert!(model
            .supported_input_mime_types
            .as_ref()
            .unwrap()
            .iter()
            .any(|mime| mime == "image/png"));
    }
    for saved in [
        "kimi-k3",
        "kimi-k2.7-code",
        "kimi-k2.7-code-highspeed",
        "kimi-k2.6",
    ] {
        listed(&metadata, saved);
    }
}

#[test]
fn inception_registers_mercury3_and_discloses_the_api_sunset() {
    let metadata = registered(include_str!("../src/providers/declarative/inception.json"));
    assert_eq!(metadata.default_model, "mercury-3");
    assert_eq!(metadata.known_models[0].name, metadata.default_model);
    let model = listed(&metadata, "mercury-3");
    assert_eq!(model.context_limit, 1_000_000);
    assert_eq!(model.supports_vision, Some(false));
    assert_eq!(model.input_token_cost, Some(0.000_000_1));
    assert_eq!(model.output_token_cost, Some(0.000_000_35));
    assert!(metadata.description.contains("December 28, 2026"));
    assert!(metadata.description.contains("requests stop"));
    listed(&metadata, "mercury-2.5");
    listed(&metadata, "mercury-2");
    for incompatible in ["mercury-decide", "mercury-edit-2", "mercury-voice"] {
        assert!(metadata
            .known_models
            .iter()
            .all(|model| model.name != incompatible));
    }
}

#[test]
fn still_current_bundled_defaults_keep_their_provider_specific_capabilities() {
    let baseten = registered(include_str!("../src/providers/declarative/baseten.json"));
    assert_eq!(baseten.default_model, "zai-org/GLM-5.3");
    assert_eq!(baseten.known_models[0].name, baseten.default_model);
    assert_eq!(
        listed(&baseten, "zai-org/GLM-5.3").supports_vision,
        Some(true)
    );
    assert!(baseten
        .known_models
        .iter()
        .all(|model| model.name != "reflection/Beam-1.0"));

    let deepseek = registered(include_str!("../src/providers/declarative/deepseek.json"));
    assert_eq!(deepseek.default_model, "deepseek-flash");
    assert_eq!(deepseek.known_models[0].name, deepseek.default_model);
    listed(&deepseek, "deepseek-v4-pro");

    let groq = registered(include_str!("../src/providers/declarative/groq.json"));
    assert_eq!(groq.default_model, "openai/gpt-oss-120b");
    assert_eq!(groq.known_models[0].name, groq.default_model);
    assert_eq!(
        listed(&groq, "qwen/qwen3.8-27b").supports_vision,
        Some(true)
    );
    assert!(groq
        .known_models
        .iter()
        .all(|model| !model.name.starts_with("groq/compound")));
}

#[test]
fn baseten_adds_k31_pro_without_changing_its_flagship_or_protocol_exclusions() {
    let metadata = registered(include_str!("../src/providers/declarative/baseten.json"));
    assert_eq!(metadata.default_model, "zai-org/GLM-5.3");
    assert_eq!(metadata.known_models[0].name, metadata.default_model);
    let model = listed(&metadata, "moonshotai/Kimi-K3.1-Pro");
    assert_eq!(model.context_limit, 1_048_576);
    assert_eq!(model.supports_vision, Some(true));
    assert_eq!(model.input_token_cost, Some(0.000_002));
    assert_eq!(model.output_token_cost, Some(0.000_01));
    assert_eq!(model.currency.as_deref(), Some("$"));
    let names: Vec<_> = metadata
        .known_models
        .iter()
        .map(|model| model.name.as_str())
        .collect();
    assert!(
        names
            .iter()
            .position(|name| *name == "moonshotai/Kimi-K3.1-Pro")
            < names.iter().position(|name| *name == "moonshotai/Kimi-K3")
    );
    assert!(!names.contains(&"reflection/Beam-1.0"));
}

#[tokio::test]
async fn baseten_k31_chat_completions_preserves_images_and_tool_history() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let after_tool = body["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message["role"] == "tool");
            let (message, finish_reason) = if after_tool {
                (
                    json!({"role": "assistant", "content": "Sample S-42 is confirmed."}),
                    "stop",
                )
            } else {
                (
                    json!({
                        "role": "assistant",
                        "content": null,
                        "reasoning_content": "Read the sample identifier before answering.",
                        "tool_calls": [{
                            "id": "call_sample_42",
                            "type": "function",
                            "function": {
                                "name": "lookup_sample",
                                "arguments": "{\"sample\":\"S-42\"}"
                            }
                        }]
                    }),
                    "tool_calls",
                )
            };
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "chatcmpl-k31-fixture",
                "object": "chat.completion",
                "created": 0,
                "model": "moonshotai/Kimi-K3.1-Pro",
                "choices": [{"index": 0, "message": message, "finish_reason": finish_reason}],
                "usage": {"prompt_tokens": 30, "completion_tokens": 12, "total_tokens": 42}
            }))
        })
        .expect(2)
        .mount(&server)
        .await;

    let mut config: DeclarativeProviderConfig =
        serde_json::from_str(include_str!("../src/providers/declarative/baseten.json")).unwrap();
    config.base_url = format!("{}/v1", server.uri());
    config.api_key_env = "BIOROUTER_TEST_BASETEN_CATALOGUE_KEY".to_string();
    let provider = with_config_overrides(
        HashMap::from([(config.api_key_env.clone(), "synthetic-test-key".to_string())]),
        async {
            OpenAiProvider::from_custom_config(
                ModelConfig::new("moonshotai/Kimi-K3.1-Pro")
                    .unwrap()
                    .with_context_limit(Some(1_048_576))
                    .with_max_tokens(Some(4096)),
                config,
            )
        },
    )
    .await
    .unwrap();
    let tool = Tool::new(
        "lookup_sample",
        "Look up a sample identifier",
        object!({
            "type": "object",
            "properties": {"sample": {"type": "string"}},
            "required": ["sample"]
        }),
    );
    let image = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aX1cAAAAASUVORK5CYII=";
    let question = Message::user()
        .with_text("Look up the sample identifier in this image.")
        .with_image(image, "image/png");
    let (assistant, _) = provider
        .complete(
            "Use the lookup tool.",
            std::slice::from_ref(&question),
            std::slice::from_ref(&tool),
        )
        .await
        .unwrap();
    let call = assistant
        .content
        .iter()
        .find_map(|content| content.as_tool_request())
        .expect("the model response must retain its tool call");
    assert_eq!(call.id, "call_sample_42");
    let result = Message::user().with_tool_response(
        &call.id,
        Ok(CallToolResult {
            content: vec![Content::text("Sample S-42 is confirmed.")],
            structured_content: None,
            is_error: Some(false),
            meta: None,
        }),
    );
    let (answer, _) = provider
        .complete(
            "Use the lookup tool.",
            &[question, assistant, result],
            &[tool],
        )
        .await
        .unwrap();
    assert_eq!(answer.as_concat_text(), "Sample S-42 is confirmed.");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["model"], "moonshotai/Kimi-K3.1-Pro");
        assert_eq!(body["max_tokens"], 4096);
        assert!(body.get("max_completion_tokens").is_none());
        assert_eq!(body["tools"][0]["function"]["name"], "lookup_sample");
        assert!(body["messages"].as_array().unwrap().iter().any(|message| {
            message["role"] == "user"
                && message["content"].as_array().is_some_and(|content| {
                    content.iter().any(|part| {
                        part["type"] == "image_url"
                            && part["image_url"]["url"] == format!("data:image/png;base64,{image}")
                    })
                })
        }));
    }
    let second: Value = serde_json::from_slice(&requests[1].body).unwrap();
    let history = second["messages"].as_array().unwrap();
    let assistant = history
        .iter()
        .find(|message| message["role"] == "assistant")
        .unwrap();
    assert_eq!(assistant["tool_calls"][0]["id"], "call_sample_42");
    assert_eq!(
        assistant["tool_calls"][0]["function"]["name"],
        "lookup_sample"
    );
    assert_eq!(
        serde_json::from_str::<Value>(
            assistant["tool_calls"][0]["function"]["arguments"]
                .as_str()
                .unwrap()
        )
        .unwrap(),
        json!({"sample": "S-42"})
    );
    let result = history
        .iter()
        .find(|message| message["role"] == "tool")
        .unwrap();
    assert_eq!(result["tool_call_id"], "call_sample_42");
    assert!(result["content"].as_str().unwrap().contains("S-42"));
}

#[test]
fn zai_distinguishes_its_text_flagship_from_its_multimodal_flash_variants() {
    let metadata = ZaiProvider::metadata();
    assert_eq!(metadata.default_model, "glm-5.3");
    assert_eq!(metadata.known_models[0].name, metadata.default_model);
    let flagship = listed(&metadata, "glm-5.3");
    assert_eq!(flagship.supports_vision, Some(false));
    assert!(flagship.supported_input_mime_types.is_none());
    for name in ["glm-5.3-flash", "glm-5.3-flashx"] {
        let model = listed(&metadata, name);
        assert_eq!(model.supports_vision, Some(true), "{name}");
        assert!(model
            .supported_input_mime_types
            .as_ref()
            .unwrap()
            .iter()
            .any(|mime| mime == "image/png"));
    }
}
