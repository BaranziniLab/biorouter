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
use tower_http::cors::CorsLayer;
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

/// ⚠ **An empty `--auth-token` is not a token, and must not satisfy this guard.**
/// `Some("")` used to pass it, so `--host 0.0.0.0 --auth-token ""` bound to every
/// interface while `auth_middleware` would admit anyone who sent
/// `Authorization: Bearer ` with nothing after it — no protection at all, past
/// the one check whose whole job is to insist on protection. `cli.rs` now refuses
/// an empty value at argument-parse time, which is the real fix; this treats it
/// as absent as well, because `handle_web` is a public function and the guard
/// must not depend on its one caller having been careful.
fn validate_network_auth(host: &str, auth_token: &Option<String>) {
    let auth_token = auth_token.as_deref().filter(|token| !token.is_empty());
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

/// No origin but this server's own may read anything this server serves, and
/// nothing needs to.
///
/// ⚠ **This layer used to hand the WebSocket token to another origin, which is
/// the same capability the reflected XSS gave** (see [`serve_session`]). Without
/// `--auth-token` — the default — `auth_middleware` lets every request through,
/// so a cross-origin `fetch` of `/session/…` that the browser permits *reads the
/// page*, and `data-ws-token` is in it. From there: open `/ws` with the token,
/// which is not subject to the same-origin policy, and send a message to an
/// agent holding `developer__shell`. Escaping the reflection while leaving this
/// open would have closed the sink and left the outcome.
///
/// The allow-list was `localhost:3000`, `127.0.0.1:3000` and this server's own
/// origin. `--port` **defaults to 3000**, so on a default run all three are this
/// server; the grant only starts meaning something on any other port, where it
/// hands `http://…:3000` — a frontend dev server, or a page the operator was
/// talked into opening — read access to a chat page on, say, `:8080`. The
/// documented invocations include `--port 8080`.
///
/// ⚠ **There is deliberately no flag to turn this back on.** The two routes a
/// cross-origin browser client could have wanted, `/api/sessions` and
/// `/api/sessions/{id}`, are the ones SD-13 deleted; what is left is the page
/// itself, `/static/*`, a static `/api/health` and the WebSocket, which CORS
/// does not govern. Nothing in this repository reads any of it from another
/// origin — `scripts/test_web.sh` uses `curl`, which ignores CORS entirely. An
/// opt-in would therefore be an opt-in to the token leak and to nothing else.
///
/// The layer is kept rather than removed so that a preflight gets a definite
/// answer from code that says why, instead of a 405 from the router.
fn build_cors_layer() -> CorsLayer {
    CorsLayer::new()
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

    // Unconditional. It used to be empty whenever `--auth-token` was set, which
    // was only safe because [`websocket_handler`] then skipped the check
    // altogether — and `token_matches("", "")` is `true`, so the pair was one
    // careless edit away from an open socket. See that handler's doc comment.
    let ws_token = uuid::Uuid::new_v4().to_string();

    let state = AppState {
        agent: Arc::new(agent),
        cancellations: Arc::new(RwLock::new(std::collections::HashMap::new())),
        auth_token: auth_token.clone(),
        ws_token,
        started_here: Arc::default(),
        server_tier,
    };

    let cors_layer = build_cors_layer();
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

/// The line in `index.html` the boot values are written in front of.
const BOOT_ANCHOR: &str = "<script src=\"/static/script.js\"></script>";

/// What this page is allowed to do, sent as a header rather than a `<meta>` so
/// that injected markup cannot appear above it and displace it.
///
/// `script-src 'self'` is the half that earns its keep: with it, an injected
/// `<script>` or `onerror=` does not run even if some *other* sink on this page
/// is missed later. That is also why `index.html`'s suggestion pills bind their
/// handlers in `script.js` instead of carrying `onclick` attributes — an inline
/// handler is precisely what this directive refuses, so the two move together.
///
/// `connect-src` names the WebSocket schemes as well as `'self'`. A same-origin
/// `ws://` is covered by `'self'` in the specification, but this page's entire
/// function is that socket, and a directive that is right on paper and wrong in
/// some browser would take the page down rather than harden it.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
     img-src 'self' data:; connect-src 'self' ws: wss:; base-uri 'none'; form-action 'none'; \
     frame-ancestors 'none'; object-src 'none'";

/// Escape a value for an HTML double-quoted attribute.
///
/// `&` and `"` are what that grammar strictly requires; `<`, `>` and `'` are
/// escaped too, so the same function stays correct if a value is ever moved
/// into a single-quoted attribute or into element content. Written as a
/// character walk rather than a chain of `replace`s on purpose: a chain has an
/// ordering hazard — escape `&` anywhere but first and it re-escapes the `&` of
/// every entity the earlier steps just wrote — and a walk cannot have it.
fn escape_html_attribute(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

/// The chat page, carrying the two values it needs to boot.
///
/// ⚠ **`session_name` is whatever the URL path said**, so this is the one place
/// on this server where a stranger's bytes are written into a document — behind
/// no credential at all on the loopback bind that requires none. It used to be
/// written into *script context*:
///
/// ```text
/// <script>window.BIOROUTER_SESSION_NAME = '{session_name}'; …</script>
/// ```
///
/// where a `'` ended the string literal and `</script>` ended the element, so
/// `GET /session/</script><img src=x onerror=…>` ran the sender's code. On this
/// page that is not defacement: the injected script runs on the server's own
/// origin, reads the WebSocket token out of the very document it was injected
/// into, opens `/ws` with it, and sends a message to an agent holding
/// `developer__shell`. That token is the only thing standing between a drive-by
/// page and the socket — WebSockets are not subject to CORS — and the injection
/// is handed it. One link is remote code execution.
///
/// ⚠ **The fix is to leave script context, not to escape for it.** HTML-escaping
/// a `<script>` body is not a fix and looks like one: the HTML parser does not
/// decode entities inside `<script>`, so `&lt;/script&gt;` would reach the
/// JavaScript parser verbatim and nothing would have been neutralised. In a
/// double-quoted attribute the parser *does* decode entities, so the page reads
/// the value back through `dataset` exactly as it arrived, and no byte can
/// leave the attribute. `state.ws_token` goes through the same escape even
/// though this server generates it: a value is escaped for where it is going,
/// never for where the reader believes it came from.
async fn serve_session(
    axum::extract::Path(session_name): axum::extract::Path<String>,
    State(state): State<AppState>,
) -> Response {
    let html = include_str!("../../static/index.html").replace(
        BOOT_ANCHOR,
        &format!(
            "<div id=\"biorouter-boot\" hidden data-session-name=\"{}\" data-ws-token=\"{}\"></div>\n    {BOOT_ANCHOR}",
            escape_html_attribute(&session_name),
            escape_html_attribute(&state.ws_token),
        ),
    );
    (
        [("content-security-policy", CONTENT_SECURITY_POLICY)],
        Html(html),
    )
        .into_response()
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

/// Is this `Origin` this very server?
///
/// A mirror of the daemon's `routes::origin_matches_host`, in the same spirit as
/// [`token_matches`] mirroring its `secret_matches`: `biorouter-cli` does not
/// depend on `biorouter-server` and must not start — SD-7 is why `serve` spawns
/// `biorouterd` as a subprocess rather than linking it — so the rule is
/// duplicated rather than imported. If the two ever need to be one symbol, the
/// move is into the `biorouter` core library that both already depend on, never
/// a new command-line-interface-to-server dependency. ⚠ PR #233 is editing the
/// daemon's copy; the shape below is that PR's, not the older one.
///
/// It is the **strict core of that rule and neither of its exceptions**, and
/// deliberately so:
///
/// - No `is_local_origin` widening. #233 removes exactly that from the daemon's
///   socket gates — "`is_local_origin` is the CORS rule now and nothing else; do
///   not hand it back to a socket" — and here it would re-open the hole
///   [`build_cors_layer`] just closed, by admitting a page on `localhost:3000`.
/// - No `file://` and no declared-renderer origin. Those exist for the Electron
///   renderer, which reaches the daemon from another local origin. This server
///   serves its own page from its own origin and has no such client, so an
///   opaque origin is refused like any other.
fn origin_is_this_server(origin: &str, host: Option<&str>) -> bool {
    let Some(host) = host else {
        // Nothing to compare against. Refuse rather than guess.
        return false;
    };
    // `null`, `file://` and anything else opaque strip no scheme and so can
    // never match.
    let Some(authority) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    !authority.is_empty() && !host.is_empty() && authority.eq_ignore_ascii_case(host)
}

/// The socket is the chat, so it carries both locks the rest of this file
/// assumes: it is this server's own page asking, and it holds this process's
/// token.
///
/// ⚠ **The token check used to be skipped entirely whenever `--auth-token` was
/// set**, on the reasoning that `auth_middleware` had already authenticated the
/// handshake. That is defensible and it left a landmine, because `handle_web`
/// also made `ws_token` the empty string in that mode: `token_matches("", "")`
/// is `true`, so deleting the `if` without touching the generation would have
/// admitted *every* socket while reading like a tightening. The generation is
/// unconditional now, the check is unconditional, and an empty expected token is
/// refused outright so the landmine cannot be re-armed.
///
/// ⚠ **And there was no `Origin` check on any path**, while the tree's other two
/// upgrade sites (`routes/workspace.rs`, `routes/apps.rs`) both have one. CORS
/// does not govern a WebSocket handshake, so a page on any origin that had the
/// token could drive an agent holding `developer__shell` (CSWSH). The token is no
/// longer readable cross-origin either ([`build_cors_layer`]), which is the point
/// of having both: the two locks fail independently.
///
/// A client that sends no `Origin` at all is let past this gate, as the daemon's
/// gates let one past: that is a non-browser client, and the token still guards
/// it. A local process reading the token off this port is issue #47 and unchanged.
async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Query(query): Query<WsQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    if let Some(origin) = headers.get(axum::http::header::ORIGIN) {
        let host = headers
            .get(axum::http::header::HOST)
            .and_then(|h| h.to_str().ok());
        if !origin_is_this_server(origin.to_str().unwrap_or(""), host) {
            tracing::warn!("WebSocket connection rejected: cross-origin handshake");
            return Err(StatusCode::FORBIDDEN);
        }
    }

    if state.ws_token.is_empty() {
        // Unreachable through `handle_web`, which always generates one. A
        // refusal rather than a `debug_assert`, because the cost of being wrong
        // is an open socket.
        tracing::error!("WebSocket connection rejected: this server has no socket token");
        return Err(StatusCode::FORBIDDEN);
    }
    if !token_matches(query.token.as_deref().unwrap_or(""), &state.ws_token) {
        tracing::warn!("WebSocket connection rejected: invalid token");
        return Err(StatusCode::FORBIDDEN);
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

/// ⚠ **Gated, even though the map it reads can only hold chats the gate already
/// admitted.** A handle lands in `cancellations` only after
/// [`handle_user_message`] passed [`turn_reach`], so a cancel naming an
/// unreachable chat could never abort anything it was not already allowed to.
/// What it *could* do is answer: the old code replied `Cancelled` when a handle
/// existed and said nothing when it did not, which is one bit about a chat the
/// sender may not reach. Judging it first also makes the property this file
/// wants a flat one — **every socket message that names a chat is judged before
/// the chat is touched** — rather than a claim that has to be re-derived from
/// what else happens to be true of the map.
async fn handle_cancel_message(
    session_id: String,
    sender: &Arc<Mutex<futures::stream::SplitSink<WebSocket, Message>>>,
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
        send_error(sender, refusal).await;
        return;
    }

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

    /// A session name that tries to leave a double-quoted HTML attribute, and
    /// the script context it used to be written into. Percent-encoded at the
    /// call site because a raw `"` is not legal in a request target.
    const BREAKOUT: &str = "</script><img src=x onerror=alert(1)>\"'&";
    const BREAKOUT_ENCODED: &str =
        "%3C%2Fscript%3E%3Cimg%20src%3Dx%20onerror%3Dalert(1)%3E%22%27%26";

    #[test]
    fn an_attribute_escape_neutralises_every_character_that_could_leave_one() {
        assert_eq!(
            escape_html_attribute(BREAKOUT),
            "&lt;/script&gt;&lt;img src=x onerror=alert(1)&gt;&quot;&#39;&amp;"
        );
        // An `&` escaped anywhere but first would come back doubled.
        assert_eq!(escape_html_attribute("&lt;"), "&amp;lt;");
        assert_eq!(
            escape_html_attribute("plain-20260911_120000"),
            "plain-20260911_120000"
        );
    }

    /// ⚠ **The reflected XSS this branch closes.** `GET /session/{name}` wrote
    /// the name of the chat straight into an inline `<script>`, so a name
    /// holding `</script>` ended the element and everything after it was parsed
    /// as markup. The consequence is not cosmetic — see [`serve_session`]: the
    /// injected script reads the WebSocket token out of the same document and
    /// drives an agent that holds `developer__shell`.
    ///
    /// Asserted as **"the page has exactly the script elements its own template
    /// has"** rather than as "the payload does not appear". The weaker form
    /// passes an implementation that HTML-escapes inside the `<script>` body,
    /// which is not a fix at all: the HTML parser does not decode entities
    /// there, so the JavaScript parser still receives `</script>`.
    #[tokio::test]
    async fn a_session_name_cannot_reach_script_context() {
        let server = TestServer::start(ProviderTier::Public).await;
        let template = include_str!("../../static/index.html");
        let response = server
            .raw_get(&format!("/session/{BREAKOUT_ENCODED}"))
            .await;

        assert_eq!(
            response.matches("<script").count(),
            template.matches("<script").count(),
            "the name opened a script element the template does not have:\n{response}"
        );
        assert_eq!(
            response.matches("</script").count(),
            template.matches("</script").count(),
            "the name closed a script element early:\n{response}"
        );
        // To run, the payload has to become a tag, and a tag needs a raw `<`.
        // Asserted against the element it would open and the `>` that would
        // close it, not against `onerror=` — that substring survives inside the
        // escaped attribute, where it is text and not a handler.
        assert!(
            !response.contains("<img"),
            "the payload opened an element the template has none of:\n{response}"
        );
        assert!(
            !response.contains("alert(1)>"),
            "the payload kept a raw `>`, so something closed a tag:\n{response}"
        );
        assert!(
            response.contains(&format!(
                "data-session-name=\"{}\"",
                escape_html_attribute(BREAKOUT)
            )),
            "the name is not where the page reads it from, escaped:\n{response}"
        );
    }

    /// The same route on the same server, with an ordinary name: the page still
    /// gets the value it needs, unescaped once the parser has decoded it. A
    /// breakout test alone passes a handler that drops the name entirely.
    #[tokio::test]
    async fn an_ordinary_session_name_still_reaches_the_page() {
        let server = TestServer::start(ProviderTier::Public).await;
        let started = server.start_chat().await;
        let response = server.raw_get(&format!("/session/{started}")).await;
        assert!(
            response.contains(&format!("data-session-name=\"{started}\"")),
            "the page cannot tell which chat it is in:\n{response}"
        );
        assert!(
            response.contains(&format!("data-ws-token=\"{WS_TOKEN}\"")),
            "the page cannot open the socket:\n{response}"
        );
    }

    /// Defence in depth behind the escape, and the two halves that have to move
    /// together: a policy refusing inline script, and a template carrying none.
    #[tokio::test]
    async fn the_page_is_served_under_a_policy_that_refuses_inline_script() {
        let server = TestServer::start(ProviderTier::Public).await;
        let response = server.raw_get("/session/20260911_120000").await;
        let policy = TestServer::header(&response, "content-security-policy")
            .unwrap_or_else(|| panic!("no content-security-policy header in {response:?}"));
        assert!(
            policy.contains("script-src 'self'") && !policy.contains("unsafe-inline"),
            "a policy that permits inline script is not one: {policy}"
        );

        let template = include_str!("../../static/index.html");
        assert!(
            !template.contains("onclick="),
            "the template carries an inline handler the policy above would refuse"
        );
    }

    /// The origin rule, at every corner. The strict core of the daemon's
    /// `origin_matches_host` with neither of its exceptions — see
    /// [`origin_is_this_server`] for why each is deliberately absent.
    #[test]
    fn an_origin_is_this_server_only_when_it_matches_this_request_s_host() {
        let host = Some("127.0.0.1:8080");
        assert!(origin_is_this_server("http://127.0.0.1:8080", host));
        // Case-insensitive on the authority, as the daemon's copy is.
        assert!(origin_is_this_server(
            "http://LOCALHOST:8080",
            Some("localhost:8080")
        ));
        // The scheme prefix is matched literally, so an odd spelling of it is a
        // refusal rather than a match.
        assert!(!origin_is_this_server("HTTP://127.0.0.1:8080", host));
        // A different port is a different origin. This is the whole point: the
        // CORS allow-list this replaces named `localhost:3000` by hand.
        assert!(!origin_is_this_server("http://127.0.0.1:3000", host));
        assert!(!origin_is_this_server("http://localhost:8080", host));
        assert!(!origin_is_this_server("http://evil.example", host));
        // Opaque origins strip no scheme, so they can never match — and unlike
        // the daemon's gates, `file://` gets no exception here.
        for opaque in ["null", "file://", "", "ws://127.0.0.1:8080"] {
            assert!(!origin_is_this_server(opaque, host), "{opaque} admitted");
        }
        // Nothing to compare against is a refusal, not a guess.
        assert!(!origin_is_this_server("http://127.0.0.1:8080", None));
        assert!(!origin_is_this_server("http://", Some("")));
    }

    /// ⚠ **The socket is the chat, and it had no `Origin` check on any path**
    /// while the tree's other two upgrade sites both have one. CORS does not
    /// govern a handshake, so a page on any origin holding the token could drive
    /// an agent with `developer__shell` (CSWSH). Driven as a real handshake
    /// because `WebSocketUpgrade` is extracted before the handler body runs: a
    /// request without the upgrade headers is rejected with 400 by the extractor
    /// and would never reach the rule under test.
    #[tokio::test]
    async fn a_handshake_from_another_origin_is_refused_even_with_the_right_token() {
        let server = TestServer::start(ProviderTier::Public).await;

        assert_eq!(server.handshake(Some(WS_TOKEN), None).await, 101);
        let own = format!("http://{}", server.addr);
        assert_eq!(server.handshake(Some(WS_TOKEN), Some(&own)).await, 101);

        for origin in [
            "http://localhost:3000",
            "http://127.0.0.1:3000",
            "http://evil.example",
            "null",
            "file://",
        ] {
            assert_eq!(
                server.handshake(Some(WS_TOKEN), Some(origin)).await,
                403,
                "{origin} opened the socket"
            );
        }
    }

    /// The token gate, now that it runs on every path rather than only when
    /// `--auth-token` is absent.
    ///
    /// ⚠ **This is the landmine under "just make the check unconditional".**
    /// `ws_token` used to be the empty string whenever `--auth-token` was set,
    /// precisely because the check was skipped in that mode — and
    /// `token_matches("", "")` is `true`, so deleting the `if` without also
    /// changing the generation would have admitted *every* socket while reading
    /// like a tightening. Both halves are pinned: an empty expected token is
    /// refused by the handler, and `handle_web` never produces one.
    #[tokio::test]
    async fn the_socket_token_is_required_on_every_path_and_never_empty() {
        assert!(
            token_matches("", ""),
            "the reason an empty expected token must never reach the handler"
        );

        let server = TestServer::start(ProviderTier::Public).await;
        assert_eq!(server.handshake(Some(WS_TOKEN), None).await, 101);
        assert_eq!(server.handshake(None, None).await, 403);
        assert_eq!(server.handshake(Some("wrong"), None).await, 403);

        // A server whose token is empty refuses everything, including the empty
        // token that `token_matches` would otherwise accept.
        let tokenless = TestServer::start_with_ws_token(ProviderTier::Public, "").await;
        assert_eq!(tokenless.handshake(None, None).await, 403);
        assert_eq!(tokenless.handshake(Some(""), None).await, 403);
    }

    /// `--host 0.0.0.0 --auth-token ""` used to bind to every interface behind a
    /// token that `Authorization: Bearer ` satisfies. Pinned at both layers: the
    /// argument parser refuses the value, and the guard treats it as absent even
    /// if a programmatic caller hands it one.
    #[test]
    fn an_empty_auth_token_is_refused_and_does_not_satisfy_the_network_guard() {
        assert!(crate::cli::parse_auth_token("").is_err());
        assert!(crate::cli::parse_auth_token("   ").is_err());
        assert_eq!(
            crate::cli::parse_auth_token("secret").as_deref(),
            Ok("secret")
        );
        // `validate_network_auth` exits the process when it refuses, so the
        // normalisation it applies is asserted rather than the exit: an empty
        // token must reduce to `None`, which is the refusing branch.
        for token in [None, Some(String::new()), Some("  ".to_string())] {
            assert!(
                token.as_deref().filter(|t| !t.trim().is_empty()).is_none(),
                "{token:?} must not read as a token"
            );
        }
    }

    /// ⚠ **The other route to the same capability as the reflected XSS.** The
    /// allow-list held `http://localhost:3000`, so a page there could `fetch`
    /// `/session/…` on any other port, read `data-ws-token` out of the reply, and
    /// open `/ws` with it — WebSockets are not subject to the same-origin policy —
    /// reaching an agent that holds `developer__shell`. Escaping the reflection
    /// and leaving this would have closed the sink and left the outcome.
    ///
    /// Asserted on the **header**, not on the body: the token is still in the
    /// page, because the page needs it. What must not happen is a browser being
    /// told another origin may read that page.
    #[tokio::test]
    async fn no_other_origin_may_read_the_page_that_carries_the_ws_token() {
        let server = TestServer::start(ProviderTier::Public).await;
        let path = "/session/20260911_120000";

        for origin in [
            // Both spellings the allow-list named, on the port it hard-coded,
            // which is also `--port`'s default — so on any other port these are
            // a foreign origin, and `--port 8080` is a documented invocation.
            "http://localhost:3000",
            "http://127.0.0.1:3000",
            "http://evil.example",
        ] {
            let response = server.raw_request("GET", path, &[("Origin", origin)]).await;
            assert!(
                response.contains("data-ws-token=\""),
                "this test is meaningless if the page stopped carrying the token:\n{response}"
            );
            assert_eq!(
                TestServer::header(&response, "access-control-allow-origin"),
                None,
                "{origin} is told it may read the page holding the token"
            );

            // And the preflight, which is what a browser actually asks first
            // for anything beyond a simple request.
            let preflight = server
                .raw_request(
                    "OPTIONS",
                    path,
                    &[("Origin", origin), ("Access-Control-Request-Method", "GET")],
                )
                .await;
            assert_eq!(
                TestServer::header(&preflight, "access-control-allow-origin"),
                None,
                "the preflight grants {origin} what the response above refused"
            );
        }
    }

    /// Same-origin still works, which is the only case the page needs — a
    /// cross-origin refusal is worthless if it also broke the page itself.
    #[tokio::test]
    async fn the_page_still_serves_the_origin_it_is_served_from() {
        let server = TestServer::start(ProviderTier::Public).await;
        let own_origin = format!("http://{}", server.addr);
        let response = server
            .raw_request(
                "GET",
                "/session/20260911_120000",
                &[("Origin", &own_origin)],
            )
            .await;
        assert!(
            response.starts_with("HTTP/1.1 200"),
            "the server's own origin is refused its own page:\n{response}"
        );
        assert!(response.contains("data-ws-token=\"test-ws-token\""));
    }

    /// The page reads its boot values from attributes, not from globals an
    /// inline `<script>` assigned — the half of the fix that lives in the
    /// browser. If this ever regresses to `window.BIOROUTER_*`, the server is
    /// back to writing into script context to satisfy it.
    #[test]
    fn the_page_reads_its_boot_values_from_attributes() {
        let page = include_str!("../../static/script.js");
        assert!(
            !page.contains("window.BIOROUTER_"),
            "the page still expects a value assigned in script context"
        );
        assert!(
            page.contains("dataset[name]"),
            "the page reads no attribute"
        );
    }

    /// Every other hole in this page's markup, all of them fed by the agent: a
    /// tool's name and a tool call's arguments are chosen by whatever the model
    /// decided to run, and a prompt injection in a file or a fetched page
    /// reaches them. `JSON.stringify` is the one that reads safe and is not —
    /// it escapes for JSON, which leaves `<` alone.
    #[test]
    fn model_controlled_values_are_escaped_before_they_become_markup() {
        let page = include_str!("../../static/script.js");
        for hole in ["${data.tool_name}", "${JSON.stringify(", "${action}"] {
            assert!(
                !page.contains(hole),
                "`{hole}` goes into innerHTML without escaping"
            );
        }
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
            Self::start_with_ws_token(server_tier, WS_TOKEN).await
        }

        async fn start_with_ws_token(server_tier: ProviderTier, ws_token: &str) -> Self {
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
                ws_token: ws_token.to_string(),
                started_here: Arc::default(),
                server_tier,
            };
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let app = build_router(state, build_cors_layer());
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            Self {
                addr,
                manager,
                _store: store,
            }
        }

        /// `GET path` over a plain socket, headers and body as one string.
        /// Hand-rolled, as this crate's other HTTP clients are
        /// (`session_watch.rs`).
        async fn raw_get(&self, path: &str) -> String {
            self.raw_request("GET", path, &[]).await
        }

        /// One request with whatever extra headers the caller needs, so a test
        /// can put an `Origin` on it and read what the CORS layer answers.
        async fn raw_request(&self, method: &str, path: &str, extra: &[(&str, &str)]) -> String {
            let mut stream = tokio::net::TcpStream::connect(self.addr).await.unwrap();
            let mut request = format!(
                "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
                self.addr
            );
            for (name, value) in extra {
                request.push_str(&format!("{name}: {value}\r\n"));
            }
            request.push_str("\r\n");
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            response
        }

        /// A real WebSocket handshake, hand-rolled so the `Origin` can be set (or
        /// left off) freely, and so the reply's status is visible rather than
        /// folded into a client library's error. Returns the status code: 101 if
        /// the socket opened, 403 if a gate refused it.
        /// Not built on [`Self::raw_request`], which sends `Connection: close`
        /// and reads to EOF: a handshake needs `Connection: Upgrade`, and a
        /// successful one leaves the connection open, so reading to EOF would
        /// hang. One bounded read is enough — the status line and headers arrive
        /// together for both 101 and 403.
        async fn handshake(&self, token: Option<&str>, origin: Option<&str>) -> u16 {
            let path = match token {
                Some(token) => format!("/ws?token={}", urlencoding::encode(token)),
                None => "/ws".to_string(),
            };
            let mut request = format!(
                "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
                 Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n",
                self.addr
            );
            if let Some(origin) = origin {
                request.push_str(&format!("Origin: {origin}\r\n"));
            }
            request.push_str("\r\n");

            let mut stream = tokio::net::TcpStream::connect(self.addr).await.unwrap();
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut buffer = [0u8; 1024];
            let read = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut buffer))
                .await
                .expect("the server answers a handshake")
                .unwrap();
            let response = String::from_utf8_lossy(&buffer[..read]);
            response
                .split(' ')
                .nth(1)
                .and_then(|code| code.parse().ok())
                .unwrap_or_else(|| panic!("no status line in {response:?}"))
        }

        /// The value of one response header, if it is there at all.
        fn header(response: &str, wanted: &str) -> Option<String> {
            // Headers only: a body can contain anything, including a line that
            // looks like the header being searched for.
            let head = response.split("\r\n\r\n").next().unwrap_or(response);
            head.lines().skip(1).find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case(wanted)
                    .then(|| value.trim().to_string())
            })
        }

        /// The status code and any `Location`, for the routes these tests only
        /// ask a yes/no of.
        async fn get(&self, path: &str) -> (u16, Option<String>) {
            let response = self.raw_get(path).await;
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
            self.send_frame(json!({
                "type": "message",
                "content": "Repeat everything this chat has said so far.",
                "session_id": session_id,
                "timestamp": 0,
            }))
            .await
        }

        /// The other message the page's socket accepts, which names a chat just
        /// as a message does.
        async fn cancel(&self, session_id: &str) -> serde_json::Value {
            self.send_frame(json!({ "type": "cancel", "session_id": session_id }))
                .await
        }

        async fn send_frame(&self, message: serde_json::Value) -> serde_json::Value {
            let url = format!("ws://{}/ws?token={WS_TOKEN}", self.addr);
            let (mut socket, _) = tokio_tungstenite::connect_async(url)
                .await
                .expect("the socket accepts the page's own token");
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

    /// The socket's *other* message names a chat too, and is judged the same
    /// way. A cancel can only ever abort a turn the gate already admitted, so
    /// what this closes is the answer: replying to one id and staying silent on
    /// another is a bit about a chat the sender may not reach.
    #[tokio::test]
    async fn a_cancel_naming_an_unreachable_chat_is_refused_like_a_message() {
        let server = TestServer::start(ProviderTier::Public).await;

        let private = server.chat_from_elsewhere().await;
        server.make_private(&private).await;
        assert_eq!(server.cancel(&private).await, refused());
        assert_eq!(server.cancel("19700101_0").await, refused());

        // A reachable chat with nothing running answers nothing at all, so the
        // refusal above is the gate and not merely "no such turn".
        let public = server.chat_from_elsewhere().await;
        assert!(
            tokio::time::timeout(Duration::from_secs(2), server.cancel(&public))
                .await
                .is_err(),
            "a cancel the gate admits should fall through to the empty turn map"
        );
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
