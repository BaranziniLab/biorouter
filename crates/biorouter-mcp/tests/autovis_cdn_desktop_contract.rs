//! The contract between what the Auto Visualiser emits in CDN mode and what the
//! desktop app is able to turn back into a runnable figure.
//!
//! Own test binary because `use_cdn()` reads a process-wide environment
//! variable, which would race every other figure test running in parallel.
//!
//! ## The regression this exists for
//!
//! Every artifact is displayed under `ARTIFACT_BROWSER_CSP`
//! (`default-src 'none'`), so a figure can never fetch anything itself. CDN mode
//! is nevertheless the desktop *default* (`BIOROUTER_AUTOVIS_CDN=1`, set in
//! `ui/desktop/src/biorouterd.ts`) because it is the stored blob that matters —
//! a few KB instead of megabytes. What makes that work is the Electron main
//! process pre-fetching each CDN URL and splicing the source in as an inline
//! `<script>` before the CSP is applied
//! (`ui/desktop/src/utils/artifactCdnAssets.ts`).
//!
//! Mermaid shipped outside that mechanism twice over: its URL was absent from
//! the desktop asset list, and it was emitted as
//! `<script type="module">import mermaid from '…/+esm'</script>`, a shape the
//! rewriter's `<script src=…>` pattern can never match. Every Rust test passed;
//! every standalone Mermaid figure in the packaged app said "Mermaid library
//! failed to load."
//!
//! So this asserts both halves, for every asset, against the real desktop
//! source: the URL is in the list, and the tag matches the pattern that list is
//! consumed with.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use biorouter_mcp::autovisualiser::{
    AutoVisualiserRouter, RenderMapParams, RenderMermaidParams, RenderNetworkParams,
    RenderSankeyParams, ShowChartParams,
};
use regex::Regex;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, RawContent, ResourceContents};
use serde_json::json;
use std::path::{Path, PathBuf};

/// `ui/desktop/src/utils/artifactCdnAssets.ts`, the desktop half of the contract.
const DESKTOP_ASSET_MODULE: &str = "ui/desktop/src/utils/artifactCdnAssets.ts";

/// The CDN-mode Mermaid document the browser-level regression test replays.
const BROWSER_FIXTURE: &str = "ui/desktop/src/utils/__fixtures__/autovis-mermaid-cdn.html";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn read_repo_file(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn html_of(result: &CallToolResult) -> String {
    if let RawContent::Resource(resource) = &*result.content[0] {
        if let ResourceContents::BlobResourceContents { blob, .. } = &resource.resource {
            return String::from_utf8(STANDARD.decode(blob).unwrap()).unwrap();
        }
    }
    panic!("expected a blob resource");
}

/// The URL list the desktop rewriter iterates, read from its own source.
fn desktop_asset_urls(module: &str) -> Vec<String> {
    let body = module
        .split_once("export const ARTIFACT_CDN_ASSETS = [")
        .expect("ARTIFACT_CDN_ASSETS declaration")
        .1
        .split_once("\n];")
        .expect("end of ARTIFACT_CDN_ASSETS")
        .0;
    Regex::new(r"'(https://[^']+)'")
        .unwrap()
        .captures_iter(body)
        .map(|c| c[1].to_string())
        .collect()
}

/// Rebuild one of the rewriter's regexes from the desktop source, so a change to
/// the tag shape it accepts is a change to what this test demands.
///
/// The TS side is a template literal, `` new RegExp(`…${<url expression>}…`, 'g') ``.
/// The interpolation is replaced with the URL, escaped, and the literal's
/// doubled backslashes are unescaped.
///
/// ⚠ The renderer's own interpolation leaves the version segment of a jsdelivr
/// URL free, so a figure stored under an older pin is still recognised. This
/// substitutes the whole URL instead, which is deliberate: every document here
/// was rendered moments ago with today's pin, so the exact URL is the one to
/// demand, and it keeps the question this test asks about the *tag shape* the
/// tools emit. Version tolerance is a property of the stored blob and is tested
/// where it lives, in `artifactCdnAssets.test.ts`.
fn rewriter_pattern(module: &str, function: &str, url: &str) -> Regex {
    let template = module
        .split_once(&format!("export const {function} = "))
        .unwrap_or_else(|| panic!("{function} declaration"))
        .1
        .split_once("new RegExp(`")
        .expect("RegExp template")
        .1
        .split_once('`')
        .expect("end of RegExp template")
        .0;
    let interpolation = Regex::new(r"\$\{[^}]*\}").unwrap();
    assert!(
        interpolation.is_match(template),
        "{function} no longer interpolates the URL into its pattern, so this \
         test would be asserting against a fixed string: {template}"
    );
    // `$` is a capture reference in a replacement string, so double it.
    let source = interpolation
        .replace_all(template, regex::escape(url).replace('$', "$$"))
        .replace("\\\\", "\\");
    Regex::new(&source).unwrap_or_else(|e| panic!("{function} is not a valid pattern: {e}"))
}

/// Every distinct CDN reference in a rendered figure.
fn cdn_urls_in(html: &str) -> Vec<String> {
    let mut urls: Vec<String> = Regex::new(r#"https://cdn\.jsdelivr\.net/[^"'\s>]+"#)
        .unwrap()
        .find_iter(html)
        .map(|m| m.as_str().to_string())
        .collect();
    urls.sort();
    urls.dedup();
    urls
}

/// Assert a figure's CDN references are all ones the desktop app can inline, in
/// a shape it can actually rewrite.
fn assert_reachable_from_the_desktop(figure: &str, html: &str, module: &str) {
    let known = desktop_asset_urls(module);
    let urls = cdn_urls_in(html);
    assert!(
        !urls.is_empty(),
        "{figure} referenced no CDN asset in CDN mode — the test is not exercising what it claims"
    );

    for url in urls {
        assert!(
            known.contains(&url),
            "{figure} references {url}, which is missing from ARTIFACT_CDN_ASSETS in \
             {DESKTOP_ASSET_MODULE}. The desktop app will not inline it, the artifact CSP will \
             block it, and the figure will fail in the packaged app while every Rust test passes."
        );

        let is_stylesheet = url.ends_with(".css");
        let function = if is_stylesheet {
            "artifactCdnStylePattern"
        } else {
            "artifactCdnScriptPattern"
        };
        assert!(
            rewriter_pattern(module, function, &url).is_match(html),
            "{figure} references {url} in a shape {function} cannot rewrite. Only \
             `<script src=\"URL\"></script>` and `<link href=\"URL\">` are understood — an ESM \
             `import` inside a module script is never matched, and never runs under the CSP."
        );
    }
}

#[tokio::test]
async fn cdn_figures_are_reachable_from_the_desktop_rewriter() {
    std::env::set_var("BIOROUTER_AUTOVIS_CDN", "1");
    let router = AutoVisualiserRouter::new();
    let module = read_repo_file(DESKTOP_ASSET_MODULE);

    // One figure per library, so every asset arm of `asset_html` is exercised.
    let mermaid = html_of(
        &router
            .render_mermaid(Parameters(
                serde_json::from_value::<RenderMermaidParams>(
                    json!({"mermaid_code": "graph TD; A-->B; B-->C;"}),
                )
                .unwrap(),
            ))
            .await
            .unwrap(),
    );
    let chart = html_of(
        &router
            .show_chart(Parameters(
                serde_json::from_value::<ShowChartParams>(json!({"data": {
                    "type": "bar", "labels": ["A"], "datasets": [{"label": "S", "data": [1.0]}]
                }}))
                .unwrap(),
            ))
            .await
            .unwrap(),
    );
    let network = html_of(
        &router
            .render_network(Parameters(
                serde_json::from_value::<RenderNetworkParams>(json!({"data": {
                    "nodes": [{"id": "a"}, {"id": "b"}],
                    "links": [{"source": "a", "target": "b"}]
                }}))
                .unwrap(),
            ))
            .await
            .unwrap(),
    );
    let sankey = html_of(
        &router
            .render_sankey(Parameters(
                serde_json::from_value::<RenderSankeyParams>(json!({"data": {
                    "nodes": [{"name": "A"}, {"name": "B"}],
                    "links": [{"source": "A", "target": "B", "value": 10.0}]
                }}))
                .unwrap(),
            ))
            .await
            .unwrap(),
    );
    let map = html_of(
        &router
            .render_map(Parameters(
                serde_json::from_value::<RenderMapParams>(json!({"data": {
                    "markers": [{"lat": 37.76, "lng": -122.45, "title": "UCSF"}]
                }}))
                .unwrap(),
            ))
            .await
            .unwrap(),
    );
    std::env::remove_var("BIOROUTER_AUTOVIS_CDN");

    assert_reachable_from_the_desktop("render_mermaid", &mermaid, &module);
    assert_reachable_from_the_desktop("show_chart", &chart, &module);
    assert_reachable_from_the_desktop("render_network", &network, &module);
    assert_reachable_from_the_desktop("render_sankey", &sankey, &module);
    assert_reachable_from_the_desktop("render_map", &map, &module);

    // Together these five cover every `Asset` variant. If one is ever dropped
    // from the list above, this notices before the coverage claim goes stale.
    let mut covered = cdn_urls_in(&mermaid);
    for html in [&chart, &network, &sankey, &map] {
        covered.extend(cdn_urls_in(html));
    }
    covered.sort();
    covered.dedup();
    let mut known = desktop_asset_urls(&module);
    known.sort();
    assert_eq!(
        covered, known,
        "ARTIFACT_CDN_ASSETS and the URLs the Auto Visualiser actually emits have diverged. \
         An entry the tools never emit is dead weight; one they do emit is a broken figure."
    );

    // The browser-level replay in `artifactCdnAssets.browser.test.ts` runs
    // against a real CDN-mode document rather than a hand-written one. Keep it
    // honest: regenerate with UPDATE_AUTOVIS_FIXTURES=1.
    let fixture_path = repo_root().join(BROWSER_FIXTURE);
    if std::env::var("UPDATE_AUTOVIS_FIXTURES").is_ok() {
        std::fs::create_dir_all(fixture_path.parent().unwrap()).unwrap();
        std::fs::write(&fixture_path, &mermaid).unwrap();
    } else {
        assert_eq!(
            read_repo_file(BROWSER_FIXTURE),
            mermaid,
            "{BROWSER_FIXTURE} is stale. Regenerate it with \
             `UPDATE_AUTOVIS_FIXTURES=1 cargo test -p biorouter-mcp \
             --test autovis_cdn_desktop_contract`."
        );
    }
}
