//! A **real** knowledge ingest macro, driven by a **real** Claude Code or Codex,
//! writing **real** pages through the tool bridge (#109).
//!
//! `--ignored`: both need the vendor CLI installed and signed in, and each run
//! spends the user's own plan quota.
//!
//! ```text
//! cargo test -p biorouter-server --test knowledge_macro_live_bridge -- --ignored --nocapture
//! ```
//!
//! # What only this can prove
//!
//! `crates/biorouter/tests/knowledge_macro_tool_bridge.rs` pins the wiring with a
//! stub provider: the dispatcher reaches the grant, the mirrored pairs come back
//! as records, nothing is run twice. It cannot prove the half that broke in
//! production, which is that a **real child agent**, given the macro's tools over
//! MCP and no others, chooses to call them and writes something to disk.
//!
//! That is the measured failure this issue is about. The model produced a
//! complete, correct plan with every call written out as prose, invented its own
//! `<tool_response>OK</tool_response>` replies to continue against, and the
//! knowledge base stayed empty — after a full run the user had paid for. So the
//! assertion here is deliberately about the **filesystem**, not about the reply:
//! a run that narrates perfectly and writes nothing fails.

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::Arc;

use biorouter::knowledge::provider_completer::ProviderCompleter;
use biorouter::model::ModelConfig;
use biorouter::providers::base::Provider;
use biorouter::providers::coding_agent::bridge;
use biorouter_mcp::knowledge::affiliation::CallerAffiliation;
use biorouter_mcp::knowledge::convert::SourceInput;
use biorouter_mcp::knowledge::macros::ingest::{ingest, IngestArgs};
use biorouter_mcp::knowledge::service::KnowledgeService;
use biorouter_mcp::knowledge::subagent::loop_::SubAgentBounds;

/// Serve the real bridge route on an ephemeral port and publish it.
async fn serve_real_bridge() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port");
    let addr = listener.local_addr().expect("a bound address");
    let app = biorouter_server::routes::tool_bridge::routes();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    bridge::publish_base_url(format!("http://{addr}"));
}

/// A short source with one unmistakable fact, so "did it read the source" is
/// answerable off the pages rather than inferred.
const SOURCE: &str = "\
# Tocilizumab in giant cell arteritis

Tocilizumab is an interleukin-6 receptor antagonist. In the GiACTA trial it \
sustained remission in giant cell arteritis at 52 weeks. The trial identifier \
recorded here is GIACTA-52W-XKCD.
";

/// The one string that can only come from the source.
const SOURCE_MARKER: &str = "GIACTA-52W-XKCD";

/// Run one real ingest and assert it wrote something real.
async fn ingest_under(provider: Arc<dyn Provider>, label: &str) {
    serve_real_bridge().await;

    let dir = tempfile::tempdir().expect("a temp knowledge root");
    let svc = KnowledgeService::new(dir.path().to_path_buf());
    svc.create_base("live", "Live", None)
        .expect("the base is created");

    let (completer, _tier, _affiliation) = ProviderCompleter::paired(provider);
    let result = ingest(
        &svc,
        IngestArgs {
            kb_id: "live".to_string(),
            caller_is_private: false,
            caller_affiliation: CallerAffiliation::default(),
            source: SourceInput::Text {
                text: SOURCE.to_string(),
                title: Some("Tocilizumab in GCA".to_string()),
            },
            completer: Box::new(completer.in_session("live-macro-e2e")),
            focus: None,
            bounds: SubAgentBounds {
                // Generous: a child agent runs its own multi-step loop inside ONE
                // provider call, so the wall clock here covers the whole run
                // rather than one model round trip.
                max_wall: std::time::Duration::from_secs(600),
                ..SubAgentBounds::default()
            },
            event_sink: None,
            cancel: None,
        },
    )
    .await;

    let result = result.unwrap_or_else(|e| {
        panic!("{label}: the ingest failed: {e}");
    });

    // The pages are the assertion. A run that narrated its calls perfectly and
    // wrote nothing is exactly the failure #109 is about, and it would sail past
    // any check on the reply text.
    let knowledge = dir.path().join("live").join("knowledge");
    let mut written: Vec<String> = Vec::new();
    let mut stack = vec![knowledge.clone()];
    while let Some(path) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|e| e.to_str()) == Some("md") {
                written.push(std::fs::read_to_string(&p).unwrap_or_default());
            }
        }
    }

    assert!(
        !written.is_empty(),
        "{label}: the ingest reported success ({} step(s), commit {}) but wrote no \
         knowledge pages — which is precisely the shape of the bug: the model was \
         handed no usable tools and narrated its calls instead",
        result.steps,
        result.commit_sha
    );
    assert!(
        written.iter().any(|page| page.contains(SOURCE_MARKER)),
        "{label}: {} page(s) were written but none carries the one fact that only \
         the source contains ({SOURCE_MARKER}), so the child did not actually read \
         it through the bridge",
        written.len()
    );
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "needs the `claude` CLI installed and signed in; spends the user's own plan quota"]
async fn a_real_ingest_macro_runs_on_claude_code() {
    use biorouter::providers::claude_code::ClaudeCodeProvider;
    // Haiku 4.5: the cheapest advertised model, served by both CLI generations
    // on this machine (2.1.235 and 2.1.260) with no version floor of its own.
    let provider =
        ClaudeCodeProvider::from_env(ModelConfig::new("claude-haiku-4-5").expect("a known model"))
            .await
            .expect("the claude CLI is installed");
    ingest_under(Arc::new(provider), "claude_code").await;
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "needs the `codex` CLI installed and signed in; spends the user's own plan quota"]
async fn a_real_ingest_macro_runs_on_codex() {
    use biorouter::providers::codex::CodexProvider;
    let provider = CodexProvider::from_env(ModelConfig::new("gpt-5.5").expect("a known model"))
        .await
        .expect("the codex CLI is installed");
    ingest_under(Arc::new(provider), "codex").await;
}
