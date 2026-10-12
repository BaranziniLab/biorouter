//! Diagnostics bundle 20261006_2, replayed against the real Ollama provider.
//!
//! The report: Biorouter 1.92.0 on an Intel Mac, Ollama with llama3.1:8b and
//! qwen2.5-coder:7b, a chat with Code Execution on. Every request that needed a
//! tool came back as JSON, and the person "decided just to ask for the code and
//! run it myself". The cause (fixed in #393) was the `/v1` route, which cannot
//! set Ollama's context window: each ~160 KB request was cut to 2,050 tokens,
//! so the model saw the end of the tool list and the question, and wrote calls
//! to tools it could not see as text.
//!
//! Each test serves one of the reporter's exact assistant replies
//! (`tests/fixtures/ollama/diagnostics_20261006_2/replies.json`, the account
//! name in paths replaced) from a mock Ollama, with the reporter's measured
//! usage where it matters, and asserts what Biorouter does with it today: the
//! cut turn is refused, a call to an offered tool written as text is asked for
//! once, a call nothing can run is shown and never run (the agent adds a line
//! saying so; `agents::reply_parts` tests that), an answer is an answer, and a
//! first prompt that outlasts the idle bound says which setting to raise and is
//! sent once.
//!
//! Nothing here reaches a network beyond the loopback mock server.

use biorouter::agents::workspace_extension::WorkspaceClient;
use biorouter::config::declarative_providers::{DeclarativeProviderConfig, ProviderEngine};
use biorouter::conversation::message::{Message, MessageContent, ToolRequest};
use biorouter::model::ModelConfig;
use biorouter::providers::base::{Provider, ProviderStreamItem};
use biorouter::providers::errors::{ProviderError, ProviderErrorKind};
use biorouter::providers::ollama::OllamaProvider;
use futures::StreamExt;
use rmcp::model::Tool;
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Sandbox this binary's config root before any test runs: the request log
/// directory and `Config::global()` resolve from `BIOROUTER_PATH_ROOT` the
/// first time anything touches them. An outer root wins.
#[ctor::ctor]
fn sandbox_config_root_for_this_test_binary() {
    if biorouter::config::paths::Paths::path_root_override().is_some() {
        return;
    }
    let root = tempfile::TempDir::new().expect("a scratch config root");
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let _ = ROOT.set(root);
}

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ollama");

/// The reporter's exact reply at `index` in the session.
fn reply(index: u64) -> String {
    let doc: Value = serde_json::from_str(
        &std::fs::read_to_string(format!("{FIXTURES}/diagnostics_20261006_2/replies.json"))
            .unwrap(),
    )
    .unwrap();
    doc["replies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|reply| reply["index"] == index)
        .and_then(|reply| reply["reply"].as_str())
        .unwrap_or_else(|| panic!("no reply {index} in the fixture"))
        .to_string()
}

fn tool(name: &str, schema: Value) -> Tool {
    Tool::new(
        name.to_string(),
        format!("{name} tool"),
        rmcp::model::object(schema),
    )
}

/// What the reporter's chat offered the model directly with Code Execution
/// on: its three tools (schemas mirroring its parameter structs), a todo tool,
/// and the real workspace tools. Developer and Agent Drafter were reachable
/// only from a script.
fn code_execution_roster() -> Vec<Tool> {
    let mut tools = vec![
        tool(
            "code_execution__execute_code",
            json!({"type": "object", "properties": {
                "code": {"type": "string"}, "tool_graph": {"type": "array"}
            }, "required": ["code"]}),
        ),
        tool(
            "code_execution__read_module",
            json!({"type": "object", "properties": {"module_path": {"type": "string"}}, "required": ["module_path"]}),
        ),
        tool(
            "code_execution__search_modules",
            json!({"type": "object", "properties": {
                "terms": {"anyOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}}]},
                "regex": {"type": "boolean"}
            }, "required": ["terms"]}),
        ),
        tool(
            "todo__todo_write",
            json!({"type": "object", "properties": {"content": {"type": "string"}}, "required": ["content"]}),
        ),
    ];
    for mut workspace in WorkspaceClient::get_tools() {
        workspace.name = format!("workspace__{}", workspace.name).into();
        tools.push(workspace);
    }
    tools
}

/// The same chat with Code Execution off: Developer offered directly.
fn direct_roster() -> Vec<Tool> {
    let mut tools: Vec<Tool> = code_execution_roster()
        .into_iter()
        .filter(|tool| !tool.name.starts_with("code_execution__"))
        .collect();
    tools.push(tool(
        "developer__shell",
        json!({"type": "object", "properties": {"command": {"type": "string"}}, "required": ["command"]}),
    ));
    tools
}

/// `/api/show` for a model that calls tools, trained on `context_length`.
fn show(context_length: u64, template: Option<String>) -> Value {
    let mut body = json!({
        "capabilities": ["completion", "tools"],
        "model_info": {"general.architecture": "llama", "llama.context_length": context_length},
    });
    if let Some(template) = template {
        body["template"] = json!(template);
    }
    body
}

/// One non-streamed native body answering `content`, with the prompt counts
/// the server reported.
fn text_body(content: &str, prompt: u64, cached: u64) -> String {
    json!({
        "model": "model",
        "message": {"role": "assistant", "content": content},
        "done": true, "done_reason": "stop",
        "prompt_eval_count": prompt, "prompt_eval_cached_count": cached, "eval_count": 120,
    })
    .to_string()
}

/// One native body making a structured call.
fn structured_call(name: &str, arguments: Value) -> String {
    json!({
        "model": "model",
        "message": {"role": "assistant", "content": "",
                    "tool_calls": [{"id": "call_ok", "function": {"name": name, "arguments": arguments}}]},
        "done": true, "done_reason": "stop", "prompt_eval_count": 31_000, "eval_count": 40,
    })
    .to_string()
}

/// A mock Ollama whose `/api/chat` answers `replies` in order (the last one
/// repeats), each after `delay`, and a provider pointed at it with an idle
/// bound of `timeout_seconds`.
async fn serve(
    model: &str,
    show: Value,
    replies: &[(u16, String)],
    delay: std::time::Duration,
    timeout_seconds: u64,
) -> (MockServer, OllamaProvider) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/show"))
        .respond_with(ResponseTemplate::new(200).set_body_json(show))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/version"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"version": "0.40.2"})))
        .mount(&server)
        .await;
    for (index, (status, body)) in replies.iter().enumerate() {
        let mock = Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(
                ResponseTemplate::new(*status)
                    .set_delay(delay)
                    .set_body_raw(body.clone(), "application/x-ndjson"),
            );
        let mock = if index + 1 < replies.len() {
            mock.up_to_n_times(1)
        } else {
            mock
        };
        mock.mount(&server).await;
    }
    let provider = OllamaProvider::from_custom_config(
        ModelConfig::new_or_fail(&format!("{model}-{}", uuid::Uuid::new_v4().simple())),
        DeclarativeProviderConfig {
            name: "ollama-diagnostics-20261006-2".to_string(),
            engine: ProviderEngine::Ollama,
            display_name: "Ollama diagnostics 20261006_2".to_string(),
            description: None,
            api_key_env: "NOT_USED".to_string(),
            base_url: server.uri(),
            models: vec![],
            headers: None,
            timeout_seconds: Some(timeout_seconds),
            supports_streaming: Some(true),
        },
    )
    .expect("the provider constructs");
    (server, provider)
}

async fn serve_replies(model: &str, replies: &[(u16, String)]) -> (MockServer, OllamaProvider) {
    serve(
        model,
        show(131_072, None),
        replies,
        std::time::Duration::ZERO,
        10,
    )
    .await
}

async fn chat_requests(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.url.path() == "/api/chat")
        .map(|request| serde_json::from_slice(&request.body).unwrap())
        .collect()
}

/// The user message the provider sent last in `request`: on a corrective
/// retry, the question it asked.
fn last_user_text(request: &Value) -> String {
    request["messages"]
        .as_array()
        .and_then(|messages| messages.last())
        .and_then(|message| message["content"].as_str())
        .unwrap_or_default()
        .to_string()
}

struct Turn {
    items: Vec<ProviderStreamItem>,
    error: Option<ProviderError>,
}

impl Turn {
    fn text(&self) -> String {
        self.items
            .iter()
            .filter_map(|(message, _, _)| message.as_ref())
            .map(Message::as_concat_text)
            .collect()
    }

    fn calls(&self) -> Vec<&ToolRequest> {
        self.items
            .iter()
            .filter_map(|(message, _, _)| message.as_ref())
            .flat_map(|message| message.content.iter())
            .filter_map(MessageContent::as_tool_request)
            .collect()
    }

    fn call_names(&self) -> Vec<String> {
        self.calls()
            .iter()
            .map(|call| call.tool_call.as_ref().unwrap().name.to_string())
            .collect()
    }
}

async fn turn(provider: &OllamaProvider, system: &str, user: &str, tools: &[Tool]) -> Turn {
    let mut stream = match provider
        .stream(system, &[Message::user().with_text(user)], tools)
        .await
    {
        Ok(stream) => stream,
        Err(error) => {
            return Turn {
                items: Vec::new(),
                error: Some(error),
            }
        }
    };
    let mut items = Vec::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(item) => items.push(item),
            Err(error) => {
                return Turn {
                    items,
                    error: Some(error),
                }
            }
        }
    }
    Turn { items, error: None }
}

/// The reporter's first turn as 1.92.0 sent it: a 164,005-byte request
/// (`llm_request.0.jsonl`) that Ollama answered from 2,046 new and 4 cached
/// prompt tokens, its 4,096-token default window keeping 4 head tokens and the
/// last 2,046. Today the request asks for the window it needs and refuses a
/// cut, and a reply that comes back cut anyway is an error, never the JSON the
/// reporter read.
#[tokio::test]
async fn the_reporters_cut_first_turn_is_refused_and_the_window_is_asked_for() {
    let template =
        std::fs::read_to_string(format!("{FIXTURES}/templates/llama3.1_8b.tmpl")).unwrap();
    let (server, provider) = serve(
        "llama3.1:8b",
        show(131_072, Some(template)),
        &[(200, text_body(&reply(1), 2046, 4))],
        std::time::Duration::ZERO,
        10,
    )
    .await;
    let system = "s".repeat(164_005 - 60);
    let turn = turn(
        &provider,
        &system,
        "write me some code to find the factorial of any number",
        &code_execution_roster(),
    )
    .await;
    let requests = chat_requests(&server).await;
    assert_eq!(requests.len(), 1);
    assert!(requests[0]["options"]["num_ctx"].as_u64().unwrap() >= 65_536);
    assert_eq!(requests[0]["truncate"], false);
    assert!(
        matches!(turn.error, Some(ProviderError::ContextLengthExceeded(_))),
        "{:?}",
        turn.error
    );
    assert!(turn.calls().is_empty());
    assert!(
        !turn.text().contains("text_editor"),
        "the cut reply is never shown"
    );
    assert_eq!(
        provider.get_model_config().context_limit,
        requests[0]["options"]["num_ctx"]
            .as_u64()
            .map(|n| n as usize),
        "the gauge and compaction read the window really asked for, not the name's guess"
    );
}

/// qwen2.5-coder:7b was trained on 32,768 tokens, and the reporter's prompt
/// was about 37,000. The request asks for the trained window and Ollama's
/// over-window refusal is a context error, sent once. (Whether a small model
/// should get a smaller prompt is an open product decision, not tested here.)
#[tokio::test]
async fn qwen25_coder_at_its_trained_window_refuses_the_reporters_prompt_size() {
    let overflow = r#"{"error":"{\"error\":{\"code\":400,\"message\":\"request (37274 tokens) exceeds the available context size (32768 tokens), try increasing it\",\"type\":\"exceed_context_size_error\",\"n_prompt_tokens\":37274,\"n_ctx\":32768}}"}"#;
    let (server, provider) = serve(
        "qwen2.5-coder:7b",
        show(32_768, None),
        &[(400, overflow.to_string())],
        std::time::Duration::ZERO,
        10,
    )
    .await;
    let turn = turn(&provider, "system", "hello", &code_execution_roster()).await;
    let requests = chat_requests(&server).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["options"]["num_ctx"], 32_768);
    assert!(
        matches!(turn.error, Some(ProviderError::ContextLengthExceeded(_))),
        "{:?}",
        turn.error
    );
}

/// Reply 9: `search_modules` without its prefix, `query` for `terms`. The
/// model is asked once, naming the real tool, and its call completes the turn.
#[tokio::test]
async fn reply_9_is_asked_for_once_naming_code_execution_search_modules() {
    let (server, provider) = serve_replies(
        "qwen2.5-coder:7b",
        &[
            (200, text_body(&reply(9), 31_000, 30_500)),
            (
                200,
                structured_call("code_execution__search_modules", json!({"terms": "llama"})),
            ),
        ],
    )
    .await;
    let turn = turn(
        &provider,
        "system",
        "can you actually execute llama3.1:8b against those notes",
        &code_execution_roster(),
    )
    .await;
    assert!(turn.error.is_none(), "{:?}", turn.error);
    assert_eq!(turn.call_names(), ["code_execution__search_modules"]);
    let requests = chat_requests(&server).await;
    assert_eq!(requests.len(), 2, "one corrective retry");
    let nudge = last_user_text(&requests[1]);
    assert!(
        nudge.contains("a call to 'search_modules' written as text")
            && nudge.contains("If you intended to call 'code_execution__search_modules'"),
        "{nudge}"
    );
}

/// Reply 11: two objects, `workspace_read_conversation` then
/// `workspace_read_panel`, each a real workspace tool without its prefix.
/// Read as one value the pair was neither a call nor an attempt, so the turn
/// ended on the JSON; now the model is asked once about both.
#[tokio::test]
async fn reply_11_two_calls_in_one_reply_are_asked_for_once() {
    let (server, provider) = serve_replies(
        "llama3.1:8b",
        &[
            (200, text_body(&reply(11), 31_000, 30_500)),
            (
                200,
                structured_call(
                    "workspace__workspace_read_conversation",
                    json!({"session_id": "20261006_2"}),
                ),
            ),
        ],
    )
    .await;
    let turn = turn(
        &provider,
        "system",
        "take the file called ~/temp and find any occurrences of blasts",
        &code_execution_roster(),
    )
    .await;
    assert!(turn.error.is_none(), "{:?}", turn.error);
    assert_eq!(
        turn.call_names(),
        ["workspace__workspace_read_conversation"]
    );
    let requests = chat_requests(&server).await;
    assert_eq!(requests.len(), 2, "one corrective retry");
    let nudge = last_user_text(&requests[1]);
    assert!(
        nudge.contains("calls to 'workspace_read_conversation' and 'workspace_read_panel'")
            && nudge.contains(
                "'workspace__workspace_read_conversation' and 'workspace__workspace_read_panel'"
            )
            && nudge.contains("otherwise answer in text"),
        "{nudge}"
    );
}

/// Replies 1 (`text_editor`), 7 (`shell`) and 15 (`agent_drafter`) in the
/// reporter's Code Execution chat: none names a tool that chat offered, so
/// nothing is run and nothing is asked. The reply is shown as it came; the
/// agent then adds a line saying nothing ran.
#[tokio::test]
async fn replies_1_7_and_15_in_a_code_execution_chat_are_shown_and_never_run() {
    for index in [1, 7, 15] {
        let (server, provider) = serve_replies(
            "llama3.1:8b",
            &[(200, text_body(&reply(index), 31_000, 30_500))],
        )
        .await;
        let turn = turn(&provider, "system", "do it", &code_execution_roster()).await;
        assert!(turn.error.is_none(), "reply {index}: {:?}", turn.error);
        assert!(turn.calls().is_empty(), "reply {index}");
        assert_eq!(turn.text().trim(), reply(index).trim(), "reply {index}");
        assert_eq!(
            chat_requests(&server).await.len(),
            1,
            "reply {index}: no retry"
        );
    }
}

/// Reply 7 where Developer IS offered directly: the same `shell` call is
/// asked for once, naming `developer__shell`.
#[tokio::test]
async fn reply_7_with_developer_offered_is_asked_for_once() {
    let (server, provider) = serve_replies(
        "llama3.1:8b",
        &[
            (200, text_body(&reply(7), 31_000, 30_500)),
            (
                200,
                structured_call("developer__shell", json!({"command": "cat ~/temp"})),
            ),
        ],
    )
    .await;
    let turn = turn(&provider, "system", "do it", &direct_roster()).await;
    assert!(turn.error.is_none(), "{:?}", turn.error);
    assert_eq!(turn.call_names(), ["developer__shell"]);
    let requests = chat_requests(&server).await;
    assert_eq!(requests.len(), 2);
    assert!(
        last_user_text(&requests[1]).contains("If you intended to call 'developer__shell'"),
        "{}",
        last_user_text(&requests[1])
    );
}

/// The reporter's answers: prose with code for the person to run (3, 5, 17,
/// 19) and the JSON the person asked for (13). Shown, never asked about.
#[tokio::test]
async fn the_reporters_answers_are_never_asked_about() {
    for index in [3, 5, 13, 17, 19] {
        let (server, provider) = serve_replies(
            "llama3.1:8b",
            &[(200, text_body(&reply(index), 31_000, 30_500))],
        )
        .await;
        let turn = turn(&provider, "system", "do it", &code_execution_roster()).await;
        assert!(turn.error.is_none(), "reply {index}: {:?}", turn.error);
        assert!(turn.calls().is_empty(), "reply {index}");
        assert_eq!(turn.text().trim(), reply(index).trim(), "reply {index}");
        assert_eq!(
            chat_requests(&server).await.len(),
            1,
            "reply {index}: no retry"
        );
    }
}

/// The reporter's machine read about 25 prompt tokens a second (llama3.1:8b)
/// with `OLLAMA_TIMEOUT` at 600 s, so a whole first prompt of about 37,000
/// tokens would be a quarter of an hour of silence before Ollama's first byte.
/// Scaled down (a 1 s bound, 3 s of silence): the person reads which setting to
/// raise and how large the prompt is, the request is sent once, and the agent
/// does not send it again either.
#[tokio::test]
async fn a_first_prompt_slower_than_the_idle_bound_names_the_bound_and_is_sent_once() {
    let (server, provider) = serve(
        "llama3.1:8b",
        show(131_072, None),
        &[(200, text_body("hello", 37_000, 0))],
        std::time::Duration::from_secs(3),
        1,
    )
    .await;
    let system = "s".repeat(160_000);
    let turn = turn(&provider, &system, "hello", &code_execution_roster()).await;
    let error = turn.error.expect("the bound fires before the first byte");
    let text = error.to_string();
    assert!(matches!(error, ProviderError::IdleTimeout(_)), "{error:?}");
    assert!(
        text.contains("sent nothing back for 1 second,")
            && text.contains("Raise timeout_seconds")
            && text.contains("about 4")
            && text.contains(",000 tokens"),
        "{text}"
    );
    assert_eq!(error.kind(), ProviderErrorKind::Network);
    assert!(
        !biorouter::agents::mistakes::is_recoverable(&error),
        "the agent must not send the whole prompt again behind the first"
    );
    assert_eq!(chat_requests(&server).await.len(), 1, "sent once");
}

// ---------------------------------------------------------------------------
// Through the agent: what the person reads, and what the model is sent next.
// ---------------------------------------------------------------------------

/// A model that answers its first request with the reporter's reply 7 and
/// every later one with "ok", recording what each request carried.
struct ReporterModel {
    requests: std::sync::Mutex<Vec<Vec<Message>>>,
}

#[async_trait::async_trait]
impl Provider for ReporterModel {
    async fn complete(
        &self,
        _system: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, biorouter::providers::base::ProviderUsage), ProviderError> {
        let first = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(messages.to_vec());
            requests.len() == 1
        };
        let text = if first { reply(7) } else { "ok".to_string() };
        let mut usage = biorouter::providers::base::ProviderUsage::new(
            "llama3.1:8b".to_string(),
            biorouter::providers::base::Usage::new(Some(31_000), Some(20), Some(31_020)),
        );
        usage.finish_reason = Some("stop".to_string());
        Ok((Message::assistant().with_text(text), usage))
    }

    async fn complete_with_model(
        &self,
        _model: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, biorouter::providers::base::ProviderUsage), ProviderError> {
        self.complete(system, messages, tools).await
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("llama3.1:8b")
    }

    fn metadata() -> biorouter::providers::base::ProviderMetadata {
        biorouter::providers::base::ProviderMetadata::empty()
    }

    fn get_name(&self) -> &str {
        "diagnostics-20261006-2-model"
    }
}

fn is_unrun_call_notice(message: &Message) -> bool {
    message.content.iter().any(|content| {
        matches!(
            content,
            MessageContent::SystemNotification(note) if note.msg.starts_with("Nothing ran")
        )
    })
}

/// End to end through the agent, on the reporter's reply 7 (`shell` written as
/// JSON) in a turn that offered tools: the reply is shown and stored as it
/// came, one line after it tells the person that nothing ran, that line is
/// stored for the person only, and the model's next request does not carry it.
#[tokio::test]
async fn the_agent_tells_the_person_nothing_ran_and_keeps_it_from_the_model() {
    use biorouter::agents::extension::ExtensionConfig;
    use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};

    let work = tempfile::TempDir::new().unwrap();
    let data = tempfile::TempDir::new().unwrap();
    let permissions = tempfile::TempDir::new().unwrap();
    let manager = std::sync::Arc::new(biorouter::session::SessionManager::new(
        data.path().to_path_buf(),
    ));
    let agent = Agent::with_config(AgentConfig::new(
        manager.clone(),
        std::sync::Arc::new(biorouter::config::permission::PermissionManager::new(
            permissions.path().to_path_buf(),
        )),
        None,
        biorouter::config::BioRouterMode::Auto,
    ));
    let session = manager
        .create_session(
            work.path().to_path_buf(),
            "diagnostics 20261006_2".to_string(),
            biorouter::session::session_manager::SessionType::Hidden,
        )
        .await
        .unwrap();
    let model = std::sync::Arc::new(ReporterModel {
        requests: std::sync::Mutex::new(Vec::new()),
    });
    agent
        .update_provider(model.clone(), &session.id)
        .await
        .unwrap();
    agent
        .add_extension(ExtensionConfig::Frontend {
            name: "notes".to_string(),
            description: "Read the person's notes".to_string(),
            tools: vec![tool(
                "notes__read",
                json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            )],
            instructions: None,
            bundled: None,
            available_tools: Vec::new(),
        })
        .await
        .unwrap();
    let config = || SessionConfig {
        id: session.id.clone(),
        schedule_id: None,
        max_turns: Some(3),
        max_tool_calls: Some(3),
        budget: None,
        retry_config: None,
        reasoning_effort: None,
    };

    let mut shown = Vec::new();
    for user in ["count the word urine in ~/temp", "thanks"] {
        let stream = agent
            .reply(Message::user().with_text(user), config(), None)
            .await
            .unwrap();
        tokio::pin!(stream);
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            while let Some(event) = stream.next().await {
                if let AgentEvent::Message(message) = event.unwrap() {
                    shown.push(message);
                }
            }
        })
        .await
        .expect("a bounded turn");
    }

    let notices: Vec<&Message> = shown
        .iter()
        .filter(|message| is_unrun_call_notice(message))
        .collect();
    assert_eq!(
        notices.len(),
        1,
        "one notice, after the first turn: {shown:?}"
    );
    let MessageContent::SystemNotification(note) = &notices[0].content[0] else {
        unreachable!()
    };
    assert!(
        note.msg.contains("a call to 'shell' written as text"),
        "{}",
        note.msg
    );
    assert!(shown
        .iter()
        .any(|message| message.as_concat_text() == reply(7)));

    let stored = manager
        .get_session(&session.id, true)
        .await
        .unwrap()
        .conversation
        .unwrap();
    let stored_notice = stored
        .iter()
        .find(|message| is_unrun_call_notice(message))
        .expect("the notice is stored, so a reload shows it");
    assert!(stored_notice.is_user_visible() && !stored_notice.is_agent_visible());

    let requests = model.requests.lock().unwrap();
    let second = requests
        .iter()
        .find(|request| {
            request
                .iter()
                .any(|message| message.as_concat_text().contains("thanks"))
        })
        .unwrap_or_else(|| panic!("the second turn reached the model: {requests:?}"));
    // Providers are handed the whole history and every wire format keeps only
    // the agent-visible messages (the mechanism compaction relies on too), so
    // what the model is sent is exactly this filter.
    let sent_to_the_model: Vec<&Message> = second
        .iter()
        .filter(|message| message.is_agent_visible())
        .collect();
    assert!(
        sent_to_the_model
            .iter()
            .all(|message| !is_unrun_call_notice(message)
                && !message.as_concat_text().contains("Nothing ran")),
        "the notice is the person's, never the model's: {sent_to_the_model:?}"
    );
    assert!(
        sent_to_the_model
            .iter()
            .any(|message| message.as_concat_text() == reply(7)),
        "the reply itself stays in the model's history"
    );
    // And through Ollama's own request builder, which is what the reporter's
    // model was sent.
    let wire = biorouter::providers::formats::ollama::create_request(
        &biorouter::providers::formats::ollama::ChatRequest {
            model: "llama3.1:8b",
            system: "system",
            messages: second,
            tools: &[],
            stream: true,
            num_ctx: 65_536,
            temperature: None,
            num_predict: None,
            think: None,
            keep_alive: None,
            vision: false,
            calls_without_text: false,
        },
    )
    .unwrap()
    .to_string();
    assert!(!wire.contains("Nothing ran"), "{wire}");
}
