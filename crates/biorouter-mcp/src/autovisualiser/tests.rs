use super::common::validate_data_param;
use super::*;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{ErrorCode, RawContent, ResourceContents, Role};
use serde_json::json;

#[tokio::test]
async fn figure_receipt_is_small_while_the_user_keeps_the_complete_artifact() {
    let html = format!(
        "<html><body>{}</body></html>",
        "synthetic-figure".repeat(20_000)
    );
    let (result, _) = common::render_fragment(async {
        common::finish(
            "ui://chart/test",
            "receipt-test",
            &"Δ".repeat(2_000),
            html.clone(),
        )
    })
    .await;
    let wire = serde_json::to_value(&result).unwrap();
    let receipt = wire.get("structuredContent").expect("model-facing receipt");
    assert_eq!(receipt["status"], "created");
    assert_eq!(receipt["uri"], "ui://chart/test");
    assert_eq!(receipt["mimeType"], "text/html");
    assert_eq!(receipt["summary"].as_str().unwrap().chars().count(), 512);
    assert!(serde_json::to_string(receipt).unwrap().len() < 2_000);
    assert_eq!(common::html_from_result(&result).unwrap(), html);
    assert_eq!(result.content.len(), 2);
    assert_eq!(result.content[0].audience().unwrap(), &vec![Role::User]);
    assert_eq!(
        result.content[1].audience().unwrap(),
        &vec![Role::Assistant]
    );
}

#[test]
fn forced_theme_injection_preserves_unicode_document_content() {
    let html = "<!doctype html><html><head><title>Résumé</title></head><body>Δ</body></html>";
    let result = inject_forced_theme(html.to_string(), "dark");

    assert!(result
        .contains("<head><script>window.__BR_VIZ_THEME__=\"dark\";</script><title>Résumé</title>"));
    assert!(result.ends_with("<body>Δ</body></html>"));
}

// ---------------------------------------------------------------------------
// validate_data_param (loosely-typed data guard)
// ---------------------------------------------------------------------------

#[test]
fn test_validate_data_param_rejects_string() {
    let params = json!({
        "data": "{\"labels\": [\"A\", \"B\"], \"matrix\": [[0, 1], [1, 0]]}"
    });
    let err = validate_data_param(&params, false).unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    assert!(err
        .message
        .contains("must be a JSON object, not a JSON string"));
    assert!(err.message.contains("without comments"));
}

#[test]
fn test_validate_data_param_accepts_object() {
    let params = json!({ "data": { "labels": ["A", "B"], "matrix": [[0, 1], [1, 0]] } });
    let data = validate_data_param(&params, false).unwrap();
    assert!(data.is_object());
    assert_eq!(data["labels"][0], "A");
}

#[test]
fn test_validate_data_param_rejects_array_when_not_allowed() {
    let params = json!({ "data": [{"label": "A", "value": 10}] });
    let err = validate_data_param(&params, false).unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    assert!(err.message.contains("must be a JSON object"));
}

#[test]
fn test_validate_data_param_accepts_array_when_allowed() {
    let params = json!({ "data": [{"label": "A", "value": 10}] });
    let data = validate_data_param(&params, true).unwrap();
    assert!(data.is_array());
    assert_eq!(data[0]["label"], "A");
}

#[test]
fn test_validate_data_param_missing_data() {
    let params = json!({ "other": "value" });
    let err = validate_data_param(&params, false).unwrap_err();
    assert!(err.message.contains("Missing 'data' parameter"));
}

#[test]
fn test_validate_data_param_rejects_primitive_values() {
    assert!(validate_data_param(&json!({ "data": 42 }), false).is_err());
    assert!(validate_data_param(&json!({ "data": true }), false).is_err());
    assert!(validate_data_param(&json!({ "data": null }), false).is_err());
}

// ---------------------------------------------------------------------------
// Shared infrastructure (escaping, assets, lenient enums)
// ---------------------------------------------------------------------------

#[test]
fn test_js_data_neutralizes_script_breakout() {
    // A literal </script> in data must not be able to break out of the script tag.
    let v = json!({ "name": "</script><script>alert(1)</script>" });
    let s = common::js_data(&v).unwrap();
    assert!(!s.contains("</script>"));
    assert!(s.contains("\\u003c"));
}

#[test]
fn test_js_data_escapes_line_separators() {
    let v = Value::String("line\u{2028}sep\u{2029}end".to_string());
    let s = common::js_data(&v).unwrap();
    assert!(!s.contains('\u{2028}'));
    assert!(!s.contains('\u{2029}'));
    assert!(s.contains("\\u2028"));
}

#[test]
fn test_html_escape() {
    assert_eq!(
        common::html_escape("<b>\"x\" & 'y'</b>"),
        "&lt;b&gt;&quot;x&quot; &amp; &#39;y&#39;&lt;/b&gt;"
    );
}

#[test]
fn test_asset_html_inline_default() {
    // Default (no env) inlines the library.
    let html = common::asset_html(&[Asset::ChartJs]);
    assert!(html.contains("<script>"));
    assert!(!html.contains("cdn.jsdelivr.net"));
}

#[test]
fn test_lenient_chart_type_parsing() {
    // Capitalized / uppercase / padded all parse.
    for raw in ["\"Line\"", "\"LINE\"", "\" line \"", "\"line\""] {
        let parsed: ChartType = serde_json::from_str(raw).unwrap();
        assert!(matches!(parsed, ChartType::Line));
    }
    assert!(serde_json::from_str::<ChartType>("\"pie\"").is_err());
}

#[test]
fn test_lenient_donut_type_parsing() {
    for raw in ["\"Doughnut\"", "\"DONUT\"", "\"doughnut\""] {
        let parsed: DonutChartType = serde_json::from_str(raw).unwrap();
        assert!(matches!(parsed, DonutChartType::Doughnut));
    }
    assert!(matches!(
        serde_json::from_str::<DonutChartType>("\"Pie\"").unwrap(),
        DonutChartType::Pie
    ));
}

// ---------------------------------------------------------------------------
// Result-shape helper used by every render-tool test below.
// ---------------------------------------------------------------------------

fn assert_resource_result(result: &CallToolResult, expected_uri: &str) {
    // Two items: user-audience resource + assistant-audience text confirmation.
    assert_eq!(result.content.len(), 2);
    assert_eq!(result.content[0].audience().unwrap(), &vec![Role::User]);
    assert_eq!(
        result.content[1].audience().unwrap(),
        &vec![Role::Assistant]
    );
    assert!(matches!(&*result.content[1], RawContent::Text(_)));
    if let RawContent::Text(text) = &*result.content[1] {
        assert!(!text.text.contains("rendered inline"));
        assert!(!text.text.contains("already displayed"));
    }
    if let RawContent::Resource(resource) = &*result.content[0] {
        if let ResourceContents::BlobResourceContents {
            uri,
            mime_type,
            blob,
            ..
        } = &resource.resource
        {
            assert_eq!(uri, expected_uri);
            assert_eq!(mime_type.as_ref().unwrap(), "text/html");
            assert!(!blob.is_empty(), "HTML content should not be empty");
        } else {
            panic!("Expected BlobResourceContents");
        }
    } else {
        panic!("Expected Resource content");
    }
}

// ---------------------------------------------------------------------------
// Existing tools (happy path)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_render_sankey() {
    let router = AutoVisualiserRouter::new();
    let params = Parameters(RenderSankeyParams {
        data: SankeyData {
            nodes: vec![
                SankeyNode {
                    name: "A".to_string(),
                    category: None,
                },
                SankeyNode {
                    name: "B".to_string(),
                    category: None,
                },
            ],
            links: vec![SankeyLink {
                source: "A".to_string(),
                target: "B".to_string(),
                value: 10.0,
            }],
        },
    });
    let result = router.render_sankey(params).await.unwrap();
    assert_resource_result(&result, "ui://sankey/diagram");
}

#[tokio::test]
async fn test_render_sankey_rejects_unknown_node() {
    let router = AutoVisualiserRouter::new();
    let params = Parameters(RenderSankeyParams {
        data: SankeyData {
            nodes: vec![SankeyNode {
                name: "A".to_string(),
                category: None,
            }],
            links: vec![SankeyLink {
                source: "A".to_string(),
                target: "GHOST".to_string(),
                value: 1.0,
            }],
        },
    });
    let err = router.render_sankey(params).await.unwrap_err();
    assert!(err.message.contains("GHOST"));
}

#[tokio::test]
async fn test_render_sankey_rejects_empty() {
    let router = AutoVisualiserRouter::new();
    let params = Parameters(RenderSankeyParams {
        data: SankeyData {
            nodes: vec![],
            links: vec![],
        },
    });
    assert!(router.render_sankey(params).await.is_err());
}

#[tokio::test]
async fn test_render_radar() {
    let router = AutoVisualiserRouter::new();
    let params = Parameters(RenderRadarParams {
        data: RadarData {
            labels: vec![
                "Speed".to_string(),
                "Power".to_string(),
                "Agility".to_string(),
            ],
            datasets: vec![RadarDataset {
                label: "Player 1".to_string(),
                data: vec![80.0, 90.0, 85.0],
            }],
        },
    });
    let result = router.render_radar(params).await.unwrap();
    assert_resource_result(&result, "ui://radar/chart");
}

#[tokio::test]
async fn test_render_radar_rejects_length_mismatch() {
    let router = AutoVisualiserRouter::new();
    let params = Parameters(RenderRadarParams {
        data: RadarData {
            labels: vec!["A".to_string(), "B".to_string()],
            datasets: vec![RadarDataset {
                label: "x".to_string(),
                data: vec![1.0],
            }],
        },
    });
    assert!(router.render_radar(params).await.is_err());
}

#[tokio::test]
async fn test_render_donut() {
    let router = AutoVisualiserRouter::new();
    let params = Parameters(RenderDonutParams {
        data: DonutData {
            data: DonutChartData::Single(SingleDonutChart {
                data: vec![
                    DonutDataItem::Number(30.0),
                    DonutDataItem::Number(40.0),
                    DonutDataItem::Number(30.0),
                ],
                labels: Some(vec!["A".to_string(), "B".to_string(), "C".to_string()]),
                title: None,
                chart_type: None,
            }),
        },
    });
    let result = router.render_donut(params).await.unwrap();
    assert_resource_result(&result, "ui://donut/chart");
}

#[tokio::test]
async fn test_render_treemap() {
    let router = AutoVisualiserRouter::new();
    let params = Parameters(RenderTreemapParams {
        data: TreemapNode {
            name: "root".to_string(),
            value: None,
            category: None,
            children: Some(vec![
                TreemapNode {
                    name: "A".to_string(),
                    value: Some(100.0),
                    category: Some("Type1".to_string()),
                    children: None,
                },
                TreemapNode {
                    name: "B".to_string(),
                    value: Some(200.0),
                    category: Some("Type2".to_string()),
                    children: None,
                },
            ]),
        },
    });
    let result = router.render_treemap(params).await.unwrap();
    assert_resource_result(&result, "ui://treemap/visualization");
}

#[tokio::test]
async fn test_render_chord() {
    let router = AutoVisualiserRouter::new();
    let params = Parameters(RenderChordParams {
        data: ChordData {
            labels: vec!["A".to_string(), "B".to_string(), "C".to_string()],
            matrix: vec![
                vec![0.0, 10.0, 5.0],
                vec![10.0, 0.0, 15.0],
                vec![5.0, 15.0, 0.0],
            ],
        },
    });
    let result = router.render_chord(params).await.unwrap();
    assert_resource_result(&result, "ui://chord/diagram");
}

#[tokio::test]
async fn test_render_chord_rejects_non_square() {
    let router = AutoVisualiserRouter::new();
    let params = Parameters(RenderChordParams {
        data: ChordData {
            labels: vec!["A".to_string(), "B".to_string()],
            matrix: vec![vec![0.0, 1.0]],
        },
    });
    assert!(router.render_chord(params).await.is_err());
}

#[tokio::test]
async fn test_render_map() {
    let router = AutoVisualiserRouter::new();
    let params = Parameters(RenderMapParams {
        data: MapData {
            markers: vec![MapMarker {
                lat: 0.0,
                lng: 0.0,
                name: Some("Origin".to_string()),
                value: None,
                description: None,
                popup: None,
                color: None,
                label: None,
                use_default_icon: None,
            }],
            title: None,
            subtitle: None,
            center: None,
            zoom: None,
            clustering: None,
            cluster_radius: None,
            auto_fit: None,
        },
    });
    let result = router.render_map(params).await.unwrap();
    assert_resource_result(&result, "ui://map/visualization");
}

#[tokio::test]
async fn test_render_map_rejects_bad_coords() {
    let router = AutoVisualiserRouter::new();
    let params = Parameters(RenderMapParams {
        data: MapData {
            markers: vec![MapMarker {
                lat: 999.0,
                lng: 0.0,
                name: None,
                value: None,
                description: None,
                popup: None,
                color: None,
                label: None,
                use_default_icon: None,
            }],
            title: None,
            subtitle: None,
            center: None,
            zoom: None,
            clustering: None,
            cluster_radius: None,
            auto_fit: None,
        },
    });
    assert!(router.render_map(params).await.is_err());
}

#[tokio::test]
async fn test_show_chart() {
    let router = AutoVisualiserRouter::new();
    let params = Parameters(ShowChartParams {
        data: ChartData {
            chart_type: ChartType::Scatter,
            datasets: vec![ChartDataset {
                label: "Test Data".to_string(),
                data: ChartDataValues::Points(vec![
                    ChartPoint { x: 1.0, y: 2.0 },
                    ChartPoint { x: 2.0, y: 4.0 },
                ]),
                background_color: None,
                border_color: None,
                border_width: None,
                tension: None,
                fill: None,
            }],
            labels: None,
            title: None,
            subtitle: None,
            x_axis_label: None,
            y_axis_label: None,
        },
    });
    let result = router.show_chart(params).await.unwrap();
    assert_resource_result(&result, "ui://scatter/chart");
}

#[tokio::test]
async fn academic_chart_defaults_preserve_labels_styles_and_script_escaping() {
    let html = render_standalone_figure(
        "show_chart",
        json!({"data": {
            "type": "line",
            "title": "Δοκιμή 東京 — study outcome",
            "subtitle": "Synthetic </script><script>bad()</script> evidence",
            "xAxisLabel": "Follow-up (days)",
            "yAxisLabel": "Outcome (mg/L)",
            "labels": ["Long Unicode category — 東京 🧬", "Comparison"],
            "datasets": [{"label": "Cohort A", "data": [1.0, 2.0],
                "backgroundColor": "#123456", "borderColor": "#654321",
                "borderWidth": 3.0, "tension": 0.2, "fill": true}]
        }}),
    )
    .await
    .expect("academic chart renders");

    assert!(!html.contains("linear-gradient"));
    assert!(!html.contains("Interactive data visualization"));
    assert!(html.contains("BioRouterViz.applyChartDefaults()"));
    assert!(html.contains("BioRouterViz.wrapLabel"));
    assert!(html.contains("role=\"img\""));
    assert!(html.contains("<table"));
    assert!(html.contains("Δοκιμή 東京 — study outcome"));
    assert!(html.contains("Outcome (mg/L)"));
    assert!(html.contains("#123456"));
    assert!(html.contains("#654321"));
    assert!(html.contains("\"tension\":0.2"));
    assert!(html.contains("\"fill\":true"));
    assert!(!html.contains("</script><script>bad()"));
    assert!(html.contains("\\u003c/script>\\u003cscript>bad()"));
}

#[test]
fn academic_figure_guidance_is_part_of_the_capability_prompt() {
    let router = AutoVisualiserRouter::new();
    for guidance in [
        "clean, minimal academic figure",
        "label axes with quantities and units",
        "largest legible text that fits without overlap",
        "long Unicode labels",
        "do not smooth measured line data",
        "Do not claim visual verification",
    ] {
        assert!(router.instructions.contains(guidance), "{guidance}");
    }
}

#[tokio::test]
async fn test_render_mermaid() {
    let router = AutoVisualiserRouter::new();
    let params = Parameters(RenderMermaidParams {
        mermaid_code: "graph TD;\n    A-->B;\n    A-->C;".to_string(),
    });
    let result = router.render_mermaid(params).await.unwrap();
    assert_resource_result(&result, "ui://mermaid/diagram");
}

#[tokio::test]
async fn test_render_mermaid_rejects_empty() {
    let router = AutoVisualiserRouter::new();
    let params = Parameters(RenderMermaidParams {
        mermaid_code: "   ".to_string(),
    });
    assert!(router.render_mermaid(params).await.is_err());
}

#[tokio::test]
async fn test_mermaid_blob_has_escaped_code() {
    // The mermaid source must be injected without a raw </script> breakout.
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let router = AutoVisualiserRouter::new();
    let params = Parameters(RenderMermaidParams {
        mermaid_code: "graph TD; A[\"</script>\"]-->B;".to_string(),
    });
    let result = router.render_mermaid(params).await.unwrap();
    if let RawContent::Resource(resource) = &*result.content[0] {
        if let ResourceContents::BlobResourceContents { blob, .. } = &resource.resource {
            let html = String::from_utf8(STANDARD.decode(blob).unwrap()).unwrap();
            // The injected JS string literal must not contain a literal </script>.
            let marker = "const mermaidCode =";
            let start = html.find(marker).unwrap();
            let snippet: String = html
                .get(start..)
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect();
            assert!(!snippet.contains("</script>"));
        }
    }
}

// ---------------------------------------------------------------------------
// render_standalone_figure — the embedding API (figures inside apps).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn standalone_figure_returns_self_contained_html() {
    let html = render_standalone_figure(
        "show_chart",
        json!({"data": {
            "type": "bar",
            "labels": ["A", "B"],
            "datasets": [{"label": "S", "data": [1.0, 2.0]}]
        }}),
    )
    .await
    .expect("show_chart should render");

    // A complete standalone document...
    assert!(html.contains("<!DOCTYPE") || html.contains("<html"));
    // ...with the chart library inlined (not a CDN reference).
    assert!(html.contains("Chart.js v"), "Chart.js should be inlined");
    assert!(html.contains("<script>"));
    assert!(!html.contains("cdn.jsdelivr.net"));
}

#[tokio::test]
async fn standalone_figure_accepts_prefixless_name() {
    // "volcano" must resolve to render_volcano just like the dashboard panels.
    let html = render_standalone_figure(
        "volcano",
        json!({"data": {"points": [{"label": "MYC", "log2fc": 2.4, "negLog10P": 4.0}]}}),
    )
    .await
    .expect("prefixless 'volcano' should render");
    assert!(html.contains("<!DOCTYPE") || html.contains("<html"));
}

#[tokio::test]
async fn standalone_figure_dashboard_works() {
    // A report embedded in an app is legitimate: render_dashboard must dispatch.
    let html = render_standalone_figure(
        "render_dashboard",
        json!({
            "title": "Embedded report",
            "panels": [
                {"title": "Counts", "figure": {"tool": "show_chart", "params": {"data": {
                    "type": "bar", "labels": ["A"], "datasets": [{"label": "S", "data": [1.0]}]
                }}}}
            ]
        }),
    )
    .await
    .expect("render_dashboard should render");
    assert!(html.contains("Embedded report"));
    // A report inlines its libraries too — never a CDN reference.
    assert!(!html.contains("cdn.jsdelivr.net"));
}

/// ⚠ This asserts the per-kind TOOL NAMES on purpose, and it is the one place
/// that still should.
///
/// The dashboard's copy of this message was rewritten to name `render_figure`
/// and the `kind` slugs, because a chat agent sees only three tools and cannot
/// call `render_volcano` (#142). This door is Agent Drafter's `ui_figure`, whose
/// own description hands the app agent exactly these names and which has neither
/// `render_figure` nor `describe_figure` — `configure_agent` never injects
/// autovisualiser into an app agent. Sharing one phrasing between the two doors
/// is what created a NEW dead end here while fixing the one over there; the
/// vocabulary is therefore chosen per call site.
#[tokio::test]
async fn standalone_figure_unknown_tool_errs_with_suggestions() {
    let err = render_standalone_figure("totally_made_up", json!({"data": {}}))
        .await
        .unwrap_err();
    assert!(err.contains("Unknown visualization"), "got: {err}");
    // Names the caller can reach for instead.
    assert!(
        err.contains("render_volcano") || err.contains("show_chart"),
        "got: {err}"
    );
    assert!(
        !err.contains("describe_figure"),
        "an app agent has no describe_figure to call: {err}"
    );
}

#[tokio::test]
async fn standalone_figure_invalid_args_err_is_friendly() {
    // show_chart validates that at least one dataset is present.
    let err = render_standalone_figure(
        "show_chart",
        json!({"data": {"type": "bar", "datasets": []}}),
    )
    .await
    .unwrap_err();
    assert!(err.contains("at least one dataset"), "got: {err}");
}

/// The same rule for a REJECTED PAYLOAD, which is the half the shared
/// `figure_argument_error` choke point actually broke.
///
/// Measured before this fix: `ui_figure("render_volcano", …)` with a missing
/// field came back as "`render_figure` arguments are invalid for kind
/// \"volcano\": missing field `log2fc`. Call describe_figure with kind
/// \"volcano\"…" — an app agent being told to fix a call it never made, with two
/// tools it does not have. It must name the tool the caller named.
#[tokio::test]
async fn standalone_figure_invalid_args_name_the_tool_the_caller_named() {
    let err = render_standalone_figure(
        "render_volcano",
        json!({"data": {"points": [{"label": "MYC", "negLog10P": 4.0}]}}),
    )
    .await
    .unwrap_err();

    assert!(
        err.contains("log2fc"),
        "must still say what is wrong: {err}"
    );
    assert!(
        err.contains("render_volcano"),
        "must name the tool `ui_figure` takes: {err}"
    );
    assert!(
        !err.contains("render_figure"),
        "an app agent cannot call render_figure: {err}"
    );
    assert!(
        !err.contains("describe_figure"),
        "an app agent cannot call describe_figure: {err}"
    );
}

#[tokio::test]
async fn standalone_figure_ignores_cdn_env_flag() {
    // The standalone path forces inlining via a task-local checked *before* the
    // BIOROUTER_AUTOVIS_CDN env read, so a figure is self-contained even when the
    // desktop app has CDN mode on. We assert the override mechanism directly here
    // rather than mutating the process-wide env var, which would race the other
    // figure unit tests that assert no CDN (see autovis_dashboard_cdn.rs, which is
    // a separate binary for exactly that reason).
    let cdn_inside = common::with_inline_assets(async { common::use_cdn() }).await;
    assert!(!cdn_inside, "with_inline_assets must force use_cdn() off");

    // And end-to-end: the emitted document inlines the library, never a CDN tag.
    let html = render_standalone_figure(
        "show_chart",
        json!({"data": {
            "type": "line",
            "labels": ["A"],
            "datasets": [{"label": "S", "data": [1.0]}]
        }}),
    )
    .await
    .unwrap();
    assert!(html.contains("Chart.js v"));
    assert!(!html.contains("cdn.jsdelivr.net"));
    assert!(!html.contains("<script src="));
}

// ---------------------------------------------------------------------------
// Third-party attribution.
//
// Every file in `templates/assets/` is `include_str!`d into the binary and
// inlined into the figures BioRouter generates, so shipping a figure is
// redistributing those libraries. MIT, BSD and ISC all require the copyright
// notice and the licence text to travel with the copy. Nothing used to check
// that, which is how four of the seven files came to ship with no notice at
// all, so these tests are the part that stops the fix rotting.
// ---------------------------------------------------------------------------

/// Fails when a file is added to `templates/assets/` without a `LICENSES.md`
/// entry, and when an entry names a file that is no longer there.
#[test]
fn every_vendored_asset_is_covered_by_the_licence_file() {
    use std::collections::BTreeSet;
    use std::path::Path;

    // Resolved through the constant the generated notice points readers at, so a
    // moved or misspelled path fails here rather than sending a user nowhere.
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<crate> sits two levels below the repository root");
    let licences_path = repo_root.join(common::LICENSES_PATH);
    let licences = std::fs::read_to_string(&licences_path).unwrap_or_else(|e| {
        panic!(
            "{} is the notice BioRouter redistributes with every figure, and it \
             could not be read: {e}",
            licences_path.display()
        )
    });
    let dir = licences_path
        .parent()
        .expect("LICENSES.md lives in the assets directory");

    let on_disk: BTreeSet<String> = std::fs::read_dir(dir)
        .expect("assets directory")
        .map(|entry| entry.expect("assets directory entry"))
        .filter(|entry| entry.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "LICENSES.md" && !name.starts_with('.'))
        .collect();

    assert!(
        on_disk.len() >= 7,
        "expected the vendored libraries to still be present, found {on_disk:?}"
    );

    // A section per file, so an entry cannot decay into a passing mention of the
    // filename somewhere in the prose.
    let documented: BTreeSet<String> = licences
        .lines()
        .filter_map(|line| line.strip_prefix("## `"))
        .filter_map(|rest| rest.strip_suffix('`'))
        .map(str::to_string)
        .collect();

    for name in &on_disk {
        assert!(
            documented.contains(name),
            "{name} is inlined into generated figures, so BioRouter redistributes \
             it, but {} has no '## `{name}`' section. Add the library, version, \
             licence, copyright line and full licence text.",
            common::LICENSES_PATH
        );
    }
    for name in &documented {
        assert!(
            on_disk.contains(name),
            "{} documents {name}, which is not in the assets directory. Remove \
             the stale entry.",
            common::LICENSES_PATH
        );
    }

    // Each section must carry the four facts plus a fenced licence text. A URL
    // is not a notice that travels, so the text has to be here.
    for section in licences.split("\n## `").skip(1) {
        let name = section.split('`').next().unwrap_or("<unnamed>").to_string();
        for field in ["**Version:**", "**Licence:**", "**Copyright:**"] {
            assert!(
                section.contains(field),
                "the {name} entry in {} is missing {field}",
                common::LICENSES_PATH
            );
        }
        let fenced: String = section
            .split("\n```")
            .nth(1)
            .unwrap_or_default()
            .trim()
            .to_string();
        assert!(
            fenced.len() > 400,
            "the {name} entry in {} has no full licence text (found {} bytes in \
             its code fence)",
            common::LICENSES_PATH,
            fenced.len()
        );
    }
}

#[test]
fn attribution_comment_is_a_well_formed_html_comment() {
    let notice = common::ATTRIBUTION_COMMENT;
    assert!(notice.starts_with("<!--"));
    assert!(notice.trim_end().ends_with("-->"));

    let inner = notice
        .trim_end()
        .trim_start_matches("<!--")
        .trim_end_matches("-->");
    assert!(
        !inner.contains("--"),
        "'--' is illegal inside an HTML comment: {inner}"
    );

    for library in [
        "Chart.js 4.5.0",
        "D3 7.9.0",
        "d3-sankey 0.12.3",
        "Leaflet 1.9.4",
        "Leaflet.markercluster 1.5.3",
        "Mermaid 11.17.2",
    ] {
        assert!(
            notice.contains(library),
            "the notice does not name {library}"
        );
    }
    for licence in ["MIT", "ISC", "BSD-3-Clause", "BSD-2-Clause"] {
        assert!(notice.contains(licence), "the notice omits {licence}");
    }
    assert!(
        notice.contains(common::LICENSES_PATH),
        "the notice must say where the full texts live"
    );
    assert!(
        notice.len() < 700,
        "this rides in every figure and every dashboard panel, so keep it short: \
         {} bytes",
        notice.len()
    );
}

/// Every `"X.Y.Z"` string literal in a minified bundle, for reading a library's
/// own version back out of the bytes that ship rather than trusting a comment.
fn quoted_semvers(source: &str) -> std::collections::BTreeSet<String> {
    // Splitting on the quote is enough: a `"` byte cannot occur inside a
    // multi-byte UTF-8 character, so every segment is a whole string, and the
    // escaped quotes that break the strict inside/outside alternation only ever
    // add candidates, never hide one.
    source
        .split('"')
        .filter(|segment| is_semver(segment))
        .map(str::to_string)
        .collect()
}

/// Exactly three dot-separated runs of ASCII digits, nothing else.
fn is_semver(text: &str) -> bool {
    let mut parts = text.split('.');
    let three_numbers = (0..3).all(|_| {
        parts
            .next()
            .is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    });
    three_numbers && parts.next().is_none()
}

/// Every semver-shaped token in `text`, by splitting on anything that is not a
/// digit or a dot.
fn semvers_in(text: &str) -> std::collections::BTreeSet<String> {
    text.split(|c: char| !c.is_ascii_digit() && c != '.')
        .filter(|token| is_semver(token))
        .map(str::to_string)
        .collect()
}

/// The versions a vendored bundle declares about itself.
///
/// Two places carry one, and which one it is varies by library: a `"x.y.z"`
/// string literal in the code (Chart.js, D3, Leaflet) and the banner comment the
/// minifier preserved at the top (d3-sankey has only that). Both are read, so no
/// library needs a rule of its own.
fn declared_versions(source: &str) -> std::collections::BTreeSet<String> {
    /// Long enough for the longest banner here, short enough that this is not a
    /// scan of a 3.5 MB bundle for tokens that are not version declarations.
    const BANNER_CHARS: usize = 2048;

    let banner: String = source.chars().take(BANNER_CHARS).collect();
    let mut versions = quoted_semvers(source);
    versions.extend(semvers_in(&banner));
    versions
}

/// The exact `major.minor.patch` a jsdelivr npm URL pins, or `None` when it
/// floats (`@11`, `@0.12`) or is not such a URL. A floating pin fails the test.
fn exact_version_pinned_in(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://cdn.jsdelivr.net/npm/")?;
    // `<package>@<version>/<path>`. A scoped package's leading `@scope` segment
    // has its `@` at index 0, which is how it is told apart from the one that
    // introduces the version.
    let segment = rest
        .split('/')
        .find(|s| s.find('@').is_some_and(|i| i > 0))?;
    let version = segment.split_once('@')?.1;
    is_semver(version).then(|| version.to_string())
}

/// The two vendored files that carry no version string anywhere in their bytes,
/// so the comparison below has nothing to read.
///
/// They are named rather than skipped silently: a new version-less asset has to
/// be a decision someone took, not one this test took for them.
const NO_VERSION_IN_THE_BYTES: &[&str] = &["leaflet.min.css", "leaflet.markercluster.min.js"];

fn repo_root_for_tests() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crates/<crate> sits two levels below the repository root")
        .to_path_buf()
}

/// `PINNED_LIBRARIES` must hold every file that ships inlined, or a library
/// could be added on one delivery path and never checked against the other.
#[test]
fn every_shipped_library_has_a_pinned_cdn_row() {
    use std::collections::BTreeSet;

    let dir = repo_root_for_tests()
        .join(common::LICENSES_PATH)
        .parent()
        .expect("LICENSES.md lives in the assets directory")
        .to_path_buf();

    let on_disk: BTreeSet<String> = std::fs::read_dir(&dir)
        .expect("assets directory")
        .map(|entry| entry.expect("assets directory entry"))
        .filter(|entry| entry.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "LICENSES.md" && !name.starts_with('.'))
        .collect();

    let tabled: BTreeSet<String> = common::PINNED_LIBRARIES
        .iter()
        .map(|lib| lib.file.to_string())
        .collect();

    assert_eq!(
        on_disk, tabled,
        "PINNED_LIBRARIES and templates/assets/ have diverged. A vendored file \
         with no row is a library whose CDN pin nothing compares against, which \
         is how chart.js came to serve 4.5.1 against a vendored 4.5.0."
    );
}

/// For every library, the vendored bytes, the CDN pin, the attribution notice
/// and `LICENSES.md` must all name one release.
///
/// A library with two live delivery paths can disagree with itself without
/// anything failing. A standalone figure loads the CDN URL (the desktop sets
/// `BIOROUTER_AUTOVIS_CDN=1` by default) while a dashboard always inlines the
/// vendored bytes, so a floating pin means the same figure is drawn by two
/// different releases inside one app, and the licence notice every figure
/// carries states a version that figure does not contain. Both happened:
/// Mermaid's pin floated on `@11` against a vendored 10.9.0, and `chart.js@4`
/// resolved to 4.5.1 against a vendored 4.5.0.
///
/// The vendored version is read back out of the bundle rather than written down
/// a second time, because a hand-written constant is exactly what drifts. An
/// inexact pin fails outright: a floating major cannot be compared to anything,
/// which is how the drift went unnoticed for so long.
#[test]
fn vendored_and_cdn_pins_are_the_same_version() {
    let licences = std::fs::read_to_string(repo_root_for_tests().join(common::LICENSES_PATH))
        .expect("LICENSES.md is read by every_vendored_asset_is_covered_by_the_licence_file");

    for lib in common::PINNED_LIBRARIES {
        let file = lib.file;
        let pinned = exact_version_pinned_in(lib.cdn_url).unwrap_or_else(|| {
            panic!(
                "the CDN pin for {file} must name an exact major.minor.patch so the \
                 vendored copy can be compared against it, found {}. A floating \
                 version silently changes what a figure loads, with no commit.",
                lib.cdn_url
            )
        });

        let declared = declared_versions(lib.vendored);
        if declared.is_empty() {
            assert!(
                NO_VERSION_IN_THE_BYTES.contains(&file),
                "{file} carries no version string, so its pin at {pinned} is \
                 unverifiable. Either the file was replaced with a build that \
                 dropped its banner, or it is a new asset that needs a line in \
                 NO_VERSION_IN_THE_BYTES saying so."
            );
        } else {
            assert!(
                declared.contains(&pinned),
                "the CDN pin for {file} names {pinned}, but the vendored bundle \
                 does not carry that version string. The versions it does carry \
                 are {declared:?}. Replace templates/assets/{file} with the {pinned} \
                 build, or move the pin to match the file."
            );
        }

        // The notice rides in every figure, so a stale version here is a licence
        // statement about bytes the figure does not contain.
        let named = format!("{} {pinned} ", lib.notice_name);
        assert!(
            common::ATTRIBUTION_COMMENT.contains(&named),
            "the attribution notice must say '{named}', the version {file} now \
             ships on both delivery paths: {}",
            common::ATTRIBUTION_COMMENT
        );

        let section = licences
            .split("\n## `")
            .find(|s| s.starts_with(&format!("{file}`")))
            .unwrap_or_else(|| panic!("{} has no {file} section", common::LICENSES_PATH));
        assert!(
            section.contains(&pinned),
            "the {file} entry in {} must record version {pinned}",
            common::LICENSES_PATH
        );
    }
}

#[test]
fn a_floating_cdn_pin_is_rejected() {
    // The shape the drift shipped in, kept as a negative control so the guard
    // above cannot quietly stop distinguishing the two.
    assert_eq!(
        exact_version_pinned_in("https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.min.js"),
        None
    );
    assert_eq!(
        exact_version_pinned_in("https://cdn.jsdelivr.net/npm/chart.js@4/dist/chart.umd.min.js"),
        None
    );
    assert_eq!(
        exact_version_pinned_in("https://cdn.jsdelivr.net/npm/d3@7.9.0/dist/d3.min.js").as_deref(),
        Some("7.9.0")
    );
    assert_eq!(
        exact_version_pinned_in("https://example.com/npm/d3@7.9.0/dist/d3.min.js"),
        None
    );
}

#[test]
fn the_attribution_notice_lands_in_the_head_and_never_twice() {
    let notice = common::ATTRIBUTION_COMMENT;

    let once = common::assemble(
        "<!DOCTYPE html><html><head>\n<title>t</title>\n</head><body></body></html>",
        &[],
        &[],
    );
    assert_eq!(once.matches(notice).count(), 1);
    assert!(once.contains(&format!("<head>\n{notice}")));

    // Re-assembling an assembled document must not stack notices.
    assert_eq!(common::assemble(&once, &[], &[]).matches(notice).count(), 1);

    // A template with no head still carries it.
    assert!(common::assemble("<p>x</p>", &[], &[]).starts_with(notice));

    // The combined report has its own template with no {{ASSETS}} slot, so it
    // would be the easy one to miss.
    let report = common::assemble(include_str!("templates/dashboard_template.html"), &[], &[]);
    assert_eq!(report.matches(notice).count(), 1);
}

#[tokio::test]
async fn a_generated_figure_carries_the_attribution_beside_the_libraries() {
    let html = render_standalone_figure(
        "show_chart",
        json!({"data": {
            "type": "line",
            "labels": ["A"],
            "datasets": [{"label": "S", "data": [1.0]}]
        }}),
    )
    .await
    .unwrap();

    let notice = common::ATTRIBUTION_COMMENT;
    assert_eq!(html.matches(notice).count(), 1);

    let head = html.find("<head>").expect("figure has a head");
    let at = html.find(notice).expect("figure carries the notice");
    let body = html.find("<body").expect("figure has a body");
    assert!(head < at && at < body, "the notice belongs in the head");

    // The figure really does inline the library the notice covers.
    assert!(html.contains("Chart.js v4.5.0"));
}

include!("tests_extra.rs");
include!("tests_dashboard.rs");
include!("tests_distributions.rs");
include!("tests_cartesian.rs");
