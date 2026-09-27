use super::response_compression;
use axum::{body::Body, http::Request, routing::get, Router};
use bytes::Bytes;
use futures::StreamExt;
use std::convert::Infallible;
use tower::ServiceExt;

fn app(content_type: &'static str, body: Bytes) -> Router {
    Router::new()
        .route(
            "/",
            get(move || {
                let body = body.clone();
                async move {
                    axum::response::Response::builder()
                        .header("content-type", content_type)
                        .body(Body::from(body))
                        .expect("test response is valid")
                }
            }),
        )
        .layer(response_compression())
}

fn streaming_app(content_type: &'static str, body: Bytes) -> Router {
    Router::new()
        .route(
            "/",
            get(move || {
                let body = body.clone();
                async move {
                    let stream =
                        futures::stream::once(async move { Ok::<Bytes, Infallible>(body) })
                            .chain(futures::stream::pending());
                    axum::response::Response::builder()
                        .header("content-type", content_type)
                        .body(Body::from_stream(stream))
                        .expect("test response is valid")
                }
            }),
        )
        .layer(response_compression())
}

async fn request(router: Router) -> axum::response::Response {
    router
        .oneshot(
            Request::get("/")
                .header("accept-encoding", "gzip")
                .body(Body::empty())
                .expect("test request is valid"),
        )
        .await
        .expect("compression route responds")
}

#[tokio::test]
async fn large_json_still_uses_gzip() {
    let original = serde_json::to_vec(&serde_json::json!({
        "payload": "x".repeat(16 * 1024),
    }))
    .expect("test JSON is valid");
    let response = request(app("application/json", Bytes::from(original.clone()))).await;
    assert_eq!(response.headers()["content-encoding"], "gzip");
    let decoded = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("compressed body reads");
    assert!(!decoded.is_empty());
    assert_ne!(decoded.as_ref(), original.as_slice());
}

#[tokio::test]
async fn ndjson_first_frame_is_available_without_stream_close() {
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        request(streaming_app(
            "application/x-ndjson",
            Bytes::from_static(b"{\"type\":\"state\"}\n"),
        )),
    )
    .await
    .expect("NDJSON headers arrive promptly");
    assert!(response.headers().get("content-encoding").is_none());
    let mut body = response.into_body().into_data_stream();
    let frame = tokio::time::timeout(std::time::Duration::from_secs(8), body.next())
        .await
        .expect("first NDJSON frame arrives without stream close")
        .expect("stream has a first frame")
        .expect("first frame is readable");
    assert_eq!(frame, Bytes::from_static(b"{\"type\":\"state\"}\n"));
}

#[tokio::test]
async fn sse_first_frame_is_available_without_stream_close() {
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        request(streaming_app(
            "text/event-stream",
            Bytes::from_static(b"data: state\n\n"),
        )),
    )
    .await
    .expect("SSE headers arrive promptly");
    assert!(response.headers().get("content-encoding").is_none());
    let mut body = response.into_body().into_data_stream();
    let frame = tokio::time::timeout(std::time::Duration::from_secs(8), body.next())
        .await
        .expect("first SSE frame arrives without stream close")
        .expect("stream has a first frame")
        .expect("first frame is readable");
    assert_eq!(frame, Bytes::from_static(b"data: state\n\n"));
}
