use anyhow::Result;
use axum::response::Redirect;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, Request, State,
    },
    http::{StatusCode, Uri},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use base64::Engine;
use biorouter::agents::turn_abort::TurnFailed;
use biorouter::agents::{Agent, AgentEvent};
use biorouter::conversation::message::Message as BioRouterMessage;
use biorouter::session::session_manager::SessionType;
use futures::{sink::SinkExt, stream::StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{net::ToSocketAddrs, sync::Arc};
use tokio::sync::{Mutex, RwLock};
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tracing::error;
use webbrowser;

type CancellationStore = Arc<RwLock<std::collections::HashMap<String, tokio::task::AbortHandle>>>;

#[derive(Clone)]
struct AppState {
    agent: Arc<Agent>,
    cancellations: CancellationStore,
    auth_token: Option<String>,
    ws_token: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
enum WebSocketMessage {
    #[serde(rename = "message")]
    Message {
        content: String,
        session_id: String,
        timestamp: i64,
    },
    #[serde(rename = "cancel")]
    Cancel { session_id: String },
    #[serde(rename = "response")]
    Response {
        content: String,
        role: String,
        timestamp: i64,
    },
    #[serde(rename = "tool_request")]
    ToolRequest {
        id: String,
        tool_name: String,
        arguments: serde_json::Value,
    },
    #[serde(rename = "tool_response")]
    ToolResponse {
        id: String,
        result: serde_json::Value,
        is_error: bool,
    },
    #[serde(rename = "tool_confirmation")]
    ToolConfirmation {
        id: String,
        tool_name: String,
        arguments: serde_json::Value,
        needs_confirmation: bool,
    },
    #[serde(rename = "error")]
    Error { message: String },
    #[serde(rename = "thinking")]
    Thinking { message: String },
    #[serde(rename = "context_exceeded")]
    ContextExceeded { message: String },
    #[serde(rename = "cancelled")]
    Cancelled { message: String },
    #[serde(rename = "complete")]
    Complete { message: String },
}

async fn auth_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if req.uri().path() == "/api/health" {
        return Ok(next.run(req).await);
    }

    let Some(ref expected_token) = state.auth_token else {
        return Ok(next.run(req).await);
    };

    if let Some(auth_header) = req.headers().get("authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if let Some(token) = auth_str.strip_prefix("Bearer ") {
                if token_matches(token, expected_token) {
                    return Ok(next.run(req).await);
                }
            }

            if let Some(basic_token) = auth_str.strip_prefix("Basic ") {
                if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(basic_token) {
                    if let Ok(credentials) = String::from_utf8(decoded) {
                        // Basic auth is `username:password`; the token is the
                        // password. Compare that field in constant time rather
                        // than a `ends_with` substring check.
                        let password = credentials
                            .split_once(':')
                            .map(|(_, pw)| pw)
                            .unwrap_or(&credentials);
                        if token_matches(password, expected_token) {
                            return Ok(next.run(req).await);
                        }
                    }
                }
            }
        }
    }

    let mut response = Response::new("Authentication required".into());
    *response.status_mut() = StatusCode::UNAUTHORIZED;
    response.headers_mut().insert(
        "WWW-Authenticate",
        "Basic realm=\"Biorouter Web Interface\"".parse().unwrap(),
    );
    Ok(response)
}

fn is_loopback_address(host: &str) -> bool {
    (host, 0)
        .to_socket_addrs()
        .map(|mut addrs| addrs.any(|addr| addr.ip().is_loopback()))
        .unwrap_or(false)
}

/// Compare a candidate token to the expected one in constant time, so a network
/// peer can't recover the secret one byte at a time by timing the reply. Mirrors
/// the daemon's `secret_matches` (biorouter-server auth.rs). Length is not
/// secret — the tokens are fixed-width — so an early length check is fine.
fn token_matches(candidate: &str, expected: &str) -> bool {
    let (a, b) = (candidate.as_bytes(), expected.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn validate_network_auth(host: &str, auth_token: &Option<String>) {
    if !is_loopback_address(host) && auth_token.is_none() {
        eprintln!(
            "Error: --auth-token is required when the server is exposed on the network ({}).",
            host
        );
        eprintln!(
            "For security, use --auth-token <TOKEN> or bind to a local address (e.g., localhost)."
        );
        std::process::exit(1);
    }
}

fn get_provider_and_model() -> (String, String) {
    let config = biorouter::config::Config::global();

    let provider_name: String = match config.get_biorouter_provider() {
        Ok(p) => p,
        Err(_) => {
            eprintln!("No provider configured. Run 'biorouter configure' first");
            std::process::exit(1);
        }
    };

    let model: String = match config.get_biorouter_model() {
        Ok(m) => m,
        Err(_) => {
            eprintln!("No model configured. Run 'biorouter configure' first");
            std::process::exit(1);
        }
    };

    (provider_name, model)
}

async fn create_agent(provider_name: &str, model: &str) -> Result<Agent> {
    let model_config = biorouter::model::ModelConfig::new(model)?;

    let agent = Agent::new();

    let session_manager = agent.config.session_manager.clone();
    let init_session = session_manager
        .create_session(
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            "Web Agent Initialization".to_string(),
            SessionType::Hidden,
        )
        .await?;

    let provider = biorouter::providers::create(provider_name, model_config).await?;
    agent.update_provider(provider, &init_session.id).await?;

    let enabled_configs = biorouter::config::get_enabled_extensions();
    for config in enabled_configs {
        if let Err(e) = agent.add_extension(config.clone()).await {
            eprintln!("Warning: Failed to load extension {}: {}", config.name(), e);
        }
    }

    Ok(agent)
}

fn build_cors_layer(auth_token: &Option<String>, host: &str, port: u16) -> CorsLayer {
    if auth_token.is_none() {
        let allowed_origins = [
            "http://localhost:3000".parse().unwrap(),
            "http://127.0.0.1:3000".parse().unwrap(),
            format!("http://{}:{}", host, port).parse().unwrap(),
        ];
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(allowed_origins))
            .allow_methods(Any)
            .allow_headers(Any)
    } else {
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
    }
}

fn build_router(state: AppState, cors_layer: CorsLayer) -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/session/{session_name}", get(serve_session))
        .route("/ws", get(websocket_handler))
        .route("/api/health", get(health_check))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/{session_id}", get(get_session))
        .route("/static/{*path}", get(serve_static))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .layer(cors_layer)
        .with_state(state)
}

pub async fn handle_web(
    port: u16,
    host: String,
    open: bool,
    auth_token: Option<String>,
    _no_auth: bool, // kept for CLI backwards compatibility but no longer bypasses auth
) -> Result<()> {
    // Deprecated in favour of `biorouter serve`, which serves the real
    // Biorouter interface instead of this inherited single-page chat. Printed
    // rather than refused: someone is following instructions that were correct
    // when they were written, and stopping them dead would not help.
    eprintln!(
        "`biorouter web` is deprecated and will be removed in a future release.\n\
         Use `biorouter serve` instead -- it serves the full Biorouter interface, \n\
         with sessions, extensions and knowledge bases.\n"
    );
    validate_network_auth(&host, &auth_token);
    crate::logging::setup_logging(Some("biorouter-web"), None)?;

    let (provider_name, model) = get_provider_and_model();
    let agent = create_agent(&provider_name, &model).await?;

    let ws_token = if auth_token.is_none() {
        uuid::Uuid::new_v4().to_string()
    } else {
        String::new()
    };

    let state = AppState {
        agent: Arc::new(agent),
        cancellations: Arc::new(RwLock::new(std::collections::HashMap::new())),
        auth_token: auth_token.clone(),
        ws_token,
    };

    let cors_layer = build_cors_layer(&auth_token, &host, port);
    let app = build_router(state, cors_layer);

    let addr = (host.as_str(), port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow::anyhow!("Could not resolve address: {}", host))?;

    println!("\n🪿 Starting biorouter web server");
    println!("   Provider: {} | Model: {}", provider_name, model);
    println!(
        "   Working directory: {}",
        std::env::current_dir()?.display()
    );
    println!("   Server: http://{}", addr);
    println!("   Press Ctrl+C to stop\n");

    if open {
        let url = format!("http://{}", addr);
        if let Err(e) = webbrowser::open(&url) {
            eprintln!("Failed to open browser: {}", e);
        }
    }

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn serve_index(
    State(state): State<AppState>,
    uri: Uri,
) -> Result<Redirect, (http::StatusCode, String)> {
    let session = state
        .agent
        .config
        .session_manager
        .create_session(
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            "Web session".to_string(),
            SessionType::User,
        )
        .await
        .map_err(|err| (http::StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    let redirect_url = if let Some(query) = uri.query() {
        format!("/session/{}?{}", session.id, query)
    } else {
        format!("/session/{}", session.id)
    };

    Ok(Redirect::to(&redirect_url))
}

async fn serve_session(
    axum::extract::Path(session_name): axum::extract::Path<String>,
    State(state): State<AppState>,
) -> Html<String> {
    let html = include_str!("../../static/index.html");
    let html_with_session = html.replace(
        "<script src=\"/static/script.js\"></script>",
        &format!(
            "<script>window.BIOROUTER_SESSION_NAME = '{}'; window.BIOROUTER_WS_TOKEN = '{}';</script>\n    <script src=\"/static/script.js\"></script>",
            session_name,
            state.ws_token
        )
    );
    Html(html_with_session)
}

async fn serve_static(axum::extract::Path(path): axum::extract::Path<String>) -> Response {
    match path.as_str() {
        "style.css" => (
            [("content-type", "text/css")],
            include_str!("../../static/style.css"),
        )
            .into_response(),
        "script.js" => (
            [("content-type", "application/javascript")],
            include_str!("../../static/script.js"),
        )
            .into_response(),
        "img/logo_dark.png" => (
            [("content-type", "image/png")],
            include_bytes!("../../static/img/logo_dark.png").to_vec(),
        )
            .into_response(),
        "img/logo_light.png" => (
            [("content-type", "image/png")],
            include_bytes!("../../static/img/logo_light.png").to_vec(),
        )
            .into_response(),
        _ => (http::StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "service": "biorouter-web"
    }))
}

async fn list_sessions(State(state): State<AppState>) -> Json<serde_json::Value> {
    match state.agent.config.session_manager.list_sessions().await {
        Ok(sessions) => {
            let mut session_info = Vec::new();

            for session in sessions {
                session_info.push(serde_json::json!({
                    "name": session.id,
                    "path": session.id,
                    "description": session.name,
                    "message_count": session.message_count,
                    "working_dir": session.working_dir
                }));
            }
            Json(serde_json::json!({
                "sessions": session_info
            }))
        }
        Err(e) => Json(serde_json::json!({
            "error": e.to_string()
        })),
    }
}
async fn get_session(
    State(state): State<AppState>,
    axum::extract::Path(session_id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    match state
        .agent
        .config
        .session_manager
        .get_session(&session_id, true)
        .await
    {
        Ok(session) => Json(serde_json::json!({
            "metadata": session,
            "messages": session.conversation.unwrap_or_default().messages()
        })),
        Err(e) => Json(serde_json::json!({
            "error": e.to_string()
        })),
    }
}

#[derive(Deserialize)]
struct WsQuery {
    token: Option<String>,
}

async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(query): Query<WsQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    if state.auth_token.is_none() {
        let provided_token = query.token.as_deref().unwrap_or("");
        if !token_matches(provided_token, &state.ws_token) {
            tracing::warn!("WebSocket connection rejected: invalid token");
            return Err(StatusCode::FORBIDDEN);
        }
    }

    Ok(ws.on_upgrade(|socket| handle_socket(socket, state)))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (sender, mut receiver) = socket.split();
    let sender = Arc::new(Mutex::new(sender));

    while let Some(msg) = receiver.next().await {
        if let Ok(msg) = msg {
            match msg {
                Message::Text(text) => {
                    handle_text_message(&text.to_string(), &sender, &state).await;
                }
                Message::Close(_) => break,
                _ => {}
            }
        } else {
            break;
        }
    }
}

async fn handle_text_message(
    text: &str,
    sender: &Arc<Mutex<futures::stream::SplitSink<WebSocket, Message>>>,
    state: &AppState,
) {
    match serde_json::from_str::<WebSocketMessage>(text) {
        Ok(WebSocketMessage::Message {
            content,
            session_id,
            ..
        }) => {
            handle_user_message(content, session_id, sender.clone(), state).await;
        }
        Ok(WebSocketMessage::Cancel { session_id }) => {
            handle_cancel_message(session_id, sender, state).await;
        }
        Ok(_) => {}
        Err(e) => {
            error!("Failed to parse WebSocket message: {}", e);
        }
    }
}

async fn handle_user_message(
    content: String,
    session_id: String,
    sender: Arc<Mutex<futures::stream::SplitSink<WebSocket, Message>>>,
    state: &AppState,
) {
    let agent = state.agent.clone();
    let session_id_clone = session_id.clone();

    let task_handle = tokio::spawn(async move {
        let result = process_message_streaming(&agent, session_id_clone, content, sender).await;

        if let Err(e) = result {
            error!("Error processing message: {}", e);
        }
    });

    {
        let mut cancellations = state.cancellations.write().await;
        cancellations.insert(session_id.clone(), task_handle.abort_handle());
    }

    let cancellations_for_cleanup = state.cancellations.clone();
    let session_id_for_cleanup = session_id;

    tokio::spawn(async move {
        if let Err(e) = task_handle.await {
            if e.is_cancelled() {
                tracing::debug!("Task was cancelled");
            } else {
                error!("Task error: {}", e);
            }
        }

        let mut cancellations = cancellations_for_cleanup.write().await;
        cancellations.remove(&session_id_for_cleanup);
    });
}

async fn handle_cancel_message(
    session_id: String,
    sender: &Arc<Mutex<futures::stream::SplitSink<WebSocket, Message>>>,
    state: &AppState,
) {
    let abort_handle = {
        let mut cancellations = state.cancellations.write().await;
        cancellations.remove(&session_id)
    };

    if let Some(handle) = abort_handle {
        handle.abort();

        let mut sender = sender.lock().await;
        let _ = sender
            .send(Message::Text(
                serde_json::to_string(&WebSocketMessage::Cancelled {
                    message: "Operation cancelled".to_string(),
                })
                .unwrap()
                .into(),
            ))
            .await;
    }
}

async fn process_message_streaming(
    agent: &Agent,
    session_id: String,
    content: String,
    sender: Arc<Mutex<futures::stream::SplitSink<WebSocket, Message>>>,
) -> Result<()> {
    use biorouter::agents::SessionConfig;

    let user_message = BioRouterMessage::user().with_text(content.clone());

    let provider = agent.provider().await;
    if provider.is_err() {
        let error_msg = "I'm not properly configured yet. Please configure a provider through the CLI first using `biorouter configure`.".to_string();
        let mut sender = sender.lock().await;
        let _ = sender
            .send(Message::Text(
                serde_json::to_string(&WebSocketMessage::Response {
                    content: error_msg,
                    role: "assistant".to_string(),
                    timestamp: chrono::Utc::now().timestamp_millis(),
                })
                .unwrap()
                .into(),
            ))
            .await;
        return Ok(());
    }

    let session = agent
        .config
        .session_manager
        .get_session(&session_id, true)
        .await?;
    let mut messages = session.conversation.unwrap_or_default();
    messages.push(user_message.clone());

    let session_config = SessionConfig {
        id: session.id.clone(),
        schedule_id: None,
        max_turns: None,
        max_tool_calls: None,
        budget: None,
        retry_config: None,
        reasoning_effort: None,
    };

    match agent.reply(user_message, session_config, None).await {
        Ok(mut stream) => {
            while let Some(result) = stream.next().await {
                match result {
                    Ok(AgentEvent::Message(message)) => {
                        process_agent_message(&message, &sender, agent).await;
                    }
                    Ok(AgentEvent::HistoryReplaced(_)) => {
                        tracing::info!("History replaced, compacting happened in reply");
                    }
                    Ok(AgentEvent::McpNotification(_)) => {
                        tracing::info!("Received MCP notification in web interface");
                    }
                    Ok(AgentEvent::ModelChange { model, mode }) => {
                        tracing::info!("Model changed to {} in {} mode", model, mode);
                    }
                    // Issue #56 Gate B: what this turn ran on, and how the chat
                    // is classified. Logged rather than sent: the web bridge's
                    // frame vocabulary is the transcript, and a browser session
                    // cannot change its model anyway (SD-1), so there is no wrong
                    // choice on screen for it to correct.
                    //
                    // ⚠ `debug!`, not `info!`. The frame arrives on every turn
                    // now, so an `info!` line naming a binding nothing can change
                    // would be one log line per turn, forever, saying what the
                    // start-up banner already said.
                    Ok(AgentEvent::PrivacyProviderPinned {
                        provider,
                        model,
                        privacy_tier,
                        ..
                    }) => {
                        tracing::debug!(
                            "Turn ran on {provider} / {model} (chat classified {privacy_tier:?})"
                        );
                    }
                    // BR-52: the web bridge reads token counts from the session
                    // row when it needs them, so the carried snapshot is a no-op here.
                    Ok(AgentEvent::TokenUsage(_)) => {}
                    // Advisory pending tool-call UI hint; the web bridge renders
                    // authoritative messages only.
                    Ok(AgentEvent::ToolCallPending(_)) => {}
                    // #59: persisted-id bookkeeping; not part of the web transcript.
                    Ok(AgentEvent::MessagesPersisted(_)) => {}
                    Ok(AgentEvent::TurnAborted { code, message }) => {
                        // The turn ended without doing its work — report it as an
                        // error rather than letting the stream finish normally.
                        // The preceding Message has already preserved the
                        // human-readable explanation for the user.
                        error!(abort = code.wire_code(), "Turn aborted: {message}");
                        send_error(&sender, &format!("{}: {message}", code.wire_code())).await;
                        return Err(TurnFailed::new(code, message).into());
                    }
                    Err(e) => {
                        error!("Error in message stream: {}", e);
                        send_error(&sender, &format!("Error: {}", e)).await;
                        return Err(e);
                    }
                }
            }
        }
        Err(e) => {
            error!("Error calling agent: {}", e);
            send_error(&sender, &format!("Error: {}", e)).await;
            return Err(e);
        }
    }

    let mut sender = sender.lock().await;
    let _ = sender
        .send(Message::Text(
            serde_json::to_string(&WebSocketMessage::Complete {
                message: "Response complete".to_string(),
            })
            .unwrap()
            .into(),
        ))
        .await;

    Ok(())
}

async fn process_agent_message(
    message: &BioRouterMessage,
    sender: &Arc<Mutex<futures::stream::SplitSink<WebSocket, Message>>>,
    agent: &Agent,
) {
    use biorouter::conversation::message::MessageContent;

    for content in &message.content {
        match content {
            MessageContent::Text(text) => {
                let mut sender = sender.lock().await;
                let _ = sender
                    .send(Message::Text(
                        serde_json::to_string(&WebSocketMessage::Response {
                            content: text.text.clone(),
                            role: "assistant".to_string(),
                            timestamp: chrono::Utc::now().timestamp_millis(),
                        })
                        .unwrap()
                        .into(),
                    ))
                    .await;
            }
            MessageContent::ToolRequest(req) => {
                let mut sender = sender.lock().await;
                if let Ok(tool_call) = &req.tool_call {
                    let _ = sender
                        .send(Message::Text(
                            serde_json::to_string(&WebSocketMessage::ToolRequest {
                                id: req.id.clone(),
                                tool_name: tool_call.name.to_string(),
                                arguments: Value::from(tool_call.arguments.clone()),
                            })
                            .unwrap()
                            .into(),
                        ))
                        .await;
                }
            }
            MessageContent::ToolResponse(_) => {}
            MessageContent::ToolConfirmationRequest(confirmation) => {
                {
                    let mut sender = sender.lock().await;
                    let _ = sender
                        .send(Message::Text(
                            serde_json::to_string(&WebSocketMessage::ToolConfirmation {
                                id: confirmation.id.clone(),
                                tool_name: confirmation.tool_name.to_string(),
                                arguments: Value::from(confirmation.arguments.clone()),
                                needs_confirmation: true,
                            })
                            .unwrap()
                            .into(),
                        ))
                        .await;
                }

                agent
                    .handle_confirmation(
                        confirmation.id.clone(),
                        biorouter::permission::PermissionConfirmation {
                            principal_type:
                                biorouter::permission::permission_confirmation::PrincipalType::Tool,
                            permission: biorouter::permission::Permission::AllowOnce,
                        },
                    )
                    .await;
            }
            MessageContent::Thinking(thinking) => {
                let mut sender = sender.lock().await;
                let _ = sender
                    .send(Message::Text(
                        serde_json::to_string(&WebSocketMessage::Thinking {
                            message: thinking.thinking.clone(),
                        })
                        .unwrap()
                        .into(),
                    ))
                    .await;
            }
            _ => {}
        }
    }
}

async fn send_error(
    sender: &Arc<Mutex<futures::stream::SplitSink<WebSocket, Message>>>,
    message: &str,
) {
    let mut sender = sender.lock().await;
    let _ = sender
        .send(Message::Text(
            serde_json::to_string(&WebSocketMessage::Error {
                message: message.to_string(),
            })
            .unwrap()
            .into(),
        ))
        .await;
}

#[cfg(test)]
mod tests {
    use super::token_matches;

    #[test]
    fn token_match_is_exact_and_length_checked() {
        assert!(token_matches("abc123", "abc123"));
        assert!(!token_matches("abc123", "abc124"));
        // Differing lengths never match (no false positive on a prefix).
        assert!(!token_matches("abc", "abc123"));
        assert!(!token_matches("abc123", "abc"));
        assert!(!token_matches("", "abc123"));
        assert!(token_matches("", ""));
    }
}
