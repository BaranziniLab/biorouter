pub use biorouter::providers::coding_agent::bridge_http::routes;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::{http::StatusCode, Router};
    use serde_json::{json, Value};
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
        routes()
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
