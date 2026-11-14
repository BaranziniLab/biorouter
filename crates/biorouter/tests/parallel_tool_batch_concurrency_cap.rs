//! PAR-05a — the 8-permit dispatch semaphore actually bounds a wide batch.
//!
//! `tool_dispatch_limits` caps concurrent tool futures at
//! `DEFAULT_MAX_CONCURRENT_TOOLS` (8). The unit tests cover the permit
//! arithmetic; this one measures the cap on the REAL dispatch path, because the
//! permit is acquired inside the tool future (`agent.rs`) and a refactor that
//! moved or dropped that acquisition would leave the unit tests green.
//!
//! Each of 12 tools marks itself live, samples how many tools are live at that
//! moment, then clears its mark. The peak sample is the observed concurrency.
//! Every tool samples into its OWN file, so the per-path write locks never
//! serialize the probes and confound the measurement.
//!
//! Its own test binary: it asserts against the compiled default, so it must not
//! share a process with the `BIOROUTER_TOOL_MAX_CONCURRENT` override test.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

mod parallel_batch_support;

use parallel_batch_support::{agent_with_batch, drain, tool_responses};
use tempfile::TempDir;

/// A shell command that marks itself live, samples the live count into its own
/// file, holds the mark long enough to overlap with its siblings, then clears it.
fn probe(root: &std::path::Path, i: usize) -> String {
    format!(
        "touch {root}/live-{i} && sleep 0.15 && \
         ls {root}/live-* | wc -l > {root}/count-{i} && \
         sleep 0.15 && rm -f {root}/live-{i} && echo probe-{i}",
        root = root.display(),
        i = i
    )
}

/// The highest live-count any probe observed.
fn peak_observed(root: &std::path::Path, n: usize) -> usize {
    (0..n)
        .filter_map(|i| std::fs::read_to_string(root.join(format!("count-{i}"))).ok())
        .filter_map(|s| s.trim().parse::<usize>().ok())
        .max()
        .expect("at least one probe recorded a live count")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_wide_batch_never_exceeds_the_eight_permit_ceiling() {
    let scratch = TempDir::new().unwrap();
    let root = scratch.path().to_path_buf();

    // Comfortably more tools than permits, so the cap has to bind.
    const N: usize = 12;
    let batch: Vec<(String, String)> = (0..N)
        .map(|i| (format!("call_{i}"), probe(&root, i)))
        .collect();

    let (agent, session_id, _work) = agent_with_batch(batch).await;
    let streamed = drain(&agent, &session_id).await.unwrap();
    assert_eq!(
        tool_responses(&streamed).len(),
        N,
        "every tool in the batch must still complete under the cap"
    );

    let peak = peak_observed(&root, N);

    // The cap holds: never more than 8 tools in flight at once.
    assert!(
        peak <= 8,
        "observed {peak} tools running concurrently, above the 8-permit ceiling. \
         the dispatch semaphore is not bounding the batch"
    );

    // And the cap is a ceiling, not a serializer: the batch really did run in
    // parallel. Without this, a change that accidentally serialized every tool
    // would still satisfy the assertion above.
    assert!(
        peak > 1,
        "observed no parallelism at all (peak {peak}): the batch serialized, so \
         this run did not actually exercise the concurrency ceiling"
    );
}
