//! Deleting an app closes the agent sockets it still has open, over a real
//! loopback TCP connection to the daemon's real router.
//!
//! Before, a socket open when its app was deleted kept running until its page
//! closed it. Its agent's files, data and compute tools are bound to the path
//! `<store>/<id>/workspace`, not to the app, so the deleted app's page could
//! go on reading and writing the workspace of whatever was built under the id
//! next (`create_app {id, replace: true}` from another chat).
//! `routes::apps::forget_app_runtime` now cancels a token every socket of the
//! app watches, and the socket ends as if its page had gone away.
//!
//! **Why it is here and not beside the code.** `tower::ServiceExt::oneshot`
//! cannot reach a WebSocket handler: axum's `WebSocketUpgrade` needs a
//! connection hyper is really serving (see `workspace_socket.rs`), and the unit
//! tests in `routes::apps` can only show the token being cancelled, not the
//! socket loop answering it.
//!
//! The same real socket carries the turn-level contracts a unit test cannot
//! see either: an app's `guardrails.needs_approval` raising an `approval` frame
//! in Auto mode, and a turn a usage limit stops ending with that sentence.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so the app store and `sessions.db` here are not the
// developer's real ones.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use futures::StreamExt;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::Message as ClientMessage;

const SECRET: &str = "app-socket-lifecycle-secret";
const APP: &str = "socket-retired-on-delete";

/// The page `serve_index` serves for `APP`, fetched as a client that is not a
/// browser, and the socket token embedded in it.
async fn served_page_token(addr: std::net::SocketAddr) -> String {
    served_page_token_for(addr, APP).await
}

async fn served_page_token_for(addr: std::net::SocketAddr, app: &str) -> String {
    let page = client()
        .get(format!("http://{addr}/apps/{app}/"))
        .header("X-Secret-Key", SECRET)
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), 200);
    let html = page.text().await.unwrap();
    let start = html
        .find("\"wsToken\":\"")
        .expect("the page carries its socket token")
        + "\"wsToken\":\"".len();
    html.get(start..start + 32)
        .expect("the socket token is 32 ASCII hex characters")
        .to_string()
}

/// Straight to the loopback listener, whatever proxy the environment names.
fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(addr: std::net::SocketAddr, token: &str) -> Result<Socket, WsError> {
    connect_to(addr, APP, token).await
}

async fn connect_to(addr: std::net::SocketAddr, app: &str, token: &str) -> Result<Socket, WsError> {
    let mut request = format!("ws://{addr}/apps/{app}/agent?token={token}")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("X-Secret-Key", SECRET.parse().unwrap());
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(socket, _)| socket)
}

#[tokio::test]
async fn deleting_an_app_closes_its_open_agent_socket() {
    let store = biorouter_mcp::agent_drafter::store::ArtifactStore::new(
        biorouter_mcp::agent_drafter::default_root(),
    );
    store
        .create_with_id(
            APP,
            "Socket retired on delete",
            "",
            biorouter_mcp::agent_drafter::store::ArtifactKind::Static,
            "index.html",
            &[(
                "index.html".to_string(),
                "<!doctype html><html><head></head><body>app</body></html>".to_string(),
            )],
        )
        .unwrap();

    let state = biorouter_server::state::AppState::new().await.unwrap();
    let app = biorouter_server::routes::configure(state, SECRET.to_string()).layer(
        axum::middleware::from_fn_with_state(
            SECRET.to_string(),
            biorouter_server::auth::check_token,
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await;
    });

    let token = served_page_token(addr).await;
    let mut socket = connect(addr, &token)
        .await
        .expect("the app's own token upgrades");
    // The socket is up and its loop is running once it says `ready`.
    let ready = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            match socket.next().await {
                Some(Ok(ClientMessage::Text(text))) => {
                    let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                    if frame["type"] == "ready" {
                        return frame;
                    }
                    assert_ne!(frame["type"], "error", "{frame}");
                }
                Some(Ok(_)) => {}
                other => panic!("the socket ended before it was ready: {other:?}"),
            }
        }
    })
    .await
    .expect("the socket says it is ready");
    assert_eq!(ready["protocol"], 2);

    let deleted = client()
        .delete(format!("http://{addr}/apps/{APP}"))
        .header("X-Secret-Key", SECRET)
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), 200);

    // Nothing is sent on the socket, so only the delete can end it.
    let ended = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            match socket.next().await {
                None | Some(Err(_)) | Some(Ok(ClientMessage::Close(_))) => return,
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(ended.is_ok(), "the deleted app's socket was left open");

    // And its token opens nothing any more.
    match connect(addr, &token).await {
        Err(WsError::Http(response)) => assert_eq!(response.status(), 403),
        other => panic!("a deleted app's token must be refused, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------
// Turns over the real socket
// ---------------------------------------------------------------------------------------------

/// A model that asks for `files__files_write` once, then answers in text once
/// the call has a result (allowed or denied).
struct WritesAFile;

#[async_trait::async_trait]
impl biorouter::providers::base::Provider for WritesAFile {
    fn metadata() -> biorouter::providers::base::ProviderMetadata {
        biorouter::providers::base::ProviderMetadata::empty()
    }

    fn get_name(&self) -> &str {
        "app-socket-scripted"
    }

    fn get_model_config(&self) -> biorouter::model::ModelConfig {
        biorouter::model::ModelConfig::new_or_fail("scripted")
    }

    async fn complete_with_model(
        &self,
        _: &biorouter::model::ModelConfig,
        _: &str,
        messages: &[biorouter::conversation::message::Message],
        _: &[rmcp::model::Tool],
    ) -> Result<
        (
            biorouter::conversation::message::Message,
            biorouter::providers::base::ProviderUsage,
        ),
        biorouter::providers::errors::ProviderError,
    > {
        use biorouter::conversation::message::{Message, MessageContent};
        let answered = messages.last().is_some_and(|last| {
            last.content
                .iter()
                .any(|c| matches!(c, MessageContent::ToolResponse(_)))
        });
        let reply = if answered {
            Message::assistant().with_text("finished")
        } else {
            Message::assistant().with_tool_request(
                "call_write_1",
                Ok(rmcp::model::CallToolRequestParams {
                    task: None,
                    meta: None,
                    name: "files__files_write".into(),
                    arguments: Some(rmcp::object!({"path": "note.txt", "content": "hi"})),
                }),
            )
        };
        Ok((
            reply,
            biorouter::providers::base::ProviderUsage::new(
                "scripted".to_string(),
                biorouter::providers::base::Usage::default(),
            ),
        ))
    }
}

/// A daemon serving the real router on a loopback port, and its state.
async fn daemon() -> (
    std::net::SocketAddr,
    std::sync::Arc<biorouter_server::state::AppState>,
) {
    let state = biorouter_server::state::AppState::new().await.unwrap();
    let app = biorouter_server::routes::configure(state.clone(), SECRET.to_string()).layer(
        axum::middleware::from_fn_with_state(
            SECRET.to_string(),
            biorouter_server::auth::check_token,
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await;
    });
    (addr, state)
}

/// Create an app whose agent configuration is `agent`. Its kind is `Static`,
/// which needs no built bundle to be served (an agentic app with none is
/// served its build failure page); the socket reads the manifest's `agent`
/// block whatever the kind.
fn agentic_app(id: &str, agent: serde_json::Value) {
    let store = biorouter_mcp::agent_drafter::store::ArtifactStore::new(
        biorouter_mcp::agent_drafter::default_root(),
    );
    let mut manifest = store
        .create_with_id(
            id,
            id,
            "",
            biorouter_mcp::agent_drafter::store::ArtifactKind::Static,
            "index.html",
            &[(
                "index.html".to_string(),
                "<!doctype html><html><head></head><body>app</body></html>".to_string(),
            )],
        )
        .unwrap();
    manifest.agent = Some(serde_json::from_value(agent).unwrap());
    store.save_manifest(&manifest).unwrap();
}

/// The next text frame, as JSON.
async fn next_frame(socket: &mut Socket) -> serde_json::Value {
    loop {
        match socket.next().await {
            Some(Ok(ClientMessage::Text(text))) => return serde_json::from_str(&text).unwrap(),
            Some(Ok(_)) => {}
            other => panic!("the socket ended: {other:?}"),
        }
    }
}

/// Open `app`'s socket and wait for its `ready` frame.
async fn opened(addr: std::net::SocketAddr, app: &str) -> (Socket, serde_json::Value) {
    let token = served_page_token_for(addr, app).await;
    let mut socket = connect_to(addr, app, &token)
        .await
        .expect("the app's own token upgrades");
    let ready = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let frame = next_frame(&mut socket).await;
            if frame["type"] == "ready" {
                return frame;
            }
        }
    })
    .await
    .expect("the socket says it is ready");
    (socket, ready)
}

fn send(frame: serde_json::Value) -> ClientMessage {
    ClientMessage::Text(frame.to_string().into())
}

/// Every frame until the turn's `done` or `error`, answering an `approval`
/// with `reject` so the turn can move on.
async fn turn_frames(socket: &mut Socket) -> Vec<serde_json::Value> {
    use futures::SinkExt;
    let mut frames = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let frame = next_frame(socket).await;
            let kind = frame["type"].as_str().unwrap_or_default().to_string();
            if kind == "approval" {
                socket
                    .send(send(serde_json::json!({
                        "type": "reject",
                        "request": frame["requestId"],
                        "reason": "test",
                    })))
                    .await
                    .unwrap();
            }
            frames.push(frame);
            if kind == "done" || kind == "error" {
                // A second terminal frame would arrive at once; give it a moment.
                if let Ok(frame) =
                    tokio::time::timeout(std::time::Duration::from_millis(300), next_frame(socket))
                        .await
                {
                    frames.push(frame);
                }
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the turn never ended: {frames:?}"));
    frames
}

/// Run `body` on the runtime the daemon itself uses. An agent turn needs the
/// daemon's worker stacks (`biorouter::execution::runtime`): on tokio's
/// default 2 MiB a turn overflows the stack and aborts the whole binary.
fn on_the_daemons_runtime(body: impl std::future::Future<Output = ()>) {
    biorouter::execution::runtime::build_agent_runtime()
        .unwrap()
        .block_on(body);
}

/// rq2-193/198/211: an app that lists a tool in `guardrails.needs_approval`
/// promises the person a pause before it runs. In Auto mode (the daemon's
/// default) the tool ran with no `approval` frame: nothing read the list.
#[test]
fn a_tool_listed_in_needs_approval_raises_an_approval_frame_in_auto_mode() {
    on_the_daemons_runtime(needs_approval_in_auto_mode());
}

async fn needs_approval_in_auto_mode() {
    use futures::SinkExt;
    const ID: &str = "needs-approval-in-auto";
    agentic_app(
        ID,
        serde_json::json!({
            "system_prompt": "write the note",
            "capabilities": { "files": {} },
            "guardrails": { "needs_approval": ["files__files_write"] },
        }),
    );
    let (addr, state) = daemon().await;
    let (mut socket, ready) = opened(addr, ID).await;
    let session = ready["sessionId"].as_str().unwrap().to_string();
    let agent = state.get_agent(session.clone()).await.unwrap();
    agent
        .update_provider(std::sync::Arc::new(WritesAFile), &session)
        .await
        .unwrap();

    socket
        .send(send(
            serde_json::json!({"type": "prompt", "text": "write it"}),
        ))
        .await
        .unwrap();
    let frames = turn_frames(&mut socket).await;
    let approval = frames
        .iter()
        .find(|f| f["type"] == "approval")
        .unwrap_or_else(|| panic!("no approval frame for a listed tool: {frames:?}"));
    assert_eq!(approval["tool"], "files__files_write", "{approval}");
    assert!(
        !frames
            .iter()
            .any(|f| f["type"] == "tool" && f["status"] == "completed"),
        "the rejected tool must not have run: {frames:?}"
    );
    let workspace = biorouter_mcp::agent_drafter::store::ArtifactStore::new(
        biorouter_mcp::agent_drafter::default_root(),
    )
    .artifact_dir(ID)
    .unwrap()
    .join("workspace")
    .join("note.txt");
    assert!(!workspace.exists(), "the rejected write landed");
}

/// rq2-207/210: at a usage limit the agent stores a stop sentence and makes no
/// model request, and the app socket dropped that notice and sent a bare
/// `done`, so the page showed nothing at all. The turn now ends with the
/// `error` frame, code `usage_limit`, carrying the sentence the desktop shows.
#[test]
fn a_turn_stopped_by_a_usage_limit_sends_the_sentence_not_a_bare_done() {
    on_the_daemons_runtime(usage_limit_stops_the_turn());
}

async fn usage_limit_stops_the_turn() {
    use biorouter::agents::usage_limit::{self, ChatLimitOverride, CHAT_LIMIT_REACHED_PREFIX};
    use futures::SinkExt;
    const ID: &str = "usage-limit-stops-the-turn";
    agentic_app(ID, serde_json::json!({ "system_prompt": "answer" }));
    let (addr, state) = daemon().await;
    let (mut socket, ready) = opened(addr, ID).await;
    let session = ready["sessionId"].as_str().unwrap().to_string();
    let agent = state.get_agent(session.clone()).await.unwrap();
    agent
        .update_provider(std::sync::Arc::new(WritesAFile), &session)
        .await
        .unwrap();

    // The chat has spent more than its own limit already.
    state
        .session_manager()
        .apply_side_usage_event(biorouter::session::SideUsageEntry {
            event_key: "app-socket-usage-limit:seed".to_string(),
            session_id: session.clone(),
            provider: "app-socket-scripted".to_string(),
            model_id: "scripted".to_string(),
            input_tokens: Some(1_000),
            output_tokens: Some(0),
        })
        .await
        .unwrap();
    usage_limit::set_for_session(
        state.session_manager(),
        &session,
        Some(ChatLimitOverride::Limit {
            max_tokens: Some(100),
            max_usd: None,
        }),
    )
    .await
    .unwrap();

    socket
        .send(send(serde_json::json!({"type": "prompt", "text": "hello"})))
        .await
        .unwrap();
    let frames = turn_frames(&mut socket).await;
    let error = frames
        .iter()
        .find(|f| f["type"] == "error")
        .unwrap_or_else(|| panic!("the stop was not said: {frames:?}"));
    assert_eq!(error["code"], "usage_limit", "{error}");
    assert!(
        error["message"]
            .as_str()
            .unwrap_or_default()
            .starts_with(CHAT_LIMIT_REACHED_PREFIX),
        "the page must get the sentence the desktop shows: {error}"
    );
    assert!(
        !frames.iter().any(|f| f["type"] == "done"),
        "a stopped turn is not a finished one: {frames:?}"
    );
}
