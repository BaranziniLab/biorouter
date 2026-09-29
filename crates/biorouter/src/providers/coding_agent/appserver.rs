//! Newline-delimited JSON-RPC 2.0 over a child process's stdio.
//!
//! This is the transport `codex app-server` speaks. It is kept separate from the
//! Codex provider's protocol semantics so the framing, the request/response
//! correlation and the three-way message classification can be tested without
//! spawning a real Codex.
//!
//! # Why the traffic is genuinely bidirectional
//!
//! The obvious shape for a client is "send a request, await a response", and it is
//! not sufficient here. `codex app-server` sends **requests of its own** — every
//! approval flows back to the host as a server-originated request
//! (`item/commandExecution/requestApproval`, `item/fileChange/requestApproval`,
//! `mcpServer/elicitation/request`, `item/permissions/requestApproval`) and the
//! turn **blocks** until it is answered. A client that only reads responses
//! deadlocks the first time the agent wants to do anything.
//!
//! That property is the reason this surface was chosen over `codex exec --json`:
//! `exec` cannot answer an approval at all, so an MCP tool call there fails with
//! "user cancelled MCP tool call" unless the sandbox is opened up wholesale.
//! Approvals arriving here as requests is exactly what lets Biorouter keep the
//! decision.
//!
//! Inbound messages are therefore classified the way the protocol defines them:
//! an `id` with no `method` is a response; a `method` with an `id` is a request we
//! must answer; a `method` with no `id` is a notification.

use std::collections::HashMap;
use std::process::Stdio;

/// What the stdout reader hands to every in-flight request when the child's
/// stdout closes — i.e. when the vendor CLI died.
///
/// ⚠ It is a SENTINEL, not a message to show anyone. On its own it says only
/// that the pipe closed, which is the least useful true statement available:
/// the reason is on the child's stderr and this arrives without it. The request
/// path recognises it and re-reports with `stderr_suffix()` attached; see the
/// note there.
const CLOSED_OUTPUT: &str = "app server closed its output";
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

use super::super::errors::ProviderError;

/// In-flight requests, keyed by the id we sent.
///
/// A named type because it appears in five signatures and clippy is right that the
/// spelled-out form is unreadable: the `oneshot` carries `Result<Value, String>`
/// where the `String` is the JSON-RPC error message, so a rejected request is
/// distinguishable from one that resolved to `null`.
type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>>;

/// A single line may not exceed this. A runaway child that never emits a newline
/// would otherwise grow one `String` without bound.
const MAX_LINE_BYTES: usize = 32 * 1024 * 1024;

/// Something the server sent us that is not a reply to one of our requests.
#[derive(Debug, Clone)]
pub enum Inbound {
    /// Fire-and-forget. The turn's whole event stream arrives this way.
    Notification { method: String, params: Value },
    /// The server is asking us something and is **blocked** until we answer.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
}

/// A live JSON-RPC connection to a child process.
pub struct AppServer {
    child: tokio::process::Child,
    /// Process-scoped files that must outlive the child. Codex uses this to
    /// retain its isolated config home until the app server has stopped.
    _home_guard: Option<tempfile::TempDir>,
    stdin: Arc<Mutex<ChildStdin>>,
    next_id: AtomicI64,
    pending: Pending,
    /// Behind a `Mutex` so it can be pumped through `&self`. That is not
    /// incidental: a turn is driven by awaiting `turn/start` *while*
    /// simultaneously answering the requests the server makes during it, so the
    /// caller needs concurrent access to both halves. A `&mut self` receiver
    /// makes that shape impossible to write.
    inbound: Mutex<mpsc::UnboundedReceiver<Inbound>>,
    /// Bounded tail of the child's stderr, for error messages. Unbounded growth
    /// here would be a leak on a chatty child, and an undrained stderr pipe can
    /// block the child outright — which is what every one of the deleted CLI
    /// providers did.
    stderr: Arc<Mutex<String>>,
}

impl AppServer {
    /// Spawn `cmd` and start the reader.
    ///
    /// `cmd` must already have had its environment configured — including the
    /// subscription scrub — because this only sets the three stdio pipes.
    pub async fn spawn(cmd: Command) -> Result<Self, ProviderError> {
        Self::spawn_with_home(cmd, None).await
    }

    /// Spawn while retaining an isolated process home for the connection's
    /// entire lifetime.
    pub async fn spawn_with_home(
        mut cmd: Command,
        home_guard: Option<tempfile::TempDir>,
    ) -> Result<Self, ProviderError> {
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // ⚠ `kill_on_drop`, because a cancelled turn drops this future rather
        // than unwinding it. `drive_stream`'s hard-cancellation escape breaks
        // out of its `select!` while the provider call is still pending, the
        // stream is dropped, and every explicit reap below is skipped. Without
        // this the vendor CLI keeps running detached, holding the user's
        // subscription credential and burning their quota on an answer nobody
        // will read - and on the Codex path it also keeps the app-server port.
        // The default is false, which is why this has to be said out loud.
        cmd.kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| {
            ProviderError::ExecutionError(format!("could not start app server: {e}"))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ProviderError::ExecutionError("no stdin on app server".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ProviderError::ExecutionError("no stdout on app server".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ProviderError::ExecutionError("no stderr on app server".into()))?;

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (tx, inbound) = mpsc::unbounded_channel();
        let stderr_buf = Arc::new(Mutex::new(String::new()));

        spawn_reader(stdout, Arc::clone(&pending), tx);
        spawn_stderr_drain(stderr, Arc::clone(&stderr_buf));

        Ok(Self {
            child,
            _home_guard: home_guard,
            stdin: Arc::new(Mutex::new(stdin)),
            next_id: AtomicI64::new(1),
            pending,
            inbound: Mutex::new(inbound),
            stderr: stderr_buf,
        })
    }

    async fn write_line(&self, value: &Value) -> Result<(), ProviderError> {
        let mut line = serde_json::to_string(value)
            .map_err(|e| ProviderError::ExecutionError(format!("could not encode request: {e}")))?;
        line.push('\n');
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| ProviderError::ExecutionError(format!("app server stdin closed: {e}")))?;
        stdin
            .flush()
            .await
            .map_err(|e| ProviderError::ExecutionError(format!("app server stdin flush: {e}")))
    }

    /// Send a request and await its response.
    ///
    /// This has no timeout of its own, by design. The wait ends when the child
    /// answers, when the child exits (the reader then fails every waiter), or
    /// when the caller drops this future on cancellation. A turn has a wall
    /// clock only if an operator sets `BIOROUTER_CODING_AGENT_TURN_TIMEOUT_SECS`,
    /// and the provider applies it through [`super::await_turn`], not here. A
    /// caller that awaits this against a child that never answers therefore
    /// waits forever.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, ProviderError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let mut msg = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        if !params.is_null() {
            msg["params"] = params;
        }
        self.write_line(&msg).await?;

        match rx.await {
            Ok(Ok(result)) => Ok(result),
            // ⚠ The child-died case arrives HERE, not in the `Err(_)` arm below,
            // and that is why this arm cannot just format `e`. When stdout
            // closes the reader drains `pending` and answers every waiter with
            // `CLOSED_OUTPUT` — so the sender is used, not dropped, and the arm
            // written to explain a dead child never runs. Reported literally it
            // produced
            //
            //     Request failed: initialize: app server closed its output
            //
            // for a `codex` that had exited instantly with `env: node: No such
            // file or directory`, which was sitting unread on its stderr the
            // whole time. The pipe closing is the symptom; stderr is the cause.
            Ok(Err(e)) if e == CLOSED_OUTPUT => Err(ProviderError::ExecutionError(format!(
                "app server exited during `{method}`{}",
                self.stderr_suffix().await
            ))),
            Ok(Err(e)) => Err(ProviderError::RequestFailed(format!("{method}: {e}"))),
            // The reader dropped the sender without answering. Rarer than the
            // arm above, and the same explanation applies.
            Err(_) => Err(ProviderError::ExecutionError(format!(
                "app server exited during `{method}`{}",
                self.stderr_suffix().await
            ))),
        }
    }

    /// Send a notification, which has no reply.
    pub async fn notify(&self, method: &str, params: Value) -> Result<(), ProviderError> {
        let mut msg = json!({ "jsonrpc": "2.0", "method": method });
        if !params.is_null() {
            msg["params"] = params;
        }
        self.write_line(&msg).await
    }

    /// Answer a server-originated request. Until this is sent the turn is stalled.
    pub async fn respond(&self, id: &Value, result: Value) -> Result<(), ProviderError> {
        self.write_line(&json!({ "jsonrpc": "2.0", "id": id, "result": result }))
            .await
    }

    /// Reject a server-originated request.
    pub async fn respond_error(
        &self,
        id: &Value,
        code: i64,
        message: &str,
    ) -> Result<(), ProviderError> {
        self.write_line(
            &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
        )
        .await
    }

    /// The next notification or server request, or `None` once stdout closes.
    pub async fn next_inbound(&self) -> Option<Inbound> {
        self.inbound.lock().await.recv().await
    }

    /// A bounded tail of the child's stderr, formatted for appending to an error.
    pub async fn stderr_suffix(&self) -> String {
        let text = self.stderr.lock().await;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            String::new()
        } else {
            format!(": {trimmed}")
        }
    }

    /// Stop the child. Called on both the success and failure paths — a leaked
    /// `codex app-server` is a live process holding the user's credential.
    pub async fn shutdown(mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }
}

/// Classify and dispatch every line the child writes to stdout.
fn spawn_reader(
    stdout: tokio::process::ChildStdout,
    pending: Pending,
    tx: mpsc::UnboundedSender<Inbound>,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.len() > MAX_LINE_BYTES || line.trim().is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                // Not JSON. The app server writes only JSON to stdout, so this is
                // banner or diagnostic noise; dropping it is correct and must not
                // abort the stream.
                continue;
            };
            dispatch(value, &pending, &tx).await;
        }
        // stdout closed: fail every in-flight request rather than hanging.
        let mut guard = pending.lock().await;
        for (_, sender) in guard.drain() {
            let _ = sender.send(Err(CLOSED_OUTPUT.into()));
        }
    });
}

async fn dispatch(value: Value, pending: &Pending, tx: &mpsc::UnboundedSender<Inbound>) {
    let method = value.get("method").and_then(Value::as_str);
    let id = value.get("id");

    match (method, id) {
        // A response: has an id, carries no method.
        (None, Some(id)) => {
            let Some(id) = id.as_i64() else { return };
            let Some(sender) = pending.lock().await.remove(&id) else {
                return;
            };
            let outcome = if let Some(err) = value.get("error") {
                Err(err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error")
                    .to_string())
            } else {
                Ok(value.get("result").cloned().unwrap_or(Value::Null))
            };
            let _ = sender.send(outcome);
        }
        // A request: method AND id. The server is blocked until we answer.
        (Some(method), Some(id)) => {
            let _ = tx.send(Inbound::Request {
                id: id.clone(),
                method: method.to_string(),
                params: value.get("params").cloned().unwrap_or(Value::Null),
            });
        }
        // A notification: method, no id.
        (Some(method), None) => {
            let _ = tx.send(Inbound::Notification {
                method: method.to_string(),
                params: value.get("params").cloned().unwrap_or(Value::Null),
            });
        }
        (None, None) => {}
    }
}

fn spawn_stderr_drain(stderr: tokio::process::ChildStderr, buf: Arc<Mutex<String>>) {
    /// Keep only the tail. An error message wants the last thing that went wrong,
    /// not a megabyte of progress output.
    const STDERR_TAIL_BYTES: usize = 8 * 1024;

    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut guard = buf.lock().await;
            guard.push_str(&line);
            guard.push('\n');
            if guard.len() > STDERR_TAIL_BYTES {
                // `cut` is a BYTE offset and the child's stderr is arbitrary text, so
                // it can land inside a multi-byte character — slicing there panics.
                // Walk forward to the next boundary instead; losing a few bytes off
                // an already-truncated tail costs nothing.
                let cut = guard.len() - STDERR_TAIL_BYTES;
                let boundary = (cut..=guard.len())
                    .find(|&i| guard.is_char_boundary(i))
                    .unwrap_or(guard.len());
                let tail = guard.split_off(boundary);
                *guard = tail;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three-way classification is the whole correctness of the transport, and
    /// it is what a response-only client gets wrong. Exercised directly against
    /// `dispatch` so it needs no child process.
    #[tokio::test]
    async fn classifies_responses_requests_and_notifications() {
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (tx, mut rx) = mpsc::unbounded_channel();

        // A response resolves the matching pending request and emits nothing.
        let (rtx, rrx) = oneshot::channel();
        pending.lock().await.insert(7, rtx);
        dispatch(
            json!({"jsonrpc":"2.0","id":7,"result":{"threadId":"t-1"}}),
            &pending,
            &tx,
        )
        .await;
        assert_eq!(
            rrx.await.unwrap().unwrap(),
            json!({"threadId":"t-1"}),
            "a response must resolve its pending request"
        );
        assert!(rx.try_recv().is_err(), "a response is not an inbound event");

        // A server request must surface, or the turn stalls forever.
        dispatch(
            json!({"jsonrpc":"2.0","id":"srv-1","method":"mcpServer/elicitation/request",
                   "params":{"serverName":"biorouter"}}),
            &pending,
            &tx,
        )
        .await;
        match rx.try_recv().expect("a server request must be surfaced") {
            Inbound::Request { id, method, .. } => {
                assert_eq!(method, "mcpServer/elicitation/request");
                assert_eq!(id, json!("srv-1"), "a string id must round-trip unchanged");
            }
            other => panic!("expected a request, got {other:?}"),
        }

        // A notification surfaces without an id.
        dispatch(
            json!({"jsonrpc":"2.0","method":"turn/completed","params":{"usage":{}}}),
            &pending,
            &tx,
        )
        .await;
        assert!(matches!(
            rx.try_recv().unwrap(),
            Inbound::Notification { .. }
        ));
    }

    /// A JSON-RPC error response must reject the request rather than resolve it
    /// with an empty result, which would look like success.
    #[tokio::test]
    async fn an_error_response_rejects_the_request() {
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (tx, _rx) = mpsc::unbounded_channel();
        let (rtx, rrx) = oneshot::channel();
        pending.lock().await.insert(1, rtx);

        dispatch(
            json!({"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"no such method"}}),
            &pending,
            &tx,
        )
        .await;
        assert_eq!(rrx.await.unwrap().unwrap_err(), "no such method");
    }

    /// Non-JSON on stdout is banner noise and must not abort the stream or be
    /// mistaken for a message.
    #[tokio::test]
    async fn unparseable_lines_are_ignored() {
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (tx, mut rx) = mpsc::unbounded_channel();
        // A bare JSON value is neither a response, a request, nor a notification.
        dispatch(json!("just a string"), &pending, &tx).await;
        assert!(rx.try_recv().is_err());
    }

    /// End-to-end against a scripted fake server, so the framing and the
    /// bidirectional ordering are exercised for real: our request, its request
    /// back to us, our answer, then its response to our original.
    // ⚠ `cfg(unix)`: this test drives a fake app server written in Python over
    // stdio, and CI installs no Python on Windows (there is no `setup-python`
    // step in `.github/workflows/rust.yml`). On Windows `python3` resolves to
    // the Store's App Execution Alias or to nothing, the child never answers,
    // and `AppServer::request` — which by design has no timeout of its own, see
    // its doc comment — awaits its oneshot forever. That does not fail the job,
    // it HANGS it: `test (windows-latest)` ran 120+ minutes against a ~20 minute
    // norm until the runner's ceiling, on every head carrying this code. The
    // transport semantics under test are not platform-specific; the fake server
    // is.
    #[cfg(unix)]
    #[tokio::test]
    async fn drives_a_fake_server_including_a_server_originated_request() {
        let script = r#"
import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    m = json.loads(line)
    if m.get("method") == "initialize":
        print(json.dumps({"jsonrpc":"2.0","id":m["id"],"result":{"codexHome":"/tmp"}}), flush=True)
    elif m.get("method") == "turn/start":
        # Ask the host something first, and do not answer until it replies.
        print(json.dumps({"jsonrpc":"2.0","id":"ask-1","method":"item/tool/requestUserInput",
                          "params":{"q":"ok?"}}), flush=True)
        turn_id = m["id"]
        for reply in sys.stdin:
            r = json.loads(reply)
            if r.get("id") == "ask-1":
                print(json.dumps({"jsonrpc":"2.0","method":"turn/completed",
                                  "params":{"answered":r.get("result")}}), flush=True)
                print(json.dumps({"jsonrpc":"2.0","id":turn_id,"result":{}}), flush=True)
                break
"#;
        let mut cmd = Command::new("python3");
        cmd.arg("-c").arg(script);
        let server = AppServer::spawn(cmd)
            .await
            .expect("fake server should start");

        let init = server.request("initialize", json!({})).await.unwrap();
        assert_eq!(init["codexHome"], "/tmp");

        // Drive turn/start concurrently, because it cannot complete until we
        // answer the request it makes — which is the deadlock this transport
        // exists to avoid.
        let turn = async { server.request("turn/start", json!({})).await };
        let pump = async {
            let mut answered = None;
            while let Some(msg) = server.next_inbound().await {
                match msg {
                    Inbound::Request { id, method, .. } => {
                        assert_eq!(method, "item/tool/requestUserInput");
                        server
                            .respond(&id, json!({"decision": "yes"}))
                            .await
                            .unwrap();
                    }
                    Inbound::Notification { method, params } => {
                        if method == "turn/completed" {
                            answered = Some(params);
                            break;
                        }
                    }
                }
            }
            answered
        };
        let (turn_result, answered) = tokio::join!(turn, pump);
        turn_result.expect("turn/start should resolve once we answer");
        assert_eq!(
            answered.unwrap()["answered"],
            json!({"decision": "yes"}),
            "our answer must have reached the server"
        );
    }

    /// A child that dies mid-request must fail it, not hang, and must surface its
    /// stderr — the diagnostic is the only thing the user can act on.
    // ⚠ `cfg(unix)`: this test drives a fake app server written in Python over
    // stdio, and CI installs no Python on Windows (there is no `setup-python`
    // step in `.github/workflows/rust.yml`). On Windows `python3` resolves to
    // the Store's App Execution Alias or to nothing, the child never answers,
    // and `AppServer::request` — which by design has no timeout of its own, see
    // its doc comment — awaits its oneshot forever. That does not fail the job,
    // it HANGS it: `test (windows-latest)` ran 120+ minutes against a ~20 minute
    // norm until the runner's ceiling, on every head carrying this code. The
    // transport semantics under test are not platform-specific; the fake server
    // is.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_dead_child_fails_in_flight_requests_with_its_stderr() {
        let mut cmd = Command::new("python3");
        cmd.arg("-c")
            .arg("import sys; print('boom: could not start', file=sys.stderr); sys.exit(3)");
        let server = AppServer::spawn(cmd).await.unwrap();

        let err = server
            .request("initialize", json!({}))
            .await
            .expect_err("a dead child must fail the request");
        let text = err.to_string();
        assert!(
            text.contains("initialize"),
            "the error should name the method: {text}"
        );
    }
}
