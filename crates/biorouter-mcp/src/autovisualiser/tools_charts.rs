// Chart.js-based tools: histogram, bubble, area, gauge, volcano, manhattan.

// ----- render_histogram ----------------------------------------------------

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct HistogramData {
    /// Raw numeric values to bin
    pub values: Vec<f64>,
    /// Number of bins (optional; auto via Sturges if omitted)
    #[serde(default)]
    pub bins: Option<u32>,
    /// Optional title
    #[serde(default)]
    pub title: Option<String>,
    /// Optional bar color
    #[serde(default)]
    pub color: Option<String>,
    /// Optional x-axis label
    #[serde(default, rename = "xAxisLabel")]
    pub x_axis_label: Option<String>,
    /// Optional y-axis label
    #[serde(default, rename = "yAxisLabel")]
    pub y_axis_label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RenderHistogramParams {
    #[serde(deserialize_with = "common::de_flexible")]
    pub data: HistogramData,
}

// ----- render_bubble -------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct BubblePoint {
    /// X coordinate
    pub x: f64,
    /// Y coordinate
    pub y: f64,
    /// Radius (relative size)
    pub r: f64,
    /// Optional point label (shown in tooltip)
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct BubbleDataset {
    /// Series label
    pub label: String,
    /// Bubble points
    pub data: Vec<BubblePoint>,
    /// Optional color
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct BubbleData {
    /// One or more bubble series
    pub datasets: Vec<BubbleDataset>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, rename = "xAxisLabel")]
    pub x_axis_label: Option<String>,
    #[serde(default, rename = "yAxisLabel")]
    pub y_axis_label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RenderBubbleParams {
    #[serde(deserialize_with = "common::de_flexible")]
    pub data: BubbleData,
}

// ----- render_area ---------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct AreaDataset {
    /// Series label
    pub label: String,
    /// Y values (one per x-axis label)
    pub data: Vec<f64>,
    /// Optional color
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct AreaData {
    /// X-axis category labels
    pub labels: Vec<String>,
    /// One or more series
    pub datasets: Vec<AreaDataset>,
    /// Stack the series (default false)
    #[serde(default)]
    pub stacked: Option<bool>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, rename = "xAxisLabel")]
    pub x_axis_label: Option<String>,
    #[serde(default, rename = "yAxisLabel")]
    pub y_axis_label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RenderAreaParams {
    #[serde(deserialize_with = "common::de_flexible")]
    pub data: AreaData,
}

// ----- render_gauge --------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct GaugeThreshold {
    /// Upper bound of this band
    pub value: f64,
    /// Color for this band
    pub color: String,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct GaugeData {
    /// The value to display
    pub value: f64,
    /// Range minimum (default 0)
    #[serde(default)]
    pub min: Option<f64>,
    /// Range maximum (default 100)
    #[serde(default)]
    pub max: Option<f64>,
    /// Units/label shown under the value
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// Optional colored bands (ascending by value)
    #[serde(default)]
    pub thresholds: Vec<GaugeThreshold>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RenderGaugeParams {
    #[serde(deserialize_with = "common::de_flexible")]
    pub data: GaugeData,
}

// ----- render_volcano ------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct VolcanoPoint {
    /// Gene/feature label
    #[serde(default)]
    pub label: Option<String>,
    /// log2 fold-change (x-axis)
    #[serde(rename = "log2fc")]
    pub log2fc: f64,
    /// -log10(p-value) (y-axis)
    #[serde(rename = "negLog10P")]
    pub neg_log10_p: f64,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct VolcanoData {
    /// Points, one per gene/feature
    pub points: Vec<VolcanoPoint>,
    #[serde(default)]
    pub title: Option<String>,
    /// |log2FC| significance threshold (default 1.0)
    #[serde(default, rename = "fcThreshold")]
    pub fc_threshold: Option<f64>,
    /// -log10(p) significance threshold (default 1.301 ≈ p<0.05)
    #[serde(default, rename = "pThreshold")]
    pub p_threshold: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RenderVolcanoParams {
    #[serde(deserialize_with = "common::de_flexible")]
    pub data: VolcanoData,
}

// ----- render_manhattan ----------------------------------------------------

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ManhattanPoint {
    /// Chromosome (e.g. "1", "X")
    pub chrom: String,
    /// Base-pair position within the chromosome
    pub pos: f64,
    /// -log10(p-value)
    #[serde(rename = "negLog10P")]
    pub neg_log10_p: f64,
    /// Optional SNP/marker label
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ManhattanData {
    /// Association points
    pub points: Vec<ManhattanPoint>,
    #[serde(default)]
    pub title: Option<String>,
    /// Genome-wide significance line in -log10(p) (default 7.301 ≈ 5e-8)
    #[serde(default, rename = "significanceLine")]
    pub significance_line: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RenderManhattanParams {
    #[serde(deserialize_with = "common::de_flexible")]
    pub data: ManhattanData,
}

// ===========================================================================
// Tools
// ===========================================================================

#[tool_router(router = charts_router)]
impl AutoVisualiserRouter {
    /// Histogram of a single numeric variable
    #[tool(
        name = "render_histogram",
        description = r#"Render a histogram showing the distribution of a single numeric variable.

- values (required): array of numbers
- bins (optional): number of bins (auto if omitted)
- title, xAxisLabel, yAxisLabel, color (optional)

Example:
{"title":"Ages","values":[23,25,31,34,34,35,40,41,42,55],"bins":5}"#
    )]
    pub async fn render_histogram(
        &self,
        params: Parameters<RenderHistogramParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let d = &params.0.data;
        if d.values.is_empty() {
            return Err(invalid("Histogram requires at least one value."));
        }
        check_limit(d.values.len(), MAX_VALUES, "values")?;
        if d.values.iter().any(|v| !v.is_finite()) {
            return Err(invalid("Histogram values must all be finite numbers."));
        }
        let data_json = js_value(d)?;
        render(
            "ui://histogram/chart",
            "histogram",
            "Histogram created for the artifact panel.",
            include_str!("templates/histogram_template.html"),
            &[Asset::ChartJs],
            &[("{{HISTOGRAM_DATA}}", &data_json)],
        )
    }

    /// Bubble chart (x, y, size)
    #[tool(
        name = "render_bubble",
        description = r#"Render a bubble chart encoding three variables (x, y, and bubble size r).

- datasets (required): [{label, data: [{x, y, r, label?}], color?}]
- Coordinates must be finite; r is a finite non-negative radius in pixels. Zero-radius observations remain in the data table without a visible marker.
- title, xAxisLabel, yAxisLabel (optional)

Example:
{"title":"Markets","datasets":[{"label":"2024","data":[{"x":10,"y":20,"r":15,"label":"A"},{"x":30,"y":12,"r":8,"label":"B"}]}]}"#
    )]
    pub async fn render_bubble(
        &self,
        params: Parameters<RenderBubbleParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let d = &params.0.data;
        if d.datasets.is_empty() {
            return Err(invalid("Bubble chart requires at least one dataset."));
        }
        if d.datasets.iter().all(|ds| ds.data.is_empty()) {
            return Err(invalid("Bubble chart requires at least one point."));
        }
        check_limit(d.datasets.len(), MAX_LABELS, "datasets")?;
        check_limit(
            d.datasets.iter().map(|dataset| dataset.data.len()).sum(),
            MAX_VALUES,
            "points",
        )?;
        if d.datasets
            .iter()
            .flat_map(|dataset| &dataset.data)
            .any(|point| {
                !point.x.is_finite()
                    || !point.y.is_finite()
                    || !point.r.is_finite()
                    || point.r < 0.0
            })
        {
            return Err(invalid(
                "Bubble coordinates must be finite and radii finite and non-negative.",
            ));
        }
        let data_json = js_value(d)?;
        render(
            "ui://bubble/chart",
            "bubble",
            "Bubble chart created for the artifact panel.",
            include_str!("templates/bubble_template.html"),
            &[Asset::ChartJs],
            &[("{{BUBBLE_DATA}}", &data_json)],
        )
    }

    /// Area chart (optionally stacked)
    #[tool(
        name = "render_area",
        description = r#"Render an area chart (optionally stacked) for composition/trends over an ordered axis.

- labels (required): x-axis categories
- datasets (required): [{label, data: [numbers], color?}]
- stacked (optional, default false)
- Observations must be finite. Signed values and straight segments are preserved.
- title, xAxisLabel, yAxisLabel (optional)

Example:
{"title":"Traffic","labels":["Jan","Feb","Mar"],"stacked":true,"datasets":[{"label":"Web","data":[10,15,12]},{"label":"Mobile","data":[5,9,14]}]}"#
    )]
    pub async fn render_area(
        &self,
        params: Parameters<RenderAreaParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let d = &params.0.data;
        if d.labels.is_empty() {
            return Err(invalid("Area chart requires at least one x-axis label."));
        }
        if d.datasets.is_empty() {
            return Err(invalid("Area chart requires at least one dataset."));
        }
        check_limit(d.labels.len(), MAX_LABELS, "labels")?;
        check_limit(d.datasets.len(), MAX_LABELS, "datasets")?;
        check_limit(
            d.datasets.iter().map(|dataset| dataset.data.len()).sum(),
            MAX_VALUES,
            "observations",
        )?;
        for ds in &d.datasets {
            if ds.data.len() != d.labels.len() {
                return Err(invalid(format!(
                    "Dataset '{}' has {} values but there are {} labels; they must match.",
                    ds.label,
                    ds.data.len(),
                    d.labels.len()
                )));
            }
        }
        if d.datasets
            .iter()
            .flat_map(|dataset| &dataset.data)
            .any(|value| !value.is_finite())
        {
            return Err(invalid("Area chart observations must be finite numbers."));
        }
        let data_json = js_value(d)?;
        render(
            "ui://area/chart",
            "area",
            "Area chart created for the artifact panel.",
            include_str!("templates/area_template.html"),
            &[Asset::ChartJs],
            &[("{{AREA_DATA}}", &data_json)],
        )
    }

    /// Gauge / KPI dial
    #[tool(
        name = "render_gauge",
        description = r##"Render a gauge (dial) showing a single value against a range.

- value (required)
- min (default 0), max (default 100)
- label (units), title (optional)
- thresholds (optional): [{value, color}] ascending colored bands

Example:
{"title":"CPU","value":72,"min":0,"max":100,"label":"%","thresholds":[{"value":50,"color":"#2ecc71"},{"value":80,"color":"#f6c945"},{"value":100,"color":"#e74c3c"}]}"##
    )]
    pub async fn render_gauge(
        &self,
        params: Parameters<RenderGaugeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let d = &params.0.data;
        let min = d.min.unwrap_or(0.0);
        let max = d.max.unwrap_or(100.0);
        if !d.value.is_finite() || !min.is_finite() || !max.is_finite() {
            return Err(invalid("Gauge value/min/max must be finite numbers."));
        }
        if max <= min {
            return Err(invalid("Gauge 'max' must be greater than 'min'."));
        }
        let data_json = js_value(d)?;
        render(
            "ui://gauge/chart",
            "gauge",
            "Gauge created for the artifact panel.",
            include_str!("templates/gauge_template.html"),
            &[Asset::ChartJs],
            &[("{{GAUGE_DATA}}", &data_json)],
        )
    }

    /// Volcano plot (differential expression)
    #[tool(
        name = "render_volcano",
        description = r#"Render a volcano plot for differential-expression / statistical results.

- points (required): [{label?, log2fc, negLog10P}]
- fcThreshold (default 1.0): |log2FC| significance cutoff
- pThreshold (default 1.301 ≈ p<0.05): -log10(p) cutoff
- All observations must be finite; negative-log p-values and thresholds must be non-negative. Supply already-transformed values; the renderer does not adjust p-values.
- title (optional)

Points are colored up/down/non-significant against the thresholds.

Example:
{"title":"Tumor vs Normal","points":[{"label":"TP53","log2fc":2.4,"negLog10P":6.1},{"label":"GAPDH","log2fc":0.1,"negLog10P":0.3}]}"#
    )]
    pub async fn render_volcano(
        &self,
        params: Parameters<RenderVolcanoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let d = &params.0.data;
        if d.points.is_empty() {
            return Err(invalid("Volcano plot requires at least one point."));
        }
        check_limit(d.points.len(), MAX_VALUES, "points")?;
        if [d.fc_threshold, d.p_threshold]
            .into_iter()
            .flatten()
            .any(|value| !value.is_finite() || value < 0.0)
            || d.points.iter().any(|point| {
                !point.log2fc.is_finite()
                    || !point.neg_log10_p.is_finite()
                    || point.neg_log10_p < 0.0
            })
        {
            return Err(invalid("Volcano observations must be finite; thresholds and negative-log p-values must be non-negative."));
        }
        let data_json = js_value(d)?;
        render(
            "ui://volcano/chart",
            "volcano",
            "Volcano plot created for the artifact panel.",
            include_str!("templates/volcano_template.html"),
            &[Asset::ChartJs],
            &[("{{VOLCANO_DATA}}", &data_json)],
        )
    }

    /// Manhattan plot (GWAS)
    #[tool(
        name = "render_manhattan",
        description = r#"Render a Manhattan plot for genome-wide association results.

- points (required): [{chrom, pos, negLog10P, label?}]
- significanceLine (default 7.301 ≈ 5e-8): genome-wide significance threshold
- Positions, negative-log p-values and the threshold must be finite and non-negative.
- title (optional)

Points are grouped and colored by chromosome along a cumulative x-axis.

Example:
{"title":"GWAS","points":[{"chrom":"1","pos":12345,"negLog10P":3.2},{"chrom":"1","pos":98765,"negLog10P":8.1,"label":"rs123"},{"chrom":"2","pos":4567,"negLog10P":2.0}]}"#
    )]
    pub async fn render_manhattan(
        &self,
        params: Parameters<RenderManhattanParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let d = &params.0.data;
        if d.points.is_empty() {
            return Err(invalid("Manhattan plot requires at least one point."));
        }
        check_limit(d.points.len(), MAX_VALUES, "points")?;
        if d.significance_line
            .is_some_and(|value| !value.is_finite() || value < 0.0)
            || d.points.iter().any(|point| {
                !point.pos.is_finite()
                    || point.pos < 0.0
                    || !point.neg_log10_p.is_finite()
                    || point.neg_log10_p < 0.0
            })
        {
            return Err(invalid("Manhattan positions, negative-log p-values and significance thresholds must be finite and non-negative."));
        }
        let data_json = js_value(d)?;
        render(
            "ui://manhattan/chart",
            "manhattan",
            "Manhattan plot created for the artifact panel.",
            include_str!("templates/manhattan_template.html"),
            &[Asset::ChartJs],
            &[("{{MANHATTAN_DATA}}", &data_json)],
        )
    }
}
