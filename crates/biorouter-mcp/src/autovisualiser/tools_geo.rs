// Leaflet-based geo tool: choropleth (value-shaded regions from GeoJSON).

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ChoroplethData {
    /// A GeoJSON FeatureCollection describing the region boundaries
    pub geojson: Value,
    /// Name of a numeric property within each feature's `properties` to color by.
    /// Alternatively supply `values` keyed by an id property.
    #[serde(default, rename = "valueProperty")]
    pub value_property: Option<String>,
    /// Optional map of region-id -> value (used with `idProperty`)
    #[serde(default)]
    pub values: Option<std::collections::HashMap<String, f64>>,
    /// Feature property to use as the region id when matching `values`
    #[serde(default, rename = "idProperty")]
    pub id_property: Option<String>,
    /// Feature property to use for hover labels
    #[serde(default, rename = "nameProperty")]
    pub name_property: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, rename = "legendTitle")]
    pub legend_title: Option<String>,
    /// Optional initial center {lat, lng}
    #[serde(default)]
    pub center: Option<MapCenter>,
    /// Optional initial zoom level
    #[serde(default)]
    pub zoom: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, rmcp::schemars::JsonSchema)]
pub struct RenderChoroplethParams {
    #[serde(deserialize_with = "common::de_flexible")]
    pub data: ChoroplethData,
}

#[tool_router(router = geo_router)]
impl AutoVisualiserRouter {
    /// Choropleth map (value-shaded GeoJSON regions)
    #[tool(
        name = "render_choropleth",
        description = r#"Render a choropleth map: GeoJSON regions shaded by a value (disease prevalence by region, metrics by country/state, etc.).

- geojson (required): a GeoJSON FeatureCollection of region polygons
- valueProperty (optional): name of a numeric field in each feature's properties to color by
  OR values + idProperty: a {regionId: value} map matched on a feature property
- nameProperty (optional): feature property used for hover labels
- title, legendTitle, center {lat,lng}, zoom (optional)

Provide GeoJSON you have already obtained (e.g. read from a file or fetched). Example:
{"valueProperty":"cases","nameProperty":"name","geojson":{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"name":"Region A","cases":120},"geometry":{"type":"Polygon","coordinates":[[[0,0],[0,1],[1,1],[1,0],[0,0]]]}}]}}"#
    )]
    pub async fn render_choropleth(
        &self,
        params: Parameters<RenderChoroplethParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let d = &params.0.data;
        let fc = d
            .geojson
            .as_object()
            .ok_or_else(|| invalid("`geojson` must be a GeoJSON object (FeatureCollection)."))?;
        let features = fc
            .get("features")
            .and_then(|f| f.as_array())
            .ok_or_else(|| invalid("`geojson` must contain a `features` array."))?;
        if features.is_empty() {
            return Err(invalid("`geojson` has no features to render."));
        }
        check_limit(features.len(), MAX_MARKERS, "features")?;
        if d.value_property.is_none() && d.values.is_none() {
            return Err(invalid(
                "Provide either `valueProperty` or `values`+`idProperty` to color regions.",
            ));
        }
        let data_json = js_value(d)?;
        render(
            "ui://choropleth/map",
            "choropleth",
            "Choropleth map created for the artifact panel.",
            include_str!("templates/choropleth_template.html"),
            &[Asset::Leaflet],
            &[("{{CHOROPLETH_DATA}}", &data_json)],
        )
    }
}
