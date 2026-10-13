use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use super::bundle::{LintFinding, LintLevel};
use super::render::{is_handler_name, scan_tags, script_is_executable, Tag, TagAttr};

pub(super) const PASSED: &str = "Browser-free page check passed: entry, local scripts/styles and JavaScript structure checked. Runtime behavior was not verified.";
const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;

fn error(msg: impl Into<String>) -> LintFinding {
    LintFinding {
        level: LintLevel::Error,
        msg: msg.into(),
    }
}

fn read_source(root: &Path, path: &Path) -> io::Result<String> {
    let canonical = path.canonicalize()?;
    if !canonical.starts_with(root.canonicalize()?) {
        return Err(io::Error::other("the file is outside the app directory"));
    }
    if !canonical.metadata()?.is_file() {
        return Err(io::Error::other("the path is not a regular source file"));
    }
    let mut source = String::new();
    std::fs::File::open(canonical)?
        .take(MAX_SOURCE_BYTES + 1)
        .read_to_string(&mut source)?;
    if source.len() as u64 > MAX_SOURCE_BYTES {
        return Err(io::Error::other(
            "the source exceeds the 16 MiB check limit",
        ));
    }
    Ok(source)
}

fn attr<'a>(attrs: &'a [TagAttr], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|a| !a.duplicate && a.key == key)
        .map(|a| a.value.as_str())
}

/// Resolve against the same app-root base the served page receives. URLs are
/// parsed, never fetched; canonicalization below also catches encoded traversal.
fn local_asset(root: &Path, value: &str) -> Result<Option<PathBuf>, String> {
    let base = url::Url::parse("https://biorouter.invalid/apps/check/").expect("static URL");
    let value = value.trim();
    let resolved = base.join(value).map_err(|e| e.to_string())?;
    if resolved.origin() != base.origin() {
        return Ok(None);
    }
    let relative = resolved.path().strip_prefix(base.path()).ok_or_else(|| {
        "the URL leaves the app's serving directory; use an app-relative path".to_string()
    })?;
    let file_base = url::Url::from_directory_path(root)
        .map_err(|()| "the app directory is not an absolute path".to_string())?;
    let decoded = file_base
        .join(relative)
        .map_err(|e| e.to_string())?
        .to_file_path()
        .map_err(|()| "the URL is not a local file path".to_string())?;
    let relative = decoded.strip_prefix(root).map_err(|e| e.to_string())?;
    if !relative.starts_with("assets") && !relative.starts_with("dist") {
        return Err("the app serves local scripts and styles only from assets/ or dist/; move the file there and update the URL".into());
    }
    Ok(Some(decoded))
}

pub(super) fn inputs_fingerprint(root: &Path, entry: &str) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    if let Ok(html) = read_source(root, &root.join(entry)) {
        html.hash(&mut hash);
        scan_tags(&html, |tag| {
            if let Tag::Start { name, attrs, .. } = tag {
                let value = match name.as_str() {
                    "script" => attr(&attrs, "src"),
                    "link" => attr(&attrs, "href"),
                    _ => None,
                };
                if let Some(Ok(Some(path))) = value.map(|v| local_asset(root, v)) {
                    path.hash(&mut hash);
                    let metadata = path
                        .canonicalize()
                        .ok()
                        .filter(|p| root.canonicalize().is_ok_and(|r| p.starts_with(r)))
                        .and_then(|p| p.metadata().ok());
                    metadata
                        .as_ref()
                        .map(std::fs::Metadata::len)
                        .hash(&mut hash);
                    metadata.and_then(|m| m.modified().ok()).hash(&mut hash);
                }
            }
        });
    }
    hash.finish()
}

pub(super) fn javascript_error(source: &str, label: &str, module: bool) -> Option<LintFinding> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_javascript::LANGUAGE.into())
        .expect("the compiled JavaScript grammar is valid");
    let Some(tree) = parser.parse(source, None) else {
        return Some(error(format!(
            "{label}: JavaScript could not be parsed. Fix the script and rebuild."
        )));
    };
    let mut nodes = vec![tree.root_node()];
    while let Some(node) = nodes.pop() {
        let invalid = node.is_error()
            || node.is_missing()
            || node.kind().starts_with("jsx_")
            || (!module && matches!(node.kind(), "import_statement" | "export_statement"));
        if invalid {
            let at = node.start_position();
            return Some(error(format!(
                "{label}: invalid browser JavaScript near line {}, column {}. Fix the syntax (and use type=\"module\" for imports/exports), then rebuild.",
                at.row + 1,
                at.column + 1
            )));
        }
        let mut cursor = node.walk();
        nodes.extend(node.children(&mut cursor));
    }
    None
}

pub(super) fn findings(root: &Path, entry: &str, building: bool) -> Vec<LintFinding> {
    findings_with(root, entry, building, None, false)
}

pub(super) fn staged_findings(
    root: &Path,
    entry: &str,
    staging: &Path,
    replaces_css: bool,
) -> Vec<LintFinding> {
    findings_with(root, entry, false, Some(staging), replaces_css)
}

fn authored_asset<'a>(
    name: &str,
    attrs: &'a [TagAttr],
    script_text: Option<&str>,
    entry: &str,
    script_number: &mut usize,
    out: &mut Vec<LintFinding>,
) -> Option<(&'a str, bool, bool)> {
    for handler in attrs
        .iter()
        .filter(|a| !a.duplicate && is_handler_name(&a.key))
    {
        let source = format!("function __handler(event){{\n{}\n}}", handler.value);
        if let Some(finding) =
            javascript_error(&source, &format!("{entry} {} handler", handler.key), false)
        {
            out.push(finding);
        }
    }
    if name == "script" {
        let kind = attr(attrs, "type");
        if !script_is_executable(kind, attr(attrs, "language")) {
            return None;
        }
        if kind.is_some_and(|t| t.trim().eq_ignore_ascii_case("importmap")) {
            if let Err(e) =
                serde_json::from_str::<serde_json::Value>(script_text.unwrap_or_default())
            {
                out.push(error(format!(
                    "{entry} importmap: invalid JSON ({e}). Fix the import map before launch."
                )));
            }
            return None;
        }
        let module = kind.is_some_and(|t| t.trim().eq_ignore_ascii_case("module"));
        if let Some(src) = attr(attrs, "src") {
            return Some((src, true, module));
        }
        *script_number += 1;
        if let Some(finding) = javascript_error(
            script_text.unwrap_or_default(),
            &format!("{entry} inline script {script_number}"),
            module,
        ) {
            out.push(finding);
        }
    } else if name == "link"
        && attr(attrs, "rel").is_some_and(|rel| {
            rel.split_ascii_whitespace()
                .any(|word| word.eq_ignore_ascii_case("stylesheet"))
        })
    {
        return attr(attrs, "href").map(|href| (href, false, false));
    }
    None
}

fn findings_with(
    root: &Path,
    entry: &str,
    building: bool,
    staging: Option<&Path>,
    replaces_css: bool,
) -> Vec<LintFinding> {
    let mut out = Vec::new();
    let html = match read_source(root, &root.join(entry)) {
        Ok(html) if !html.trim().is_empty() => html,
        Ok(_) => return vec![error(format!("{entry}: the entry page is empty. Add its HTML before building or launching."))],
        Err(e) => return vec![error(format!("{entry}: the entry page could not be read ({e}). Restore the file or fix the manifest entry."))],
    };
    let mut checked = HashSet::new();
    let mut bundle_loaded = false;
    let mut template_depth = 0usize;
    let mut script_number = 0usize;
    scan_tags(&html, |tag| {
        let Tag::Start {
            name,
            attrs,
            script_text,
        } = tag
        else {
            if let Tag::End { name } = tag {
                if name == "template" {
                    template_depth = template_depth.saturating_sub(1);
                }
            }
            return;
        };
        if name == "template" {
            template_depth += 1;
        }
        if template_depth > 0 {
            return;
        }
        let asset = authored_asset(
            &name,
            &attrs,
            script_text,
            entry,
            &mut script_number,
            &mut out,
        );
        if let Some((url, script, module)) = asset {
            match local_asset(root, url) {
                Ok(Some(path)) => {
                    let generated =
                        path == root.join("dist/app.js") || path == root.join("dist/app.css");
                    if building && generated && root.join("src/main.ts").is_file() {
                        return;
                    }
                    if script && path == root.join("dist/app.js") {
                        bundle_loaded = true;
                    }
                    if !checked.insert((path.clone(), script, module)) {
                        return;
                    }
                    let replaced = path == root.join("dist/app.js")
                        || (replaces_css && path == root.join("dist/app.css"));
                    let path = staging.filter(|_| replaced).map_or(path.clone(), |dir| {
                        dir.join(path.file_name().expect("generated asset has a name"))
                    });
                    match read_source(root, &path) {
                        Ok(source) if script => {
                            if let Some(finding) = javascript_error(&source, &format!("{entry} script '{url}'"), module) {
                                out.push(finding);
                            }
                        }
                        Ok(_) => {}
                        Err(e) => out.push(error(format!("{entry}: local {} '{url}' could not be read ({e}). Add the file or correct its path.", if script { "script" } else { "stylesheet" }))),
                    }
                }
                Ok(None) => {}
                Err(e) => out.push(error(format!(
                    "{entry}: asset '{url}' is invalid ({e}). Correct the URL."
                ))),
            }
        }
    });
    let bundle = staging.map_or_else(|| root.join("dist/app.js"), |dir| dir.join("app.js"));
    if !building && bundle.is_file() && !bundle_loaded {
        match read_source(root, &bundle) {
            Ok(source) => {
                if let Some(finding) = javascript_error(&source, "dist/app.js", false) {
                    out.push(finding);
                }
            }
            Err(e) => out.push(error(format!(
                "dist/app.js could not be checked ({e}). Rebuild the bundle."
            ))),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(html: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), html).unwrap();
        dir
    }

    #[test]
    fn broken_scripts_handlers_and_local_assets_are_actionable() {
        let dir = page(
            r#"<script>const value = ;</script><button onclick="const x = ;">Go</button><script src="assets/missing.js?v=1#x"></script><link rel="stylesheet" href="assets/missing.css">"#,
        );
        let errors = findings(dir.path(), "index.html", false);
        assert_eq!(errors.len(), 4);
        assert!(errors.iter().all(|f| f.msg.contains("index.html")));
        assert!(errors.iter().any(|f| f.msg.contains("missing.js?v=1#x")));
        assert!(errors.iter().any(|f| f.msg.contains("missing.css")));
    }

    #[test]
    fn valid_modules_data_templates_and_query_urls_are_not_runtime_tests() {
        let dir = page(
            r#"<script type="module">import {x} from './assets/helper.js';</script><script type="application/json">not JavaScript</script><script type="importmap">{"imports":{}}</script><template><script>const broken = ;</script></template><textarea><script>const broken = ;</script></textarea><button onclick="return false">Go</button><script src="assets/helper.js?v=2#hash"></script><script src="https://example.test/no-fetch.js"></script><link rel="stylesheet" href="assets/styles.css?x=1">"#,
        );
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        std::fs::write(dir.path().join("assets/helper.js"), "const x = 1;").unwrap();
        std::fs::write(dir.path().join("assets/styles.css"), "body { color: red; }").unwrap();
        assert!(findings(dir.path(), "index.html", false).is_empty());
        assert!(PASSED.contains("Runtime behavior was not verified"));
    }

    #[test]
    fn nested_entries_use_the_served_app_root_and_encoded_paths() {
        let dir = page("placeholder");
        std::fs::create_dir(dir.path().join("pages")).unwrap();
        std::fs::write(
            dir.path().join("pages/home.html"),
            r#"<script src="assets/helper%20file.js?cache=1"></script>"#,
        )
        .unwrap();
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        std::fs::write(dir.path().join("assets/helper file.js"), "const x = 1;").unwrap();
        assert!(findings(dir.path(), "pages/home.html", false).is_empty());
        std::fs::write(
            dir.path().join("pages/home.html"),
            r#"<script src="../outside.js"></script>"#,
        )
        .unwrap();
        assert!(findings(dir.path(), "pages/home.html", false)[0]
            .msg
            .contains("leaves the app"));
    }

    #[test]
    fn empty_entries_importmaps_and_corrupted_existing_bundles_fail() {
        let dir = page("");
        assert!(findings(dir.path(), "index.html", false)[0]
            .msg
            .contains("empty"));
        std::fs::write(
            dir.path().join("index.html"),
            "<script type=importmap>{bad}</script>",
        )
        .unwrap();
        assert!(findings(dir.path(), "index.html", false)[0]
            .msg
            .contains("importmap"));
        std::fs::write(dir.path().join("index.html"), "<p>Valid page</p>").unwrap();
        std::fs::create_dir(dir.path().join("dist")).unwrap();
        std::fs::write(dir.path().join("dist/app.js"), "(()=>{ const x = ;").unwrap();
        assert!(findings(dir.path(), "index.html", false)[0]
            .msg
            .contains("dist/app.js"));
        assert!(javascript_error("<div />", "inline", false).is_some());
        assert!(javascript_error("import x from './x.js';", "inline", false).is_some());
    }

    #[test]
    fn less_common_handlers_share_the_renderers_predicate() {
        let dir = page(
            r#"<div onwheel="const broken = ;" onscroll="const broken = ;" data-onclick="not JS"></div>"#,
        );
        let errors = findings(dir.path(), "index.html", false);
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().any(|f| f.msg.contains("onwheel handler")));
        assert!(errors.iter().any(|f| f.msg.contains("onscroll handler")));
    }

    #[test]
    fn source_paths_cannot_read_directories_or_escape_the_app() {
        let dir = page(r#"<script src="assets/"></script>"#);
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        assert!(findings(dir.path(), "index.html", false)[0]
            .msg
            .contains("not a regular source file"));
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("external.js"), "const x = 1;").unwrap();
        assert!(read_source(dir.path(), &outside.path().join("external.js"))
            .unwrap_err()
            .to_string()
            .contains("outside the app"));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                outside.path().join("external.js"),
                dir.path().join("assets/escape.js"),
            )
            .unwrap();
            std::fs::write(
                dir.path().join("index.html"),
                r#"<script src="assets/escape.js"></script>"#,
            )
            .unwrap();
            assert!(findings(dir.path(), "index.html", false)[0]
                .msg
                .contains("outside the app"));
        }
    }

    #[test]
    fn each_script_loading_context_is_checked_even_after_a_stylesheet() {
        let dir = page(
            r#"<script type="module" src="assets/shared.js"></script><script src="assets/shared.js"></script>"#,
        );
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        std::fs::write(dir.path().join("assets/shared.js"), "export const x = 1;").unwrap();
        assert_eq!(findings(dir.path(), "index.html", false).len(), 1);
        std::fs::write(dir.path().join("index.html"), r#"<link rel="stylesheet" href="assets/shared.js"><script src="assets/shared.js"></script>"#).unwrap();
        assert_eq!(findings(dir.path(), "index.html", false).len(), 1);
        std::fs::write(
            dir.path().join("index.html"),
            r#"<script type="module" src="assets/shared.js"></script>"#,
        )
        .unwrap();
        assert!(findings(dir.path(), "index.html", false).is_empty());
    }

    #[test]
    fn local_files_must_have_a_served_asset_path() {
        let dir =
            page(r#"<script src="helper.js"></script><link rel="stylesheet" href="style.css">"#);
        std::fs::write(dir.path().join("helper.js"), "const x = 1;").unwrap();
        std::fs::write(dir.path().join("style.css"), "body{}").unwrap();
        let errors = findings(dir.path(), "index.html", false);
        assert_eq!(errors.len(), 2);
        assert!(errors
            .iter()
            .all(|f| f.msg.contains("only from assets/ or dist/")));
    }
}
