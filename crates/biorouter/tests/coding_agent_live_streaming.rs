//! **Live** streaming checks against the user's real, signed-in vendor CLIs.
//!
//! Every other test in this feature replays recorded frames through a fake
//! binary. That proves the pipeline handles the frames correctly — it does not
//! prove the real CLI *emits* them for the invocation Biorouter actually
//! builds. Those are separate claims, and only this file tests the second one.
//!
//! ⚠ `#[ignore]`d, because each test spends a little of the user's own
//! subscription quota. Run deliberately:
//!
//! ```text
//! cargo test -p biorouter --test coding_agent_live_streaming -- --ignored --test-threads=1
//! ```
//!
//! **What makes these tests meaningful is the timing, not the text.** A
//! blocking provider also returns the right answer eventually; what separates
//! streaming from buffering is that the FIRST piece of the answer arrives well
//! before the LAST. So each test records when each item landed and asserts a
//! real gap between them. Asserting only "some text came back" would pass
//! against the old blocking implementation and prove nothing.

use std::time::Instant;

use biorouter::conversation::message::{Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::providers::base::Provider;
use futures::StreamExt;

/// One text item and how long after the turn started it arrived.
struct Arrival {
    at_ms: u128,
    text: String,
}

/// Drive one real turn and record when each piece of text arrived.
async fn timed_turn(provider: &dyn Provider, prompt: &str) -> (Vec<Arrival>, u128) {
    let started = Instant::now();
    let messages = vec![Message::user().with_text(prompt)];

    let mut stream = provider
        .stream("You are a terse assistant. Answer briefly.", &messages, &[])
        .await
        .expect("the stream should open");

    let mut arrivals = Vec::new();
    while let Some(item) = stream.next().await {
        let (message, _usage, _pending) = item.expect("no item should error");
        let Some(message) = message else { continue };
        for content in &message.content {
            if let MessageContent::Text(t) = content {
                if !t.text.is_empty() {
                    arrivals.push(Arrival {
                        at_ms: started.elapsed().as_millis(),
                        text: t.text.clone(),
                    });
                }
            }
        }
    }
    (arrivals, started.elapsed().as_millis())
}

fn report(label: &str, arrivals: &[Arrival], total_ms: u128) {
    let joined: String = arrivals.iter().map(|a| a.text.as_str()).collect();
    eprintln!("\n=== {label} ===");
    eprintln!("text items: {}", arrivals.len());
    if let Some(first) = arrivals.first() {
        eprintln!("first text at: {} ms", first.at_ms);
    }
    if let Some(last) = arrivals.last() {
        eprintln!("last text at:  {} ms", last.at_ms);
    }
    eprintln!("turn total:    {total_ms} ms");
    eprintln!("answer: {joined:?}");
}

/// Claude Code really streams: the first token lands long before the turn ends.
#[tokio::test]
#[ignore = "spends the user's own Claude subscription quota; run deliberately"]
async fn claude_code_streams_a_real_turn() {
    let provider = biorouter::providers::claude_code::ClaudeCodeProvider::from_env(
        ModelConfig::new("claude-haiku-4-5").unwrap(),
    )
    .await
    .expect("the `claude` CLI must be installed and signed in");

    // ⚠ The prompt has to be long enough to span several chunks. Claude Code
    // batches text coarsely — a one-line answer measured as a SINGLE text_delta
    // even with `--include-partial-messages`, which makes a short prompt
    // indistinguishable from buffering and produces a false failure. A ~150-word
    // answer measured 5 deltas.
    let (arrivals, total_ms) = timed_turn(
        &provider,
        "Write a 150-word paragraph about mitochondria. Prose only, no lists.",
    )
    .await;
    report("claude_code", &arrivals, total_ms);

    assert!(
        arrivals.len() > 1,
        "the answer arrived as a single item — that is buffering, not streaming"
    );
    let first = arrivals.first().unwrap().at_ms;
    let last = arrivals.last().unwrap().at_ms;
    assert!(
        last > first,
        "every item carried the same timestamp; the turn was assembled and then \
         released at once"
    );
    assert!(
        !arrivals
            .iter()
            .map(|a| a.text.as_str())
            .collect::<String>()
            .trim()
            .is_empty(),
        "the streamed pieces must reconstruct a real answer"
    );
}

/// Codex really streams, by the same measure.
#[tokio::test]
#[ignore = "spends the user's own ChatGPT subscription quota; run deliberately"]
async fn codex_streams_a_real_turn() {
    // The small, cheap rung, to keep the quota spend low. Was `gpt-5.4-mini`,
    // which OpenAI retired on 2026-08-31 — `codex exec -m gpt-5.4-mini` now
    // fails `400 … not supported when using Codex with a ChatGPT account`, so
    // this test could no longer pass. `gpt-5.6-luna` is OpenAI's own named
    // replacement for it, and unlike `gpt-6-astra` it needs no particular CLI
    // version: it is listed by codex-cli 0.147.0 and 0.153.4 alike.
    let provider = biorouter::providers::codex::CodexProvider::from_env(
        ModelConfig::new("gpt-5.6-luna").unwrap(),
    )
    .await
    .expect("the `codex` CLI must be installed and signed in");

    let (arrivals, total_ms) = timed_turn(
        &provider,
        "Write a 150-word paragraph about mitochondria. Prose only, no lists.",
    )
    .await;
    report("codex", &arrivals, total_ms);

    assert!(
        arrivals.len() > 1,
        "the answer arrived as a single item — that is buffering, not streaming"
    );
    let first = arrivals.first().unwrap().at_ms;
    let last = arrivals.last().unwrap().at_ms;
    assert!(
        last > first,
        "every item carried the same timestamp; the turn was assembled and then \
         released at once"
    );
}
