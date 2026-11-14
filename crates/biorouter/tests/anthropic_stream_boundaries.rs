biorouter::keep_this_test_binary_off_the_os_credential_store!();

use biorouter::conversation::message::{MessageContent, ToolRequest};
use biorouter::providers::base::ProviderStreamItem;
use biorouter::providers::formats::anthropic::response_to_streaming_message;
use futures::{pin_mut, StreamExt};
use serde_json::{json, Value};

const RAW_SENTINEL: &str = "ANTHROPIC_RAW_ARGUMENT_SENTINEL";

fn data(event: Value) -> String {
    format!("data: {event}")
}

fn tool_start(id: &str, name: &str) -> String {
    indexed_tool_start(0, id, name)
}

fn indexed_tool_start(index: usize, id: &str, name: &str) -> String {
    data(json!({
        "type": "content_block_start",
        "index": index,
        "content_block": {"type": "tool_use", "id": id, "name": name}
    }))
}

fn args_delta(fragment: &str) -> String {
    indexed_args_delta(0, fragment)
}

fn indexed_args_delta(index: usize, fragment: &str) -> String {
    data(json!({
        "type": "content_block_delta",
        "index": index,
        "delta": {"type": "input_json_delta", "partial_json": fragment}
    }))
}

fn tool_stop() -> String {
    indexed_tool_stop(0)
}

fn indexed_tool_stop(index: usize) -> String {
    data(json!({"type": "content_block_stop", "index": index}))
}

fn message_delta() -> String {
    data(json!({
        "type": "message_delta",
        "delta": {"stop_reason": "tool_use"},
        "usage": {"output_tokens": 4}
    }))
}

async fn decode(lines: Vec<String>) -> anyhow::Result<Vec<ProviderStreamItem>> {
    let stream = response_to_streaming_message(tokio_stream::iter(lines.into_iter().map(Ok)));
    pin_mut!(stream);
    let mut items = Vec::new();
    while let Some(item) = stream.next().await {
        items.push(item?);
    }
    Ok(items)
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

fn assert_failure(request: &ToolRequest, expected_kind: &str) {
    let error = request
        .tool_call
        .as_ref()
        .expect_err("the decoder must not expose a callable request");
    assert_eq!(
        error
            .data
            .as_ref()
            .and_then(|data| data.get("biorouterToolCallFailure"))
            .and_then(Value::as_str),
        Some(expected_kind)
    );
    assert!(
        !error.message.contains(RAW_SENTINEL),
        "raw arguments reached the tool-call error"
    );
}

#[tokio::test]
async fn anthropic_stream_boundary_accepts_no_space_data_fields() -> anyhow::Result<()> {
    let lines = vec![
        tool_start("toolu_complete", "developer__shell"),
        args_delta(r#"{"command":"printf complete"}"#),
        tool_stop(),
        message_delta(),
    ]
    .into_iter()
    .map(|line| line.replacen("data: ", "data:", 1))
    .collect();

    let items = decode(lines).await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    let call = calls[0].tool_call.as_ref().expect("complete tool call");
    assert_eq!(
        call.arguments
            .as_ref()
            .and_then(|arguments| arguments.get("command"))
            .and_then(Value::as_str),
        Some("printf complete")
    );
    Ok(())
}

#[tokio::test]
async fn anthropic_stream_boundary_rejects_completed_malformed_json_without_raw_arguments(
) -> anyhow::Result<()> {
    let items = decode(vec![
        tool_start("toolu_bad_json", "developer__shell"),
        args_delta(&format!(r#"{{"command":"{RAW_SENTINEL}""#)),
        tool_stop(),
        message_delta(),
    ])
    .await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    assert_failure(calls[0], "invalid_arguments");
    Ok(())
}

async fn assert_completed_non_object_fails(arguments: &str) -> anyhow::Result<()> {
    let items = decode(vec![
        tool_start("toolu_non_object", "developer__shell"),
        args_delta(arguments),
        tool_stop(),
        message_delta(),
    ])
    .await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1, "arguments: {arguments}");
    assert_failure(calls[0], "invalid_arguments");
    Ok(())
}

#[tokio::test]
async fn anthropic_stream_boundary_rejects_completed_array_arguments() -> anyhow::Result<()> {
    assert_completed_non_object_fails("[]").await
}

#[tokio::test]
async fn anthropic_stream_boundary_rejects_completed_null_arguments() -> anyhow::Result<()> {
    assert_completed_non_object_fails("null").await
}

#[tokio::test]
async fn anthropic_stream_boundary_rejects_completed_scalar_arguments() -> anyhow::Result<()> {
    assert_completed_non_object_fails("42").await
}

#[tokio::test]
async fn anthropic_stream_boundary_rejects_completed_string_arguments() -> anyhow::Result<()> {
    assert_completed_non_object_fails(r#""text""#).await
}

async fn assert_unfinished_fails(lines: Vec<String>) -> anyhow::Result<()> {
    let items = decode(lines).await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    assert_failure(calls[0], "incomplete_stream");
    Ok(())
}

#[tokio::test]
async fn anthropic_stream_boundary_rejects_parseable_arguments_at_eof_without_block_stop(
) -> anyhow::Result<()> {
    assert_unfinished_fails(vec![
        tool_start("toolu_eof", "developer__shell"),
        args_delta(r#"{"command":"printf never-ran"}"#),
    ])
    .await
}

#[tokio::test]
async fn anthropic_stream_boundary_rejects_partial_arguments_at_done_without_block_stop(
) -> anyhow::Result<()> {
    assert_unfinished_fails(vec![
        tool_start("toolu_done", "developer__shell"),
        args_delta(r#"{"command":"partial""#),
        "data: [DONE]".to_string(),
    ])
    .await
}

#[tokio::test]
async fn anthropic_stream_boundary_rejects_open_block_at_message_stop() -> anyhow::Result<()> {
    assert_unfinished_fails(vec![
        tool_start("toolu_message_stop", "developer__shell"),
        args_delta("{}"),
        data(json!({"type": "message_stop"})),
    ])
    .await
}

#[tokio::test]
async fn anthropic_stream_boundary_malformed_event_fails_closed_without_echoing_payload() {
    let error = decode(vec![
        tool_start("toolu_bad_frame", "developer__shell"),
        format!("data: {{not-json:{RAW_SENTINEL}"),
        tool_stop(),
        message_delta(),
    ])
    .await
    .expect_err("a malformed SSE event must fail the decoder");
    assert!(
        !format!("{error:#}").contains(RAW_SENTINEL),
        "the decoder error disclosed the malformed payload"
    );
}

#[tokio::test]
async fn anthropic_stream_boundary_keeps_closed_call_and_rejects_later_open_call(
) -> anyhow::Result<()> {
    let items = decode(vec![
        tool_start("toolu_closed", "developer__shell"),
        args_delta(r#"{"command":"printf closed"}"#),
        tool_stop(),
        data(json!({
            "type": "content_block_start",
            "index": 1,
            "content_block": {
                "type": "tool_use",
                "id": "toolu_open",
                "name": "developer__text_editor"
            }
        })),
        data(json!({
            "type": "content_block_delta",
            "index": 1,
            "delta": {
                "type": "input_json_delta",
                "partial_json": r#"{"command":"view""#
            }
        })),
        "data: [DONE]".to_string(),
    ])
    .await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 2);
    assert!(calls
        .iter()
        .any(|call| call.id == "toolu_closed" && call.tool_call.is_ok()));
    let unfinished = calls
        .iter()
        .find(|call| call.id == "toolu_open")
        .expect("unfinished second call");
    assert_failure(unfinished, "incomplete_stream");
    Ok(())
}

#[tokio::test]
async fn anthropic_stream_boundary_binds_interleaved_blocks_by_index_and_start_order(
) -> anyhow::Result<()> {
    let items = decode(vec![
        indexed_tool_start(0, "toolu_first", "developer__shell"),
        indexed_tool_start(1, "toolu_second", "developer__text_editor"),
        indexed_args_delta(0, r#"{"command":"printf first"}"#),
        indexed_args_delta(1, r#"{"path":"synthetic.txt"}"#),
        indexed_tool_stop(0),
        indexed_tool_stop(1),
        message_delta(),
    ])
    .await?;
    let calls = requests(&items);
    assert_eq!(
        calls
            .iter()
            .map(|call| call.id.as_str())
            .collect::<Vec<_>>(),
        vec!["toolu_first", "toolu_second"]
    );
    assert_eq!(
        calls[0]
            .tool_call
            .as_ref()
            .expect("first call")
            .arguments
            .as_ref()
            .and_then(|arguments| arguments.get("command"))
            .and_then(Value::as_str),
        Some("printf first")
    );
    assert_eq!(
        calls[1]
            .tool_call
            .as_ref()
            .expect("second call")
            .arguments
            .as_ref()
            .and_then(|arguments| arguments.get("path"))
            .and_then(Value::as_str),
        Some("synthetic.txt")
    );
    Ok(())
}

#[tokio::test]
async fn anthropic_stream_boundary_ignores_mismatched_delta_and_stop_indices() -> anyhow::Result<()>
{
    let items = decode(vec![
        indexed_tool_start(0, "toolu_bound", "developer__shell"),
        indexed_args_delta(1, &format!(r#"{{"command":"{RAW_SENTINEL}"}}"#)),
        indexed_tool_stop(1),
        indexed_args_delta(0, r#"{"command":"printf bound"}"#),
        indexed_tool_stop(0),
        message_delta(),
    ])
    .await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    let call = calls[0].tool_call.as_ref().expect("index-bound tool call");
    assert_eq!(
        call.arguments
            .as_ref()
            .and_then(|arguments| arguments.get("command"))
            .and_then(Value::as_str),
        Some("printf bound")
    );
    Ok(())
}

#[tokio::test]
async fn anthropic_stream_boundary_preserves_start_order_when_stops_are_reversed(
) -> anyhow::Result<()> {
    let items = decode(vec![
        indexed_tool_start(0, "toolu_first_reverse", "developer__shell"),
        indexed_tool_start(1, "toolu_second_reverse", "developer__text_editor"),
        indexed_args_delta(0, r#"{"command":"printf first"}"#),
        indexed_args_delta(1, r#"{"path":"synthetic.txt"}"#),
        indexed_tool_stop(1),
        indexed_tool_stop(0),
        message_delta(),
    ])
    .await?;
    let calls = requests(&items);
    assert_eq!(
        calls
            .iter()
            .map(|call| call.id.as_str())
            .collect::<Vec<_>>(),
        vec!["toolu_first_reverse", "toolu_second_reverse"]
    );
    assert!(calls.iter().all(|call| call.tool_call.is_ok()));
    Ok(())
}

#[tokio::test]
async fn anthropic_stream_boundary_duplicate_ids_keep_index_scoped_state() -> anyhow::Result<()> {
    let items = decode(vec![
        indexed_tool_start(0, "toolu_duplicate", "developer__shell"),
        indexed_tool_start(1, "toolu_duplicate", "developer__text_editor"),
        indexed_args_delta(0, r#"{"command":"printf first"}"#),
        indexed_args_delta(1, r#"{"path":"synthetic.txt"}"#),
        indexed_tool_stop(0),
        indexed_tool_stop(1),
        message_delta(),
    ])
    .await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 2);
    let first = calls[0]
        .tool_call
        .as_ref()
        .expect("first duplicate-id call");
    assert_eq!(first.name.as_ref(), "developer__shell");
    assert_eq!(
        first
            .arguments
            .as_ref()
            .and_then(|arguments| arguments.get("command"))
            .and_then(Value::as_str),
        Some("printf first")
    );
    let second = calls[1]
        .tool_call
        .as_ref()
        .expect("second duplicate-id call");
    assert_eq!(second.name.as_ref(), "developer__text_editor");
    assert_eq!(
        second
            .arguments
            .as_ref()
            .and_then(|arguments| arguments.get("path"))
            .and_then(Value::as_str),
        Some("synthetic.txt")
    );
    Ok(())
}
