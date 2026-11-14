//! PAR-06 — `BIOROUTER_TOOL_RESPONSE_STREAMING=0` restores pre-§6.2c ordering.
//!
//! §6.2c moved tool-response emission into the execution loop, so the live
//! transcript surfaces each result in COMPLETION order. The kill switch is the
//! documented rollback: with it off, every response is yielded from the
//! post-batch loop in REQUEST order again.
//!
//! `streaming_tool_response_ordering.rs` gates the switch's ON behaviour (fast
//! result streams before its slow sibling). This gate is the mirror image, and
//! together they prove the flag actually selects between two behaviours rather
//! than being dead config.
//!
//! Either way the PERSISTED order must stay REQUEST order — that invariant is
//! not the flag's to change, so it is asserted here too.
//!
//! Its own test binary: the variable is process-global and read per dispatch, so
//! a sibling test in the same process would race it.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

mod parallel_batch_support;

use parallel_batch_support::{agent_with_batch, drain, persisted_tool_blocks, tool_responses};

const SLOW_ID: &str = "call_slow";
const FAST_ID: &str = "call_fast";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn streaming_disabled_yields_responses_in_request_order() {
    // SAFETY: set before any agent work in this single-test binary.
    unsafe {
        std::env::set_var("BIOROUTER_TOOL_RESPONSE_STREAMING", "0");
    }

    // Request order [slow, fast]; completion order [fast, slow]. With streaming
    // ON the transcript would read fast-then-slow; with it OFF it must read
    // slow-then-fast.
    let batch = vec![
        (SLOW_ID.to_string(), "sleep 0.4 && echo slow".to_string()),
        (FAST_ID.to_string(), "sleep 0.02 && echo fast".to_string()),
    ];

    let (agent, session_id, _work) = agent_with_batch(batch).await;
    let streamed = drain(&agent, &session_id).await.unwrap();

    let streamed_ids: Vec<String> = tool_responses(&streamed)
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    assert_eq!(
        streamed_ids,
        vec![SLOW_ID.to_string(), FAST_ID.to_string()],
        "with BIOROUTER_TOOL_RESPONSE_STREAMING=0 the streamed transcript must fall \
         back to REQUEST order (slow, fast); completion order means the kill switch \
         no longer disables per-tool emission"
    );

    // Exactly one response per call — the rollback path must not double-emit the
    // responses the execution loop would otherwise have streamed.
    assert_eq!(
        streamed_ids.len(),
        2,
        "expected exactly one response per tool with streaming disabled, got \
         {streamed_ids:?}"
    );

    // The persistence invariant is independent of the flag.
    let blocks = persisted_tool_blocks(&agent, &session_id).await;
    let expected: Vec<(&str, String)> = [SLOW_ID, FAST_ID]
        .iter()
        .flat_map(|id| [("req", id.to_string()), ("resp", id.to_string())])
        .collect();
    assert_eq!(
        blocks, expected,
        "persisted order must stay request-ordered tool_use/tool_result pairs \
         regardless of the streaming flag"
    );
}
