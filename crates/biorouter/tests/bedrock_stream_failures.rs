#![cfg(feature = "aws-providers")]

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use aws_sdk_bedrockruntime::types::{
    ContentBlockDelta, ContentBlockDeltaEvent, ContentBlockStart, ContentBlockStartEvent,
    ContentBlockStopEvent, ConverseStreamOutput, ToolUseBlockDelta, ToolUseBlockStart,
};
use biorouter::conversation::message::MessageContent;
use biorouter::providers::formats::bedrock::BedrockStreamDecoder;
use serde_json::json;

fn events(input: &str, closed: bool) -> Vec<ConverseStreamOutput> {
    let mut events = vec![
        ConverseStreamOutput::ContentBlockStart(
            ContentBlockStartEvent::builder()
                .content_block_index(0)
                .start(ContentBlockStart::ToolUse(
                    ToolUseBlockStart::builder()
                        .tool_use_id("synthetic_call")
                        .name("developer__text_editor")
                        .build()
                        .unwrap(),
                ))
                .build()
                .unwrap(),
        ),
        ConverseStreamOutput::ContentBlockDelta(
            ContentBlockDeltaEvent::builder()
                .content_block_index(0)
                .delta(ContentBlockDelta::ToolUse(
                    ToolUseBlockDelta::builder().input(input).build().unwrap(),
                ))
                .build()
                .unwrap(),
        ),
    ];
    if closed {
        events.push(ConverseStreamOutput::ContentBlockStop(
            ContentBlockStopEvent::builder()
                .content_block_index(0)
                .build()
                .unwrap(),
        ));
    }
    events
}

#[test]
fn missing_block_stop_classifies_completion_not_json_validity() {
    for input in [r#"{"path":"/tmp/synthetic.py"}"#, r#"{"path":""#] {
        let mut decoder = BedrockStreamDecoder::new("synthetic-claude");
        let mut items = Vec::new();
        for event in events(input, false) {
            items.extend(decoder.on_event(&event));
        }
        items.extend(decoder.finish());
        let calls = items
            .iter()
            .filter_map(|(message, _)| message.as_ref())
            .flat_map(|message| &message.content)
            .filter_map(|content| match content {
                MessageContent::ToolRequest(request) => Some(request),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 1);
        let error = calls[0]
            .tool_call
            .as_ref()
            .expect_err("unconfirmed block must not execute");
        assert_eq!(
            error.data,
            Some(json!({"biorouterToolCallFailure":"incomplete_stream"}))
        );
        assert!(error.message.contains("completion"));
        assert!(!error.message.contains(input));
    }
}

#[test]
fn completed_invalid_arguments_have_safe_machine_readable_classification() {
    for input in ["[]", "{\"private_marker\":"] {
        let mut decoder = BedrockStreamDecoder::new("synthetic-claude");
        let mut items = Vec::new();
        for event in events(input, true) {
            items.extend(decoder.on_event(&event));
        }
        items.extend(decoder.finish());
        let call = items
            .iter()
            .filter_map(|(message, _)| message.as_ref())
            .flat_map(|message| &message.content)
            .find_map(|content| match content {
                MessageContent::ToolRequest(request) => Some(request),
                _ => None,
            })
            .expect("invalid call surfaced");
        let error = call
            .tool_call
            .as_ref()
            .expect_err("invalid arguments must not execute");
        assert_eq!(
            error.data,
            Some(json!({"biorouterToolCallFailure":"invalid_arguments"}))
        );
        assert!(!error.message.contains("private_marker"));
    }
}
