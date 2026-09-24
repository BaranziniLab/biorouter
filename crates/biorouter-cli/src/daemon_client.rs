//! Human-operated clients for the profile's shared daemon.
use anyhow::{bail, ensure, Context, Result};
use biorouter::crew::observation::{ObserveEvent, ObserveRequest};
use biorouter::daemon_runtime::{self, Descriptor, Identity};
use serde_json::{json, Value};
use std::io::{IsTerminal, Read};
use std::time::Duration;
use zeroize::Zeroizing;

pub struct CrewClient {
    descriptor: Descriptor,
    proof: Zeroizing<String>,
}

#[derive(Debug)]
pub struct DaemonRefusal {
    pub status: u16,
    pub kind: Option<String>,
    /// The broker's own code when the daemon forwarded a broker refusal (`name_taken`,
    /// `already_approved`, …): the body's `broker_code`, or the code inside an older daemon's
    /// `Crew broker refused request: {json}` text.
    pub broker_code: Option<String>,
    message: String,
}

impl DaemonRefusal {
    /// The daemon's text, terminal-safe. For a broker refusal it is the broker's own
    /// `code: sentence`, with an older daemon's envelope removed.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for DaemonRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Daemon returned {}: {}",
            self.status, self.message
        )
    }
}

impl std::error::Error for DaemonRefusal {}

#[cfg(unix)]
fn daemon_refusal(status: u16, value: Option<&Value>, fallback: &str) -> DaemonRefusal {
    let message = value
        .and_then(|value| value.get("error").or_else(|| value.get("message")))
        .and_then(Value::as_str)
        .unwrap_or(fallback);
    // An older daemon forwarded a broker refusal as `Crew broker refused request: {json}`.
    let legacy = legacy_broker_refusal(message);
    let broker_code = value
        .and_then(|value| value.get("broker_code"))
        .and_then(Value::as_str)
        .and_then(broker_code_text)
        .or_else(|| legacy.as_ref().map(|(code, _)| code.clone()));
    let message = legacy.as_ref().map_or(message, |(_, text)| text.as_str());
    DaemonRefusal {
        status,
        kind: value
            .and_then(|value| value.get("status").or_else(|| value.get("code")))
            .and_then(Value::as_str)
            .map(|kind| kind.chars().take(128).collect()),
        broker_code,
        message: terminal_safe(&message.chars().take(1024).collect::<String>()),
    }
}

/// What an older daemon wrote before the broker's JSON error object.
#[cfg(unix)]
const LEGACY_BROKER_REFUSAL: &str = "Crew broker refused request: ";

/// A broker code as the broker writes one (`[a-z][a-z0-9_]*`), capped like `kind`. Anything
/// else is not a code and is dropped rather than printed.
#[cfg(unix)]
fn broker_code_text(code: &str) -> Option<String> {
    let mut chars = code.chars();
    let shaped = chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_');
    shaped.then(|| code.chars().take(128).collect())
}

/// `Crew broker refused request: {"code":…,"message":…}` from a daemon that predates
/// `broker_code`: the code, and the broker's own text (its message, else the code alone).
#[cfg(unix)]
fn legacy_broker_refusal(text: &str) -> Option<(String, String)> {
    let encoded = text.trim().strip_prefix(LEGACY_BROKER_REFUSAL)?;
    let error = serde_json::from_str::<Value>(encoded).ok()?;
    let code = broker_code_text(error.get("code")?.as_str()?)?;
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .unwrap_or(&code)
        .to_owned();
    Some((code, message))
}

/// The daemon's sentence exactly, quotes and apostrophes included, with only what a terminal
/// would act on escaped: control characters and invisible formatting. `escape_debug` used to
/// run here and printed `the model\'s` in text and `model\\'s` in JSON, so a script matching
/// the daemon's canonical sentence missed it.
#[cfg(unix)]
fn terminal_safe(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    for ch in message.chars() {
        if ch.is_control() || biorouter::utils::is_invisible_formatting(ch) {
            out.extend(ch.escape_default());
        } else {
            out.push(ch);
        }
    }
    out
}

impl CrewClient {
    #[cfg(all(test, unix))]
    pub(crate) fn for_test(descriptor: Descriptor, proof: &str) -> Self {
        Self {
            descriptor,
            proof: Zeroizing::new(proof.to_owned()),
        }
    }

    pub async fn connect(no_start: bool) -> Result<Self> {
        Self::connect_with_input(no_start, false).await
    }

    pub async fn connect_with_input(no_start: bool, approval_key_stdin: bool) -> Result<Self> {
        let descriptor = match discover_shared_daemon().await? {
            Some(descriptor) => descriptor,
            None => {
                ensure!(
                    !no_start,
                    "No live shared daemon is available; run biorouter crew daemon start"
                );
                let proof = read_approval_secret(
                    "Crew approval secret (held separately from your profile):",
                    approval_key_stdin,
                )
                .await?;
                let descriptor = start_daemon(&proof).await?;
                return Ok(Self { descriptor, proof });
            }
        };
        ensure!(
            descriptor.user_action_installed,
            "This daemon has no human approval authority; restart it with the trusted Crew terminal launcher"
        );
        let proof = read_approval_secret(
            "Crew approval secret (printable ASCII, no spaces):",
            approval_key_stdin,
        )
        .await?;
        let client = Self { descriptor, proof };
        client.request("GET", "/crew/connections", None).await?;
        Ok(client)
    }

    pub async fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        request(
            &self.descriptor,
            Some(self.proof.as_str()),
            method,
            path,
            body,
        )
        .await
    }

    pub async fn request_text(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<String> {
        #[cfg(not(unix))]
        {
            let _ = (method, path, body);
            bail!("Shared daemon IPC is unavailable on this platform");
        }
        #[cfg(unix)]
        {
            let (response, _connection) = self
                .authenticated_response(method, path, body, "application/json, text/plain")
                .await?;
            let content_type = response_media_type(&response)?;
            let bytes = bounded_response(response, 16 * 1024 * 1024).await?;
            if bytes.is_empty() {
                return Ok(String::new());
            }
            let text = String::from_utf8(bytes.to_vec()).context("Daemon response is not UTF-8")?;
            match content_type.as_str() {
                "application/json" => {
                    serde_json::from_str::<Value>(&text).context("Invalid daemon JSON response")?;
                }
                "text/plain" => (),
                _ => bail!("Daemon returned an unsupported response content type"),
            }
            Ok(text)
        }
    }

    pub async fn event_stream(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<DaemonEventStream> {
        #[cfg(not(unix))]
        {
            let _ = (method, path, body);
            bail!("Shared daemon event streaming is unavailable on this platform");
        }
        #[cfg(unix)]
        {
            let (response, connection) = self
                .authenticated_response(method, path, body, "text/event-stream")
                .await?;
            ensure!(
                response_media_type(&response)? == "text/event-stream",
                "Daemon returned an invalid event stream content type"
            );
            Ok(DaemonEventStream {
                body: response.into_body(),
                connection,
                chunk: bytes::Bytes::new(),
                decoder: EventDecoder::default(),
                ended: false,
            })
        }
    }

    #[cfg(unix)]
    async fn authenticated_response(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        accept: &str,
    ) -> Result<(hyper::Response<hyper::body::Incoming>, ObserverConnection)> {
        use http_body_util::Full;
        ensure!(
            path.starts_with('/') && !path.starts_with("//") && !path.contains(['\r', '\n']),
            "Invalid daemon request path"
        );
        let (mut sender, connection) = verified_observer_connection(&self.descriptor).await?;
        let has_json_body = body.is_some();
        let bytes = body
            .map(|body| serde_json::to_vec(&body))
            .transpose()?
            .unwrap_or_default();
        ensure!(
            bytes.len() <= 16 * 1024 * 1024,
            "Daemon request exceeds 16 MiB"
        );
        let mut request = hyper::Request::builder()
            .method(method)
            .uri(path)
            .header("Host", "localhost")
            .header("Accept", accept)
            .header("X-Secret-Key", &self.descriptor.api_secret)
            .header("X-Daemon-Instance", &self.descriptor.instance_id)
            .header("X-User-Action", self.proof.as_str());
        if has_json_body {
            request = request.header("Content-Type", "application/json");
        }
        let request = request.body(Full::new(bytes::Bytes::from(bytes)))?;
        let response = tokio::time::timeout(Duration::from_secs(180), sender.send_request(request))
            .await
            .context("Daemon response timed out; inspect session state before retrying")??;
        if !response.status().is_success() {
            let status = response.status();
            let bytes = bounded_response(response, 16 * 1024)
                .await
                .with_context(|| format!("Daemon returned {status}; refusal body unavailable"))?;
            let value = serde_json::from_slice::<Value>(&bytes).ok();
            return Err(daemon_refusal(
                status.as_u16(),
                value.as_ref(),
                std::str::from_utf8(&bytes)
                    .unwrap_or("Request refused; check the daemon and approval secret"),
            )
            .into());
        }
        Ok((response, connection))
    }

    pub async fn observe<F>(
        &self,
        path: &str,
        body: &ObserveRequest,
        on_frame: F,
    ) -> Result<Option<String>>
    where
        F: FnMut(ObserveEvent) -> Result<std::ops::ControlFlow<Option<String>>>,
    {
        #[cfg(not(unix))]
        {
            let _ = (path, body, on_frame);
            bail!("Shared Crew daemon IPC is unavailable on this platform");
        }
        #[cfg(unix)]
        {
            use bytes::Bytes;
            use http_body_util::Full;
            use hyper::Request;
            ensure!(
                path.starts_with('/') && !path.starts_with("//") && !path.contains(['\r', '\n']),
                "Invalid daemon observer path"
            );
            let (mut sender, _connection) = verified_observer_connection(&self.descriptor).await?;
            let request = Request::builder()
                .method("POST")
                .uri(path)
                .header("Host", "localhost")
                .header("Content-Type", "application/json")
                .header("Accept", "application/x-ndjson")
                .header("X-Secret-Key", &self.descriptor.api_secret)
                .header("X-Daemon-Instance", &self.descriptor.instance_id)
                .header("X-User-Action", self.proof.as_str())
                .body(Full::new(Bytes::from(serde_json::to_vec(body)?)))?;
            let response =
                tokio::time::timeout(Duration::from_secs(180), sender.send_request(request))
                    .await??;
            ensure!(
                response.status().is_success(),
                "Crew observer refused ({}); check the connection, daemon and approval secret",
                response.status()
            );
            ensure!(
                response
                    .headers()
                    .get("content-type")
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value.split(';').next() == Some("application/x-ndjson")),
                "Daemon returned an invalid observer content type"
            );
            tokio::time::timeout(
                Duration::from_secs(610),
                read_observer_frames(response.into_body(), on_frame),
            )
            .await
            .context("Crew observation timed out without a reconnect frame; inspect connection status before watching again")?
        }
    }

    pub async fn authenticate_ssh(&self, connection_id: &str) -> Result<Value> {
        #[cfg(not(unix))]
        {
            let _ = connection_id;
            bail!("Shared Crew terminal authentication is unavailable on this platform");
        }
        #[cfg(unix)]
        {
            use crossterm::{
                event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers},
                terminal,
            };
            use futures::{SinkExt, StreamExt};
            use tokio_tungstenite::tungstenite::Message;
            ensure!(
                std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
                "SSH authentication needs an interactive terminal for native host-key and MFA prompts"
            );
            ensure!(
                connection_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
                "Invalid connection ID"
            );
            let controller = uuid::Uuid::new_v4().to_string();
            let (cols, rows) = terminal::size()?;
            let started = self.request("POST", &format!("/crew/connections/{connection_id}/authentication"), Some(json!({"request_id": uuid::Uuid::new_v4().to_string(), "controller_id": controller, "cols": cols, "rows": rows}))).await?;
            let id = started["authentication_id"]
                .as_str()
                .context("Daemon returned no authentication ID")?;
            let path = format!("/crew/authentication/{id}/terminal");
            let mut socket =
                authenticated_terminal_socket(&self.descriptor, &self.proof, &path, &controller)
                    .await?;
            terminal::enable_raw_mode()?;
            struct RawTerminal;
            impl Drop for RawTerminal {
                fn drop(&mut self) {
                    let _ = crossterm::terminal::disable_raw_mode();
                }
            }
            let _raw = RawTerminal;
            let mut input = EventStream::new();
            let mut exit_code = None;
            let mut authenticated = false;
            loop {
                tokio::select! {
                    frame = socket.next() => match frame {
                        Some(Ok(Message::Binary(bytes))) => { use std::io::Write; std::io::stdout().write_all(&bytes)?; std::io::stdout().flush()?; },
                        Some(Ok(Message::Text(text))) => {
                            let value: Value = serde_json::from_str(&text)?;
                            if value["type"] == "exit" { exit_code = value["exit_code"].as_i64(); authenticated = value["authenticated"].as_bool() == Some(true); break; }
                            if value["type"] == "error" {
                                let message = value.get("code").and_then(Value::as_str)
                                    .and_then(biorouter::crew::authentication::terminal_failure_message)
                                    .unwrap_or("SSH authentication ended; inspect the daemon's connection status");
                                bail!("{message}");
                            }
                        },
                        Some(Ok(Message::Ping(data))) => socket.send(Message::Pong(data)).await?,
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Err(error)) => return Err(error.into()),
                        _ => (),
                    },
                    event = input.next() => match event {
                        Some(Ok(Event::Resize(cols, rows))) => socket.send(Message::Text(json!({"type":"resize","cols":cols,"rows":rows}).to_string().into())).await?,
                        Some(Ok(Event::Key(key))) if key.kind != KeyEventKind::Release => {
                            let data = match key.code {
                                KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) && c.is_ascii() => Some(((c.to_ascii_lowercase() as u8) & 0x1f) as char),
                                KeyCode::Char(c) => Some(c), KeyCode::Enter => Some('\r'), KeyCode::Backspace => Some('\x7f'),
                                KeyCode::Tab => Some('\t'), KeyCode::Esc => Some('\x1b'), _ => None,
                            };
                            if let Some(data) = data { socket.send(Message::Text(json!({"type":"input","data":data.to_string()}).to_string().into())).await?; }
                        },
                        Some(Ok(Event::Paste(data))) => { ensure!(data.len() <= 4096, "Terminal paste exceeds 4096 bytes"); socket.send(Message::Text(json!({"type":"input","data":data}).to_string().into())).await?; },
                        Some(Err(error)) => return Err(error.into()),
                        None => break,
                        _ => (),
                    }
                }
            }
            socket.close(None).await.ok();
            ensure!(
                authenticated,
                "SSH authentication ended before the daemon verified and retained the broker connection"
            );
            Ok(json!({"authentication_id":id,"exit_code":exit_code,"authenticated":true}))
        }
    }
}

// Match the daemon turn replay hard ceiling, with room for SSE field framing.
// This is a client refusal bound, not a claim that every server event is capped.
#[cfg(unix)]
const MAX_SSE_FRAME: usize = 32 * 1024 * 1024 + 64 * 1024;

pub struct DaemonEventStream {
    #[cfg(unix)]
    body: hyper::body::Incoming,
    #[cfg(unix)]
    connection: ObserverConnection,
    #[cfg(unix)]
    chunk: bytes::Bytes,
    #[cfg(unix)]
    decoder: EventDecoder,
    #[cfg(unix)]
    ended: bool,
}
impl DaemonEventStream {
    pub async fn next_event(&mut self) -> Result<Option<Value>> {
        #[cfg(not(unix))]
        bail!("Shared daemon event streaming is unavailable on this platform");
        #[cfg(unix)]
        {
            if self.ended {
                return Ok(None);
            }
            let result = self.read_event().await;
            if !matches!(result, Ok(Some(_))) {
                self.ended = true;
                self.connection.0.abort();
            }
            result
        }
    }
    #[cfg(unix)]
    async fn read_event(&mut self) -> Result<Option<Value>> {
        use bytes::Buf;
        use http_body_util::BodyExt;
        loop {
            while self.chunk.has_remaining() {
                let byte = self.chunk.get_u8();
                if let Some(value) = self.decoder.push(byte)? {
                    return Ok(Some(value));
                }
            }
            let frame = tokio::time::timeout(Duration::from_secs(180), self.body.frame())
                .await
                .context("Daemon event stream stalled; inspect session state before retrying")?;
            let Some(frame) = frame else {
                ensure!(
                    self.decoder.at_boundary(),
                    "Daemon event stream ended with a truncated frame"
                );
                return Ok(None);
            };
            if let Ok(bytes) = frame
                .context("Daemon event stream interrupted; no request was resubmitted")?
                .into_data()
            {
                ensure!(
                    bytes.len() <= MAX_SSE_FRAME,
                    "Daemon stream transport chunk exceeds 32 MiB plus framing allowance"
                );
                self.chunk = bytes;
            }
        }
    }
}

#[cfg(unix)]
#[derive(Default)]
struct EventDecoder {
    line: Vec<u8>,
    data: String,
    has_data: bool,
    frame_bytes: usize,
}
#[cfg(unix)]
impl EventDecoder {
    fn at_boundary(&self) -> bool {
        self.frame_bytes == 0
    }
    fn push(&mut self, byte: u8) -> Result<Option<Value>> {
        self.frame_bytes += 1;
        ensure!(
            self.frame_bytes <= MAX_SSE_FRAME,
            "Daemon SSE frame exceeds 32 MiB plus framing allowance"
        );
        if byte != b'\n' {
            self.line.push(byte);
            return Ok(None);
        }
        if self.line.last() == Some(&b'\r') {
            self.line.pop();
        }
        let line =
            std::str::from_utf8(&self.line).context("Daemon SSE frame contains invalid UTF-8")?;
        if line.is_empty() {
            self.frame_bytes = 0;
            let value = if self.has_data {
                Some(
                    serde_json::from_str(&self.data)
                        .context("Daemon SSE data is not valid JSON")?,
                )
            } else {
                None
            };
            self.data.clear();
            self.has_data = false;
            self.line.clear();
            return Ok(value);
        }
        if let Some(data) = line
            .strip_prefix("data:")
            .or_else(|| (line == "data").then_some(""))
        {
            if self.has_data {
                self.data.push('\n');
            }
            self.data.push_str(data.strip_prefix(' ').unwrap_or(data));
            self.has_data = true;
        }
        self.line.clear();
        Ok(None)
    }
}

#[cfg(unix)]
fn response_media_type(response: &hyper::Response<hyper::body::Incoming>) -> Result<String> {
    Ok(response
        .headers()
        .get("content-type")
        .map(|value| value.to_str())
        .transpose()
        .context("Invalid daemon content type")?
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase())
}

#[cfg(unix)]
async fn bounded_response(
    response: hyper::Response<hyper::body::Incoming>,
    limit: usize,
) -> Result<bytes::Bytes> {
    use http_body_util::{BodyExt, Limited};
    tokio::time::timeout(
        Duration::from_secs(180),
        Limited::new(response.into_body(), limit).collect(),
    )
    .await
    .context("Daemon response body timed out; inspect session state before retrying")?
    .map(|body| body.to_bytes())
    .map_err(|_| anyhow::anyhow!("Daemon response body was interrupted or exceeded its size limit"))
}

#[cfg(unix)]
struct ObserverConnection(tokio::task::JoinHandle<Result<(), hyper::Error>>);

#[cfg(unix)]
impl Drop for ObserverConnection {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(unix)]
async fn verified_observer_connection(
    descriptor: &Descriptor,
) -> Result<(
    hyper::client::conn::http1::SendRequest<http_body_util::Full<bytes::Bytes>>,
    ObserverConnection,
)> {
    use bytes::Bytes;
    use http_body_util::{BodyExt, Full, Limited};
    use hyper::{client::conn::http1, Request};
    let stream = checked_socket(descriptor).await?;
    let (mut sender, connection) = tokio::time::timeout(
        Duration::from_secs(5),
        http1::handshake(hyper_util::rt::TokioIo::new(stream)),
    )
    .await??;
    let connection = ObserverConnection(tokio::spawn(connection));
    let request = Request::builder()
        .method("GET")
        .uri("/daemon/identity")
        .header("Host", "localhost")
        .header("X-Secret-Key", &descriptor.api_secret)
        .body(Full::new(Bytes::new()))?;
    let response =
        tokio::time::timeout(Duration::from_secs(5), sender.send_request(request)).await??;
    ensure!(
        response.status().is_success(),
        "Shared daemon identity was refused"
    );
    let bytes = tokio::time::timeout(
        Duration::from_secs(5),
        Limited::new(response.into_body(), 16 * 1024).collect(),
    )
    .await?
    .map_err(|_| anyhow::anyhow!("Invalid shared daemon identity response"))?
    .to_bytes();
    let identity: Identity = serde_json::from_slice(&bytes)?;
    ensure!(
        identity == descriptor.identity(),
        "Shared daemon identity changed; no approval credentials were sent"
    );
    Ok((sender, connection))
}

#[cfg(unix)]
async fn read_observer_frames<F>(
    mut body: hyper::body::Incoming,
    mut on_frame: F,
) -> Result<Option<String>>
where
    F: FnMut(ObserveEvent) -> Result<std::ops::ControlFlow<Option<String>>>,
{
    use http_body_util::BodyExt;
    const MAX_FRAME: usize = 1024 * 1024;
    let mut pending = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.context(
            "Crew observation interrupted; inspect connection status before watching again",
        )?;
        let Ok(bytes) = frame.into_data() else {
            continue;
        };
        for segment in bytes.split_inclusive(|byte| *byte == b'\n') {
            ensure!(
                pending.len() + segment.len() <= MAX_FRAME,
                "Crew observer frame exceeds 1 MiB"
            );
            pending.extend_from_slice(segment);
            if segment.last() == Some(&b'\n') {
                let value =
                    serde_json::from_slice(&pending).context("Invalid Crew observer frame")?;
                pending.clear();
                if let std::ops::ControlFlow::Break(cursor) = on_frame(value)? {
                    return Ok(cursor);
                }
            }
        }
    }
    bail!("Crew observation ended without a reconnect frame; inspect connection status before watching again")
}

#[cfg(unix)]
async fn authenticated_terminal_socket(
    descriptor: &Descriptor,
    proof: &str,
    path: &str,
    controller: &str,
) -> Result<tokio_tungstenite::WebSocketStream<hyper_util::rt::TokioIo<hyper::upgrade::Upgraded>>> {
    use bytes::Bytes;
    use http_body_util::{BodyExt, Full, Limited};
    use hyper::{client::conn::http1, Request};
    use hyper_util::rt::TokioIo;
    use tokio_tungstenite::{
        tungstenite::{
            client::IntoClientRequest,
            handshake::derive_accept_key,
            protocol::{Role, WebSocketConfig},
        },
        WebSocketStream,
    };
    ensure!(
        path.starts_with('/') && !path.starts_with("//") && !path.contains(['\r', '\n']),
        "Invalid daemon terminal path"
    );
    let stream = checked_socket(descriptor).await?;
    let (mut sender, connection) = tokio::time::timeout(
        Duration::from_secs(5),
        http1::handshake(TokioIo::new(stream)),
    )
    .await??;
    struct ConnectionTask(tokio::task::JoinHandle<Result<(), hyper::Error>>);
    impl Drop for ConnectionTask {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _connection = ConnectionTask(tokio::spawn(connection.with_upgrades()));
    let identity_request = Request::builder()
        .method("GET")
        .uri("/daemon/identity")
        .header("Host", "localhost")
        .header("X-Secret-Key", &descriptor.api_secret)
        .body(Full::new(Bytes::new()))?;
    let response = tokio::time::timeout(
        Duration::from_secs(5),
        sender.send_request(identity_request),
    )
    .await??;
    ensure!(
        response.status().is_success(),
        "Shared daemon identity was refused before terminal authentication"
    );
    let bytes = tokio::time::timeout(
        Duration::from_secs(5),
        Limited::new(response.into_body(), 16 * 1024).collect(),
    )
    .await?
    .map_err(|error| anyhow::anyhow!(error.to_string()))?
    .to_bytes();
    let identity: Identity = serde_json::from_slice(&bytes)?;
    ensure!(
        identity == descriptor.identity(),
        "Shared daemon identity changed; no approval credentials were sent"
    );
    // Upgrade the identity-verified HTTP/1 connection itself. Never reconnect
    // between this check and sending the separately held human approval proof.
    let mut upgrade = format!("ws://localhost{path}").into_client_request()?;
    *upgrade.uri_mut() = path.parse()?;
    let expected_accept = derive_accept_key(upgrade.headers()["Sec-WebSocket-Key"].as_bytes());
    upgrade
        .headers_mut()
        .insert("X-Secret-Key", descriptor.api_secret.parse()?);
    upgrade
        .headers_mut()
        .insert("X-User-Action", proof.parse()?);
    upgrade
        .headers_mut()
        .insert("X-Crew-Controller", controller.parse()?);
    upgrade
        .headers_mut()
        .insert("X-Daemon-Instance", descriptor.instance_id.parse()?);
    let (parts, _) = upgrade.into_parts();
    let mut response = tokio::time::timeout(
        Duration::from_secs(15),
        sender.send_request(Request::from_parts(parts, Full::new(Bytes::new()))),
    )
    .await??;
    validate_terminal_upgrade(&response, &expected_accept)?;
    let upgraded =
        tokio::time::timeout(Duration::from_secs(5), hyper::upgrade::on(&mut response)).await??;
    let config = WebSocketConfig::default()
        .max_message_size(Some(16 * 1024))
        .max_frame_size(Some(16 * 1024));
    Ok(WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Client, Some(config)).await)
}

#[cfg(unix)]
fn validate_terminal_upgrade(
    response: &hyper::Response<hyper::body::Incoming>,
    expected_accept: &str,
) -> Result<()> {
    use hyper::StatusCode;
    ensure!(
        response.status() == StatusCode::SWITCHING_PROTOCOLS,
        "Daemon refused terminal websocket upgrade"
    );
    ensure!(
        response
            .headers()
            .get("Upgrade")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("websocket")),
        "Invalid terminal websocket upgrade protocol"
    );
    ensure!(
        response
            .headers()
            .get("Connection")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))),
        "Invalid terminal websocket connection upgrade"
    );
    ensure!(
        response
            .headers()
            .get("Sec-WebSocket-Accept")
            .and_then(|v| v.to_str().ok())
            == Some(expected_accept),
        "Invalid terminal websocket acceptance key"
    );
    ensure!(
        !response.headers().contains_key("Sec-WebSocket-Extensions")
            && !response.headers().contains_key("Sec-WebSocket-Protocol"),
        "Daemon selected an unsolicited websocket extension or protocol"
    );
    Ok(())
}

async fn read_approval_secret(prompt: &'static str, from_stdin: bool) -> Result<Zeroizing<String>> {
    let secret = read_secret(prompt, from_stdin).await?;
    validate_approval_secret(&secret)?;
    Ok(secret)
}
fn validate_approval_secret(secret: &str) -> Result<()> {
    ensure!(
        (32..=4096).contains(&secret.len())
            && secret.bytes().all(|byte| (0x21..=0x7e).contains(&byte)),
        "Crew approval secrets must contain 32–4096 printable ASCII characters without spaces; vault passphrases may contain Unicode"
    );
    Ok(())
}

async fn read_secret(prompt: &'static str, from_stdin: bool) -> Result<Zeroizing<String>> {
    tokio::task::spawn_blocking(move || {
        let secret = if from_stdin {
            let mut bytes = Zeroizing::new(Vec::new());
            let mut stdin = std::io::stdin().lock();
            for _ in 0..4097 {
                let mut byte = [0u8];
                if stdin.read(&mut byte)? == 0 || byte[0] == b'\n' { break; }
                bytes.push(byte[0]);
            }
            ensure!(bytes.len() <= 4096, "Secret exceeds 4096 bytes");
            if bytes.last() == Some(&b'\r') { bytes.pop(); }
            Zeroizing::new(std::str::from_utf8(&bytes)?.to_owned())
        } else {
            ensure!(std::io::stdin().is_terminal() && std::io::stderr().is_terminal(), "An interactive no-echo prompt is required; select --approval-key-stdin explicitly to use a secret input pipe");
            eprintln!("{prompt}");
            Zeroizing::new(console::Term::stderr().read_secure_line()?)
        };
        ensure!(!secret.is_empty() && secret.len() <= 4096, "A nonempty secret of at most 4096 bytes is required");
        Ok(secret)
    }).await?
}

#[cfg(unix)]
async fn checked_socket(descriptor: &Descriptor) -> Result<tokio::net::UnixStream> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    let current = daemon_runtime::read_descriptor()?;
    ensure!(
        current.identity() == descriptor.identity(),
        "Daemon instance changed; reconnect and authorize the current instance"
    );
    let daemon_runtime::Endpoint::Unix { path } = &descriptor.endpoint;
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.file_type().is_socket()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.permissions().mode() & 0o077 == 0,
        "Daemon socket is not private to this user"
    );
    let stream = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::UnixStream::connect(path),
    )
    .await??;
    ensure!(
        stream.peer_cred()?.uid() == unsafe { libc::geteuid() },
        "Daemon socket peer belongs to another user"
    );
    Ok(stream)
}

async fn discover_shared_daemon() -> Result<Option<Descriptor>> {
    match std::fs::symlink_metadata(daemon_runtime::descriptor_path()) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .context("Cannot inspect shared daemon discovery; refusing replacement");
        }
        Ok(_) => (),
    }
    let descriptor = daemon_runtime::read_descriptor()
        .context("Invalid or inaccessible daemon discovery; refusing replacement")?;
    match verify_identity(&descriptor).await {
        Ok(()) => Ok(Some(descriptor)),
        Err(error)
            if error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                )
            }) =>
        {
            // Only native missing/refused sockets mark a validated descriptor
            // stale. The new daemon acquires owner.lock before replacing it.
            Ok(None)
        }
        Err(error) => {
            Err(error).context("Daemon identity could not be verified; refusing replacement")
        }
    }
}

async fn verify_identity(descriptor: &Descriptor) -> Result<()> {
    let value = request(descriptor, None, "GET", "/daemon/identity", None).await?;
    let identity: Identity = serde_json::from_value(value)?;
    ensure!(
        identity == descriptor.identity(),
        "Daemon identity does not match this profile's discovery record"
    );
    Ok(())
}

async fn request(
    descriptor: &Descriptor,
    proof: Option<&str>,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    #[cfg(not(unix))]
    {
        let _ = (descriptor, proof, method, path, body);
        bail!("Shared Crew daemon IPC is unavailable on this platform");
    }
    #[cfg(unix)]
    {
        use bytes::Bytes;
        use http_body_util::{BodyExt, Full, Limited};
        use hyper::{client::conn::http1, Request};
        use hyper_util::rt::TokioIo;
        ensure!(
            path.starts_with('/') && !path.starts_with("//") && !path.contains(['\r', '\n']),
            "Invalid daemon request path"
        );
        let stream = checked_socket(descriptor).await?;
        let (mut sender, connection) = http1::handshake(TokioIo::new(stream)).await?;
        let connection_task = tokio::spawn(connection);
        struct ConnectionTask(tokio::task::JoinHandle<Result<(), hyper::Error>>);
        impl Drop for ConnectionTask {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _connection = ConnectionTask(connection_task);
        let identity_request = Request::builder()
            .method("GET")
            .uri("/daemon/identity")
            .header("Host", "localhost")
            .header("X-Secret-Key", &descriptor.api_secret)
            .body(Full::new(Bytes::new()))?;
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            sender.send_request(identity_request),
        )
        .await??;
        ensure!(
            response.status().is_success(),
            "Shared daemon identity was refused"
        );
        let bytes = tokio::time::timeout(
            Duration::from_secs(5),
            Limited::new(response.into_body(), 16 * 1024).collect(),
        )
        .await?
        .map_err(|error| anyhow::anyhow!(error.to_string()))?
        .to_bytes();
        let identity: Identity = serde_json::from_slice(&bytes)?;
        ensure!(
            identity == descriptor.identity(),
            "Shared daemon identity changed; no approval credentials were sent"
        );
        if path == "/daemon/identity" && method == "GET" {
            return Ok(serde_json::to_value(identity)?);
        }
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("Host", "localhost")
            .header("X-Secret-Key", &descriptor.api_secret)
            .header("X-Daemon-Instance", &descriptor.instance_id);
        if body.is_some() {
            builder = builder.header("Content-Type", "application/json");
        }
        if let Some(proof) = proof {
            builder = builder.header("X-User-Action", proof);
        }
        let bytes = body
            .map(|body| serde_json::to_vec(&body))
            .transpose()?
            .unwrap_or_default();
        let response = tokio::time::timeout(
            Duration::from_secs(180),
            sender.send_request(builder.body(Full::new(Bytes::from(bytes)))?),
        )
        .await??;
        let status = response.status();
        let bytes = bounded_response(response, 16 * 1024 * 1024).await?;
        let value: Value = if bytes.is_empty() {
            json!({})
        } else {
            serde_json::from_slice(&bytes).context("Daemon returned a non-JSON response")?
        };
        if !status.is_success() {
            return Err(daemon_refusal(
                status.as_u16(),
                Some(&value),
                "Request refused; check the shared daemon and approval secret",
            )
            .into());
        }
        Ok(value)
    }
}

async fn start_daemon(proof: &Zeroizing<String>) -> Result<Descriptor> {
    #[cfg(not(unix))]
    {
        let _ = proof;
        bail!("Shared Crew daemon IPC is unavailable on this platform");
    }
    #[cfg(unix)]
    {
        use sha2::{Digest, Sha256};
        use std::process::Stdio;
        use tokio::io::AsyncWriteExt;
        validate_approval_secret(proof)?;
        ensure!(
            discover_shared_daemon().await?.is_none(),
            "A shared daemon already owns this profile; connect to it using its existing approval secret"
        );
        let binary = std::env::current_exe()?
            .parent()
            .context("CLI executable has no directory")?
            .join("biorouterd");
        ensure!(
            binary.is_file(),
            "biorouterd must be installed alongside biorouter for shared Crew startup"
        );
        daemon_runtime::private_directory(&daemon_runtime::runtime_directory())?;
        let mut command = tokio::process::Command::new(binary);
        command
            .arg("agent")
            .env(daemon_runtime::SHARED_ENV, "1")
            .env("BIOROUTER_USER_ACTION_EXPECTED", "1")
            .env("BIOROUTER_HOST", "127.0.0.1")
            .env("BIOROUTER_PORT", "0")
            .env(
                "BIOROUTER_SERVER__SECRET_KEY",
                uuid::Uuid::new_v4().simple().to_string(),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
        let mut child = command.spawn()?;
        let child_pid = child.id().context("New daemon has no process ID")?;
        let mut pipe = child
            .stdin
            .take()
            .context("New daemon has no startup pipe")?;
        let digest: String = Sha256::digest(proof.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        if let Err(error) = pipe.write_all(format!("{digest}\n").as_bytes()).await {
            child.kill().await.ok();
            child.wait().await.ok();
            return Err(error.into());
        }
        drop(pipe);
        for _ in 0..300 {
            if let Some(status) = child.try_wait()? {
                bail!(
                    "Shared daemon exited during startup ({status}); inspect the profile and installed daemon"
                );
            }
            if let Ok(descriptor) = daemon_runtime::read_descriptor() {
                if descriptor.pid == child_pid
                    && descriptor.user_action_installed
                    && verify_identity(&descriptor).await.is_ok()
                {
                    if let Err(error) = request(
                        &descriptor,
                        Some(proof.as_str()),
                        "GET",
                        "/crew/connections",
                        None,
                    )
                    .await
                    {
                        child.kill().await.ok();
                        child.wait().await.ok();
                        return Err(error);
                    }
                    return Ok(descriptor);
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        child.kill().await.ok();
        child.wait().await.ok();
        bail!(
            "The daemon did not establish authenticated shared readiness; only the newly started child was stopped"
        )
    }
}

#[cfg(unix)]
fn open_daemon_owner_lock() -> Result<std::fs::File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    daemon_runtime::private_directory(&daemon_runtime::runtime_directory())?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(daemon_runtime::runtime_directory().join("owner.lock"))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.nlink() == 1,
        "Daemon owner lock must be a private regular file owned by this user"
    );
    Ok(file)
}

async fn wait_for_daemon_stop(expected: &Descriptor) -> Result<Value> {
    #[cfg(not(unix))]
    {
        let _ = expected;
        bail!("Shared daemon shutdown verification is unavailable on this platform");
    }
    #[cfg(unix)]
    {
        use std::os::{fd::AsRawFd, unix::fs::MetadataExt};
        let owner = open_daemon_owner_lock()
            .context("Stop accepted, but daemon ownership could not be inspected")?;
        let original = owner.metadata()?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let current =
                std::fs::symlink_metadata(daemon_runtime::runtime_directory().join("owner.lock"))?;
            ensure!(
                current.dev() == original.dev() && current.ino() == original.ino(),
                "Stop accepted, but daemon owner lock changed; shutdown is unconfirmed"
            );
            if unsafe { libc::flock(owner.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                // Dropping this descriptor releases the probe lock before a subsequent start.
                return Ok(json!({"stopped":true,"instance_id":expected.instance_id}));
            }
            let error = std::io::Error::last_os_error();
            ensure!(
                error.kind() == std::io::ErrorKind::WouldBlock,
                "Stop accepted, but daemon owner lock could not be checked: {error}"
            );
            match daemon_runtime::read_descriptor() {
                Ok(current) => {
                    if current.identity() != expected.identity() {
                        tokio::time::timeout_at(deadline, verify_identity(&current))
                            .await
                            .context(
                                "Stop accepted, but replacement identity verification timed out",
                            )?
                            .context("Stop accepted, but replacement identity is unverified")?;
                        return Ok(json!({"stopped":true,"instance_id":expected.instance_id,
                            "replacement_instance_id":current.instance_id}));
                    }
                }
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) => {}
                Err(error) => {
                    return Err(error)
                        .context("Stop accepted, but current daemon discovery is invalid");
                }
            }
            ensure!(
                tokio::time::Instant::now() < deadline,
                "Stop accepted, but the daemon still owns this profile after 30 seconds; shutdown is unconfirmed. Check daemon status before restarting"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

pub async fn daemon_control(action: &str, approval_key_stdin: bool) -> Result<Value> {
    match action {
        "status" => {
            let descriptor = daemon_runtime::read_descriptor()?;
            verify_identity(&descriptor).await?;
            Ok(serde_json::to_value(descriptor.identity())?)
        }
        "start" => {
            let proof = read_secret(
                "Create/use your separately held Crew approval secret (at least 32 bytes):",
                approval_key_stdin,
            )
            .await?;
            let descriptor = start_daemon(&proof).await?;
            Ok(serde_json::to_value(descriptor.identity())?)
        }
        "stop" => {
            let client = CrewClient::connect_with_input(true, approval_key_stdin).await?;
            client
                .request(
                    "POST",
                    "/daemon/stop",
                    Some(json!({"instance_id":client.descriptor.instance_id})),
                )
                .await?;
            wait_for_daemon_stop(&client.descriptor).await
        }
        _ => bail!("Unknown daemon action"),
    }
}

pub async fn credentials_control(action: &str, approval_key_stdin: bool) -> Result<Value> {
    let client = CrewClient::connect_with_input(true, approval_key_stdin).await?;
    match action {
        "status" => client.request("GET", "/crew/credentials", None).await,
        "lock" => client.request("POST", "/crew/credentials/lock", None).await,
        "init" | "unlock" => {
            let passphrase = read_secret(
                "Vault passphrase (different from the Crew approval secret):",
                approval_key_stdin,
            )
            .await?;
            ensure!(
                passphrase.as_str() != client.proof.as_str(),
                "The vault passphrase must differ from the human approval secret"
            );
            client
                .request(
                    "POST",
                    &format!("/crew/credentials/{action}"),
                    Some(json!({"passphrase":passphrase.as_str()})),
                )
                .await
        }
        _ => bail!("Unknown credential action"),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{
        daemon_refusal, open_daemon_owner_lock, read_observer_frames, wait_for_daemon_stop,
        CrewClient, DaemonRefusal, EventDecoder, MAX_SSE_FRAME,
    };
    use biorouter::crew::observation::ObserveEvent;
    use biorouter::daemon_runtime::{self, Descriptor, Endpoint, Identity};
    use bytes::Bytes;
    use http_body_util::{BodyExt, Full, StreamBody};
    use hyper::server::conn::http1 as server_http1;
    use hyper::{body::Frame, client::conn::http1, service::service_fn, Request, Response};
    use hyper_util::rt::TokioIo;
    use serde_json::Value;
    use serial_test::serial;
    use std::fs::{self, File};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn runtime_dir() -> PathBuf {
        let directory = daemon_runtime::runtime_directory();
        daemon_runtime::private_directory(&directory).expect("test runtime directory is private");
        let _ = fs::remove_file(directory.join("owner.lock"));
        let _ = fs::remove_file(directory.join("owner.lock.old"));
        let _ = fs::remove_file(daemon_runtime::descriptor_path());
        directory
    }

    fn expected_descriptor(directory: &Path) -> Descriptor {
        Descriptor {
            version: daemon_runtime::VERSION,
            profile_id: daemon_runtime::profile_identity()
                .expect("synthetic test profile identity"),
            instance_id: "22222222-2222-4222-8222-222222222222".to_owned(),
            pid: std::process::id(),
            endpoint: Endpoint::Unix {
                path: directory.join("daemon.sock"),
            },
            api_secret: "daemon-secret-for-owner-lock-tests-123".to_owned(),
            user_action_installed: true,
        }
    }

    async fn daemon_http_fixture(
        body: Bytes,
        content_type: &'static str,
        identity: Identity,
        response_status: hyper::StatusCode,
    ) -> (Descriptor, Arc<Mutex<Vec<Option<String>>>>) {
        let directory = runtime_dir();
        let socket_path = directory.join("daemon.sock");
        let _ = fs::remove_file(&socket_path);
        let listener = tokio::net::UnixListener::bind(&socket_path).expect("fixture socket binds");
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))
            .expect("fixture socket is private");
        let mut descriptor = expected_descriptor(&directory);
        descriptor.endpoint = Endpoint::Unix { path: socket_path };
        daemon_runtime::write_private(&daemon_runtime::descriptor_path(), &descriptor)
            .expect("fixture descriptor writes");
        let proofs = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&proofs);
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("fixture accepts connection");
            let service = service_fn(move |request: Request<hyper::body::Incoming>| {
                let captured = Arc::clone(&captured);
                let body = body.clone();
                let identity = identity.clone();
                async move {
                    captured.lock().expect("fixture capture lock").push(
                        request
                            .headers()
                            .get("X-User-Action")
                            .and_then(|v| v.to_str().ok().map(str::to_owned)),
                    );
                    if request.uri().path() == "/daemon/identity" {
                        Ok::<_, std::convert::Infallible>(
                            Response::builder()
                                .header("content-type", "application/json")
                                .body(Full::new(Bytes::from(
                                    serde_json::to_vec(&identity).expect("identity serializes"),
                                )))
                                .expect("identity response builds"),
                        )
                    } else {
                        Ok::<_, std::convert::Infallible>(
                            Response::builder()
                                .status(response_status)
                                .header("content-type", content_type)
                                .body(Full::new(body))
                                .expect("fixture response builds"),
                        )
                    }
                }
            });
            let _ = server_http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
        (descriptor, proofs)
    }

    #[derive(Debug, Clone)]
    struct CapturedRequest {
        method: String,
        content_type: Option<String>,
        body: Bytes,
    }

    async fn request_header_fixture() -> (Descriptor, Arc<Mutex<Vec<CapturedRequest>>>) {
        let directory = runtime_dir();
        let socket_path = directory.join("daemon.sock");
        let _ = fs::remove_file(&socket_path);
        let listener = tokio::net::UnixListener::bind(&socket_path).expect("fixture socket binds");
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))
            .expect("fixture socket is private");
        let mut descriptor = expected_descriptor(&directory);
        descriptor.endpoint = Endpoint::Unix { path: socket_path };
        daemon_runtime::write_private(&daemon_runtime::descriptor_path(), &descriptor)
            .expect("fixture descriptor writes");
        let identity = descriptor.identity();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let requests = Arc::clone(&captured);
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("fixture accepts connection");
            let service = service_fn(move |request: Request<hyper::body::Incoming>| {
                let requests = Arc::clone(&requests);
                let identity = identity.clone();
                async move {
                    if request.uri().path() == "/daemon/identity" {
                        return Ok::<_, std::convert::Infallible>(
                            Response::builder()
                                .header("content-type", "application/json")
                                .body(Full::new(Bytes::from(
                                    serde_json::to_vec(&identity).expect("identity serializes"),
                                )))
                                .expect("identity response builds"),
                        );
                    }
                    let (parts, body) = request.into_parts();
                    let body = body.collect().await.expect("request body reads").to_bytes();
                    requests
                        .lock()
                        .expect("request capture lock")
                        .push(CapturedRequest {
                            method: parts.method.to_string(),
                            content_type: parts
                                .headers
                                .get("content-type")
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned),
                            body,
                        });
                    Ok::<_, std::convert::Infallible>(
                        Response::builder()
                            .header("content-type", "application/json")
                            .body(Full::new(Bytes::from_static(br#"{}"#)))
                            .expect("fixture response builds"),
                    )
                }
            });
            server_http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await
                .expect("fixture serves request");
        });
        (descriptor, captured)
    }

    fn create_owner_lock(directory: &Path) {
        fs::write(directory.join("owner.lock"), b"lock").expect("lock writes");
        fs::set_permissions(
            directory.join("owner.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .expect("lock mode sets");
    }

    fn hold_lock(file: &File) {
        assert_eq!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
    }

    #[tokio::test]
    #[serial]
    async fn stop_waits_for_the_current_owner_to_release_then_returns_verified_stop() {
        let directory = runtime_dir();
        create_owner_lock(&directory);
        let owner = open_daemon_owner_lock().expect("owner lock opens");
        hold_lock(&owner);
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(owner);
        });

        let result = wait_for_daemon_stop(&expected_descriptor(&directory))
            .await
            .expect("owner release confirms stop");
        release.join().expect("release thread exits");
        assert!(result["stopped"].as_bool().unwrap_or(false));
        assert_eq!(
            result["instance_id"],
            "22222222-2222-4222-8222-222222222222"
        );
    }

    #[tokio::test]
    #[serial]
    async fn stop_does_not_confirm_when_descriptor_disappears_before_lock_release() {
        let directory = runtime_dir();
        create_owner_lock(&directory);
        let owner = open_daemon_owner_lock().expect("owner lock opens");
        hold_lock(&owner);
        let expected = expected_descriptor(&directory);
        let mut waiter = Box::pin(wait_for_daemon_stop(&expected));

        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut waiter)
                .await
                .is_err(),
            "a missing descriptor cannot prove shutdown while the owner lock is held"
        );

        drop(owner);
        let result = waiter
            .await
            .expect("releasing the owner lock confirms stop");
        assert!(result["stopped"].as_bool().unwrap_or(false));
        assert_eq!(
            result["instance_id"],
            "22222222-2222-4222-8222-222222222222"
        );
    }

    #[tokio::test]
    #[serial]
    async fn stop_refuses_malformed_and_insecure_existing_descriptors() {
        let directory = runtime_dir();
        create_owner_lock(&directory);
        let owner = open_daemon_owner_lock().expect("owner lock opens");
        hold_lock(&owner);
        let descriptor_path = daemon_runtime::descriptor_path();

        fs::write(&descriptor_path, b"{not-json}").expect("malformed descriptor writes");
        fs::set_permissions(&descriptor_path, fs::Permissions::from_mode(0o600))
            .expect("malformed descriptor mode sets");
        let expected = expected_descriptor(&directory);
        let error = wait_for_daemon_stop(&expected)
            .await
            .expect_err("malformed descriptor must refuse shutdown confirmation");
        assert!(
            format!("{error:#}").contains("current daemon discovery is invalid"),
            "unexpected malformed descriptor refusal: {error:#}"
        );

        fs::write(
            &descriptor_path,
            serde_json::to_vec(&expected).expect("descriptor serializes"),
        )
        .expect("insecure descriptor writes");
        fs::set_permissions(&descriptor_path, fs::Permissions::from_mode(0o644))
            .expect("insecure descriptor mode sets");
        let error = wait_for_daemon_stop(&expected)
            .await
            .expect_err("insecure descriptor must refuse shutdown confirmation");
        assert!(
            format!("{error:#}").contains("current daemon discovery is invalid"),
            "unexpected insecure descriptor refusal: {error:#}"
        );

        drop(owner);
        fs::remove_file(descriptor_path).expect("descriptor removes");
    }

    #[test]
    #[serial]
    fn owner_lock_rejects_symlink_and_insecure_mode() {
        let directory = runtime_dir();
        let target = directory.join("owner-target");
        fs::write(&target, b"lock").expect("target writes");
        std::os::unix::fs::symlink(&target, directory.join("owner.lock")).expect("symlink creates");
        assert!(open_daemon_owner_lock().is_err());
        fs::remove_file(directory.join("owner.lock")).expect("symlink removes");
        fs::write(directory.join("owner.lock"), b"lock").expect("lock writes");
        fs::set_permissions(
            directory.join("owner.lock"),
            fs::Permissions::from_mode(0o644),
        )
        .expect("insecure mode sets");
        assert!(open_daemon_owner_lock().is_err());
        fs::remove_file(target).expect("target removes");
    }

    #[test]
    #[serial]
    fn owner_lock_rejects_a_displaced_hardlink_inode() {
        let directory = runtime_dir();
        let target = directory.join("owner-target");
        fs::write(&target, b"lock").expect("target writes");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).expect("target mode sets");
        fs::hard_link(&target, directory.join("owner.lock")).expect("hardlink creates");
        assert!(open_daemon_owner_lock().is_err());
        fs::remove_file(directory.join("owner.lock")).expect("hardlink removes");
        fs::remove_file(target).expect("target removes");
    }

    async fn observer_body(chunks: Vec<Vec<u8>>) -> hyper::body::Incoming {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("observer fixture listener");
        let address = listener.local_addr().expect("observer fixture address");
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("observer fixture accept");
            let service = service_fn(move |_request: Request<hyper::body::Incoming>| {
                let chunks = chunks.clone();
                async move {
                    let frames = futures::stream::iter(chunks.into_iter().map(|chunk| {
                        Ok::<_, std::convert::Infallible>(Frame::data(Bytes::from(chunk)))
                    }));
                    let body = StreamBody::new(frames);
                    Ok::<_, std::convert::Infallible>(Response::new(body))
                }
            });
            server_http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await
                .expect("observer fixture response");
        });
        let stream = tokio::net::TcpStream::connect(address)
            .await
            .expect("observer fixture connect");
        let (mut sender, connection) = http1::handshake(TokioIo::new(stream))
            .await
            .expect("observer fixture handshake");
        tokio::spawn(async move {
            connection
                .await
                .expect("observer fixture client connection");
        });
        sender
            .send_request(
                Request::builder()
                    .uri("/")
                    .body(Full::new(Bytes::new()))
                    .expect("observer fixture request"),
            )
            .await
            .expect("observer fixture response headers")
            .into_body()
    }

    #[tokio::test]
    async fn observer_parser_handles_split_frames_and_stops_on_reconnect() {
        let body = observer_body(vec![
            br#"{"type":"state""#.to_vec(),
            br#","connection_id":"synthetic-connection","connection_mode":"private","connection_policy_epoch":7,"connection_institution_id":"ucsf","snapshot":{},"runs":[]}
{"type":"reconnect","cursor":"cursor-2"}
"#
            .to_vec(),
        ])
        .await;
        let mut frames = Vec::new();
        let cursor = read_observer_frames(body, |frame| {
            let reconnect = matches!(&frame, ObserveEvent::Reconnect { .. });
            frames.push(frame);
            if reconnect {
                Ok(std::ops::ControlFlow::Break(Some("cursor-2".into())))
            } else {
                Ok(std::ops::ControlFlow::Continue(()))
            }
        })
        .await
        .expect("split NDJSON frames parse");
        assert_eq!(cursor.as_deref(), Some("cursor-2"));
        assert_eq!(frames.len(), 2);
        match &frames[0] {
            ObserveEvent::State {
                connection_policy_epoch,
                connection_institution_id,
                ..
            } => {
                assert_eq!(*connection_policy_epoch, 7);
                assert_eq!(connection_institution_id.as_deref(), Some("ucsf"));
            }
            _ => panic!("expected state frame"),
        }
    }

    #[tokio::test]
    async fn observer_parser_rejects_malformed_and_oversized_framing() {
        let malformed = observer_body(vec![b"{not-json}\n".to_vec()]).await;
        let error = read_observer_frames(malformed, |_| Ok(std::ops::ControlFlow::Continue(())))
            .await
            .expect_err("malformed observer JSON must fail closed");
        assert!(error.to_string().contains("Invalid Crew observer frame"));

        let mut oversized = vec![b'x'; 1_048_577];
        oversized.push(b'\n');
        let oversized = observer_body(vec![oversized]).await;
        let error = read_observer_frames(oversized, |_| Ok(std::ops::ControlFlow::Continue(())))
            .await
            .expect_err("oversized observer frame must fail closed");
        assert!(error.to_string().contains("exceeds 1 MiB"));
    }

    #[tokio::test]
    #[serial]
    async fn identity_mismatch_refuses_before_sending_human_proof() {
        let expected = expected_descriptor(&runtime_dir());
        let mut mismatched = expected.identity();
        mismatched.instance_id = "33333333-3333-4333-8333-333333333333".into();
        let (descriptor, proofs) = daemon_http_fixture(
            Bytes::from_static(b"must not reach handler"),
            "text/plain",
            mismatched,
            hyper::StatusCode::OK,
        )
        .await;
        let client = CrewClient {
            descriptor,
            proof: zeroize::Zeroizing::new("synthetic-human-proof-01234567890123456789".into()),
        };
        let error = client
            .request_text(
                "POST",
                "/crew/conversations",
                Some(serde_json::json!({"prompt":"x"})),
            )
            .await
            .expect_err("mismatched daemon identity must refuse");
        assert!(error.to_string().contains("identity changed"));
        assert_eq!(
            proofs.lock().expect("fixture capture lock").as_slice(),
            &[None],
            "the identity probe carries no human proof and the authenticated request is never sent"
        );
    }

    #[tokio::test]
    #[serial]
    async fn request_text_returns_plain_text_and_empty_successes() {
        for body in [Bytes::from_static(b"daemon reply"), Bytes::new()] {
            let descriptor = expected_descriptor(&runtime_dir());
            let (descriptor, proofs) = daemon_http_fixture(
                body.clone(),
                "text/plain",
                descriptor.identity(),
                hyper::StatusCode::OK,
            )
            .await;
            let client = CrewClient {
                descriptor,
                proof: zeroize::Zeroizing::new("synthetic-human-proof-01234567890123456789".into()),
            };
            let response = client
                .request_text("GET", "/crew/conversations/session-1", None)
                .await
                .expect("plain-text response succeeds");
            assert_eq!(response.as_bytes(), body.as_ref());
            assert_eq!(
                proofs.lock().expect("fixture capture lock").as_slice(),
                &[
                    None,
                    Some("synthetic-human-proof-01234567890123456789".into())
                ]
            );
        }
    }

    #[tokio::test]
    #[serial]
    async fn authenticated_requests_only_mark_json_when_a_body_is_present() {
        for (body, expected_wire_body, expected_content_type) in [
            (None, Bytes::new(), None),
            (
                Some(serde_json::json!({"file_capability":"capability-1"})),
                Bytes::from_static(br#"{"file_capability":"capability-1"}"#),
                Some("application/json"),
            ),
            (
                Some(Value::Null),
                Bytes::from_static(b"null"),
                Some("application/json"),
            ),
        ] {
            let (descriptor, captured) = request_header_fixture().await;
            let client = CrewClient {
                descriptor,
                proof: zeroize::Zeroizing::new("synthetic-human-proof-01234567890123456789".into()),
            };
            client
                .request("DELETE", "/crew/transfers/transfer-1", body)
                .await
                .expect("synthetic cleanup request succeeds");
            let requests = captured.lock().expect("request capture lock");
            let request = requests.first().expect("authenticated request captured");
            assert_eq!(request.method, "DELETE");
            assert_eq!(request.content_type.as_deref(), expected_content_type);
            assert_eq!(request.body, expected_wire_body);
        }
    }

    #[tokio::test]
    #[serial]
    async fn request_text_refuses_a_body_over_the_response_bound() {
        let body = Bytes::from(vec![b'x'; 16 * 1024 * 1024 + 1]);
        let expected = expected_descriptor(&runtime_dir());
        let (descriptor, _) = daemon_http_fixture(
            body,
            "text/plain",
            expected.identity(),
            hyper::StatusCode::OK,
        )
        .await;
        let client = CrewClient {
            descriptor,
            proof: zeroize::Zeroizing::new("synthetic-human-proof-01234567890123456789".into()),
        };
        let error = client
            .request_text("GET", "/crew/conversations/session-1", None)
            .await
            .expect_err("oversized response must be refused");
        assert!(error.to_string().contains("exceeded its size limit"));
    }

    #[tokio::test]
    #[serial]
    async fn typed_daemon_http_refusals_preserve_status_and_kind() {
        let expected = expected_descriptor(&runtime_dir());
        let (descriptor, _) = daemon_http_fixture(
            Bytes::from_static(br#"{"status":"unknown","error":"expired"}"#),
            "application/json",
            expected.identity(),
            hyper::StatusCode::CONFLICT,
        )
        .await;
        let client = CrewClient {
            descriptor,
            proof: zeroize::Zeroizing::new("synthetic-human-proof-01234567890123456789".into()),
        };
        let error = client
            .request("POST", "/action-required/elicitation", None)
            .await
            .expect_err("typed daemon refusal must be returned");
        let refusal = error
            .downcast_ref::<DaemonRefusal>()
            .expect("HTTP refusal must retain its typed error");
        assert_eq!(refusal.status, 409);
        assert_eq!(refusal.kind.as_deref(), Some("unknown"));
    }

    const INSTITUTION_REFUSAL: &str = "Crew institution does not match the model's resolved affiliation; choose a local model or a model approved for this institution";

    #[test]
    fn daemon_refusals_keep_the_daemons_exact_sentence() {
        let refusal = daemon_refusal(
            400,
            Some(&serde_json::json!({ "error": INSTITUTION_REFUSAL })),
            "fallback",
        );
        assert_eq!(
            refusal.to_string(),
            format!("Daemon returned 400: {INSTITUTION_REFUSAL}")
        );
        let quoted = daemon_refusal(
            409,
            Some(&serde_json::json!({ "message": r#"Say "stop" \ then retry"# })),
            "fallback",
        );
        assert_eq!(
            quoted.to_string(),
            r#"Daemon returned 409: Say "stop" \ then retry"#
        );
    }

    #[test]
    fn daemon_refusals_still_escape_terminal_controls_and_invisible_formatting() {
        let hostile = "red\u{1b}[31m\nnext\u{202e}flip\u{2068}iso\u{200b}zero";
        let refusal = daemon_refusal(
            400,
            Some(&serde_json::json!({ "error": hostile })),
            "fallback",
        )
        .to_string();
        for raw in ['\u{1b}', '\n', '\u{202e}', '\u{2068}', '\u{200b}'] {
            assert!(!refusal.contains(raw), "raw {raw:?} reached the terminal");
        }
        for escaped in ["\\u{1b}[31m", "\\n", "\\u{202e}", "\\u{2068}", "\\u{200b}"] {
            assert!(
                refusal.contains(escaped),
                "{escaped} missing from {refusal}"
            );
        }
        let fallback = daemon_refusal(502, None, "Gateway's answer was not JSON").to_string();
        assert_eq!(
            fallback,
            "Daemon returned 502: Gateway's answer was not JSON"
        );
    }

    /// `broker/join.rs`'s literal `already_approved` refusal.
    const ALREADY_APPROVED: &str = "already_approved: You already let a device in for @eve. Replace the code only if they sent you a new one.";

    #[test]
    fn a_broker_refusal_carries_the_brokers_code_and_its_own_text() {
        let refusal = daemon_refusal(
            400,
            Some(&serde_json::json!({
                "code": "crew_request_refused",
                "broker_code": "already_approved",
                "error": ALREADY_APPROVED,
            })),
            "fallback",
        );
        assert_eq!(refusal.kind.as_deref(), Some("crew_request_refused"));
        assert_eq!(refusal.broker_code.as_deref(), Some("already_approved"));
        assert_eq!(refusal.message(), ALREADY_APPROVED);

        let plain = daemon_refusal(
            400,
            Some(&serde_json::json!({ "code": "crew_request_refused", "error": "x" })),
            "fallback",
        );
        assert_eq!(plain.broker_code, None);
    }

    #[test]
    fn an_older_daemons_envelope_is_unwrapped_to_the_brokers_text() {
        let envelope = format!(
            "Crew broker refused request: {}",
            serde_json::json!({"code": "already_approved", "message": ALREADY_APPROVED})
        );
        let refusal = daemon_refusal(
            400,
            Some(&serde_json::json!({ "code": "crew_request_refused", "error": envelope })),
            "fallback",
        );
        assert_eq!(refusal.broker_code.as_deref(), Some("already_approved"));
        assert_eq!(refusal.message(), ALREADY_APPROVED);
        assert!(!refusal.to_string().contains("Crew broker refused request"));
        assert!(!refusal.to_string().contains('{'));

        let code_only = daemon_refusal(
            400,
            Some(&serde_json::json!({
                "error": "Crew broker refused request: {\"code\":\"stale_cursor\"}"
            })),
            "fallback",
        );
        assert_eq!(code_only.broker_code.as_deref(), Some("stale_cursor"));
        assert_eq!(code_only.message(), "stale_cursor");

        for unreadable in [
            "Crew broker refused request: not json",
            "Crew broker refused request: {\"message\":\"no code\"}",
            "Crew broker refused request: {\"code\":\"Not A Code\",\"message\":\"x\"}",
        ] {
            let refusal = daemon_refusal(
                400,
                Some(&serde_json::json!({ "error": unreadable })),
                "fallback",
            );
            assert_eq!(refusal.broker_code, None, "{unreadable}");
            assert_eq!(refusal.message(), unreadable);
        }
    }

    #[test]
    fn a_broker_code_is_only_ever_a_code() {
        let hostile = daemon_refusal(
            400,
            Some(&serde_json::json!({
                "broker_code": "name_taken\u{1b}[31m",
                "error": "name_taken: x",
            })),
            "fallback",
        );
        assert_eq!(hostile.broker_code, None);
        let long = "a".repeat(300);
        let capped = daemon_refusal(
            400,
            Some(&serde_json::json!({ "broker_code": long, "error": "x" })),
            "fallback",
        );
        assert_eq!(capped.broker_code.map(|code| code.len()), Some(128));
    }

    #[test]
    fn a_broker_refusal_keeps_its_apostrophes_and_still_escapes_controls() {
        let text = "code_mismatch: The host hasn't let this device in.\u{1b}[31m";
        let envelope = format!(
            "Crew broker refused request: {}",
            serde_json::json!({"code": "code_mismatch", "message": text})
        );
        for body in [
            serde_json::json!({ "broker_code": "code_mismatch", "error": text }),
            serde_json::json!({ "error": envelope }),
        ] {
            let refusal = daemon_refusal(400, Some(&body), "fallback");
            assert!(
                refusal.message().contains("hasn't"),
                "{}",
                refusal.message()
            );
            assert!(!refusal.message().contains("hasn\\'t"));
            assert!(!refusal.message().contains('\u{1b}'));
            assert!(refusal.message().ends_with("in.\\u{1b}[31m"));
        }
    }

    #[tokio::test]
    #[serial]
    async fn daemon_http_refusal_text_carries_the_apostrophe_unescaped() {
        let expected = expected_descriptor(&runtime_dir());
        let body = serde_json::to_vec(&serde_json::json!({
            "status": "crew_institution_mismatch",
            "error": INSTITUTION_REFUSAL,
        }))
        .expect("refusal serializes");
        let (descriptor, _) = daemon_http_fixture(
            Bytes::from(body),
            "application/json",
            expected.identity(),
            hyper::StatusCode::BAD_REQUEST,
        )
        .await;
        let client = CrewClient {
            descriptor,
            proof: zeroize::Zeroizing::new("synthetic-human-proof-01234567890123456789".into()),
        };
        let error = client
            .request("POST", "/crew/connections/c/runs", None)
            .await
            .expect_err("the daemon refused");
        let text = format!("{error:#}");
        assert!(text.contains("the model's resolved affiliation"), "{text}");
        assert!(!text.contains("model\\'s"), "{text}");
    }

    fn decode_sse(input: &[u8]) -> anyhow::Result<(Vec<serde_json::Value>, EventDecoder)> {
        let mut decoder = EventDecoder::default();
        let mut events = Vec::new();
        for byte in input {
            if let Some(value) = decoder.push(*byte)? {
                events.push(value);
            }
        }
        Ok((events, decoder))
    }

    #[test]
    fn sse_decoder_accepts_lf_crlf_and_multidata_frames() {
        let (events, decoder) =
            decode_sse(b": keep-alive\r\n\r\ndata: {\"kind\":\ndata: \"joined\"}\r\n\r\n")
                .expect("valid SSE frames decode");
        assert!(decoder.at_boundary());
        assert_eq!(events, vec![serde_json::json!({"kind": "joined"})]);
    }

    #[test]
    fn sse_decoder_emits_each_event_and_ignores_non_data_fields() {
        let (events, decoder) =
            decode_sse(b"event: delta\nid: first\ndata: {\"n\":1}\n\n\ndata: {\"n\":2}\n\n")
                .expect("valid events decode");
        assert!(decoder.at_boundary());
        assert_eq!(
            events,
            vec![serde_json::json!({"n": 1}), serde_json::json!({"n": 2})]
        );
    }

    #[test]
    fn sse_decoder_rejects_invalid_utf8_and_json() {
        let utf8 = decode_sse(b"data: \xff\n\n")
            .map(|_| ())
            .expect_err("invalid UTF-8 must fail closed");
        assert!(utf8.to_string().contains("invalid UTF-8"));

        let json = decode_sse(b"data: {not-json}\n\n")
            .map(|_| ())
            .expect_err("invalid JSON must fail closed");
        assert!(json.to_string().contains("not valid JSON"));
    }

    #[test]
    fn sse_decoder_rejects_empty_data_frames() {
        let error = decode_sse(b"data\n\n")
            .map(|_| ())
            .expect_err("empty SSE data must fail closed");
        assert!(error.to_string().contains("not valid JSON"));
    }

    #[test]
    fn sse_decoder_detects_truncated_and_oversized_frames() {
        let (_, truncated) = decode_sse(b"data: {\"unfinished\": true}\n")
            .expect("a partial frame is accepted until EOF");
        assert!(!truncated.at_boundary());

        let mut decoder = EventDecoder {
            frame_bytes: MAX_SSE_FRAME,
            ..EventDecoder::default()
        };
        let error = decoder
            .push(b'x')
            .expect_err("the byte after the bound must be refused");
        assert!(error.to_string().contains("exceeds 32 MiB"));
    }
}
