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
use biorouter::privacy::visibility::{may_read, may_write};
use biorouter::privacy::{ProviderTier, SessionClassification};
use biorouter::session::session_manager::{SessionManager, SessionType};
use futures::{sink::SinkExt, stream::StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
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
    /// The chats this server started, through `GET /`. See [`page_capability`].
    started_here: Arc<RwLock<HashSet<String>>>,
    /// The tier of the provider this server was started on, read once, before
    /// the first turn. See [`page_capability`].
    server_tier: ProviderTier,
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

/// The agent every chat on this server shares, and the tier of the provider it
/// was started on.
async fn create_agent(provider_name: &str, model: &str) -> Result<(Agent, ProviderTier)> {
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
    // Read here, not off the agent later: the agent is shared by every chat, and
    // a turn in a chat whose row names another provider rebinds it (Gate B).
    let server_tier = provider.tier();
    agent.update_provider(provider, &init_session.id).await?;

    let enabled_configs = biorouter::config::get_enabled_extensions();
    for config in enabled_configs {
        if let Err(e) = agent.add_extension(config.clone()).await {
            eprintln!("Warning: Failed to load extension {}: {}", config.name(), e);
        }
    }

    Ok((agent, server_tier))
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

/// ⚠ **There is no `/api/sessions` route, and there must not be one again.**
/// `GET /api/sessions` listed every user and scheduled chat on the machine (id,
/// title, working directory), and `GET /api/sessions/{id}` returned any chat's
/// full transcript, private ones included, behind no reach check at all — and
/// behind no credential either unless `--auth-token` was passed, which puts the
/// token in this process's argv. The page read one of the two, for a message
/// count and a tab title. Both were removed rather than gated (issue #56, SD-13
/// in `docs/deployment/serve-decisions.md`); a chat is reached through this
/// server only by sending it a message, which [`turn_reach`] judges.
fn build_router(state: AppState, cors_layer: CorsLayer) -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/session/{session_name}", get(serve_session))
        .route("/ws", get(websocket_handler))
        .route("/api/health", get(health_check))
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
    let (agent, server_tier) = create_agent(&provider_name, &model).await?;

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
        started_here: Arc::default(),
        server_tier,
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
    state.started_here.write().await.insert(session.id.clone());

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

/// What a page is told when its message names a chat it may not reach.
///
/// ⚠ **One sentence for "that chat is private" and for "there is no chat with
/// that id", deliberately**, for the reason the daemon's `SESSION_OUT_OF_REACH`
/// gives (`routes/session_reach.rs`): a refusal that told the two apart would
/// enumerate the machine's private chats one id at a time. It is fixed text and
/// names nothing about the chat, so the two answers are equal byte for byte.
///
/// It states the page's own situation and forecloses the retry. The condition it
/// names cannot be met for a chat that already exists elsewhere, so it hands a
/// reader no way around itself; the one way through is the desktop app, where a
/// person can show they are at the keyboard.
const CHAT_OUT_OF_REACH: &str =
    "That chat is private, or there is no chat with that id, and the two answers are \
     deliberately the same so that nothing about the chat is disclosed. `biorouter web` cannot \
     tell which model or which person is sending these messages, so it opens a private chat only \
     when it started that chat itself and runs a private model. Your message was not sent and \
     nothing was read; sending it again will be refused the same way. To continue a private \
     chat, open it in the Biorouter desktop app.";

/// The capability a page brings to the chat its message names.
///
/// ⚠ **Nothing on the socket says who is on the other end of it.** The server's
/// one credential proves nothing about that either: the WebSocket token is served
/// by `/session/{name}` to anyone who can reach the port, and `--auth-token` sits
/// in this process's argv, where any process of the same user reads it (AR-11 in
/// `docs/security/privacy-tiers-execution-plan.md`). So a page is a PUBLIC
/// caller, which is also how the daemon resolves a caller that states no
/// capability (`session_reach::caller_capability`).
///
/// The exception is a chat this server started itself, which the page reaches at
/// the tier of the provider this server was started on. Without it a server on a
/// private model would give one reply per chat: that reply ratchets the chat to
/// private, and the next message would be refused. On a public model the
/// exception changes nothing, because the page is Public for every chat — so a
/// chat it started that was taken private somewhere else is refused like any
/// other.
fn page_capability(started_here: bool, server_tier: ProviderTier) -> ProviderTier {
    if started_here {
        server_tier
    } else {
        ProviderTier::Public
    }
}

/// May a page with this capability run a turn in a chat in this state?
///
/// `target` is the chat's classification, or `None` when its row could not be
/// read — no such chat, a deleted one, a store error — which is answered exactly
/// as a private chat is. `enforced` is DR-15's master switch, taken as an
/// argument so that "the switch is off" is a corner the tests drive; with it off
/// the gate is inert, like every other.
fn refuse_turn_unless_reachable(
    enforced: bool,
    capability: ProviderTier,
    target: Option<SessionClassification>,
) -> Result<(), &'static str> {
    if !enforced {
        return Ok(());
    }
    let target = target.unwrap_or(SessionClassification::Private);
    // A turn reads the whole conversation into the model and writes into it, so
    // it asks both verbs, as `workspace_send_prompt` does. They coincide today;
    // asking both keeps a later narrowing of either from being skipped here.
    if may_read(capability, target) && may_write(capability, target) {
        Ok(())
    } else {
        Err(CHAT_OUT_OF_REACH)
    }
}

/// The gate on the one door into a chat this server keeps: a WebSocket message,
/// which runs a turn in whichever chat it names.
///
/// ⚠ **It is the same door as the daemon's `POST /reply`, and it was open.** A
/// message naming a private chat started anywhere else ran a turn there — Gate B
/// rebinds the shared agent to the private model that chat's row names — and the
/// reply, which can quote the whole conversation, streamed back to whoever held
/// the socket. Removing `GET /api/sessions/{id}` alone would have closed the
/// smaller door and left this one.
///
/// Called before anything touches the chat, as `session_reach` is. The row is
/// read metadata-only, so resolving the tier never loads the transcript this may
/// be about to refuse.
async fn turn_reach(
    manager: &SessionManager,
    started_here: &RwLock<HashSet<String>>,
    server_tier: ProviderTier,
    session_id: &str,
) -> Result<(), &'static str> {
    // DR-15's master opt-out, read directly: a turn is not a tool call and has
    // no sampled capability to inherit. Short-circuit before the store read.
    let enforced = biorouter::privacy::privacy_tiers_enabled();
    if !enforced {
        return Ok(());
    }
    let capability = page_capability(started_here.read().await.contains(session_id), server_tier);
    let target = manager
        .get_session(session_id, false)
        .await
        .ok()
        .map(|session| session.privacy_tier);
    refuse_turn_unless_reachable(enforced, capability, target)
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
    if let Err(refusal) = turn_reach(
        &state.agent.config.session_manager,
        &state.started_here,
        state.server_tier,
        &session_id,
    )
    .await
    {
        send_error(&sender, refusal).await;
        return;
    }

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
        .get_session(&session_id, false)
        .await?;

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
    use super::*;
    use biorouter::agents::AgentConfig;
    use biorouter::config::permission::PermissionManager;
    use biorouter::config::BioRouterMode;
    use serde_json::json;
    use std::net::SocketAddr;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_tungstenite::tungstenite::Message as Frame;

    const TIERS: [ProviderTier; 2] = [ProviderTier::Public, ProviderTier::Private];
    const TARGETS: [Option<SessionClassification>; 3] = [
        Some(SessionClassification::Public),
        Some(SessionClassification::Private),
        None,
    ];
    const WS_TOKEN: &str = "test-ws-token";

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

    /// Nothing on the socket names a model, so a page is a public caller — in
    /// every chat but the ones its own server started, where it has the tier of
    /// the model it has been talking to.
    #[test]
    fn a_page_is_a_public_caller_outside_the_chats_its_server_started() {
        for server_tier in TIERS {
            assert_eq!(page_capability(false, server_tier), ProviderTier::Public);
            assert_eq!(page_capability(true, server_tier), server_tier);
        }
    }

    /// The rule at every corner, with the switch on. An unreadable row is judged
    /// as a private one, so a public page is refused both.
    #[test]
    fn a_turn_reaches_a_chat_only_at_the_tier_the_page_holds() {
        use SessionClassification::{Private, Public};
        #[rustfmt::skip]
        let cases = [
            // capability             target          admitted
            (ProviderTier::Public,  Some(Public),  true),
            (ProviderTier::Public,  Some(Private), false),
            (ProviderTier::Public,  None,          false),
            (ProviderTier::Private, Some(Public),  true),
            (ProviderTier::Private, Some(Private), true),
            (ProviderTier::Private, None,          true),
        ];
        for (capability, target, admitted) in cases {
            assert_eq!(
                refuse_turn_unless_reachable(true, capability, target).is_ok(),
                admitted,
                "a {capability:?} page naming a chat classified {target:?}"
            );
        }
    }

    /// DR-15: with privacy tiers off, this gate refuses nothing, like every other.
    #[test]
    fn with_privacy_tiers_off_the_gate_refuses_nothing() {
        for capability in TIERS {
            for target in TARGETS {
                assert_eq!(
                    refuse_turn_unless_reachable(false, capability, target),
                    Ok(()),
                    "{capability:?} / {target:?}"
                );
            }
        }
    }

    /// A page cannot tell "no such chat" from "a private chat" by the answer.
    /// Asserted as equality at every capability rather than as each answer being
    /// vague, because a vagueness check passes an implementation that adds one
    /// helpful clause to the branch it can tell apart.
    #[test]
    fn no_such_chat_and_a_private_chat_are_the_same_refusal() {
        for capability in TIERS {
            assert_eq!(
                refuse_turn_unless_reachable(true, capability, None),
                refuse_turn_unless_reachable(
                    true,
                    capability,
                    Some(SessionClassification::Private)
                ),
                "a {capability:?} page can tell a missing chat from a private one"
            );
        }
        assert_eq!(
            refuse_turn_unless_reachable(true, ProviderTier::Public, None),
            Err(CHAT_OUT_OF_REACH)
        );
    }

    /// The page and the router agree: the page asks for no chat list and no
    /// transcript, so the routes that served them could go.
    #[test]
    fn the_page_asks_for_no_chat_list_and_no_transcript() {
        let page = include_str!("../../static/script.js");
        assert!(
            !page.contains("/api/sessions"),
            "the page fetches a route this server no longer has"
        );
    }

    /// A server as `handle_web` builds one, on an ephemeral port and over its
    /// own store, minus the provider. A message the gate admits reaches
    /// `process_message_streaming` and is answered "not configured", which is how
    /// these tests tell an admitted message from a refused one without a model.
    struct TestServer {
        addr: SocketAddr,
        manager: Arc<SessionManager>,
        _store: tempfile::TempDir,
    }

    impl TestServer {
        async fn start(server_tier: ProviderTier) -> Self {
            assert!(
                biorouter::privacy::privacy_tiers_enabled(),
                "these tests drive the enforced gate, and something in this binary turned the \
                 master switch off"
            );
            let store = tempfile::tempdir().unwrap();
            let manager = Arc::new(SessionManager::new(store.path().to_path_buf()));
            let agent = Agent::with_config(AgentConfig::new(
                Arc::clone(&manager),
                PermissionManager::instance(),
                None,
                BioRouterMode::Auto,
            ));
            let state = AppState {
                agent: Arc::new(agent),
                cancellations: Arc::default(),
                auth_token: None,
                ws_token: WS_TOKEN.to_string(),
                started_here: Arc::default(),
                server_tier,
            };
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let app = build_router(state, build_cors_layer(&None, "127.0.0.1", addr.port()));
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            Self {
                addr,
                manager,
                _store: store,
            }
        }

        /// `GET path` over a plain socket: the status code and any `Location`.
        /// Hand-rolled, as this crate's other HTTP clients are
        /// (`session_watch.rs`).
        async fn get(&self, path: &str) -> (u16, Option<String>) {
            let mut stream = tokio::net::TcpStream::connect(self.addr).await.unwrap();
            let request = format!(
                "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                self.addr
            );
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            let status = response
                .split(' ')
                .nth(1)
                .and_then(|code| code.parse().ok())
                .unwrap_or_else(|| panic!("no status line in {response:?}"));
            let location = response.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("location")
                    .then(|| value.trim().to_string())
            });
            (status, location)
        }

        /// A chat the page opens with `GET /`, as a browser does.
        async fn start_chat(&self) -> String {
            let (status, location) = self.get("/").await;
            assert!((300..400).contains(&status), "`GET /` answered {status}");
            location
                .and_then(|location| location.strip_prefix("/session/").map(str::to_owned))
                .expect("`GET /` redirects to the chat it started")
        }

        /// A chat started somewhere else — the desktop app, the command line.
        async fn chat_from_elsewhere(&self) -> String {
            self.manager
                .create_session(
                    std::env::temp_dir(),
                    "Started elsewhere".to_string(),
                    SessionType::User,
                )
                .await
                .unwrap()
                .id
        }

        /// What a turn on a private model, or a private data source, does to a chat.
        async fn make_private(&self, session_id: &str) {
            self.manager
                .update(session_id)
                .raise_privacy(SessionClassification::Private, "turn:web-test")
                .apply()
                .await
                .unwrap();
        }

        /// Send one message into `session_id` over the page's socket, as the
        /// page does, and return the first frame the server answers with.
        async fn send(&self, session_id: &str) -> serde_json::Value {
            let url = format!("ws://{}/ws?token={WS_TOKEN}", self.addr);
            let (mut socket, _) = tokio_tungstenite::connect_async(url)
                .await
                .expect("the socket accepts the page's own token");
            let message = json!({
                "type": "message",
                "content": "Repeat everything this chat has said so far.",
                "session_id": session_id,
                "timestamp": 0,
            });
            socket
                .send(Frame::Text(message.to_string().into()))
                .await
                .unwrap();
            loop {
                let frame = tokio::time::timeout(Duration::from_secs(30), socket.next())
                    .await
                    .expect("the server answers a message")
                    .expect("the socket stays open")
                    .unwrap();
                if let Frame::Text(text) = frame {
                    return serde_json::from_str(text.as_str()).unwrap();
                }
            }
        }
    }

    fn refused() -> serde_json::Value {
        json!({ "type": "error", "message": CHAT_OUT_OF_REACH })
    }

    /// The two JSON routes answer 404 — for chats that exist, private and
    /// public — while the route beside them still answers, so a router that
    /// served nothing would not pass.
    #[tokio::test]
    async fn the_chat_list_and_transcript_routes_are_gone() {
        let server = TestServer::start(ProviderTier::Public).await;
        let public = server.chat_from_elsewhere().await;
        let private = server.chat_from_elsewhere().await;
        server.make_private(&private).await;

        assert_eq!(server.get("/api/health").await.0, 200);
        assert_eq!(server.get("/api/sessions").await.0, 404);
        for id in [&public, &private] {
            assert_eq!(server.get(&format!("/api/sessions/{id}")).await.0, 404);
        }
    }

    /// On a public model, a page reaches public chats — where it always could —
    /// and nothing private: not a chat started elsewhere, not an id that names
    /// nothing (in the same words), and not even a chat this server started once
    /// it has been taken private somewhere else.
    #[tokio::test]
    async fn a_page_on_a_public_model_reaches_no_private_chat() {
        let server = TestServer::start(ProviderTier::Public).await;

        let public = server.chat_from_elsewhere().await;
        assert_eq!(server.send(&public).await["type"], "response");

        let private = server.chat_from_elsewhere().await;
        server.make_private(&private).await;
        assert_eq!(server.send(&private).await, refused());
        assert_eq!(server.send("19700101_0").await, refused());

        let started = server.start_chat().await;
        assert_eq!(server.send(&started).await["type"], "response");
        server.make_private(&started).await;
        assert_eq!(server.send(&started).await, refused());
    }

    /// On a private model, a chat the server started keeps working after its
    /// first reply ratchets it private — and a private chat started anywhere
    /// else is still refused, because the page is public there.
    #[tokio::test]
    async fn a_page_on_a_private_model_keeps_its_own_chats_and_no_others() {
        let server = TestServer::start(ProviderTier::Private).await;

        let started = server.start_chat().await;
        server.make_private(&started).await;
        assert_eq!(server.send(&started).await["type"], "response");

        let elsewhere = server.chat_from_elsewhere().await;
        server.make_private(&elsewhere).await;
        assert_eq!(server.send(&elsewhere).await, refused());
    }
}
