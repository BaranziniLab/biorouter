// Tests for `render_dashboard` — combining several figures into one artifact.

// `STANDARD` and the `Engine` trait already reach here through `use super::*`.
use base64::engine::general_purpose::STANDARD as B64;

/// Decode the `ui://` HTML blob out of a tool result.
fn dashboard_html(result: &CallToolResult) -> String {
    if let RawContent::Resource(resource) = &*result.content[0] {
        if let ResourceContents::BlobResourceContents { blob, .. } = &resource.resource {
            return String::from_utf8(B64.decode(blob).unwrap()).unwrap();
        }
    }
    panic!("expected a blob resource as the first content item");
}

/// Decode every embedded panel document out of an assembled report.
fn panel_documents(html: &str) -> Vec<String> {
    html.split("<script type=\"text/plain\" id=\"autovis-panel-")
        .skip(1)
        .filter_map(|chunk| {
            // chunk == `0">BASE64</script>…`; take the text between the tags.
            let (_attrs, rest) = chunk.split_once('>')?;
            let (b64, _tail) = rest.split_once("</script>")?;
            Some(String::from_utf8(B64.decode(b64.trim()).unwrap()).unwrap())
        })
        .collect()
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// The `var DATA = {…};` payload the report's runtime reads.
fn dashboard_data(html: &str) -> Value {
    let (_, after) = html.split_once("var DATA = ").expect("report must embed DATA");
    let line = after.lines().next().unwrap();
    serde_json::from_str(line.trim_end().trim_end_matches(';')).expect("DATA must be valid JSON")
}

fn params_from(value: serde_json::Value) -> Parameters<RenderDashboardParams> {
    Parameters(serde_json::from_value(value).unwrap())
}

fn volcano_figure() -> serde_json::Value {
    json!({
        "tool": "render_volcano",
        "params": {
            "data": {
                "points": [
                    {"label": "MYC", "log2fc": 2.4, "negLog10P": 4.0},
                    {"label": "TP53", "log2fc": -1.8, "negLog10P": 2.7},
                ]
            }
        }
    })
}

fn bar_chart_figure() -> serde_json::Value {
    json!({
        "tool": "show_chart",
        "params": {
            "data": {
                "type": "bar",
                "title": "Counts",
                "labels": ["A", "B"],
                "datasets": [{"label": "Sample", "data": [3.0, 7.0]}]
            }
        }
    })
}

fn mermaid_figure() -> serde_json::Value {
    json!({
        "tool": "render_mermaid",
        "params": { "mermaid_code": "graph TD; A-->B;" }
    })
}

// ---------------------------------------------------------------------------
// Registration — the tool is useless if the model never sees it
// ---------------------------------------------------------------------------

#[test]
fn test_dashboard_tool_is_registered_and_advertised() {
    let router = AutoVisualiserRouter::new();
    assert!(router.tool_router.has_route("render_dashboard"));

    // Every figure the dashboard can dispatch to must still BE a tool — it is
    // just no longer an ADVERTISED one. The invariant did not weaken when the
    // 32 moved behind `render_figure`; it moved to `figure_router`, which is
    // also what `describe_figure` reads their schemas out of.
    for name in [
        "show_chart",
        "render_volcano",
        "render_heatmap",
        "render_mermaid",
        "render_choropleth",
    ] {
        assert!(
            router.figure_router.has_route(name),
            "{name} is not registered"
        );
        assert!(
            !router.tool_router.has_route(name),
            "{name} is advertised again; the 128-tool ceiling is why it must not be"
        );
    }

    // And the instructions must steer the model to combine figures.
    let instructions = router.get_info().instructions.unwrap();
    assert!(instructions.contains("render_dashboard"));
    assert!(instructions.contains("call `render_dashboard` once"));

    // The tool DESCRIPTION (not just the server instructions) must carry the
    // call-once guidance, so the model reads it before the FIRST call — this is
    // the only lever that can curb the ~20% same-turn duplicate-call rate before
    // the model emits the second call.
    let tools = router.tool_router.list_all();
    let dash = tools
        .iter()
        .find(|t| t.name.as_ref() == "render_dashboard")
        .expect("render_dashboard must be listed");
    let desc = dash.description.as_deref().unwrap_or_default();
    assert!(desc.to_lowercase().contains("once"), "description must say call it once");
    assert!(desc.contains("finalise or confirm"));
}

// ---------------------------------------------------------------------------
// Tool-name normalization + slugs
// ---------------------------------------------------------------------------

#[test]
fn test_normalize_tool_name_is_lenient() {
    assert_eq!(normalize_tool_name("render_volcano"), "render_volcano");
    assert_eq!(normalize_tool_name("volcano"), "render_volcano");
    assert_eq!(normalize_tool_name("Volcano"), "render_volcano");
    assert_eq!(normalize_tool_name(" render-volcano "), "render_volcano");
    assert_eq!(normalize_tool_name("kaplan_meier"), "render_kaplan_meier");
    assert_eq!(normalize_tool_name("show_chart"), "show_chart");
    assert_eq!(normalize_tool_name("chart"), "show_chart");
    assert_eq!(normalize_tool_name("render_chart"), "show_chart");
}

#[test]
fn test_slugify() {
    assert_eq!(slugify("Tumour vs Normal"), "tumour-vs-normal");
    assert_eq!(slugify("  A/B  test!  "), "a-b-test");
    assert_eq!(slugify("!!!"), "report");
    assert_eq!(slugify(""), "report");
}

// ---------------------------------------------------------------------------
// Figure spec deserialization (models emit several shapes)
// ---------------------------------------------------------------------------

#[test]
fn test_figure_accepts_tool_and_params() {
    let f: DashboardFigure =
        serde_json::from_value(json!({"tool": "render_volcano", "params": {"data": {}}})).unwrap();
    assert_eq!(f.tool, "render_volcano");
    assert!(f.params["data"].is_object());
}

#[test]
fn test_figure_accepts_type_alias_and_bare_args() {
    // `type` instead of `tool`, and the tool's args inline rather than nested.
    let f: DashboardFigure =
        serde_json::from_value(json!({"type": "volcano", "data": {"points": []}})).unwrap();
    assert_eq!(f.tool, "volcano");
    assert!(f.params["data"]["points"].is_array());
}

#[test]
fn test_figure_accepts_stringified_object() {
    let f: DashboardFigure = serde_json::from_value(json!(
        "{\"tool\": \"show_chart\", \"params\": {\"data\": {}}}"
    ))
    .unwrap();
    assert_eq!(f.tool, "show_chart");
}

#[test]
fn test_figure_requires_a_tool() {
    let err = serde_json::from_value::<DashboardFigure>(json!({"params": {}})).unwrap_err();
    assert!(err.to_string().contains("needs a `tool`"));
}

// ---------------------------------------------------------------------------
// A panel names `render_figure`, because that is the only figure tool the model
// can see (#142)
//
// The 32 per-figure tools are still REGISTERED but no longer ADVERTISED, so
// `render_figure` is the one figure name in the model's roster — and therefore
// the one it writes into a panel. Measured live on Versa: asked for a dashboard
// with one chart, the model sent
// `{"tool": "render_figure", "params": {"kind": "chart", "data": …}}` and the
// whole call failed with "Every figure in the dashboard failed to render: …
// Unknown visualization 'render_figure'", whose advice named three tools it
// cannot call. `render_dashboard` was unusable from a real chat.
// ---------------------------------------------------------------------------

/// One payload, so the spellings below can be compared against each other
/// rather than against a hand-written expectation that could encode the bug.
fn chart_payload() -> serde_json::Value {
    json!({
        "type": "bar",
        "title": "Counts",
        "labels": ["A", "B"],
        "datasets": [{"label": "Sample", "data": [3.0, 7.0]}]
    })
}

async fn one_panel_report(figure: serde_json::Value) -> Result<CallToolResult, ErrorData> {
    AutoVisualiserRouter::new()
        .render_dashboard(params_from(json!({
            "title": "Made-up counts",
            "panels": [{"title": "Counts", "figure": figure}],
        })))
        .await
}

/// The exact shape the live model sent.
///
/// Fails the shipped implementation, whose dispatch table had no `render_figure`
/// arm — and also fails a fix that teaches only `DashboardFigure` about `kind`
/// without routing `render_figure` to a real figure tool.
#[tokio::test]
async fn test_panel_accepts_the_render_figure_tool_with_a_kind() {
    let result = one_panel_report(json!({
        "tool": "render_figure",
        "params": {"kind": "chart", "data": chart_payload()},
    }))
    .await
    .expect("a `render_figure` panel must render");

    let html = dashboard_html(&result);
    assert_eq!(
        panel_documents(&html).len(),
        1,
        "the panel must have rendered"
    );
    assert!(!html.contains("Unknown visualization"));
    let receipt = result.structured_content.as_ref().unwrap();
    assert_eq!(receipt["figuresCreated"], 1);
    assert_eq!(receipt["figuresFailed"], 0);
}

/// A model that writes the `render_figure` arguments straight onto the figure
/// object, with no `params` wrapper. `DashboardFigure` already hands the
/// leftovers through as the call's arguments, so the unwrap must read whatever
/// it produced.
///
/// Fails an implementation that reaches for `figure.params["params"]["kind"]`,
/// i.e. one that assumes the nested spelling.
#[tokio::test]
async fn test_panel_accepts_a_flattened_render_figure_call() {
    let result = one_panel_report(json!({
        "tool": "render_figure",
        "kind": "chart",
        "data": chart_payload(),
    }))
    .await
    .expect("a flattened `render_figure` panel must render");
    assert_eq!(panel_documents(&dashboard_html(&result)).len(), 1);
}

/// The two spellings must draw the SAME figure, not merely both draw one — and
/// the retired name must keep working, because a dashboard call already in
/// flight (or an older transcript being re-run) uses it.
///
/// Fails an implementation that accepts `render_figure` by routing it somewhere
/// of its own — re-serializing `data`, dropping unrecognised fields, defaulting
/// the kind — which would render a panel and quietly draw the wrong thing. Also
/// fails one that REPLACES the retired names rather than adding to them.
#[tokio::test]
async fn test_render_figure_panel_matches_the_retired_tool_name_byte_for_byte() {
    let via_entry = one_panel_report(json!({
        "tool": "render_figure",
        "params": {"kind": "chart", "data": chart_payload()},
    }))
    .await
    .expect("render_figure panel");
    let via_retired = one_panel_report(json!({
        "tool": "show_chart",
        "params": {"data": chart_payload()},
    }))
    .await
    .expect("the retired tool name must still dispatch");

    let entry_panels = panel_documents(&dashboard_html(&via_entry));
    let retired_panels = panel_documents(&dashboard_html(&via_retired));
    assert_eq!(retired_panels.len(), 1, "the retired name rendered nothing");
    assert_eq!(
        entry_panels, retired_panels,
        "`render_figure` and the retired name must produce the same figure"
    );
}

/// `mermaid` is the one kind whose payload is NOT the underlying tool's `data`
/// argument — it takes `mermaid_code`. A panel therefore has to reshape it
/// through `figure_arguments`, exactly as the declared `render_figure` does.
///
/// Fails an implementation that unwraps the panel by hand as
/// `call_figure_tool(kind.tool_name(), json!({"data": data}))`, which draws 31
/// kinds correctly and fails only this one.
#[tokio::test]
async fn test_render_figure_panel_reshapes_mermaid_like_the_entry_point_does() {
    let via_entry = one_panel_report(json!({
        "tool": "render_figure",
        "params": {"kind": "mermaid", "data": "graph TD; A-->B;"},
    }))
    .await
    .expect("mermaid through a `render_figure` panel");
    let via_retired = one_panel_report(mermaid_figure())
        .await
        .expect("retired mermaid panel");

    assert_eq!(
        panel_documents(&dashboard_html(&via_entry)),
        panel_documents(&dashboard_html(&via_retired)),
    );
}

/// A kind the enum does not have must say so against `kind`, and point at the
/// tool that lists the kinds — not fall through to "Unknown visualization",
/// which talks about a tool name the model did not get wrong.
#[tokio::test]
async fn test_render_figure_panel_with_an_unknown_kind_says_so() {
    let err = one_panel_report(json!({
        "tool": "render_figure",
        "params": {"kind": "sparkline", "data": {}},
    }))
    .await
    .unwrap_err();

    let message = err.message.to_string();
    assert!(message.contains("Unknown figure kind"), "{message}");
    assert!(message.contains("sparkline"), "{message}");
    assert!(message.contains("describe_figure"), "{message}");
}

// ---------------------------------------------------------------------------
// A rejected payload must name a call the model can make (#150 item 1)
// ---------------------------------------------------------------------------

/// `render_volcano` still dispatches but is advertised nowhere, so naming it
/// hands the model an identifier it cannot call — it retries the same rejected
/// shape or gives up.
///
/// Fails the shipped implementation, which interpolated the dispatch table's own
/// literal (`"`{}` arguments are invalid: {e}", $tool`).
#[tokio::test]
async fn test_bad_arguments_name_render_figure_and_the_kind() {
    let router = AutoVisualiserRouter::new();
    let err = router
        .render_figure(Parameters(RenderFigureParams {
            kind: FigureKind::Volcano,
            // `negLog10P` but no `log2fc` — the exact case in the issue.
            data: json!({"points": [{"label": "MYC", "negLog10P": 4.0}]}),
        }))
        .await
        .unwrap_err();

    let message = err.message.to_string();
    assert!(message.contains("log2fc"), "must still say what is wrong: {message}");
    assert!(message.contains("render_figure"), "{message}");
    assert!(message.contains("\"volcano\""), "must name the kind: {message}");
    assert!(
        !message.contains("render_volcano"),
        "the model cannot call `render_volcano`: {message}"
    );
    assert!(
        message.contains("describe_figure"),
        "must point at the schema: {message}"
    );
}

/// The same message reaches the model through a panel's error card and through
/// the assistant-audience failure list, which is where it is actually read.
#[tokio::test]
async fn test_a_failed_panel_names_render_figure_not_the_retired_tool() {
    let result = AutoVisualiserRouter::new()
        .render_dashboard(params_from(json!({
            "title": "Partial",
            "panels": [
                {"title": "Good", "figure": bar_chart_figure()},
                {"title": "Bad volcano", "figure": {
                    "tool": "render_figure",
                    "params": {"kind": "volcano", "data": {"points": [{"negLog10P": 4.0}]}},
                }},
            ],
        })))
        .await
        .expect("one good panel keeps the report alive");

    let html = dashboard_html(&result);
    assert_eq!(panel_documents(&html).len(), 1);
    assert!(html.contains("render_figure"));
    assert!(
        !html.contains("render_volcano"),
        "the error card names a tool the model cannot call"
    );

    let RawContent::Text(text) = &*result.content[1] else {
        panic!("expected assistant text");
    };
    assert!(text.text.contains("Figure 2 (Bad volcano)"), "{}", text.text);
    assert!(text.text.contains("render_figure"), "{}", text.text);
    assert!(!text.text.contains("render_volcano"), "{}", text.text);
}

/// The dashboard is not the only door into the dispatcher: Agent Drafter's
/// `ui_figure` takes a model-written tool name and calls
/// `render_standalone_figure` with it. Putting the unwrap in `call_figure_tool`
/// rather than in the panel path is what makes that door accept `render_figure`
/// too, so assert it rather than assuming it.
///
/// Fails an implementation that unwraps `render_figure` inside
/// `render_figure_fragment` — every dashboard test above would still pass.
#[tokio::test]
async fn test_the_standalone_embedding_api_accepts_render_figure_too() {
    let via_entry = render_standalone_figure(
        "render_figure",
        json!({"kind": "chart", "data": chart_payload()}),
    )
    .await
    .expect("`render_figure` through the standalone API");
    let via_retired = render_standalone_figure("show_chart", json!({"data": chart_payload()}))
        .await
        .expect("the retired name through the standalone API");
    assert_eq!(via_entry, via_retired);
}

/// The unit half of the two panel tests above: the leniency is deliberate, and
/// each accepted alias is one a model was seen to reach for.
///
/// ⚠ NOT wiring coverage. This calls `render_figure_call` directly, so it passes
/// with the `render_figure` arm of `call_figure_tool` deleted outright — the
/// unwrap could be unreachable and every assertion here would still hold. The
/// wiring is pinned above, by `test_panel_accepts_the_render_figure_tool_with_a_kind`,
/// `test_panel_accepts_a_flattened_render_figure_call`,
/// `test_render_figure_panel_matches_the_retired_tool_name_byte_for_byte`,
/// `test_render_figure_panel_reshapes_mermaid_like_the_entry_point_does`,
/// `test_render_figure_panel_with_an_unknown_kind_says_so` and
/// `test_the_standalone_embedding_api_accepts_render_figure_too`. What this adds
/// is the per-alias detail those would only fail on in aggregate.
#[test]
fn test_render_figure_call_unwraps_the_shapes_a_model_sends() {
    for payload in [
        json!({"kind": "chart", "data": {"type": "bar"}}),
        json!({"kind": "chart", "params": {"type": "bar"}}),
        json!({"kind": "chart", "arguments": {"type": "bar"}}),
        json!({"kind": "chart", "args": {"type": "bar"}}),
        // No payload key at all — the leftovers ARE the payload.
        json!({"kind": "chart", "type": "bar"}),
        // The whole object stringified. `DashboardFigure` accepts this shape at
        // the panel level and this refused it one level in, so a model that
        // stringifies nested arguments (a habit recorded in this codebase, and
        // the reason `normalize_dashboard_args` exists) got a message about
        // `kind` when the fault was the quoting.
        json!("{\"kind\": \"chart\", \"data\": {\"type\": \"bar\"}}"),
    ] {
        let (kind, data) = render_figure_call(payload.clone(), FigureVocabulary::RenderFigure)
            .unwrap_or_else(|e| panic!("{payload} was refused: {}", e.message));
        assert_eq!(kind, FigureKind::Chart, "{payload}");
        assert_eq!(data["type"], "bar", "{payload}");
    }

    // A string that is not JSON at all must say so, not fall through to a
    // complaint about a missing `kind`.
    let err =
        render_figure_call(json!("kind=chart"), FigureVocabulary::RenderFigure).unwrap_err();
    assert!(
        err.message.contains("did not parse"),
        "{}",
        err.message
    );

    // `kind` has no aliases on purpose. Two independent reasons: when a panel
    // omitted `tool`, `DashboardFigure` will already have consumed
    // `type`/`name`/`kind` hunting for the TOOL name; and in every case, a chart
    // payload's own `type: "bar"` must not be able to choose the figure.
    let err = render_figure_call(json!({"type": "chart", "data": {}}), FigureVocabulary::RenderFigure)
        .unwrap_err();
    assert!(err.message.contains("needs a `kind`"), "{}", err.message);
}

// ---------------------------------------------------------------------------
// A panel that names a KIND and no tool at all (#142, the regression the
// `render_figure` unwrap introduced)
//
// `DashboardFigure` hunts `tool`/`type`/`name`/`kind` for the TOOL name, so
// `{"kind": "mermaid", "data": "<source>"}` resolves straight to
// `render_mermaid` and never reaches the `render_figure` unwrap. Thirty-one
// kinds survive that by accident — their payload already IS the tool's `data`.
// `mermaid` is the one that needs reshaping, and it was reshaped in
// `figure_arguments`, i.e. behind a door this shape does not use. The result was
// a REFUSAL phrased against `render_figure` telling the model to go and check a
// schema that would have confirmed exactly what it had already sent.
// ---------------------------------------------------------------------------

/// Every shape a `kind`-only mermaid panel can take must RENDER, not explain
/// itself. `describe_figure` reports mermaid's `dataIs` as "the Mermaid diagram
/// source, as a string", so `{"kind": "mermaid", "data": "<source>"}` is the
/// documented payload and refusing it is a false error whatever it says.
///
/// Fails an implementation that keeps the mermaid leniency in `figure_arguments`
/// instead of on `RenderMermaidParams`, whichever way that refusal is worded.
#[tokio::test]
async fn test_a_kind_only_mermaid_panel_renders_instead_of_being_refused() {
    let canonical = one_panel_report(mermaid_figure())
        .await
        .expect("the retired spelling still renders");
    let expected = panel_documents(&dashboard_html(&canonical));

    for figure in [
        json!({"kind": "mermaid", "data": "graph TD; A-->B;"}),
        json!({"kind": "mermaid", "mermaid_code": "graph TD; A-->B;"}),
        json!({"kind": "mermaid", "params": {"data": "graph TD; A-->B;"}}),
        json!({"kind": "mermaid", "params": {"mermaid_code": "graph TD; A-->B;"}}),
        json!({"type": "mermaid", "data": "graph TD; A-->B;"}),
        // And through the `render_figure` door, which must not have regressed
        // while the leniency moved out from under it.
        json!({"tool": "render_figure", "params": {"kind": "mermaid", "data": "graph TD; A-->B;"}}),
        json!({"tool": "render_figure", "params": {"kind": "mermaid",
                                                   "data": {"mermaid_code": "graph TD; A-->B;"}}}),
    ] {
        let result = one_panel_report(figure.clone())
            .await
            .unwrap_or_else(|e| panic!("{figure} was refused: {}", e.message));
        assert_eq!(
            panel_documents(&dashboard_html(&result)),
            expected,
            "{figure} drew a different diagram"
        );
    }
}

/// The `kind`-only shape for the other 31, and for the spelling the review
/// measured working today — `{"kind": "volcano", "params": {…}}`, where `params`
/// is the per-kind tool's own arguments because `DashboardFigure` resolved the
/// kind to that tool directly.
///
/// ⚠ Fails any "fix" that drops `kind` from `DashboardFigure`'s tool-name hunt:
/// the panel would then have no tool at all, or `render_figure_call` would take
/// `params` as the payload and double-wrap it.
#[tokio::test]
async fn test_a_kind_only_panel_draws_the_same_figure_as_the_tool_name() {
    let canonical = one_panel_report(json!({
        "tool": "render_figure",
        "params": {"kind": "chart", "data": chart_payload()},
    }))
    .await
    .expect("canonical render_figure panel");
    let expected = panel_documents(&dashboard_html(&canonical));

    for figure in [
        json!({"kind": "chart", "data": chart_payload()}),
        json!({"kind": "chart", "params": {"data": chart_payload()}}),
        json!({"kind": "show_chart", "params": {"data": chart_payload()}}),
    ] {
        let result = one_panel_report(figure.clone())
            .await
            .unwrap_or_else(|e| panic!("{figure} was refused: {}", e.message));
        assert_eq!(
            panel_documents(&dashboard_html(&result)),
            expected,
            "{figure} drew a different figure"
        );
    }

    // The volcano spelling from the review, asserted against its own canonical
    // form rather than the chart's.
    let via_kind = one_panel_report(json!({
        "kind": "volcano",
        "params": {"data": {"points": [
            {"label": "MYC", "log2fc": 2.4, "negLog10P": 4.0},
            {"label": "TP53", "log2fc": -1.8, "negLog10P": 2.7},
        ]}},
    }))
    .await
    .expect("`{kind: volcano, params: {data}}` is measured working and must stay so");
    assert_eq!(
        panel_documents(&dashboard_html(&via_kind)),
        panel_documents(&dashboard_html(
            &one_panel_report(volcano_figure()).await.unwrap()
        )),
    );
}

/// A mermaid payload with no source anywhere is the one case that SHOULD fail,
/// and it must fail against `render_figure`/`kind` — not render an empty diagram
/// (which looks like the tool worked) and not name `render_mermaid` (which a
/// chat agent cannot call).
#[tokio::test]
async fn test_a_mermaid_panel_with_no_source_is_refused_in_the_models_vocabulary() {
    let result = AutoVisualiserRouter::new()
        .render_dashboard(params_from(json!({
            "title": "Partial",
            "panels": [
                {"title": "Good", "figure": bar_chart_figure()},
                {"title": "Sourceless", "figure": {"kind": "mermaid", "data": {"nodes": []}}},
            ],
        })))
        .await
        .expect("one good panel keeps the report alive");

    let html = dashboard_html(&result);
    assert_eq!(panel_documents(&html).len(), 1);
    assert!(html.contains("Mermaid diagram is its source text"), "{html}");
    assert!(html.contains("render_figure"));
    assert!(
        !html.contains("render_mermaid"),
        "the model cannot call `render_mermaid`"
    );
}

// ---------------------------------------------------------------------------
// Argument shapes real models send
//
// Every other Auto Visualiser tool takes a single `data` argument, so models
// generalise. GPT-5.5 wrapped the whole report in one and, when rejected,
// retried with the same shape — two wasted turns and no figure.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_accepts_the_report_wrapped_in_a_data_envelope() {
    let router = AutoVisualiserRouter::new();
    let result = router
        .render_dashboard(params_from(json!({
            "data": {
                "title": "RNA-seq quality and signal",
                "summary": "Even depth; MYC up, TP53 down.",
                "sections": [{
                    "title": "Overview",
                    "panels": [
                        {"title": "Depth", "figure": bar_chart_figure()},
                        {"title": "Volcano", "figure": volcano_figure()},
                    ]
                }]
            }
        })))
        .await
        .unwrap();

    let html = dashboard_html(&result);
    assert_eq!(panel_documents(&html).len(), 2);
    assert_eq!(dashboard_data(&html)["title"], "RNA-seq quality and signal");
}

#[tokio::test]
async fn test_accepts_a_stringified_data_envelope() {
    let router = AutoVisualiserRouter::new();
    let payload = json!({
        "title": "Stringified",
        "panels": [{"figure": bar_chart_figure()}]
    })
    .to_string();

    let html = dashboard_html(
        &router
            .render_dashboard(params_from(json!({ "data": payload })))
            .await
            .unwrap(),
    );
    assert_eq!(panel_documents(&html).len(), 1);
    assert_eq!(dashboard_data(&html)["title"], "Stringified");
}

#[tokio::test]
async fn test_accepts_stringified_sections() {
    let router = AutoVisualiserRouter::new();
    let sections = json!([{"panels": [{"figure": bar_chart_figure()}]}]).to_string();

    let html = dashboard_html(
        &router
            .render_dashboard(params_from(json!({
                "title": "Stringified sections",
                "sections": sections
            })))
            .await
            .unwrap(),
    );
    assert_eq!(panel_documents(&html).len(), 1);
}

#[test]
fn test_plain_shape_still_wins_over_a_data_field() {
    // A report that legitimately has a `title` is never unwrapped, even if some
    // stray `data` key rides along.
    let params: RenderDashboardParams = serde_json::from_value(json!({
        "title": "Real title",
        "data": {"title": "Decoy"},
        "panels": []
    }))
    .unwrap();
    assert_eq!(params.title, "Real title");
}

#[test]
fn test_missing_title_error_names_the_data_envelope() {
    let err = serde_json::from_value::<RenderDashboardParams>(json!({"summary": "no title"}))
        .unwrap_err();
    assert!(err.to_string().contains("not wrapped in a `data` argument"));
}

// ---------------------------------------------------------------------------
// Happy path: several figures become one artifact
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_dashboard_combines_multiple_figures() {
    let router = AutoVisualiserRouter::new();
    let result = router
        .render_dashboard(params_from(json!({
            "title": "Tumour vs normal",
            "subtitle": "RNA-seq, n=48",
            "summary": "412 genes pass FDR < 0.05.",
            "sections": [{
                "title": "Genome-wide signal",
                "description": "Effect size against significance.",
                "panels": [
                    {"title": "Volcano plot", "caption": "Above the line: FDR < 0.05.",
                     "notes": "Wald test.", "figure": volcano_figure()},
                    {"title": "Top genes", "width": "half", "figure": bar_chart_figure()},
                ]
            }],
            "footer": "GENCODE v44."
        })))
        .await
        .unwrap();

    let html = dashboard_html(&result);

    // One artifact, two embedded panel documents.
    assert_eq!(panel_documents(&html).len(), 2);

    // The prose all made it into the page data.
    assert!(html.contains("Tumour vs normal"));
    assert!(html.contains("RNA-seq, n=48"));
    assert!(html.contains("412 genes pass FDR"));
    assert!(html.contains("Volcano plot"));
    assert!(html.contains("Above the line"));
    assert!(html.contains("Wald test."));
    assert!(html.contains("GENCODE v44."));

    // The assistant is told what happened, and told not to re-render on success.
    if let RawContent::Text(text) = &*result.content[1] {
        assert!(text.text.contains("2 figures"));
        assert!(text.text.contains("The report is complete"));
        assert!(text.text.contains("ready for the artifact panel"));
        assert!(text.text.contains("Inspect the existing artifact"));
        assert!(!text.text.contains("already displayed"));
        assert!(text.text.contains("finalise or confirm"));
    } else {
        panic!("expected an assistant-audience text note");
    }
}

#[tokio::test]
async fn test_dashboard_uri_and_frame_size() {
    let router = AutoVisualiserRouter::new();
    let result = router
        .render_dashboard(params_from(json!({
            "title": "Cohort Overview",
            "panels": [{"figure": bar_chart_figure()}]
        })))
        .await
        .unwrap();

    if let RawContent::Resource(resource) = &*result.content[0] {
        if let ResourceContents::BlobResourceContents { uri, meta, .. } = &resource.resource {
            assert_eq!(uri, "ui://dashboard/cohort-overview");
            let meta = meta.as_ref().expect("frame size meta");
            assert!(meta.0.contains_key("mcpui.dev/ui-preferred-frame-size"));
            return;
        }
    }
    panic!("expected a blob resource");
}

// ---------------------------------------------------------------------------
// Asset de-duplication — the reason panels are rendered in fragment mode
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_panels_carry_placeholders_not_libraries() {
    let router = AutoVisualiserRouter::new();
    let result = router
        .render_dashboard(params_from(json!({
            "title": "Two charts",
            "panels": [{"figure": bar_chart_figure()}, {"figure": bar_chart_figure()}]
        })))
        .await
        .unwrap();
    let html = dashboard_html(&result);

    // Each panel document has exactly one asset placeholder and no library:
    // Chart.js alone is ~200 KB, so a panel carrying it could not be this small.
    let panels = panel_documents(&html);
    assert_eq!(panels.len(), 2);
    for panel in &panels {
        assert_eq!(count_occurrences(panel, common::ASSET_PLACEHOLDER), 1);
        assert!(
            panel.len() < 80_000,
            "panel must not inline Chart.js ({} bytes); the report stores it once",
            panel.len()
        );
    }

    // Chart.js appears exactly once in the whole report, in the shared store.
    assert_eq!(
        count_occurrences(&html, "data-autovis-asset=\"chartjs\""),
        1,
        "Chart.js must be stored once, not once per panel"
    );
}

#[tokio::test]
async fn test_shared_store_holds_each_library_once_across_types() {
    let router = AutoVisualiserRouter::new();
    let result = router
        .render_dashboard(params_from(json!({
            "title": "Mixed",
            "panels": [
                {"figure": bar_chart_figure()},   // Chart.js
                {"figure": volcano_figure()},     // Chart.js again
                {"figure": mermaid_figure()},     // Mermaid
            ]
        })))
        .await
        .unwrap();
    let html = dashboard_html(&result);

    assert_eq!(count_occurrences(&html, "data-autovis-asset=\"chartjs\""), 1);
    assert_eq!(count_occurrences(&html, "data-autovis-asset=\"mermaid\""), 1);
    assert_eq!(panel_documents(&html).len(), 3);
}

#[tokio::test]
async fn test_dedup_keeps_the_report_far_smaller_than_naive_composition() {
    let router = AutoVisualiserRouter::new();

    // A standalone Mermaid figure inlines the whole ~3.3 MB library.
    let standalone = dashboard_html(
        &router
            .render_mermaid(Parameters(RenderMermaidParams {
                mermaid_code: "graph TD; A-->B;".to_string(),
            }))
            .await
            .unwrap(),
    );

    // Three Mermaid panels in one report must not cost three copies of it.
    let report = dashboard_html(
        &router
            .render_dashboard(params_from(json!({
                "title": "Three diagrams",
                "panels": [
                    {"figure": mermaid_figure()},
                    {"figure": mermaid_figure()},
                    {"figure": mermaid_figure()},
                ]
            })))
            .await
            .unwrap(),
    );

    assert!(
        report.len() < standalone.len() * 2,
        "3-panel report ({} bytes) should stay near one library copy, not three \
         (a standalone figure is {} bytes)",
        report.len(),
        standalone.len()
    );
}

#[tokio::test]
async fn test_panel_assets_are_declared_for_the_runtime() {
    let router = AutoVisualiserRouter::new();
    let html = dashboard_html(
        &router
            .render_dashboard(params_from(json!({
                "title": "Mixed",
                "panels": [{"figure": bar_chart_figure()}, {"figure": mermaid_figure()}]
            })))
            .await
            .unwrap(),
    );

    // The page data tells the runtime which stored libraries each panel needs.
    let panels = &dashboard_data(&html)["sections"][0]["panels"];
    assert_eq!(panels[0]["assets"], json!(["chartjs"]));
    assert_eq!(panels[1]["assets"], json!(["mermaid"]));
    assert_eq!(panels[0]["index"], json!(0));
    assert_eq!(panels[1]["index"], json!(1));
}

#[tokio::test]
async fn test_fragment_mode_does_not_leak_into_standalone_figures() {
    // Rendering a dashboard must not leave later standalone figures assetless.
    let router = AutoVisualiserRouter::new();
    router
        .render_dashboard(params_from(json!({
            "title": "First",
            "panels": [{"figure": bar_chart_figure()}]
        })))
        .await
        .unwrap();

    let standalone = dashboard_html(
        &router
            .show_chart(Parameters(
                serde_json::from_value(bar_chart_figure()["params"].clone()).unwrap(),
            ))
            .await
            .unwrap(),
    );
    assert!(!standalone.contains(common::ASSET_PLACEHOLDER));
    assert!(
        standalone.len() > 150_000,
        "a standalone chart must still inline Chart.js, got {} bytes",
        standalone.len()
    );
}

// ---------------------------------------------------------------------------
// Structure, layout and documentation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_flat_panels_shorthand_becomes_one_section() {
    let router = AutoVisualiserRouter::new();
    let html = dashboard_html(
        &router
            .render_dashboard(params_from(json!({
                "title": "Quick look",
                "panels": [{"title": "Counts", "figure": bar_chart_figure()}]
            })))
            .await
            .unwrap(),
    );
    // One implicit, untitled section wrapping the single panel.
    let data = dashboard_data(&html);
    assert_eq!(data["sections"].as_array().unwrap().len(), 1);
    assert!(data["sections"][0]["title"].is_null());
    assert_eq!(data["sections"][0]["panels"].as_array().unwrap().len(), 1);
    assert_eq!(panel_documents(&html).len(), 1);
}

#[tokio::test]
async fn test_panel_width_defaults_to_full_and_accepts_half() {
    let router = AutoVisualiserRouter::new();
    let html = dashboard_html(
        &router
            .render_dashboard(params_from(json!({
                "title": "Widths",
                "panels": [
                    {"figure": bar_chart_figure()},
                    {"width": "Half", "figure": bar_chart_figure()},
                ]
            })))
            .await
            .unwrap(),
    );
    let panels = &dashboard_data(&html)["sections"][0]["panels"];
    assert_eq!(panels[0]["width"], "full");
    assert_eq!(panels[1]["width"], "half");
}

#[tokio::test]
async fn test_rejects_unknown_width() {
    let router = AutoVisualiserRouter::new();
    let err = router
        .render_dashboard(params_from(json!({
            "title": "Widths",
            "panels": [{"width": "third", "figure": bar_chart_figure()}]
        })))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    assert!(err.message.contains("must be 'full' or 'half'"));
}

// ---------------------------------------------------------------------------
// Validation + partial failure
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_rejects_empty_title() {
    let router = AutoVisualiserRouter::new();
    let err = router
        .render_dashboard(params_from(json!({
            "title": "   ",
            "panels": [{"figure": bar_chart_figure()}]
        })))
        .await
        .unwrap_err();
    assert!(err.message.contains("non-empty `title`"));
}

#[tokio::test]
async fn test_rejects_no_panels() {
    let router = AutoVisualiserRouter::new();
    let err = router
        .render_dashboard(params_from(json!({ "title": "Empty" })))
        .await
        .unwrap_err();
    assert!(err.message.contains("at least one figure"));
}

#[tokio::test]
async fn test_rejects_too_many_panels() {
    let router = AutoVisualiserRouter::new();
    let panels: Vec<_> = (0..MAX_PANELS + 1)
        .map(|_| json!({"figure": bar_chart_figure()}))
        .collect();
    let err = router
        .render_dashboard(params_from(json!({"title": "Too many", "panels": panels})))
        .await
        .unwrap_err();
    assert!(err.message.contains("dashboard panels"));
}

#[tokio::test]
async fn partial_dashboard_receipt_preserves_failure_guidance_after_a_long_title() {
    let router = AutoVisualiserRouter::new();
    let result = router
        .render_dashboard(params_from(json!({
            "title": "Δ".repeat(2_000),
            "panels": [
                {"title": "Good", "figure": bar_chart_figure()},
                {"title": "Long panel title ".repeat(100), "figure": {
                    "tool": "show_chart", "params": {"data": {"type":"bar", "datasets":[]}}
                }}
            ]
        })))
        .await
        .unwrap();
    let receipt = result.structured_content.as_ref().unwrap();
    assert_eq!(receipt["status"], "created_with_errors");
    assert_eq!(receipt["figuresCreated"], 1);
    assert_eq!(receipt["figuresFailed"], 1);
    assert_eq!(receipt["failures"][0]["figure"], 2);
    assert_eq!(receipt["failures"][0]["tool"], "show_chart");
    assert!(receipt["failures"][0]["error"].as_str().unwrap().contains("at least one dataset"));
    assert!(receipt["recovery"].as_str().unwrap().contains("whole report"));
    assert!(serde_json::to_string(receipt).unwrap().len() < 3_000);
    assert_eq!(panel_documents(&dashboard_html(&result)).len(), 1);
}

#[tokio::test]
async fn dashboard_receipt_keeps_guidance_for_a_successful_long_title() {
    let result = AutoVisualiserRouter::new()
        .render_dashboard(params_from(json!({
            "title": "Δ".repeat(2_000),
            "panels": [{"figure": bar_chart_figure()}]
        })))
        .await
        .unwrap();
    let receipt = result.structured_content.as_ref().unwrap();
    assert_eq!(receipt["status"], "created");
    assert_eq!(receipt["figuresCreated"], 1);
    assert_eq!(receipt["figuresFailed"], 0);
    assert_eq!(receipt["failuresOmitted"], 0);
    assert_eq!(receipt["failures"], json!([]));
    assert!(receipt["recovery"].as_str().unwrap().contains("Inspect the existing artifact"));
    assert!(serde_json::to_string(receipt).unwrap().len() < 1_000);
}

#[tokio::test]
async fn partial_dashboard_receipt_bounds_errors_and_numbers_them_across_sections() {
    let bad_panels: Vec<_> = (1..MAX_PANELS).map(|_| json!({
        "figure": {"tool": "unknown_🧬".repeat(200), "params": {}}
    })).collect();
    let result = AutoVisualiserRouter::new()
        .render_dashboard(params_from(json!({
            "title": "Partial",
            "sections": [
                {"title": "Good", "panels": [{"figure": bar_chart_figure()}]},
                {"title": "Failures", "panels": bad_panels}
            ]
        })))
        .await
        .unwrap();
    let receipt = result.structured_content.as_ref().unwrap();
    assert_eq!(receipt["status"], "created_with_errors");
    assert_eq!(receipt["figuresCreated"], 1);
    assert_eq!(receipt["figuresFailed"], 23);
    assert_eq!(receipt["failuresOmitted"], 15);
    let errors = receipt["failures"].as_array().unwrap();
    assert_eq!(errors.len(), 8);
    for (index, error) in errors.iter().enumerate() {
        assert_eq!(error["figure"], index + 2);
        assert!(error["tool"].as_str().unwrap().chars().count() <= 64);
        assert!(error["error"].as_str().unwrap().chars().count() <= 128);
        assert_eq!(error["detailsTruncated"], true);
    }
    assert!(serde_json::to_string(receipt).unwrap().len() < 12_000);
    assert_eq!(panel_documents(&dashboard_html(&result)).len(), 1);
}

#[tokio::test]
async fn test_unknown_tool_fails_only_its_own_panel() {
    let router = AutoVisualiserRouter::new();
    let result = router
        .render_dashboard(params_from(json!({
            "title": "Partial",
            "panels": [
                {"title": "Good", "figure": bar_chart_figure()},
                {"title": "Bad", "figure": {"tool": "render_sparkline", "params": {}}},
            ]
        })))
        .await
        .unwrap();

    let html = dashboard_html(&result);
    // The good panel still renders; the bad one becomes an error card in-page.
    assert_eq!(panel_documents(&html).len(), 1);
    assert!(html.contains("Unknown visualization 'render_sparkline'"));

    // ...and the advice must be a call the model can actually make. This used to
    // read "Use one of the Auto Visualiser render_* tool names, e.g.
    // render_volcano, render_heatmap, show_chart" — three names absent from a
    // chat agent's roster, which is #142's complaint one layer above the
    // argument errors #150 is about. Both halves are asserted, because dropping
    // the retired names while saying nothing usable in their place would pass a
    // "does not contain render_volcano" check and still strand the model.
    assert!(
        html.contains("render_figure"),
        "must name the tool that draws figures: {html}"
    );
    assert!(
        html.contains("volcano") && html.contains("heatmap"),
        "must list the kinds to choose from"
    );
    assert!(
        html.contains("describe_figure"),
        "must point at the schema tool"
    );
    for retired in ["render_volcano", "render_heatmap", "show_chart"] {
        assert!(
            !html.contains(retired),
            "`{retired}` is not in a chat agent's roster; suggesting it is a dead end"
        );
    }

    // ...and the model is told precisely which figure to fix.
    if let RawContent::Text(text) = &*result.content[1] {
        assert!(text.text.contains("1 figure(s) could not be rendered"));
        assert!(text.text.contains("Figure 2 (Bad)"));
    } else {
        panic!("expected assistant text");
    }
}

#[tokio::test]
async fn test_bad_figure_arguments_fail_only_their_panel() {
    let router = AutoVisualiserRouter::new();
    let result = router
        .render_dashboard(params_from(json!({
            "title": "Partial",
            "panels": [
                {"title": "Good", "figure": bar_chart_figure()},
                // show_chart requires at least one dataset.
                {"title": "Empty chart", "figure": {"tool": "show_chart",
                    "params": {"data": {"type": "bar", "datasets": []}}}},
            ]
        })))
        .await
        .unwrap();

    let html = dashboard_html(&result);
    assert_eq!(panel_documents(&html).len(), 1);
    assert!(html.contains("at least one dataset"));
}

#[tokio::test]
async fn test_all_panels_failing_is_a_tool_error() {
    let router = AutoVisualiserRouter::new();
    let err = router
        .render_dashboard(params_from(json!({
            "title": "All bad",
            "panels": [{"figure": {"tool": "render_nonsense", "params": {}}}]
        })))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    assert!(err.message.contains("Every figure in the dashboard failed"));
}

#[tokio::test]
async fn test_dashboard_theme_param_bakes_a_locked_theme() {
    let router = AutoVisualiserRouter::new();

    // Explicit light/dark lock `window.__BR_VIZ_THEME__` into the report so the
    // preview and the expanded view render identically regardless of the host.
    for theme in ["light", "dark"] {
        let result = router
            .render_dashboard(params_from(json!({
                "title": "Themed report",
                "theme": theme,
                "panels": [{"title": "A", "figure": bar_chart_figure()}],
            })))
            .await
            .unwrap();
        let html = dashboard_html(&result);
        assert!(
            html.contains(&format!("window.__BR_VIZ_THEME__=\"{theme}\"")),
            "theme={theme} must bake a locked __BR_VIZ_THEME__"
        );
    }

    // Default (auto) bakes NO locked theme — the report follows the app's theme.
    let auto = router
        .render_dashboard(params_from(json!({
            "title": "Themed report",
            "panels": [{"title": "A", "figure": bar_chart_figure()}],
        })))
        .await
        .unwrap();
    // The runtime's panel-propagation helper contains `__BR_VIZ_THEME__=' +` (JS
    // concatenation); a *baked* literal is the double-quoted form, which must be absent.
    assert!(!dashboard_html(&auto).contains("__BR_VIZ_THEME__=\""));
}

#[tokio::test]
async fn test_rejects_both_sections_and_panels() {
    let router = AutoVisualiserRouter::new();
    let err = router
        .render_dashboard(params_from(json!({
            "title": "Conflict",
            "sections": [{"panels": [{"figure": bar_chart_figure()}]}],
            "panels": [{"figure": bar_chart_figure()}]
        })))
        .await
        .unwrap_err();
    assert!(err.message.contains("not both"));
}

#[tokio::test]
async fn test_rejects_overlong_prose() {
    let router = AutoVisualiserRouter::new();
    let err = router
        .render_dashboard(params_from(json!({
            "title": "Long",
            "summary": "x".repeat(MAX_PROSE_LEN + 1),
            "panels": [{"figure": bar_chart_figure()}]
        })))
        .await
        .unwrap_err();
    assert!(err.message.contains("too long"));
}

// ---------------------------------------------------------------------------
// Escaping — user prose and figure data are both model-influenced
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_prose_cannot_break_out_of_the_data_script() {
    let router = AutoVisualiserRouter::new();
    let html = dashboard_html(
        &router
            .render_dashboard(params_from(json!({
                "title": "Escaping",
                "summary": "</script><img src=x onerror=alert(1)>",
                "panels": [{"title": "</script>", "figure": bar_chart_figure()}]
            })))
            .await
            .unwrap(),
    );

    // The payload survives only as an escaped JS string literal.
    let (_, after) = html.split_once("var DATA = ").unwrap();
    let data_line = after.lines().next().unwrap();
    assert!(!data_line.contains("</script>"));
    assert!(!data_line.contains("<img"));
    assert!(data_line.contains("\\u003c"));
}

#[tokio::test]
async fn test_title_cannot_inject_a_template_placeholder() {
    let router = AutoVisualiserRouter::new();
    let html = dashboard_html(
        &router
            .render_dashboard(params_from(json!({
                "title": "{{PANEL_STORE}}",
                "panels": [{"figure": bar_chart_figure()}]
            })))
            .await
            .unwrap(),
    );
    // Exactly one panel store, and the title is inert text in <title>.
    assert_eq!(panel_documents(&html).len(), 1);
    assert!(html.contains("&#123;&#123;PANEL_STORE}}"));
}

#[tokio::test]
async fn test_asset_placeholder_literal_is_substituted_in_the_runtime() {
    let router = AutoVisualiserRouter::new();
    let html = dashboard_html(
        &router
            .render_dashboard(params_from(json!({
                "title": "Runtime",
                "panels": [{"figure": bar_chart_figure()}]
            })))
            .await
            .unwrap(),
    );
    // The template's `html.replace('{{ASSET_PLACEHOLDER}}', …)` must have been
    // rewritten to the real sentinel — otherwise no panel ever loads. It arrives
    // `<`-escaped so the literal never breaks out of the surrounding script.
    assert!(!html.contains("{{ASSET_PLACEHOLDER}}"));
    // `<` is escaped so the sentinel literal can never close the script it sits
    // in; JS unescapes it back to `<!--AUTOVIS_ASSETS-->` before matching.
    assert!(html.contains("replace(\"\\u003c!--AUTOVIS_ASSETS-->\""));

    // And the panel it will splice into really does contain that sentinel.
    assert!(panel_documents(&html)[0].contains(common::ASSET_PLACEHOLDER));
}

// ---------------------------------------------------------------------------
// Fixture generator: writes a realistic multi-figure report so the rendered page
// can be opened in a real browser. Not part of the normal run.
//
//   AUTOVIS_DUMP=/tmp/report.html cargo test -p biorouter-mcp --lib \
//     autovisualiser::tests::dump_sample_dashboard -- --ignored --nocapture
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "fixture generator; writes HTML to $AUTOVIS_DUMP"]
async fn dump_sample_dashboard() {
    let out = std::env::var("AUTOVIS_DUMP").expect("set AUTOVIS_DUMP to an output path");
    let router = AutoVisualiserRouter::new();

    let result = router
        .render_dashboard(params_from(json!({
            "title": "Tumour vs normal: differential expression",
            "subtitle": "Bulk RNA-seq · 48 paired samples · GENCODE v44",
            "summary": "Across 18,204 tested genes, **412 pass FDR < 0.05**. The signal is \
                        dominated by proliferation programmes: *MYC*, *CDK4* and *CCND1* are all \
                        strongly up-regulated in tumour tissue.\n\n\
                        - Library sizes are even across the two arms.\n\
                        - No chromosome shows a positional artefact.\n\
                        - Survival separates cleanly on the `MYC`-high stratum.",
            "sections": [
                {
                    "title": "Quality control",
                    "description": "Before interpreting effect sizes, confirm the libraries are \
                                    comparable and the count distribution is well behaved.",
                    "panels": [
                        {
                            "title": "Library size per sample",
                            "caption": "Total mapped reads. Even coverage across arms.",
                            "width": "half",
                            "figure": {"tool": "show_chart", "params": {"data": {
                                "type": "bar",
                                "labels": ["S1", "S2", "S3", "S4", "S5", "S6"],
                                "datasets": [{"label": "Million reads",
                                              "data": [31.2, 29.8, 33.1, 30.4, 32.7, 29.1]}]
                            }}}
                        },
                        {
                            "title": "Count distribution",
                            "caption": "log10 counts per gene, pooled across samples.",
                            "width": "half",
                            "figure": {"tool": "render_histogram", "params": {"data": {
                                "values": [1.2, 2.4, 2.9, 3.1, 3.3, 3.4, 3.6, 3.8, 4.0, 4.1,
                                           4.2, 4.4, 4.5, 4.7, 5.0, 5.2, 5.6, 6.1, 2.8, 3.9],
                                "title": "log10(count)"
                            }}}
                        }
                    ]
                },
                {
                    "title": "Genome-wide signal",
                    "description": "Effect size against statistical significance, then position \
                                    along the genome.",
                    "panels": [
                        {
                            "title": "Volcano plot",
                            "caption": "Points beyond the dashed thresholds pass **FDR < 0.05** \
                                        with |log2FC| > 1.",
                            "notes": "Wald test on DESeq2 shrunken effect sizes, \
                                      Benjamini-Hochberg correction across 18,204 genes.",
                            "figure": volcano_figure()
                        },
                        {
                            "title": "Sample correlation",
                            "caption": "Spearman rho between the six representative samples.",
                            "width": "half",
                            "figure": {"tool": "render_heatmap", "params": {"data": {
                                "xLabels": ["S1", "S2", "S3", "S4"],
                                "yLabels": ["S1", "S2", "S3", "S4"],
                                "values": [[1.0, 0.91, 0.62, 0.60],
                                           [0.91, 1.0, 0.64, 0.61],
                                           [0.62, 0.64, 1.0, 0.93],
                                           [0.60, 0.61, 0.93, 1.0]]
                            }}}
                        },
                        {
                            "title": "Analysis pipeline",
                            "caption": "How the counts reached this report.",
                            "width": "half",
                            "figure": {"tool": "render_mermaid", "params": {
                                "mermaid_code": "graph TD; FASTQ-->STAR; STAR-->featureCounts; \
                                                 featureCounts-->DESeq2; DESeq2-->Report;"
                            }}
                        }
                    ]
                }
            ],
            "footer": "Counts from GENCODE v44. Thresholds are the DESeq2 defaults. \
                       Raw data under restricted access."
        })))
        .await
        .unwrap();

    let html = dashboard_html(&result);
    std::fs::write(&out, &html).unwrap();
    println!("wrote {} bytes to {out}", html.len());
}

// ---------------------------------------------------------------------------
// The consolidation guards: 33 declared tools became 3, and nothing was lost
// ---------------------------------------------------------------------------

/// The ceiling this consolidation exists for. Azure and OpenAI reject a
/// `tools` array longer than 128 with a non-retryable 400, and Auto Visualiser
/// alone was 33 of the 130 built-in tools. A future contributor re-adding a
/// `#[tool]` attribute to a figure method fails here rather than silently
/// re-inflating the surface and killing every turn on those providers.
#[test]
fn exactly_three_tools_are_advertised() {
    let router = AutoVisualiserRouter::new();
    let mut advertised: Vec<String> = router
        .tool_router
        .list_all()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect();
    advertised.sort();
    assert_eq!(
        advertised,
        ["describe_figure", "render_dashboard", "render_figure"],
        "the advertised Auto Visualiser surface changed"
    );
}

/// The three tables agree by construction — `figure_kinds!` generates the enum,
/// the slug and the tool name together — so what this actually checks is the
/// join to the ROUTER: that each generated tool name is a registered `#[tool]`,
/// and that no registered figure tool is unnamed by any kind. A kind naming a
/// tool that is not registered would break `describe_figure`, which reads each
/// kind's schema off `figure_router`.
///
/// ⚠ It does NOT reach `call_figure_tool`'s dispatch table, which is a separate
/// hand-written list — registration and dispatch are different maps, and a name
/// can be in one and not the other.
/// `every_kind_reports_bad_arguments_against_render_figure` below covers that
/// join, by walking all 32 kinds through the dispatcher itself.
#[test]
fn every_kind_resolves_to_a_real_figure_tool() {
    let router = AutoVisualiserRouter::new();
    assert_eq!(FigureKind::ALL.len(), 32);
    for kind in FigureKind::ALL {
        assert!(
            router.figure_router.has_route(kind.tool_name()),
            "kind `{}` names `{}`, which is not a registered tool",
            kind.slug(),
            kind.tool_name()
        );
    }
    // And no figure tool is orphaned — one that no kind names would be
    // unreachable through the only door the model now has.
    let named: std::collections::HashSet<&str> =
        FigureKind::ALL.iter().map(|k| k.tool_name()).collect();
    for tool in router.figure_router.list_all() {
        assert!(
            named.contains(tool.name.as_ref()),
            "`{}` is registered but no kind reaches it",
            tool.name
        );
    }
}

/// The join `every_kind_resolves_to_a_real_figure_tool` cannot see: every kind
/// must reach a DISPATCH-TABLE arm, and that arm must phrase its refusal in the
/// chat agent's vocabulary.
///
/// This is the guard `figure_argument_error`'s fallback comment cites. Two ways
/// a kind can land in that fallback, and neither is visible to the router check
/// above: its `tool_name()` is missing from the hand-written dispatch table (it
/// then falls to `unknown_figure_error` instead), or the table dispatches it
/// under a literal no kind claims (it is then named back at a model that cannot
/// call it — #150's exact failure).
///
/// An empty payload is used because every figure's parameter struct requires
/// its data: `render_mermaid` wants a source and the other 31 want `data`.
#[tokio::test]
async fn every_kind_reports_bad_arguments_against_render_figure() {
    let router = AutoVisualiserRouter::new();
    for kind in FigureKind::ALL {
        let err = router
            .call_figure_tool(kind.tool_name(), json!({}), FigureVocabulary::RenderFigure)
            .await
            .err()
            .unwrap_or_else(|| {
                panic!(
                    "`{}` accepted an empty payload; this guard needs one it refuses",
                    kind.slug()
                )
            });
        let message = err.message.to_string();

        assert!(
            !message.contains("Unknown visualization"),
            "kind `{}` names `{}`, which the dispatch table does not answer to: {message}",
            kind.slug(),
            kind.tool_name()
        );
        assert!(
            message.starts_with("`render_figure` arguments are invalid for kind"),
            "kind `{}` fell through to the tool-name fallback: {message}",
            kind.slug()
        );
        assert!(
            message.contains(&format!("\"{}\"", kind.slug())),
            "must name the kind the model passed: {message}"
        );
        assert!(
            message.contains("describe_figure"),
            "must point at the schema: {message}"
        );
        assert!(
            !message.contains(kind.tool_name()),
            "`{}` is not in a chat agent's roster: {message}",
            kind.tool_name()
        );
    }
}

/// The same walk, in the OTHER vocabulary. `ui_figure` hands its app agent the
/// per-kind tool names and has no `render_figure`/`describe_figure`, so this
/// door must keep naming the tool — the shared choke point cannot phrase both.
#[tokio::test]
async fn the_standalone_vocabulary_keeps_naming_the_tool() {
    let router = AutoVisualiserRouter::new();
    for kind in FigureKind::ALL {
        let message = router
            .call_figure_tool(kind.tool_name(), json!({}), FigureVocabulary::ToolName)
            .await
            .expect_err("an empty payload must be refused")
            .message
            .to_string();

        assert!(
            message.starts_with(&format!("`{}` arguments are invalid", kind.tool_name())),
            "kind `{}` lost the tool-name phrasing: {message}",
            kind.slug()
        );
        assert!(
            !message.contains("describe_figure"),
            "an app agent has no describe_figure: {message}"
        );
    }
}

/// The no-regression proof, aimed at the only place behaviour could differ.
///
/// For 31 of the 32 kinds `render_figure` builds `{"data": …}` and calls
/// `call_figure_tool` — byte for byte the call the dashboard already makes, so
/// equivalence there is structural, and `every_kind_resolves_to_a_real_figure_tool`
/// covers the part that could actually be wrong (the kind → tool join). What
/// needs real evidence is `mermaid`, the one kind whose payload is reshaped,
/// and a spread of kinds across the families to catch a dispatch mix-up.
#[tokio::test]
async fn the_entry_point_renders_what_the_legacy_call_rendered() {
    let router = AutoVisualiserRouter::new();

    let cases: Vec<(FigureKind, serde_json::Value)> = vec![
        (
            FigureKind::Chart,
            serde_json::json!({"type":"bar","title":"T","labels":["a","b"],
                "datasets":[{"label":"s","data":[1.0,2.0]}]}),
        ),
        (
            FigureKind::Donut,
            serde_json::json!({"title":"T","data":[{"label":"a","value":10.0}]}),
        ),
        (
            FigureKind::Sankey,
            serde_json::json!({"nodes":[{"name":"a"},{"name":"b"}],
                "links":[{"source":"a","target":"b","value":10.0}]}),
        ),
        (
            FigureKind::Chord,
            serde_json::json!({"labels":["a","b"],"matrix":[[0.0,10.0],[5.0,0.0]]}),
        ),
        (
            FigureKind::Treemap,
            serde_json::json!({"name":"root","children":[{"name":"a","value":10.0}]}),
        ),
        (
            FigureKind::Map,
            serde_json::json!({"title":"T","markers":[
                {"lat":37.7,"lng":-122.4,"name":"a","color":"#556677","value":0.0}]}),
        ),
    ];

    for (kind, payload) in cases {
        let legacy = router
            .call_figure_tool(
                kind.tool_name(),
                serde_json::json!({ "data": payload }),
                FigureVocabulary::RenderFigure,
            )
            .await
            .unwrap_or_else(|e| panic!("legacy call for `{}` failed: {e:?}", kind.slug()));
        let via_entry = router
            .render_figure(Parameters(RenderFigureParams {
                kind,
                data: payload,
            }))
            .await
            .unwrap_or_else(|e| panic!("render_figure for `{}` failed: {e:?}", kind.slug()));
        assert_eq!(
            decode_html(&legacy),
            decode_html(&via_entry),
            "`{}` renders differently through render_figure",
            kind.slug()
        );
    }
}

/// `mermaid` is the one kind whose wire shape is genuinely different — it takes
/// `mermaid_code`, not `data`. The normalization has to accept what a model told
/// "pass it as `data`" will send.
///
/// ⚠ That normalization is NOT the entry point's any more; it belongs to
/// `RenderMermaidParams`'s own `Deserialize`, so every door inherits it. This
/// test still enters through `render_figure` because that is the door whose
/// behaviour must not regress, but it is no longer the one doing the work — see
/// `test_a_kind_only_mermaid_panel_renders_instead_of_being_refused` for the
/// door that had none.
#[tokio::test]
async fn mermaid_accepts_the_shapes_a_model_will_send_and_refuses_the_rest() {
    let router = AutoVisualiserRouter::new();
    let source = "graph TD; A-->B;";

    let legacy = router
        .call_figure_tool(
            "render_mermaid",
            serde_json::json!({ "mermaid_code": source }),
            FigureVocabulary::RenderFigure,
        )
        .await
        .expect("legacy mermaid");

    for shape in [
        serde_json::json!(source),
        serde_json::json!({ "mermaid_code": source }),
        serde_json::json!({ "code": source }),
        serde_json::json!({ "source": source }),
        serde_json::json!({ "data": source }),
        serde_json::json!({ "diagram": source }),
        // The shape a model that stringifies nested arguments sends.
        serde_json::json!(format!("{{\"mermaid_code\": \"{source}\"}}")),
    ] {
        let rendered = router
            .render_figure(Parameters(RenderFigureParams {
                kind: FigureKind::Mermaid,
                data: shape.clone(),
            }))
            .await
            .unwrap_or_else(|e| panic!("mermaid rejected {shape}: {e:?}"));
        assert_eq!(decode_html(&legacy), decode_html(&rendered), "{shape}");
    }

    // A payload with no source anywhere must say what to do, not render an
    // empty diagram.
    let err = router
        .render_figure(Parameters(RenderFigureParams {
            kind: FigureKind::Mermaid,
            data: serde_json::json!({ "nodes": [] }),
        }))
        .await
        .unwrap_err();
    assert!(err.message.contains("describe_figure"), "{}", err.message);
}

/// A describe tool that advertised shapes the renderer rejects would be worse
/// than none: it is the model's only route to a kind's arguments now.
#[tokio::test]
async fn describe_figure_answers_for_every_kind_and_lists_them_all() {
    let router = AutoVisualiserRouter::new();

    let catalog = router
        .describe_figure(Parameters(DescribeFigureParams { kind: None }))
        .await
        .expect("catalog");
    let catalog = result_text(&catalog);
    for kind in FigureKind::ALL {
        assert!(
            catalog.contains(kind.slug()),
            "the catalog omits `{}`",
            kind.slug()
        );
    }

    for kind in FigureKind::ALL {
        let described = router
            .describe_figure(Parameters(DescribeFigureParams { kind: Some(*kind) }))
            .await
            .unwrap_or_else(|e| panic!("describe_figure({}) failed: {e:?}", kind.slug()));
        let body: serde_json::Value =
            serde_json::from_str(&result_text(&described)).expect("describe returns JSON");
        assert_eq!(body["kind"], kind.slug());
        assert!(
            body["schema"].is_object() && !body["schema"].as_object().unwrap().is_empty(),
            "`{}` has no schema",
            kind.slug()
        );
        assert!(
            body["guidance"].as_str().is_some_and(|g| !g.trim().is_empty()),
            "`{}` lost its worked example — that text is the model's only \
             per-kind documentation now",
            kind.slug()
        );
    }
}

/// The 17 KB of per-kind prose must stay in `describe_figure`'s RESULT, not
/// creep back into the declarations.
///
/// ⚠ **This is NOT a provider limit, and an earlier draft of this test wrongly
/// said it was.** A design review claimed Azure/OpenAI cap
/// `tools[n].function.description` at 1024 characters and reject the request.
/// Measured against the live Versa Azure gpt-5.5 endpoint on 2026-08-31: a turn
/// carrying `render_dashboard`'s 2,626-character description succeeded. The
/// array-length cap of 128 is real and enforced; a description cap is not, at
/// least not at that size.
///
/// What this guards is the thing that made the surface expensive in the first
/// place: 33 declarations carrying 17,509 bytes of worked examples, sent on
/// every single turn. The budget is generous enough that `render_dashboard`'s
/// deliberately long call-once guidance fits, and tight enough that folding a
/// kind's examples back into a declared tool fails here.
#[test]
fn per_kind_documentation_stays_out_of_the_declarations() {
    let router = AutoVisualiserRouter::new();
    let total: usize = router
        .tool_router
        .list_all()
        .iter()
        .map(|tool| tool.description.as_deref().unwrap_or_default().len())
        .sum();
    assert!(
        total <= 4_096,
        "the advertised Auto Visualiser descriptions total {total} bytes; per-kind \
         schemas and examples belong in describe_figure's result, which is not sent \
         on every turn"
    );
}

/// The advertised surface must not hand a chat agent a name it cannot call.
///
/// #142 is about the error strings, but they are not the only place the retired
/// names leaked: a tool's INPUT SCHEMA is read on every turn, and
/// `DashboardFigure`'s doc comment shipped `{"tool": "render_volcano", …}` as a
/// worked example — twice — while the errors around it were being rewritten to
/// stop naming it. A model copying an example out of the schema it was just
/// handed is not making a mistake.
///
/// Those shapes are still ACCEPTED; they moved to `//` comments beside the
/// struct, where a contributor reads them and a model does not. `describe_figure`
/// is exempt because it answers with a per-kind schema as a RESULT, and the
/// schema it reads back is the real tool's.
#[test]
fn the_advertised_surface_names_no_retired_figure_tool() {
    let router = AutoVisualiserRouter::new();
    let retired: Vec<&str> = FigureKind::ALL.iter().map(|k| k.tool_name()).collect();

    // Every string the model is handed unprompted: the server instructions, and
    // each declared tool's description AND input schema.
    let mut surfaces = vec![(
        "server instructions".to_string(),
        router.get_info().instructions.unwrap_or_default(),
    )];
    for tool in router.tool_router.list_all() {
        surfaces.push((
            tool.name.to_string(),
            format!(
                "{}{}",
                tool.description.as_deref().unwrap_or_default(),
                serde_json::to_string(&*tool.input_schema).unwrap()
            ),
        ));
    }

    for (where_, text) in surfaces {
        for name in &retired {
            assert!(
                !text.contains(name),
                "{where_} advertises `{name}`, which is not in a chat agent's roster"
            );
        }
    }
}

/// The plain text of a tool result, for the tools that answer with text rather
/// than a `ui://` resource.
fn result_text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("")
}

// ---------------------------------------------------------------------------
// The first `render_figure` call must land (handoff 07; #150 one layer up)
// ---------------------------------------------------------------------------

/// The strict door — what `rmcp` itself does with a model's raw arguments.
///
/// ⚠ Every other `render_figure` test in this file builds
/// `RenderFigureParams { kind, data }` in Rust, which skips deserialization
/// altogether, so none of them can see a payload refused *before* the tool body
/// runs. `rmcp` calls
/// `serde_json::from_value::<RenderFigureParams>(Value::Object(arguments))`
/// (`handler/server/tool.rs`) and turns any failure into a bare
/// "failed to deserialize parameters: …". This mirrors that call exactly as
/// `params_from` mirrors it for `render_dashboard`.
fn figure_params_from(value: Value) -> Result<Parameters<RenderFigureParams>, String> {
    serde_json::from_value::<RenderFigureParams>(value)
        .map(Parameters)
        .map_err(|e| e.to_string())
}

/// The canonical bar-chart payload, as `describe_figure` reports it.
fn first_call_chart_data() -> Value {
    json!({
        "type": "bar",
        "labels": ["a", "b", "c"],
        "datasets": [{"label": "Value", "data": [1.0, 2.0, 3.0]}],
    })
}

/// A model's FIRST `render_figure` call must render, not cost a round trip.
///
/// Measured on Versa GPT-5.5 in the round-2 and round-3 walkthroughs: "render a
/// bar chart of three values a=1, b=2, c=3" cost four or five tool-call cards,
/// two or three of them rejections, before the model called `describe_figure`,
/// read the schema and succeeded — once even *after* `describe_figure` had
/// already answered.
///
/// The rejections did not come from this crate. The advertised `render_figure`
/// had a DERIVED `Deserialize`, so `rmcp` refused the arguments with
/// "missing field `data`" — no kind, no `describe_figure` pointer — before a
/// line of Auto Visualiser ran, while the lenient `render_figure_call` sat one
/// door over, wired only to dashboard panels and Agent Drafter's `ui_figure`.
/// Through the `code_execution` sandbox (default-enabled) the model sees
/// `data: any` and one line of description, so it cannot read the shape off the
/// schema either.
///
/// Fails the shipped implementation on the flattened, envelope-wrapped and
/// chart-type-as-kind shapes.
#[tokio::test]
async fn render_figure_accepts_the_first_call_shapes_a_model_sends() {
    let router = AutoVisualiserRouter::new();

    let canonical = router
        .render_figure(
            figure_params_from(json!({"kind": "chart", "data": first_call_chart_data()}))
                .expect("the documented shape must deserialize"),
        )
        .await
        .expect("the documented shape must render");
    let canonical = decode_html(&canonical);

    for payload in [
        // (a) Flattened: the payload written straight onto the arguments. The
        //     shape `rmcp` answered with "missing field `data`".
        json!({
            "kind": "chart",
            "type": "bar",
            "labels": ["a", "b", "c"],
            "datasets": [{"label": "Value", "data": [1.0, 2.0, 3.0]}],
        }),
        // The envelope keys a model reaches for because that is what a
        // dashboard panel calls the same object.
        json!({"kind": "chart", "params": first_call_chart_data()}),
        json!({"kind": "chart", "arguments": first_call_chart_data()}),
        json!({"kind": "chart", "args": first_call_chart_data()}),
        // The `{"tool": …, "params": {…}}` envelope the panel vocabulary uses —
        // and, before this fix, the shape the missing-`kind` error itself told a
        // direct caller to send.
        json!({
            "tool": "render_figure",
            "params": {"kind": "chart", "data": first_call_chart_data()},
        }),
        // (e) Stringified `data`, and the whole object stringified.
        json!({"kind": "chart", "data": first_call_chart_data().to_string()}),
        json!(json!({"kind": "chart", "data": first_call_chart_data()}).to_string()),
        // (d) A chart TYPE used as the kind. `FigureKind` had no lenient
        //     `Deserialize` (unlike `ChartType`), so this died at
        //     "unknown variant `bar`".
        json!({"kind": "bar", "data": first_call_chart_data()}),
        json!({"kind": "Bar Chart", "data": first_call_chart_data()}),
        // …and the same, with the `type` the model already stated as the kind
        // left out of the payload.
        json!({
            "kind": "bar",
            "data": {
                "labels": ["a", "b", "c"],
                "datasets": [{"label": "Value", "data": [1.0, 2.0, 3.0]}],
            },
        }),
        // The kind named by the tool it dispatches to — the spelling
        // `describe_figure`'s own guidance is written in.
        json!({"kind": "show_chart", "data": first_call_chart_data()}),
    ] {
        let params = figure_params_from(payload.clone())
            .unwrap_or_else(|e| panic!("{payload} was refused before dispatch: {e}"));
        let rendered = router
            .render_figure(params)
            .await
            .unwrap_or_else(|e| panic!("{payload} was refused: {}", e.message));
        assert_eq!(
            canonical,
            decode_html(&rendered),
            "{payload} must draw the SAME figure, not merely draw one"
        );
    }
}

/// A kind a model can plausibly write must resolve; one it cannot must fail with
/// a message it can act on.
///
/// Goes through `serde_json::from_value` rather than a helper so it exercises
/// the same door `rmcp` uses. Fails the shipped implementation, whose derived
/// enum `Deserialize` accepted the 32 slugs and nothing else.
#[test]
fn figure_kind_resolves_the_names_a_model_writes() {
    for (written, expected) in [
        ("chart", FigureKind::Chart),
        ("Chart", FigureKind::Chart),
        ("show_chart", FigureKind::Chart),
        ("bar", FigureKind::Chart),
        ("bar chart", FigureKind::Chart),
        ("scatter-plot", FigureKind::Chart),
        ("pie", FigureKind::Donut),
        ("render_volcano", FigureKind::Volcano),
        ("Volcano", FigureKind::Volcano),
        ("kaplan-meier", FigureKind::KaplanMeier),
        ("kaplanmeier", FigureKind::KaplanMeier),
        ("word cloud", FigureKind::Wordcloud),
        ("ER Diagram", FigureKind::ErDiagram),
        ("calendar heatmap", FigureKind::CalendarHeatmap),
    ] {
        let parsed: FigureKind = serde_json::from_value(json!(written))
            .unwrap_or_else(|e| panic!("`{written}` must resolve to a kind: {e}"));
        assert_eq!(parsed, expected, "`{written}`");
    }

    // A word that names no figure must not be guessed at, and must say where the
    // list of kinds is.
    let err = serde_json::from_value::<FigureKind>(json!("wombat")).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("wombat"), "{message}");
    assert!(message.contains("describe_figure"), "{message}");
}

/// A payload that is genuinely wrong must still be reported against a call the
/// model can make, whichever door refused it.
///
/// The kind-and-`describe_figure` phrasing already held for a payload that
/// reached dispatch (`test_bad_arguments_name_render_figure_and_the_kind`).
/// What it did NOT hold for was a payload refused during deserialization, which
/// is where the walkthrough's rejections actually came from.
#[tokio::test]
async fn render_figure_refusals_name_render_figure_the_kind_and_describe_figure() {
    let router = AutoVisualiserRouter::new();
    let retired: Vec<&str> = FigureKind::ALL.iter().map(|k| k.tool_name()).collect();

    // (c) A chart payload with no `type`, and nothing in the kind to imply one.
    let message = match figure_params_from(json!({"kind": "chart", "data": {"labels": ["a"]}})) {
        Err(message) => message,
        Ok(params) => router
            .render_figure(params)
            .await
            .expect_err("a chart with no `type` must be refused")
            .message
            .to_string(),
    };
    assert!(message.contains("type"), "must say what is missing: {message}");
    assert!(message.contains("render_figure"), "{message}");
    assert!(message.contains("\"chart\""), "must name the kind: {message}");
    assert!(message.contains("describe_figure"), "{message}");
    for name in &retired {
        assert!(
            !message.contains(name),
            "the model cannot call `{name}`: {message}"
        );
    }

    // No `kind` at all: the example the message offers must be one this door
    // accepts. It used to hand a direct caller the PANEL spelling
    // (`{"tool": …, "params": {…}}`), teaching the shape that had just failed.
    let message =
        figure_params_from(json!({"data": first_call_chart_data()})).expect_err("no kind");
    assert!(message.contains("kind"), "{message}");
    assert!(message.contains("describe_figure"), "{message}");
    // Brace-matched from the first `{`, char by char: `clippy::string_slice` is
    // denied repo-wide, and a byte index into a message that carries `…` would
    // be the exact panic that lint exists for.
    let example: Value = {
        let mut depth = 0usize;
        let mut json = String::new();
        for ch in message.chars() {
            if depth == 0 && ch != '{' {
                continue;
            }
            json.push(ch);
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        assert!(
            json.contains("\"kind\""),
            "the missing-`kind` message must show a shape this door accepts: {message}"
        );
        serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("the example must be valid JSON ({e}): {json}"))
    };
    let params = figure_params_from(example.clone())
        .unwrap_or_else(|e| panic!("the example the error offers is itself refused: {e}"));
    router
        .render_figure(params)
        .await
        .unwrap_or_else(|e| panic!("the example the error offers does not render: {}", e.message));
}

/// Azure and OpenAI reject a request whose `tools[n].function.description`
/// exceeds 1024 characters with a non-retryable 400 — the same class of failure
/// as the 128-tool ceiling, and the reason the comment above `render_figure`
/// says to keep its description short. The limit is real and unhandled:
/// `providers::utils` classifies that exact 400 (`http_context_classifier_keeps_
/// tool_description_limits_as_request_failures`) and nothing truncates a
/// description on the way out.
///
/// `per_kind_documentation_stays_out_of_the_declarations` caps the SUM at 4096,
/// which a single 1500-byte description passes. Nothing pinned the per-tool
/// limit until now.
///
/// ⚠ **`render_dashboard` is already over it**, at 2705 bytes, measured on
/// `main` at f350cdfc — so this is a RATCHET, not a clean gate. A gate that
/// fails on arrival gets disabled rather than obeyed. Every other advertised
/// tool must fit the real cap, and the one that does not may only shrink toward
/// it; when it fits, delete its entry. Fixing that description is a change to
/// `render_dashboard`'s call-once guidance and belongs to whoever owns it.
#[test]
fn every_advertised_description_fits_the_provider_cap() {
    /// Tools measured over the cap today, with the size they must not exceed.
    const KNOWN_OVER_CAP: &[(&str, usize)] = &[("render_dashboard", 2_705)];

    let router = AutoVisualiserRouter::new();
    for tool in router.tool_router.list_all() {
        let len = tool.description.as_deref().unwrap_or_default().len();
        let (cap, note) = match KNOWN_OVER_CAP
            .iter()
            .find(|(name, _)| *name == tool.name.as_ref())
        {
            Some((_, allowed)) => (
                *allowed,
                "it is a known offender and may only shrink; it must never grow",
            ),
            None => (
                1_024,
                "Azure and OpenAI reject a tool description over 1024 characters \
                 with a non-retryable 400",
            ),
        };
        assert!(
            len <= cap,
            "`{}`'s description is {len} bytes against a budget of {cap}: {note}",
            tool.name
        );
    }

    // An entry that has become unnecessary must be deleted, not left to rot into
    // a licence for a description that now fits to grow again.
    for (name, allowed) in KNOWN_OVER_CAP {
        let len = router
            .tool_router
            .list_all()
            .iter()
            .find(|tool| tool.name.as_ref() == *name)
            .and_then(|tool| tool.description.as_deref())
            .map(str::len)
            .unwrap_or_else(|| panic!("`{name}` is exempted here but is not advertised"));
        assert!(
            len > 1_024,
            "`{name}` now fits the 1024-byte cap at {len} bytes; remove its \
             KNOWN_OVER_CAP entry (budget was {allowed})"
        );
    }
}
