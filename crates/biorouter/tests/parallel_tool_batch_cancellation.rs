//! PAR-04 — cancelling a turn in the MIDDLE of a parallel tool batch.
//!
//! The batch execution loop breaks out of `select_all` the moment the cancel
//! token trips (`agent.rs`, the `is_token_cancelled` check inside the result
//! loop), abandoning any tool that has not yet returned. The post-batch loop
//! then still persists a `tool_use` + response pair for EVERY request in the
//! batch, including the abandoned ones — whose response slot is the empty
//! placeholder `Message` allocated up front.
//!
//! That is the half-persist hazard this gate pins. The invariant is not "every
//! tool finishes" (cancelling is allowed to abandon work); it is that whatever
//! is written to the session must stay REPLAYABLE:
//!
//!   * no `tool_use` may be persisted without a matching `tool_result`, because
//!     a provider replaying the session rejects an unmatched block outright;
//!   * no tool may be recorded twice;
//!   * the turn must actually end rather than hanging on the abandoned tool.
//!
//! A separate binary from the other batch gates because it drives its own
//! cancellation token and timing.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

mod parallel_batch_support;

use std::collections::HashSet;
use std::time::Duration;

use parallel_batch_support::{agent_with_batch, drain_with_token, persisted_tool_blocks};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_mid_batch_leaves_no_unmatched_tool_use_persisted() {
    let scratch = TempDir::new().unwrap();
    let root = scratch.path().to_path_buf();

    // One tool finishes almost immediately, two are still running when the
    // cancel lands — so the batch is genuinely cut in half.
    let batch = vec![
        (
            "call_quick".to_string(),
            format!("echo quick > {}/quick.log && echo quick", root.display()),
        ),
        (
            "call_slow_a".to_string(),
            format!("sleep 5 && echo slow-a > {}/slow-a.log", root.display()),
        ),
        (
            "call_slow_b".to_string(),
            format!("sleep 5 && echo slow-b > {}/slow-b.log", root.display()),
        ),
    ];

    let (agent, session_id, _work) = agent_with_batch(batch).await;

    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        // Long enough for the quick tool to land and the two slow ones to be
        // in flight; far short of their 5s runtime.
        tokio::time::sleep(Duration::from_millis(400)).await;
        trigger.cancel();
    });

    // The turn must END. If cancellation left the loop awaiting an abandoned
    // tool, this times out well before the 5s sleeps could mask it.
    let streamed = tokio::time::timeout(
        Duration::from_secs(4),
        drain_with_token(&agent, &session_id, Some(cancel)),
    )
    .await
    .expect("a cancelled turn must end promptly, not wait out the abandoned tools")
    .expect("the cancelled turn must end cleanly rather than erroring");

    // Sanity: cancellation really did cut the batch short — the slow tools
    // never wrote their side effects.
    assert!(
        !root.join("slow-a.log").exists() && !root.join("slow-b.log").exists(),
        "the slow tools completed, so this run never exercised mid-batch cancellation"
    );
    let _ = streamed;

    // THE INVARIANT: the persisted transcript is replayable.
    let blocks = persisted_tool_blocks(&agent, &session_id).await;

    let requested: Vec<&String> = blocks
        .iter()
        .filter(|(kind, _)| *kind == "req")
        .map(|(_, id)| id)
        .collect();
    let answered: HashSet<&String> = blocks
        .iter()
        .filter(|(kind, _)| *kind == "resp")
        .map(|(_, id)| id)
        .collect();

    for id in &requested {
        assert!(
            answered.contains(*id),
            "persisted tool_use {id} has no matching tool_result after a mid-batch \
             cancel, so replaying this session makes the provider reject the turn. \
             Persisted blocks: {blocks:?}"
        );
    }

    // No id may appear twice on either side: a cancelled tool must not be both
    // abandoned and recorded again.
    let mut seen_req = HashSet::new();
    for id in &requested {
        assert!(
            seen_req.insert(*id),
            "tool_use {id} persisted more than once: {blocks:?}"
        );
    }
    let resp_ids: Vec<&String> = blocks
        .iter()
        .filter(|(kind, _)| *kind == "resp")
        .map(|(_, id)| id)
        .collect();
    assert_eq!(
        resp_ids.len(),
        answered.len(),
        "a tool_result was persisted more than once: {blocks:?}"
    );

    // And the pairing must be positional, not merely set-equal: each tool_use is
    // immediately followed by its own tool_result.
    for pair in blocks.chunks(2) {
        assert_eq!(pair.len(), 2, "persisted blocks do not pair up: {blocks:?}");
        assert_eq!(
            (pair[0].0, pair[1].0),
            ("req", "resp"),
            "expected a tool_use immediately followed by its tool_result: {blocks:?}"
        );
        assert_eq!(
            pair[0].1, pair[1].1,
            "a tool_result was persisted against the wrong tool_use: {blocks:?}"
        );
    }
}
