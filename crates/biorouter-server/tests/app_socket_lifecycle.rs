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
    let page = client()
        .get(format!("http://{addr}/apps/{APP}/"))
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
    html[start..start + 32].to_string()
}

/// Straight to the loopback listener, whatever proxy the environment names.
fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

async fn connect(
    addr: std::net::SocketAddr,
    token: &str,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    WsError,
> {
    let mut request = format!("ws://{addr}/apps/{APP}/agent?token={token}")
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
