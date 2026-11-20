//! S-14: deterministic shell dispatch reaches the served provider-choice gate.
//! The historical model refused before issuing this request, so its chat was
//! not evidence that the HTTP gate would refuse a tool carrying the secret.
#![cfg(unix)]

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::middleware::Next;
use biorouter::agents::extension::ExtensionConfig;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::paths::Paths;
use biorouter::config::permission::PermissionManager;
use biorouter::config::{with_config_overrides, BioRouterMode, Config};
use biorouter::conversation::message::{ActionRequiredData, Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::permission::permission_confirmation::PrincipalType;
use biorouter::permission::{Permission, PermissionConfirmation};
use biorouter::privacy::refusal::PrivacyRefusal;
use biorouter::privacy::ProviderTier;
use biorouter::providers::google::GoogleProvider;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use biorouter_server::auth::{
    install_served_operator, provider_choice_proof, user_action_proof, UserActionProof,
};
use biorouter_server::state::AppState;
use futures::StreamExt;
use serde_json::{json, Value};
use serial_test::serial;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MODEL: &str = "gemini-3.8-flash";
const SECRET: &str = "s14-synthetic-daemon-secret";
const BROWSER_TOKEN: &str = "s14-synthetic-served-page-token";
const ORIGINAL_PROVIDER_BODY: &str =
    r#"{"provider":"baseten","model":"deepseek-ai/DeepSeek-V3.1"}"#;
const BOUND: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct ObservedRequest {
    path: String,
    method: String,
    body: Value,
    carried_secret: bool,
    person_proof: UserActionProof,
    status: StatusCode,
}

struct ServedGate {
    url: String,
    observed: Arc<Mutex<Vec<ObservedRequest>>>,
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for ServedGate {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

fn launch_posture() {
    assert!(Paths::path_root_override().is_some());
    assert!(biorouter::privacy::privacy_tiers_enabled());
    let config = Config::global();
    config
        .set_biorouter_provider_and_model("google", MODEL)
        .unwrap();
    config
        .set_secret("BASETEN_API_KEY", &"s14-synthetic-baseten-key")
        .unwrap();
    config
        .set_secret("S14_SENTINEL", &"s14-unchanged-secret")
        .unwrap();
    // A key a built-in provider declares plaintext: `/config/read` answers only
    // declared settings as themselves, and an undeclared key as `null` (#532).
    config
        .set_param("OPENAI_PROJECT", "visible control")
        .unwrap();
    install_served_operator(BROWSER_TOKEN.into(), ProviderTier::Public);
    biorouter_server::auth::adopt_served_operator_capability(ProviderTier::Public);
    biorouter_server::launch::record_launch_state(false);
    assert_eq!(
        user_action_proof(&HeaderMap::new()),
        UserActionProof::NoKeyInstalled
    );
}

async fn serve_gate() -> ServedGate {
    launch_posture();
    let state = AppState::new().await.unwrap();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let captured = observed.clone();
    let app = biorouter_server::routes::config_management::routes(state.clone())
        .layer(axum::middleware::from_fn_with_state(
            SECRET.to_string(),
            biorouter_server::auth::check_token,
        ))
        .layer(axum::middleware::from_fn(
            move |request: Request<Body>, next: Next| {
                let captured = captured.clone();
                async move {
                    let (parts, body) = request.into_parts();
                    let bytes = axum::body::to_bytes(body, 16_384).await.unwrap();
                    let observation = ObservedRequest {
                        path: parts.uri.path().to_string(),
                        method: parts.method.to_string(),
                        body: serde_json::from_slice(&bytes).unwrap(),
                        carried_secret: parts
                            .headers
                            .get("x-secret-key")
                            .is_some_and(|value| value == SECRET),
                        person_proof: provider_choice_proof(&parts.headers),
                        status: StatusCode::CONTINUE,
                    };
                    let response = next
                        .run(Request::from_parts(parts, Body::from(bytes)))
                        .await;
                    captured.lock().unwrap().push(ObservedRequest {
                        status: response.status(),
                        ..observation
                    });
                    response
                }
            },
        ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let cancel = CancellationToken::new();
    let stopped = cancel.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(stopped.cancelled_owned())
            .await
            .unwrap();
    });
    ServedGate {
        url,
        observed,
        cancel,
        task,
    }
}

#[derive(Debug, PartialEq)]
struct ConfigSnapshot {
    config_file: Vec<u8>,
    secrets_file: Vec<u8>,
    values: HashMap<String, Value>,
    secrets: HashMap<String, Value>,
}

fn snapshot() -> ConfigSnapshot {
    let config = Config::global();
    ConfigSnapshot {
        config_file: std::fs::read(config.path()).expect("sandbox config bytes"),
        secrets_file: std::fs::read(Paths::config_dir().join("secrets.yaml"))
            .expect("sandbox secrets bytes"),
        values: config.all_values().unwrap(),
        secrets: config.all_secrets().unwrap(),
    }
}

struct DispatchResult {
    output: String,
    before: ConfigSnapshot,
    after: ConfigSnapshot,
}

fn curl_command(gate: &ServedGate, route: &str, body: &str, person: bool) -> String {
    let proof = if person {
        format!(
            "--header 'Cookie: biorouter_session={BROWSER_TOKEN}' \
             --header 'Sec-Fetch-Site: same-origin' --header 'Sec-Fetch-Mode: cors' \
             --header 'Sec-Fetch-Dest: empty'"
        )
    } else {
        String::new()
    };
    format!(
        "/usr/bin/curl --disable --noproxy '*' --connect-timeout 2 --max-time 5 --silent --show-error \
         --request POST '{url}{route}' --header 'X-Secret-Key: {SECRET}' \
         --header 'Content-Type: application/json' {proof} --data '{body}' \
         --write-out '\nS14_HTTP_STATUS:%{{http_code}}\n'",
        url = gate.url,
    )
}

fn sse(part: Value) -> String {
    let response = json!({"candidates": [{"content": {"role": "model", "parts": [part]}}]});
    let usage = json!({"usageMetadata": {"promptTokenCount": 12, "candidatesTokenCount": 3, "totalTokenCount": 15}});
    format!("data: {response}\n\ndata: {usage}\n\ndata: [DONE]\n\n")
}

async fn dispatch_shell(command: String) -> DispatchResult {
    let provider_server = MockServer::start().await;
    let replies = [
        sse(json!({"functionCall": {"name": "developer__shell", "args": {"command": command}}})),
        sse(json!({"text": "Synthetic configuration gate checked."})),
    ];
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = requests.clone();
    Mock::given(method("POST"))
        .and(path(format!(
            "/v1beta/models/{MODEL}:streamGenerateContent"
        )))
        .respond_with(move |request: &wiremock::Request| {
            let mut captured = captured.lock().unwrap();
            let index = captured.len();
            captured.push(serde_json::from_slice(&request.body).unwrap());
            match replies.get(index) {
                Some(reply) => {
                    ResponseTemplate::new(200).set_body_raw(reply.clone(), "text/event-stream")
                }
                None => ResponseTemplate::new(500).set_body_string("unexpected provider request"),
            }
        })
        .mount(&provider_server)
        .await;
    let overrides = HashMap::from([
        ("GOOGLE_API_KEY".into(), "s14-loopback-provider-key".into()),
        ("GOOGLE_HOST".into(), provider_server.uri()),
    ]);
    let output = with_config_overrides(overrides, async {
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
        agent.add_extension(ExtensionConfig::Builtin {
            name: "developer".into(),
            description: "S-14 isolated shell dispatch".into(),
            display_name: None,
            timeout: Some(10),
            bundled: Some(true),
            available_tools: vec!["shell".into()],
        }).await.unwrap();
        let session = manager.create_session(work.path().to_path_buf(), "S-14 dispatch fixture".into(), SessionType::Hidden).await.unwrap();
        agent.update_provider(Arc::new(GoogleProvider::from_env(ModelConfig::new_or_fail(MODEL)).await.unwrap()), &session.id).await.unwrap();
        let before = snapshot();
        let stream = agent.reply(Message::user().with_text("Run the single synthetic configuration probe and report its HTTP status and body only."), SessionConfig {
            id: session.id.clone(), schedule_id: None, max_turns: Some(3), max_tool_calls: Some(3), budget: None, retry_config: None, reasoning_effort: None,
        }, None).await.unwrap();
        tokio::pin!(stream);
        tokio::time::timeout(BOUND, async {
            while let Some(event) = stream.next().await {
                match event.unwrap() {
                    AgentEvent::Message(message) => for content in message.content {
                        if let MessageContent::ActionRequired(action) = content {
                            if let ActionRequiredData::ToolConfirmation { id, .. } = action.data {
                                agent.handle_confirmation(id, PermissionConfirmation {principal_type: PrincipalType::Tool, permission: Permission::AllowOnce}).await;
                            }
                        }
                    },
                    AgentEvent::TurnAborted { code, message, .. } => panic!("S-14 dispatch aborted: {code:?}: {message}"),
                    _ => {}
                }
            }
        }).await.expect("bounded actual shell dispatch");
        let stored = manager.get_session(&session.id, true).await.unwrap().conversation.unwrap();
        let results: Vec<_> = stored.iter().flat_map(|message| &message.content).filter_map(MessageContent::as_tool_response).collect();
        assert_eq!(results.len(), 1, "exactly one real shell tool must answer");
        let result = results[0].tool_result.as_ref().expect("shell dispatch itself succeeded");
        assert_ne!(result.is_error, Some(true));
        let output = serde_json::to_string(result).unwrap();
        manager.delete_session(&session.id).await.unwrap();
        DispatchResult { output, before, after: snapshot() }
    }).await;
    let requests = requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        2,
        "the model must receive the actual tool result"
    );
    assert!(requests[1].to_string().contains("S14_HTTP_STATUS:"));
    output
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn serve_secret_tool_dispatch_refuses_original_provider_change_without_writes() {
    let gate = serve_gate().await;
    let output = dispatch_shell(curl_command(
        &gate,
        "/config/set_provider",
        ORIGINAL_PROVIDER_BODY,
        false,
    ))
    .await;
    assert!(output.output.contains("S14_HTTP_STATUS:409"));
    assert!(output.output.contains(
        &PrivacyRefusal::CapabilityConfigNeedsUser {
            key: "BIOROUTER_PROVIDER".into()
        }
        .to_string()
    ));
    assert_eq!(
        output.after, output.before,
        "the refused tool request wrote config or secrets"
    );
    let observed = gate.observed.lock().unwrap();
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].path, "/config/set_provider");
    assert_eq!(observed[0].method, "POST");
    assert_eq!(
        observed[0].body,
        serde_json::from_str::<Value>(ORIGINAL_PROVIDER_BODY).unwrap()
    );
    assert!(observed[0].carried_secret);
    assert_eq!(observed[0].person_proof, UserActionProof::NoKeyInstalled);
    assert_eq!(observed[0].status, StatusCode::CONFLICT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn serve_secret_tool_dispatch_admits_authorized_provider_change() {
    let gate = serve_gate().await;
    let output = dispatch_shell(curl_command(
        &gate,
        "/config/set_provider",
        ORIGINAL_PROVIDER_BODY,
        true,
    ))
    .await;
    assert!(output.output.contains("S14_HTTP_STATUS:200"));
    assert_eq!(
        Config::global()
            .get_param::<String>("BIOROUTER_PROVIDER")
            .unwrap(),
        "baseten"
    );
    assert_eq!(
        Config::global()
            .get_param::<String>("BIOROUTER_MODEL")
            .unwrap(),
        "deepseek-ai/DeepSeek-V3.1"
    );
    assert_eq!(output.after.secrets, output.before.secrets);
    assert_eq!(output.after.secrets_file, output.before.secrets_file);
    let observed = gate.observed.lock().unwrap();
    assert_eq!(observed.len(), 1);
    assert!(observed[0].carried_secret);
    assert_eq!(observed[0].person_proof, UserActionProof::Proven);
    assert_eq!(observed[0].status, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn serve_secret_tool_dispatch_allows_nonsecret_configuration_read() {
    let gate = serve_gate().await;
    let output = dispatch_shell(curl_command(
        &gate,
        "/config/read",
        r#"{"key":"OPENAI_PROJECT","is_secret":false}"#,
        false,
    ))
    .await;
    assert!(output.output.contains("S14_HTTP_STATUS:200"));
    assert!(output.output.contains("visible control"));
    assert_eq!(output.after, output.before);
    let observed = gate.observed.lock().unwrap();
    assert_eq!(observed.len(), 1);
    assert!(observed[0].carried_secret);
    assert_eq!(observed[0].person_proof, UserActionProof::NoKeyInstalled);
    assert_eq!(observed[0].status, StatusCode::OK);
}
