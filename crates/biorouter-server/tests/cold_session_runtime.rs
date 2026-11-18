//! First turns after a daemon restart restore the saved tool roster without a resume request.

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::Router;
use biorouter::agents::ExtensionConfig;
use biorouter::conversation::message::{Message, MessageContent};
use biorouter::execution::manager::AgentManager;
use biorouter::model::ModelConfig;
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::extension_data::TodoState;
use biorouter::session::{EnabledExtensionsState, ExtensionState, Session, SessionType};
use biorouter_server::state::AppState;
use rmcp::model::{CallToolRequestParams, Tool};
use serde_json::{json, Value};
use serial_test::serial;
use tokio::sync::oneshot;
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

const USER_ACTION_KEY: &str = "cold-session-runtime-proof";
const BOUND: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
struct ModelCall {
    system: String,
    tools: Vec<String>,
}

struct RosterProvider {
    target: Option<(String, Value)>,
    calls: Mutex<Vec<ModelCall>>,
}

impl RosterProvider {
    fn new(target: Option<(&str, Value)>) -> Arc<Self> {
        Arc::new(Self {
            target: target.map(|(name, arguments)| (name.to_owned(), arguments)),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<ModelCall> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl Provider for RosterProvider {
    async fn complete(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.calls.lock().unwrap().push(ModelCall {
            system: system.to_owned(),
            tools: tools.iter().map(|tool| tool.name.to_string()).collect(),
        });
        if tools.is_empty() {
            return Ok((
                Message::assistant().with_text("Cold runtime fixture"),
                usage(),
            ));
        }
        let Some((name, arguments)) = &self.target else {
            return Ok((Message::assistant().with_text("Roster recorded"), usage()));
        };
        let marker = messages
            .iter()
            .rev()
            .flat_map(|message| message.content.iter())
            .filter_map(MessageContent::as_text)
            .find(|text| text.starts_with("cold-roster-"))
            .unwrap_or("cold-roster-fixture");
        let call_id = format!("{marker}-tool");
        let answered = messages.iter().any(|message| {
            message.content.iter().any(|content| {
                content
                    .as_tool_response()
                    .is_some_and(|response| response.id == call_id)
            })
        });
        if answered || !tools.iter().any(|tool| tool.name.as_ref() == name) {
            return Ok((Message::assistant().with_text("Roster checked"), usage()));
        }
        Ok((
            Message::assistant().with_tool_request(
                call_id,
                Ok(CallToolRequestParams {
                    task: None,
                    meta: None,
                    name: name.clone().into(),
                    arguments: arguments.as_object().cloned(),
                }),
            ),
            usage(),
        ))
    }

    async fn complete_with_model(
        &self,
        _model: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.complete(system, messages, tools).await
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("cold-roster-model")
    }

    fn metadata() -> ProviderMetadata {
        ProviderMetadata::new(
            "cold-roster",
            "Fixture",
            "Synthetic provider",
            "cold-roster-model",
            vec![],
            "",
            vec![],
        )
    }

    fn get_name(&self) -> &str {
        "cold-roster"
    }
}

fn usage() -> ProviderUsage {
    ProviderUsage::new(
        "cold-roster-model".into(),
        Usage::new(Some(1), Some(1), Some(2)),
    )
}

fn install_person_proof() {
    let digest = <sha2::Sha256 as sha2::Digest>::digest(USER_ACTION_KEY.as_bytes());
    biorouter_server::auth::install_user_action_digest(Some(digest.into()));
    let mut headers = HeaderMap::new();
    headers.insert("X-User-Action", USER_ACTION_KEY.parse().unwrap());
    assert!(biorouter_server::auth::is_user_action(&headers));
}

async fn state(provider: &Arc<RosterProvider>) -> Arc<AppState> {
    install_person_proof();
    let manager = AgentManager::instance().await.unwrap();
    manager
        .set_default_provider(Arc::clone(provider) as Arc<dyn Provider>)
        .await;
    AppState::new().await.unwrap()
}

async fn saved_session(
    state: &Arc<AppState>,
    extensions: Vec<ExtensionConfig>,
    kind: SessionType,
) -> (tempfile::TempDir, Session) {
    let workdir = tempfile::tempdir().unwrap();
    let mut session = state
        .session_manager()
        .create_session(
            workdir.path().to_path_buf(),
            "Cold roster fixture".into(),
            kind,
        )
        .await
        .unwrap();
    EnabledExtensionsState::new(extensions)
        .to_extension_data(&mut session.extension_data)
        .unwrap();
    state
        .session_manager()
        .update(&session.id)
        .extension_data(session.extension_data.clone())
        .apply()
        .await
        .unwrap();
    assert!(state.peek_agent(&session.id).await.is_none());
    (workdir, session)
}

fn todo() -> ExtensionConfig {
    ExtensionConfig::Platform {
        name: "todo".into(),
        description: "The saved checklist".into(),
        bundled: Some(true),
        available_tools: vec![],
    }
}

fn reply_app(state: &Arc<AppState>) -> Router {
    biorouter_server::routes::reply::routes(Arc::clone(state))
}

fn post(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("X-User-Action", USER_ACTION_KEY)
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn send(app: Router, request: Request<Body>) -> (StatusCode, String) {
    tokio::time::timeout(BOUND, async move {
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    })
    .await
    .expect("the HTTP turn settles")
}

fn reply(session_id: &str, turn: &str) -> Request<Body> {
    post(
        "/reply",
        json!({
            "session_id": session_id,
            "turn_id": turn,
            "user_message": Message::user().with_text(turn),
        }),
    )
}

async fn discard(state: &Arc<AppState>, session_id: &str) {
    AgentManager::instance()
        .await
        .unwrap()
        .remove_session(session_id)
        .await
        .unwrap();
    state
        .session_manager()
        .delete_session(session_id)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn a_cold_http_reply_can_call_the_saved_builtin_before_any_resume() {
    if !test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let provider = RosterProvider::new(Some((
        "todo__todo_write",
        json!({
            "content": "- [ ] Inspect saved runtime\n- [ ] Exercise the first turn\n- [ ] Verify the stored result"
        }),
    )));
    let state = state(&provider).await;
    let (_workdir, session) = saved_session(&state, vec![todo()], SessionType::User).await;
    let (status, body) = send(reply_app(&state), reply(&session.id, "cold-roster-builtin")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let calls = provider.calls();
    assert!(
        calls
            .first()
            .is_some_and(|call| call.tools.iter().any(|name| name == "todo__todo_write")),
        "first model call lost the saved checklist: {calls:?}"
    );
    let stored = state
        .session_manager()
        .get_session(&session.id, true)
        .await
        .unwrap();
    let checklist = TodoState::from_extension_data(&stored.extension_data)
        .expect("the model's real tool call persisted its checklist");
    assert_eq!(checklist.items.len(), 3);
    assert_eq!(checklist.items[0].text, "Inspect saved runtime");
    let roster = EnabledExtensionsState::from_extension_data(&stored.extension_data).unwrap();
    assert_eq!(
        roster.extensions,
        vec![todo()],
        "a cold turn must not replace the saved roster with today's defaults"
    );
    assert!(stored
        .conversation
        .unwrap()
        .messages()
        .iter()
        .any(|message| message.content.iter().any(|content| content
            .as_tool_response()
            .is_some_and(|response| response
                .tool_result
                .as_ref()
                .is_ok_and(|result| result.is_error != Some(true))))));
    discard(&state, &session.id).await;
}

#[derive(Clone)]
struct ProbeServer {
    starts: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
}

impl Respond for ProbeServer {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let Some(id) = body.get("id") else {
            return ResponseTemplate::new(202);
        };
        let result = match body["method"].as_str().unwrap() {
            "initialize" => {
                self.starts.fetch_add(1, Ordering::SeqCst);
                json!({"protocolVersion": body["params"]["protocolVersion"], "capabilities": {"tools": {}}, "serverInfo": {"name": "cold-roster-probe", "version": "1"}})
            }
            "tools/list" => {
                json!({"tools": [{"name": "ping", "description": "Read the synthetic probe", "inputSchema": {"type": "object", "properties": {}}, "annotations": {"readOnlyHint": true, "destructiveHint": false}}]})
            }
            "tools/call" => {
                self.calls.fetch_add(1, Ordering::SeqCst);
                json!({"content": [{"type": "text", "text": "cold-probe-pong"}], "isError": false})
            }
            "resources/list" => json!({"resources": []}),
            "prompts/list" => json!({"prompts": []}),
            other => panic!("unexpected MCP request {other}: {body}"),
        };
        ResponseTemplate::new(200)
            .set_body_json(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }
}

async fn probe() -> (MockServer, ProbeServer, ExtensionConfig) {
    let server = MockServer::start().await;
    let probe = ProbeServer {
        starts: Arc::new(AtomicUsize::new(0)),
        calls: Arc::new(AtomicUsize::new(0)),
    };
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(probe.clone())
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/mcp"))
        .respond_with(ResponseTemplate::new(405))
        .mount(&server)
        .await;
    let extension = ExtensionConfig::StreamableHttp {
        name: "cold_probe".into(),
        description: "Saved loopback fixture".into(),
        uri: format!("{}/mcp", server.uri()),
        envs: Default::default(),
        env_keys: vec![],
        headers: Default::default(),
        timeout: Some(5),
        bundled: None,
        available_tools: vec!["ping".into()],
    };
    (server, probe, extension)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn concurrent_cold_http_requests_attach_once_and_eviction_reloads_once() {
    if !test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let (_server, probe, extension) = probe().await;
    let provider = RosterProvider::new(Some(("cold_probe__ping", json!({}))));
    let state = state(&provider).await;
    let (_workdir, session) =
        saved_session(&state, vec![extension.clone()], SessionType::User).await;
    let app = reply_app(&state);
    let (first, second, resumed) = tokio::join!(
        send(app.clone(), reply(&session.id, "cold-roster-concurrent")),
        send(app.clone(), reply(&session.id, "cold-roster-concurrent")),
        send(
            biorouter_server::routes::agent::routes(Arc::clone(&state)),
            post(
                "/agent/resume",
                json!({
                    "session_id": session.id,
                    "load_model_and_extensions": true,
                })
            ),
        ),
    );
    for (status, body) in [first, second] {
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let (status, body) = resumed;
    assert_eq!(status, StatusCode::OK, "{body}");
    let resumed: Value = serde_json::from_str(&body).unwrap();
    assert!(resumed["initialization_error"].is_null(), "{body}");
    let results = resumed["extension_results"]
        .as_array()
        .expect("the explicit resume reports extension initialization results");
    assert_eq!(results.len(), 1, "{body}");
    assert_eq!(results[0]["name"], "cold_probe", "{body}");
    assert_eq!(results[0]["success"], true, "{body}");
    assert_eq!(
        probe.starts.load(Ordering::SeqCst),
        1,
        "the saved MCP server must initialize once per agent"
    );
    assert_eq!(
        probe.calls.load(Ordering::SeqCst),
        1,
        "duplicate HTTP attachment must not execute the tool twice"
    );
    AgentManager::instance()
        .await
        .unwrap()
        .remove_session(&session.id)
        .await
        .unwrap();
    assert!(state.peek_agent(&session.id).await.is_none());
    for turn in ["cold-roster-after-eviction", "cold-roster-already-ready"] {
        let (status, body) = send(app.clone(), reply(&session.id, turn)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    assert_eq!(
        probe.starts.load(Ordering::SeqCst),
        2,
        "a resumed runtime must not reconnect its MCP server on every turn"
    );
    assert_eq!(probe.calls.load(Ordering::SeqCst), 3);
    let stored = state
        .session_manager()
        .get_session(&session.id, false)
        .await
        .unwrap();
    assert_eq!(
        EnabledExtensionsState::from_extension_data(&stored.extension_data)
            .unwrap()
            .extensions,
        vec![extension]
    );
    discard(&state, &session.id).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn cold_subagent_http_reply_keeps_its_delegated_tools() {
    if !test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let provider = RosterProvider::new(None);
    let state = state(&provider).await;
    let (_workdir, mut session) = saved_session(&state, vec![todo()], SessionType::SubAgent).await;
    session.extension_data.set_extension_state(
        "subagent_runtime_profile",
        "v2",
        json!({
            "format_version": 2, "system_prompt": "COLD_CHILD_ONLY", "sub_workflows": [],
            "extension_grants": [{"name": "todo", "kind": "platform", "tools": ["todo_update"]}]
        }),
    );
    state
        .session_manager()
        .update(&session.id)
        .extension_data(session.extension_data)
        .apply()
        .await
        .unwrap();
    let (status, body) = send(reply_app(&state), reply(&session.id, "cold-roster-child")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let calls = provider.calls();
    let first = calls
        .first()
        .expect("the restricted child reaches its provider");
    assert!(first.system.contains("COLD_CHILD_ONLY"));
    assert!(
        first.tools.iter().any(|name| name == "todo__todo_update"),
        "{first:?}"
    );
    assert!(
        !first.tools.iter().any(|name| name == "todo__todo_write"
            || name.starts_with("workspace__")
            || name.starts_with("developer__")),
        "a cold child gained tools outside its delegated profile: {first:?}"
    );
    discard(&state, &session.id).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn cancelling_while_the_saved_runtime_loads_settles_without_inference() {
    if !test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let provider = RosterProvider::new(None);
    let state = state(&provider).await;
    let (_workdir, session) = saved_session(&state, vec![todo()], SessionType::User).await;
    let (release, loaded) = oneshot::channel::<()>();
    state
        .set_extension_loading_task(
            session.id.clone(),
            tokio::spawn(async move {
                let _ = loaded.await;
                vec![]
            }),
        )
        .await;
    let app = reply_app(&state);
    let task = tokio::spawn(send(
        app.clone(),
        reply(&session.id, "cold-roster-cancelled"),
    ));
    tokio::time::timeout(BOUND, async {
        while !state.is_turn_active(&session.id) || state.peek_agent(&session.id).await.is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the turn starts while its extension task remains held");
    let (status, body) = send(
        app,
        post("/agent/cancel", json!({"session_id": session.id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    release.send(()).unwrap();
    let (status, body) = tokio::time::timeout(BOUND, task).await.unwrap().unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("cancelled"), "{body}");
    assert!(
        provider.calls().is_empty(),
        "Stop during loading must not start a model turn"
    );
    discard(&state, &session.id).await;
}
