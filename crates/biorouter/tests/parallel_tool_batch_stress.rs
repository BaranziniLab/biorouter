//! I1/PHASE-2 gates — parallel tool-call batches under stress and at the edges.
//!
//! The §6.2 work made ONE assistant message able to carry MANY tool calls that
//! are dispatched concurrently (`stream::select_all` in `agent.rs`), bounded by
//! `tool_dispatch_limits`. That path is now load-bearing for correctness, not
//! just latency, so these tests pin the invariants a regression would silently
//! break:
//!
//!   * PAR-01 a wide batch runs every tool EXACTLY once, with complete arguments,
//!     and each result maps back to its own request id;
//!   * PAR-03 one tool failing mid-batch does not cancel its siblings, surfaces as
//!     `is_error`, and the turn continues;
//!   * PAR-07 two writers to the SAME path never interleave (per-path write locks).
//!
//! Ordering (streamed = completion order, persisted = request order) is gated
//! separately by `streaming_tool_response_ordering.rs`; the env-var kill switches
//! get their own single-test binaries because the process env is global
//! (`tool_response_streaming_killswitch.rs`, `tool_max_concurrent_serialization.rs`).

biorouter::keep_this_test_binary_off_the_os_credential_store!();

mod parallel_batch_support;

use parallel_batch_support::{agent_with_batch, drain, persisted_tool_blocks, tool_responses};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// PAR-01 — a wide batch: every tool runs exactly once, results map to their own
// request id.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wide_batch_runs_every_tool_exactly_once_with_results_mapped_to_ids() {
    let scratch = TempDir::new().unwrap();
    let root = scratch.path().to_path_buf();

    // Six shells. Each APPENDS a marker to its own file, so a tool executed
    // twice leaves two lines — the exactly-once check. The `sleep`s are
    // deliberately staggered so completion order differs from request order.
    const N: usize = 6;
    let batch: Vec<(String, String)> = (0..N)
        .map(|i| {
            let sleep = 0.30 - (i as f64 * 0.04);
            (
                format!("call_{i}"),
                format!(
                    "sleep {sleep:.2} && echo marker-{i} >> {}/tool-{i}.log && echo result-{i}",
                    root.display()
                ),
            )
        })
        .collect();

    let (agent, session_id, _work) = agent_with_batch(batch).await;
    let streamed = drain(&agent, &session_id).await.unwrap();

    let responses = tool_responses(&streamed);
    assert_eq!(
        responses.len(),
        N,
        "expected one response per requested tool, got {responses:?}"
    );

    for i in 0..N {
        let id = format!("call_{i}");
        let (_, text, is_error) = responses
            .iter()
            .find(|(rid, _, _)| *rid == id)
            .unwrap_or_else(|| panic!("no tool response for {id}; got {responses:?}"));

        // The result maps to the RIGHT call: the response carried on `call_i`
        // must contain `result-i` and nothing from a sibling.
        assert!(
            text.contains(&format!("result-{i}")),
            "response for {id} does not carry its own output: {text:?}"
        );
        assert!(!is_error, "{id} unexpectedly errored: {text:?}");

        // Exactly once: one appended marker line on disk.
        let log = root.join(format!("tool-{i}.log"));
        let contents = std::fs::read_to_string(&log)
            .unwrap_or_else(|e| panic!("{id} never executed ({}): {e}", log.display()));
        assert_eq!(
            contents.lines().filter(|l| !l.trim().is_empty()).count(),
            1,
            "{id} did not execute exactly once; log = {contents:?}"
        );
    }

    // The persisted transcript must pair every tool_use with its tool_result, in
    // request order — a provider replaying it must never see an orphan block.
    let ordered = persisted_tool_blocks(&agent, &session_id).await;
    let expected: Vec<(&str, String)> = (0..N)
        .flat_map(|i| [("req", format!("call_{i}")), ("resp", format!("call_{i}"))])
        .collect();
    assert_eq!(
        ordered, expected,
        "persisted batch must be request-ordered tool_use/tool_result pairs"
    );
}

// ---------------------------------------------------------------------------
// PAR-03 — partial failure mid-batch.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failing_tool_does_not_cancel_its_siblings() {
    let scratch = TempDir::new().unwrap();
    let root = scratch.path().to_path_buf();

    // The failure sits in the MIDDLE of the batch and fails FAST, so if a
    // failure aborted the batch the two slower siblings would never land.
    let batch = vec![
        (
            "call_ok_a".to_string(),
            format!(
                "sleep 0.20 && echo ok-a > {}/a.log && echo ok-a",
                root.display()
            ),
        ),
        (
            "call_boom".to_string(),
            "echo boom-stderr 1>&2 && exit 7".to_string(),
        ),
        (
            "call_ok_b".to_string(),
            format!(
                "sleep 0.25 && echo ok-b > {}/b.log && echo ok-b",
                root.display()
            ),
        ),
    ];

    let (agent, session_id, _work) = agent_with_batch(batch).await;
    let streamed = drain(&agent, &session_id).await.unwrap();
    let responses = tool_responses(&streamed);

    assert_eq!(
        responses.len(),
        3,
        "a mid-batch failure must not swallow sibling responses: {responses:?}"
    );

    let boom = responses
        .iter()
        .find(|(id, _, _)| id == "call_boom")
        .expect("the failing tool must still produce a response");
    assert!(
        boom.2,
        "the failing tool must be flagged is_error so the UI renders it as an \
         error card rather than a green success: {boom:?}"
    );
    assert!(
        boom.1.contains("status 7"),
        "the failing tool's output must name the exit status, so the model can \
         see the failure even when the command printed nothing: {boom:?}"
    );

    for id in ["call_ok_a", "call_ok_b"] {
        let sibling = responses
            .iter()
            .find(|(rid, _, _)| rid == id)
            .unwrap_or_else(|| panic!("sibling {id} produced no response"));
        assert!(
            !sibling.2,
            "sibling {id} must succeed independently of the failing tool: {sibling:?}"
        );
    }

    // Both slow siblings really ran to completion on disk.
    for name in ["a.log", "b.log"] {
        assert!(
            root.join(name).exists(),
            "sibling side effect {name} missing, so the failure aborted the batch"
        );
    }

    // The turn continued past the failure: every one of the three calls is
    // persisted as a matched tool_use/tool_result pair, so the next provider
    // call replays a coherent transcript.
    let ordered = persisted_tool_blocks(&agent, &session_id).await;
    let expected: Vec<(&str, String)> = ["call_ok_a", "call_boom", "call_ok_b"]
        .iter()
        .flat_map(|id| [("req", id.to_string()), ("resp", id.to_string())])
        .collect();
    assert_eq!(
        ordered, expected,
        "a partial failure must leave the persisted batch complete and matched"
    );
}

// ---------------------------------------------------------------------------
// PAR-07 — same-path contention: per-path write locks.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_writers_to_the_same_path_never_interleave() {
    let scratch = TempDir::new().unwrap();
    let target = scratch.path().join("contended.txt");

    // Two shells whose redirect target is the SAME file (`write_paths_for_tool`
    // captures `>` targets, so both take the per-path exclusive lock). Each
    // writes its own 200-line block one line at a time with the file held open,
    // which is exactly the pattern that interleaves without the lock.
    let writer = |tag: &str| {
        format!(
            "for i in $(seq 1 200); do echo {tag}; done > {}",
            target.display()
        )
    };
    let batch = vec![
        ("call_writer_a".to_string(), writer("AAAA")),
        ("call_writer_b".to_string(), writer("BBBB")),
    ];

    let (agent, session_id, _work) = agent_with_batch(batch).await;
    let streamed = drain(&agent, &session_id).await.unwrap();
    assert_eq!(
        tool_responses(&streamed).len(),
        2,
        "both writers must complete"
    );

    let contents = std::fs::read_to_string(&target).expect("the contended file exists");
    let lines: Vec<&str> = contents.lines().filter(|l| !l.trim().is_empty()).collect();

    // A last-writer-wins outcome is fine and expected; a MIXED file is not — it
    // means the two writers' output interleaved inside one file.
    let distinct: std::collections::HashSet<&&str> = lines.iter().collect();
    assert_eq!(
        distinct.len(),
        1,
        "the contended file mixes both writers' output ({} distinct line values) \
         so the per-path write lock did not serialize them",
        distinct.len()
    );
    assert_eq!(
        lines.len(),
        200,
        "the contended file is truncated/doubled ({} lines), so the two writers \
         overlapped rather than running strictly one after the other",
        lines.len()
    );
}
