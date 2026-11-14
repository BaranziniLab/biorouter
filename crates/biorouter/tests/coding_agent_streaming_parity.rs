//! **The parity gate.** A coding-agent turn must reach the transcript in the
//! same shape an API-provider turn does, because that shape is the entire
//! contract the GUI renders from.
//!
//! The GUI has no provider-specific branch for tool cards:
//! `ToolCallWithResponse` pairs a `ToolRequest` with the `ToolResponse` carrying
//! the same id, reads the name out of the request, colours the card from
//! `isError` on the result, and expands the arguments and the output. So "the
//! same thing the user sees" means, precisely: the same sequence of message
//! shapes, with the same pairing, the same names, and the same success/failure
//! signal.
//!
//! ⚠ **The comparison is only worth anything if one side is real.** An earlier
//! version of this file built BOTH sides by hand — the mirrored side by calling
//! `mirror::request_message` directly — so the two agreed by construction and
//! the test could not fail for any bug in the decoders or the providers. It has
//! been rewritten to drive **recorded vendor frames** through the real
//! `ClaudeCodeProvider::stream`, via a fake `claude` that replays a fixture, and
//! to compare the result against a hand-written API-provider turn. The
//! hand-written side is the specification; the recorded side is the thing under
//! test.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use biorouter::conversation::message::{Message, MessageContent};
use biorouter::providers::base::Provider;
use biorouter::providers::coding_agent::mirror::{self, Execution};

/// The user-visible skeleton of one turn.
///
/// Deliberately *not* the messages themselves: two turns that render
/// identically may differ in ids, timestamps and how text was chunked, and a
/// comparison sensitive to those would fail for reasons no user could see.
#[derive(Debug, PartialEq, Eq)]
struct Shape {
    /// Did the turn produce assistant prose at all?
    has_text: bool,
    /// Tool cards, in order: (tool name, ok/failed).
    cards: Vec<(String, bool)>,
    /// Every card's request has a response with the same id.
    every_card_settles: bool,
}

/// Build the shape from whatever a provider yielded.
fn shape_of(messages: &[Message]) -> Shape {
    let mut has_text = false;
    let mut requests: Vec<(String, String)> = Vec::new();
    let mut responses: Vec<(String, bool)> = Vec::new();

    for message in messages {
        for content in &message.content {
            match content {
                MessageContent::Text(t) if !t.text.trim().is_empty() => has_text = true,
                MessageContent::ToolRequest(r) => {
                    let name = r
                        .tool_call
                        .as_ref()
                        .map(|c| c.name.to_string())
                        .unwrap_or_default();
                    requests.push((r.id.clone(), name));
                }
                MessageContent::ToolResponse(r) => {
                    let failed = r
                        .tool_result
                        .as_ref()
                        .ok()
                        .and_then(|v| v.is_error)
                        .unwrap_or(false);
                    responses.push((r.id.clone(), failed));
                }
                _ => {}
            }
        }
    }

    let every_card_settles = requests
        .iter()
        .all(|(id, _)| responses.iter().any(|(rid, _)| rid == id));
    let cards = requests
        .iter()
        .map(|(id, name)| {
            let failed = responses
                .iter()
                .find(|(rid, _)| rid == id)
                .map(|(_, failed)| *failed)
                .unwrap_or(false);
            (name.clone(), failed)
        })
        .collect();

    Shape {
        has_text,
        cards,
        every_card_settles,
    }
}

/// What an **API provider** produces for the same conversation: the loop
/// dispatches the request and appends the response it got back.
///
/// Written by hand rather than replayed, because that is the reference the
/// coding-agent path has to match, and hand-writing it is what makes the
/// expected shape explicit instead of circular.
fn api_provider_turn(tool: &str, failed: bool) -> Vec<Message> {
    let call = rmcp::model::CallToolRequestParams {
        name: tool.to_string().into(),
        arguments: Some(rmcp::model::object(serde_json::json!({ "command": "ls" }))),
        meta: None,
        task: None,
    };
    let request = Message::assistant().with_tool_request("call_1", Ok(call));

    let body = vec![rmcp::model::Content::text("a.txt")];
    let result = if failed {
        rmcp::model::CallToolResult::error(body)
    } else {
        rmcp::model::CallToolResult::success(body)
    };
    let response = Message::user().with_tool_response("call_1", Ok(result));

    vec![
        request,
        response,
        Message::assistant().with_text("Listed the directory."),
    ]
}

/// The equivalent turn as the **coding-agent** path really produces it: a fake
/// `claude` replays a recorded fixture cell through the actual provider.
///
/// This is what makes the comparison meaningful — everything from argv
/// construction through the line router, the reused Anthropic decoder and the
/// mirror is exercised.
#[cfg(unix)]
async fn mirrored_turn_from_fixture(cell: &str) -> Vec<Message> {
    use std::os::unix::fs::PermissionsExt;

    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/coding_agent/claude")
        .join(format!("{cell}.ndjson"));
    assert!(fixture.exists(), "missing fixture: {}", fixture.display());

    // ⚠ `fs::write` into a directory, never a `NamedTempFile`: the latter keeps
    // the file open read-write, and Linux refuses to `exec` a file open for
    // writing (`ETXTBSY` / "Text file busy"). macOS allows it, so this fails
    // only on the Linux CI job.
    let dir = tempfile::tempdir().expect("temp dir");
    let script = dir.path().join("fake-claude");
    std::fs::write(
        &script,
        format!("#!/bin/sh\ncat {}\ncat > /dev/null\n", fixture.display()),
    )
    .expect("write the fake CLI");
    let mut perms = std::fs::metadata(&script).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&script, perms).unwrap();

    let provider = biorouter::providers::claude_code::ClaudeCodeProvider::for_tests(
        script.clone(),
        "claude-sonnet-4-6",
    );

    let mut stream = provider
        .stream("SYS", &[Message::user().with_text("hello")], &[])
        .await
        .expect("the stream should open");

    let mut messages = Vec::new();
    while let Some(item) = futures::StreamExt::next(&mut stream).await {
        let (message, _, _) = item.expect("no item should error");
        if let Some(message) = message {
            messages.push(message);
        }
    }
    messages
}

/// A tool call the child made renders like an API provider's.
///
/// The recorded `turn-tools` cell contains two real `Bash` calls with real
/// `tool_result` blocks; the specification side describes one successful call.
/// The comparison is on the *card shape* — name present, pairing intact, not
/// failed — rather than on the count, because the two turns are different
/// conversations.
#[cfg(unix)]
#[tokio::test]
async fn a_recorded_tool_turn_produces_api_provider_shaped_cards() {
    let spec = shape_of(&api_provider_turn("developer__shell", false));
    let real = shape_of(&mirrored_turn_from_fixture("turn-tools").await);

    assert!(real.has_text, "the turn's prose must reach the transcript");
    assert!(
        !real.cards.is_empty(),
        "the child's tool calls must be visible as cards"
    );
    assert!(
        real.every_card_settles,
        "every card must have a matching response, or it spins forever — the \
         specification turn does: {:?}",
        spec.cards
    );
    for (name, failed) in &real.cards {
        assert!(
            !name.is_empty(),
            "a card with no tool name renders as a blank row"
        );
        assert!(
            !name.starts_with("mcp__biorouter__"),
            "the card must show the tool name the user knows, not the child's \
             MCP-namespaced spelling (got {name})"
        );
        assert!(!failed, "the recorded calls all succeeded");
    }
}

/// A failed call must reach the card as a failure — the signal that colours it
/// red. Driven from the recorded `turn-tool-error` cell, where `is_error` sits
/// on the `tool_result` block.
#[cfg(unix)]
#[tokio::test]
async fn a_recorded_failed_call_is_shaped_like_an_api_provider_failure() {
    let spec = shape_of(&api_provider_turn("developer__shell", true));
    assert!(
        spec.cards.iter().any(|(_, failed)| *failed),
        "the specification turn is a failing one"
    );

    let real = shape_of(&mirrored_turn_from_fixture("turn-tool-error").await);
    assert!(
        real.cards.iter().any(|(_, failed)| *failed),
        "the recorded failure must survive as a failed card (got {:?})",
        real.cards
    );
    assert!(real.every_card_settles);
}

/// The one difference that must exist: only the mirrored side is marked, and the
/// marker is what stops the loop dispatching a call that already ran.
///
/// Without this, the two paths could be made "identical" by dropping the
/// marker — which would satisfy every shape comparison above and reintroduce
/// double execution.
#[cfg(unix)]
#[tokio::test]
async fn only_the_mirrored_turn_carries_the_marker() {
    let api = api_provider_turn("developer__shell", false);
    assert!(
        !api.iter().any(mirror::contains_provider_executed),
        "an API provider's pair must never look mirrored, or the loop would stop \
         dispatching real tool calls"
    );

    let real = mirrored_turn_from_fixture("turn-tools").await;
    assert!(
        real.iter().any(mirror::contains_provider_executed),
        "the recorded turn's cards must be marked, or the loop would run the \
         child's calls a second time"
    );
    for message in &real {
        for content in &message.content {
            if let MessageContent::ToolRequest(r) = content {
                assert_eq!(
                    mirror::request_execution(r),
                    Some(Execution::Bridged),
                    "every mirrored request must carry the marker"
                );
            }
        }
    }
}
