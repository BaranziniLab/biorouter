biorouter::keep_this_test_binary_off_the_os_credential_store!();

use biorouter::conversation::message::{MessageContent, ToolRequest};
use biorouter::providers::base::ProviderStreamItem;
use biorouter::providers::formats::openai_responses::responses_api_to_streaming_message;
use futures::{pin_mut, StreamExt};
use serde_json::{json, Value};

const RAW_SENTINEL: &str = "OPENAI_RESPONSES_RAW_ARGUMENT_SENTINEL";

fn data(event: Value) -> String {
    format!("data: {event}")
}

fn function_item(status: &str, arguments: &str) -> Value {
    function_item_named(
        "fc_item",
        status,
        "call_responses",
        "developer__shell",
        arguments,
    )
}

fn function_item_named(
    item_id: &str,
    status: &str,
    call_id: &str,
    name: &str,
    arguments: &str,
) -> Value {
    json!({
        "type": "function_call",
        "id": item_id,
        "status": status,
        "call_id": call_id,
        "name": name,
        "arguments": arguments
    })
}

fn item_added() -> String {
    data(json!({
        "type": "response.output_item.added",
        "sequence_number": 1,
        "output_index": 0,
        "item": function_item("in_progress", "")
    }))
}

fn arguments_delta(fragment: &str) -> String {
    data(json!({
        "type": "response.function_call_arguments.delta",
        "sequence_number": 2,
        "item_id": "fc_item",
        "output_index": 0,
        "delta": fragment
    }))
}

fn arguments_done(arguments: &str) -> String {
    data(json!({
        "type": "response.function_call_arguments.done",
        "sequence_number": 3,
        "item_id": "fc_item",
        "output_index": 0,
        "arguments": arguments
    }))
}

fn item_done(status: &str, arguments: &str) -> String {
    data(json!({
        "type": "response.output_item.done",
        "sequence_number": 4,
        "output_index": 0,
        "item": function_item(status, arguments)
    }))
}

fn response_completed(output: Vec<Value>) -> String {
    data(json!({
        "type": "response.completed",
        "sequence_number": 5,
        "response": {
            "id": "resp_boundary",
            "object": "response",
            "created_at": 1,
            "status": "completed",
            "model": "gpt-5.6",
            "output": output,
            "usage": {"input_tokens": 10, "output_tokens": 4, "total_tokens": 14}
        }
    }))
}

async fn decode(lines: Vec<String>) -> anyhow::Result<Vec<ProviderStreamItem>> {
    let stream = responses_api_to_streaming_message(tokio_stream::iter(lines.into_iter().map(Ok)));
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
async fn openai_responses_stream_boundary_item_done_confirms_complete_call_without_response_done(
) -> anyhow::Result<()> {
    let items = decode(vec![
        item_added(),
        arguments_delta(r#"{"command":"printf "#),
        arguments_delta(r#"complete"}"#),
        arguments_done(r#"{"command":"printf complete"}"#),
        item_done("completed", r#"{"command":"printf complete"}"#),
    ])
    .await?;
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
async fn openai_responses_stream_boundary_accepts_no_space_data_fields() -> anyhow::Result<()> {
    let items = decode(vec![
        item_done("completed", "{}").replacen("data: ", "data:", 1)
    ])
    .await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    assert!(calls[0].tool_call.is_ok());
    Ok(())
}

#[tokio::test]
async fn openai_responses_stream_boundary_rejects_malformed_completed_arguments_without_raw_input(
) -> anyhow::Result<()> {
    let items = decode(vec![item_done(
        "completed",
        &format!(r#"{{"command":"{RAW_SENTINEL}""#),
    )])
    .await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    assert_failure(calls[0], "invalid_arguments");
    Ok(())
}

async fn assert_completed_non_object_fails(arguments: &str) -> anyhow::Result<()> {
    let items = decode(vec![item_done("completed", arguments)]).await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1, "arguments: {arguments}");
    assert_failure(calls[0], "invalid_arguments");
    Ok(())
}

#[tokio::test]
async fn openai_responses_stream_boundary_rejects_completed_array_arguments() -> anyhow::Result<()>
{
    assert_completed_non_object_fails("[]").await
}

#[tokio::test]
async fn openai_responses_stream_boundary_rejects_completed_null_arguments() -> anyhow::Result<()> {
    assert_completed_non_object_fails("null").await
}

#[tokio::test]
async fn openai_responses_stream_boundary_rejects_completed_scalar_arguments() -> anyhow::Result<()>
{
    assert_completed_non_object_fails("42").await
}

#[tokio::test]
async fn openai_responses_stream_boundary_rejects_completed_string_arguments() -> anyhow::Result<()>
{
    assert_completed_non_object_fails(r#""text""#).await
}

#[tokio::test]
async fn openai_responses_stream_boundary_rejects_added_call_at_eof() -> anyhow::Result<()> {
    let items = decode(vec![
        item_added(),
        arguments_delta(r#"{"command":"partial""#),
    ])
    .await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    assert_failure(calls[0], "incomplete_stream");
    Ok(())
}

#[tokio::test]
async fn openai_responses_stream_boundary_arguments_done_without_item_done_is_incomplete(
) -> anyhow::Result<()> {
    let items = decode(vec![
        item_added(),
        arguments_done(r#"{"command":"printf never-ran"}"#),
        "data: [DONE]".to_string(),
    ])
    .await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    assert_failure(calls[0], "incomplete_stream");
    Ok(())
}

#[tokio::test]
async fn openai_responses_stream_boundary_response_completed_does_not_replace_missing_item_done(
) -> anyhow::Result<()> {
    let items = decode(vec![response_completed(vec![function_item(
        "completed",
        r#"{"command":"printf never-ran"}"#,
    )])])
    .await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    assert_failure(calls[0], "incomplete_stream");
    Ok(())
}

#[tokio::test]
async fn openai_responses_stream_boundary_item_done_with_unfinished_status_is_not_callable(
) -> anyhow::Result<()> {
    let items = decode(vec![item_done("in_progress", "{}")]).await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    assert_failure(calls[0], "incomplete_stream");
    Ok(())
}

#[tokio::test]
async fn openai_responses_stream_boundary_malformed_event_does_not_echo_payload() {
    let error = decode(vec![
        item_added(),
        format!("data: {{not-json:{RAW_SENTINEL}"),
    ])
    .await
    .expect_err("a malformed SSE event must fail the decoder");
    assert!(
        !format!("{error:#}").contains(RAW_SENTINEL),
        "the decoder error disclosed the malformed payload"
    );
}

#[tokio::test]
async fn openai_responses_stream_boundary_preserves_exact_item_done_snapshot() -> anyhow::Result<()>
{
    let replacement = function_item_named(
        "fc_item",
        "completed",
        "call_replaced",
        "developer__text_editor",
        &format!(r#"{{"path":"{RAW_SENTINEL}"}}"#),
    );
    let items = decode(vec![
        item_done("completed", r#"{"command":"printf confirmed"}"#),
        response_completed(vec![replacement]),
    ])
    .await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_responses");
    let call = calls[0].tool_call.as_ref().expect("confirmed snapshot");
    assert_eq!(call.name.as_ref(), "developer__shell");
    assert_eq!(
        call.arguments
            .as_ref()
            .and_then(|arguments| arguments.get("command"))
            .and_then(Value::as_str),
        Some("printf confirmed")
    );
    Ok(())
}

#[tokio::test]
async fn openai_responses_stream_boundary_mismatched_done_id_does_not_erase_pending_call(
) -> anyhow::Result<()> {
    let mismatched_done = data(json!({
        "type": "response.output_item.done",
        "sequence_number": 4,
        "output_index": 0,
        "item": function_item_named(
            "fc_other",
            "completed",
            "call_other",
            "developer__text_editor",
            r#"{"path":"never.txt"}"#,
        )
    }));
    let items = decode(vec![
        item_added(),
        mismatched_done,
        "data: [DONE]".to_string(),
    ])
    .await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_responses");
    assert_failure(calls[0], "incomplete_stream");
    Ok(())
}

#[tokio::test]
async fn openai_responses_stream_boundary_retains_output_index_order_for_mixed_completion(
) -> anyhow::Result<()> {
    let completed_second = data(json!({
        "type": "response.output_item.done",
        "sequence_number": 4,
        "output_index": 1,
        "item": function_item_named(
            "fc_second",
            "completed",
            "call_second",
            "developer__shell",
            r#"{"command":"printf second"}"#,
        )
    }));
    let items = decode(vec![
        item_added(),
        completed_second,
        "data: [DONE]".to_string(),
    ])
    .await?;
    let calls = requests(&items);
    assert_eq!(
        calls
            .iter()
            .map(|call| call.id.as_str())
            .collect::<Vec<_>>(),
        vec!["call_responses", "call_second"]
    );
    assert_failure(calls[0], "incomplete_stream");
    assert!(calls[1].tool_call.is_ok());
    Ok(())
}

#[tokio::test]
async fn openai_responses_stream_boundary_drains_unfinished_nested_message_tool_call(
) -> anyhow::Result<()> {
    let nested_added = data(json!({
        "type": "response.output_item.added",
        "sequence_number": 1,
        "output_index": 0,
        "item": {
            "type": "message",
            "id": "msg_nested",
            "status": "in_progress",
            "role": "assistant",
            "content": [{
                "type": "tool_call",
                "id": "call_nested",
                "name": "developer__shell",
                "arguments": ""
            }]
        }
    }));
    let items = decode(vec![nested_added, "data: [DONE]".to_string()]).await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_nested");
    assert_failure(calls[0], "incomplete_stream");
    Ok(())
}

#[tokio::test]
async fn openai_responses_stream_boundary_drains_tool_call_added_as_later_content_part(
) -> anyhow::Result<()> {
    let message_added = data(json!({
        "type": "response.output_item.added",
        "sequence_number": 1,
        "output_index": 0,
        "item": {
            "type": "message",
            "id": "msg_incremental",
            "status": "in_progress",
            "role": "assistant",
            "content": []
        }
    }));
    let tool_part_added = data(json!({
        "type": "response.content_part.added",
        "sequence_number": 2,
        "item_id": "msg_incremental",
        "output_index": 0,
        "content_index": 0,
        "part": {
            "type": "tool_call",
            "id": "call_incremental",
            "name": "developer__shell",
            "arguments": ""
        }
    }));
    let items = decode(vec![
        message_added,
        tool_part_added,
        "data: [DONE]".to_string(),
    ])
    .await?;
    let calls = requests(&items);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_incremental");
    assert_failure(calls[0], "incomplete_stream");
    Ok(())
}
