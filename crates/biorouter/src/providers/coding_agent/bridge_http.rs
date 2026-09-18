//! The MCP endpoint a bridged coding agent calls to use Biorouter's own tools.
//!
//! `claude_code` and `codex` run their own agent loop in a child process with
//! their own file and shell tools switched off. This is how they get Biorouter's
//! tools instead: an MCP server, spoken over HTTP, whose `tools/call` runs inside
//! Biorouter behind Biorouter's inspectors, permission mode and privacy gates.
//! See [`crate::providers::coding_agent::bridge`] for why MCP is the only
//! channel that can return a tool *result* into a live turn of either CLI.
//!
//! # Authentication is the URL, and that is not a shortcut
//!
//! There is no secret-key check on this route. The path carries a 32-hex-character
//! single-turn nonce, and possession of it *is* the capability — a much narrower
//! one than the daemon's REST API, which this deliberately does not use: a grant
//! covers one session's already-filtered tool set for the duration of one turn,
//! and the lease revokes it on drop.
//!
//! It has to work this way. Claude Code will send an `Authorization` header from
//! its config file, but **Codex sends none at all** (observed: `auth=None` on
//! every request it made). A header-based scheme would therefore authenticate one
//! client and not the other. The nonce travels in the URL both CLIs are given, so
//! it works for both by construction.
//!
//! Consequences that follow, and are honoured below: the route must not log the
//! path, and it must answer an unknown nonce identically to a well-formed miss so
//! it cannot be used as an oracle.

use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use rmcp::model::CallToolRequestParams;
use serde_json::{json, Value};

use super::bridge;

/// The MCP protocol version this endpoint speaks.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// Takes no `AppState`, and that is deliberate rather than an oversight: the grant
/// registry is process-global because the handler runs on a different task from the
/// turn that issued the grant, so there is nothing session-shaped for this route to
/// be given. Being state-free is also what lets a live test serve the real route.
pub fn routes() -> Router {
    Router::new().route("/tool_bridge/{nonce}", post(handle))
}

/// One JSON-RPC message from the child.
async fn handle(Path(nonce): Path<String>, Json(message): Json<Value>) -> impl IntoResponse {
    let id = message.get("id").cloned();
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    // A notification (no id) is acknowledged and nothing is returned — answering
    // one with a JSON-RPC envelope is a protocol error some clients reject.
    if id.is_none() {
        return (StatusCode::ACCEPTED, Json(Value::Null)).into_response();
    }
    let id = id.unwrap_or(Value::Null);

    let Some(grant) = bridge::lookup(&nonce) else {
        // Deliberately indistinguishable from any other unusable nonce, and
        // deliberately not logged with the path: this is the credential.
        return rpc_error(
            id,
            -32001,
            "this tool bridge is no longer active; its turn has finished",
        );
    };

    match method.as_str() {
        "initialize" => rpc_ok(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "biorouter", "version": env!("CARGO_PKG_VERSION") }
            }),
        ),
        "tools/list" => {
            // Served verbatim from the grant. The set was filtered when the grant
            // was issued — tier and extension reach already applied — so there is
            // no second policy here to drift from the first.
            let tools: Vec<Value> = grant
                .tools()
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description.as_deref().unwrap_or_default(),
                        "inputSchema": t.input_schema,
                    })
                })
                .collect();
            rpc_ok(id, json!({ "tools": tools }))
        }
        "tools/call" => call_tool(id, grant, message.get("params")).await,
        // Claude Code probes this before `initialize`; an empty result is a clean
        // "nothing extra to discover" rather than an error it would log.
        "server/discover" | "ping" => rpc_ok(id, json!({})),
        other => rpc_error(id, -32601, &format!("`{other}` is not supported")),
    }
}

async fn call_tool(
    id: Value,
    grant: std::sync::Arc<bridge::BridgeGrant>,
    params: Option<&Value>,
) -> axum::response::Response {
    let Some(params) = params else {
        return rpc_error(id, -32602, "tools/call needs params");
    };
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return rpc_error(id, -32602, "tools/call needs a tool name");
    };
    let arguments = match params.get("arguments") {
        None | Some(Value::Null) => None,
        Some(Value::Object(map)) => Some(map.clone()),
        Some(_) => return rpc_error(id, -32602, "tools/call arguments must be an object"),
    };

    // The child's own id for this call. Not forwarded to the tool — `meta` stays
    // `None` — but it is what lets the transcript pair the full result the grant
    // keeps with the frame on which the child reports the call.
    let child_call_id = bridge::child_call_id(params.get("_meta"));
    let call = CallToolRequestParams {
        name: name.to_string().into(),
        arguments,
        meta: None,
        task: None,
    };

    // The child is answered with the model's view of the result: the blocks a
    // model is sent, unannotated (`bridge::child_view`, QA-E F4), and framed as
    // untrusted data + scanned for injection and PII first (A2). The child is a
    // whole agent reading third-party bytes, so the guardrail applies to it for
    // the same reason it applies to the parent model; `call_for_child` is the
    // bridge's half of that funnel, and the copy it keeps for the transcript is
    // framed too, so a coding agent's transcript matches every other provider's.
    match grant.call_for_child(call, child_call_id).await {
        Ok(result) => rpc_ok(
            id,
            serde_json::to_value(result).unwrap_or_else(|e| {
                json!({ "content": [{ "type": "text", "text": format!("could not encode result: {e}") }],
                        "isError": true })
            }),
        ),
        // A refusal is returned as a tool result with `isError`, not as a JSON-RPC
        // error. The distinction matters: a JSON-RPC error is a transport failure
        // the child may retry or treat as a broken server, whereas `isError` is a
        // result the model reads and can act on — which is what a policy refusal
        // is. It is how the model learns to ask the user instead of retrying.
        Err(reason) => rpc_ok(
            id,
            json!({ "content": [{ "type": "text", "text": reason }], "isError": true }),
        ),
    }
}

fn rpc_ok(id: Value, result: Value) -> axum::response::Response {
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}

fn rpc_error(id: Value, code: i64, message: &str) -> axum::response::Response {
    Json(json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    }))
    .into_response()
}

/// A standalone agent host serves only the nonce-authorized MCP surface.
/// The guard lives through command dispatch, including every session turn.
pub struct LoopbackBridge {
    base_url: String,
    published: bool,
    shutdown: tokio_util::sync::CancellationToken,
}

impl LoopbackBridge {
    pub async fn start() -> std::io::Result<Self> {
        let mut listener = Self::bind().await?;
        bridge::publish_base_url(listener.base_url.clone());
        listener.published = true;
        Ok(listener)
    }

    #[cfg(test)]
    pub(crate) async fn start_for_test() -> std::io::Result<Self> {
        Self::bind().await
    }

    async fn bind() -> std::io::Result<Self> {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let base_url = format!("http://{}", listener.local_addr()?);
        let shutdown = tokio_util::sync::CancellationToken::new();
        let request_shutdown = shutdown.clone();
        let app = routes().layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let shutdown = request_shutdown.clone();
                async move {
                    tokio::select! {
                        biased;
                        _ = shutdown.cancelled() => StatusCode::SERVICE_UNAVAILABLE.into_response(),
                        response = next.run(request) => response,
                    }
                }
            },
        ));
        let stop = shutdown.clone();
        tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, app)
                .with_graceful_shutdown(stop.cancelled_owned())
                .await
            {
                tracing::error!(%error, "Standalone tool bridge stopped");
            }
        });
        Ok(Self {
            base_url,
            published: false,
            shutdown,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

impl Drop for LoopbackBridge {
    fn drop(&mut self) {
        if self.published {
            bridge::clear_base_url_if(&self.base_url);
        }
        self.shutdown.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn standalone_listener_has_no_host_api_and_closes_existing_connections() {
        let listener = LoopbackBridge::start_for_test().await.unwrap();
        let address = listener
            .base_url()
            .strip_prefix("http://")
            .unwrap()
            .to_string();
        let client = reqwest::Client::new();
        assert_eq!(
            client
                .get(format!("{}/config", listener.base_url()))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        let mut pending = tokio::net::TcpStream::connect(&address).await.unwrap();
        pending.write_all(b"POST /tool_bridge/not-a-grant HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{}").await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        drop(listener);
        let mut response = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            pending.read_to_end(&mut response),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(tokio::net::TcpStream::connect(&address).await.is_err());
    }
}
