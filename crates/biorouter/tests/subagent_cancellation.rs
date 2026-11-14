//! SUB: parent-turn cancellation with subagents in flight (own test binary).
//!
//! The in-flight subagent counter and the concurrency semaphore are
//! process-global, so "nothing leaked" is only a truthful assertion when this is
//! the only test running. That is the same reason the parallel-tool-batch kill
//! switches each got a binary of their own.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

mod subagent_support;

use std::time::{Duration, Instant};

use subagent_support::{drain_with_token, harness, persisted_tool_blocks, Call};
use tokio_util::sync::CancellationToken;

fn call(id: &str, c: Call) -> (String, Call) {
    (id.to_string(), c)
}

/// SUB: cancelling the parent turn tears the children down rather than orphaning
/// them, and leaves no `tool_use` block without a `tool_result` at rest.
#[tokio::test]
async fn cancelling_the_parent_tears_down_its_children() {
    let h = harness(vec![
        call("quick", Call::sub("quick", "ok:quick")),
        call("slow_a", Call::sub("slow_a", "slow:slow_a:6000")),
        call("slow_b", Call::sub("slow_b", "slow:slow_b:6000")),
    ])
    .await;

    let token = CancellationToken::new();
    let canceller = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(700)).await;
        canceller.cancel();
    });

    let started = Instant::now();
    let messages = drain_with_token(&h.agent, &h.session_id, Some(token))
        .await
        .expect("a cancelled turn still ends cleanly");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(5),
        "cancel must not wait out the slow children ({elapsed:?})"
    );

    // No leaked in-flight subagents once the turn is over.

    let deadline = Instant::now() + Duration::from_secs(10);
    while biorouter::agents::subagent_tool::inflight_subagent_count() > 0
        && Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        biorouter::agents::subagent_tool::inflight_subagent_count(),
        0,
        "every subagent slot was released after the cancel"
    );

    // The session must not persist a `tool_use` with no `tool_result`: a
    // provider rejects that on the next turn (this is PAR-04's invariant, held
    // for subagents too).
    let blocks = persisted_tool_blocks(&h.agent, &h.session_id).await;
    let requests: Vec<&String> = blocks
        .iter()
        .filter(|(kind, _)| *kind == "req")
        .map(|(_, id)| id)
        .collect();
    let responses: Vec<&String> = blocks
        .iter()
        .filter(|(kind, _)| *kind == "resp")
        .map(|(_, id)| id)
        .collect();
    for id in &requests {
        assert!(
            responses.contains(id),
            "orphaned tool_use `{id}` persisted after cancel: {blocks:?}\nmessages: {}",
            messages.len()
        );
    }
}
