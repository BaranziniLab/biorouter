biorouter::keep_this_test_binary_off_the_os_credential_store!();

use biorouter::conversation::message::{Message, MessageContent};
use biorouter::providers::formats::openai::response_to_streaming_message;
use futures::StreamExt;
use serde_json::{json, Value};

fn chunk(delta: Value, finish: Option<&str>) -> String {
    format!(
        "data: {}",
        json!({"id":"issue-548-turn","object":"chat.completion.chunk","created":1,"model":"synthetic/model","choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
    )
}

async fn captured(
    before: Vec<Value>,
    after: Vec<Value>,
    parallel: bool,
) -> (Vec<Value>, Vec<Message>) {
    let mut frames = Vec::new();
    for detail in before {
        frames.push(Ok(chunk(json!({"reasoning_details":[detail]}), None)));
    }
    frames.push(Ok(chunk(json!({"tool_calls":[{"index":0,"id":"call-a","type":"function","function":{"name":"first_tool","arguments":"{"}}]}), None)));
    for detail in after {
        frames.push(Ok(chunk(json!({"reasoning_details":[detail]}), None)));
    }
    let mut calls = vec![json!({"index":0,"function":{"arguments":"}"}})];
    if parallel {
        calls.push(json!({"index":1,"id":"call-b","type":"function","function":{"name":"second_tool","arguments":"{\"b\":2}"}}));
    }
    frames.push(Ok(chunk(json!({"tool_calls":calls}), Some("tool_calls"))));
    frames.push(Ok("data: [DONE]".to_string()));
    let stream = response_to_streaming_message(futures::stream::iter(frames));
    tokio::pin!(stream);
    let mut messages = Vec::new();
    while let Some(item) = stream.next().await {
        if let (Some(message), _, _) = item.unwrap() {
            messages.push(message);
        }
    }
    let details = messages
        .iter()
        .flat_map(|message| &message.content)
        .find_map(|content| match content {
            MessageContent::ToolRequest(request) => request
                .metadata
                .as_ref()?
                .get("reasoning_details")?
                .as_array()
                .cloned(),
            _ => None,
        })
        .expect("reasoning metadata belongs to the decoded tool turn");
    (details, messages)
}

fn summary(text: &str) -> Value {
    json!({"type":"reasoning.summary","summary":text,"id":"summary-a","index":0,"format":"openai-responses-v1"})
}

fn text_detail(text: &str) -> Value {
    json!({"type":"reasoning.text","text":text,"id":"thinking-a","index":0,"format":"anthropic-claude-v1"})
}

#[tokio::test]
async fn issue_548_summary_deltas_merge_across_both_decoder_accumulation_sites() {
    let fragments: Vec<_> = (0..78).map(|n| format!("part-{n} ")).collect();
    let encrypted = json!({"type":"reasoning.encrypted","data":"AAEC/w==","id":"opaque-a","index":1,"format":"openai-responses-v1"});
    let before = fragments[..39].iter().map(|part| summary(part)).collect();
    let mut after: Vec<_> = fragments[39..].iter().map(|part| summary(part)).collect();
    after.push(encrypted.clone());
    let (details, _) = captured(before, after, false).await;
    assert_eq!(details.len(), 2, "one logical summary and one opaque entry");
    assert_eq!(details[0], summary(&fragments.concat()));
    assert_eq!(details[1], encrypted);
}

#[tokio::test]
async fn issue_548_signature_only_delta_closes_one_text_block_and_starts_the_next() {
    let first_signature = json!({"type":"reasoning.text","signature":"signature-one","id":"thinking-a","index":0,"format":"anthropic-claude-v1"});
    let second_signature = json!({"type":"reasoning.text","signature":"signature-two","id":"thinking-a","index":0,"format":"anthropic-claude-v1"});
    let (details, _) = captured(
        vec![text_detail("first "), text_detail("block")],
        vec![first_signature, text_detail("second"), second_signature],
        false,
    )
    .await;
    assert_eq!(details.len(), 2);
    let mut first = text_detail("first block");
    first["signature"] = json!("signature-one");
    let mut second = text_detail("second");
    second["signature"] = json!("signature-two");
    assert_eq!(details, vec![first, second]);
}

#[tokio::test]
async fn issue_548_equal_identity_encrypted_entries_remain_distinct_and_byte_exact() {
    let one = json!({"type":"reasoning.encrypted","data":"AAEC/w==","id":"same","index":1,"format":"openai-responses-v1"});
    let two = json!({"type":"reasoning.encrypted","data":"AQID+Q==","id":"same","index":1,"format":"openai-responses-v1"});
    let (details, _) = captured(vec![one.clone()], vec![two.clone()], false).await;
    assert_eq!(details, vec![one, two]);
}

#[tokio::test]
async fn issue_548_different_identity_or_unknown_metadata_cannot_be_discarded_by_merging() {
    let mut different_id = summary("id");
    different_id["id"] = json!("other");
    let mut different_index = summary("index");
    different_index["index"] = json!(2);
    let mut different_format = summary("format");
    different_format["format"] = json!("unknown");
    let mut extension_one = summary("extension-one");
    extension_one["vendor_extension"] = json!({"opaque":"one"});
    let mut extension_two = summary("extension-two");
    extension_two["vendor_extension"] = json!({"opaque":"two"});
    let mut different_type = summary("type");
    different_type["type"] = json!("reasoning.text");
    for changed in [
        different_id,
        different_index,
        different_format,
        different_type,
    ] {
        let expected = vec![summary("base"), changed];
        let (details, _) =
            captured(vec![expected[0].clone()], vec![expected[1].clone()], false).await;
        assert_eq!(details, expected);
    }
    let expected = vec![extension_one, extension_two];
    let (details, _) = captured(vec![expected[0].clone()], vec![expected[1].clone()], false).await;
    assert_eq!(details, expected);
}

#[tokio::test]
async fn issue_548_null_identity_and_nonstring_content_are_not_silently_normalized() {
    let absent_id = json!({"type":"reasoning.text","text":"absent","index":0,"format":"unknown"});
    let null_id =
        json!({"type":"reasoning.text","text":"null","id":null,"index":0,"format":"unknown"});
    let malformed_text = json!({"type":"reasoning.text","text":{"opaque":"value"},"id":null,"index":0,"format":"unknown"});
    let expected = vec![absent_id, null_id, malformed_text];
    let (details, _) = captured(expected.clone(), vec![], false).await;
    assert_eq!(details, expected);
}

#[tokio::test]
async fn issue_548_compact_replay_keeps_parallel_tool_identity_arguments_and_origin() {
    use biorouter::providers::formats::openrouter::{
        add_reasoning_details_to_request, stamp_reasoning_details_provenance,
    };
    let encrypted = json!({"type":"reasoning.encrypted","data":"AAEC/w==","id":"opaque-a","index":1,"format":"openai-responses-v1"});
    let (details, mut messages) = captured(
        vec![summary("one ")],
        vec![summary("two"), encrypted.clone()],
        true,
    )
    .await;
    let expected = vec![summary("one two"), encrypted];
    assert_eq!(details, expected);
    let calls: Vec<_> = messages
        .iter()
        .flat_map(|message| &message.content)
        .filter_map(|content| match content {
            MessageContent::ToolRequest(request) => Some((
                request.id.clone(),
                request.tool_call.as_ref().unwrap().name.to_string(),
                request.tool_call.as_ref().unwrap().arguments.clone(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        calls,
        vec![
            (
                "call-a".into(),
                "first_tool".into(),
                Some(serde_json::Map::new())
            ),
            (
                "call-b".into(),
                "second_tool".into(),
                Some(serde_json::from_value(json!({"b":2})).unwrap())
            )
        ]
    );
    let mut unstamped = json!({"messages":[{"role":"assistant","content":""}]});
    add_reasoning_details_to_request(&mut unstamped, &messages);
    assert!(unstamped["messages"][0].get("reasoning_details").is_none());
    for message in &mut messages {
        stamp_reasoning_details_provenance(message, "openrouter");
    }
    let mut native = json!({"messages":[{"role":"assistant","content":""}]});
    add_reasoning_details_to_request(&mut native, &messages);
    assert_eq!(native["messages"][0]["reasoning_details"], json!(expected));
    for message in &mut messages {
        stamp_reasoning_details_provenance(message, "foreign-provider");
    }
    let mut foreign = json!({"messages":[{"role":"assistant","content":""}]});
    add_reasoning_details_to_request(&mut foreign, &messages);
    assert!(foreign["messages"][0].get("reasoning_details").is_none());
}
