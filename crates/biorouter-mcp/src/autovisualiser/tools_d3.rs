// D3-based tools: network, heatmap, sunburst, dendrogram, calendar_heatmap,
// boxplot, wordcloud, kaplan_meier, forest.
//
// sunburst and dendrogram reuse the hierarchical `TreemapNode` defined in mod.rs.

// ----- render_network ------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct NetworkNode {
    /// Unique node id
    pub id: String,
    /// Display label (defaults to id)
    #[serde(default)]
    pub label: Option<String>,
    /// Group/cluster for coloring
    #[serde(default)]
    pub group: Option<String>,
    /// Relative size
    #[serde(default)]
    pub value: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct NetworkLink {
    /// Source node id
    pub source: String,
    /// Target node id
    pub target: String,
    /// Edge weight (affects thickness)
    #[serde(default)]
    pub value: Option<f64>,
    /// Optional edge label
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct NetworkData {
    /// Graph nodes
    pub nodes: Vec<NetworkNode>,
    /// Graph edges
    pub links: Vec<NetworkLink>,
    /// Draw arrowheads (directed graph). Default false.
    #[serde(default)]
    pub directed: Option<bool>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RenderNetworkParams {
    #[serde(deserialize_with = "common::de_flexible")]
    pub data: NetworkData,
}

// ----- render_heatmap ------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct HeatmapData {
    /// Column labels (x-axis)
    #[serde(rename = "xLabels")]
    pub x_labels: Vec<String>,
    /// Row labels (y-axis)
    #[serde(rename = "yLabels")]
    pub y_labels: Vec<String>,
    /// values[row][col] — one row per y label, one entry per x label
    pub values: Vec<Vec<f64>>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, rename = "xAxisLabel")]
    pub x_axis_label: Option<String>,
    #[serde(default, rename = "yAxisLabel")]
    pub y_axis_label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RenderHeatmapParams {
    #[serde(deserialize_with = "common::de_flexible")]
    pub data: HeatmapData,
}

// ----- render_sunburst / render_dendrogram (reuse TreemapNode) -------------

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RenderSunburstParams {
    /// Hierarchical root: {name, value?, children?, category?}
    #[serde(deserialize_with = "common::de_flexible")]
    pub data: TreemapNode,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RenderDendrogramParams {
    /// Hierarchical root: {name, value?, children?, category?}
    #[serde(deserialize_with = "common::de_flexible")]
    pub data: TreemapNode,
}

// ----- render_calendar_heatmap --------------------------------------------

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct CalendarDay {
    /// Date in YYYY-MM-DD format
    pub date: String,
    /// Value for that day
    pub value: f64,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct CalendarData {
    /// One entry per day
    pub values: Vec<CalendarDay>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RenderCalendarParams {
    #[serde(deserialize_with = "common::de_flexible")]
    pub data: CalendarData,
}

// ----- render_boxplot ------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct BoxGroup {
    /// Group label
    pub label: String,
    /// Raw numeric values (quartiles computed automatically)
    pub values: Vec<f64>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct BoxplotData {
    /// Groups to compare
    pub groups: Vec<BoxGroup>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, rename = "yAxisLabel")]
    pub y_axis_label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RenderBoxplotParams {
    #[serde(deserialize_with = "common::de_flexible")]
    pub data: BoxplotData,
}

// ----- render_wordcloud ----------------------------------------------------

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct Word {
    /// The term
    pub text: String,
    /// Weight/frequency (controls size)
    pub weight: f64,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct WordCloudData {
    /// Words with weights
    pub words: Vec<Word>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RenderWordcloudParams {
    #[serde(deserialize_with = "common::de_flexible")]
    pub data: WordCloudData,
}

// ----- render_kaplan_meier -------------------------------------------------

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct KmPoint {
    /// Time
    pub time: f64,
    /// Survival probability at this time (0..1)
    pub survival: f64,
    /// Whether this is a censoring event (draws a tick)
    #[serde(default)]
    pub censored: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct KmGroup {
    /// Group label
    pub label: String,
    /// Survival points (ascending time). The curve is drawn as a step function.
    pub points: Vec<KmPoint>,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct KaplanMeierData {
    /// One or more survival groups
    pub groups: Vec<KmGroup>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, rename = "xAxisLabel")]
    pub x_axis_label: Option<String>,
    #[serde(default, rename = "yAxisLabel")]
    pub y_axis_label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RenderKaplanMeierParams {
    #[serde(deserialize_with = "common::de_flexible")]
    pub data: KaplanMeierData,
}

// ----- render_forest -------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ForestRow {
    /// Study/variable label
    pub label: String,
    /// Point estimate (e.g. odds ratio, hazard ratio, mean difference)
    pub estimate: f64,
    /// Lower confidence bound
    pub lower: f64,
    /// Upper confidence bound
    pub upper: f64,
    /// Optional weight (controls marker size)
    #[serde(default)]
    pub weight: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ForestData {
    /// Rows of the forest plot
    pub rows: Vec<ForestRow>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, rename = "xAxisLabel")]
    pub x_axis_label: Option<String>,
    /// Reference line (null line). Default 1.0 (ratio scale); use 0 for differences.
    #[serde(default, rename = "referenceLine")]
    pub reference_line: Option<f64>,
    /// Use a log scale for the x-axis (typical for odds/hazard ratios)
    #[serde(default, rename = "logScale")]
    pub log_scale: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RenderForestParams {
    #[serde(deserialize_with = "common::de_flexible")]
    pub data: ForestData,
}

// ===========================================================================
// Tools
// ===========================================================================

#[tool_router(router = d3_router)]
impl AutoVisualiserRouter {
    /// Force-directed network graph
    #[tool(
        name = "render_network",
        description = r#"Render an interactive force-directed network (node-link) graph. Ideal for knowledge graphs, gene/protein interaction networks, dependency graphs.

- nodes (required): [{id, label?, group?, value?}]; group colors nodes, value sizes them
- links (required): [{source, target, value?, label?}]; source/target reference node ids
- directed (optional, default false): draw arrowheads
- title (optional)

Example:
{"nodes":[{"id":"TP53","group":"tumor"},{"id":"MDM2","group":"regulator"}],"links":[{"source":"MDM2","target":"TP53","value":3}],"directed":true}"#
    )]
    pub async fn render_network(
        &self,
        params: Parameters<RenderNetworkParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let d = &params.0.data;
        if d.nodes.is_empty() {
            return Err(invalid("Network requires at least one node."));
        }
        check_limit(d.nodes.len(), MAX_NODES, "nodes")?;
        check_limit(d.links.len(), MAX_LINKS, "links")?;
        let ids: std::collections::HashSet<&str> = d.nodes.iter().map(|n| n.id.as_str()).collect();
        for l in &d.links {
            if !ids.contains(l.source.as_str()) {
                return Err(invalid(format!(
                    "Network link references unknown source node '{}'.",
                    l.source
                )));
            }
            if !ids.contains(l.target.as_str()) {
                return Err(invalid(format!(
                    "Network link references unknown target node '{}'.",
                    l.target
                )));
            }
        }
        let data_json = js_value(d)?;
        render(
            "ui://network/graph",
            "network",
            "Network graph created for the artifact panel.",
            include_str!("templates/network_template.html"),
            &[Asset::D3],
            &[("{{NETWORK_DATA}}", &data_json)],
        )
    }

    /// Heatmap (matrix as a color grid)
    #[tool(
        name = "render_heatmap",
        description = r#"Render a heatmap of a matrix as a colored grid (expression matrices, correlation matrices, confusion matrices).

- xLabels (required): column labels
- yLabels (required): row labels
- values (required): values[row][col]; one row per yLabel, one entry per xLabel
- title, xAxisLabel, yAxisLabel (optional)

Example:
{"xLabels":["S1","S2"],"yLabels":["GeneA","GeneB"],"values":[[1.2,-0.4],[0.0,2.1]]}"#
    )]
    pub async fn render_heatmap(
        &self,
        params: Parameters<RenderHeatmapParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let d = &params.0.data;
        if d.x_labels.is_empty() || d.y_labels.is_empty() {
            return Err(invalid("Heatmap requires non-empty xLabels and yLabels."));
        }
        check_limit(d.x_labels.len(), MAX_LABELS, "columns")?;
        check_limit(d.y_labels.len(), MAX_LABELS, "rows")?;
        if d.values.len() != d.y_labels.len() {
            return Err(invalid(format!(
                "Heatmap has {} value rows but {} yLabels; they must match.",
                d.values.len(),
                d.y_labels.len()
            )));
        }
        for (i, row) in d.values.iter().enumerate() {
            if row.len() != d.x_labels.len() {
                return Err(invalid(format!(
                    "Heatmap row {i} has {} values but there are {} xLabels.",
                    row.len(),
                    d.x_labels.len()
                )));
            }
        }
        let data_json = js_value(d)?;
        render(
            "ui://heatmap/chart",
            "heatmap",
            "Heatmap created for the artifact panel.",
            include_str!("templates/heatmap_template.html"),
            &[Asset::D3],
            &[("{{HEATMAP_DATA}}", &data_json)],
        )
    }

    /// Sunburst (radial hierarchy)
    #[tool(
        name = "render_sunburst",
        description = r#"Render a sunburst chart for hierarchical part-of-whole data (radial treemap).

Data is a hierarchical root: {name, value?, children?: [...], category?}. Leaf nodes need a value.
All supplied values must be finite and non-negative. Aggregate values add a node's own value and its descendants' values.

Example:
{"name":"Body","children":[{"name":"Brain","children":[{"name":"Cortex","value":40},{"name":"Cerebellum","value":10}]},{"name":"Heart","value":20}]}"#
    )]
    pub async fn render_sunburst(
        &self,
        params: Parameters<RenderSunburstParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let d = &params.0.data;
        let (count, depth) = treemap_stats(d, 1);
        check_limit(count, MAX_NODES, "nodes")?;
        if depth > MAX_TREE_DEPTH {
            return Err(invalid("Sunburst nesting is too deep."));
        }
        let mut pending = vec![d];
        while let Some(node) = pending.pop() {
            if node
                .value
                .is_some_and(|value| !value.is_finite() || value < 0.0)
            {
                return Err(invalid("Sunburst values must be finite and non-negative."));
            }
            if let Some(children) = &node.children {
                pending.extend(children);
            }
        }
        let data_json = js_value(d)?;
        render(
            "ui://sunburst/chart",
            "sunburst",
            "Sunburst created for the artifact panel.",
            include_str!("templates/sunburst_template.html"),
            &[Asset::D3],
            &[("{{SUNBURST_DATA}}", &data_json)],
        )
    }

    /// Dendrogram (hierarchical tree)
    #[tool(
        name = "render_dendrogram",
        description = r#"Render a dendrogram / hierarchical tree (clustering results, taxonomies, phylogenies, org charts).

Data is a hierarchical root: {name, children?: [...], value?, category?}.

Example:
{"name":"root","children":[{"name":"Cluster A","children":[{"name":"x"},{"name":"y"}]},{"name":"Cluster B","children":[{"name":"z"}]}]}"#
    )]
    pub async fn render_dendrogram(
        &self,
        params: Parameters<RenderDendrogramParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let d = &params.0.data;
        let (count, depth) = treemap_stats(d, 1);
        check_limit(count, MAX_NODES, "nodes")?;
        if depth > MAX_TREE_DEPTH {
            return Err(invalid("Dendrogram nesting is too deep."));
        }
        let data_json = js_value(d)?;
        render(
            "ui://dendrogram/chart",
            "dendrogram",
            "Dendrogram created for the artifact panel.",
            include_str!("templates/dendrogram_template.html"),
            &[Asset::D3],
            &[("{{DENDROGRAM_DATA}}", &data_json)],
        )
    }

    /// Calendar heatmap (value per day)
    #[tool(
        name = "render_calendar_heatmap",
        description = r#"Render a calendar heatmap (GitHub-style) showing a value for each day.

- values (required): [{date: "YYYY-MM-DD", value}]
- Dates must be valid calendar dates spanning no more than 3,660 days.
- For duplicate dates, the last supplied value is used in the cells and scale.
- title (optional)

Example:
{"title":"Activity","values":[{"date":"2024-01-01","value":3},{"date":"2024-01-02","value":7}]}"#
    )]
    pub async fn render_calendar_heatmap(
        &self,
        params: Parameters<RenderCalendarParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let d = &params.0.data;
        if d.values.is_empty() {
            return Err(invalid("Calendar heatmap requires at least one day."));
        }
        check_limit(d.values.len(), MAX_VALUES, "days")?;
        let dates = d
            .values
            .iter()
            .map(|day| {
                chrono::NaiveDate::parse_from_str(&day.date, "%Y-%m-%d")
                    .ok()
                    .filter(|date| {
                        day.date.len() == 10 && date.format("%Y-%m-%d").to_string() == day.date
                    })
                    .ok_or_else(|| {
                        invalid(format!(
                            "Invalid calendar date '{}'; use YYYY-MM-DD.",
                            day.date
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (first, last) = dates
            .iter()
            .fold((dates[0], dates[0]), |(first, last), date| {
                (first.min(*date), last.max(*date))
            });
        if (last - first).num_days() + 1 > 3660 {
            return Err(invalid("Calendar date range exceeds 3,660 days. Choose a shorter range; no dates were silently omitted."));
        }
        let data_json = js_value(d)?;
        render(
            "ui://calendar/heatmap",
            "calendar",
            "Calendar heatmap created for the artifact panel.",
            include_str!("templates/calendar_template.html"),
            &[Asset::D3],
            &[("{{CALENDAR_DATA}}", &data_json)],
        )
    }

    /// Box plot (distribution comparison)
    #[tool(
        name = "render_boxplot",
        description = r#"Render box plots comparing the distribution/spread of several groups (quartiles, whiskers, outliers).

- groups (required): [{label, values: [numbers]}]
- Mixed empty groups are retained as no-data groups. Observations must be finite.
- title, yAxisLabel (optional)

Example:
{"title":"Expression","yAxisLabel":"TPM","groups":[{"label":"Control","values":[5,6,7,6,8,5,20]},{"label":"Treated","values":[10,12,11,13,12,11]}]}"#
    )]
    pub async fn render_boxplot(
        &self,
        params: Parameters<RenderBoxplotParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let d = &params.0.data;
        if d.groups.is_empty() {
            return Err(invalid("Box plot requires at least one group."));
        }
        if d.groups.iter().all(|g| g.values.is_empty()) {
            return Err(invalid("Box plot groups require at least one value."));
        }
        check_limit(d.groups.len(), MAX_LABELS, "groups")?;
        check_limit(
            d.groups.iter().map(|group| group.values.len()).sum(),
            MAX_VALUES,
            "observations",
        )?;
        if d.groups
            .iter()
            .flat_map(|group| &group.values)
            .any(|value| !value.is_finite())
        {
            return Err(invalid("Box plot observations must be finite numbers."));
        }
        let data_json = js_value(d)?;
        render(
            "ui://boxplot/chart",
            "boxplot",
            "Box plot created for the artifact panel.",
            include_str!("templates/boxplot_template.html"),
            &[Asset::D3],
            &[("{{BOXPLOT_DATA}}", &data_json)],
        )
    }

    /// Word cloud (term frequencies)
    #[tool(
        name = "render_wordcloud",
        description = r#"Render a word cloud where size encodes weight/frequency.

- words (required): [{text, weight}]
- Terms must be non-blank; weights must be finite and non-negative. Zero is allowed. Terms that do not fit remain in the bounded data table, with an explicit omission count.
- title (optional)

Example:
{"title":"Topics","words":[{"text":"genomics","weight":40},{"text":"AI","weight":30},{"text":"clinical","weight":18}]}"#
    )]
    pub async fn render_wordcloud(
        &self,
        params: Parameters<RenderWordcloudParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let d = &params.0.data;
        if d.words.is_empty() {
            return Err(invalid("Word cloud requires at least one word."));
        }
        check_limit(d.words.len(), MAX_LABELS, "words")?;
        if d.words.iter().any(|word| {
            word.text.trim().is_empty() || !word.weight.is_finite() || word.weight < 0.0
        }) {
            return Err(invalid(
                "Word cloud terms must be non-blank and weights finite and non-negative.",
            ));
        }
        let data_json = js_value(d)?;
        render(
            "ui://wordcloud/chart",
            "wordcloud",
            "Word cloud created for the artifact panel.",
            include_str!("templates/wordcloud_template.html"),
            &[Asset::D3],
            &[("{{WORDCLOUD_DATA}}", &data_json)],
        )
    }

    /// Kaplan–Meier survival curves
    #[tool(
        name = "render_kaplan_meier",
        description = r#"Render Kaplan–Meier survival curves (step functions, optional censoring ticks).

- groups (required): [{label, points: [{time, survival (0..1), censored?}], color?}]
  points should be ordered by ascending time; survival is the cumulative survival probability.
- title, xAxisLabel, yAxisLabel (optional)

Example:
{"title":"Survival","groups":[{"label":"Arm A","points":[{"time":0,"survival":1.0},{"time":5,"survival":0.8},{"time":10,"survival":0.6,"censored":true}]}]}"#
    )]
    pub async fn render_kaplan_meier(
        &self,
        params: Parameters<RenderKaplanMeierParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let d = &params.0.data;
        if d.groups.is_empty() {
            return Err(invalid("Kaplan–Meier plot requires at least one group."));
        }
        if d.groups.iter().all(|g| g.points.is_empty()) {
            return Err(invalid("Kaplan–Meier groups require at least one point."));
        }
        let data_json = js_value(d)?;
        render(
            "ui://kaplanmeier/chart",
            "kaplan_meier",
            "Kaplan–Meier plot created for the artifact panel.",
            include_str!("templates/kaplan_meier_template.html"),
            &[Asset::D3],
            &[("{{KM_DATA}}", &data_json)],
        )
    }

    /// Forest plot (effect sizes with CIs)
    #[tool(
        name = "render_forest",
        description = r#"Render a forest plot of effect sizes with confidence intervals (meta-analysis, odds/hazard ratios).

- rows (required): [{label, estimate, lower, upper, weight?}]
- Weights must be finite and non-negative. Omitted weights default to 1; zero retains a minimum visible marker.
- referenceLine (optional): null line (default 1.0; use 0 for mean differences)
- logScale (optional): log x-axis (typical for ratios)
- title, xAxisLabel (optional)

Example:
{"title":"Odds ratios","logScale":true,"rows":[{"label":"Study 1","estimate":1.4,"lower":1.1,"upper":1.8,"weight":3},{"label":"Study 2","estimate":0.9,"lower":0.6,"upper":1.3}]}"#
    )]
    pub async fn render_forest(
        &self,
        params: Parameters<RenderForestParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let d = &params.0.data;
        if d.rows.is_empty() {
            return Err(invalid("Forest plot requires at least one row."));
        }
        check_limit(d.rows.len(), MAX_LABELS, "rows")?;
        for r in &d.rows {
            if r.weight
                .is_some_and(|weight| !weight.is_finite() || weight < 0.0)
            {
                return Err(invalid("Forest weights must be finite and non-negative."));
            }
            if r.lower > r.upper {
                return Err(invalid(format!(
                    "Forest row '{}' has lower bound greater than upper bound.",
                    r.label
                )));
            }
            if d.log_scale.unwrap_or(false) && (r.lower <= 0.0 || r.estimate <= 0.0) {
                return Err(invalid(format!(
                    "Forest row '{}' has non-positive values, which are invalid on a log scale.",
                    r.label
                )));
            }
        }
        let data_json = js_value(d)?;
        render(
            "ui://forest/chart",
            "forest",
            "Forest plot created for the artifact panel.",
            include_str!("templates/forest_template.html"),
            &[Asset::D3],
            &[("{{FOREST_DATA}}", &data_json)],
        )
    }
}
