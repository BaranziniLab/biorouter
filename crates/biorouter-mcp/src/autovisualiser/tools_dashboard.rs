// Dashboard: compose several figures into a single, documented report artifact.
//
// Why this exists: every other tool returns one figure as one `ui://` resource.
// When an analysis produces six of them the user has to open six artifacts to
// see one story. `render_dashboard` renders the same figures — through the very
// same tools, unchanged — into one scrollable report with a title, contents,
// section prose, and a numbered caption under each figure.
//
// How the panels get in there: each figure is rendered in *fragment mode*
// (see `common::render_fragment`), which leaves a placeholder where the figure's
// libraries would be inlined and reports which libraries it wanted. The report
// stores each library's source exactly once and its own JS splices the sources
// back into each panel's `srcdoc`. Without this, a report containing a Mermaid
// diagram plus two D3 charts would carry ~4 MB of duplicated library code.

/// How wide a panel sits in the report grid.
const WIDTH_FULL: &str = "full";
const WIDTH_HALF: &str = "half";

/// Guard rails. Generous — a report beyond this is unreadable anyway.
const MAX_PANELS: usize = 24;
const MAX_RECEIPT_FAILURES: usize = 8;
const MAX_PROSE_LEN: usize = 8_000;

/// One `render_figure` call: which figure to draw in this panel, and with what.
///
/// Write it as `{"tool": "render_figure", "params": {"kind": …, "data": …}}`.
// ⚠ Everything below this line is a `//` comment on purpose: this doc comment
// ships to the model inside `render_dashboard`'s advertised schema, and the
// shapes recorded here are BACK-COMPAT, not recommendations. Naming a retired
// tool in the advertised schema is the very thing #142/#150 are about — a model
// that copies `render_volcano` out of a schema has been handed a tool it cannot
// call. The deserializer below still accepts all of them:
//
//   `{"tool": "render_figure", "params": {"kind": "volcano", "data": {…}}}`  ← canonical
//   `{"tool": "render_figure", "kind": "volcano", "data": {…}}`   ← flattened
//   `{"kind": "volcano", "data": {…}}`      ← `kind` alone; no `tool` key at all
//   `{"type": "volcano", "params": {…}}`    ← the kind under another name
//   `{"tool": "render_volcano", "params": {…}}`  ← the retired per-kind tool name
//   `{"tool": "render_volcano", "data": {…}}`    ← bare tool args, no `params`
//
// The last four resolve to the per-kind tool DIRECTLY (`normalize_tool_name`
// turns `volcano` into `render_volcano`), so they never reach the
// `render_figure` unwrap in `call_figure_tool` and their `params` are that
// tool's own arguments, not `{kind, data}`. That is why the two doc comments
// below describe only the canonical shape: a field doc that tried to describe
// both would contradict itself, which is exactly what it used to do.
#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct DashboardFigure {
    /// `render_figure` — the one tool that draws a figure.
    pub tool: String,
    /// Exactly the arguments `render_figure` takes: a `kind` plus that kind's
    /// payload as `data`, e.g. `{"kind": "volcano", "data": {...}}`. Call
    /// `describe_figure` with a kind for its exact `data` schema.
    pub params: Value,
}

impl<'de> Deserialize<'de> for DashboardFigure {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error as DeError;

        // Some models stringify nested tool-call arguments; accept that too.
        let value = match Value::deserialize(d)? {
            Value::String(s) => serde_json::from_str::<Value>(&s).map_err(|e| {
                D::Error::custom(format!("`figure` was a JSON string that did not parse: {e}"))
            })?,
            other => other,
        };

        let mut map = match value {
            Value::Object(map) => map,
            _ => return Err(D::Error::custom("`figure` must be a JSON object")),
        };

        let tool = ["tool", "type", "name", "kind"]
            .iter()
            .find_map(|key| map.remove(*key))
            .ok_or_else(|| {
                D::Error::custom("`figure` needs a `tool` naming the visualization to render")
            })?;
        let tool = match tool {
            Value::String(s) => s,
            other => return Err(D::Error::custom(format!("`figure.tool` must be a string, got {other}"))),
        };

        // An explicit `params`/`arguments` wins; otherwise whatever else is on
        // the object *is* the tool's arguments.
        let params = ["params", "arguments", "args", "input"]
            .iter()
            .find_map(|key| map.remove(*key))
            .unwrap_or(Value::Object(map));

        Ok(DashboardFigure { tool, params })
    }
}

/// One figure in the report, with the prose that explains it.
#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct DashboardPanel {
    /// Short heading for this figure, shown above it. Strongly recommended.
    #[serde(default)]
    pub title: Option<String>,
    /// One or two sentences saying what the figure shows and what to look at.
    /// Supports `**bold**`, `*italic*`, `` `code` `` and `[links](https://…)`.
    #[serde(default)]
    pub caption: Option<String>,
    /// Longer methods / interpretation text, shown in a collapsed "Notes &
    /// methods" disclosure under the figure.
    #[serde(default)]
    pub notes: Option<String>,
    /// `full` (default, one figure per row) or `half` (two side by side).
    #[serde(default)]
    pub width: Option<String>,
    /// Fixed panel height in CSS px. Omit to let the figure size itself.
    #[serde(default)]
    pub height: Option<u32>,
    /// The visualization to render here.
    pub figure: DashboardFigure,
}

/// A titled group of panels.
#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct DashboardSection {
    /// Section heading, e.g. "Quality control".
    #[serde(default)]
    pub title: Option<String>,
    /// Prose introducing the section. Blank lines separate paragraphs; lines
    /// starting with `- ` become bullets.
    #[serde(default)]
    pub description: Option<String>,
    /// The figures in this section.
    #[serde(default)]
    pub panels: Vec<DashboardPanel>,
}

/// Report color theme. `Auto` (default) follows the desktop app's light/dark
/// setting so the report matches the rest of the UI (and stays identical in the
/// side-panel preview and the expanded view); `Light`/`Dark` force a look
/// regardless of the host — set one when the user asks for a specific background.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DashboardTheme {
    #[default]
    Auto,
    Light,
    Dark,
}

impl DashboardTheme {
    /// The value to bake into `window.__BR_VIZ_THEME__`, or `None` to follow the host.
    fn forced(self) -> Option<&'static str> {
        match self {
            DashboardTheme::Auto => None,
            DashboardTheme::Light => Some("light"),
            DashboardTheme::Dark => Some("dark"),
        }
    }
}

/// Bake a locked theme into an assembled report: `window.__BR_VIZ_THEME__` runs
/// before the report's `{{COMMON}}` (right after `<head>`), so `resolveTheme`
/// honours it and the report propagates it down to every panel.
fn inject_forced_theme(html: String, theme: &str) -> String {
    let tag = format!("<script>window.__BR_VIZ_THEME__=\"{theme}\";</script>");
    match html.split_once("<head>") {
        Some((before, after)) => {
            let mut out = String::with_capacity(html.len() + tag.len());
            out.push_str(before);
            out.push_str("<head>");
            out.push_str(&tag);
            out.push_str(after);
            out
        }
        None => format!("{tag}{html}"),
    }
}

/// Parameters for `render_dashboard`.
#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct RenderDashboardParams {
    /// The report's title, e.g. "Differential expression: tumour vs normal".
    pub title: String,
    /// Optional one-line standfirst under the title.
    #[serde(default)]
    pub subtitle: Option<String>,
    /// Opening prose: what this report covers and the headline findings.
    /// Blank lines separate paragraphs; lines starting with `- ` become bullets.
    #[serde(default)]
    pub summary: Option<String>,
    /// Grouped figures. Use this when the report has distinct parts.
    #[serde(default)]
    pub sections: Option<Vec<DashboardSection>>,
    /// Shorthand for a single unnamed section. Use `sections` or `panels`, not both.
    #[serde(default)]
    pub panels: Option<Vec<DashboardPanel>>,
    /// Closing prose: caveats, data provenance, next steps.
    #[serde(default)]
    pub footer: Option<String>,
    /// Report color theme: `auto` (default, follows the app's light/dark setting),
    /// `light`, or `dark`. Set `light` or `dark` when the user asks for a specific
    /// background; leave it `auto` to match whatever theme the app is in.
    #[serde(default)]
    pub theme: DashboardTheme,
}

/// The exact shape, deserialized once [`normalize_dashboard_args`] has coaxed the
/// model's arguments into it.
#[derive(Deserialize)]
struct RenderDashboardParamsRaw {
    title: String,
    #[serde(default)]
    subtitle: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    sections: Option<Vec<DashboardSection>>,
    #[serde(default)]
    panels: Option<Vec<DashboardPanel>>,
    #[serde(default)]
    footer: Option<String>,
    #[serde(default)]
    theme: DashboardTheme,
}

/// Parse a value that may have arrived as a JSON string instead of JSON.
fn de_stringified(value: Value) -> Value {
    match value {
        Value::String(s) => serde_json::from_str(&s).unwrap_or(Value::String(s)),
        other => other,
    }
}

/// Reshape the arguments a model actually sends into the documented shape.
///
/// Every *other* Auto Visualiser tool takes a single `data` argument, so models
/// generalise and wrap the whole report in one — observed with GPT-5.5, which
/// sent `{"data": {"title": …, "sections": […]}}` and then retried identically
/// after the rejection. Some models also stringify nested arguments. Rejecting
/// either costs the user a wasted turn for no reason, so accept both.
fn normalize_dashboard_args(value: Value) -> Value {
    let mut value = de_stringified(value);

    // Unwrap a `data` (or `dashboard`/`report`) envelope that carries the report.
    if let Value::Object(map) = &value {
        if !map.contains_key("title") {
            if let Some(inner) = ["data", "dashboard", "report"]
                .iter()
                .find_map(|key| map.get(*key))
            {
                let unwrapped = de_stringified(inner.clone());
                if unwrapped.is_object() {
                    value = unwrapped;
                }
            }
        }
    }

    // `sections` / `panels` may themselves arrive stringified.
    if let Value::Object(map) = &mut value {
        for key in ["sections", "panels"] {
            if let Some(entry) = map.get_mut(key) {
                *entry = de_stringified(entry.clone());
            }
        }
    }
    value
}

impl<'de> Deserialize<'de> for RenderDashboardParams {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error as DeError;
        let value = normalize_dashboard_args(Value::deserialize(d)?);
        let raw: RenderDashboardParamsRaw = serde_json::from_value(value).map_err(|e| {
            DeError::custom(format!(
                "{e}. `render_dashboard` takes the report directly \
                 (title, summary, sections/panels), not wrapped in a `data` argument."
            ))
        })?;
        Ok(RenderDashboardParams {
            title: raw.title,
            subtitle: raw.subtitle,
            summary: raw.summary,
            sections: raw.sections,
            panels: raw.panels,
            footer: raw.footer,
            theme: raw.theme,
        })
    }
}

/// Canonicalise a model-supplied tool name: `Volcano`, `volcano`,
/// `render-volcano` and `render_volcano` all mean the same tool.
fn normalize_tool_name(raw: &str) -> String {
    let base = raw
        .trim()
        .to_lowercase()
        .replace([' ', '-', '.'], "_")
        .replace("__", "_");
    match base.as_str() {
        // The one tool that isn't `render_*`.
        "chart" | "show_chart" | "render_chart" => "show_chart".to_string(),
        other if other.starts_with("render_") => other.to_string(),
        other => format!("render_{other}"),
    }
}

/// Turn a title into a URI slug: "Tumour vs Normal" -> "tumour-vs-normal".
fn slugify(title: &str) -> String {
    let slug: String = title
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "report".to_string()
    } else {
        slug.chars().take(60).collect()
    }
}

fn check_prose(value: &Option<String>, what: &str) -> Result<(), ErrorData> {
    if let Some(text) = value {
        if text.len() > MAX_PROSE_LEN {
            return Err(invalid(format!(
                "{what} is too long ({} chars, max {MAX_PROSE_LEN}). Summarise it.",
                text.len()
            )));
        }
    }
    Ok(())
}

/// Embed a library source in the report as an inert, readable text block.
///
/// The HTML tokenizer ends a `<script>` at the first `</script`, even in a
/// `text/plain` block, so a source containing that sequence is base64-encoded
/// instead. Our vendored libraries don't, but a future asset bump might.
fn asset_store_entry(key: &str, kind: &str, src: &str) -> String {
    if src.to_lowercase().contains("</script") {
        format!(
            "<script type=\"text/plain\" data-autovis-asset=\"{key}\" data-kind=\"{kind}\" data-b64=\"1\">{}</script>\n",
            STANDARD.encode(src.as_bytes())
        )
    } else {
        format!(
            "<script type=\"text/plain\" data-autovis-asset=\"{key}\" data-kind=\"{kind}\">{src}</script>\n"
        )
    }
}

/// The outcome of rendering one panel's figure.
struct BuiltPanel {
    /// The figure's HTML, still carrying `ASSET_PLACEHOLDER`. `None` on failure.
    html: Option<String>,
    /// Library keys this figure needs, in load order.
    assets: Vec<String>,
    /// Why the figure could not be rendered.
    error: Option<String>,
}

#[tool_router(router = dashboard_router)]
impl AutoVisualiserRouter {
    /// Render every panel, keeping failures local to their own panel.
    async fn build_panels(&self, panels: &[DashboardPanel]) -> Vec<BuiltPanel> {
        let mut built = Vec::with_capacity(panels.len());
        for panel in panels {
            built.push(self.build_panel(&panel.figure).await);
        }
        built
    }

    async fn build_panel(&self, figure: &DashboardFigure) -> BuiltPanel {
        match self.render_figure_fragment(figure).await {
            Ok((html, assets)) => BuiltPanel {
                html: Some(html),
                assets: assets.iter().map(|a| a.key().to_string()).collect(),
                error: None,
            },
            Err(e) => BuiltPanel {
                html: None,
                assets: Vec::new(),
                error: Some(e.message.to_string()),
            },
        }
    }

    /// Call the real figure tool in fragment mode and recover its HTML.
    ///
    /// Every panel therefore inherits that tool's validation, limits and
    /// template verbatim — a dashboard volcano plot is byte-for-byte the same
    /// figure as a standalone one, minus the duplicated libraries.
    async fn render_figure_fragment(
        &self,
        figure: &DashboardFigure,
    ) -> Result<(String, Vec<Asset>), ErrorData> {
        let name = normalize_tool_name(&figure.tool);
        let params = figure.params.clone();
        // A panel is written by a chat agent, whose figure vocabulary is
        // `render_figure` + `kind` — the per-kind names it may also have used
        // are back-compat, not something it can look up.
        let call = self.call_figure_tool(&name, params, FigureVocabulary::RenderFigure);
        let (result, assets) = common::render_fragment(call).await;
        Ok((common::html_from_result(&result?)?, assets))
    }

    /// Dispatch a normalized single-figure tool name to its implementation.
    ///
    /// This is the one table mapping `render_*`/`show_chart` names onto the real
    /// tool methods and their parameter structs. Both the dashboard (which wraps
    /// this in [`common::render_fragment`]) and the standalone embedding API
    /// ([`render_standalone_figure`]) go through here, so a figure is
    /// byte-for-byte identical however it is reached. `render_dashboard` is not in
    /// this table — it composes these figures rather than being one.
    ///
    /// `vocab` says which figure names the *caller's* model can actually emit, and
    /// only the error paths read it. The two doors have different rosters, so a
    /// single phrasing is wrong for one of them however it is written — see
    /// [`FigureVocabulary`].
    async fn call_figure_tool(
        &self,
        name: &str,
        params: Value,
        vocab: FigureVocabulary,
    ) -> Result<CallToolResult, ErrorData> {
        // `render_figure` is the ONLY figure tool a chat model is offered, so it
        // is also the name it reaches for inside a dashboard panel — measured
        // live on Versa, which sent `{"tool": "render_figure", "params":
        // {"kind": …, "data": …}}` and was told the visualization did not exist
        // (#142).
        //
        // Two callers reach this branch: a dashboard panel, and
        // `render_standalone_figure` (Agent Drafter's `ui_figure`, whose `tool`
        // argument a model also fills in). The declared `render_figure` does
        // NOT — it resolves its own `kind` and calls in as `kind.tool_name()`,
        // which is why one unwrap is always enough: `tool_name()` never returns
        // `render_figure`, so this cannot recurse.
        let (name, params) = if name == RENDER_FIGURE {
            let (kind, data) = render_figure_call(params, vocab)?;
            (kind.tool_name(), figure_arguments(data))
        } else {
            (name, params)
        };

        // Deserialize into the tool's own parameter struct, then call it.
        macro_rules! dispatch {
            ($($tool:literal => ($method:ident, $params_ty:ty)),+ $(,)?) => {
                match name {
                    $(
                        $tool => {
                            let typed: $params_ty = serde_json::from_value(params)
                                .map_err(|e| invalid(figure_argument_error(vocab, $tool, &e.to_string())))?;
                            self.$method(Parameters(typed)).await
                        }
                    )+
                    other => Err(invalid(unknown_figure_error(vocab, other))),
                }
            };
        }

        dispatch! {
            "show_chart"             => (show_chart, ShowChartParams),
            "render_sankey"          => (render_sankey, RenderSankeyParams),
            "render_radar"           => (render_radar, RenderRadarParams),
            "render_donut"           => (render_donut, RenderDonutParams),
            "render_treemap"         => (render_treemap, RenderTreemapParams),
            "render_chord"           => (render_chord, RenderChordParams),
            "render_map"             => (render_map, RenderMapParams),
            "render_mermaid"         => (render_mermaid, RenderMermaidParams),
            "render_histogram"       => (render_histogram, RenderHistogramParams),
            "render_bubble"          => (render_bubble, RenderBubbleParams),
            "render_area"            => (render_area, RenderAreaParams),
            "render_gauge"           => (render_gauge, RenderGaugeParams),
            "render_volcano"         => (render_volcano, RenderVolcanoParams),
            "render_manhattan"       => (render_manhattan, RenderManhattanParams),
            "render_network"         => (render_network, RenderNetworkParams),
            "render_heatmap"         => (render_heatmap, RenderHeatmapParams),
            "render_sunburst"        => (render_sunburst, RenderSunburstParams),
            "render_dendrogram"      => (render_dendrogram, RenderDendrogramParams),
            "render_calendar_heatmap"=> (render_calendar_heatmap, RenderCalendarParams),
            "render_boxplot"         => (render_boxplot, RenderBoxplotParams),
            "render_wordcloud"       => (render_wordcloud, RenderWordcloudParams),
            "render_kaplan_meier"    => (render_kaplan_meier, RenderKaplanMeierParams),
            "render_forest"          => (render_forest, RenderForestParams),
            "render_flowchart"       => (render_flowchart, RenderFlowchartParams),
            "render_gantt"           => (render_gantt, RenderGanttParams),
            "render_sequence"        => (render_sequence, RenderSequenceParams),
            "render_mindmap"         => (render_mindmap, RenderMindmapParams),
            "render_timeline"        => (render_timeline, RenderTimelineParams),
            "render_er_diagram"      => (render_er_diagram, RenderErParams),
            "render_state_diagram"   => (render_state_diagram, RenderStateParams),
            "render_class_diagram"   => (render_class_diagram, RenderClassParams),
            "render_choropleth"      => (render_choropleth, RenderChoroplethParams),
        }
    }

    /// Combine several figures into one documented, self-contained report.
    #[tool(
        name = "render_dashboard",
        description = r#"Combine several figures into ONE scrollable report artifact, with a title, contents, section prose and a numbered caption under each figure.

Use this WHENEVER an answer needs more than one figure. Calling render_* several times leaves the user opening one artifact per figure; render_dashboard gives them a single page that tells the whole story.

Each panel's `figure` is a `render_figure` call: a `kind` and that kind's payload as `data`. Call describe_figure with a kind for its exact schema.

Example:
{
  "title": "Synthetic workflow comparison",
  "subtitle": "Illustrative observations, not research findings",
  "summary": "Workflow B costs 20 units more per month and saves 15 more minutes per run than workflow A in this synthetic example.",
  "sections": [
    {
      "title": "Cost and time, shown on separate scales",
      "description": "Two supplied observations per quantity; no statistical inference.",
      "panels": [
        {
          "title": "Monthly cost",
          "caption": "A costs 100 units per month; B costs 120.",
          "notes": "Synthetic inputs supplied only to demonstrate the report format.",
          "figure": {"tool": "render_figure", "params": {"kind": "chart", "data": {"type": "bar", "labels": ["A", "B"], "yAxisLabel": "Cost (units/month)", "datasets": [{"label": "Monthly cost", "data": [100, 120]}]}}}
        },
        {
          "title": "Time saved per run",
          "width": "half",
          "caption": "A saves 15 minutes per run; B saves 30.",
          "figure": {"tool": "render_figure", "params": {"kind": "chart", "data": {"type": "bar", "labels": ["A", "B"], "yAxisLabel": "Time saved (minutes/run)", "datasets": [{"label": "Time saved", "data": [15, 30]}]}}}
        }
      ]
    }
  ],
  "footer": "Replace these illustrative values with the user's actual observations."
}

Panel width is `full` (default) or `half` (two per row). Use `sections` for grouped reports, or the flat `panels` shorthand for a simple one.
Prefer full width for dense diagrams, long labels, or several series. Large diagrams retain readable text and can scroll. Captions and summaries must describe supplied values; do not invent sample sizes, provenance, statistical significance or findings.

Set `theme` to `light` or `dark` if the user asks for a specific background; the default `auto` follows the app's own light/dark setting.

Call this ONCE per report: the result contains the complete artifact for the side panel. Do not call it again merely to display, finalise or confirm it. Inspect the existing artifact to verify rendering; generation alone is not visual verification. Call it again only to change the report or correct figures that failed."#
    )]
    pub async fn render_dashboard(
        &self,
        params: Parameters<RenderDashboardParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = params.0;
        // Copy out the theme before `p`'s other fields are moved below.
        let forced_theme = p.theme.forced();

        if p.title.trim().is_empty() {
            return Err(invalid("Dashboard requires a non-empty `title`."));
        }
        check_prose(&p.summary, "`summary`")?;
        check_prose(&p.footer, "`footer`")?;

        // `sections` and `panels` are two spellings of the same thing.
        let mut sections = match (p.sections, p.panels) {
            (Some(sections), None) => sections,
            (None, Some(panels)) => vec![DashboardSection {
                title: None,
                description: None,
                panels,
            }],
            (Some(sections), Some(panels)) if panels.is_empty() => sections,
            (Some(_), Some(_)) => {
                return Err(invalid(
                    "Provide either `sections` or `panels`, not both.",
                ));
            }
            (None, None) => {
                return Err(invalid(
                    "Dashboard requires at least one figure: pass `panels` or `sections`.",
                ));
            }
        };
        sections.retain(|section| !section.panels.is_empty());

        let total_panels: usize = sections.iter().map(|s| s.panels.len()).sum();
        if total_panels == 0 {
            return Err(invalid(
                "Dashboard requires at least one figure: every section was empty.",
            ));
        }
        check_limit(total_panels, MAX_PANELS, "dashboard panels")?;

        for section in &sections {
            check_prose(&section.description, "a section `description`")?;
            for panel in &section.panels {
                check_prose(&panel.caption, "a panel `caption`")?;
                check_prose(&panel.notes, "a panel `notes`")?;
                if let Some(width) = &panel.width {
                    let w = width.trim().to_lowercase();
                    if w != WIDTH_FULL && w != WIDTH_HALF {
                        return Err(invalid(format!(
                            "Panel `width` must be '{WIDTH_FULL}' or '{WIDTH_HALF}', got '{width}'."
                        )));
                    }
                }
            }
        }

        // --- render every figure -------------------------------------------
        let mut panel_store = String::new();
        let mut asset_store = String::new();
        let mut stored_assets: Vec<Asset> = Vec::new();
        let mut failures: Vec<String> = Vec::new();
        let mut failure_receipts = Vec::new();
        let mut json_sections = Vec::with_capacity(sections.len());
        let mut panel_index = 0usize;
        let mut figure_number = 0usize;

        for section in &sections {
            let built = self.build_panels(&section.panels).await;
            let mut json_panels = Vec::with_capacity(section.panels.len());

            for (panel, built) in section.panels.iter().zip(built) {
                figure_number += 1;

                let mut entry = serde_json::Map::new();
                entry.insert("title".into(), json!(panel.title));
                entry.insert("caption".into(), json!(panel.caption));
                entry.insert("notes".into(), json!(panel.notes));
                entry.insert("height".into(), json!(panel.height));
                entry.insert(
                    "width".into(),
                    json!(panel
                        .width
                        .as_deref()
                        .map(|w| w.trim().to_lowercase())
                        .unwrap_or_else(|| WIDTH_FULL.to_string())),
                );

                match built.html {
                    Some(html) => {
                        // A report always inlines its libraries, even when
                        // `BIOROUTER_AUTOVIS_CDN` is on — which the desktop app sets by
                        // default. CDN mode only ever worked for a *standalone* figure,
                        // because the Electron main process rewrites that figure's
                        // `<script src=…>` back into an inline script before display: the
                        // renderer's CSP is `script-src 'self' 'unsafe-inline'`, so a
                        // remote script never loads. A report keeps its library tags
                        // inside base64 asset/panel blobs, where that rewriter cannot
                        // reach them, so a CDN report rendered blank figures with
                        // "Chart is not defined". Inlining costs little: the shared store
                        // holds each library exactly once, however many panels use it.
                        for key in &built.assets {
                            if let Some(asset) = ASSET_ORDER.iter().find(|a| a.key() == key.as_str())
                            {
                                if !stored_assets.contains(asset) {
                                    stored_assets.push(*asset);
                                    for (kind, src) in asset.sources() {
                                        asset_store
                                            .push_str(&asset_store_entry(asset.key(), kind, src));
                                    }
                                }
                            }
                        }

                        panel_store.push_str(&format!(
                            "<script type=\"text/plain\" id=\"autovis-panel-{panel_index}\">{}</script>\n",
                            STANDARD.encode(html.as_bytes())
                        ));
                        entry.insert("index".into(), json!(panel_index));
                        entry.insert("assets".into(), json!(built.assets.clone()));
                        entry.insert("error".into(), Value::Null);
                        panel_index += 1;
                    }
                    None => {
                        let message = built.error.unwrap_or_else(|| "unknown error".to_string());
                        if failure_receipts.len() < MAX_RECEIPT_FAILURES {
                            failure_receipts.push(json!({
                                "figure": figure_number,
                                "tool": panel.figure.tool.chars().take(64).collect::<String>(),
                                "error": message.chars().take(128).collect::<String>(),
                                "detailsTruncated": panel.figure.tool.chars().nth(64).is_some()
                                    || message.chars().nth(128).is_some(),
                            }));
                        }
                        failures.push(format!(
                            "Figure {figure_number} ({}): {message}",
                            panel.title.as_deref().unwrap_or(&panel.figure.tool)
                        ));
                        entry.insert("index".into(), Value::Null);
                        entry.insert("assets".into(), json!([] as [&str; 0]));
                        entry.insert("error".into(), json!(message));
                    }
                }
                json_panels.push(Value::Object(entry));
            }

            json_sections.push(json!({
                "title": section.title,
                "description": section.description,
                "panels": json_panels,
            }));
        }

        if failures.len() == total_panels {
            return Err(invalid(format!(
                "Every figure in the dashboard failed to render:\n{}",
                failures.join("\n")
            )));
        }

        // --- assemble the report --------------------------------------------
        let data = json!({
            "title": p.title,
            "subtitle": p.subtitle,
            "summary": p.summary,
            "footer": p.footer,
            "sections": json_sections,
        });
        let data_json = js_data(&data)?;
        let placeholder_literal = js_data(&Value::String(common::ASSET_PLACEHOLDER.to_string()))?;
        // `assemble` substitutes in order, so any `{{…}}` surviving inside a
        // user-supplied value would be treated as a later placeholder. Titles are
        // the only user text substituted before the end, so neutralise braces.
        let title_html = html_escape(&p.title).replace('{', "&#123;");

        let html = common::assemble(
            include_str!("templates/dashboard_template.html"),
            &[],
            &[
                // The template does `html.replace('{{ASSET_PLACEHOLDER}}', …)`;
                // this substitutes the quoted JS string it matches against.
                ("'{{ASSET_PLACEHOLDER}}'", &placeholder_literal),
                ("{{ASSET_STORE}}", &asset_store),
                ("{{PANEL_STORE}}", &panel_store),
                ("{{TITLE}}", &title_html),
                // Last: user prose lands here, so nothing can rewrite it.
                ("{{DASHBOARD_DATA}}", &data_json),
            ],
        );

        // When the user asked for a specific look, lock it in so both the preview
        // and the expanded view honour it; otherwise the report follows the host.
        let html = match forced_theme {
            Some(theme) => inject_forced_theme(html, theme),
            None => html,
        };

        let rendered = total_panels - failures.len();
        let mut label = format!(
            "Combined report '{}' created for the artifact panel with {rendered} figure{}.",
            p.title,
            if rendered == 1 { "" } else { "s" }
        );
        if failures.is_empty() {
            // Discourage duplicate generation without claiming that the client
            // has rendered or visually verified the HTML this function returns.
            label.push_str(
                " The report is complete and ready for the artifact panel, so you do \
                 not need to call render_dashboard again to display, finalise or confirm it. \
                 Inspect the existing artifact to verify rendering. Call this tool again \
                 only to change the report or correct a rendering failure.",
            );
        } else {
            // render_dashboard is stateless and re-renders the WHOLE report, so tell
            // the model to re-send every panel (not just the failed ones) — otherwise
            // it drops the panels that rendered fine on the retry.
            label.push_str(&format!(
                "\n\n{} figure(s) could not be rendered and show an error card in the report. \
                 Re-send the whole report with these figures' arguments fixed (keep the panels \
                 that rendered):\n{}",
                failures.len(),
                failures.join("\n")
            ));
        }

        let uri = format!("ui://dashboard/{}", slugify(&p.title));
        let mut result = common::finish(&uri, "dashboard", &label, html);
        // Keep recovery outside user-controlled titles and bounded error text.
        // Clients preferring structured content must still see partial failures.
        result.structured_content = Some(json!({
            "status": if failures.is_empty() { "created" } else { "created_with_errors" },
            "uri": uri,
            "mimeType": "text/html",
            "summary": format!("Report artifact created with {rendered} figures; {} failed.", failures.len()),
            "figuresCreated": rendered,
            "figuresFailed": failures.len(),
            "failuresOmitted": failures.len().saturating_sub(failure_receipts.len()),
            "failures": failure_receipts,
            "recovery": if failures.is_empty() {
                "Inspect the existing artifact to verify rendering; do not regenerate it merely to display or confirm it."
            } else {
                "Inspect all error cards in the existing artifact. Re-send the whole report with failed panels corrected, retaining successful panels."
            },
        }));

        // Reports are pages, not figures: ask for a reading-pane frame.
        if let Some(content) = result.content.first_mut() {
            if let rmcp::model::RawContent::Resource(embedded) = &mut content.raw {
                if let ResourceContents::BlobResourceContents { meta, .. } = &mut embedded.resource {
                    let mut meta_obj = serde_json::Map::new();
                    meta_obj.insert(
                        "mcpui.dev/ui-preferred-frame-size".to_string(),
                        json!(["1200px", "860px"]),
                    );
                    *meta = Some(rmcp::model::Meta(meta_obj));
                }
            }
        }
        Ok(result)
    }
}

/// Every asset, in the order libraries must load. Used to map the string keys a
/// panel reports back onto the `Asset` values that own the sources.
const ASSET_ORDER: [Asset; 5] = [
    Asset::D3,
    Asset::D3Sankey,
    Asset::ChartJs,
    Asset::Leaflet,
    Asset::Mermaid,
];

/// Render one named Auto Visualiser figure as a complete, self-contained
/// HTML document (inlined assets), for embedding in a sandboxed iframe.
/// `tool` is the tool name with or without the `render_`/`show_` prefix
/// (e.g. "kaplan_meier", "render_kaplan_meier", "show_chart").
/// Returns Err with a human-fixable message for unknown tools or invalid args.
///
/// `render_dashboard` is accepted too — a report embedded in an app is
/// legitimate. `args` are exactly the arguments the tool takes on its own
/// (e.g. `{"data": …}`).
///
/// ⚠ The caller here is Agent Drafter's `ui_figure`, and an app agent's
/// vocabulary is the per-kind TOOL NAMES its description lists
/// (`render_volcano`, `render_manhattan`, …). It has neither `render_figure` nor
/// `describe_figure`: `configure_agent` never injects autovisualiser into an app
/// agent. So this door keeps [`FigureVocabulary::ToolName`] — an error phrased
/// in the chat agent's vocabulary would tell it to fix a call it never made,
/// using two tools it does not have.
///
/// Assets are always inlined, ignoring `BIOROUTER_AUTOVIS_CDN`: the document
/// lands in a `srcdoc` iframe the Electron CDN→inline rewriter cannot reach, so a
/// remote `<script src=…>` would be blocked by the renderer CSP and render blank
/// — the same reasoning that makes a dashboard inline its libraries.
pub async fn render_standalone_figure(tool: &str, args: Value) -> Result<String, String> {
    let router = AutoVisualiserRouter::new();
    let name = normalize_tool_name(tool);
    let result = common::with_inline_assets(async move {
        if name == "render_dashboard" {
            let params: RenderDashboardParams = serde_json::from_value(args)
                .map_err(|e| invalid(format!("`render_dashboard` arguments are invalid: {e}")))?;
            router.render_dashboard(Parameters(params)).await
        } else {
            router
                .call_figure_tool(&name, args, FigureVocabulary::ToolName)
                .await
        }
    })
    .await
    .map_err(|e| e.message.to_string())?;
    common::html_from_result(&result).map_err(|e| e.message.to_string())
}

// ===========================================================================
// The declared figure surface: one entry point, one describe tool
// ===========================================================================
//
// ⚠ **Why the 32 figure tools are no longer DECLARED, and why their Rust
// methods are untouched.**
//
// Azure and OpenAI reject any request whose `tools` array exceeds 128 entries,
// with a non-retryable 400 before the model sees anything. Measured on this
// tree: all built-in capabilities on and Code Execution off declared 130 tools,
// 33 of them Auto Visualiser's — so a chat could not take a single turn. This
// capability is where the headroom is, and it is also where consolidating is
// nearly free, because the 32 single-figure tools are one tool with a
// discriminator and the codebase already said so twice:
//
//   1. All 32 run the same pipeline in `common.rs` — validate, `js_data`,
//      `assemble`, `finish`.
//   2. `call_figure_tool` below is ALREADY a 32-way dispatcher keyed on exactly
//      these names, and `render_dashboard` already ships a production feature
//      where the model emits `{"tool": "render_volcano", "params": {…}}` per
//      panel. Kind-dispatched figure calls are therefore measured to work with
//      real models here, not a hypothesis.
//
// So this promotes the existing dispatcher to a declared tool. It moves no
// rendering code, and the 32 `#[tool]` methods keep their names, schemas and
// worked-example descriptions — they are simply routed into `figure_router`,
// which is held for metadata rather than advertised. That is what lets
// `describe_figure` hand back each kind's REAL schema and REAL example instead
// of a second copy that could drift.
//
// Three other callers reach the figures without going through the declaration,
// and none of them changes: `call_figure_tool` (dashboard panels),
// `render_standalone_figure` (Agent Drafter's `ui_figure`), and every unit test
// via the `ok_render!`/`err_render!` macros, which call the inherent methods.

/// The one table. The `kind` enum, its slug, and the tool it dispatches to are
/// generated together, so the schema the model sees and the dispatcher it
/// reaches can never disagree — an invariant a test would otherwise have to
/// check, and would only check for the pairs someone remembered to list.
macro_rules! figure_kinds {
    ($($variant:ident => ($slug:literal, $tool:literal)),+ $(,)?) => {
        /// Which figure to draw. Enumerated in the schema, so a model cannot
        /// invent a kind and a wrong guess is refused before any work happens.
        //
        // ⚠ `Deserialize` is HAND-WRITTEN, below this macro. The derive accepted
        // the 32 slugs and nothing else, so a model that answered with a chart
        // type (`"bar"`), the tool name `describe_figure`'s own guidance is
        // written in (`"show_chart"`) or a capitalised slug (`"Chart"`) was
        // refused by serde before a line of Auto Visualiser ran — with serde's
        // "unknown variant" text, which names neither `describe_figure` nor a
        // kind it could have used instead.
        //
        // `Serialize` and `JsonSchema` keep the derive, so the schema the model
        // is SHOWN is still the strict enum of canonical slugs. The leniency is
        // what this accepts, never what it advertises.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
        #[derive(rmcp::schemars::JsonSchema)]
        pub enum FigureKind {
            $(
                #[serde(rename = $slug)]
                $variant,
            )+
        }

        impl FigureKind {
            /// Every kind, for the guards and for `describe_figure`'s catalog.
            pub const ALL: &'static [FigureKind] = &[$(FigureKind::$variant),+];

            /// The value the model passes as `kind`.
            pub fn slug(self) -> &'static str {
                match self { $(FigureKind::$variant => $slug),+ }
            }

            /// The underlying tool whose schema, description and implementation
            /// this kind is.
            pub fn tool_name(self) -> &'static str {
                match self { $(FigureKind::$variant => $tool),+ }
            }
        }
    };
}

figure_kinds! {
    Chart           => ("chart",            "show_chart"),
    Histogram       => ("histogram",        "render_histogram"),
    Boxplot         => ("boxplot",          "render_boxplot"),
    Bubble          => ("bubble",           "render_bubble"),
    Area            => ("area",             "render_area"),
    Radar           => ("radar",            "render_radar"),
    Donut           => ("donut",            "render_donut"),
    Gauge           => ("gauge",            "render_gauge"),
    Volcano         => ("volcano",          "render_volcano"),
    Manhattan       => ("manhattan",        "render_manhattan"),
    KaplanMeier     => ("kaplan_meier",     "render_kaplan_meier"),
    Forest          => ("forest",           "render_forest"),
    Network         => ("network",          "render_network"),
    Sankey          => ("sankey",           "render_sankey"),
    Chord           => ("chord",            "render_chord"),
    Heatmap         => ("heatmap",          "render_heatmap"),
    Treemap         => ("treemap",          "render_treemap"),
    Sunburst        => ("sunburst",         "render_sunburst"),
    Dendrogram      => ("dendrogram",       "render_dendrogram"),
    Wordcloud       => ("wordcloud",        "render_wordcloud"),
    CalendarHeatmap => ("calendar_heatmap", "render_calendar_heatmap"),
    Mermaid         => ("mermaid",          "render_mermaid"),
    Flowchart       => ("flowchart",        "render_flowchart"),
    Gantt           => ("gantt",            "render_gantt"),
    Sequence        => ("sequence",         "render_sequence"),
    Mindmap         => ("mindmap",          "render_mindmap"),
    Timeline        => ("timeline",         "render_timeline"),
    ErDiagram       => ("er_diagram",       "render_er_diagram"),
    StateDiagram    => ("state_diagram",    "render_state_diagram"),
    ClassDiagram    => ("class_diagram",    "render_class_diagram"),
    Map             => ("map",              "render_map"),
    Choropleth      => ("choropleth",       "render_choropleth"),
}

impl FigureKind {
    /// Resolve a figure name however a model spelled it.
    ///
    /// The advertised schema names one spelling per kind, but a model does not
    /// always answer with it — and through the `code_execution` sandbox it never
    /// sees the enum at all, only `kind: FigureKind` in a one-line signature.
    /// Every rung below is a spelling the model was handed somewhere: the slug
    /// is in the schema, the tool name is in `describe_figure`'s `guidance`
    /// (it is the underlying tool's own description), and `bar`/`line`/
    /// `scatter`/`pie` are the words the server instructions use to say what
    /// `chart` and `donut` are FOR.
    ///
    /// ⚠ This resolves the figure's NAME. It is not a licence to guess: a word
    /// that names no figure still fails, because rendering the wrong figure is
    /// worse than one more round trip.
    pub fn from_alias(raw: &str) -> Option<FigureKind> {
        // Folds case, spaces, dashes and a missing `render_` prefix into the
        // dispatch table's own spelling — including `chart` → `show_chart`.
        let tool = normalize_tool_name(raw);
        if let Some(kind) = Self::ALL.iter().copied().find(|k| k.tool_name() == tool) {
            return Some(kind);
        }
        // Word breaks a model put somewhere else, or nowhere: `kaplanmeier`,
        // `word cloud`, `ERdiagram`.
        let squashed = tool.replace('_', "");
        if let Some(kind) = Self::ALL
            .iter()
            .copied()
            .find(|k| k.tool_name().replace('_', "") == squashed)
        {
            return Some(kind);
        }
        let word = tool.trim_start_matches("render_").replace('_', "");
        if implied_chart_type(&word).is_some() {
            return Some(FigureKind::Chart);
        }
        match word.as_str() {
            // "donut: pie/donut charts for categorical proportions".
            "pie" | "piechart" | "doughnut" | "doughnutchart" => Some(FigureKind::Donut),
            _ => None,
        }
    }
}

/// The `ChartData` `type` a kind alias has already named.
///
/// `{"kind": "bar"}` states the chart type in the kind, so the payload beside it
/// usually carries no `type` of its own — and `ChartData::chart_type` is
/// required. Refusing that call would mean rejecting a model for saying what it
/// wanted twice rather than once.
fn implied_chart_type(word: &str) -> Option<&'static str> {
    match word {
        "bar" | "barchart" | "column" | "columnchart" => Some("bar"),
        "line" | "linechart" => Some("line"),
        "scatter" | "scatterplot" | "scatterchart" => Some("scatter"),
        _ => None,
    }
}

impl<'de> Deserialize<'de> for FigureKind {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error as DeError;
        let raw = String::deserialize(d)?;
        FigureKind::from_alias(&raw).ok_or_else(|| {
            DeError::custom(format!(
                "unknown figure kind {raw:?}; call describe_figure for the list of kinds"
            ))
        })
    }
}

/// Turn a `render_figure` payload into the arguments the underlying tool takes.
///
/// All thirty-two land on `{"data": …}` — including `donut`, whose
/// `#[serde(flatten)]` looks irregular in Rust but flattens a struct whose only
/// field is itself named `data`, so its wire shape is the same as the rest.
///
/// ⚠ `mermaid` used to be the one exception, reshaped here into
/// `{"mermaid_code": …}`. That leniency now lives on `RenderMermaidParams`'s own
/// `Deserialize` instead, and it had to move: `render_figure` is only ONE of the
/// four doors into `render_mermaid`. A panel written `{"kind": "mermaid",
/// "data": "<source>"}` carries no `tool` key, so `DashboardFigure` reads `kind`
/// as the tool name, resolves it straight to `render_mermaid` and never passes
/// through here — and the tool then refused a payload that was, by
/// `describe_figure`'s own account, correct.
fn figure_arguments(data: Value) -> Value {
    json!({ "data": data })
}

/// The name of the one figure tool the model is actually offered.
const RENDER_FIGURE: &str = "render_figure";

/// Pull `{kind, data}` out of a `render_figure` call's arguments.
///
/// This is the nested, model-authored half of the call, so it matches
/// [`DashboardFigure`]'s own deserializer where that leniency is earned, and for
/// the same reason: a strict `serde_json::from_value::<RenderFigureParams>`
/// would refuse a payload whose only fault is that the model flattened `data`
/// away, spelled it `params` because that is the key the panel above it uses, or
/// stringified the whole object — all three are shapes models are recorded
/// sending in this codebase.
///
/// It stays narrower in the one place that matters: it hunts no aliases for
/// `kind`. Two reasons, and only the second holds in every case. When a panel
/// omitted `tool`, `DashboardFigure` will already have consumed `type`/`name`/
/// `kind` hunting for the tool name, so a second hunt here would be re-reading
/// keys that were meant as the tool. Independently — and this is the one that
/// bites when `tool` WAS present — a figure's own `type` field (`show_chart`
/// payloads have one) must not be able to decide which figure gets drawn.
fn render_figure_call(
    params: Value,
    vocab: FigureVocabulary,
) -> Result<(FigureKind, Value), ErrorData> {
    // Some models stringify nested tool-call arguments; `DashboardFigure`
    // accepts that and this used to refuse it, so a stringified panel body
    // failed one level in with a message about `kind` rather than about the
    // string. (`normalize_dashboard_args` does the same for `sections`/`panels`.)
    let params = match params {
        Value::String(s) => serde_json::from_str::<Value>(&s).map_err(|e| {
            invalid(format!(
                "`render_figure` arguments were a JSON string that did not parse: {e}."
            ))
        })?,
        other => other,
    };
    let mut map = match params {
        Value::Object(map) => map,
        other => {
            return Err(invalid(format!(
                "`render_figure` takes an object with a `kind` and that kind's payload as \
                 `data`; got {other}."
            )))
        }
    };

    // A call wrapped the way a dashboard panel writes one:
    // `{"tool": "render_figure", "params": {"kind": …, "data": …}}`. Descend
    // before hunting `kind` — and note WHY this shape reaches the declared door
    // at all: until this change the missing-`kind` error handed every caller the
    // panel spelling as its worked example, so a direct caller that followed the
    // advice it was just given failed a second time on the shape the message
    // recommended.
    if !map.contains_key("kind") {
        if let Some(inner) = ["params", "arguments", "args", "data"]
            .iter()
            .filter_map(|key| map.get(*key))
            .find_map(|value| match de_stringified(value.clone()) {
                Value::Object(inner) if inner.contains_key("kind") => Some(inner),
                _ => None,
            })
        {
            map = inner;
        }
    }

    let raw_kind = map
        .remove("kind")
        .ok_or_else(|| invalid(missing_kind_error(vocab)))?;
    let kind: FigureKind = serde_json::from_value(raw_kind.clone()).map_err(|_| {
        invalid(format!(
            "Unknown figure kind {raw_kind}. Call describe_figure for the list of kinds."
        ))
    })?;

    // An explicit payload key wins; otherwise whatever is left on the object IS
    // the payload, which is how a model that wrote `{"kind": "chart", "type":
    // "bar", "datasets": […]}` still lands on a figure.
    let mut data = ["data", "params", "arguments", "args"]
        .iter()
        .find_map(|key| map.remove(*key))
        .unwrap_or(Value::Object(map));

    // `{"kind": "bar", …}` already named the chart type; see `implied_chart_type`.
    if kind == FigureKind::Chart {
        if let (Some(implied), Value::Object(fields)) = (
            raw_kind
                .as_str()
                .map(|raw| normalize_tool_name(raw).trim_start_matches("render_").replace('_', ""))
                .as_deref()
                .and_then(implied_chart_type),
            &mut data,
        ) {
            fields
                .entry("type")
                .or_insert_with(|| Value::String(implied.to_string()));
        }
    }
    Ok((kind, data))
}

/// The example a missing `kind` is answered with, in the caller's own vocabulary.
///
/// ⚠ The two are NOT interchangeable, and the wrong one costs the round trip
/// this message exists to save. `render_figure`'s declared door takes
/// `{"kind": …, "data": …}` directly; a dashboard panel wraps that same object
/// as `{"tool": "render_figure", "params": {…}}`. Handing the panel spelling to
/// a direct caller told it to retry with a shape that door then refused. Both
/// are accepted now, but the example should still be the one the caller's own
/// surface documents — a model copies the example it is given.
fn missing_kind_error(vocab: FigureVocabulary) -> String {
    let example = match vocab {
        FigureVocabulary::RenderFigure => {
            "{\"kind\": \"chart\", \"data\": {\"type\": \"bar\", \
             \"labels\": [\"a\", \"b\"], \
             \"datasets\": [{\"label\": \"Value\", \"data\": [1, 2]}]}}"
        }
        FigureVocabulary::ToolName => {
            "{\"tool\": \"render_figure\", \"params\": {\"kind\": \"volcano\", \"data\": {…}}}"
        }
    };
    format!(
        "`render_figure` needs a `kind` naming the figure to draw, e.g. {example}. \
         Call describe_figure for the list of kinds."
    )
}

/// Which figure names the caller's model can actually emit.
///
/// `call_figure_tool` has two doors and they hand out different rosters, so an
/// error phrased for one is a dead end for the other. Only the error paths read
/// this — dispatch is identical either way, which is what keeps a figure
/// byte-for-byte the same however it is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FigureVocabulary {
    /// A chat agent with the Auto Visualiser extension. It sees exactly three
    /// tools — `render_figure`, `describe_figure`, `render_dashboard` — so its
    /// figures are named by `kind`, and the per-kind tool names are back-compat
    /// it cannot look up.
    RenderFigure,
    /// Agent Drafter's `ui_figure`, whose own description hands the app agent
    /// the per-kind tool names (`render_volcano`, `render_manhattan`, …) and
    /// which has neither `render_figure` nor `describe_figure` to call.
    ToolName,
}

/// Report a bad figure payload against a call the caller can actually make.
///
/// The 32 per-figure tools still dispatch, but they left the advertised roster
/// when `render_figure` took over. Handing one of their names back to a CHAT
/// agent — "`render_volcano` arguments are invalid: missing field `log2fc`" —
/// gives it an identifier it cannot call, so it either retries the rejected
/// shape or gives up (#150).
///
/// ⚠ "Left the roster" is true of Auto Visualiser's OWN advertised surface and
/// nothing wider — which is why this takes a vocabulary rather than rewriting
/// unconditionally. `ui_figure`'s description still lists eight of these names
/// (plus `render_dashboard`, which IS advertised) to an audience that has
/// nothing else to call. Auto Visualiser's own surface is now pinned clean by
/// `the_advertised_surface_names_no_retired_figure_tool`; the `render_dashboard`
/// back-compat shapes moved to `//` comments beside [`DashboardFigure`] to make
/// that true.
fn figure_argument_error(vocab: FigureVocabulary, tool: &str, detail: &str) -> String {
    let by_tool_name = || format!("`{tool}` arguments are invalid: {detail}");
    if vocab == FigureVocabulary::ToolName {
        return by_tool_name();
    }
    match FigureKind::ALL.iter().find(|kind| kind.tool_name() == tool) {
        Some(kind) => format!(
            "`render_figure` arguments are invalid for kind \"{slug}\": {detail}. \
             Call describe_figure with kind \"{slug}\" for the exact schema and a worked example.",
            slug = kind.slug()
        ),
        // A dispatch-table name that no kind claims. Not dead code, and not
        // pinned as such: `every_kind_resolves_to_a_real_figure_tool` joins
        // `figure_kinds!` to the ROUTER, not to the hand-written dispatch table,
        // so it cannot see an extra table entry. What is pinned is the branch
        // that matters — `every_kind_reports_bad_arguments_against_render_figure`
        // walks all 32 kinds through the real dispatcher and asserts none of
        // them lands here. A table entry outside `figure_kinds!` would be a tool
        // reached only by its own name, and naming it back is then correct.
        None => by_tool_name(),
    }
}

/// Report an unrecognised figure name in a vocabulary the caller has.
///
/// The old text named `render_volcano`, `render_heatmap` and `show_chart` to
/// every caller. For a chat agent all three are unreachable (#142): it is being
/// told to retry with tools that are not in its roster, which is the same
/// dead end #150 describes for argument errors, one layer up.
fn unknown_figure_error(vocab: FigureVocabulary, name: &str) -> String {
    match vocab {
        FigureVocabulary::RenderFigure => {
            let kinds: Vec<&str> = FigureKind::ALL.iter().map(|kind| kind.slug()).collect();
            format!(
                "Unknown visualization '{name}'. Figures are drawn with `render_figure`, \
                 passing one of these kinds as `kind`: {}. Call describe_figure with a kind \
                 for its exact schema and a worked example.",
                kinds.join(", ")
            )
        }
        // `ui_figure` really does take a tool name, so this half is unchanged.
        FigureVocabulary::ToolName => format!(
            "Unknown visualization '{name}'. Use one of the Auto Visualiser render_* tool \
             names, e.g. render_volcano, render_heatmap, show_chart."
        ),
    }
}

/// Parameters for `render_figure`.
#[derive(Debug, Serialize, rmcp::schemars::JsonSchema)]
pub struct RenderFigureParams {
    /// Which figure to draw.
    pub kind: FigureKind,
    /// The figure's payload. Its shape depends on `kind` — call
    /// `describe_figure` with that kind for the exact schema and a worked
    /// example. For `kind: "chart"`: {"type": "bar", "labels": ["a", "b"],
    /// "datasets": [{"label": "Value", "data": [1, 2]}]}.
    pub data: Value,
}

/// ⚠ **Hand-written, and this is the fix.** The declared `render_figure` had a
/// DERIVED `Deserialize`, so `rmcp` refused a model's arguments inside
/// `Parameters::from_context_part` — `serde_json::from_value::<RenderFigureParams>`
/// — and answered with its own "failed to deserialize parameters: missing field
/// `data`". That message names no kind and no `describe_figure`, and it arrives
/// before any Auto Visualiser code runs, so none of the careful phrasing in
/// `figure_argument_error` could reach the model.
///
/// Meanwhile `render_figure_call` — which unwraps exactly the shapes models are
/// recorded sending — sat one door over, wired only to dashboard panels and
/// Agent Drafter's `ui_figure`. Two doors, two behaviours, and the lenient one
/// was not the door a chat agent uses. Measured on Versa GPT-5.5: "render a bar
/// chart of three values" cost four or five tool-call cards, two or three of
/// them rejections, before the model gave in and called `describe_figure`.
///
/// This routes the declared door through the same parser, so the two agree.
/// `render_figure`'s body then calls in as `kind.tool_name()`, which is never
/// `render_figure`, so `call_figure_tool`'s own unwrap is not re-entered and the
/// payload is unwrapped exactly once.
impl<'de> Deserialize<'de> for RenderFigureParams {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error as DeError;
        let (kind, data) =
            render_figure_call(Value::deserialize(d)?, FigureVocabulary::RenderFigure)
                .map_err(|e| DeError::custom(e.message))?;
        Ok(RenderFigureParams { kind, data })
    }
}

/// Parameters for `describe_figure`.
#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct DescribeFigureParams {
    /// A single kind to describe in full. Omit for the one-line catalog of
    /// every kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<FigureKind>,
}

#[tool_router(router = entry_router)]
impl AutoVisualiserRouter {
    /// The declared entry point for every single-figure visualization.
    //
    // ⚠ Keep this description SHORT. Azure/OpenAI cap
    // `tools[n].function.description` at 1024 characters and reject the whole
    // request — a second non-retryable 400 on the same provider this
    // consolidation exists to avoid. Per-kind documentation belongs in
    // `describe_figure`, which has no such budget because it is a RESULT.
    #[tool(
        name = "render_figure",
        description = "Draw one interactive figure. Pick `kind` from the enum, and pass that kind's payload as `data`. Example: {\"kind\": \"chart\", \"data\": {\"type\": \"bar\", \"labels\": [\"a\", \"b\", \"c\"], \"datasets\": [{\"label\": \"Value\", \"data\": [1, 2, 3]}]}}. The Auto Visualiser instructions in your system prompt list what each kind is for; call describe_figure with a kind to get its exact schema and a worked example. For an answer that needs more than one figure, call render_dashboard once instead of calling this repeatedly."
    )]
    pub async fn render_figure(
        &self,
        Parameters(params): Parameters<RenderFigureParams>,
    ) -> Result<CallToolResult, ErrorData> {
        // Resolved here, so the dispatcher is entered as e.g. `render_volcano`
        // and the `render_figure` unwrap inside it is never reached from this
        // door. It is reached from the two doors whose `tool` is model-written.
        let arguments = figure_arguments(params.data);
        self.call_figure_tool(
            params.kind.tool_name(),
            arguments,
            FigureVocabulary::RenderFigure,
        )
        .await
    }

    /// The per-kind schema and worked example, read off the real tool.
    #[tool(
        name = "describe_figure",
        description = "Get the exact `data` schema and a worked example for one render_figure kind, or omit `kind` for the catalog of every kind. Call this before render_figure whenever you are unsure of a payload's shape."
    )]
    pub async fn describe_figure(
        &self,
        Parameters(params): Parameters<DescribeFigureParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let Some(kind) = params.kind else {
            let catalog: Vec<Value> = FigureKind::ALL
                .iter()
                .map(|kind| json!({ "kind": kind.slug() }))
                .collect();
            return Ok(CallToolResult::success(vec![rmcp::model::Content::text(
                serde_json::to_string_pretty(&json!({
                    "kinds": catalog,
                    "usage": "render_figure({ kind, data }). Call describe_figure with one kind \
                              for its schema and a worked example.",
                }))
                .unwrap_or_default(),
            )]));
        };

        // Read the REAL declaration rather than a second copy of it. The 32
        // tools keep their `#[tool]` attributes precisely so this cannot drift.
        let tool_name = kind.tool_name();
        let described = self
            .figure_router
            .list_all()
            .into_iter()
            .find(|tool| tool.name.as_ref() == tool_name)
            .ok_or_else(|| {
                invalid(format!(
                    "`{}` has no underlying figure tool; this is a Biorouter bug.",
                    kind.slug()
                ))
            })?;

        let payload = json!({
            "kind": kind.slug(),
            // `mermaid` is the one kind whose payload is not the underlying
            // tool's `data` field, so say what `data` means for it rather than
            // handing back a schema keyed on a name the caller cannot use.
            "dataIs": if kind == FigureKind::Mermaid {
                "the Mermaid diagram source, as a string"
            } else {
                "the value of the underlying tool's `data` argument"
            },
            "schema": Value::Object((*described.input_schema).clone()),
            "guidance": described.description.as_deref().unwrap_or_default(),
        });
        Ok(CallToolResult::success(vec![rmcp::model::Content::text(
            serde_json::to_string_pretty(&payload).unwrap_or_default(),
        )]))
    }
}
