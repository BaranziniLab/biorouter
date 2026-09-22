//! The tunnel's websocket connect, on a socket no child process inherits.
//!
//! `tokio_tungstenite::connect_async` dials with `tokio::net::TcpStream::connect`
//! (tokio-tungstenite 0.28, `src/connect.rs`), and on Windows that socket is
//! inheritable: mio creates it with a plain `socket()` call. The tunnel's
//! websocket lives inside `biorouterd` for as long as the tunnel is up, and the
//! daemon spawns MCP servers, shells and agents throughout, so every one of
//! those children got a handle to it. When the daemon then dropped the
//! connection (the idle-timeout reconnect in `lapstone.rs`, or exiting), the
//! socket would stay open until the last of them exited: no FIN reaches the
//! relay, which may keep a half-dead connection registered for the agent id.
//! (Read from the sources, as for the listeners in `biorouter::net`; the
//! Windows tests there are where the connecting half is first run.)
//!
//! [`connect_async_non_inheritable`] is `connect_async` with only the socket
//! changed. `scripts/check-non-inheritable-sockets.sh` forbids, in production
//! code, both the `connect_async` family and the `tokio::net::TcpStream::connect`
//! it dials with, so neither a new call site nor an edit of the connect below
//! back to tokio's can bring the old socket back.

use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::error::{Error, UrlError};
use tokio_tungstenite::tungstenite::handshake::client::{Request, Response};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// `tokio_tungstenite::connect_async`, on a TCP socket that no child process
/// inherits.
///
/// Step for step what `connect_async(request)` does in tokio-tungstenite 0.28
/// (`connect_async_with_config(request, None, false)`, then the private
/// `connect`):
///
/// 1. `into_client_request()`: the same URL parsing and the same request
///    headers.
/// 2. The host from [`domain`], the port from the URL or the scheme (443 for
///    `wss`, 80 for `ws`, `UnsupportedUrlScheme` otherwise), joined as
///    `"{domain}:{port}"`, the same string it resolves.
/// 3. The TCP connect: [`biorouter::net::connect_non_inheritable`], which
///    resolves that string and tries every address in turn exactly as
///    `tokio::net::TcpStream::connect` does, with an I/O error mapped to
///    `Error::Io` as there. Nagle is left on (`disable_nagle` is `false`).
/// 4. `client_async_tls_with_config(request, socket, None, None)`: the very
///    function `connect_async` ends in, with the same arguments, so `ws` versus
///    `wss`, the TLS connector (rustls with the native roots, as this crate
///    enables it) and the handshake are not a copy but the same code.
pub async fn connect_async_non_inheritable<R>(
    request: R,
) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, Response), Error>
where
    R: IntoClientRequest + Unpin,
{
    let request = request.into_client_request()?;
    let domain = domain(&request)?;
    let port = request
        .uri()
        .port_u16()
        .or_else(|| match request.uri().scheme_str() {
            Some("wss") => Some(443),
            Some("ws") => Some(80),
            _ => None,
        })
        .ok_or(Error::Url(UrlError::UnsupportedUrlScheme))?;

    let addr = format!("{domain}:{port}");
    let socket = biorouter::net::connect_non_inheritable(addr)
        .await
        .map_err(Error::Io)?;

    tokio_tungstenite::client_async_tls_with_config(request, socket, None, None).await
}

/// tokio-tungstenite's private `domain`, as it is compiled with rustls
/// (`__rustls-tls`, which `rustls-tls-native-roots` enables in this crate):
/// the URL's host, with an IPv6 literal's brackets removed, because rustls
/// expects the bare address.
fn domain(request: &Request) -> Result<String, Error> {
    match request.uri().host() {
        Some(host) => Ok(host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .unwrap_or(host)
            .to_string()),
        None => Err(Error::Url(UrlError::NoHostName)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{SinkExt, StreamExt};
    use std::net::{Ipv4Addr, SocketAddr};
    use tokio_tungstenite::tungstenite::handshake::server::{
        ErrorResponse, Request as ServerRequest, Response as ServerResponse,
    };
    use tokio_tungstenite::tungstenite::Message;

    /// What a server saw of one handshake request: the target, and every header
    /// but the random `Sec-WebSocket-Key`.
    type SeenRequest = (String, Vec<(String, String)>);

    /// A websocket server on 127.0.0.1 that records each handshake and echoes
    /// one text message per connection. Test code: a tokio listener is fine
    /// here, since nothing in a test outlives the process.
    async fn echo_server(
        connections: usize,
    ) -> (SocketAddr, tokio::task::JoinHandle<Vec<SeenRequest>>) {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut seen = Vec::new();
            for _ in 0..connections {
                let (stream, _) = listener.accept().await.unwrap();
                let mut request = None;
                let record = |req: &ServerRequest,
                              response: ServerResponse|
                 -> Result<ServerResponse, ErrorResponse> {
                    let headers = req
                        .headers()
                        .iter()
                        .filter(|(name, _)| *name != "sec-websocket-key")
                        .map(|(name, value)| {
                            (name.to_string(), value.to_str().unwrap().to_string())
                        })
                        .collect();
                    request = Some((req.uri().to_string(), headers));
                    Ok(response)
                };
                let mut ws = tokio_tungstenite::accept_hdr_async(stream, record)
                    .await
                    .unwrap();
                seen.push(request.expect("the handshake callback ran"));
                if let Some(Ok(Message::Text(text))) = ws.next().await {
                    ws.send(Message::Text(format!("echo: {text}").into()))
                        .await
                        .unwrap();
                }
                let _ = ws.close(None).await;
            }
            seen
        });
        (addr, server)
    }

    async fn say_hello(
        ws: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    ) -> Option<Result<Message, Error>> {
        ws.send(Message::Text("hello".into())).await.unwrap();
        ws.next().await
    }

    /// A port that refuses: bound, then closed without ever listening, so no
    /// child of another test can be holding a listening copy of it (the
    /// reasoning, and why a port merely held by a bound socket does not work on
    /// macOS, is on the same helper in `biorouter::net`'s tests).
    fn refused_port() -> u16 {
        let socket =
            socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
        socket
            .bind(&SocketAddr::from((Ipv4Addr::LOCALHOST, 0)).into())
            .unwrap();
        socket.local_addr().unwrap().as_socket().unwrap().port()
    }

    /// The new connect completes a `ws://` handshake, carries messages both
    /// ways, and sends the server the same request `connect_async` does.
    #[tokio::test]
    async fn completes_a_ws_handshake_with_the_request_connect_async_sends() {
        let (addr, server) = echo_server(3).await;
        let url = format!("ws://{addr}/connect?agent_id=abc");

        let (mut ours, response) = connect_async_non_inheritable(url.clone()).await.unwrap();
        assert_eq!(response.status(), 101);
        let reply = say_hello(&mut ours).await;
        assert!(
            matches!(&reply, Some(Ok(Message::Text(t))) if t.as_str() == "echo: hello"),
            "{reply:?}"
        );
        drop(ours);

        let (mut theirs, _) = tokio_tungstenite::connect_async(url.clone()).await.unwrap();
        say_hello(&mut theirs).await;
        drop(theirs);

        // A name that needs resolving. If `localhost` resolves to `::1` first,
        // where nothing listens, this also shows the next address being tried.
        let by_name = format!("ws://localhost:{}/connect?agent_id=abc", addr.port());
        let (mut named, _) = connect_async_non_inheritable(by_name).await.unwrap();
        say_hello(&mut named).await;
        drop(named);

        let seen = server.await.unwrap();
        assert_eq!(seen[0].0, "/connect?agent_id=abc");
        assert_eq!(
            seen[0], seen[1],
            "the server must see the same request from both connects"
        );
        let header = |name: &str| {
            seen[0]
                .1
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(header("host"), Some(addr.to_string()));
        assert_eq!(header("upgrade"), Some("websocket".to_string()));
    }

    /// Each URL fails the new connect exactly as it fails `connect_async`,
    /// compared by the whole error.
    ///
    /// Every comparison runs at once, and so do the two connects inside it. On
    /// Windows a connect to a refusing loopback port is not refused at once: the
    /// stack retries the SYN and reports the refusal about two seconds later.
    /// Run one after another, the six refused connects below cost twelve
    /// seconds, in the lib target and again in the bin target that compiles
    /// this module too; run together they cost one such wait. They share
    /// nothing but the refusing port, which refuses each of them, and every
    /// error is still compared whole, URL by URL.
    #[tokio::test]
    async fn fails_the_way_connect_async_fails() {
        let port = refused_port();
        let urls = [
            // A port that refuses the connection.
            format!("ws://127.0.0.1:{port}/connect"),
            format!("wss://127.0.0.1:{port}/connect"),
            // An IPv6 literal, whose brackets both connects strip.
            format!("ws://[::1]:{port}/connect"),
            // Bad URLs: nothing is dialled.
            "not a url".to_string(),
            String::new(),
            "ws://".to_string(),
            "http://127.0.0.1/connect".to_string(),
            "ftp://127.0.0.1/connect".to_string(),
            "ws://[::1/connect".to_string(),
        ];
        let compare = |url: String| async move {
            let (ours, theirs) = tokio::join!(
                connect_async_non_inheritable(url.as_str()),
                tokio_tungstenite::connect_async(url.as_str()),
            );
            let ours = ours.map(|_| ()).expect_err("the new connect must fail");
            let theirs = theirs.map(|_| ()).expect_err("connect_async must fail");
            (url, format!("{ours:?}"), format!("{theirs:?}"))
        };
        let compared = futures::future::join_all(urls.into_iter().map(compare)).await;
        assert_eq!(compared.len(), 9, "every URL must have been compared");
        for (url, ours, theirs) in compared {
            assert_eq!(
                ours, theirs,
                "{url:?} must fail the same way through both connects"
            );
        }
    }

    /// A `wss://` URL takes the same TLS path as `connect_async`: against a
    /// server that answers the handshake with plain text, both fail in rustls
    /// with the same error.
    #[tokio::test]
    async fn a_wss_url_goes_through_the_same_tls_connector() {
        // The daemon installs this before it connects (`run_single_connection`).
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                stream
                    .write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")
                    .await
                    .unwrap();
                // Read until the client gives up, so the close is the client's
                // and no reset races its error.
                let mut sink = Vec::new();
                let _ = stream.read_to_end(&mut sink).await;
            }
        });
        let url = format!("wss://{addr}/connect");
        let ours = connect_async_non_inheritable(url.as_str())
            .await
            .map(|_| ())
            .expect_err("a plain-text answer must fail the TLS handshake");
        let theirs = tokio_tungstenite::connect_async(url.as_str())
            .await
            .map(|_| ())
            .expect_err("a plain-text answer must fail the TLS handshake");
        assert_eq!(format!("{ours:?}"), format!("{theirs:?}"));
        server.await.unwrap();
    }

    #[test]
    fn the_domain_is_the_host_with_ipv6_brackets_removed() {
        let host = |url: &str| domain(&url.into_client_request().unwrap()).unwrap();
        assert_eq!(host("wss://relay.example.com/connect"), "relay.example.com");
        assert_eq!(host("ws://127.0.0.1:8080/connect"), "127.0.0.1");
        assert_eq!(host("ws://[::1]:8080/connect"), "::1");
    }
}
