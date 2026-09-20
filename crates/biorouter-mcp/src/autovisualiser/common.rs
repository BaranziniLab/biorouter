//! Shared infrastructure for every Auto Visualiser tool.
//!
//! The render pipeline is deliberately uniform: each tool validates its input,
//! turns it into a JSON string, and hands a template + the libraries it needs to
//! [`finish`]. This module centralises the parts that used to be copy-pasted into
//! every tool (asset injection, HTML/JSON escaping, the debug dump, the
//! `CallToolResult` shape) plus the robustness guards (size limits, semantic
//! checks, lenient enum parsing helpers).

use base64::{engine::general_purpose::STANDARD, Engine as _};
use rmcp::model::{CallToolResult, Content, ErrorCode, ErrorData, ResourceContents, Role};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Limits — guard against pathological payloads that would freeze the renderer.
// They are generous (well beyond any sensible figure) and only exist to turn an
// out-of-memory / hung-iframe into a clear, actionable error.
// ---------------------------------------------------------------------------

pub const MAX_NODES: usize = 10_000;
pub const MAX_LINKS: usize = 50_000;
pub const MAX_MATRIX_DIM: usize = 500;
pub const MAX_MARKERS: usize = 100_000;
pub const MAX_VALUES: usize = 500_000;
pub const MAX_TREE_DEPTH: usize = 100;
pub const MAX_MERMAID_LEN: usize = 200_000;
pub const MAX_LABELS: usize = 10_000;

/// Build an `INVALID_PARAMS` error with a helpful, model-actionable message.
pub fn invalid(msg: impl Into<String>) -> ErrorData {
    ErrorData::new(ErrorCode::INVALID_PARAMS, msg.into(), None)
}

// ---------------------------------------------------------------------------
// Input validation
// ---------------------------------------------------------------------------

/// Validates that the `data` field of a serialized params value is a real JSON
/// object/array rather than a stringified blob. Retained as a reusable guard for
/// any future loosely-typed (`serde_json::Value`) tool and exercised by tests.
#[allow(dead_code)]
pub fn validate_data_param(params: &Value, allow_array: bool) -> Result<Value, ErrorData> {
    let data_value = params
        .get("data")
        .ok_or_else(|| invalid("Missing 'data' parameter"))?;

    if data_value.is_string() {
        return Err(invalid(
            "The 'data' parameter must be a JSON object, not a JSON string. \
             Please provide valid JSON without comments.",
        ));
    }

    if allow_array {
        if !data_value.is_object() && !data_value.is_array() {
            return Err(invalid(
                "The 'data' parameter must be a JSON object or array.",
            ));
        }
    } else if !data_value.is_object() {
        return Err(invalid("The 'data' parameter must be a JSON object."));
    }

    Ok(data_value.clone())
}

/// Enforce that a count does not exceed a limit, with a clear message.
pub fn check_limit(count: usize, limit: usize, what: &str) -> Result<(), ErrorData> {
    if count > limit {
        return Err(invalid(format!(
            "Too many {what}: {count} exceeds the maximum of {limit}. \
             Aggregate or sample the data before visualizing."
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Escaping — both data-into-<script> and text-into-HTML are user/LLM influenced.
// ---------------------------------------------------------------------------

/// Serialize a JSON value for safe embedding inside a `<script>` block.
///
/// JSON is valid JS, but a literal `</script>` (or `<!--`) inside a string would
/// terminate the script element and allow markup injection. We neutralise the
/// `<` so the payload can never break out of the script context, and escape the
/// Unicode line separators that are valid in JSON strings but illegal in JS.
pub fn js_data(v: &Value) -> Result<String, ErrorData> {
    let s = serde_json::to_string(v).map_err(|e| invalid(format!("Invalid JSON data: {e}")))?;
    Ok(s.replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029"))
}

/// Serialize an already-typed value to a JS-safe JSON literal.
pub fn js_value<T: serde::Serialize>(v: &T) -> Result<String, ErrorData> {
    let value = serde_json::to_value(v).map_err(|e| invalid(format!("Invalid data: {e}")))?;
    js_data(&value)
}

/// Escape a plain string for safe embedding in HTML text / attribute context.
pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

// ---------------------------------------------------------------------------
// Assets — the libraries each template needs, injected as either inlined
// `<script>`/`<style>` (default: fully offline & self-contained) or pinned CDN
// tags (BIOROUTER_AUTOVIS_CDN=1: tiny persisted blobs, the size fix).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asset {
    D3,
    D3Sankey,
    ChartJs,
    Leaflet,
    Mermaid,
}

impl Asset {
    /// Stable key used to name this asset in a dashboard's shared asset store.
    pub fn key(&self) -> &'static str {
        match self {
            Asset::D3 => "d3",
            Asset::D3Sankey => "d3-sankey",
            Asset::ChartJs => "chartjs",
            Asset::Leaflet => "leaflet",
            Asset::Mermaid => "mermaid",
        }
    }

    /// The raw library sources this asset injects, in load order.
    ///
    /// `(kind, source)` where kind is `"js"` or `"css"`. Only used by the
    /// dashboard, which stores each source once and re-inlines it into every
    /// panel iframe at render time (instead of duplicating megabytes per panel).
    pub fn sources(&self) -> Vec<(&'static str, &'static str)> {
        match self {
            Asset::D3 => vec![("js", D3_MIN)],
            Asset::D3Sankey => vec![("js", D3_SANKEY)],
            Asset::ChartJs => vec![("js", CHART_MIN)],
            Asset::Leaflet => vec![
                ("css", LEAFLET_CSS),
                ("js", LEAFLET_JS),
                ("js", MARKERCLUSTER_JS),
            ],
            Asset::Mermaid => vec![("js", MERMAID_MIN)],
        }
    }
}

// Vendored libraries (compiled into the binary for offline use).
const D3_MIN: &str = include_str!("templates/assets/d3.min.js");
const D3_SANKEY: &str = include_str!("templates/assets/d3.sankey.min.js");
const CHART_MIN: &str = include_str!("templates/assets/chart.min.js");
const LEAFLET_JS: &str = include_str!("templates/assets/leaflet.min.js");
const LEAFLET_CSS: &str = include_str!("templates/assets/leaflet.min.css");
const MARKERCLUSTER_JS: &str = include_str!("templates/assets/leaflet.markercluster.min.js");
const MERMAID_MIN: &str = include_str!("templates/assets/mermaid.min.js");

/// Path (relative to the repository root) of the file holding the copyright
/// lines and full licence texts for everything in `templates/assets/`.
pub const LICENSES_PATH: &str =
    "crates/biorouter-mcp/src/autovisualiser/templates/assets/LICENSES.md";

/// Attribution notice injected into the `<head>` of every generated document.
///
/// The vendored libraries are inlined into each figure, so every figure is a
/// redistribution of them, and MIT / BSD / ISC all require the notice to travel
/// with the copy. Three of the seven files carry a banner of their own inside
/// the minified source and four carry nothing at all, so the notice cannot rely
/// on a comment surviving inside a blob.
///
/// It names the whole bundled set rather than only the libraries a particular
/// figure loaded. That keeps it a single constant that cannot drift per figure,
/// and naming a library the document does not embed costs a reader nothing,
/// whereas omitting one that it does embed is the gap this closes. The full
/// texts stay in [`LICENSES_PATH`] so a 3 MB Mermaid figure does not also carry
/// six licences.
///
/// No `--` may appear inside the text: that sequence is illegal in an HTML
/// comment. `attribution_comment_is_a_well_formed_html_comment` pins it.
pub const ATTRIBUTION_COMMENT: &str = concat!(
    "<!--\n",
    "  Third-party libraries bundled in this figure, each under its own licence:\n",
    "    Chart.js 4.5.0 (MIT), D3 7.9.0 (ISC), d3-sankey 0.12.3 (BSD-3-Clause),\n",
    "    Leaflet 1.9.4 (BSD-2-Clause), Leaflet.markercluster 1.5.3 (MIT),\n",
    "    Mermaid 11.17.2 (MIT).\n",
    "  Copyright lines and the full licence texts ship with the BioRouter source at\n",
    "  crates/biorouter-mcp/src/autovisualiser/templates/assets/LICENSES.md\n",
    "-->\n"
);

// ---------------------------------------------------------------------------
// Pinned CDN URLs (used only when BIOROUTER_AUTOVIS_CDN is enabled).
//
// Every one of these pins an exact `major.minor.patch`, matching the vendored
// file beside it byte for byte, and `vendored_and_cdn_pins_are_the_same_version`
// in `tests.rs` fails on any pin that does not.
//
// A floating pin is not a convenience here, it is a second version of the
// library with no commit behind it. The desktop sets `BIOROUTER_AUTOVIS_CDN=1`
// by default, so a standalone figure loads these URLs while a dashboard always
// inlines the vendored bytes: the same chart is then drawn by two different
// releases inside one app, and `ATTRIBUTION_COMMENT` states a version the CDN
// figure does not contain. That was measurable, not hypothetical, while
// `chart.js@4` floated: it resolved to 4.5.1 against a vendored 4.5.0.
//
// ⚠ Bumping a pin does not break figures already stored in a session. The
// desktop rewriter matches the jsdelivr package and path with the version
// segment left free, so an old tag is recognised and gets today's bytes
// (`ui/desktop/src/utils/artifactCdnAssets.ts`). Before that it matched the
// whole URL as a string, and a bump blanked every figure in every history.
// ---------------------------------------------------------------------------
pub(crate) const CDN_D3: &str = "https://cdn.jsdelivr.net/npm/d3@7.9.0/dist/d3.min.js";
pub(crate) const CDN_D3_SANKEY: &str =
    "https://cdn.jsdelivr.net/npm/d3-sankey@0.12.3/dist/d3-sankey.min.js";
pub(crate) const CDN_CHART: &str =
    "https://cdn.jsdelivr.net/npm/chart.js@4.5.0/dist/chart.umd.min.js";
pub(crate) const CDN_LEAFLET_JS: &str =
    "https://cdn.jsdelivr.net/npm/leaflet@1.9.4/dist/leaflet.js";
pub(crate) const CDN_LEAFLET_CSS: &str =
    "https://cdn.jsdelivr.net/npm/leaflet@1.9.4/dist/leaflet.css";
pub(crate) const CDN_MARKERCLUSTER_JS: &str =
    "https://cdn.jsdelivr.net/npm/leaflet.markercluster@1.5.3/dist/leaflet.markercluster.js";
// Mermaid must be the *classic* (non-module) bundle, not jsdelivr's `/+esm`
// transform. Every desktop artifact is displayed under a `default-src 'none'`
// CSP, so no remote URL is ever fetched by the page itself: the Electron main
// process pre-fetches each URL below and splices the source into the document
// as an inline `<script>` (`ui/desktop/src/utils/artifactCdnAssets.ts`). That
// rewriter only recognises `<script src=…></script>`, and the tag it produces
// is a classic script, so an ESM `import` would neither be rewritten nor run.
// `dist/mermaid.min.js` is an esbuild IIFE ending in `globalThis["mermaid"] =
// …`, which is the shape the vendored offline copy has, and that is what lets
// both modes reach the same runtime state.
//
// The version is pinned exactly, for the reason given above the other pins.
// Mermaid is where the cost was largest: a floating `@11` sat against a
// vendored 10.9.0, so the same diagram really did render two ways.
pub(crate) const CDN_MERMAID: &str =
    "https://cdn.jsdelivr.net/npm/mermaid@11.17.2/dist/mermaid.min.js";

/// A vendored library together with the CDN pin that stands in for it.
///
/// Test-only: it exists so the drift guard has one list to walk, and nothing in
/// the rendering path reads it.
#[cfg(test)]
pub(crate) struct PinnedLibrary {
    /// File name inside `templates/assets/`, which is also its `LICENSES.md` key.
    pub file: &'static str,
    /// The bytes inlined into a figure when CDN mode is off.
    pub vendored: &'static str,
    /// The URL referenced instead when CDN mode is on.
    pub cdn_url: &'static str,
    /// How [`ATTRIBUTION_COMMENT`] names the library.
    pub notice_name: &'static str,
}

/// Every library BioRouter ships two ways, so every pair that can drift apart.
///
/// Adding a library to `Asset::sources` and `asset_html` without adding it here
/// fails `every_shipped_library_has_a_pinned_cdn_row`, which walks the assets
/// directory rather than this table.
#[cfg(test)]
pub(crate) const PINNED_LIBRARIES: &[PinnedLibrary] = &[
    PinnedLibrary {
        file: "chart.min.js",
        vendored: CHART_MIN,
        cdn_url: CDN_CHART,
        notice_name: "Chart.js",
    },
    PinnedLibrary {
        file: "d3.min.js",
        vendored: D3_MIN,
        cdn_url: CDN_D3,
        notice_name: "D3",
    },
    PinnedLibrary {
        file: "d3.sankey.min.js",
        vendored: D3_SANKEY,
        cdn_url: CDN_D3_SANKEY,
        notice_name: "d3-sankey",
    },
    PinnedLibrary {
        file: "leaflet.min.js",
        vendored: LEAFLET_JS,
        cdn_url: CDN_LEAFLET_JS,
        notice_name: "Leaflet",
    },
    PinnedLibrary {
        file: "leaflet.min.css",
        vendored: LEAFLET_CSS,
        cdn_url: CDN_LEAFLET_CSS,
        notice_name: "Leaflet",
    },
    PinnedLibrary {
        file: "leaflet.markercluster.min.js",
        vendored: MARKERCLUSTER_JS,
        cdn_url: CDN_MARKERCLUSTER_JS,
        notice_name: "Leaflet.markercluster",
    },
    PinnedLibrary {
        file: "mermaid.min.js",
        vendored: MERMAID_MIN,
        cdn_url: CDN_MERMAID,
        notice_name: "Mermaid",
    },
];

/// Whether to reference libraries from a pinned CDN instead of inlining them.
///
/// CDN mode shrinks the persisted/transported HTML blob from megabytes (Mermaid
/// alone is ~3 MB) to a few KB, which is the recommended mitigation for the
/// "visualization cannot be generated on reopen" issue with very large diagrams.
/// It requires network access at render time. Inlining (the default) keeps the
/// figure fully self-contained and offline.
///
/// A caller can force inlining regardless of the env flag via
/// [`with_inline_assets`] — that short-circuits here, before the env read, so a
/// figure that must be self-contained (the standalone embedding path, and the
/// dashboard) never emits a CDN `<script src=…>`.
pub fn use_cdn() -> bool {
    if force_inline() {
        return false;
    }
    matches!(
        std::env::var("BIOROUTER_AUTOVIS_CDN").ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes")
    )
}

tokio::task_local! {
    /// Set while assets must be inlined regardless of `BIOROUTER_AUTOVIS_CDN`.
    static FORCE_INLINE: ();
}

/// True while running inside [`with_inline_assets`].
fn force_inline() -> bool {
    FORCE_INLINE.try_with(|_| ()).is_ok()
}

/// Render `fut` with asset inlining forced on (CDN mode disabled for its span),
/// regardless of `BIOROUTER_AUTOVIS_CDN`.
///
/// The `render_standalone_figure` embedding path uses this: its HTML lands in a
/// `srcdoc` iframe the Electron CDN→inline rewriter cannot reach, so a remote
/// `<script src=…>` would be blocked by the renderer CSP and render blank — the
/// same reasoning that makes a dashboard always inline its libraries. Because the
/// override is a task-local checked *before* the env read, it holds even when the
/// desktop app has set `BIOROUTER_AUTOVIS_CDN=1`.
pub async fn with_inline_assets<F, T>(fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    FORCE_INLINE.scope((), fut).await
}

fn script_inline(src: &str) -> String {
    format!("<script>{src}</script>\n")
}

fn script_src(url: &str) -> String {
    format!("<script src=\"{url}\" crossorigin=\"anonymous\"></script>\n")
}

// ---------------------------------------------------------------------------
// Fragment mode — how `render_dashboard` reuses the 32 single-figure tools.
//
// A dashboard is a page of `<iframe srcdoc>` panels, one per figure. Naively it
// would call each tool and embed the returned document, but each document
// inlines its own copy of D3/Chart.js/Mermaid (Mermaid alone is 3.6 MB), so a
// six-panel dashboard would weigh tens of megabytes.
//
// Instead the dashboard runs each tool inside `render_fragment`, which swaps
// `asset_html` for a sentinel comment and records which libraries the tool asked
// for. The dashboard then stores each library's source exactly once and has the
// page's JS splice it back into every panel at render time. Tools are untouched.
// ---------------------------------------------------------------------------

/// Sentinel left in a figure's `<head>` where its libraries would have gone.
pub const ASSET_PLACEHOLDER: &str = "<!--AUTOVIS_ASSETS-->";

tokio::task_local! {
    /// Present only while a figure is being rendered in fragment mode.
    static ASSET_SINK: std::sync::Arc<std::sync::Mutex<Vec<Asset>>>;
}

fn fragment_mode() -> bool {
    ASSET_SINK.try_with(|_| ()).is_ok()
}

/// Render `fut` (a single-figure tool call) in fragment mode.
///
/// Returns the tool's own result plus the de-duplicated, load-ordered list of
/// libraries it requested. The HTML inside the result carries
/// [`ASSET_PLACEHOLDER`] instead of the inlined library sources.
pub async fn render_fragment<F, T>(fut: F) -> (T, Vec<Asset>)
where
    F: std::future::Future<Output = T>,
{
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let out = ASSET_SINK.scope(sink.clone(), fut).await;
    let captured = std::mem::take(&mut *sink.lock().expect("asset sink poisoned"));

    let mut assets = Vec::new();
    for asset in captured {
        if !assets.contains(&asset) {
            assets.push(asset);
        }
    }
    (out, assets)
}

/// Render the `<head>` asset tags (scripts + stylesheets) for the given libraries.
pub fn asset_html(assets: &[Asset]) -> String {
    // Fragment mode: record what was asked for, emit a placeholder instead.
    if ASSET_SINK
        .try_with(|sink| {
            sink.lock()
                .expect("asset sink poisoned")
                .extend_from_slice(assets)
        })
        .is_ok()
    {
        return ASSET_PLACEHOLDER.to_string();
    }

    let cdn = use_cdn();
    let mut out = String::new();
    for a in assets {
        match a {
            Asset::D3 => out.push_str(&if cdn {
                script_src(CDN_D3)
            } else {
                script_inline(D3_MIN)
            }),
            Asset::D3Sankey => out.push_str(&if cdn {
                script_src(CDN_D3_SANKEY)
            } else {
                script_inline(D3_SANKEY)
            }),
            Asset::ChartJs => out.push_str(&if cdn {
                script_src(CDN_CHART)
            } else {
                script_inline(CHART_MIN)
            }),
            Asset::Leaflet => {
                if cdn {
                    out.push_str(&format!(
                        "<link rel=\"stylesheet\" href=\"{CDN_LEAFLET_CSS}\"/>\n"
                    ));
                    out.push_str(&script_src(CDN_LEAFLET_JS));
                    out.push_str(&script_src(CDN_MARKERCLUSTER_JS));
                } else {
                    out.push_str(&format!("<style>{LEAFLET_CSS}</style>\n"));
                    out.push_str(&script_inline(LEAFLET_JS));
                    out.push_str(&script_inline(MARKERCLUSTER_JS));
                }
            }
            Asset::Mermaid => out.push_str(&if cdn {
                script_src(CDN_MERMAID)
            } else {
                script_inline(MERMAID_MIN)
            }),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Template assembly + result construction
// ---------------------------------------------------------------------------

/// Shared client runtime (theme, palette, auto-resize, error card) injected via `{{COMMON}}`.
pub const COMMON_JS: &str = include_str!("templates/_common.js");

/// Put [`ATTRIBUTION_COMMENT`] at the top of a template's `<head>`.
///
/// Every template opens its head with a literal `<head>`, so the notice lands
/// before the library tags. A template without one still gets the notice, at the
/// top of the document. Running twice is a no-op, so a document can never end up
/// with two copies.
///
/// This runs on the template, before any substitution, and the notice carries no
/// `{{KEY}}` of its own, so it cannot swallow or be rewritten by one.
fn with_attribution(template: &str) -> String {
    const HEAD: &str = "<head>";
    debug_assert!(
        ATTRIBUTION_COMMENT.contains(LICENSES_PATH),
        "the notice must name the file holding the full licence texts"
    );
    if template.contains(ATTRIBUTION_COMMENT) {
        return template.to_string();
    }
    let mut out = String::with_capacity(template.len() + ATTRIBUTION_COMMENT.len() + 1);
    match template.split_once(HEAD) {
        Some((before, after)) => {
            out.push_str(before);
            out.push_str(HEAD);
            out.push('\n');
            out.push_str(ATTRIBUTION_COMMENT);
            out.push_str(after);
        }
        None => {
            out.push_str(ATTRIBUTION_COMMENT);
            out.push_str(template);
        }
    }
    out
}

/// Assemble a template: inject the attribution notice and `{{ASSETS}}`,
/// `{{COMMON}}`, then any extra `{{KEY}}` substitutions (which callers must have
/// already escaped appropriately).
pub fn assemble(template: &str, assets: &[Asset], subs: &[(&str, &str)]) -> String {
    let mut html = with_attribution(template)
        .replace("{{ASSETS}}", &asset_html(assets))
        .replace("{{COMMON}}", COMMON_JS);
    for (key, val) in subs {
        html = html.replace(key, val);
    }
    html
}

/// Optionally dump the generated HTML to the cache dir for debugging.
///
/// Only runs when `BIOROUTER_AUTOVIS_DEBUG` is set (or in debug builds), writes to
/// a per-process unique file in the app cache dir (never the world-writable,
/// race-prone, Windows-nonexistent `/tmp`).
pub fn debug_dump(name: &str, html: &str) {
    // Panels rendered inside a dashboard are intermediates, not figures a user
    // ever sees on their own — dumping one file per panel is just noise.
    if fragment_mode() {
        return;
    }
    let enabled = cfg!(debug_assertions)
        || matches!(
            std::env::var("BIOROUTER_AUTOVIS_DEBUG").ok().as_deref(),
            Some("1") | Some("true")
        );
    if !enabled {
        return;
    }
    let Ok(strategy) = etcetera::choose_app_strategy(crate::APP_STRATEGY.clone()) else {
        return;
    };
    use etcetera::AppStrategy;
    let dir = strategy.cache_dir().join("autovisualiser");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let file = dir.join(format!("{}-{}.html", name, std::process::id()));
    match std::fs::write(&file, html) {
        Ok(_) => tracing::info!("Auto Visualiser debug HTML saved to {}", file.display()),
        Err(e) => tracing::warn!("Failed to write Auto Visualiser debug HTML: {e}"),
    }
}

/// Build the standard two-part tool result: a `ui://` HTML resource for the user
/// and an assistant-audience text confirmation (so the model gets a non-empty
/// result and does not loop retrying). The compact structured receipt also serves
/// clients that prefer structured content but do not honor audience annotations.
pub fn finish(uri: &str, debug_name: &str, label: &str, html: String) -> CallToolResult {
    debug_dump(debug_name, &html);
    let blob = STANDARD.encode(html.as_bytes());
    let resource = ResourceContents::BlobResourceContents {
        uri: uri.to_string(),
        mime_type: Some("text/html".to_string()),
        blob,
        meta: None,
    };
    let mut result = CallToolResult::success(vec![
        Content::resource(resource).with_audience(vec![Role::User]),
        Content::text(label.to_string()).with_audience(vec![Role::Assistant]),
    ]);
    result.structured_content = Some(serde_json::json!({
        "status": "created",
        "uri": uri,
        "mimeType": "text/html",
        "summary": label.chars().take(512).collect::<String>(),
    }));
    result
}

/// Recover the HTML document from a `CallToolResult` produced by [`finish`].
///
/// Used by the dashboard to pull each panel's markup back out of the figure tool
/// it just called, so that panels inherit every tool's validation and template
/// verbatim rather than re-implementing them.
pub fn html_from_result(result: &CallToolResult) -> Result<String, ErrorData> {
    for content in result.content.iter() {
        if let rmcp::model::RawContent::Resource(embedded) = &content.raw {
            if let ResourceContents::BlobResourceContents { blob, .. } = &embedded.resource {
                let bytes = STANDARD
                    .decode(blob)
                    .map_err(|e| invalid(format!("figure produced an unreadable blob: {e}")))?;
                return String::from_utf8(bytes)
                    .map_err(|e| invalid(format!("figure produced invalid UTF-8: {e}")));
            }
        }
    }
    Err(invalid("figure produced no HTML resource"))
}

/// Convenience: assemble a template and build the result in one step.
pub fn render(
    uri: &str,
    debug_name: &str,
    label: &str,
    template: &str,
    assets: &[Asset],
    subs: &[(&str, &str)],
) -> Result<CallToolResult, ErrorData> {
    let html = assemble(template, assets, subs);
    Ok(finish(uri, debug_name, label, html))
}

// ---------------------------------------------------------------------------
// Lenient enum parsing — accept any case/whitespace for small closed vocabularies
// so "Line"/"LINE"/" line " all work instead of failing at the rmcp layer
// (a primary cause of "visualization cannot be generated" on live generation).
// ---------------------------------------------------------------------------

/// Deserialize a `data` field that may arrive either as a real JSON object/array
/// **or** as a JSON string containing that object (some models — e.g. Xiaomi
/// MiMo — stringify nested tool-call arguments). Without this, a stringified
/// `data` fails to deserialize into the typed struct and the tool is rejected at
/// the rmcp layer ("interpreted as a string rather than a structured object").
///
/// Use via `#[serde(deserialize_with = "common::de_flexible")]` on `data` fields.
pub fn de_flexible<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    use serde::Deserialize;
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::String(s) => serde_json::from_str(&s).map_err(|e| {
            serde::de::Error::custom(format!(
                "the 'data' argument was a JSON string that could not be parsed \
                 into the expected structure: {e}. Pass it as a JSON object, not a string."
            ))
        }),
        other => serde_json::from_value(other).map_err(serde::de::Error::custom),
    }
}

/// Deserialize a string field leniently against a set of accepted lowercase
/// keywords, returning the canonical form. Use from a manual `Deserialize` impl.
pub fn parse_keyword<E: serde::de::Error>(raw: &str, accepted: &[&str]) -> Result<String, E> {
    let norm = raw.trim().to_lowercase();
    if accepted.contains(&norm.as_str()) {
        Ok(norm)
    } else {
        Err(E::custom(format!(
            "unknown value '{raw}', expected one of: {}",
            accepted.join(", ")
        )))
    }
}
