//! `render_dashboard` under `BIOROUTER_AUTOVIS_CDN=1`.
//!
//! This lives in its own test binary because `use_cdn()` reads a process-wide
//! environment variable: setting it inside the unit-test binary would race every
//! other figure test running in parallel.
//!
//! The regression it guards, caught by driving the real desktop app: the Electron
//! app sets `BIOROUTER_AUTOVIS_CDN=1` by default (see `ui/desktop/src/biorouterd.ts`),
//! so CDN mode is the *normal* path in the GUI. A standalone figure survives it only
//! because the main process rewrites its `<script src=…>` into an inline script before
//! display — the renderer's CSP is `script-src 'self' 'unsafe-inline'`, so a remote
//! script never loads. A report hides its library tags inside base64 blobs where that
//! rewriter cannot reach them, and every figure came up blank with
//! "Uncaught ReferenceError: Chart is not defined".
//!
//! So a report inlines its libraries regardless of the flag, and dedup keeps that to
//! exactly one copy per library however many panels use it.

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use biorouter_mcp::AutoVisualiserRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, RawContent, ResourceContents};
use serde_json::json;

const ASSET_PLACEHOLDER: &str = "<!--AUTOVIS_ASSETS-->";

fn html_of(result: &CallToolResult) -> String {
    if let RawContent::Resource(resource) = &*result.content[0] {
        if let ResourceContents::BlobResourceContents { blob, .. } = &resource.resource {
            return String::from_utf8(STANDARD.decode(blob).unwrap()).unwrap();
        }
    }
    panic!("expected a blob resource");
}

fn panel_documents(html: &str) -> Vec<String> {
    html.split("<script type=\"text/plain\" id=\"autovis-panel-")
        .skip(1)
        .filter_map(|chunk| {
            let (_, rest) = chunk.split_once('>')?;
            let (b64, _) = rest.split_once("</script>")?;
            Some(String::from_utf8(STANDARD.decode(b64.trim()).unwrap()).unwrap())
        })
        .collect()
}

#[tokio::test]
async fn cdn_mode_still_produces_a_self_contained_report() {
    std::env::set_var("BIOROUTER_AUTOVIS_CDN", "1");

    let router = AutoVisualiserRouter::new();
    let result = router
        .render_dashboard(Parameters(
            serde_json::from_value(json!({
                "title": "CDN report",
                "panels": [
                    {"title": "Counts", "figure": {"tool": "show_chart", "params": {"data": {
                        "type": "bar", "labels": ["A"], "datasets": [{"label": "S", "data": [1.0]}]
                    }}}},
                    // A second Chart.js panel: the library must still be stored once.
                    {"title": "Volcano", "figure": {"tool": "render_volcano", "params": {"data": {
                        "points": [{"label": "MYC", "log2fc": 2.4, "negLog10P": 4.0}]
                    }}}}
                ]
            }))
            .unwrap(),
        ))
        .await
        .unwrap();

    let html = html_of(&result);
    std::env::remove_var("BIOROUTER_AUTOVIS_CDN");

    // Every panel still carries the sentinel, so the page's JS runs and injects
    // both the library sources and the embed stylesheet.
    let panels = panel_documents(&html);
    assert_eq!(panels.len(), 2);
    for panel in &panels {
        assert!(
            panel.contains(ASSET_PLACEHOLDER),
            "CDN mode must not pre-substitute the panel's asset placeholder"
        );
    }

    // The report never references a CDN: the renderer's CSP would block it.
    assert!(
        !html.contains("cdn.jsdelivr.net"),
        "a report must inline its libraries, not reference a CDN"
    );
    assert!(!html.contains("data-kind=\"tag\""));

    // Chart.js is stored exactly once, as inlined source, for both panels.
    assert_eq!(html.matches("data-autovis-asset=\"chartjs\"").count(), 1);
    assert!(html.contains("data-autovis-asset=\"chartjs\" data-kind=\"js\""));
    assert!(
        html.len() > 200_000,
        "Chart.js should be inlined (~200 KB), got {} bytes",
        html.len()
    );

    // Each panel still declares which libraries it needs.
    assert!(html.contains(r#""assets":["chartjs"]"#));
}
