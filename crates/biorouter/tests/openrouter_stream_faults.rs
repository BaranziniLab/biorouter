biorouter::keep_this_test_binary_off_the_os_credential_store!();

use biorouter::agents::mistakes::is_recoverable;
use biorouter::providers::errors::ProviderError;
use biorouter::providers::formats::openai::response_to_streaming_message;
use futures::StreamExt;
use serde_json::{json, Value};

async fn error_from_frame(error: Value) -> ProviderError {
    error_from_chunk(
        json!({"error":error,"choices":[{"index":0,"delta":{},"finish_reason":"error"}]}),
    )
    .await
}

async fn error_from_chunk(frame: Value) -> ProviderError {
    let frames = vec![Ok(format!("data: {frame}")), Ok("data: [DONE]".to_string())];
    let stream = response_to_streaming_message(futures::stream::iter(frames));
    tokio::pin!(stream);
    while let Some(item) = stream.next().await {
        if let Err(error) = item {
            return error
                .downcast::<ProviderError>()
                .expect("the decoder must retain a typed provider error");
        }
    }
    panic!("an error frame cannot decode as a successful response");
}

#[tokio::test]
async fn issue_542_numeric_transient_stream_errors_do_not_require_an_openai_type() {
    for code in [408, 429, 500, 502, 503, 504] {
        let error =
            error_from_frame(json!({"code":code,"message":"PRIVATE_PROVIDER_PAYLOAD"})).await;
        assert!(is_recoverable(&error), "{code}: {error:?}");
        assert!(!error.to_string().contains("PRIVATE_PROVIDER_PAYLOAD"));
        assert!(!error.to_string().contains("Compute error."));
    }
}

#[tokio::test]
async fn issue_542_canonical_transient_metadata_is_recognized_without_provider_prose() {
    for kind in [
        "server",
        "timeout",
        "provider_overloaded",
        "provider_unavailable",
        "rate_limit_exceeded",
    ] {
        let error = error_from_frame(
            json!({"code":502,"metadata":{"error_type":kind},"message":"PRIVATE_PROVIDER_PAYLOAD"}),
        )
        .await;
        assert!(is_recoverable(&error), "{kind}: {error:?}");
        assert!(!error.to_string().contains("PRIVATE_PROVIDER_PAYLOAD"));
    }
}

#[tokio::test]
async fn issue_542_fatal_metadata_wins_over_a_server_status_and_error_finish() {
    for kind in [
        "authentication",
        "permission_denied",
        "payment_required",
        "content_policy_violation",
        "refusal",
    ] {
        let error = error_from_frame(json!({"code":500,"type":"server_error","metadata":{"error_type":kind},"message":"PRIVATE_PROVIDER_PAYLOAD"})).await;
        assert!(!is_recoverable(&error), "{kind}: {error:?}");
        assert!(!error.to_string().contains("PRIVATE_PROVIDER_PAYLOAD"));
    }
}

#[tokio::test]
async fn issue_542_explicit_invalid_request_and_fatal_statuses_do_not_become_transient() {
    for code in [400, 401, 402, 403, 500, 503] {
        let error = error_from_frame(json!({"code":code,"type":"invalid_request_error","message":"PRIVATE_PROVIDER_PAYLOAD"})).await;
        assert!(!is_recoverable(&error), "{code}: {error:?}");
    }
}

#[tokio::test]
async fn issue_542_context_metadata_preserves_the_compaction_route() {
    let error = error_from_frame(json!({"code":500,"type":"server_error","metadata":{"error_type":"context_length_exceeded"},"message":"PRIVATE_PROVIDER_PAYLOAD"})).await;
    assert!(
        matches!(error, ProviderError::ContextLengthExceeded(_)),
        "{error:?}"
    );
    assert!(!error.to_string().contains("PRIVATE_PROVIDER_PAYLOAD"));
}

#[tokio::test]
async fn issue_542_a_bare_error_finish_is_not_a_successful_stop() {
    let error = error_from_chunk(json!({"id":"synthetic","choices":[{"index":0,"delta":{"content":""},"finish_reason":"error"}]})).await;
    assert!(is_recoverable(&error), "{error:?}");
}

#[tokio::test]
async fn issue_542_specific_context_code_retains_compaction_with_a_generic_error_type() {
    let error = error_from_frame(json!({
        "code": "context_length_exceeded",
        "type": "invalid_request_error",
        "message": "PRIVATE_PROVIDER_PAYLOAD"
    }))
    .await;

    assert!(
        matches!(error, ProviderError::ContextLengthExceeded(_)),
        "context_length_exceeded must retain the compaction variant: {error:?}"
    );
    assert!(
        !error.to_string().contains("PRIVATE_PROVIDER_PAYLOAD"),
        "provider payload must not leak through the compaction error: {error:?}"
    );
}

#[tokio::test]
async fn issue_542_authentication_status_retains_the_credential_refresh_variant() {
    let cases = [
        (
            "numeric 401 with generic invalid request type",
            json!({
                "code": 401,
                "type": "invalid_request_error",
                "message": "PRIVATE_PROVIDER_PAYLOAD"
            }),
        ),
        (
            "string 401 with context-length metadata",
            json!({
                "code": "401",
                "type": "server_error",
                "metadata": {"error_type": "context_length_exceeded"},
                "message": "PRIVATE_PROVIDER_PAYLOAD"
            }),
        ),
        (
            "numeric 403 with generic invalid request type",
            json!({
                "code": 403,
                "type": "invalid_request_error",
                "message": "PRIVATE_PROVIDER_PAYLOAD"
            }),
        ),
        (
            "string 403 with context-length metadata",
            json!({
                "code": "403",
                "type": "server_error",
                "metadata": {"error_type": "context_length_exceeded"},
                "message": "PRIVATE_PROVIDER_PAYLOAD"
            }),
        ),
        (
            "ordinary server error type",
            json!({
                "code": 401,
                "type": "server_error",
                "message": "PRIVATE_PROVIDER_PAYLOAD"
            }),
        ),
    ];

    for (label, frame) in cases {
        let error = error_from_frame(frame).await;
        assert!(
            matches!(error, ProviderError::Authentication(_)),
            "{label} must retain the authentication variant: {error:?}"
        );
        assert!(
            !is_recoverable(&error),
            "{label} must not be recoverable: {error:?}"
        );
        assert!(
            !error.to_string().contains("PRIVATE_PROVIDER_PAYLOAD"),
            "{label} must not leak the provider payload: {error:?}"
        );
    }
}

#[tokio::test]
async fn issue_542_content_filter_code_cannot_be_retried_as_a_server_error() {
    let error = error_from_frame(json!({
        "code": "content_filter",
        "type": "server_error",
        "message": "PRIVATE_PROVIDER_PAYLOAD"
    }))
    .await;

    assert_eq!(
        error.kind(),
        biorouter::providers::errors::ProviderErrorKind::Policy,
        "content_filter must retain fatal policy classification: {error:?}"
    );
    assert!(
        !is_recoverable(&error),
        "content_filter must not be retried as a server error: {error:?}"
    );
    assert!(
        !error.to_string().contains("PRIVATE_PROVIDER_PAYLOAD"),
        "provider payload must not leak through the policy error: {error:?}"
    );
}
