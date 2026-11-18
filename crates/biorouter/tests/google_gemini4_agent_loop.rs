//! Native Gemini history must survive real tool execution and a later user turn.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use biorouter::agents::extension::ExtensionConfig;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::{with_config_overrides, BioRouterMode};
use biorouter::conversation::message::{ActionRequiredData, Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::permission::permission_confirmation::PrincipalType;
use biorouter::permission::{Permission, PermissionConfirmation};
use biorouter::providers::anthropic::AnthropicProvider;
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::providers::gcpvertexai::GcpVertexAIProvider;
use biorouter::providers::google::GoogleProvider;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use futures::StreamExt;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MODEL: &str = "gemini-4-hydrogen";
const FIRST_OUTPUT: &str = "GOOGLE_NATIVE_FIRST";
const SECOND_OUTPUT: &str = "GOOGLE_NATIVE_SECOND";

#[ctor::ctor]
fn sandbox_global_config() {
    if biorouter::config::paths::Paths::path_root_override().is_some() {
        return;
    }
    let root = TempDir::new().expect("scratch config root");
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    static ROOT: std::sync::OnceLock<TempDir> = std::sync::OnceLock::new();
    let _ = ROOT.set(root);
}

fn native_parts(include_thought: bool) -> Vec<Value> {
    let mut parts = Vec::new();
    if include_thought {
        parts.push(json!({
            "text": "Plan before tools ",
            "thought": true,
            "thoughtSignature": "NATIVE-THOUGHT"
        }));
    }
    parts.extend([
        json!({"text": "Working ", "thoughtSignature": "NATIVE-TEXT"}),
        json!({
            "functionCall": {
                "id": "native-first",
                "name": "developer__shell",
                "args": {"command": format!("printf {FIRST_OUTPUT}")}
            },
            "thoughtSignature": "NATIVE-CALL"
        }),
        json!({
            "functionCall": {
                "id": "native-second",
                "name": "developer__shell",
                "args": {"command": format!("printf {SECOND_OUTPUT}")}
            }
        }),
        json!({"text": "Calls emitted ", "thoughtSignature": "NATIVE-TRAILING"}),
    ]);
    parts
}

fn sse(parts: &[Value]) -> String {
    let mut body = String::new();
    for part in parts {
        let chunk = json!({"candidates": [{"content": {"role": "model", "parts": [part]}}]});
        body.push_str(&format!("data: {chunk}\n\n"));
    }
    let usage = json!({
        "usageMetadata": {"promptTokenCount": 20, "candidatesTokenCount": 4, "totalTokenCount": 24}
    });
    body.push_str(&format!("data: {usage}\n\ndata: [DONE]\n\n"));
    body
}

struct Harness {
    agent: Agent,
    manager: Arc<SessionManager>,
    session_id: String,
    _work: TempDir,
    _data: TempDir,
    _permissions: TempDir,
}

async fn harness() -> Harness {
    let work = TempDir::new().unwrap();
    let data = TempDir::new().unwrap();
    let permissions = TempDir::new().unwrap();
    let manager = Arc::new(SessionManager::new(data.path().to_path_buf()));
    let agent = Agent::with_config(AgentConfig::new(
        manager.clone(),
        Arc::new(PermissionManager::new(permissions.path().to_path_buf())),
        None,
        BioRouterMode::Auto,
    ));
    agent
        .add_extension(ExtensionConfig::Builtin {
            name: "developer".to_string(),
            description: "Synthetic loopback developer tools".to_string(),
            display_name: None,
            timeout: Some(10),
            bundled: Some(true),
            available_tools: vec!["shell".to_string()],
        })
        .await
        .unwrap();
    let session = manager
        .create_session(
            work.path().to_path_buf(),
            "google-native-history-loopback".to_string(),
            SessionType::Hidden,
        )
        .await
        .unwrap();
    agent
        .update_provider(
            Arc::new(
                GoogleProvider::from_env(ModelConfig::new_or_fail(MODEL))
                    .await
                    .unwrap(),
            ),
            &session.id,
        )
        .await
        .unwrap();
    Harness {
        agent,
        manager,
        session_id: session.id,
        _work: work,
        _data: data,
        _permissions: permissions,
    }
}

async fn run_turn(harness: &Harness, prompt: &str) {
    tokio::time::timeout(Duration::from_secs(20), async {
        let stream = harness
            .agent
            .reply(
                Message::user().with_text(prompt),
                SessionConfig {
                    id: harness.session_id.clone(),
                    schedule_id: None,
                    max_turns: Some(3),
                    max_tool_calls: Some(3),
                    budget: None,
                    retry_config: None,
                    reasoning_effort: None,
                },
                None,
            )
            .await
            .unwrap();
        tokio::pin!(stream);
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                AgentEvent::Message(message) => {
                    for content in message.content {
                        if let MessageContent::ActionRequired(action) = content {
                            if let ActionRequiredData::ToolConfirmation { id, .. } = action.data {
                                harness
                                    .agent
                                    .handle_confirmation(
                                        id,
                                        PermissionConfirmation {
                                            principal_type: PrincipalType::Tool,
                                            permission: Permission::AllowOnce,
                                        },
                                    )
                                    .await;
                            }
                        }
                    }
                }
                AgentEvent::TurnAborted { code, message, .. } => {
                    panic!("synthetic Google turn aborted: {code:?}: {message}");
                }
                _ => {}
            }
        }
    })
    .await
    .expect("bounded real Google agent turn");
}

fn assert_native_history(request: &Value, native: &[Value]) {
    let contents = request["contents"].as_array().unwrap();
    let model_turns: Vec<_> = contents
        .iter()
        .enumerate()
        .filter(|(_, message)| {
            message["parts"].as_array().is_some_and(|parts| {
                parts.iter().any(|part| {
                    matches!(
                        part["functionCall"]["id"].as_str(),
                        Some("native-first" | "native-second")
                    )
                })
            })
        })
        .collect();
    assert_eq!(
        model_turns.len(),
        1,
        "native parallel calls must remain one model turn"
    );
    let (model_index, model_turn) = model_turns[0];
    assert_eq!(model_turn["role"], "model");
    assert_eq!(model_turn["parts"], json!(native));
    let results: Vec<_> = contents
        .iter()
        .enumerate()
        .flat_map(|(index, message)| {
            message["parts"]
                .as_array()
                .unwrap()
                .iter()
                .map(move |part| (index, part))
        })
        .filter(|(_, part)| part.get("functionResponse").is_some())
        .collect();
    assert_eq!(results.len(), 2, "both real tools must return exactly once");
    for ((index, part), id, output) in [
        (results[0], "native-first", FIRST_OUTPUT),
        (results[1], "native-second", SECOND_OUTPUT),
    ] {
        assert!(
            index > model_index,
            "native model turn must precede tool results"
        );
        assert_eq!(contents[index]["role"], "user");
        assert_eq!(part["functionResponse"]["id"], id);
        assert_eq!(part["functionResponse"]["name"], "developer__shell");
        assert!(part["functionResponse"]["response"]
            .to_string()
            .contains(output));
        assert!(part.get("thoughtSignature").is_none());
    }
    for signature in native
        .iter()
        .filter_map(|part| part.get("thoughtSignature"))
    {
        let copies = contents
            .iter()
            .flat_map(|message| message["parts"].as_array().unwrap())
            .filter(|part| part.get("thoughtSignature") == Some(signature))
            .count();
        assert_eq!(
            copies, 1,
            "native signature {signature} must occur exactly once"
        );
    }
}

async fn native_history_survives_agent_turns(batch_tools: bool, include_thought: bool) {
    let _env = env_lock::lock_env([(
        "BIOROUTER_TOOL_CALL_BATCHING",
        Some(if batch_tools { "true" } else { "false" }),
    )]);
    let server = MockServer::start().await;
    let native = native_parts(include_thought);
    let final_part = json!({"text": "First turn complete  ", "thoughtSignature": "FINAL-TEXT"});
    let replies = [
        sse(&native),
        sse(std::slice::from_ref(&final_part)),
        sse(&[json!({"text": "Historical replay complete.", "thoughtSignature": "LATER-TEXT"})]),
    ];
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    Mock::given(method("POST"))
        .and(path(format!(
            "/v1beta/models/{MODEL}:streamGenerateContent"
        )))
        .respond_with(move |request: &wiremock::Request| {
            let mut captured = captured.lock().unwrap();
            let index = captured.len();
            captured.push(serde_json::from_slice::<Value>(&request.body).unwrap());
            if let Some(reply) = replies.get(index) {
                ResponseTemplate::new(200).set_body_raw(reply.clone(), "text/event-stream")
            } else {
                ResponseTemplate::new(500).set_body_string("unexpected extra synthetic request")
            }
        })
        .mount(&server)
        .await;
    let overrides = HashMap::from([
        (
            "GOOGLE_API_KEY".to_string(),
            "gemini4-loopback-key".to_string(),
        ),
        ("GOOGLE_HOST".to_string(), server.uri()),
    ]);
    with_config_overrides(overrides, async {
        let harness = harness().await;
        run_turn(
            &harness,
            "Run the two synthetic tools and report their outputs.",
        )
        .await;
        run_turn(
            &harness,
            "Confirm the previous synthetic outputs without using tools.",
        )
        .await;
        let stored = harness
            .manager
            .get_session(&harness.session_id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        let request = biorouter::providers::formats::google::create_request(
            &ModelConfig::new_or_fail(MODEL),
            "system",
            stored.messages(),
            &[],
        )
        .unwrap();
        assert_native_history(&request, &native);
    })
    .await;
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert_native_history(&requests[1], &native);
    assert_native_history(&requests[2], &native);
    assert!(requests[2]["contents"]
        .as_array()
        .unwrap()
        .iter()
        .any(|message| {
            message["parts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|part| *part == final_part)
        }));
}

#[tokio::test]
async fn gemini4_agent_preserves_native_parallel_history_with_batching() {
    native_history_survives_agent_turns(true, false).await;
}

#[tokio::test]
async fn gemini4_agent_preserves_native_parallel_history_without_batching() {
    native_history_survives_agent_turns(false, false).await;
}

#[tokio::test]
async fn gemini4_agent_preserves_optional_thought_history_with_batching() {
    native_history_survives_agent_turns(true, true).await;
}

#[tokio::test]
async fn gemini4_agent_preserves_optional_thought_history_without_batching() {
    native_history_survives_agent_turns(false, true).await;
}

#[tokio::test]
async fn gemini4_nonstreaming_provider_preserves_literal_escapes_in_native_parts() {
    let server = MockServer::start().await;
    let parts = vec![
        json!({"text": r"Keep \n and \t literal", "thoughtSignature": "LITERAL-TEXT"}),
        json!({
            "functionCall": {"id": "literal-call", "name": "read", "args": {"pattern": r"\n\t"}},
            "thoughtSignature": "LITERAL-CALL"
        }),
    ];
    Mock::given(method("POST"))
        .and(path(format!("/v1beta/models/{MODEL}:generateContent")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{"content": {"role": "model", "parts": parts}}]
        })))
        .mount(&server)
        .await;
    let overrides = HashMap::from([
        (
            "GOOGLE_API_KEY".to_string(),
            "gemini4-loopback-key".to_string(),
        ),
        ("GOOGLE_HOST".to_string(), server.uri()),
    ]);
    with_config_overrides(overrides, async {
        let provider = GoogleProvider::from_env(ModelConfig::new_or_fail(MODEL))
            .await
            .unwrap();
        let (message, _) = provider
            .complete(
                "system",
                &[Message::user().with_text("inspect literals")],
                &[],
            )
            .await
            .unwrap();
        let request = biorouter::providers::formats::google::create_request(
            &ModelConfig::new_or_fail(MODEL),
            "system",
            &[message],
            &[],
        )
        .unwrap();
        assert_eq!(request["contents"][0]["parts"], json!(parts));
    })
    .await;
}

#[tokio::test]
async fn gemini4_agent_keeps_optional_thought_with_unsigned_final_text() {
    let server = MockServer::start().await;
    let native = vec![
        json!({"text": "Private native plan", "thought": true, "thoughtSignature": "PLAIN-THOUGHT"}),
        json!({"text": "Unsigned final text"}),
    ];
    let replies = [
        sse(&native),
        sse(&[json!({"text": "Follow-up complete.", "thoughtSignature": "PLAIN-FOLLOWUP"})]),
    ];
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    Mock::given(method("POST"))
        .and(path(format!(
            "/v1beta/models/{MODEL}:streamGenerateContent"
        )))
        .respond_with(move |request: &wiremock::Request| {
            let mut captured = captured.lock().unwrap();
            let index = captured.len();
            captured.push(serde_json::from_slice::<Value>(&request.body).unwrap());
            if let Some(reply) = replies.get(index) {
                ResponseTemplate::new(200).set_body_raw(reply.clone(), "text/event-stream")
            } else {
                ResponseTemplate::new(500).set_body_string("unexpected extra synthetic request")
            }
        })
        .mount(&server)
        .await;
    let overrides = HashMap::from([
        (
            "GOOGLE_API_KEY".to_string(),
            "gemini4-loopback-key".to_string(),
        ),
        ("GOOGLE_HOST".to_string(), server.uri()),
    ]);
    let stored_request = with_config_overrides(overrides, async {
        let harness = harness().await;
        run_turn(
            &harness,
            "Describe the synthetic value without using tools.",
        )
        .await;
        run_turn(
            &harness,
            "Confirm the prior synthetic answer without using tools.",
        )
        .await;
        let stored = harness
            .manager
            .get_session(&harness.session_id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        biorouter::providers::formats::google::create_request(
            &ModelConfig::new_or_fail(MODEL),
            "system",
            stored.messages(),
            &[],
        )
        .unwrap()
    })
    .await;
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for request in [&requests[1], &stored_request] {
        let contents = request["contents"].as_array().unwrap();
        let turn = contents
            .iter()
            .find(|message| {
                message["parts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|part| part["text"] == "Unsigned final text")
            })
            .expect("original unsigned visible answer remains in history");
        assert_eq!(turn["role"], "model");
        assert_eq!(turn["parts"], json!(native));
        assert_eq!(
            contents
                .iter()
                .flat_map(|message| message["parts"].as_array().unwrap())
                .filter(|part| part["thoughtSignature"] == "PLAIN-THOUGHT")
                .count(),
            1
        );
    }
}

fn anthropic_sse(text: &str, thought: Option<(&str, &str)>) -> String {
    let mut events = vec![json!({
        "type": "message_start",
        "message": {"id": "msg_switch", "model": "claude-opus-5-5", "usage": {"input_tokens": 10, "output_tokens": 1}}
    })];
    let text_index = if let Some((thinking, signature)) = thought {
        events.extend([
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": ""}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": thinking}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "signature_delta", "signature": signature}}),
            json!({"type": "content_block_stop", "index": 0}),
        ]);
        1
    } else {
        0
    };
    events.extend([
        json!({"type": "content_block_start", "index": text_index, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": text_index, "delta": {"type": "text_delta", "text": text}}),
        json!({"type": "content_block_stop", "index": text_index}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null}, "usage": {"output_tokens": 4}}),
        json!({"type": "message_stop"}),
    ]);
    events
        .into_iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect()
}

async fn capture_sse_route(
    server: &MockServer,
    route: String,
    replies: Vec<String>,
) -> Arc<Mutex<Vec<Value>>> {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    Mock::given(method("POST"))
        .and(path(route))
        .respond_with(move |request: &wiremock::Request| {
            let mut captured = captured.lock().unwrap();
            let index = captured.len();
            captured.push(serde_json::from_slice::<Value>(&request.body).unwrap());
            if let Some(reply) = replies.get(index) {
                ResponseTemplate::new(200).set_body_raw(reply.clone(), "text/event-stream")
            } else {
                ResponseTemplate::new(500).set_body_string("unexpected extra switch request")
            }
        })
        .mount(server)
        .await;
    requests
}

fn assert_optional_native_turn(request: &Value, native: &[Value]) {
    let turns = request["contents"].as_array().unwrap();
    let matching: Vec<_> = turns
        .iter()
        .filter(|turn| {
            turn["parts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|part| part["text"] == "Google visible answer before switching")
        })
        .collect();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0]["role"], "model");
    assert_eq!(matching[0]["parts"], json!(native));
    assert_eq!(
        turns
            .iter()
            .flat_map(|turn| turn["parts"].as_array().unwrap())
            .filter(|part| part["thoughtSignature"] == "GOOGLE-SWITCH-SIGNATURE")
            .count(),
        1
    );
}

fn assert_foreign_thought_absent(request: &Value, thinking: &str, signature: &str) {
    let serialized = request.to_string();
    assert!(
        !serialized.contains(thinking),
        "foreign thought text leaked"
    );
    assert!(
        !serialized.contains(signature),
        "foreign thought signature leaked"
    );
    assert!(!serialized.contains("googleNativePart"));
}

fn switch_overrides(server: &MockServer) -> HashMap<String, String> {
    HashMap::from([
        (
            "GOOGLE_API_KEY".to_string(),
            "google-switch-loopback-key".to_string(),
        ),
        ("GOOGLE_HOST".to_string(), server.uri()),
        (
            "ANTHROPIC_API_KEY".to_string(),
            "anthropic-switch-loopback-key".to_string(),
        ),
        ("ANTHROPIC_HOST".to_string(), server.uri()),
    ])
}

#[tokio::test]
async fn gemini4_agent_provider_switch_isolates_google_thought_and_retains_durable_history() {
    let server = MockServer::start().await;
    let native = vec![
        json!({"text": "Google private thought before switching", "thought": true, "thoughtSignature": "GOOGLE-SWITCH-SIGNATURE"}),
        json!({"text": "Google visible answer before switching"}),
    ];
    let google_requests = capture_sse_route(
        &server,
        format!("/v1beta/models/{MODEL}:streamGenerateContent"),
        vec![sse(&native), sse(&[json!({"text": "Google returned"})])],
    )
    .await;
    let anthropic_requests = capture_sse_route(
        &server,
        "/v1/messages".to_string(),
        vec![anthropic_sse("Anthropic visible answer", None)],
    )
    .await;
    let stored_request = with_config_overrides(switch_overrides(&server), async {
        let harness = harness().await;
        run_turn(
            &harness,
            "Answer the first synthetic question without tools.",
        )
        .await;
        harness
            .agent
            .update_provider(
                Arc::new(
                    AnthropicProvider::from_env(ModelConfig::new_or_fail("claude-opus-5-5"))
                        .await
                        .unwrap(),
                ),
                &harness.session_id,
            )
            .await
            .unwrap();
        run_turn(
            &harness,
            "Answer the second synthetic question without tools.",
        )
        .await;
        harness
            .agent
            .update_provider(
                Arc::new(
                    GoogleProvider::from_env(ModelConfig::new_or_fail(MODEL))
                        .await
                        .unwrap(),
                ),
                &harness.session_id,
            )
            .await
            .unwrap();
        run_turn(
            &harness,
            "Answer the third synthetic question without tools.",
        )
        .await;
        let stored = harness
            .manager
            .get_session(&harness.session_id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        let (vertex_request, _) = biorouter::providers::formats::gcpvertexai::create_request(
            &ModelConfig::new_or_fail(MODEL),
            "system",
            stored.messages(),
            &[],
        )
        .unwrap();
        assert_optional_native_turn(&vertex_request, &native);
        biorouter::providers::formats::google::create_request(
            &ModelConfig::new_or_fail(MODEL),
            "system",
            stored.messages(),
            &[],
        )
        .unwrap()
    })
    .await;
    let google_requests = google_requests.lock().unwrap();
    let anthropic_requests = anthropic_requests.lock().unwrap();
    assert_eq!(google_requests.len(), 2);
    assert_eq!(anthropic_requests.len(), 1);
    assert_optional_native_turn(&google_requests[1], &native);
    assert_optional_native_turn(&stored_request, &native);
    assert!(anthropic_requests[0]
        .to_string()
        .contains("Google visible answer before switching"));
    assert_foreign_thought_absent(
        &anthropic_requests[0],
        "Google private thought before switching",
        "GOOGLE-SWITCH-SIGNATURE",
    );
}

#[tokio::test]
async fn gemini4_agent_provider_switch_strips_anthropic_thought_before_google() {
    let server = MockServer::start().await;
    let google_requests = capture_sse_route(
        &server,
        format!("/v1beta/models/{MODEL}:streamGenerateContent"),
        vec![sse(&[json!({"text": "Google answer after Anthropic"})])],
    )
    .await;
    let anthropic_requests = capture_sse_route(
        &server,
        "/v1/messages".to_string(),
        vec![anthropic_sse(
            "Anthropic visible answer before switching",
            Some((
                "Anthropic private thought before switching",
                "ANTHROPIC-SWITCH-SIGNATURE",
            )),
        )],
    )
    .await;
    with_config_overrides(switch_overrides(&server), async {
        let harness = harness().await;
        harness
            .agent
            .update_provider(
                Arc::new(
                    AnthropicProvider::from_env(ModelConfig::new_or_fail("claude-opus-5-5"))
                        .await
                        .unwrap(),
                ),
                &harness.session_id,
            )
            .await
            .unwrap();
        run_turn(
            &harness,
            "Answer the first synthetic question without tools.",
        )
        .await;
        harness
            .agent
            .update_provider(
                Arc::new(
                    GoogleProvider::from_env(ModelConfig::new_or_fail(MODEL))
                        .await
                        .unwrap(),
                ),
                &harness.session_id,
            )
            .await
            .unwrap();
        run_turn(
            &harness,
            "Answer the second synthetic question without tools.",
        )
        .await;
    })
    .await;
    let google_requests = google_requests.lock().unwrap();
    assert_eq!(google_requests.len(), 1);
    assert_eq!(anthropic_requests.lock().unwrap().len(), 1);
    assert!(google_requests[0]
        .to_string()
        .contains("Anthropic visible answer before switching"));
    assert_foreign_thought_absent(
        &google_requests[0],
        "Anthropic private thought before switching",
        "ANTHROPIC-SWITCH-SIGNATURE",
    );
}

/// Runs the actual Vertex formatter at the real Agent outbound boundary, with
/// only transport and authentication replaced by an in-memory capture.
struct VertexFormatCapture {
    model: ModelConfig,
    requests: Arc<Mutex<Vec<Value>>>,
}

#[async_trait::async_trait]
impl Provider for VertexFormatCapture {
    fn metadata() -> ProviderMetadata {
        GcpVertexAIProvider::metadata()
    }
    fn get_name(&self) -> &str {
        "gcpvertexai"
    }
    fn get_model_config(&self) -> ModelConfig {
        self.model.clone()
    }

    async fn complete_with_model(
        &self,
        model: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[rmcp::model::Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let (request, _) = biorouter::providers::formats::gcpvertexai::create_request(
            model, system, messages, tools,
        )
        .map_err(|error| ProviderError::RequestFailed(error.to_string()))?;
        self.requests.lock().unwrap().push(request);
        Ok((
            Message::assistant().with_text("Vertex formatter capture complete"),
            ProviderUsage::new(model.model_name.clone(), Usage::default()),
        ))
    }
}

async fn vertex_switch_preserves_compatible_native_history(model: &str, compatible: bool) {
    let server = MockServer::start().await;
    let native = vec![
        json!({"text": "Google private thought before switching", "thought": true, "thoughtSignature": "GOOGLE-SWITCH-SIGNATURE"}),
        json!({"text": "Google visible answer before switching"}),
    ];
    let google_requests = capture_sse_route(
        &server,
        format!("/v1beta/models/{MODEL}:streamGenerateContent"),
        vec![
            sse(&native),
            sse(&[json!({"text": "Google returned from Vertex"})]),
        ],
    )
    .await;
    let vertex_requests = Arc::new(Mutex::new(Vec::new()));
    with_config_overrides(switch_overrides(&server), async {
        let harness = harness().await;
        run_turn(
            &harness,
            "Answer the first synthetic question without tools.",
        )
        .await;
        harness
            .agent
            .update_provider(
                Arc::new(VertexFormatCapture {
                    model: ModelConfig::new_or_fail(model),
                    requests: vertex_requests.clone(),
                }),
                &harness.session_id,
            )
            .await
            .unwrap();
        run_turn(
            &harness,
            "Answer the Vertex synthetic question without tools.",
        )
        .await;
        harness
            .agent
            .update_provider(
                Arc::new(
                    GoogleProvider::from_env(ModelConfig::new_or_fail(MODEL))
                        .await
                        .unwrap(),
                ),
                &harness.session_id,
            )
            .await
            .unwrap();
        run_turn(
            &harness,
            "Answer the last synthetic question without tools.",
        )
        .await;
    })
    .await;
    let google_requests = google_requests.lock().unwrap();
    let vertex_requests = vertex_requests.lock().unwrap();
    assert_eq!(google_requests.len(), 2);
    assert_eq!(vertex_requests.len(), 1);
    assert_optional_native_turn(&google_requests[1], &native);
    if compatible {
        assert_optional_native_turn(&vertex_requests[0], &native);
    } else {
        assert!(vertex_requests[0]
            .to_string()
            .contains("Google visible answer before switching"));
        assert_foreign_thought_absent(
            &vertex_requests[0],
            "Google private thought before switching",
            "GOOGLE-SWITCH-SIGNATURE",
        );
    }
}

#[tokio::test]
async fn gemini4_agent_provider_switch_vertex_gemini_keeps_compatible_native_history() {
    vertex_switch_preserves_compatible_native_history(MODEL, true).await;
}

#[tokio::test]
async fn gemini4_agent_provider_switch_vertex_claude_strips_foreign_native_thought() {
    vertex_switch_preserves_compatible_native_history("claude-opus-4-6", false).await;
}

#[tokio::test]
async fn gemini4_agent_provider_switch_legacy_google_strips_foreign_native_thought() {
    const LEGACY_MODEL: &str = "gemini-3.8-flash";
    let server = MockServer::start().await;
    let native = vec![
        json!({"text": "Google private thought before switching", "thought": true, "thoughtSignature": "GOOGLE-SWITCH-SIGNATURE"}),
        json!({"text": "Google visible answer before switching"}),
    ];
    let google_requests = capture_sse_route(
        &server,
        format!("/v1beta/models/{MODEL}:streamGenerateContent"),
        vec![
            sse(&native),
            sse(&[json!({"text": "Google returned from legacy"})]),
        ],
    )
    .await;
    let legacy_requests = capture_sse_route(
        &server,
        format!("/v1beta/models/{LEGACY_MODEL}:streamGenerateContent"),
        vec![sse(&[json!({"text": "Legacy Google visible answer"})])],
    )
    .await;
    with_config_overrides(switch_overrides(&server), async {
        let harness = harness().await;
        run_turn(
            &harness,
            "Answer the first synthetic question without tools.",
        )
        .await;
        harness
            .agent
            .update_provider(
                Arc::new(
                    GoogleProvider::from_env(ModelConfig::new_or_fail(LEGACY_MODEL))
                        .await
                        .unwrap(),
                ),
                &harness.session_id,
            )
            .await
            .unwrap();
        run_turn(
            &harness,
            "Answer the legacy synthetic question without tools.",
        )
        .await;
        harness
            .agent
            .update_provider(
                Arc::new(
                    GoogleProvider::from_env(ModelConfig::new_or_fail(MODEL))
                        .await
                        .unwrap(),
                ),
                &harness.session_id,
            )
            .await
            .unwrap();
        run_turn(
            &harness,
            "Answer the last synthetic question without tools.",
        )
        .await;
    })
    .await;
    let google_requests = google_requests.lock().unwrap();
    let legacy_requests = legacy_requests.lock().unwrap();
    assert_eq!(google_requests.len(), 2);
    assert_eq!(legacy_requests.len(), 1);
    assert_optional_native_turn(&google_requests[1], &native);
    assert!(legacy_requests[0]
        .to_string()
        .contains("Google visible answer before switching"));
    assert_foreign_thought_absent(
        &legacy_requests[0],
        "Google private thought before switching",
        "GOOGLE-SWITCH-SIGNATURE",
    );
}
