//! End-to-end integration test: ingest → query → lint
//!
//! Exercises the full macro pipeline against a real `KnowledgeService` backed by
//! a temp directory, with a local `MockCompleter` providing canned LLM replies.
//! No network calls or real LLM provider is required.

use anyhow::Result;
use async_trait::async_trait;
use biorouter_mcp::knowledge::{
    convert::SourceInput,
    macros::{
        ingest::{ingest, IngestArgs},
        lint,
        query::{query, QueryArgs},
    },
    page_fixtures::valid_page,
    service::KnowledgeService,
    subagent::loop_::{Completer, LlmMessage, LlmReply, LlmToolCall, SubAgentBounds},
    test_mode::TestModeCompleter,
};
use rmcp::model::Tool;
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Local MockCompleter — pops canned replies from a queue.
// Public only within this test binary.
// ---------------------------------------------------------------------------

/// Test-only Completer that pops canned `LlmReply` values from a queue.
struct MockCompleter {
    replies: Mutex<Vec<LlmReply>>,
}

impl MockCompleter {
    fn new(replies: Vec<LlmReply>) -> Self {
        Self {
            replies: Mutex::new(replies),
        }
    }
}

#[async_trait]
impl Completer for MockCompleter {
    async fn complete(
        &self,
        _system: &str,
        _messages: &[LlmMessage],
        _tools: &[Tool],
    ) -> Result<LlmReply> {
        let mut q = self.replies.lock().await;
        if q.is_empty() {
            panic!("MockCompleter ran out of canned replies");
        }
        Ok(q.remove(0))
    }
}

// ---------------------------------------------------------------------------
// Reply constructors
// ---------------------------------------------------------------------------

fn tool_call_reply(tool_name: &str, args: serde_json::Value) -> LlmReply {
    LlmReply {
        text: String::new(),
        tool_calls: vec![LlmToolCall {
            id: "req-e2e".into(),
            name: tool_name.to_string(),
            args,
        }],
    }
}

fn text_reply(text: &str) -> LlmReply {
    LlmReply {
        text: text.to_string(),
        tool_calls: vec![],
    }
}

// ---------------------------------------------------------------------------
// The e2e test
// ---------------------------------------------------------------------------

/// Full pipeline: ingest a text source → query it (read-only) → lint-scan.
///
/// Uses canned LLM replies so no network access is needed.
#[tokio::test]
async fn macros_e2e_ingest_query_lint() {
    // -----------------------------------------------------------------------
    // Setup
    // -----------------------------------------------------------------------
    let dir = tempfile::tempdir().unwrap();
    let svc = KnowledgeService::new(dir.path().to_path_buf());
    svc.create_base("e2e", "E2E", None).unwrap();

    // -----------------------------------------------------------------------
    // INGEST: mock agent writes one source page then returns a text-only reply.
    // -----------------------------------------------------------------------
    let ingest_completer = MockCompleter::new(vec![
        // Step 0: write the source page for the ingested content.
        tool_call_reply(
            "kb_write_page",
            serde_json::json!({
                "path": "knowledge/sources/hrv-zone2.md",
                "content": valid_page(
                    "source",
                    "HRV zone-2",
                    "HRV improves after zone-2 training. See also [[Zone-2]].",
                ),
                "commit_message": "add hrv source page"
            }),
        ),
        // Step 1: no more tool calls → NoMoreToolCalls → macro commits.
        text_reply("Ingestion complete."),
    ]);

    let r1 = ingest(
        &svc,
        IngestArgs {
            kb_id: "e2e".into(),
            // Issue #56: this file is the only out-of-`src/` constructor of the
            // macro `Args`, and `cargo test --lib` cannot see it.
            caller_is_private: false,
            caller_affiliation: Default::default(),
            source: SourceInput::Text {
                text: "HRV improves after zone-2.".into(),
                title: Some("HRV note".into()),
            },
            completer: Box::new(ingest_completer),
            focus: None,
            bounds: SubAgentBounds::default(),
            event_sink: None,
            cancel: None,
        },
    )
    .await
    .unwrap();

    assert!(
        !r1.commit_sha.is_empty(),
        "ingest must produce a commit SHA"
    );
    assert!(r1.steps >= 1, "ingest must use at least one sub-agent step");

    // Verify the source page landed on main.
    let source_page = dir.path().join("e2e/knowledge/sources/hrv-zone2.md");
    assert!(
        source_page.exists(),
        "source page must exist after ingest commit; path={}",
        source_page.display()
    );

    // -----------------------------------------------------------------------
    // QUERY: read-only → must not commit; cited_pages extracted from [[link]].
    // -----------------------------------------------------------------------
    let query_completer = MockCompleter::new(vec![
        // Step 0: search the KB.
        tool_call_reply("kb_search", serde_json::json!({ "query": "HRV zone-2" })),
        // Step 1: synthesise answer with a wiki-link citation.
        text_reply("Zone-2 training is known to improve [[HRV]] via cardiac adaptation."),
    ]);

    let r2 = query(
        &svc,
        QueryArgs {
            kb_id: "e2e".into(),
            caller_is_private: false,
            caller_affiliation: Default::default(),
            question: "Does zone-2 affect HRV?".into(),
            completer: Box::new(query_completer),
            file_as_page: false,
            bounds: SubAgentBounds::default(),
            event_sink: None,
            cancel: None,
        },
    )
    .await
    .unwrap();

    assert!(
        !r2.cited_pages.is_empty(),
        "read-only query must extract citations from [[wiki-links]]"
    );
    assert!(
        r2.cited_pages.contains(&"HRV".to_string()),
        "expected 'HRV' in cited_pages; got: {:?}",
        r2.cited_pages
    );
    assert!(
        r2.commit_sha.is_none(),
        "read-only query (file_as_page=false) must not produce a commit"
    );

    // -----------------------------------------------------------------------
    // LINT: deterministic scan — no LLM, no commit; just verify the report
    //       is structurally correct and the call doesn't panic.
    // -----------------------------------------------------------------------
    let kb_root = dir.path().join("e2e");
    let report = lint::scan(&kb_root).unwrap();

    // The source page references [[Zone-2]] which has no corresponding page
    // under knowledge/ → should appear in missing_concept_pages.
    assert!(
        report
            .missing_concept_pages
            .iter()
            .any(|m| m.eq_ignore_ascii_case("Zone-2")),
        "expected 'Zone-2' in missing_concept_pages; got: {:?}",
        report.missing_concept_pages
    );

    // The report fields must be accessible and well-typed (not panicking).
    let _ = &report.orphans;
    let _ = &report.contradictions;
    let _ = &report.stale_sources;
    let _ = &report.missing_concept_pages;

    // Structural sanity: the source page hrv-zone2.md has no inbound links
    // from other pages so it should appear as an orphan.
    assert!(
        report.orphans.iter().any(|o| o.contains("hrv-zone2")),
        "hrv-zone2.md has no inbound links → should be an orphan; orphans={:?}",
        report.orphans
    );
}

#[tokio::test]
async fn ingest_supported_path_formats_builds_graph_nodes() {
    let dir = tempfile::tempdir().unwrap();
    let fixture_dir = dir.path().join("fixtures");
    std::fs::create_dir_all(&fixture_dir).unwrap();

    let cases = write_supported_fixtures(&fixture_dir);

    let svc = KnowledgeService::new(dir.path().to_path_buf());
    svc.create_base("formats", "Formats", None).unwrap();

    for (index, path) in cases.iter().enumerate() {
        let result = ingest(
            &svc,
            IngestArgs {
                kb_id: "formats".into(),
                caller_is_private: false,
                caller_affiliation: Default::default(),
                source: SourceInput::Path(path.clone()),
                completer: Box::new(TestModeCompleter),
                focus: None,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: None,
            },
        )
        .await
        .unwrap();

        assert!(
            !result.commit_sha.is_empty(),
            "supported format should produce a commit"
        );

        let graph = svc.get_graph("formats").unwrap();
        assert_eq!(
            graph.nodes.len(),
            index + 1,
            "graph should include one source node per digested file"
        );
        assert!(
            !graph.nodes.iter().all(|node| node.label.trim().is_empty()),
            "graph nodes should have visible labels"
        );
    }
}

fn write_supported_fixtures(dir: &Path) -> Vec<PathBuf> {
    let markdown = dir.join("note.md");
    std::fs::write(
        &markdown,
        "# Markdown Fixture\n\nDigest this markdown note.",
    )
    .unwrap();

    let text = dir.join("note.txt");
    std::fs::write(&text, "Plain text fixture for ingestion.").unwrap();

    let csv = dir.join("table.csv");
    std::fs::write(&csv, "name,score\nAlice,9\nBob,7\n").unwrap();

    let html = dir.join("article.html");
    std::fs::write(
        &html,
        include_bytes!("../src/knowledge/convert/fixtures/article.html"),
    )
    .unwrap();

    let pdf = dir.join("sample.pdf");
    std::fs::write(
        &pdf,
        include_bytes!("../src/webdocuments/tests/data/test.pdf"),
    )
    .unwrap();

    let docx = dir.join("sample.docx");
    std::fs::write(
        &docx,
        include_bytes!("../src/webdocuments/tests/data/sample.docx"),
    )
    .unwrap();

    vec![markdown, text, csv, html, pdf, docx]
}
