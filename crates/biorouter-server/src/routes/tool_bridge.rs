//! The MCP endpoint a bridged coding agent calls to use Biorouter's own tools.
//!
//! `claude_code` and `codex` run their own agent loop in a child process with
//! their own file and shell tools switched off. This is how they get Biorouter's
//! tools instead: an MCP server, spoken over HTTP, whose `tools/call` runs inside
//! Biorouter behind Biorouter's inspectors, permission mode and privacy gates.
//! See [`biorouter::providers::coding_agent::bridge`] for why MCP is the only
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

use biorouter::providers::coding_agent::bridge;

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
    // model is sent, unannotated (`bridge::child_view`, QA-E F4).
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn post_rpc(app: &Router, nonce: &str, body: Value) -> (StatusCode, Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/tool_bridge/{nonce}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }

    fn app() -> Router {
        Router::new().route("/tool_bridge/{nonce}", post(handle))
    }

    /// An expired or invented nonce is refused, and the refusal says nothing that
    /// distinguishes the two — the nonce is the credential, so the route must not
    /// be usable as an oracle for which ones exist.
    #[tokio::test]
    async fn an_unknown_nonce_is_refused_without_leaking_whether_it_ever_existed() {
        let app = app();
        let (_, invented) = post_rpc(
            &app,
            "0123456789abcdef0123456789abcdef",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        )
        .await;
        let (_, malformed) = post_rpc(
            &app,
            "not-a-nonce",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        )
        .await;
        assert_eq!(
            invented["error"]["message"], malformed["error"]["message"],
            "the two must be indistinguishable"
        );
        assert_eq!(invented["error"]["code"], -32001);
    }

    /// A notification carries no id and must not be answered with an envelope.
    #[tokio::test]
    async fn a_notification_is_accepted_without_a_response_body() {
        let (status, body) = post_rpc(
            &app(),
            "0123456789abcdef0123456789abcdef",
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(body.get("error").is_none() && body.get("result").is_none());
    }

    /// The id must round-trip unchanged, including a string id — the transport
    /// correlates on it.
    #[tokio::test]
    async fn the_request_id_round_trips() {
        let (_, body) = post_rpc(
            &app(),
            "0123456789abcdef0123456789abcdef",
            json!({"jsonrpc":"2.0","id":"req-7","method":"tools/list"}),
        )
        .await;
        assert_eq!(body["id"], "req-7");
    }
}
