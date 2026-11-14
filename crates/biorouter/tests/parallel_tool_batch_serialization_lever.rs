//! PAR-05b — `BIOROUTER_TOOL_MAX_CONCURRENT=1` still fully serializes a batch.
//!
//! This is the documented rollback lever for the whole §6.2 parallel-dispatch
//! change: if parallel tool execution ever misbehaves in the field, setting the
//! variable to 1 must restore strictly one-tool-at-a-time behaviour without a
//! rebuild. A lever nobody tests is a lever that does not work, so this gate
//! drives the real dispatch path with it set and asserts NO two tools overlap.
//!
//! Its own test binary for two reasons: the variable is process-global, and the
//! semaphore behind it is a `LazyLock` built on FIRST acquisition — so the value
//! has to be in the environment before any tool in the process dispatches. That
//! is a real operational constraint, not just a test artifact: setting the
//! variable on a *running* daemon has no effect, it must be set at start-up.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

mod parallel_batch_support;

use parallel_batch_support::{agent_with_batch, drain, tool_responses};
use tempfile::TempDir;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn max_concurrent_one_serializes_the_whole_batch() {
    // MUST precede any dispatch in this process — see the module comment.
    // SAFETY: single-threaded point in the test binary, before any agent work.
    unsafe {
        std::env::set_var("BIOROUTER_TOOL_MAX_CONCURRENT", "1");
    }

    let scratch = TempDir::new().unwrap();
    let root = scratch.path().to_path_buf();

    // Same probe shape as the concurrency-cap gate: mark live, sample the live
    // count into a private file, clear the mark. Under a 1-permit semaphore
    // every sample must read exactly 1.
    const N: usize = 6;
    let batch: Vec<(String, String)> = (0..N)
        .map(|i| {
            (
                format!("call_{i}"),
                format!(
                    "touch {root}/live-{i} && sleep 0.05 && \
                     ls {root}/live-* | wc -l > {root}/count-{i} && \
                     sleep 0.05 && rm -f {root}/live-{i} && echo probe-{i}",
                    root = root.display(),
                    i = i
                ),
            )
        })
        .collect();

    let (agent, session_id, _work) = agent_with_batch(batch).await;
    let streamed = drain(&agent, &session_id).await.unwrap();

    // Serializing must not drop work: every tool still runs.
    assert_eq!(
        tool_responses(&streamed).len(),
        N,
        "serializing the batch must not lose any tool"
    );

    for i in 0..N {
        let path = root.join(format!("count-{i}"));
        let observed: usize = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("probe {i} never sampled ({}): {e}", path.display()))
            .trim()
            .parse()
            .expect("probe wrote a number");
        assert_eq!(
            observed, 1,
            "probe {i} saw {observed} tools running at once; \
             BIOROUTER_TOOL_MAX_CONCURRENT=1 did not serialize the batch, so the \
             documented rollback lever no longer works"
        );
    }
}
