use biorouter::conversation::message::{MessageContent, ToolRequest};
use biorouter::providers::base::ProviderStreamItem;
use biorouter::providers::formats::openai::response_to_streaming_message;
use futures::{pin_mut, StreamExt};
use serde_json::{json, Value};

fn chunk(delta: Value, finish: Option<&str>) -> String {
    format!(
        "data: {}",
        json!({"model":"synthetic-gpt","choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
    )
}

fn start(arguments: &str, finish: Option<&str>) -> String {
    chunk(
        json!({"tool_calls":[{"index":0,"id":"call_sqlite","function":{"name":"developer__text_editor","arguments":arguments}}]}),
        finish,
    )
}

async fn decode(lines: Vec<String>) -> anyhow::Result<Vec<ProviderStreamItem>> {
    let stream = response_to_streaming_message(tokio_stream::iter(lines.into_iter().map(Ok)));
    pin_mut!(stream);
    let mut result = Vec::new();
    while let Some(item) = stream.next().await {
        result.push(item?);
    }
    Ok(result)
}

fn requests(items: &[ProviderStreamItem]) -> Vec<&ToolRequest> {
    items
        .iter()
        .filter_map(|(message, _, _)| message.as_ref())
        .flat_map(|message| &message.content)
        .filter_map(|content| match content {
            MessageContent::ToolRequest(request) => Some(request),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn eof_without_completion_never_dispatches_even_valid_json() -> anyhow::Result<()> {
    let items = decode(vec![start(r#"{"path":"/tmp/synthetic.py"}"#, None)]).await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    assert!(calls[0].tool_call.is_err());
    Ok(())
}

#[tokio::test]
async fn done_without_completion_reports_pending_call_instead_of_losing_it() -> anyhow::Result<()> {
    let items = decode(vec![
        start(r#"{"path":"/tmp/"#, None),
        "data: [DONE]".into(),
    ])
    .await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    assert!(calls[0].tool_call.is_err());
    Ok(())
}

#[tokio::test]
async fn length_stop_does_not_dispatch_syntactically_valid_arguments() -> anyhow::Result<()> {
    let items = decode(vec![start("{}", None), chunk(json!({}), Some("length"))]).await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    assert!(calls[0].tool_call.is_err());
    Ok(())
}

#[tokio::test]
async fn first_chunk_length_is_not_reinterpreted_as_success() -> anyhow::Result<()> {
    let items = decode(vec![start("{}", Some("length")), "data: [DONE]".into()]).await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    assert!(calls[0].tool_call.is_err());
    Ok(())
}

#[tokio::test]
async fn non_object_arguments_are_failed_calls_not_panics_or_empty_objects() -> anyhow::Result<()> {
    for arguments in ["[]", "null", "42", "\"text\""] {
        let items = decode(vec![start(arguments, Some("tool_calls"))]).await?;
        let calls = requests(&items);
        assert_eq!(calls.len(), 1);
        assert!(calls[0].tool_call.is_err(), "non-object arguments accepted");
    }
    Ok(())
}

#[tokio::test]
async fn sse_data_field_without_space_preserves_complete_call() -> anyhow::Result<()> {
    let line = start("{}", Some("tool_calls")).replacen("data: ", "data:", 1);
    let items = decode(vec![line, "data:[DONE]".into()]).await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    assert!(calls[0].tool_call.is_ok());
    Ok(())
}

#[tokio::test]
async fn usage_only_chunk_does_not_close_inflight_arguments() -> anyhow::Result<()> {
    let items = decode(vec![
        start(r#"{"path":""#, None),
        format!("data: {}", json!({"model":"synthetic-gpt","choices":[],"usage":{"prompt_tokens":12,"completion_tokens":2,"total_tokens":14}})),
        chunk(json!({"tool_calls":[{"index":0,"function":{"arguments":"/tmp/synthetic.py\"}"}}]}), None),
        chunk(json!({}), Some("tool_calls")),
        "data: [DONE]".into(),
    ]).await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    let call = calls[0].tool_call.as_ref().expect("complete call");
    assert_eq!(
        call.arguments.as_ref().unwrap().get("path"),
        Some(&json!("/tmp/synthetic.py"))
    );
    Ok(())
}

#[tokio::test]
async fn completed_fragmented_call_retains_usage_and_exact_arguments() -> anyhow::Result<()> {
    let items = decode(vec![
        start(r#"{"path":""#, None),
        chunk(json!({"tool_calls":[{"index":0,"function":{"arguments":"/tmp/分析.py\"}"}}]}), None),
        chunk(json!({}), Some("tool_calls")),
        format!("data: {}", json!({"model":"synthetic-gpt","choices":[],"usage":{"prompt_tokens":12,"completion_tokens":8,"total_tokens":20}})),
        "data: [DONE]".into(),
    ]).await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    let call = calls[0].tool_call.as_ref().expect("complete call");
    assert_eq!(
        call.arguments.as_ref().unwrap().get("path"),
        Some(&json!("/tmp/分析.py"))
    );
    let usage = items
        .iter()
        .filter_map(|(_, usage, _)| usage.as_ref())
        .next_back()
        .unwrap();
    assert_eq!(usage.finish_reason.as_deref(), Some("tool_calls"));
    assert_eq!(usage.usage.total_tokens, Some(20));
    Ok(())
}

#[tokio::test]
async fn done_after_pending_call_ignores_all_later_bytes() -> anyhow::Result<()> {
    let items = decode(vec![
        start("{}", None),
        "data: [DONE]".into(),
        "data: not-json-after-terminal-event".into(),
    ])
    .await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    assert!(calls[0].tool_call.is_err());
    Ok(())
}

#[tokio::test]
async fn done_after_pending_call_does_not_wait_for_connection_close() -> anyhow::Result<()> {
    let input = tokio_stream::iter(vec![Ok(start("{}", None)), Ok("data: [DONE]".into())])
        .chain(futures::stream::pending::<anyhow::Result<String>>());
    let stream = response_to_streaming_message(input);
    pin_mut!(stream);
    let items = tokio::time::timeout(std::time::Duration::from_millis(500), async {
        let mut items = Vec::new();
        while let Some(item) = stream.next().await {
            items.push(item?);
        }
        anyhow::Ok(items)
    })
    .await??;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    assert!(calls[0].tool_call.is_err());
    Ok(())
}

fn filter_frame(result: Value) -> String {
    format!(
        "data: {}",
        json!({"model":"","id":"","created":0,"choices":[{"index":0,"content_filter_results":result,"finish_reason":null}]})
    )
}

#[tokio::test]
async fn filter_service_errors_are_readable_and_never_echo_provider_data() {
    for key in ["content_filter_result", "content_filter_results"] {
        for delta in [None, Some(json!({}))] {
            let mut choice = json!({"index":0});
            choice[key] = json!({"error":{"code":"content_filter_error","message":"SECRET_PROVIDER_PAYLOAD"}});
            if let Some(delta) = delta {
                choice["delta"] = delta;
            }
            let error = decode(vec![format!("data: {}", json!({"choices":[choice]}))])
                .await
                .unwrap_err();
            assert!(error
                .downcast_ref::<biorouter::providers::errors::ProviderError>()
                .is_some());
            let text = error.to_string();
            assert!(text.contains("Provider content filter failed"), "{text}");
            assert!(!text.contains("SECRET_PROVIDER_PAYLOAD"));
            assert!(!text.contains("missing field"));
        }
    }
}

#[tokio::test]
async fn filter_errors_abort_pending_tools_without_dispatching_them() {
    let frames = vec![
        start("{}", None),
        filter_frame(
            json!({"error":{"code":"content_filter_error","message":"SECRET_PROVIDER_PAYLOAD"}}),
        ),
        chunk(json!({}), Some("tool_calls")),
    ];
    let stream = response_to_streaming_message(tokio_stream::iter(frames.into_iter().map(Ok)));
    pin_mut!(stream);
    let pending = stream.next().await.unwrap().unwrap();
    assert!(pending.2.is_some());
    let error = stream.next().await.unwrap().unwrap_err();
    assert!(error.to_string().contains("Provider content filter failed"));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn azure_filter_annotations_preserve_text_and_usage() -> anyhow::Result<()> {
    let items = decode(vec![
        filter_frame(json!({"hate":{"filtered":false,"severity":"safe"}})),
        chunk(json!({"content":"ready"}), None),
        filter_frame(json!({"protected_material_text":{"detected":false,"filtered":false}})),
        chunk(json!({}), Some("stop")),
        format!(
            "data: {}",
            json!({"model":"synthetic-gpt","choices":[],"usage":{"total_tokens":14}})
        ),
    ])
    .await?;
    let text: String = items
        .iter()
        .filter_map(|(message, _, _)| message.as_ref())
        .flat_map(|message| &message.content)
        .filter_map(|content| match content {
            MessageContent::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "ready");
    let usage = items
        .iter()
        .filter_map(|(_, usage, _)| usage.as_ref())
        .next_back()
        .unwrap();
    assert_eq!(usage.usage.total_tokens, Some(14));
    assert_eq!(usage.finish_reason.as_deref(), Some("stop"));
    Ok(())
}

#[tokio::test]
async fn azure_filter_annotations_do_not_complete_pending_tools() -> anyhow::Result<()> {
    let items = decode(vec![
        start(r#"{"path":""#, None),
        filter_frame(json!({"hate":{"filtered":false,"severity":"safe"}})),
        format!(
            "data: {}",
            json!({"choices":[],"usage":{"total_tokens":14}})
        ),
        chunk(
            json!({"tool_calls":[{"index":0,"function":{"arguments":"/tmp/fixture\"}"}}]}),
            None,
        ),
        chunk(json!({}), Some("tool_calls")),
    ])
    .await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0]
            .tool_call
            .as_ref()
            .unwrap()
            .arguments
            .as_ref()
            .unwrap()["path"],
        "/tmp/fixture"
    );
    let usage = items
        .iter()
        .find_map(|(_, usage, _)| usage.as_ref())
        .unwrap();
    assert_eq!(usage.model, "synthetic-gpt");
    assert_eq!(usage.usage.total_tokens, Some(14));
    Ok(())
}

#[tokio::test]
async fn blocked_filter_frames_are_errors_even_without_delta_or_finish_reason() {
    for frame in [
        filter_frame(json!({"violence":{"filtered":true,"severity":"high"}})),
        format!(
            "data: {}",
            json!({"choices":[{"index":0,"finish_reason":"content_filter"}]})
        ),
    ] {
        let error = decode(vec![frame]).await.unwrap_err();
        assert!(error.to_string().contains("safety filter blocked"));
        let provider_error = error
            .downcast_ref::<biorouter::providers::errors::ProviderError>()
            .unwrap();
        assert_eq!(
            provider_error.kind(),
            biorouter::providers::errors::ProviderErrorKind::Policy
        );
        assert!(!biorouter::agents::mistakes::is_recoverable(provider_error));
    }
}

#[tokio::test]
async fn unrecognized_or_malformed_choices_still_fail_without_payload_disclosure() {
    for choice in [
        json!({"index":0}),
        json!({"index":0,"delta":null}),
        json!({"index":0,"delta":"SECRET_PROVIDER_PAYLOAD"}),
        json!({"index":0,"delta":"SECRET_PROVIDER_PAYLOAD","content_filter_results":{"hate":{"filtered":false}}}),
        json!({"index":0,"content_filter_results":{}}),
        json!({"index":0,"content_filter_results":{"error":null}}),
        json!({"index":0,"content_filter_results":{"unexpected":"SECRET_PROVIDER_PAYLOAD"}}),
        json!({"index":0,"delta":{"content":"SECRET_PROVIDER_PAYLOAD","tool_calls":"SECRET_PROVIDER_PAYLOAD"}}),
    ] {
        for prefix in [vec![], vec![start("{}", None)]] {
            let mut frames = prefix;
            frames.push(format!("data: {}", json!({"choices":[choice]})));
            let error = decode(frames).await.unwrap_err().to_string();
            assert!(error.contains("invalid chunk shape"), "{error}");
            assert!(!error.contains("SECRET_PROVIDER_PAYLOAD"));
        }
    }
}

#[tokio::test]
async fn invalid_json_and_top_level_errors_never_echo_provider_payloads() {
    for frame in [
        "data: {\"SECRET_PROVIDER_PAYLOAD\": nope}".to_string(),
        format!(
            "data: {}",
            json!({"error":{"message":"SECRET_PROVIDER_PAYLOAD"}})
        ),
    ] {
        for prefix in [vec![], vec![start("{}", None)]] {
            let mut frames = prefix;
            frames.push(frame.clone());
            let error = decode(frames).await.unwrap_err().to_string();
            assert!(!error.contains("SECRET_PROVIDER_PAYLOAD"));
            assert!(!error.contains("Retry"));
            assert!(
                error.contains("invalid JSON") || error.contains("Provider reported an error"),
                "{error}"
            );
        }
    }
}

#[tokio::test]
async fn unknown_filter_errors_do_not_promise_retry_or_disclose_details() {
    let error = decode(vec![filter_frame(
        json!({"error":{"code":"SECRET_PROVIDER_PAYLOAD","message":"SECRET_PROVIDER_PAYLOAD"}}),
    )])
    .await
    .unwrap_err();
    let provider_error = error
        .downcast_ref::<biorouter::providers::errors::ProviderError>()
        .unwrap();
    assert_eq!(
        provider_error.kind(),
        biorouter::providers::errors::ProviderErrorKind::Other
    );
    assert!(!biorouter::agents::mistakes::is_recoverable(provider_error));
    let text = error.to_string();
    assert!(text.contains("Provider content filter reported an error"));
    assert!(!text.contains("Retry"));
    assert!(!text.contains("SECRET_PROVIDER_PAYLOAD"));
}
