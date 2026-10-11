//! Rendering + scaffolding for Agent Drafter apps.
//!
//! - [`assemble_app`] produces the HTML `biorouterd` serves at `/apps/<id>/`:
//!   the author's `index.html` with the Biorouter design system injected, an
//!   app-config script, and the esbuild bundle (`dist/app.js`). The bundle's SDK
//!   opens a WebSocket to the per-app agent backend and streams real answers.
//! - [`assemble_card`] produces a lightweight static preview shown inline in
//!   chat (apps are *used* in the browser, not in a sandboxed iframe).
//! - [`launcher_files`] and [`exported_manifest_json`] are the files an export
//!   generates around the app's own (launchers, dev server, README); the
//!   export itself is `export.rs`.

use crate::agent_drafter::store::{ArtifactKind, Manifest};

pub const THEME_CSS: &str = include_str!("templates/theme.css");
pub const STARTER_HTML: &str = include_str!("templates/app-index.html");
/// The starter a STATIC app gets: no `data-br-chat` card and no agent notes,
/// because a static app has no agent to fill them.
pub const STATIC_STARTER_HTML: &str = include_str!("templates/static-index.html");

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Build the default entry HTML for a freshly created agentic app.
pub fn starter(title: &str, description: &str) -> String {
    STARTER_HTML
        .replace("{{TITLE}}", &html_escape(title))
        .replace("{{DESCRIPTION}}", &html_escape(description))
}

/// Build the default entry HTML for a freshly created static app.
pub fn static_starter(title: &str, description: &str) -> String {
    STATIC_STARTER_HTML
        .replace("{{TITLE}}", &html_escape(title))
        .replace("{{DESCRIPTION}}", &html_escape(description))
}

/// Byte offset of the first ASCII-case-insensitive match of `needle` in `html`.
///
/// ⚠ Searched on the ORIGINAL bytes. This used to search `html.to_lowercase()`
/// and split the original at the offset it found there, but lowercasing changes
/// the UTF-8 length of some characters (U+2126 OHM SIGN 3 → 2 bytes, U+0130 'İ'
/// 2 → 3), so a page with "10 kΩ" before `</body>` had its config island spliced
/// into the middle of a tag, and an offset that landed inside a character
/// panicked the request. Every needle here is ASCII, and an ASCII byte never
/// occurs inside a multi-byte character, so every match is a char boundary.
fn find_ascii_ci(html: &str, needle: &str) -> Option<usize> {
    debug_assert!(needle.is_ascii(), "needles are ASCII markup");
    let (hay, needle) = (html.as_bytes(), needle.as_bytes());
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
}

/// Insert `insert` immediately before the first (case-insensitive) `needle`.
/// If `needle` is absent, fall back to prepend/append.
fn inject_before(html: &str, needle: &str, insert: &str, append_if_missing: bool) -> String {
    if let Some(pos) = find_ascii_ci(html, needle) {
        let (before, after) = html.split_at(pos);
        let mut out = String::with_capacity(html.len() + insert.len());
        out.push_str(before);
        out.push_str(insert);
        out.push_str(after);
        out
    } else if append_if_missing {
        format!("{html}{insert}")
    } else {
        format!("{insert}{html}")
    }
}

/// Insert immediately *after* the first occurrence of `needle`.
fn inject_after(html: &str, needle: &str, insert: &str) -> String {
    if let Some(pos) = find_ascii_ci(html, needle) {
        let at = pos + needle.len();
        let (before, after) = html.split_at(at);
        format!("{before}{insert}{after}")
    } else {
        format!("{insert}{html}")
    }
}

const THEME_TAG: &str = "<style id=\"biorouter-theme\">";
const THEME_OVERRIDES_TAG: &str = "<style id=\"biorouter-theme-overrides\">";
/// The opening of the config island [`app_config_script`] writes.
const CONFIG_ISLAND_TAG: &str = "<script type=\"application/json\" id=\"biorouter-app-config\">";
/// The bundle tag the assembler appends.
const BUNDLE_TAG: &str = "<script src=\"dist/app.js\"></script>";

/// Remove every `open … close` block (and the newline the assembler writes
/// after it) from `html`.
fn remove_blocks(html: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some((before, after_open)) = rest.split_once(open) {
        let Some((_, after_close)) = after_open.split_once(close) else {
            break;
        };
        out.push_str(before);
        rest = after_close.strip_prefix('\n').unwrap_or(after_close);
    }
    out.push_str(rest);
    out
}

/// Undo an earlier assembly, so assembling is idempotent.
///
/// ⚠ This is what heals exported apps. An export used to ship an already
/// assembled `index.html`, every launcher installed it as the app's entry, and
/// the daemon assembled it again: two config islands, and the SDK reads the
/// FIRST — the export's, which has no socket token — so the agent socket was
/// refused; the bundle ran twice; and each re-export nested one more layer. It
/// removes exactly what the assembler emits and nothing an author writes: the
/// theme and overrides `<style>` blocks, every config island, the bundle tag in
/// the assembler's own spelling, an injected `<base href="/apps/<id>/">`, and
/// `data-br-pack` on `<html>`.
fn strip_injected(html: &str) -> String {
    let mut out = remove_blocks(html, THEME_TAG, "</style>");
    out = remove_blocks(&out, THEME_OVERRIDES_TAG, "</style>");
    out = remove_blocks(&out, CONFIG_ISLAND_TAG, "</script>");
    out = remove_blocks(&out, BUNDLE_TAG, "");
    static BASE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"<base href="/apps/[A-Za-z0-9._-]+/">\n?"#).expect("static regex")
    });
    out = BASE.replace_all(&out, "").into_owned();
    if let Some(start) = find_ascii_ci(&out, "<html") {
        let (before, from_tag) = out.split_at(start);
        if let Some((tag, after)) = from_tag.split_once('>') {
            static PACK: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
                regex::Regex::new(r#" data-br-pack="[^"]*""#).expect("static regex")
            });
            out = format!("{before}{}>{after}", PACK.replace_all(tag, ""));
        }
    }
    out
}

fn theme_block() -> String {
    format!("{THEME_TAG}\n{THEME_CSS}\n</style>\n")
}

/// An extra `<style>` layer carrying the app's sanitized accent + custom token
/// overrides. Emitted only when the app declares overrides, so a v1/base app's
/// output is byte-identical to before. Scoped to `:root[data-br-pack]` (an
/// attribute [`set_root_pack`] always writes when overrides exist), so these
/// declarations win over both the base `:root` tokens and any pack layer.
fn theme_overrides_block(manifest: &Manifest) -> Option<String> {
    let accent = manifest.theme.sanitized_accent();
    let tokens = manifest.theme.sanitized_tokens();
    if accent.is_none() && tokens.is_empty() {
        return None;
    }
    let mut decls = String::new();
    if let Some(a) = accent {
        decls.push_str(&format!("--br-accent: {a};"));
    }
    for (k, v) in &tokens {
        decls.push_str(&format!(" {k}: {v};"));
    }
    Some(format!(
        "{THEME_OVERRIDES_TAG}\n:root[data-br-pack] {{ {decls} }}\n</style>\n"
    ))
}

/// The full injected head CSS: the base design system plus, when the app
/// customises its theme, an overrides layer.
fn theme_head(manifest: &Manifest) -> String {
    let mut s = theme_block();
    if let Some(ov) = theme_overrides_block(manifest) {
        s.push_str(&ov);
    }
    s
}

/// The `data-br-pack` value to stamp on `<html>`, or `None` when the base look
/// with no overrides is in effect (so nothing is written and v1 output is
/// unchanged). Present for a non-base pack, or whenever overrides exist (the
/// overrides style keys off the `[data-br-pack]` attribute).
fn pack_attr_value(manifest: &Manifest) -> Option<&str> {
    let pack = manifest.theme.resolved_pack();
    if pack == crate::agent_drafter::manifest::DEFAULT_THEME_PACK && !manifest.theme.has_overrides()
    {
        None
    } else {
        Some(pack)
    }
}

/// Stamp `data-br-pack="<pack>"` onto the document's `<html>` element. Robust to
/// `<html>` and `<html lang="en">`; if the document has no `<html>` tag (a bare
/// fragment) the string is returned unchanged — the base tokens still apply.
fn set_root_pack(html: &str, pack: &str) -> String {
    if let Some(pos) = find_ascii_ci(html, "<html") {
        let at = pos + "<html".len();
        let (before, after) = html.split_at(at);
        format!("{before} data-br-pack=\"{}\"{after}", html_escape(pack))
    } else {
        html.to_string()
    }
}

/// The script that hands the app SDK its configuration.
///
/// `endpoint` pins a single agent socket (a remote daemon); leave it `None` so
/// the SDK derives one from the page's own origin, which is what makes an app
/// work no matter which port its daemon landed on. `endpoints` are additional
/// fallbacks tried in order — exports use them to cover a `file://` open.
///
/// `ws_token` is the per-app socket token the SDK must present on the agent
/// WebSocket (`?token=…`) — `serve_index` mints one per daemon run and injects
/// it here so any page this daemon serves can authenticate. A page assembled
/// with `None` cannot open the agent socket at all; it is only for previews and
/// offline views, which is why the daemon always re-assembles the stored entry
/// (and [`strip_injected`] drops any island assembled earlier).
pub fn app_config_script(
    manifest: &Manifest,
    endpoint: Option<&str>,
    endpoints: &[String],
    ws_token: Option<&str>,
) -> String {
    let greeting = manifest.agent.as_ref().and_then(|a| a.greeting.clone());
    let mut cfg = serde_json::json!({
        "appId": manifest.id,
        "endpoint": endpoint,
        "greeting": greeting,
    });
    if !endpoints.is_empty() {
        cfg["endpoints"] = serde_json::json!(endpoints);
    }
    if let Some(token) = ws_token {
        cfg["wsToken"] = serde_json::json!(token);
    }
    // The app's declared initial state. The SDK seeds its own document from this at
    // construction, so `data-br-bind` KPIs paint their real values on FIRST LOAD —
    // before the socket connects, and even in an export with no daemon reachable.
    // Without it the shared doc is `{}` until a paid agent turn writes to it, every
    // binding renders blank, and authors work around it with a private local object
    // that then silently diverges from the doc the agent reads.
    if let Some(initial) = manifest.surface.state_initial.as_ref() {
        cfg["stateInitial"] = initial.clone();
    }
    let json = serde_json::to_string(&cfg).unwrap_or_else(|_| "{}".to_string());
    // Emit the config as a NON-executable data island so the served page can ship
    // the strict CSP (`script-src 'self'`, no `unsafe-inline`) that SDK v2 needs —
    // the SDK's `readAppConfig()` reads `#biorouter-app-config`'s textContent and
    // `JSON.parse`s it. `<` is a valid JSON string escape, so replacing every
    // `<` keeps the JSON well-formed while neutralising any `</script>` breakout
    // from string fields (the HTML parser ends a <script> on `</script`, regardless
    // of type). No legacy `window.BIOROUTER_APP_CONFIG` inline script is emitted:
    // the SDK still falls back to that global, but nothing on the served/exported
    // path needs it (rebuild-on-drift keeps every vendored SDK current), and an
    // executable inline script would defeat the CSP.
    let json = json.replace('<', "\\u003c");
    format!("<script type=\"application/json\" id=\"biorouter-app-config\">{json}</script>\n")
}

/// Assemble the HTML `biorouterd` serves for a live app at `/apps/<id>/`.
///
/// `base_href` should be the serving prefix (e.g. `/apps/<id>/`) so that the
/// relative `dist/app.js` and any relative assets resolve correctly. `endpoint`
/// overrides the agent WebSocket URL (None → derived from page location).
///
/// The bundle tag is included for an agentic app, which always has one; a
/// caller that knows whether `dist/app.js` exists uses [`assemble_app_page`].
pub fn assemble_app(
    manifest: &Manifest,
    index_html: &str,
    base_href: Option<&str>,
    endpoint: Option<&str>,
    ws_token: Option<&str>,
) -> String {
    assemble_app_with_endpoints(manifest, index_html, base_href, endpoint, &[], ws_token)
}

/// [`assemble_app`], plus fallback endpoints the SDK tries when the primary one
/// (usually the page's own origin) can't be reached.
pub fn assemble_app_with_endpoints(
    manifest: &Manifest,
    index_html: &str,
    base_href: Option<&str>,
    endpoint: Option<&str>,
    endpoints: &[String],
    ws_token: Option<&str>,
) -> String {
    assemble(
        manifest,
        index_html,
        base_href,
        endpoint,
        endpoints,
        ws_token,
        manifest.kind == ArtifactKind::Agentic,
        None,
    )
}

/// The page `serve_index` sends for `/apps/<id>/`.
///
/// `has_bundle` is whether `dist/app.js` exists. ⚠ The bundle tag used to be
/// appended unconditionally, so a static page with no bundle asked the browser
/// for a file that 404s, and one whose author already loads its bundle
/// (`<script type="module" src="./dist/app.js">`) ran it twice. Now it is
/// appended only when there is a bundle and the author does not load it.
///
/// `handler_nonce` is a fresh random value for this one response: the page's
/// `on…=` handlers are moved into a script bound to it
/// ([`bind_inline_handlers`]), which goes before the config island and the
/// bundle, so it runs before anything that could insert markup.
pub fn assemble_app_page(
    manifest: &Manifest,
    index_html: &str,
    base_href: Option<&str>,
    ws_token: Option<&str>,
    has_bundle: bool,
    handler_nonce: &str,
) -> String {
    assemble(
        manifest,
        index_html,
        base_href,
        None,
        &[],
        ws_token,
        has_bundle,
        Some(handler_nonce),
    )
}

#[allow(clippy::too_many_arguments)]
fn assemble(
    manifest: &Manifest,
    index_html: &str,
    base_href: Option<&str>,
    endpoint: Option<&str>,
    endpoints: &[String],
    ws_token: Option<&str>,
    include_bundle: bool,
    handler_nonce: Option<&str>,
) -> String {
    let authored = strip_injected(index_html);
    let mut head = String::new();
    if let Some(base) = base_href {
        head.push_str(&format!("<base href=\"{}\">\n", html_escape(base)));
    }
    head.push_str(&theme_head(manifest));
    let mut html = inject_after(&authored, "<head>", &head);
    // If there was no <head>, inject_after fell back to prepending; ensure the
    // theme is still present (it is, since `head` was prepended).
    if !html.contains(THEME_TAG) {
        html = format!("{}{html}", theme_head(manifest));
    }
    // Select the app's theme pack (and enable its override layer) on <html>.
    if let Some(pack) = pack_attr_value(manifest) {
        html = set_root_pack(&html, pack);
    }

    let mut tail = String::new();
    if let Some(nonce) = handler_nonce {
        let (bound, binder) = bind_inline_handlers(&html, nonce);
        html = bound;
        tail.extend(binder);
    }
    tail.push_str(&app_config_script(manifest, endpoint, endpoints, ws_token));
    if include_bundle && !authored_inline_code(&authored).loads_bundle() {
        tail.push_str(BUNDLE_TAG);
        tail.push('\n');
    }
    inject_before(&html, "</body>", &tail, true)
}

/// A lightweight static preview for inline chat display. No live agent — apps
/// are launched in the browser. Shows the styled UI plus a launch hint banner.
pub fn assemble_card(manifest: &Manifest, index_html: &str) -> String {
    let head = theme_head(manifest);
    let authored = strip_injected(index_html);
    let mut html = inject_after(&authored, "<head>", &head);
    if !html.contains(THEME_TAG) {
        html = format!("{head}{html}");
    }
    if let Some(pack) = pack_attr_value(manifest) {
        html = set_root_pack(&html, pack);
    }
    let launch_hint = if manifest.kind == ArtifactKind::Agentic {
        "Launch from the Applications panel to use the live agent."
    } else {
        "Launch from the Applications panel to open it in your browser."
    };
    let banner = format!(
        "<div style=\"position:sticky;top:0;z-index:10;background:var(--br-medium);\
         color:var(--br-text-muted);font-size:12px;padding:6px 12px;border-bottom:1px solid var(--br-border);\">\
         Preview of <strong>{}</strong>. {launch_hint}</div>\n",
        html_escape(&manifest.title)
    );
    inject_after(&html, "<body>", &banner)
}

// ---------------------------------------------------------------------------
// Authored inline code, and the policy sources that let exactly it run
// ---------------------------------------------------------------------------

/// The executable code an author wrote straight into an HTML page.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct InlineCode {
    /// The text of every inline executable `<script>`, exactly as a browser
    /// hashes it (newlines normalized, as the HTML parser does).
    pub scripts: Vec<String>,
    /// The decoded value of every `on…=` event-handler attribute.
    pub handlers: Vec<String>,
    /// The decoded `src` of every external `<script>`.
    pub script_srcs: Vec<String>,
    /// How many `href`/`src`/`action`/`formaction` values are `javascript:`
    /// URLs, which no script-src source can allow.
    pub javascript_urls: usize,
}

impl InlineCode {
    /// Whether the page already loads its own bundle (`dist/app.js`, however
    /// spelled), so the assembler must not add a second tag.
    pub fn loads_bundle(&self) -> bool {
        self.script_srcs.iter().any(|src| {
            let path = src.split(['?', '#']).next().unwrap_or("").trim();
            path == "dist/app.js" || path.ends_with("/dist/app.js")
        })
    }
}

/// The `script-src` sources that let the inline scripts in `html` run, and
/// nothing else: a `'sha256-…'` per distinct inline executable `<script>`.
/// Empty for a page with none.
///
/// Why hashes and not `'unsafe-inline'`: the strict policy exists to stop code
/// injected at RUNTIME — an agent-emitted `html` node, a data binding — and a
/// hash only matches text the author wrote. A `<script>` inserted with
/// `innerHTML` never runs anyway, so pinning the author's scripts by text
/// gives an injection nothing to reuse.
///
/// ⚠ Handlers are NOT hashed, and `'unsafe-hashes'` is never sent. With it, CSP
/// matches a handler by its TEXT, on whatever element carries it, so markup
/// injected later (`<img src=x onerror="…">` in an agent's `html` node, a tool
/// name the SDK writes with `innerHTML`) could run any handler the author
/// wrote, with `this` an element the injection chose — `br.chat(this.dataset.q)`
/// then sends the injection's own prompt, no click needed. The page's handlers
/// are moved into a script instead ([`bind_inline_handlers`]), whose hash this
/// returns like any other script's.
///
/// ⚠ Computed over the page as SERVED, not as stored, so the hashes describe
/// exactly the bytes the browser parses.
pub fn inline_script_sources(html: &str) -> Vec<String> {
    use base64::Engine as _;
    use sha2::Digest as _;
    let mut sources: Vec<String> = authored_inline_code(html)
        .scripts
        .iter()
        .map(|text| {
            let digest = sha2::Sha256::digest(text.as_bytes());
            format!(
                "'sha256-{}'",
                base64::engine::general_purpose::STANDARD.encode(digest)
            )
        })
        .collect();
    sources.sort();
    sources.dedup();
    sources
}

/// The `id` of the `<script>` [`bind_inline_handlers`] adds.
const HANDLER_BINDER_ID: &str = "biorouter-inline-handlers";
/// The attribute an authored handler becomes: `data-br-on-<event>`.
const HANDLER_MARKER: &str = "data-br-on-";

/// Whether an attribute name is an `on…` event handler's.
fn is_handler_name(key: &str) -> bool {
    key.len() > 2 && key.starts_with("on")
}

/// The function an authored handler's text becomes. The scope chain is the one
/// a browser compiles a handler attribute with — the document, the element's
/// form, the element — and `this` is the element.
fn handler_function(body: &str) -> String {
    format!(
        "function (event, __brForm) {{ with (document) with (__brForm || {{}}) with (this) {{\n{body}\n}} }}"
    )
}

/// Whether `body` can move into the binder script: it cannot end that script
/// early (`</script`, or `<!--`, which can make the parser skip the real end
/// tag), and wrapped in [`handler_function`] it parses as exactly ONE function,
/// so a syntax error, or a body that closes the wrapper and opens another, can
/// break neither the script nor any other handler's place in it.
fn handler_is_bindable(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    if lower.contains("</script") || lower.contains("<!--") {
        return false;
    }
    let source = format!("({});", handler_function(body));
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_javascript::LANGUAGE.into())
        .is_err()
    {
        return false;
    }
    let Some(tree) = parser.parse(&source, None) else {
        return false;
    };
    let root = tree.root_node();
    if root.has_error() || root.named_child_count() != 1 {
        return false;
    }
    // The grammar also reads JSX, which a browser does not.
    let mut cursor = root.walk();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind().starts_with("jsx") {
            return false;
        }
        stack.extend(node.children(&mut cursor));
    }
    root.named_child(0)
        .filter(|statement| statement.kind() == "expression_statement")
        .and_then(|statement| statement.named_child(0))
        .filter(|paren| {
            paren.kind() == "parenthesized_expression" && paren.named_child_count() == 1
        })
        .and_then(|paren| paren.named_child(0))
        .is_some_and(|function| function.kind() == "function_expression")
}

/// The script that attaches the handlers [`bind_inline_handlers`] took out of
/// the markup: once, to the elements whose marker carries this page's
/// `nonce`, and then never again.
fn handler_binder_script(nonce: &str, bodies: &[String]) -> String {
    let functions = bodies
        .iter()
        .map(|body| handler_function(body))
        .collect::<Vec<_>>()
        .join(",\n");
    format!(
        r#"<script id="{HANDLER_BINDER_ID}">
(function () {{
  var done = "__biorouterInlineHandlers";
  if (Object.prototype.hasOwnProperty.call(window, done)) return;
  Object.defineProperty(window, done, {{ value: true }});
  var nonce = "{nonce}";
  var marker = "{HANDLER_MARKER}";
  var handlers = [
{functions}
  ];
  var windowEvents = " afterprint beforeprint beforeunload blur error focus hashchange languagechange load message messageerror offline online pagehide pageshow popstate rejectionhandled resize scroll storage unhandledrejection unload ";
  function bind(el, type, handler) {{
    var onWindow = (el === document.body || el.localName === "frameset") && windowEvents.indexOf(" " + type + " ") >= 0;
    var target = onWindow ? window : el;
    if (!(("on" + type) in target)) return;
    var form = !onWindow && el.localName !== "form" && el.form instanceof HTMLFormElement ? el.form : null;
    target.addEventListener(type, function (event) {{
      var result = handler.call(target, event, form);
      if (onWindow && type === "error") {{
        if (result === true) event.preventDefault();
      }} else if (type === "beforeunload") {{
        if (result != null) {{ event.preventDefault(); event.returnValue = String(result); }}
      }} else if (result === false) {{
        event.preventDefault();
      }}
    }});
  }}
  var all = document.getElementsByTagName("*");
  var elements = [];
  for (var i = 0; i < all.length; i++) elements.push(all[i]);
  for (var e = 0; e < elements.length; e++) {{
    var el = elements[e];
    for (var a = el.attributes.length - 1; a >= 0; a--) {{
      var attr = el.attributes[a];
      if (attr.name.lastIndexOf(marker, 0) !== 0) continue;
      var name = attr.name, value = attr.value, dot = value.lastIndexOf(".");
      el.removeAttribute(name);
      var handler = value.slice(0, dot) === nonce ? handlers[+value.slice(dot + 1)] : null;
      if (handler) bind(el, name.slice(marker.length), handler);
    }}
  }}
}})();
</script>
"#
    )
}

/// Move the `on…=` handlers written in `html` into a script, returning the
/// rewritten page and that script (`None` when there was nothing to move).
///
/// Each handler attribute becomes `data-br-on-<event>="<nonce>.<n>"`, and the
/// script holds handler `n` as a function and attaches it, ONCE, to each
/// element whose marker names this response's `nonce`. It runs before the
/// app's bundle and before any agent frame can arrive, so markup added later —
/// an injection — is never bound: it would need the nonce, which it never sees
/// (the script removes each marker as it reads it), and the script refuses to
/// run twice. This is what lets a static page's `onclick="…"` work under a
/// policy without `'unsafe-hashes'` ([`inline_script_sources`] says why that is
/// not sent).
///
/// What is left as written, and so stays blocked as every handler was before
/// hashes: a handler inside a `<template>` (a clone could not carry the
/// listener, and binding clones later would bind injections too), and one the
/// script could not hold ([`handler_is_bindable`]). A repeated attribute, which
/// a browser drops, is removed.
pub fn bind_inline_handlers(html: &str, nonce: &str) -> (String, Option<String>) {
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    let mut bodies: Vec<String> = Vec::new();
    let mut template_depth = 0usize;
    scan_tags(html, |tag| match tag {
        Tag::End { name } => {
            if name == "template" {
                template_depth = template_depth.saturating_sub(1);
            }
        }
        Tag::Start { name, attrs, .. } => {
            if template_depth == 0 {
                for attr in attrs.iter().filter(|a| is_handler_name(&a.key)) {
                    if attr.duplicate {
                        edits.push((attr.span.clone(), String::new()));
                        continue;
                    }
                    if !handler_is_bindable(&attr.value) {
                        continue;
                    }
                    let n = bodies
                        .iter()
                        .position(|body| *body == attr.value)
                        .unwrap_or_else(|| {
                            bodies.push(attr.value.clone());
                            bodies.len() - 1
                        });
                    let event = attr.key.strip_prefix("on").unwrap_or_default();
                    edits.push((
                        attr.span.clone(),
                        format!("{HANDLER_MARKER}{event}=\"{nonce}.{n}\""),
                    ));
                }
            }
            if name == "template" {
                template_depth += 1;
            }
        }
    });
    if bodies.is_empty() && edits.is_empty() {
        return (html.to_string(), None);
    }
    let mut out = String::with_capacity(html.len());
    let mut copied = 0;
    for (span, replacement) in edits {
        out.push_str(html.get(copied..span.start).unwrap_or_default());
        out.push_str(&replacement);
        copied = span.end;
    }
    out.push_str(html.get(copied..).unwrap_or_default());
    let script = (!bodies.is_empty()).then(|| handler_binder_script(nonce, &bodies));
    (out, script)
}

/// Elements whose content is text, not markup: a `<script>` written inside one
/// is not a script.
const RAW_TEXT_ELEMENTS: &[&str] = &[
    "style", "textarea", "title", "xmp", "iframe", "noembed", "noframes", "noscript",
];

/// The `type` values (and the `language` fallback) a browser executes, per the
/// HTML standard's "JavaScript MIME type essence match". Anything else — JSON
/// islands, `text/template` — is a data block and never runs.
const JS_MIME_TYPES: &[&str] = &[
    "application/ecmascript",
    "application/javascript",
    "application/x-ecmascript",
    "application/x-javascript",
    "text/ecmascript",
    "text/javascript",
    "text/javascript1.0",
    "text/javascript1.1",
    "text/javascript1.2",
    "text/javascript1.3",
    "text/javascript1.4",
    "text/javascript1.5",
    "text/jscript",
    "text/livescript",
    "text/x-ecmascript",
    "text/x-javascript",
];

/// One attribute of a start tag, as the tokenizer reads it.
#[derive(Debug, Clone)]
struct TagAttr {
    /// Lowercase name.
    key: String,
    /// Decoded value.
    value: String,
    /// Where the whole attribute, name through value, sits in the page.
    span: std::ops::Range<usize>,
    /// A later attribute of the same name on the same tag, which a browser
    /// drops (the first occurrence wins).
    duplicate: bool,
}

/// A tag as [`scan_tags`] reports it.
enum Tag<'a> {
    Start {
        /// Lowercase name.
        name: String,
        attrs: Vec<TagAttr>,
        /// A `<script>` element's text, as written.
        script_text: Option<&'a str>,
    },
    End {
        /// Lowercase name.
        name: String,
    },
}

/// Walk `html` the way a browser tokenizes it, closely enough to find the code
/// an author wrote, handing each start and end tag to `visit`: comments, the
/// text of raw-text elements and the text of scripts are skipped, and attribute
/// values are decoded.
///
/// It is deliberately a scanner, not a parser, and errs toward MISSING code
/// (which then stays blocked, as it always was) rather than inventing it: a
/// hash can only ever allow text that is really in the page.
fn scan_tags<'a>(html: &'a str, mut visit: impl FnMut(Tag<'a>)) {
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        // `i` is at a `<`, an ASCII byte, so this always succeeds.
        let Some(rest) = html.get(i..) else {
            break;
        };
        if let Some(comment) = rest.strip_prefix("<!--") {
            match comment.find("-->") {
                Some(end) => i += 4 + end + 3,
                None => break,
            }
            continue;
        }
        match bytes.get(i + 1) {
            Some(b'/') => {
                // `</name …>`; anything else after `</` is a bogus comment.
                if bytes.get(i + 2).is_some_and(u8::is_ascii_alphabetic) {
                    let name_end = bytes
                        .get(i + 2..)
                        .and_then(|tail| {
                            tail.iter().position(|c| {
                                matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0c | b'/' | b'>')
                            })
                        })
                        .map_or(bytes.len(), |at| i + 2 + at);
                    let name = html.get(i + 2..name_end).unwrap_or_default();
                    visit(Tag::End {
                        name: name.to_ascii_lowercase(),
                    });
                }
                match rest.find('>') {
                    Some(end) => i += end + 1,
                    None => break,
                }
                continue;
            }
            Some(b'!' | b'?') => {
                match rest.find('>') {
                    Some(end) => i += end + 1,
                    None => break,
                }
                continue;
            }
            Some(c) if c.is_ascii_alphabetic() => {}
            _ => {
                i += 1;
                continue;
            }
        }
        let (name, attrs, after_tag) = scan_start_tag(html, i + 1);
        i = after_tag;
        if name == "script" {
            let (content_end, after_close) = scan_to_end_tag(html, i, "script");
            let text = html.get(i..content_end).unwrap_or("");
            visit(Tag::Start {
                name,
                attrs,
                script_text: Some(text),
            });
            i = after_close;
        } else if RAW_TEXT_ELEMENTS.contains(&name.as_str()) {
            let after_close = scan_to_end_tag(html, i, &name).1;
            visit(Tag::Start {
                name,
                attrs,
                script_text: None,
            });
            i = after_close;
        } else {
            let plaintext = name == "plaintext";
            visit(Tag::Start {
                name,
                attrs,
                script_text: None,
            });
            if plaintext {
                break;
            }
        }
    }
}

/// The inline code an author wrote into `html` ([`scan_tags`]): scripts with
/// their text newline-normalized, handlers, external script sources and
/// `javascript:` URLs.
pub fn authored_inline_code(html: &str) -> InlineCode {
    let mut code = InlineCode::default();
    scan_tags(html, |tag| {
        let Tag::Start {
            name,
            attrs,
            script_text,
        } = tag
        else {
            return;
        };
        let attrs: Vec<&TagAttr> = attrs.iter().filter(|a| !a.duplicate).collect();
        let attr = |key: &str| {
            attrs
                .iter()
                .find(|a| a.key == key)
                .map(|a| a.value.as_str())
        };
        for a in &attrs {
            if is_handler_name(&a.key) {
                code.handlers.push(a.value.clone());
            }
            if matches!(
                a.key.as_str(),
                "href" | "src" | "action" | "formaction" | "xlink:href"
            ) && a
                .value
                .trim_start()
                .get(..11)
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case("javascript:"))
            {
                code.javascript_urls += 1;
            }
        }
        if name == "script" {
            if let Some(src) = attr("src") {
                code.script_srcs.push(src.to_string());
            } else if script_is_executable(attr("type"), attr("language")) {
                code.scripts
                    .push(normalize_script_text(script_text.unwrap_or("")));
            }
        }
    });
    code
}

fn script_is_executable(type_attr: Option<&str>, language: Option<&str>) -> bool {
    let mime = match (type_attr.map(str::trim), language.map(str::trim)) {
        (Some(t), _) if !t.is_empty() => t.to_ascii_lowercase(),
        (Some(_), _) => return true,
        (None, Some(l)) if !l.is_empty() => format!("text/{}", l.to_ascii_lowercase()),
        (None, _) => return true,
    };
    mime == "module" || mime == "importmap" || JS_MIME_TYPES.contains(&mime.as_str())
}

/// The HTML parser turns CRLF and lone CR into LF before anything else, and a
/// NUL inside script text into U+FFFD; the hash a browser computes is over that.
fn normalize_script_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\0', "\u{FFFD}")
}

/// Read a start tag whose name begins at `at`: its lowercase name, its
/// attributes in order (lowercase names, decoded values, spans, and a repeated
/// name flagged as the browser drops it), and the index just past its `>`.
fn scan_start_tag(html: &str, at: usize) -> (String, Vec<TagAttr>, usize) {
    let b = html.as_bytes();
    let is_space = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0c);
    let mut j = at;
    while j < b.len() && !is_space(b[j]) && b[j] != b'/' && b[j] != b'>' {
        j += 1;
    }
    // Every boundary below is at an ASCII delimiter, so every `get` succeeds.
    let name = html.get(at..j).unwrap_or_default().to_ascii_lowercase();
    let mut attrs: Vec<TagAttr> = Vec::new();
    loop {
        while j < b.len() && (is_space(b[j]) || b[j] == b'/') {
            j += 1;
        }
        if j >= b.len() {
            return (name, attrs, b.len());
        }
        if b[j] == b'>' {
            return (name, attrs, j + 1);
        }
        let key_start = j;
        // A leading `=` belongs to the name (the tokenizer's quirk).
        j += 1;
        while j < b.len() && !is_space(b[j]) && !matches!(b[j], b'/' | b'>' | b'=') {
            j += 1;
        }
        let key_end = j;
        let key = html
            .get(key_start..j)
            .unwrap_or_default()
            .to_ascii_lowercase();
        while j < b.len() && is_space(b[j]) {
            j += 1;
        }
        let mut value = String::new();
        let mut attr_end = key_end;
        if j < b.len() && b[j] == b'=' {
            j += 1;
            while j < b.len() && is_space(b[j]) {
                j += 1;
            }
            if j < b.len() && (b[j] == b'"' || b[j] == b'\'') {
                let quote = b[j];
                j += 1;
                let start = j;
                while j < b.len() && b[j] != quote {
                    j += 1;
                }
                value = decode_char_refs(html.get(start..j).unwrap_or_default());
                j = (j + 1).min(b.len());
            } else {
                let start = j;
                while j < b.len() && !is_space(b[j]) && b[j] != b'>' {
                    j += 1;
                }
                value = decode_char_refs(html.get(start..j).unwrap_or_default());
            }
            attr_end = j;
        }
        let duplicate = attrs.iter().any(|a| a.key == key);
        attrs.push(TagAttr {
            key,
            value,
            span: key_start..attr_end,
            duplicate,
        });
    }
}

/// Find the end tag `</name` (case-insensitive, followed by whitespace, `/` or
/// `>`) from `from`. Returns where the element's text ends and the index just
/// past the end tag's `>`; both are the end of `html` when there is none.
fn scan_to_end_tag(html: &str, from: usize, name: &str) -> (usize, usize) {
    let needle = format!("</{name}");
    let mut search = from;
    while let Some(offset) = find_ascii_ci(html.get(search..).unwrap_or_default(), &needle) {
        let at = search + offset;
        let after = at + needle.len();
        match html.as_bytes().get(after) {
            None => return (at, html.len()),
            Some(b' ' | b'\t' | b'\n' | b'\r' | 0x0c | b'/' | b'>') => {
                let close = html
                    .get(after..)
                    .and_then(|tail| tail.find('>'))
                    .map_or(html.len(), |end| after + end + 1);
                return (at, close);
            }
            Some(_) => search = after,
        }
    }
    (html.len(), html.len())
}

/// Decode the character references a browser decodes in an attribute value:
/// numeric ones, and the named ones authors actually write in handlers. An
/// unknown name is left as written — that handler's hash then misses and it
/// stays blocked, as every handler was before hashes.
fn decode_char_refs(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(amp) = rest.find('&') {
        let (before, from_amp) = rest.split_at(amp);
        out.push_str(before);
        // A reference is ASCII, so `used` (and the 1 byte of `&`) always ends
        // on a character boundary.
        let used = match char_ref(from_amp) {
            Some((decoded, used)) => {
                out.push(decoded);
                used
            }
            None => {
                out.push('&');
                1
            }
        };
        rest = from_amp.get(used..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

/// The character `text` (which starts with `&`) refers to, and how many bytes
/// the reference spans.
fn char_ref(text: &str) -> Option<(char, usize)> {
    let body = text.get(1..)?;
    if let Some(num) = body.strip_prefix('#') {
        let (radix, digits_at) = match num.as_bytes().first() {
            Some(b'x' | b'X') => (16, 1),
            _ => (10, 0),
        };
        let digits: String = num
            .get(digits_at..)?
            .chars()
            .take_while(|c| c.is_digit(radix))
            .collect();
        if digits.is_empty() {
            return None;
        }
        let mut used = 2 + digits_at + digits.len();
        if text.get(used..).is_some_and(|tail| tail.starts_with(';')) {
            used += 1;
        }
        let code = u32::from_str_radix(&digits, radix).unwrap_or(u32::MAX);
        return Some((numeric_char(code), used));
    }
    const NAMED: &[(&str, char)] = &[
        ("amp", '&'),
        ("lt", '<'),
        ("gt", '>'),
        ("quot", '"'),
        ("apos", '\''),
        ("nbsp", '\u{a0}'),
        ("Tab", '\t'),
        ("NewLine", '\n'),
    ];
    for (name, ch) in NAMED {
        if let Some(after) = body.strip_prefix(name) {
            if after.starts_with(';') {
                return Some((*ch, 1 + name.len() + 1));
            }
            // The legacy no-semicolon forms, which an attribute decodes only
            // when the next character could not continue a name.
            let legacy = matches!(*name, "amp" | "lt" | "gt" | "quot" | "nbsp");
            let continues = after
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '=');
            if legacy && !continues {
                return Some((*ch, 1 + name.len()));
            }
        }
    }
    None
}

/// A numeric character reference's character, with the HTML standard's
/// replacements: NUL, surrogates and out-of-range code points become U+FFFD,
/// and 0x80–0x9F map through windows-1252.
fn numeric_char(code: u32) -> char {
    const C1: [u32; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160,
        0x2039, 0x0152, 0x8D, 0x017D, 0x8F, 0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013,
        0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
    ];
    let code = match code {
        0x80..=0x9F => C1[(code - 0x80) as usize],
        other => other,
    };
    if code == 0 {
        return '\u{FFFD}';
    }
    char::from_u32(code).unwrap_or('\u{FFFD}')
}

// ---------------------------------------------------------------------------
// Standalone export scaffolding (TypeScript project against a Biorouter daemon)
// ---------------------------------------------------------------------------

/// Files every export writes itself. An authored file with one of these names
/// is exported under `app-files/` instead (`export::export_path_for`): an app's
/// own `README.md` or `run.sh` used to replace the launcher's, and the folder
/// could then not be run as documented.
pub const GENERATED_EXPORT_FILES: &[&str] = &[
    "manifest.json",
    "package.json",
    "serve.mjs",
    "README.md",
    "biorouter-launch.sh",
    "run.command",
    "run.sh",
    "run.ps1",
    "run.bat",
    "export.json",
];

/// Top-level folders an export owns: the full-mode payload, and the place
/// authored files that collide with [`GENERATED_EXPORT_FILES`] go.
pub const RESERVED_EXPORT_DIRS: &[&str] = &["payload", "app-files"];

/// Files in an export that must be executable for the launcher to be
/// double-clickable. Every writer gives them the bit: `export::build_export`
/// on disk, the zip the HTTP route answers with (unix permissions in each
/// entry), and the desktop's extraction of that zip. `biorouter-launch.sh` is
/// sourced rather than run, so only `run.command` and `run.sh` strictly need
/// it; all three carry it so a folder never looks half-executable.
pub const EXECUTABLE_EXPORT_FILES: &[&str] = &["run.command", "run.sh", "biorouter-launch.sh"];

/// The manifest an export carries: the app's own, without `session_id`. The
/// chat an app was built in is an id on the author's machine; on another one
/// it names nothing, or an unrelated chat, which the Applications view would
/// then offer to open as "the chat where this app was built".
pub fn exported_manifest_json(manifest: &Manifest) -> String {
    let mut exported = manifest.clone();
    exported.session_id = None;
    serde_json::to_string_pretty(&exported).unwrap_or_default()
}

/// Skills that ship with Biorouter, which every install seeds for itself:
/// `skills_extension::is_builtin_skill_name` in the `biorouter` crate, which
/// this crate cannot import (`export::tests` holds the two together).
///
/// ⚠ An export never carries one, and a launcher never installs one. The
/// recipient already has its own copy, possibly newer, and installing the
/// author's over it under the seeded name is exactly what
/// `skill_package::refuse_shipped` exists to stop: the seeder then writes its
/// `SKILL.md` back inside the installed copy, which can no longer be removed.
pub const SHIPPED_SKILL_NAMES: &[&str] = &[
    "office-word",
    "office-powerpoint",
    "office-excel",
    "office-pdf",
    "about-biorouter",
    "develop-biorouter",
    "develop-biorouter-extension",
    "develop-biorouter-skill",
    "knowledge-choose-a-format",
    "knowledge-ingest-okf",
    "knowledge-ingest-biookf",
    "knowledge-lint",
    "update-soul",
];

/// What the store's copy of an app held when an export was made of it, as one
/// SHA-256: the files a launcher installs over (the entry page, `src/`,
/// `assets/`, and `dist/` only for an app with no `src/main.ts`, whose bundle
/// the daemon derives), each hashed, one `"<hash> <path>\n"` line per file,
/// lines sorted by byte, and that text hashed. `.DS_Store` files and links are
/// left out. `None` when a file cannot be read.
///
/// The launchers compute exactly this over the store before they re-sync it
/// (`runtime_digest` in `biorouter-launch.sh`, `Get-RuntimeDigest` in run.ps1,
/// `runtimeDigest` in serve.mjs), and re-sync only a store that still matches
/// what this folder was exported from or what a launcher last installed.
/// ⚠ `updated_at` alone could not say: only the drafter's own tools bump it,
/// so an edit made with the developer shell or text editor, or a newer
/// Biorouter refreshing `src/sdk.ts` on a rebuild, left it equal, and running
/// the old folder reverted the edit or brought back the old SDK.
pub fn runtime_digest(app_dir: &std::path::Path, entry: &str) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::collections::BTreeSet;
    use std::path::Path;
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
    fn walk(base: &Path, rel: &str, out: &mut BTreeSet<String>) -> std::io::Result<()> {
        for item in std::fs::read_dir(base.join(rel))? {
            let item = item?;
            let kind = item.file_type()?;
            let name = item.file_name().to_string_lossy().to_string();
            let path = format!("{rel}/{name}");
            if kind.is_dir() {
                walk(base, &path, out)?;
            } else if kind.is_file() && name != ".DS_Store" {
                out.insert(path);
            }
        }
        Ok(())
    }
    let regular = |rel: &str| {
        std::fs::symlink_metadata(app_dir.join(rel)).is_ok_and(|meta| meta.file_type().is_file())
    };
    let mut files = BTreeSet::new();
    if regular(entry) {
        files.insert(entry.to_string());
    }
    let built_from_source = regular("src/main.ts");
    for sub in ["src", "assets", "dist"] {
        if sub == "dist" && built_from_source {
            continue;
        }
        let is_dir = std::fs::symlink_metadata(app_dir.join(sub)).is_ok_and(|meta| meta.is_dir());
        if is_dir {
            walk(app_dir, sub, &mut files).ok()?;
        }
    }
    let mut lines = Vec::with_capacity(files.len());
    for rel in files {
        let bytes =
            std::fs::read(rel.split('/').fold(app_dir.to_path_buf(), |p, s| p.join(s))).ok()?;
        lines.push(format!("{} {rel}\n", hex(&Sha256::digest(&bytes))));
    }
    lines.sort();
    Some(hex(&Sha256::digest(lines.concat().as_bytes())))
}

/// The launchers, dev server, README and package.json every export writes,
/// as (relative path, contents). The app's own files, its manifest
/// ([`exported_manifest_json`]) and any payload are the writer's.
///
/// `export_digest` is the store's [`runtime_digest`] at export time, written
/// into each launcher; `None` (a store that could not be read) leaves the
/// launchers to the digest their own installs record.
pub fn launcher_files(manifest: &Manifest, export_digest: Option<&str>) -> Vec<(String, String)> {
    // Hex only: it lands inside shell, PowerShell and JavaScript string literals.
    let digest: String = export_digest
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_hexdigit)
        .collect();
    let digest = digest.as_str();
    vec![
        ("package.json".to_string(), package_json(manifest)),
        (
            "serve.mjs".to_string(),
            serve_mjs(&manifest.id, 8787, digest),
        ),
        ("README.md".to_string(), readme(manifest)),
        // Directly-runnable launchers (double-click on macOS / `bash run.sh`).
        (
            "biorouter-launch.sh".to_string(),
            launcher_lib(&manifest.id, digest),
        ),
        ("run.command".to_string(), run_script()),
        ("run.sh".to_string(), run_script()),
        // Windows launchers: run.bat (double-clickable) shells out to run.ps1.
        ("run.ps1".to_string(), run_ps1(&manifest.id, digest)),
        ("run.bat".to_string(), run_bat()),
    ]
}

fn package_json(manifest: &Manifest) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "name": manifest.id,
        "version": "0.1.0",
        "private": true,
        "type": "module",
        "scripts": {
            "build": "esbuild src/main.ts --bundle --format=iife --outfile=dist/app.js",
            "start": "node serve.mjs"
        },
        "devDependencies": { "esbuild": "^0.23.0" }
    }))
    .unwrap_or_default()
}

/// Shared shell prelude: locate a `biorouterd`, install the app into the local
/// store, and bring a daemon up on a known port. Sourced by `run.sh` so the two
/// launch paths can't drift.
///
/// Defines `BIOROUTERD`, `PORT`, `SECRET` (the daemon's, never printed and never
/// on a command line) and, after `launch_url`, `LAUNCH_URL`: the app's one-time
/// launch link (W2-HRD-1).
///
/// Built with `.replace` rather than `format!`, so the shell's braces need no
/// doubling. App ids are slugified (`[A-Za-z0-9_-]`), so they are safe inside
/// the double-quoted shell strings they land in.
fn launcher_lib(id: &str, export_digest: &str) -> String {
    LAUNCHER_LIB_TEMPLATE
        .replace("__APP_ID__", id)
        .replace("__EXPORT_DIGEST__", export_digest)
        .replace("__SHIPPED_SKILLS__", &SHIPPED_SKILL_NAMES.join("|"))
}

const LAUNCHER_LIB_TEMPLATE: &str = r##"#!/usr/bin/env bash
# Shared launcher helpers for the exported Biorouter app "__APP_ID__".
# Sourced by run.sh / run.command; also usable on its own.
APP_ID="__APP_ID__"
DIR="${DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"

die() { echo "error: $*" >&2; exit 1; }

# ── 0. Where Biorouter keeps its files ────────────────────────────────────
# Exactly the folder the daemon reads (`paths::config_dir`): $BIOROUTER_PATH_ROOT/config
# when that is set, else $XDG_CONFIG_HOME/biorouter when XDG_CONFIG_HOME is an
# absolute path (a relative one is ignored, as the daemon ignores it), else
# ~/.config/biorouter. An app installed anywhere else is one the daemon never
# finds, and the launch fails with "does not serve".
biorouter_config_dir() {
  root="${BIOROUTER_PATH_ROOT:-}"
  if [ -n "$(printf '%s' "$root" | tr -d '[:space:]')" ]; then
    printf '%s/config\n' "$root"
    return 0
  fi
  case "${XDG_CONFIG_HOME:-}" in
    /*) printf '%s/biorouter\n' "$XDG_CONFIG_HOME" ;;
    *) printf '%s/.config/biorouter\n' "$HOME" ;;
  esac
}
CFG="$(biorouter_config_dir)"
STORE="$CFG/agent_drafter/$APP_ID"
LAUNCHER_HOME="$CFG/app-launcher"

# ── 1. Find biorouterd ────────────────────────────────────────────────────
# A bare `biorouterd` on PATH is the common case, but the desktop app ships its
# own copy and never puts it on PATH, so look there too. (The old launcher
# backgrounded a bare `biorouterd` inside `( ... & )`, which returns 0 even when
# the binary is missing, so `set -e` never caught it and the user just saw a
# dead page 40 seconds later.)
BUNDLED_FAILED=""
find_biorouterd() {
  # Guard optional vars with `:-`: run.sh runs under `set -u`, so a bare
  # $BIOROUTERD_BIN (normally UNSET, since biorouterd on PATH is the common case)
  # would abort the whole launcher with "unbound variable" before the PATH
  # lookup ever ran.
  if [ -n "${BIOROUTERD_BIN:-}" ] && [ -x "${BIOROUTERD_BIN:-}" ]; then echo "${BIOROUTERD_BIN}"; return 0; fi
  # A "fat" export bundles the daemon under payload/bin, so the folder needs no
  # Biorouter install. It is used only when it runs HERE: one built for another
  # platform (or one macOS has not approved yet) cannot start, and preferring it
  # anyway made every port attempt die while an installed biorouterd sat unused.
  if [ -x "$DIR/payload/bin/biorouterd" ]; then
    if "$DIR/payload/bin/biorouterd" --version >/dev/null 2>&1; then echo "$DIR/payload/bin/biorouterd"; return 0; fi
    BUNDLED_FAILED=1
  fi
  if command -v biorouterd >/dev/null 2>&1; then command -v biorouterd; return 0; fi
  for p in \
    "$HOME/.local/bin/biorouterd" \
    "/usr/local/bin/biorouterd" \
    "/opt/homebrew/bin/biorouterd" \
    "/Applications/Biorouter.app/Contents/Resources/bin/biorouterd" \
    "$HOME/Applications/Biorouter.app/Contents/Resources/bin/biorouterd" \
    "/opt/Biorouter/resources/bin/biorouterd" \
    "/usr/lib/biorouter/resources/bin/biorouterd" \
    "/usr/lib/Biorouter/resources/bin/biorouterd"
  do
    [ -x "$p" ] && { echo "$p"; return 0; }
  done
  return 1
}

not_found() {
  if [ -n "$BUNDLED_FAILED" ]; then
    echo "the biorouterd bundled in payload/bin does not run on this computer (it was built for another platform, or macOS has not approved it yet: right-click run.command and choose Open), and no installed biorouterd was found. Install Biorouter, or set BIOROUTERD_BIN=/path/to/biorouterd"
  else
    echo "biorouterd not found. Install Biorouter, or set BIOROUTERD_BIN=/path/to/biorouterd"
  fi
}

# ── 2. Install into the local Biorouter store ─────────────────────────────
# The daemon looks apps up by `manifest.json` in the store. Only the runtime
# files go there (the manifest, the entry page, src/, dist/, assets/), and
# never over another app:
#   - nothing installed under this id: install;
#   - a DIFFERENT app has this id (another app that happened to get the same
#     name): ask, or BIOROUTER_EXPORT_REPLACE=1; it is moved to backups, never
#     deleted;
#   - this app, changed since this folder was exported (the author kept
#     working, or you did in a chat): open the installed copy as it is;
#   - this app, older than this folder: replace it, keeping a backup;
#   - this app, unchanged since the export or since a launcher last installed
#     it: re-sync, which is the edit, build, re-run loop.
# Store manifests are written two-space indented with one top-level key per
# line, which is what `manifest_field` reads. A value it cannot read is empty,
# and empty means "cannot tell".
manifest_field() {
  sed -n "s/^  \"$2\": \"\{0,1\}\([^\",]*\)\"\{0,1\},\{0,1\}\$/\1/p" "$1" 2>/dev/null | head -n 1
}

same_app() {
  ours_uid="$(manifest_field "$DIR/manifest.json" app_uid)"
  theirs_uid="$(manifest_field "$1" app_uid)"
  if [ -n "$ours_uid" ] || [ -n "$theirs_uid" ]; then
    [ "$ours_uid" = "$theirs_uid" ]
    return
  fi
  ours_created="$(manifest_field "$DIR/manifest.json" created_at)"
  theirs_created="$(manifest_field "$1" created_at)"
  [ -n "$ours_created" ] && [ "$ours_created" = "$theirs_created" ]
}

# Prints "newer", "older" or "same" for the store copy against this folder.
store_age() {
  ours="$(manifest_field "$DIR/manifest.json" updated_at)"
  theirs="$(manifest_field "$1" updated_at)"
  case "$ours:$theirs" in
    *[!0-9:]* | :* | *:) echo same; return 0 ;;
  esac
  if [ "$theirs" -gt "$ours" ]; then echo newer
  elif [ "$theirs" -lt "$ours" ]; then echo older
  else echo same
  fi
}

# Yes only from BIOROUTER_EXPORT_REPLACE=1, or from a person at a terminal.
ask_yes() {
  [ "${BIOROUTER_EXPORT_REPLACE:-}" = "1" ] && return 0
  [ -t 0 ] || return 1
  printf '%s [y/N] ' "$1"
  read -r reply || reply=""
  case "$reply" in y|Y|yes|YES) return 0 ;; *) return 1 ;; esac
}

# A fresh folder, readable by you alone, for what an install moves aside.
backup_dir() {
  dir="$LAUNCHER_HOME/backups/$APP_ID-$(date +%Y%m%d-%H%M%S)-$$"
  ( umask 077; mkdir -p "$dir" ) || return 1
  printf '%s\n' "$dir"
}

ENTRY=""

# What the store's copy held when this folder was exported (`runtime_digest`
# below, computed by the export), and where each install records what it put
# there. ⚠ Equal `updated_at` values do not mean an unchanged store: only the
# drafter's own tools bump it, so an edit made with the developer shell or a
# text editor, or a newer Biorouter refreshing src/sdk.ts on a rebuild, left
# it equal, and re-syncing then reverted the edit or brought the old SDK back.
EXPORT_DIGEST="__EXPORT_DIGEST__"
INSTALLED_RECORD="$LAUNCHER_HOME/installed/$APP_ID"

# The SHA-256 of stdin in lower-case hex; fails when there is no tool for it.
sha256_stdin() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then shasum -a 256 | cut -d' ' -f1
  else return 1
  fi
}

# One SHA-256 over the files an install replaces in the app folder $1: the
# entry page, src/, assets/, and dist/ only for an app with no src/main.ts
# (the daemon derives that bundle, and rebuilds it as it sees fit). One
# "<hash> <path>" line per file, sorted by byte, then hashed. Exactly the
# export's `render::runtime_digest`. Prints nothing when this machine has no
# hashing tool; a file it cannot read gives a digest that matches nothing, so
# both read as "changed" and nothing is overwritten.
runtime_digest() (
  set +e +u
  set +o pipefail
  cd "$1" 2>/dev/null || exit 1
  printf '' | sha256_stdin >/dev/null || exit 1
  {
    if [ -f "$ENTRY" ] && [ ! -L "$ENTRY" ]; then printf '%s\n' "$ENTRY"; fi
    for sub in src assets dist; do
      if [ "$sub" = dist ] && [ -f src/main.ts ] && [ ! -L src/main.ts ]; then continue; fi
      if [ -d "$sub" ] && [ ! -L "$sub" ]; then find "$sub" -type f ! -name .DS_Store; fi
    done
  } | LC_ALL=C sort -u | while IFS= read -r f; do
    printf '%s %s\n' "$(sha256_stdin < "$f")" "$f"
  done | LC_ALL=C sort | sha256_stdin
)

# Is the installed copy still what this folder was exported from, or what a
# launcher last installed? Anything else was changed where it is installed.
store_unchanged() {
  current="$(runtime_digest "$STORE")" || current=""
  [ -n "$current" ] || return 1
  [ "$current" = "$EXPORT_DIGEST" ] && return 0
  recorded=""
  if [ -f "$INSTALLED_RECORD" ]; then read -r recorded < "$INSTALLED_RECORD" || true; fi
  [ -n "$recorded" ] && [ "$current" = "$recorded" ]
}

record_install() {
  digest="$(runtime_digest "$STORE")" || digest=""
  ( umask 077; mkdir -p "$(dirname "$INSTALLED_RECORD")" ) 2>/dev/null || return 0
  if [ -n "$digest" ]; then
    printf '%s\n' "$digest" > "$INSTALLED_RECORD.$$" && mv -f "$INSTALLED_RECORD.$$" "$INSTALLED_RECORD" \
      || rm -f "$INSTALLED_RECORD.$$" "$INSTALLED_RECORD"
  else
    rm -f "$INSTALLED_RECORD"
  fi
  return 0
}

copy_runtime() {
  mkdir -p "$STORE" "$(dirname "$STORE/$ENTRY")" || die "could not create $STORE"
  cp "$DIR/$ENTRY" "$STORE/.entry.$$" && mv -f "$STORE/.entry.$$" "$STORE/$ENTRY" \
    || die "could not install $ENTRY"
  for sub in src dist assets; do
    [ -d "$DIR/$sub" ] || continue
    # The store keeps its manifest, whose build stamp describes ITS bundle. So
    # an app the daemon builds keeps its own dist/, and the daemon rebuilds it
    # from the src/ synced here when that changed. A copied dist/ under the kept
    # stamp could carry an older SDK that the stamp says is current, and would
    # then be served for good.
    if [ "$sub" = dist ] && [ "$1" != 1 ] && [ -f "$DIR/src/main.ts" ]; then continue; fi
    rm -rf "$STORE/.$sub.new.$$" "$STORE/.$sub.old.$$"
    cp -R "$DIR/$sub" "$STORE/.$sub.new.$$" || die "could not install $sub/"
    if [ -e "$STORE/$sub" ]; then mv "$STORE/$sub" "$STORE/.$sub.old.$$"; fi
    mv "$STORE/.$sub.new.$$" "$STORE/$sub" || die "could not install $sub/"
    rm -rf "$STORE/.$sub.old.$$"
  done
  # The manifest last: it is what makes the daemon see the app.
  if [ "$1" = 1 ]; then
    cp "$DIR/manifest.json" "$STORE/.manifest.json.$$" && mv -f "$STORE/.manifest.json.$$" "$STORE/manifest.json" \
      || die "could not install manifest.json"
  fi
}

backup_runtime() {
  backup="$(backup_dir)" || die "could not create a backup folder in $LAUNCHER_HOME/backups"
  for item in manifest.json "$ENTRY" src dist assets; do
    [ -e "$STORE/$item" ] && cp -R "$STORE/$item" "$backup/"
  done
  echo "Kept a copy of the installed '$APP_ID' in $backup"
}

install_app() {
  [ -f "$DIR/manifest.json" ] || die "manifest.json missing from this folder. Re-export the app."
  ENTRY="$(manifest_field "$DIR/manifest.json" entry)"
  [ -n "$ENTRY" ] || ENTRY="index.html"
  case "$ENTRY" in
    /* | *..* | *\\*) die "manifest.json names an unsafe entry file ($ENTRY). Re-export the app." ;;
  esac
  [ -f "$DIR/$ENTRY" ] || die "$ENTRY missing from this folder. Re-export the app."

  with_manifest=1
  if [ -e "$STORE/manifest.json" ]; then
    if ! same_app "$STORE/manifest.json"; then
      echo "Biorouter already has a different app named '$APP_ID'."
      ask_yes "Replace it with this one? It is moved to $LAUNCHER_HOME/backups first." \
        || die "left the installed '$APP_ID' as it is. To replace it, run again with BIOROUTER_EXPORT_REPLACE=1 (it is moved to $LAUNCHER_HOME/backups first)."
      backup="$(backup_dir)" || die "could not create a backup folder in $LAUNCHER_HOME/backups"
      mv "$STORE" "$backup/$APP_ID" || die "could not move the installed '$APP_ID' aside"
      echo "Moved the other '$APP_ID' to $backup"
    else
      case "$(store_age "$STORE/manifest.json")" in
        newer)
          if [ "${BIOROUTER_EXPORT_REPLACE:-}" != "1" ]; then
            echo "The installed '$APP_ID' was changed after this folder was exported, so it is opened as it is."
            echo "To replace it with this folder, run again with BIOROUTER_EXPORT_REPLACE=1 (a copy is kept in $LAUNCHER_HOME/backups)."
            return 0
          fi
          backup_runtime
          ;;
        older) backup_runtime ;;
        # The edit, build, re-run loop. The store keeps its own manifest, which
        # on the author's machine still knows the chat the app was built in.
        *)
          with_manifest=0
          if ! store_unchanged; then
            if [ "${BIOROUTER_EXPORT_REPLACE:-}" != "1" ]; then
              echo "The installed '$APP_ID' was changed where it is installed since this folder was exported or last installed (an edit, or a rebuild by a newer Biorouter), so it is opened as it is."
              echo "To replace it with this folder, run again with BIOROUTER_EXPORT_REPLACE=1 (a copy is kept in $LAUNCHER_HOME/backups)."
              return 0
            fi
            backup_runtime
          fi
          ;;
      esac
    fi
  fi
  copy_runtime "$with_manifest"
  record_install
}

port_alive() { curl -sf -o /dev/null --max-time 1 "http://127.0.0.1:$1/status" 2>/dev/null; }

# ── 3. A daemon whose secret this launcher knows ──────────────────────────
# A daemon serves an app's page only to a browser holding that app's access
# cookie, and the one-time link that sets the cookie is handed only to a caller
# holding the daemon's secret. So this launcher uses a daemon it started (its
# port and secret are kept, readable by you alone, in $LAUNCHER_STATE) or one you
# name with BIOROUTERD_PORT and BIOROUTER_SERVER__SECRET_KEY. It cannot open the
# app on any other daemon, whatever answers /status.
LAUNCHER_STATE="$LAUNCHER_HOME/daemon"
SECRET=""
LOG=""

new_secret() { od -An -N32 -tx1 /dev/urandom | tr -d ' \n'; }

# Every request that carries the secret hands it to curl on stdin, never on a
# command line, where other accounts on this machine can read it.
secret_ok() {
  printf 'X-Secret-Key: %s\n' "$2" \
    | curl -sf -o /dev/null --max-time 2 -H @- "http://127.0.0.1:$1/apps" 2>/dev/null
}

# The path of this app's one-time launch link, from the daemon on :$1.
launch_path() {
  printf 'X-Secret-Key: %s\n' "$2" \
    | curl -sf --max-time 5 -X POST -H @- "http://127.0.0.1:$1/apps/$APP_ID/launch" 2>/dev/null \
    | sed -n 's|.*"path":"\(/apps/[A-Za-z0-9_-]*/?t=[0-9a-f]*\)".*|\1|p'
}

save_daemon() {
  ( umask 077
    mkdir -p "$(dirname "$LAUNCHER_STATE")"
    printf '%s %s\n' "$PORT" "$SECRET" > "$LAUNCHER_STATE.$$" && mv -f "$LAUNCHER_STATE.$$" "$LAUNCHER_STATE"
  ) || echo "  (could not remember this daemon; the next run starts another)" >&2
}

start_daemon() {
  if [ -n "${BIOROUTER_SERVER__SECRET_KEY:-}" ] && [ -n "${BIOROUTERD_PORT:-}" ] \
    && secret_ok "$BIOROUTERD_PORT" "$BIOROUTER_SERVER__SECRET_KEY"; then
    PORT="$BIOROUTERD_PORT"; SECRET="$BIOROUTER_SERVER__SECRET_KEY"
    echo "Using the Biorouter daemon on :$PORT"
    return 0
  fi
  if [ -f "$LAUNCHER_STATE" ]; then
    saved_port=""; saved_secret=""
    read -r saved_port saved_secret < "$LAUNCHER_STATE" || true
    if [ -n "$saved_port" ] && [ -n "$saved_secret" ] && secret_ok "$saved_port" "$saved_secret"; then
      PORT="$saved_port"; SECRET="$saved_secret"
      echo "Using the Biorouter daemon this launcher started on :$PORT"
      return 0
    fi
  fi

  BIOROUTERD="$(find_biorouterd)" || die "$(not_found)"

  # The daemon's log holds what the app's agent did, so it lives in a folder
  # only you can read, never under a shared /tmp name another account can read
  # or pre-create.
  LOGS="$LAUNCHER_HOME/logs"
  ( umask 077; mkdir -p "$LOGS" ) && chmod 700 "$LOGS" || die "could not create $LOGS"

  for p in "${BIOROUTERD_PORT:-3000}" 3001 3002 3003 3004 3005; do
    # A daemon already here is not one whose secret this launcher knows.
    if port_alive "$p"; then continue; fi
    # A new secret for every attempt: until the daemon answers with it, whatever
    # listens on the port is asked with it, and that must never be the secret of a
    # daemon that goes on to start somewhere else.
    SECRET="$(new_secret)"
    [ "${#SECRET}" -eq 64 ] || die "could not make a secret for the daemon (this needs od and /dev/urandom)"
    LOG="$LOGS/biorouterd-$p.log"
    echo "Starting $BIOROUTERD on :$p (using your configured Biorouter provider)..."
    ( umask 077; : > "$LOG" ) || true
    BIOROUTER_SERVER__SECRET_KEY="$SECRET" BIOROUTER_PORT="$p" "$BIOROUTERD" agent </dev/null >>"$LOG" 2>&1 &
    pid=$!
    # A first start can be slow (macOS checks a new binary before it runs, and
    # the daemon opens its stores), so wait 60 seconds, as `biorouter apps open`
    # does.
    for _ in $(seq 1 120); do
      if secret_ok "$p" "$SECRET"; then PORT="$p"; save_daemon; return 0; fi
      kill -0 "$pid" 2>/dev/null || break   # it exited; try the next port
      sleep 0.5
    done
    # Never leave a daemon running whose secret nobody will ever know: it would
    # hold the port for good and serve nobody.
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    echo "  :$p did not come up (see $LOG)" >&2
  done
  die "could not start biorouterd.${LOG:+ Last log: $LOG}"
}

# ── 4. The one-time address that opens this app ───────────────────────────
# Opening it sets the app's access cookie in your browser and lands on the app.
# It works once and for a few minutes; run the launcher again for a new one.
launch_url() {
  path="$(launch_path "$PORT" "$SECRET")"
  [ -n "$path" ] || die "the daemon on :$PORT does not serve '$APP_ID'. Is the store at $STORE?"
  LAUNCH_URL="http://127.0.0.1:$PORT$path"
}

# The link opens the app for whoever uses it FIRST, and a command line is
# readable by every account on this machine (`ps`, /proc/<pid>/cmdline) while
# `open` or `xdg-open` runs. So the link never goes on one: it is written into a
# page only you can read, which sends the browser on to it, and the opener is
# handed that page. Pages older than ten minutes hold expired links and are
# removed here.
LAUNCH_PAGES="$LAUNCHER_HOME/open"

launch_page() {
  ( umask 077
    set -C
    mkdir -p "$LAUNCH_PAGES" || exit 1
    [ -d "$LAUNCH_PAGES" ] && [ ! -L "$LAUNCH_PAGES" ] && [ -O "$LAUNCH_PAGES" ] || exit 1
    chmod 700 "$LAUNCH_PAGES" || exit 1
    find "$LAUNCH_PAGES" -type f -name 'launch-*.html' -mmin +10 -exec rm -f {} + 2>/dev/null || true
    name="$(new_secret)"
    [ "${#name}" -eq 64 ] || exit 1
    page="$LAUNCH_PAGES/launch-${name:0:32}.html"
    printf '%s%s%s%s%s\n' \
      '<!doctype html><meta charset=utf-8><meta name=referrer content=no-referrer><meta http-equiv="refresh" content="0;url=' \
      "$1" '"><title>Opening Biorouter app</title><p>Opening the app. If nothing happens, <a href="' \
      "$1" '">open it here</a>. The address works once.</p>' > "$page" || exit 1
    printf '%s\n' "$page"
  )
}

# Hand the page to the default WEB BROWSER. `open` and `xdg-open` hand it to
# whatever opens .html files, which on a developer's machine is often an
# editor: that shows the page's source, link included, and opens no app.
open_page() {
  case "$(uname -s)" in
    Darwin)
      browser="$(osascript -l JavaScript -e 'ObjC.import("AppKit"); var u = $.NSWorkspace.sharedWorkspace.URLForApplicationToOpenURL($.NSURL.URLWithString("https://example.com")); u.isNil() ? "" : u.path.js' 2>/dev/null || true)"
      if [ -n "$browser" ] && [ -d "$browser" ]; then
        open -a "$browser" "$1" && return 0
      fi
      open "$1"
      ;;
    *)
      desktop="$(xdg-settings get default-web-browser 2>/dev/null || true)"
      if [ -n "$desktop" ] && command -v gtk-launch >/dev/null 2>&1; then
        gtk-launch "${desktop%.desktop}" "$1" >/dev/null 2>&1 && return 0
      fi
      command -v xdg-open >/dev/null 2>&1 && xdg-open "$1"
      ;;
  esac
}

open_url() {
  echo "Opening $APP_ID at http://127.0.0.1:$PORT/apps/$APP_ID/"
  token="${1#"http://127.0.0.1:$PORT/apps/$APP_ID/?t="}"
  # The digits and letters are listed, not written as ranges: macOS's bash 3.2
  # reads [0-9a-f] by the locale's collation under UTF-8, where it takes
  # upper-case letters too.
  case "$token" in
    "$1" | *[!0123456789abcdef]*) die "not a launch link for $APP_ID; refusing to open it" ;;
  esac
  [ "${#token}" -eq 64 ] || die "not a launch link for $APP_ID; refusing to open it"
  page="$(launch_page "$1")" || page=""
  opened=0
  if [ -n "$page" ]; then
    open_page "$page" && opened=1
  fi
  # This terminal is yours alone, so the link can be shown here: for a browser
  # that cannot read the page (a snap-packaged one cannot read hidden folders),
  # or for when none opened.
  if [ "$opened" = 1 ]; then
    echo "If the app does not appear, open this address instead (it works once): $1"
  else
    echo "Open this address in your browser (it works once): $1"
  fi
}

# ── 5. First-run payload install (full-mode exports only) ─────────────────
# A "full" export carries the app's server-side dependencies under payload/:
# knowledge bases (.brkb archives) and skills (plain directory trees), listed in
# export.json at the top of this folder. On the FIRST run they are imported
# into your store, guarded by a marker so re-runs are no-ops. A launcher-mode
# export has no payload/ dir, so this is a clean no-op there.
#
# Consent: the payload list is printed and an interactive y/N confirmation is
# required. Set BIOROUTER_EXPORT_YES=1 to skip the prompt (CI / headless).
#
# ⚠ The lists are bash arrays, expanded quoted. They were space-separated
# strings, so a folder unzipped under "My Apps" split into "My" and "Apps/...":
# the prompt named a skill "My", and the install copied whatever "My" was.
install_payload() {
  [ -d "$DIR/payload" ] || return 0                     # launcher-mode export: nothing to install
  MARKER="$CFG/.apps-payload-installed-$APP_ID"
  [ -f "$MARKER" ] && return 0                          # already installed on a previous run

  # Collect what there is to install (knowledge bases + skills). A payload that
  # only carries a bundled daemon (payload/bin) has nothing to import here.
  kbs=(); skills=(); nkb=0; nskill=0
  if [ -d "$DIR/payload/knowledge" ]; then
    for f in "$DIR/payload/knowledge/"*.brkb; do
      if [ -f "$f" ] && [ ! -L "$f" ]; then kbs+=("$f"); nkb=$((nkb + 1)); fi
    done
  fi
  if [ -d "$DIR/payload/skills" ]; then
    for d in "$DIR/payload/skills/"*/; do
      d="${d%/}"
      name="${d##*/}"
      case "$name" in '' | .* | *[!A-Za-z0-9._-]*) continue ;; esac
      # A skill Biorouter ships is seeded by your own install; one copied over
      # it could no longer be removed. Exports leave them out; so does this.
      case "$name" in
        __SHIPPED_SKILLS__) echo "  (skipping skill $name: it ships with Biorouter)"; continue ;;
      esac
      if [ -d "$d" ] && [ ! -L "$d" ]; then skills+=("$d"); nskill=$((nskill + 1)); fi
    done
  fi
  [ "$nkb" -eq 0 ] && [ "$nskill" -eq 0 ] && return 0

  echo ""
  echo "This app ships a payload to install into your Biorouter (see export.json):"
  for f in ${kbs[@]+"${kbs[@]}"}; do echo "  - knowledge base: ${f##*/}"; done
  for d in ${skills[@]+"${skills[@]}"}; do echo "  - skill: ${d##*/}"; done

  if [ "${BIOROUTER_EXPORT_YES:-}" != "1" ]; then
    printf "Install this payload now? [y/N] "
    read -r reply || reply=""
    case "$reply" in
      y|Y|yes|YES) ;;
      *) echo "Skipping payload install. Re-run to install it later."; return 0 ;;
    esac
  fi

  mkdir -p "$CFG/skills"
  # Knowledge bases: the `biorouter` CLI has no `knowledge import` subcommand as
  # of the 2026-07 tree, so each .brkb is staged under knowledge-imports/ and you
  # are pointed at the Knowledge panel's importer. If a future CLI grows
  # `knowledge import`, it is preferred.
  for f in ${kbs[@]+"${kbs[@]}"}; do
    if command -v biorouter >/dev/null 2>&1 && biorouter knowledge import --help >/dev/null 2>&1; then
      biorouter knowledge import "$f" && echo "  - imported ${f##*/}" || echo "  ! import failed for ${f##*/}"
    else
      mkdir -p "$CFG/knowledge-imports"
      cp "$f" "$CFG/knowledge-imports/"
      echo "  - staged ${f##*/} in $CFG/knowledge-imports/ (import it from Biorouter's Knowledge panel)"
    fi
  done
  # Skills install straight into the skills dir the daemon already reads. One
  # you already have under the same name is moved to backups, never deleted.
  for d in ${skills[@]+"${skills[@]}"}; do
    name="${d##*/}"
    dest="$CFG/skills/$name"
    if [ -e "$dest" ] || [ -L "$dest" ]; then
      backup="$(backup_dir)" || die "could not create a backup folder in $LAUNCHER_HOME/backups"
      mv "$dest" "$backup/$name" || die "could not move your installed skill $name aside"
      echo "  - moved your installed skill $name to $backup"
    fi
    cp -R "$d" "$dest" || die "could not install skill $name"
    echo "  - installed skill $name"
  done

  mkdir -p "$CFG"
  : > "$MARKER"
  echo "Payload installed."
}
"##;

/// A double-clickable launcher (`run.command` on macOS / `run.sh` elsewhere).
///
/// Serving the page *from the daemon* is what makes this robust: the SDK derives
/// its WebSocket endpoint from the page origin, so the app connects no matter
/// which port the daemon ended up on. Needs no Node — `dist/app.js` ships
/// prebuilt.
fn run_script() -> String {
    r#"#!/usr/bin/env bash
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=biorouter-launch.sh
. "$DIR/biorouter-launch.sh"

install_app
install_payload          # no-op for launcher-mode exports (no payload/ dir)
start_daemon
launch_url
open_url "$LAUNCH_URL"
"#
    .to_string()
}

/// The Windows launcher (`run.ps1`), invoked by `run.bat`. Mirrors
/// `biorouter-launch.sh` + `run.sh`: locate a `biorouterd` (a bundled
/// `payload\bin` one first, when it runs here; then PATH and the install
/// locations), install the app into the store the daemon reads under the same
/// never-over-another-app rules, run the first-run payload install (with
/// consent), start the daemon headlessly, and open the app in the default
/// browser. The page is served *by* the daemon, so the SDK derives its
/// WebSocket endpoint — and the per-app token, which rides the query string —
/// from the page origin; nothing is hard-coded.
///
/// Built with `.replace` rather than `format!` so the many PowerShell braces
/// need no doubling.
fn run_ps1(id: &str, export_digest: &str) -> String {
    // App ids are slugified (a-z0-9-), so they are safe to embed in a
    // double-quoted PowerShell string literal without escaping; the digest is
    // hex and the skill names are `[a-z-]`.
    let shipped: Vec<String> = SHIPPED_SKILL_NAMES
        .iter()
        .map(|name| format!("\"{name}\""))
        .collect();
    RUN_PS1_TEMPLATE
        .replace("__APP_ID__", id)
        .replace("__EXPORT_DIGEST__", export_digest)
        .replace("__SHIPPED_SKILLS__", &shipped.join(", "))
}

const RUN_PS1_TEMPLATE: &str = r##"#Requires -Version 5.0
# Windows launcher for the exported Biorouter app "__APP_ID__".
$ErrorActionPreference = "Stop"
$AppId = "__APP_ID__"
$Dir   = Split-Path -Parent $MyInvocation.MyCommand.Definition
# Exactly the folder the daemon reads (biorouter-mcp `paths::config_dir`):
# BIOROUTER_PATH_ROOT\config when that is set, else %APPDATA%\Block\biorouter\config.
# This used to be a .config folder in your profile, which no Windows daemon
# reads, so every launch failed with "does not serve".
if ($env:BIOROUTER_PATH_ROOT -and $env:BIOROUTER_PATH_ROOT.Trim()) {
  $Cfg = Join-Path $env:BIOROUTER_PATH_ROOT "config"
} elseif ($env:APPDATA) {
  $Cfg = Join-Path $env:APPDATA "Block\biorouter\config"
} else {
  $Cfg = Join-Path $HOME "AppData\Roaming\Block\biorouter\config"
}
$Store = Join-Path $Cfg "agent_drafter\$AppId"
$LauncherHome = Join-Path $Cfg "app-launcher"
$script:BundledFailed = $false

function Find-Biorouterd {
  if ($env:BIOROUTERD_BIN -and (Test-Path -LiteralPath $env:BIOROUTERD_BIN)) { return $env:BIOROUTERD_BIN }
  # A fat export's bundled daemon, only when it runs here: one built for another
  # platform cannot start, and an installed biorouterd should be used instead.
  $bundled = Join-Path $Dir "payload\bin\biorouterd.exe"
  if (Test-Path -LiteralPath $bundled) {
    try { & $bundled --version *> $null; if ($LASTEXITCODE -eq 0) { return $bundled } } catch { }
    $script:BundledFailed = $true
  }
  $cmd = Get-Command biorouterd -ErrorAction SilentlyContinue
  if ($cmd) { return $cmd.Source }
  if ($env:LOCALAPPDATA) {
    $programs = Join-Path $env:LOCALAPPDATA "Programs\Biorouter\resources\bin\biorouterd.exe"
    if (Test-Path -LiteralPath $programs) { return $programs }
    # The Squirrel installer's layout: %LOCALAPPDATA%\biorouter_app\app-<version>\.
    $squirrel = Join-Path $env:LOCALAPPDATA "biorouter_app"
    if (Test-Path -LiteralPath $squirrel) {
      $found = Get-ChildItem -LiteralPath $squirrel -Directory -Filter "app-*" -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending |
        ForEach-Object { Join-Path $_.FullName "resources\bin\biorouterd.exe" } |
        Where-Object { Test-Path -LiteralPath $_ } |
        Select-Object -First 1
      if ($found) { return $found }
    }
  }
  $programFiles = "C:\Program Files\Biorouter\resources\bin\biorouterd.exe"
  if (Test-Path -LiteralPath $programFiles) { return $programFiles }
  if ($script:BundledFailed) {
    throw "the biorouterd bundled in payload\bin does not run on this computer, and no installed biorouterd was found. Install Biorouter, or set BIOROUTERD_BIN."
  }
  throw "biorouterd not found. Install Biorouter, or set BIOROUTERD_BIN."
}

function Read-Manifest([string]$Path) {
  try { return (Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json) } catch { return $null }
}

# The same app: the same app_uid, or for apps made before it existed, the same
# creation time.
function Test-SameApp($Ours, $Theirs) {
  if ($Ours.app_uid -or $Theirs.app_uid) { return ([string]$Ours.app_uid -eq [string]$Theirs.app_uid) }
  return ($Ours.created_at -and ([string]$Ours.created_at -eq [string]$Theirs.created_at))
}

# Yes only from BIOROUTER_EXPORT_REPLACE=1, or from a person at the console.
function Confirm-Replace([string]$Question) {
  if ($env:BIOROUTER_EXPORT_REPLACE -eq "1") { return $true }
  if (-not [Environment]::UserInteractive) { return $false }
  $reply = Read-Host "$Question [y/N]"
  return ($reply -match '^(y|Y|yes|YES)$')
}

function New-Backup {
  $stamp = (Get-Date -Format "yyyyMMdd-HHmmss") + "-" + $PID
  $dir = Join-Path $LauncherHome ("backups\" + $AppId + "-" + $stamp)
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  return $dir
}

function Backup-Runtime([string]$Entry) {
  $backup = New-Backup
  foreach ($item in @("manifest.json", $Entry, "src", "dist", "assets")) {
    $p = Join-Path $Store $item
    if (Test-Path -LiteralPath $p) { Copy-Item -LiteralPath $p -Destination $backup -Recurse -Force }
  }
  Write-Host "Kept a copy of the installed '$AppId' in $backup"
}

# What the store's copy held when this folder was exported, and where each
# install records what it put there. Mirrors bash `runtime_digest`: equal
# updated_at values do not mean an unchanged store, since only the drafter's own
# tools bump it.
$ExportDigest = "__EXPORT_DIGEST__"
$InstalledRecord = Join-Path $LauncherHome "installed\$AppId"

# One SHA-256 over the files an install replaces in app folder $Root: the entry
# page, src\, assets\, and dist\ only for an app with no src\main.ts. One
# "<hash> <path>" line per file (paths with /), sorted by ordinal, then hashed.
# Exactly the export's `render::runtime_digest`; "" when it cannot tell.
function Get-RuntimeDigest([string]$Root, [string]$Entry) {
  try {
    if (-not (Test-Path -LiteralPath $Root -PathType Container)) { return "" }
    $base = (Get-Item -LiteralPath $Root -Force).FullName.TrimEnd('\', '/')
    $plain = { param($item) $item -and -not ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) }
    $files = New-Object 'System.Collections.Generic.HashSet[string]'
    $entryItem = Get-Item -LiteralPath (Join-Path $Root $Entry) -Force -ErrorAction SilentlyContinue
    if ((& $plain $entryItem) -and -not $entryItem.PSIsContainer) { [void]$files.Add(($Entry -replace '\\', '/')) }
    $mainItem = Get-Item -LiteralPath (Join-Path $Root "src\main.ts") -Force -ErrorAction SilentlyContinue
    $builtFromSource = (& $plain $mainItem) -and -not $mainItem.PSIsContainer
    foreach ($sub in @("src", "assets", "dist")) {
      if ($sub -eq "dist" -and $builtFromSource) { continue }
      $dirItem = Get-Item -LiteralPath (Join-Path $Root $sub) -Force -ErrorAction SilentlyContinue
      if (-not (& $plain $dirItem) -or -not $dirItem.PSIsContainer) { continue }
      Get-ChildItem -LiteralPath $dirItem.FullName -Recurse -File -Force -ErrorAction Stop |
        Where-Object { (& $plain $_) -and $_.Name -ne ".DS_Store" } |
        ForEach-Object { [void]$files.Add(($_.FullName.Substring($base.Length + 1) -replace '\\', '/')) }
    }
    $lines = New-Object 'System.Collections.Generic.List[string]'
    foreach ($rel in $files) {
      $hash = (Get-FileHash -LiteralPath (Join-Path $Root $rel) -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()
      $lines.Add("$hash $rel`n")
    }
    $sorted = $lines.ToArray()
    [Array]::Sort($sorted, [StringComparer]::Ordinal)
    $sha = [Security.Cryptography.SHA256]::Create()
    $digest = $sha.ComputeHash([Text.Encoding]::UTF8.GetBytes(($sorted -join "")))
    return (($digest | ForEach-Object { $_.ToString("x2") }) -join "")
  } catch { return "" }
}

# Is the installed copy still what this folder was exported from, or what a
# launcher last installed? Anything else was changed where it is installed.
function Test-StoreUnchanged([string]$Entry) {
  $current = Get-RuntimeDigest $Store $Entry
  if (-not $current) { return $false }
  if ($current -eq $ExportDigest) { return $true }
  if (Test-Path -LiteralPath $InstalledRecord) {
    $recorded = (Get-Content -LiteralPath $InstalledRecord -TotalCount 1 -ErrorAction SilentlyContinue)
    if ($recorded -and ([string]$recorded).Trim() -eq $current) { return $true }
  }
  return $false
}

function Write-InstallRecord([string]$Entry) {
  try {
    $digest = Get-RuntimeDigest $Store $Entry
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $InstalledRecord) | Out-Null
    if ($digest) { Set-Content -LiteralPath $InstalledRecord -Value $digest -Encoding ASCII }
    elseif (Test-Path -LiteralPath $InstalledRecord) { Remove-Item -LiteralPath $InstalledRecord -Force }
  } catch { }
}

# Install the runtime files into the store the daemon reads, never over another
# app: a different app under this id is moved to backups only when you say so
# (or BIOROUTER_EXPORT_REPLACE=1); this app changed since the export (by the
# drafter, or where it is installed) is opened as it is; an older copy of this
# app is replaced with a backup kept; and an unchanged one is re-synced (the
# edit, build, re-run loop).
function Install-App {
  $manifestPath = Join-Path $Dir "manifest.json"
  if (-not (Test-Path -LiteralPath $manifestPath)) { throw "manifest.json missing from this folder - re-export the app." }
  $ours = Read-Manifest $manifestPath
  if (-not $ours) { throw "manifest.json in this folder could not be read - re-export the app." }
  $entry = if ($ours.entry) { [string]$ours.entry } else { "index.html" }
  if ($entry -match '^[\\/]' -or $entry -match '\.\.' -or $entry -match ':') {
    throw "manifest.json names an unsafe entry file ($entry) - re-export the app."
  }
  $entrySrc = Join-Path $Dir $entry
  if (-not (Test-Path -LiteralPath $entrySrc)) { throw "$entry missing from this folder - re-export the app." }

  $withManifest = $true
  $theirsPath = Join-Path $Store "manifest.json"
  if (Test-Path -LiteralPath $theirsPath) {
    $theirs = Read-Manifest $theirsPath
    if (-not $theirs -or -not (Test-SameApp $ours $theirs)) {
      Write-Host "Biorouter already has a different app named '$AppId'."
      if (-not (Confirm-Replace "Replace it with this one? It is moved to $LauncherHome\backups first.")) {
        throw "left the installed '$AppId' as it is. To replace it, run again with BIOROUTER_EXPORT_REPLACE=1."
      }
      $backup = New-Backup
      Move-Item -LiteralPath $Store -Destination (Join-Path $backup $AppId)
      Write-Host "Moved the other '$AppId' to $backup"
    } elseif ([int64]$theirs.updated_at -gt [int64]$ours.updated_at) {
      if ($env:BIOROUTER_EXPORT_REPLACE -ne "1") {
        Write-Host "The installed '$AppId' was changed after this folder was exported, so it is opened as it is."
        Write-Host "To replace it with this folder, run again with BIOROUTER_EXPORT_REPLACE=1."
        return
      }
      Backup-Runtime $entry
    } elseif ([int64]$theirs.updated_at -lt [int64]$ours.updated_at) {
      Backup-Runtime $entry
    } else {
      $withManifest = $false
      if (-not (Test-StoreUnchanged $entry)) {
        if ($env:BIOROUTER_EXPORT_REPLACE -ne "1") {
          Write-Host "The installed '$AppId' was changed where it is installed since this folder was exported or last installed (an edit, or a rebuild by a newer Biorouter), so it is opened as it is."
          Write-Host "To replace it with this folder, run again with BIOROUTER_EXPORT_REPLACE=1."
          return
        }
        Backup-Runtime $entry
      }
    }
  }
  New-Item -ItemType Directory -Force -Path $Store | Out-Null
  $entryDst = Join-Path $Store $entry
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $entryDst) | Out-Null
  Copy-Item -LiteralPath $entrySrc -Destination $entryDst -Force
  foreach ($sub in @("src", "dist", "assets")) {
    $src = Join-Path $Dir $sub
    # A kept store manifest's build stamp describes the store's own bundle, so
    # an app the daemon builds keeps its dist\ and is rebuilt from src\.
    if ($sub -eq "dist" -and -not $withManifest -and (Test-Path -LiteralPath (Join-Path $Dir "src\main.ts"))) { continue }
    if (Test-Path -LiteralPath $src) {
      $dst = Join-Path $Store $sub
      $fresh = Join-Path $Store ".$sub.new"
      if (Test-Path -LiteralPath $fresh) { Remove-Item -LiteralPath $fresh -Recurse -Force }
      Copy-Item -LiteralPath $src -Destination $fresh -Recurse -Force
      if (Test-Path -LiteralPath $dst) { Remove-Item -LiteralPath $dst -Recurse -Force }
      Move-Item -LiteralPath $fresh -Destination $dst
    }
  }
  # The manifest last: it is what makes the daemon see the app.
  if ($withManifest) { Copy-Item -LiteralPath $manifestPath -Destination $theirsPath -Force }
  Write-InstallRecord $entry
}

# First-run payload install (full-mode exports). Mirrors bash install_payload:
# import knowledge bases + skills once, guarded by a marker, consent required
# (set BIOROUTER_EXPORT_YES=1 to skip the prompt).
function Install-Payload {
  $payload = Join-Path $Dir "payload"
  if (-not (Test-Path -LiteralPath $payload)) { return }    # launcher-mode export
  $marker = Join-Path $Cfg ".apps-payload-installed-$AppId"
  if (Test-Path -LiteralPath $marker) { return }            # already installed

  $kbs = @(); $skills = @()
  $kdir = Join-Path $payload "knowledge"
  if (Test-Path -LiteralPath $kdir) { $kbs = @(Get-ChildItem -LiteralPath $kdir -Filter *.brkb -File -ErrorAction SilentlyContinue) }
  $sdir = Join-Path $payload "skills"
  if (Test-Path -LiteralPath $sdir) {
    $skills = @(Get-ChildItem -LiteralPath $sdir -Directory -ErrorAction SilentlyContinue |
      Where-Object { -not ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -and $_.Name -match '^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$' })
    # A skill Biorouter ships is seeded by your own install; one copied over it
    # could no longer be removed. Exports leave them out; so does this.
    $shipped = @(__SHIPPED_SKILLS__)
    $skills = @($skills | Where-Object {
      if ($shipped -contains $_.Name) { Write-Host "  (skipping skill $($_.Name): it ships with Biorouter)"; $false } else { $true }
    })
  }
  if ($kbs.Count -eq 0 -and $skills.Count -eq 0) { return }

  Write-Host ""
  Write-Host "This app ships a payload to install into your Biorouter (see export.json):"
  foreach ($f in $kbs)    { Write-Host "  - knowledge base: $($f.Name)" }
  foreach ($d in $skills) { Write-Host "  - skill: $($d.Name)" }

  if ($env:BIOROUTER_EXPORT_YES -ne "1") {
    $reply = Read-Host "Install this payload now? [y/N]"
    if ($reply -notmatch '^(y|Y|yes|YES)$') {
      Write-Host "Skipping payload install. Re-run to install it later."
      return
    }
  }

  New-Item -ItemType Directory -Force -Path (Join-Path $Cfg "skills") | Out-Null
  # No `biorouter knowledge import` CLI subcommand exists yet (2026-07 tree), so
  # stage each .brkb and point the user at the Knowledge panel importer. Prefer
  # the CLI if a future version grows the command.
  foreach ($f in $kbs) {
    $bio = Get-Command biorouter -ErrorAction SilentlyContinue
    $canImport = $false
    if ($bio) { & $bio.Source knowledge import --help *> $null; $canImport = ($LASTEXITCODE -eq 0) }
    if ($canImport) {
      & $bio.Source knowledge import $f.FullName
      Write-Host "  - imported $($f.Name)"
    } else {
      $imp = Join-Path $Cfg "knowledge-imports"
      New-Item -ItemType Directory -Force -Path $imp | Out-Null
      Copy-Item -LiteralPath $f.FullName -Destination $imp -Force
      Write-Host "  - staged $($f.Name) in $imp (import it from Biorouter's Knowledge panel)"
    }
  }
  # A skill you already have under the same name is moved to backups, never deleted.
  foreach ($d in $skills) {
    $dst = Join-Path (Join-Path $Cfg "skills") $d.Name
    if (Test-Path -LiteralPath $dst) {
      $backup = New-Backup
      Move-Item -LiteralPath $dst -Destination (Join-Path $backup $d.Name)
      Write-Host "  - moved your installed skill $($d.Name) to $backup"
    }
    Copy-Item -LiteralPath $d.FullName -Destination $dst -Recurse -Force
    Write-Host "  - installed skill $($d.Name)"
  }

  New-Item -ItemType Directory -Force -Path $Cfg | Out-Null
  New-Item -ItemType File -Force -Path $marker | Out-Null
  Write-Host "Payload installed."
}

function Test-Port([int]$Port) {
  try {
    $r = Invoke-WebRequest -Uri "http://127.0.0.1:$Port/status" -TimeoutSec 1 -UseBasicParsing
    return ($r.StatusCode -eq 200)
  } catch { return $false }
}

# A daemon serves an app's page only to a browser holding that app's access
# cookie, and the one-time link that sets it is handed only to a caller holding
# the daemon's secret. So this launcher uses a daemon it started (its port and
# secret are kept in $StateFile, under your profile) or one you name with
# BIOROUTERD_PORT and BIOROUTER_SERVER__SECRET_KEY, and no other.
$StateFile = Join-Path $LauncherHome "daemon"

function New-Secret {
  $bytes = New-Object byte[] 32
  $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
  try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
  return (($bytes | ForEach-Object { $_.ToString("x2") }) -join "")
}

function Test-Secret([int]$Port, [string]$Secret) {
  try {
    $r = Invoke-WebRequest -Uri "http://127.0.0.1:$Port/apps" -Headers @{ "X-Secret-Key" = $Secret } -TimeoutSec 2 -UseBasicParsing
    return ($r.StatusCode -eq 200)
  } catch { return $false }
}

function Save-Daemon([int]$Port, [string]$Secret) {
  try {
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $StateFile) | Out-Null
    Set-Content -Path $StateFile -Value "$Port $Secret" -NoNewline
  } catch { Write-Host "  (could not remember this daemon; the next run starts another)" }
}

# Use a daemon whose secret is known, else start one headlessly with a new secret.
function Start-Daemon {
  if ($env:BIOROUTER_SERVER__SECRET_KEY -and $env:BIOROUTERD_PORT -and
      (Test-Secret ([int]$env:BIOROUTERD_PORT) $env:BIOROUTER_SERVER__SECRET_KEY)) {
    Write-Host "Using the Biorouter daemon on :$env:BIOROUTERD_PORT"
    return @([int]$env:BIOROUTERD_PORT, [string]$env:BIOROUTER_SERVER__SECRET_KEY)
  }
  if (Test-Path $StateFile) {
    try {
      $saved = ((Get-Content $StateFile -Raw) -as [string]).Trim() -split "\s+"
      if ($saved.Count -eq 2 -and (Test-Secret ([int]$saved[0]) $saved[1])) {
        Write-Host "Using the Biorouter daemon this launcher started on :$($saved[0])"
        return @([int]$saved[0], [string]$saved[1])
      }
    } catch { }  # an unreadable record: start a daemon of our own
  }
  $bin = Find-Biorouterd
  $preferred = if ($env:BIOROUTERD_PORT) { [int]$env:BIOROUTERD_PORT } else { 3000 }
  foreach ($p in @($preferred, 3001, 3002, 3003, 3004, 3005)) {
    # A daemon already here is not one whose secret this launcher knows.
    if (Test-Port $p) { continue }
    # A new secret for every attempt, so one asked of whatever held the port
    # never becomes the secret of a daemon started elsewhere.
    $secret = New-Secret
    Write-Host "Starting $bin on :$p ..."
    $env:BIOROUTER_PORT = "$p"
    $env:BIOROUTER_SERVER__SECRET_KEY = $secret
    $proc = Start-Process -FilePath $bin -ArgumentList "agent" -WindowStyle Hidden -PassThru
    # Only the daemon needs it: the browser started below inherits this
    # process's environment, so the secret must not stay in it.
    Remove-Item Env:BIOROUTER_SERVER__SECRET_KEY -ErrorAction SilentlyContinue
    # A first start can be slow, so wait 60 seconds, as `biorouter apps open` does.
    for ($i = 0; $i -lt 120; $i++) {
      if (Test-Secret $p $secret) { Save-Daemon $p $secret; return @($p, $secret) }
      if ($proc.HasExited) { break }
      Start-Sleep -Milliseconds 500
    }
    # Never leave a daemon running whose secret nobody will ever know.
    if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue }
    Write-Host "  :$p did not come up"
  }
  throw "could not start biorouterd."
}

Install-App
Install-Payload
$Daemon = Start-Daemon
$Port = [int]$Daemon[0]
$Secret = [string]$Daemon[1]
$Base = "http://127.0.0.1:$Port"
# The app's one-time launch link: opening it sets the app's access cookie in the
# browser and lands on the app. It works once and for a few minutes.
$LaunchPath = $null
try {
  $LaunchPath = [string](Invoke-RestMethod -Method Post -Uri "$Base/apps/$AppId/launch" -Headers @{ "X-Secret-Key" = $Secret } -TimeoutSec 5).path
} catch { $LaunchPath = $null }
if (-not $LaunchPath -or $LaunchPath -notmatch '^/apps/[A-Za-z0-9_-]+/\?t=[0-9a-f]{64}$') {
  throw "the daemon on :$Port does not serve '$AppId'."
}
Write-Host "Opening $AppId at $Base/apps/$AppId/"
# The link opens the app for whoever uses it first, so it never goes on a
# command line: it is written into a page under your profile, which only you can
# read, and the page sends the browser on to it. Pages older than ten minutes
# hold expired links and are removed here.
$Link = "$Base$LaunchPath"
$Pages = Join-Path $LauncherHome "open"
New-Item -ItemType Directory -Force -Path $Pages | Out-Null
Get-ChildItem -Path $Pages -Filter "launch-*.html" -File -ErrorAction SilentlyContinue |
  Where-Object { $_.LastWriteTime -lt (Get-Date).AddMinutes(-10) } |
  Remove-Item -Force -ErrorAction SilentlyContinue
$Page = Join-Path $Pages ("launch-" + (New-Secret).Substring(0, 32) + ".html")
$Html = '<!doctype html><meta charset=utf-8><meta name=referrer content=no-referrer>' +
  '<meta http-equiv="refresh" content="0;url=' + $Link + '"><title>Opening Biorouter app</title>' +
  '<p>Opening the app. If nothing happens, <a href="' + $Link + '">open it here</a>. The address works once.</p>'
try {
  Set-Content -LiteralPath $Page -Value $Html -Encoding UTF8
  Invoke-Item -LiteralPath $Page
  # This window is yours alone, so the link can be shown here too.
  Write-Host "If the app does not appear, open this address instead (it works once): $Link"
} catch {
  Write-Host "Open this address in your browser (it works once): $Link"
}
"##;

/// The thin `run.bat` wrapper: double-clickable on Windows, it just runs
/// `run.ps1` with an unrestricted policy for this one invocation. Kept as a
/// separate file so Explorer shows a runnable `.bat` (a bare `.ps1` opens in an
/// editor by default).
fn run_bat() -> String {
    "@echo off\r\nrem Windows launcher for the exported Biorouter app.\r\npowershell -NoProfile -ExecutionPolicy Bypass -File \"%~dp0run.ps1\"\r\n".to_string()
}

/// The dev server (`npm start`): installs the app into the local store under
/// the launchers' rules, starts or reuses a daemon, and proxies `/apps/**` to
/// it — including the agent WebSocket — so the page and its agent share an
/// origin and no port is ever hard-coded.
///
/// The page itself is always the DAEMON's (only the daemon can put this run's
/// socket token into it), so `/` redirects there. The app's own `dist/` and
/// `assets/` come from this folder, so a rebuilt bundle shows up on refresh.
/// Nothing else in the folder is served (never `payload/`, `export.json` or the
/// scripts), and only to a `Host` of this server's own loopback address, which
/// closes DNS rebinding.
fn serve_mjs(id: &str, default_port: u16, export_digest: &str) -> String {
    SERVE_MJS_TEMPLATE
        .replace("__EXPORT_DIGEST__", export_digest)
        .replace(
            "__APP_ID_JSON__",
            &serde_json::to_string(id).unwrap_or_else(|_| "\"app\"".to_string()),
        )
        .replace("__APP_ID__", id)
        .replace("__DEFAULT_PORT__", &default_port.to_string())
}

const SERVE_MJS_TEMPLATE: &str = r##"// Dev server for the exported Biorouter app "__APP_ID__".
//
//   node serve.mjs          # installs the app, starts/reuses a daemon, serves + proxies
//   PORT=9000 node serve.mjs
//   BIOROUTERD_BIN=... BIOROUTERD_PORT=... node serve.mjs
//
// The app's page is served by the Biorouter daemon (only it can put this run's
// socket token into the page), through this server: /apps/** is proxied to the
// daemon, including the agent WebSocket at /apps/<id>/agent, so the page and its
// agent share an origin and no port is ever hard-coded. The app's dist/ and
// assets/ come from this folder instead, so `npm run build` shows on refresh.
import { createServer, request as httpRequest } from "node:http";
import { spawn, spawnSync } from "node:child_process";
import { readFile, writeFile, rename, mkdir, cp, rm, access } from "node:fs/promises";
import { readdirSync, statSync, lstatSync, readFileSync, constants } from "node:fs";
import { randomBytes, createHash } from "node:crypto";
import { extname, resolve, join, dirname, relative, isAbsolute, win32, posix } from "node:path";
import { fileURLToPath } from "node:url";
import { homedir } from "node:os";
import { createInterface } from "node:readline/promises";

const APP_ID = __APP_ID_JSON__;
const PORT = Number(process.env.PORT || __DEFAULT_PORT__);
const ROOT = dirname(fileURLToPath(import.meta.url));

// biorouter:config-dir:start
/**
 * Exactly the folder the daemon reads (biorouter-mcp `paths::config_dir`):
 * BIOROUTER_PATH_ROOT/config when that is set; on Windows %APPDATA%\Block\biorouter\config;
 * elsewhere $XDG_CONFIG_HOME/biorouter when that is absolute, else ~/.config/biorouter.
 */
function configDir(env = process.env, platform = process.platform, home = homedir()) {
  const paths = platform === "win32" ? win32 : posix;
  const root = env.BIOROUTER_PATH_ROOT;
  if (root && root.trim()) return paths.join(root, "config");
  if (platform === "win32") {
    const roaming = env.APPDATA || paths.join(home, "AppData", "Roaming");
    return paths.join(roaming, "Block", "biorouter", "config");
  }
  const xdg = env.XDG_CONFIG_HOME;
  const base = xdg && paths.isAbsolute(xdg) ? xdg : paths.join(home, ".config");
  return paths.join(base, "biorouter");
}
// biorouter:config-dir:end

const CFG = configDir();
const STORE = join(CFG, "agent_drafter", APP_ID);
const LAUNCHER_HOME = join(CFG, "app-launcher");
// A daemon serves an app's page only to a browser holding that app's access
// cookie, and the one-time link that sets it is handed only to a caller holding
// the daemon's secret. So this server uses a daemon it (or run.sh) started, whose
// port and secret are kept here readable by you alone, or one you name with
// BIOROUTERD_PORT and BIOROUTER_SERVER__SECRET_KEY, and no other.
const LAUNCHER_STATE = join(LAUNCHER_HOME, "daemon");
const APP_PREFIX = `/apps/${APP_ID}/`;
const MIME = { ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8", ".json": "application/json", ".svg": "image/svg+xml",
  ".png": "image/png", ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".gif": "image/gif",
  ".webp": "image/webp", ".ico": "image/x-icon", ".woff": "font/woff", ".woff2": "font/woff2",
  ".wasm": "application/wasm", ".map": "application/json" };

const exists = (p) => access(p, constants.F_OK).then(() => true, () => false);
const readManifest = (p) => readFile(p, "utf8").then((t) => JSON.parse(t), () => null);

let ENTRY = "index.html";
let installedFromHere = false;

// What the store's copy held when this folder was exported, and where each
// install records what it put there. Equal updated_at values do not mean an
// unchanged store: only the drafter's own tools bump it, so an edit made with
// the developer shell or a text editor, or a newer Biorouter refreshing
// src/sdk.ts on a rebuild, left it equal, and re-syncing reverted the edit.
const EXPORT_DIGEST = "__EXPORT_DIGEST__";
const INSTALLED_RECORD = join(LAUNCHER_HOME, "installed", APP_ID);

// biorouter:runtime-digest:start
/**
 * One SHA-256 over the files an install replaces in app folder `root`: the
 * entry page, src/, assets/, and dist/ only for an app with no src/main.ts.
 * One "<hash> <path>" line per file, sorted by byte, then hashed. Exactly the
 * export's `render::runtime_digest`; "" when a file cannot be read.
 */
function runtimeDigest(root, entry) {
  try {
    const kind = (rel) => {
      try {
        return lstatSync(join(root, ...rel.split("/")));
      } catch {
        return null;
      }
    };
    const files = new Set();
    if (kind(entry)?.isFile()) files.add(entry);
    const walk = (rel) => {
      for (const item of readdirSync(join(root, ...rel.split("/")), { withFileTypes: true })) {
        const path = `${rel}/${item.name}`;
        if (item.isDirectory()) walk(path);
        else if (item.isFile() && item.name !== ".DS_Store") files.add(path);
      }
    };
    const builtFromSource = kind("src/main.ts")?.isFile();
    for (const sub of ["src", "assets", "dist"]) {
      if (sub === "dist" && builtFromSource) continue;
      if (kind(sub)?.isDirectory()) walk(sub);
    }
    const lines = [...files].map((rel) => {
      const hash = createHash("sha256").update(readFileSync(join(root, ...rel.split("/")))).digest("hex");
      return Buffer.from(`${hash} ${rel}\n`);
    });
    lines.sort(Buffer.compare);
    return createHash("sha256").update(Buffer.concat(lines)).digest("hex");
  } catch {
    return "";
  }
}
// biorouter:runtime-digest:end

/** Is the installed copy what this folder was exported from, or what a launcher last installed? */
async function storeUnchanged() {
  const current = runtimeDigest(STORE, ENTRY);
  if (!current) return false;
  if (current === EXPORT_DIGEST) return true;
  const recorded = await readFile(INSTALLED_RECORD, "utf8").then((t) => t.trim(), () => "");
  return recorded !== "" && recorded === current;
}

async function recordInstall() {
  const digest = runtimeDigest(STORE, ENTRY);
  try {
    await mkdir(dirname(INSTALLED_RECORD), { recursive: true, mode: 0o700 });
    if (digest) {
      const tmp = `${INSTALLED_RECORD}.${process.pid}`;
      await writeFile(tmp, `${digest}\n`);
      await rename(tmp, INSTALLED_RECORD);
    } else {
      await rm(INSTALLED_RECORD, { force: true });
    }
  } catch {
    // Without a record, the next run trusts only the export's digest.
  }
}

/** The same app: the same app_uid, or for apps made before it, the same creation time. */
function sameApp(ours, theirs) {
  if (ours.app_uid || theirs.app_uid) return ours.app_uid === theirs.app_uid;
  return ours.created_at !== undefined && ours.created_at === theirs.created_at;
}

/** Yes only from BIOROUTER_EXPORT_REPLACE=1, or from a person at a terminal. */
async function askYes(question) {
  if (process.env.BIOROUTER_EXPORT_REPLACE === "1") return true;
  if (!process.stdin.isTTY) return false;
  const rl = createInterface({ input: process.stdin, output: process.stdout });
  try {
    return /^(y|yes)$/i.test((await rl.question(`${question} [y/N] `)).trim());
  } finally {
    rl.close();
  }
}

async function backupDir() {
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  const dir = join(LAUNCHER_HOME, "backups", `${APP_ID}-${stamp}-${process.pid}`);
  await mkdir(dir, { recursive: true, mode: 0o700 });
  return dir;
}

async function backupRuntime() {
  const backup = await backupDir();
  for (const item of ["manifest.json", ENTRY, "src", "dist", "assets"]) {
    const from = join(STORE, item);
    if (await exists(from)) await cp(from, join(backup, item), { recursive: true });
  }
  console.log(`Kept a copy of the installed '${APP_ID}' in ${backup}`);
}

async function copyRuntime(withManifest) {
  await mkdir(dirname(join(STORE, ENTRY)), { recursive: true });
  const entryTmp = join(STORE, `.entry.${process.pid}`);
  await cp(join(ROOT, ENTRY), entryTmp);
  await rename(entryTmp, join(STORE, ENTRY));
  for (const sub of ["src", "dist", "assets"]) {
    if (!(await exists(join(ROOT, sub)))) continue;
    // A kept store manifest's build stamp describes the store's own bundle, so
    // an app the daemon builds keeps its dist/ and is rebuilt from src/.
    if (sub === "dist" && !withManifest && (await exists(join(ROOT, "src", "main.ts")))) continue;
    const fresh = join(STORE, `.${sub}.new.${process.pid}`);
    const old = join(STORE, `.${sub}.old.${process.pid}`);
    await rm(fresh, { recursive: true, force: true });
    await cp(join(ROOT, sub), fresh, { recursive: true });
    if (await exists(join(STORE, sub))) await rename(join(STORE, sub), old);
    await rename(fresh, join(STORE, sub));
    await rm(old, { recursive: true, force: true });
  }
  // The manifest last: it is what makes the daemon see the app.
  if (withManifest) await cp(join(ROOT, "manifest.json"), join(STORE, "manifest.json"));
}

/**
 * Install this folder's app into the store the daemon reads, never over another
 * app: a different app under this id is moved to backups only when you say so
 * (or BIOROUTER_EXPORT_REPLACE=1); this app changed since the export (by the
 * drafter, or where it is installed) is used as it is; an older copy is
 * replaced with a backup kept; an unchanged one is re-synced, which is the
 * edit, build, re-run loop.
 */
async function installApp() {
  const ours = await readManifest(join(ROOT, "manifest.json"));
  if (!ours) throw new Error("manifest.json is missing from this folder or unreadable - re-export the app.");
  ENTRY = typeof ours.entry === "string" && ours.entry ? ours.entry : "index.html";
  if (isAbsolute(ENTRY) || /^[\\/]/.test(ENTRY) || ENTRY.includes(":") || ENTRY.split(/[\\/]/).includes("..")) {
    throw new Error(`manifest.json names an unsafe entry file (${ENTRY}) - re-export the app.`);
  }
  if (!(await exists(join(ROOT, ENTRY)))) throw new Error(`${ENTRY} is missing from this folder - re-export the app.`);
  let withManifest = true;
  if (await exists(join(STORE, "manifest.json"))) {
    const theirs = await readManifest(join(STORE, "manifest.json"));
    if (!theirs || !sameApp(ours, theirs)) {
      console.log(`Biorouter already has a different app named '${APP_ID}'.`);
      if (!(await askYes(`Replace it with this one? It is moved to ${join(LAUNCHER_HOME, "backups")} first.`))) {
        throw new Error(`Left the installed '${APP_ID}' as it is. To replace it, run again with BIOROUTER_EXPORT_REPLACE=1.`);
      }
      const backup = await backupDir();
      await rename(STORE, join(backup, APP_ID));
      console.log(`Moved the other '${APP_ID}' to ${backup}`);
    } else if (Number(theirs.updated_at) > Number(ours.updated_at)) {
      if (process.env.BIOROUTER_EXPORT_REPLACE !== "1") {
        console.log(`The installed '${APP_ID}' was changed after this folder was exported, so it is used as it is.`);
        console.log("To replace it with this folder, run again with BIOROUTER_EXPORT_REPLACE=1.");
        return;
      }
      await backupRuntime();
    } else if (Number(theirs.updated_at) < Number(ours.updated_at)) {
      await backupRuntime();
    } else {
      withManifest = false;
      if (!(await storeUnchanged())) {
        if (process.env.BIOROUTER_EXPORT_REPLACE !== "1") {
          console.log(`The installed '${APP_ID}' was changed where it is installed since this folder was exported or last installed (an edit, or a rebuild by a newer Biorouter), so it is used as it is.`);
          console.log("To replace it with this folder, run again with BIOROUTER_EXPORT_REPLACE=1.");
          return;
        }
        await backupRuntime();
      }
    }
  }
  await copyRuntime(withManifest);
  await recordInstall();
  installedFromHere = true;
}

/** A bundled daemon is used only when it runs on this machine. */
function runs(bin) {
  const probe = spawnSync(bin, ["--version"], { stdio: "ignore", timeout: 30000 });
  return !probe.error && probe.status === 0;
}

function daemonPaths() {
  const exe = process.platform === "win32" ? "biorouterd.exe" : "biorouterd";
  const list = [];
  if (process.env.BIOROUTERD_BIN) list.push(process.env.BIOROUTERD_BIN);
  const bundled = join(ROOT, "payload", "bin", exe);
  if (runs(bundled)) list.push(bundled);
  list.push(exe);
  if (process.platform === "win32") {
    const local = process.env.LOCALAPPDATA;
    if (local) {
      list.push(join(local, "Programs", "Biorouter", "resources", "bin", exe));
      // The Squirrel installer's layout: %LOCALAPPDATA%\biorouter_app\app-<version>\.
      try {
        const squirrel = join(local, "biorouter_app");
        readdirSync(squirrel)
          .filter((name) => name.startsWith("app-"))
          .map((name) => join(squirrel, name))
          .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)
          .forEach((dir) => list.push(join(dir, "resources", "bin", exe)));
      } catch { /* not installed that way */ }
    }
    list.push("C:\\Program Files\\Biorouter\\resources\\bin\\biorouterd.exe");
  } else {
    list.push(
      join(homedir(), ".local/bin/biorouterd"),
      "/usr/local/bin/biorouterd",
      "/opt/homebrew/bin/biorouterd",
      "/Applications/Biorouter.app/Contents/Resources/bin/biorouterd",
      join(homedir(), "Applications/Biorouter.app/Contents/Resources/bin/biorouterd"),
      "/opt/Biorouter/resources/bin/biorouterd",
      // Linux GUI packages install under usr/lib/<name>: the deb maker lowercases
      // the name and the rpm maker preserves its case, so both spellings are real.
      "/usr/lib/biorouter/resources/bin/biorouterd",
      "/usr/lib/Biorouter/resources/bin/biorouterd",
    );
  }
  return list;
}

function probe(port) {
  return new Promise((res) => {
    const r = httpRequest({ host: "127.0.0.1", port, path: "/status", timeout: 900 }, (resp) => {
      resp.resume();
      res(resp.statusCode === 200);
    });
    r.on("error", () => res(false));
    r.on("timeout", () => { r.destroy(); res(false); });
    r.end();
  });
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** One request to the daemon on `port` carrying its secret; resolves to { status, body }. */
function withSecret(port, secret, method, path) {
  return new Promise((res) => {
    const r = httpRequest(
      { host: "127.0.0.1", port, path, method, timeout: 5000, headers: { "X-Secret-Key": secret } },
      (resp) => {
        let body = "";
        resp.setEncoding("utf8");
        resp.on("data", (chunk) => { body += chunk; });
        resp.on("end", () => res({ status: resp.statusCode, body }));
      }
    );
    r.on("error", () => res({ status: 0, body: "" }));
    r.on("timeout", () => { r.destroy(); res({ status: 0, body: "" }); });
    r.end();
  });
}

const secretWorks = async (port, secret) =>
  (await withSecret(port, secret, "GET", "/apps")).status === 200;

async function rememberDaemon(port, secret) {
  try {
    await mkdir(dirname(LAUNCHER_STATE), { recursive: true, mode: 0o700 });
    const staged = `${LAUNCHER_STATE}.${process.pid}`;
    await writeFile(staged, `${port} ${secret}\n`, { mode: 0o600 });
    await rename(staged, LAUNCHER_STATE);
  } catch {
    console.warn("(could not remember this daemon; the next run starts another)");
  }
}

/** A daemon whose secret is known: named, remembered, or started here. */
async function ensureDaemon() {
  const named = process.env.BIOROUTER_SERVER__SECRET_KEY;
  const preferred = Number(process.env.BIOROUTERD_PORT || 3000);
  if (named && process.env.BIOROUTERD_PORT && (await secretWorks(preferred, named))) {
    console.log(`Using the Biorouter daemon on :${preferred}`);
    return { port: preferred, secret: named };
  }
  try {
    const [port, secret] = (await readFile(LAUNCHER_STATE, "utf8")).trim().split(/\s+/);
    if (port && secret && (await secretWorks(Number(port), secret))) {
      console.log(`Using the Biorouter daemon started for your apps on :${port}`);
      return { port: Number(port), secret };
    }
  } catch { /* no remembered daemon */ }
  for (const bin of daemonPaths()) {
    for (const p of [preferred, 3001, 3002, 3003, 3004, 3005]) {
      // A daemon already here is not one whose secret this server knows.
      if (await probe(p)) continue;
      // A new secret for every attempt, so one asked of whatever held the port
      // never becomes the secret of a daemon started elsewhere.
      const secret = randomBytes(32).toString("hex");
      let child;
      try {
        child = spawn(bin, ["agent"], {
          env: { ...process.env, BIOROUTER_PORT: String(p), BIOROUTER_SERVER__SECRET_KEY: secret },
          stdio: "ignore", detached: true,
        });
      } catch { break; }          // this binary doesn't exist; try the next one
      let spawnFailed = false;
      let exited = false;
      child.on("error", () => { spawnFailed = true; });
      child.on("exit", () => { exited = true; });
      child.unref();
      // A first start can be slow, so wait 60 seconds, as `biorouter apps open` does.
      for (let i = 0; i < 120 && !spawnFailed && !exited; i++) {
        if (await secretWorks(p, secret)) {
          console.log(`Started ${bin} on :${p}`);
          await rememberDaemon(p, secret);
          return { port: p, secret };
        }
        await sleep(500);
      }
      if (spawnFailed) break;
      // Never leave a daemon running whose secret nobody will ever know.
      if (!exited) { try { child.kill(); } catch { /* already gone */ } }
      console.warn(`  :${p} did not come up`);
    }
  }
  throw new Error(
    "Could not find or start biorouterd. Install Biorouter, or set BIOROUTERD_BIN=/path/to/biorouterd"
  );
}

/** The path of this app's one-time launch link: opening it sets the app's access cookie. */
async function launchPath(port, secret) {
  const { status, body } = await withSecret(port, secret, "POST", `/apps/${APP_ID}/launch`);
  let path;
  try { path = JSON.parse(body).path; } catch { /* not JSON */ }
  if (status !== 200 || typeof path !== "string" || !/^\/apps\/[A-Za-z0-9_-]+\/\?t=[0-9a-f]{64}$/.test(path)) {
    throw new Error(`The daemon on :${port} does not serve '${APP_ID}'. Is the store at ${STORE}?`);
  }
  return path;
}

// Install the app, then bring up a daemon. On failure, print the actionable
// message (not an unhandled-rejection stack trace) and exit cleanly.
let daemonPort;
let launch;
try {
  await installApp();
  const daemon = await ensureDaemon();
  daemonPort = daemon.port;
  launch = await launchPath(daemon.port, daemon.secret);
} catch (e) {
  console.error("\n" + (e && e.message ? e.message : e) + "\n");
  process.exit(1);
}

/** Proxy an ordinary HTTP request to the daemon. */
function proxyHttp(req, res) {
  const up = httpRequest(
    { host: "127.0.0.1", port: daemonPort, path: req.url, method: req.method, headers: req.headers },
    (upRes) => {
      res.writeHead(upRes.statusCode || 502, upRes.headers);
      upRes.pipe(res);
    }
  );
  up.on("error", () => { res.writeHead(502); res.end("Biorouter daemon unreachable"); });
  req.pipe(up);
}

/** One file of this folder's `sub` (dist or assets), contained to it on every platform. */
async function serveFromFolder(res, sub, rest) {
  let name;
  try { name = decodeURIComponent(rest); } catch { res.writeHead(400); res.end("Bad request"); return; }
  const base = join(ROOT, sub);
  const file = resolve(base, name);
  const rel = relative(base, file);
  if (!rel || name.includes("\0") || rel.startsWith("..") || isAbsolute(rel)) {
    res.writeHead(403); res.end("Forbidden"); return;
  }
  try {
    const body = await readFile(file);
    res.writeHead(200, { "Content-Type": MIME[extname(file).toLowerCase()] || "application/octet-stream" });
    res.end(body);
  } catch {
    res.writeHead(404); res.end("Not found");
  }
}

let boundPort = PORT;
// Only this server's own loopback address. A page on any other site whose name
// someone points at 127.0.0.1 (DNS rebinding) sends its own Host, and is refused.
const ownHost = (host) => host === `127.0.0.1:${boundPort}` || host === `localhost:${boundPort}`;

const server = createServer(async (req, res) => {
  if (!ownHost(req.headers.host || "")) { res.writeHead(421); res.end("Misdirected request"); return; }
  const url = (req.url || "/").split("?")[0];
  // The app's page is the daemon's: only the daemon can put this run's socket
  // token into it, and the folder's own index.html has none. Open the printed
  // one-time address first; it sets the cookie this redirect then relies on.
  if (url === "/" || url === "/index.html") {
    res.writeHead(302, { Location: APP_PREFIX }); res.end(); return;
  }
  if (installedFromHere && req.method === "GET") {
    for (const sub of ["dist", "assets"]) {
      const prefix = `${APP_PREFIX}${sub}/`;
      if (url.startsWith(prefix)) return serveFromFolder(res, sub, url.slice(prefix.length));
    }
    // An edited page shows on refresh too.
    if (url === APP_PREFIX) {
      try {
        const tmp = join(STORE, `.entry.${process.pid}`);
        await cp(join(ROOT, ENTRY), tmp);
        await rename(tmp, join(STORE, ENTRY));
      } catch { /* the installed page is served as it is */ }
    }
  }
  if (url === "/apps" || url.startsWith("/apps/")) return proxyHttp(req, res);
  res.writeHead(404); res.end("Not found");
});

// The agent WebSocket. Forward the upgrade verbatim and splice the sockets.
server.on("upgrade", (req, socket, head) => {
  if (!ownHost(req.headers.host || "")) { socket.destroy(); return; }
  const up = httpRequest({
    host: "127.0.0.1", port: daemonPort, path: req.url, method: "GET", headers: req.headers,
  });
  up.on("upgrade", (upRes, upSocket, upHead) => {
    const lines = ["HTTP/1.1 101 Switching Protocols"];
    for (const [k, v] of Object.entries(upRes.headers)) lines.push(`${k}: ${v}`);
    socket.write(lines.join("\r\n") + "\r\n\r\n");
    if (upHead && upHead.length) socket.write(upHead);
    upSocket.pipe(socket);
    socket.pipe(upSocket);
    const bail = () => { upSocket.destroy(); socket.destroy(); };
    upSocket.on("error", bail);
    socket.on("error", bail);
  });
  up.on("response", (r) => { socket.write(`HTTP/1.1 ${r.statusCode} \r\n\r\n`); socket.destroy(); });
  up.on("error", () => socket.destroy());
  if (head && head.length) up.write(head);
  up.end();
});

// Bind loopback ONLY. This server proxies the daemon's `/apps/**` routes and
// never adds the secret to what it forwards: a browser gets the app only with the
// access cookie its one-time launch link sets. Listening on 0.0.0.0 would still
// put the app's page one stolen link away from the whole LAN.
function listen(port, attempt = 0) {
  server.once("error", (e) => {
    if (e.code === "EADDRINUSE" && attempt < 10) {
      console.warn(`:${port} is busy, trying :${port + 1}`);
      listen(port + 1, attempt + 1);
    } else {
      console.error(e.message);
      process.exit(1);
    }
  });
  server.listen(port, "127.0.0.1", () => {
    boundPort = port;
    console.log(`App running at http://127.0.0.1:${port}  (agent -> :${daemonPort})`);
    console.log(`Open it here first (this address works once): http://127.0.0.1:${port}${launch}`);
  });
}
listen(PORT);
"##;

fn readme(manifest: &Manifest) -> String {
    format!(
        r#"# {title}

{desc}

A standalone **Biorouter app** generated by Agent Drafter (`{id}`). The UI is
prebuilt (`dist/app.js` ships with it); the agent loop runs on a Biorouter
daemon using whatever LLM provider you have configured.

## Run it

Double-click **`run.command`** (macOS), run **`bash run.sh`** (Linux/WSL), or
double-click **`run.bat`** (Windows; it runs `run.ps1`).

That's the whole thing. The launcher:

1. installs the app into your local Biorouter store (`manifest.json` and all),
2. installs any first-run payload this export carries (`payload/`: knowledge
   bases and skills, with your consent; `export.json` lists them),
3. reuses the `biorouterd` it started last time, or starts one on the first
   free port,
4. asks that daemon for a one-time link to the app, and
5. opens the link in your web browser, and prints it in the terminal too.

**It never installs over another app.** If Biorouter already has a different
app with this name, the launcher asks before replacing it (or set
`BIOROUTER_EXPORT_REPLACE=1`), and moves the old one to
`app-launcher/backups/` in your Biorouter folder rather than deleting it. If this
same app was changed where it is installed after the folder was exported (in a
chat, or by editing its files there), the launcher opens that copy as it is and
says how to replace it. Edits you make in this folder are installed each time
you run the launcher, as long as the installed copy is still the one this folder
put there.

A daemon serves an app only to a browser that opened such a link, and hands one
out only to a caller that knows the daemon's secret, so another account on the
same computer cannot open your app. That is why the launcher starts a daemon of
its own rather than using one that happens to be running: it keeps that
daemon's port and secret, readable by you alone, in `app-launcher/daemon` in
your Biorouter folder (`~/.config/biorouter` on macOS and Linux,
`%APPDATA%\Block\biorouter\config` on Windows, or `$BIOROUTER_PATH_ROOT/config`
when you set that). To use a daemon you run yourself, set `BIOROUTERD_PORT` and
`BIOROUTER_SERVER__SECRET_KEY` to its port and secret. The link works once and
for a few minutes; after that the browser keeps the app open until the daemon
restarts. Run the launcher again for a new link.

The link opens the app for whoever uses it first, so the launcher never puts it
on a command line, which other accounts on the computer can read. It writes the
link into a small page only you can read, in `app-launcher/open`, and opens that
page in your web browser, which goes on to the app. A browser installed as a
snap package cannot read that folder; if yours shows an error instead of the
app, open the address the terminal printed.

No Node, no `npm install`, no build step. You need Biorouter installed with a
provider configured (`biorouter configure`), unless this is a **fat** export,
which bundles `biorouterd` under `payload/bin/` and needs nothing installed. The
bundled daemon only runs on the kind of computer the export was made on; on any
other, the launcher uses an installed `biorouterd`. If `biorouterd` lives
somewhere unusual, set `BIOROUTERD_BIN=/path/to/biorouterd`.

- **macOS quarantine (fat exports):** a bundled `payload/bin/biorouterd`
  downloaded from the internet is quarantined by Gatekeeper. If macOS blocks it,
  right-click the launcher (or the binary) and choose **Open** once to approve
  it. Per-app notarized packaging is a later milestone.
- **Non-interactive install:** set `BIOROUTER_EXPORT_YES=1` to accept the
  payload install without the interactive prompt.

## Editing the UI

`src/main.ts` is the app logic; `src/sdk.ts` is the Biorouter App SDK. To
iterate, use the dev server:

    npm install
    npm run build
    npm start            # http://127.0.0.1:8787

`serve.mjs` installs the app the same way the launcher does, starts (or reuses)
a daemon, and proxies `/apps/**` to it, including the agent WebSocket. It prints
a one-time address under `/apps/{id}/`: open that first. After that,
`http://127.0.0.1:8787/` takes you to the app, and a rebuilt `dist/app.js` or an
edited `index.html` shows up when you refresh.

## Notes

- **`index.html` is the app's source page, not something to open off disk.**
  The daemon adds the app's configuration and its connection to the agent when
  it serves the page; opened as a file, it has neither. Use `run.sh` or
  `npm start`.
- The daemon uses **your existing Biorouter configuration**: provider, model,
  and credentials from your OS keychain. Biorouter supports many providers
  (Anthropic, OpenAI, Azure, Bedrock, Ollama, Xiaomi MiMo, local llama.cpp, …);
  this app is provider-agnostic. If a key isn't in your keychain, export it
  first: `export <PROVIDER>_API_KEY=...`
- The agent's model, extensions, skills, and knowledge base come from
  `manifest.json`.
- If the export moved one of the app's own files because it had the name of a
  launcher file, it is under `app-files/`.
"#,
        title = manifest.title,
        desc = manifest.description,
        id = manifest.id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_drafter::store::{AgentConfig, ArtifactKind, ModelSelection};
    use std::path::{Path, PathBuf};

    fn command_has_version(program: &Path, work_dir: &Path) -> bool {
        std::process::Command::new(program)
            .arg("--version")
            .current_dir(work_dir)
            .output()
            .is_ok_and(|output| output.status.success())
    }

    #[cfg(not(windows))]
    fn find_command(program: &str, work_dir: &Path) -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|directory| {
                if directory.is_absolute() {
                    directory.join(program)
                } else {
                    work_dir.join(directory).join(program)
                }
            })
            .find(|candidate| candidate.is_file() && command_has_version(candidate, work_dir))
    }

    #[cfg(not(windows))]
    fn find_usable_bash(work_dir: &Path) -> Option<PathBuf> {
        find_command("bash", work_dir)
    }

    #[cfg(windows)]
    fn push_git_bash_candidates(candidates: &mut Vec<PathBuf>, git_root: &Path) {
        candidates.push(git_root.join("bin").join("bash.exe"));
        candidates.push(git_root.join("usr").join("bin").join("bash.exe"));
    }

    #[cfg(windows)]
    fn where_paths(program: &str, work_dir: &Path) -> Vec<PathBuf> {
        let Ok(output) = std::process::Command::new("where.exe")
            .arg(program)
            .current_dir(work_dir)
            .output()
        else {
            return Vec::new();
        };
        if !output.status.success() {
            return Vec::new();
        }
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(PathBuf::from)
            .collect()
    }

    #[cfg(windows)]
    fn find_usable_bash(work_dir: &Path) -> Option<PathBuf> {
        let mut candidates = Vec::new();
        for variable in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(program_files) = std::env::var_os(variable) {
                push_git_bash_candidates(
                    &mut candidates,
                    &PathBuf::from(program_files).join("Git"),
                );
            }
        }
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            push_git_bash_candidates(
                &mut candidates,
                &PathBuf::from(local_app_data).join("Programs").join("Git"),
            );
        }

        for git in where_paths("git.exe", work_dir) {
            if let Some(git_root) = git.ancestors().find(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("git"))
            }) {
                push_git_bash_candidates(&mut candidates, git_root);
            }
        }
        candidates.extend(
            where_paths("bash.exe", work_dir)
                .into_iter()
                .filter(|path| {
                    let path = path.to_string_lossy().replace('\\', "/");
                    path.to_ascii_lowercase().contains("/git/")
                }),
        );

        candidates
            .into_iter()
            .find(|candidate| command_has_version(candidate, work_dir))
    }

    fn manifest(kind: ArtifactKind) -> Manifest {
        Manifest {
            id: "demo".into(),
            title: "Demo <App>".into(),
            description: "d".into(),
            kind,
            entry: "index.html".into(),
            created_at: 0,
            updated_at: 0,
            agent: if kind == ArtifactKind::Agentic {
                Some(AgentConfig {
                    system_prompt: "be helpful".into(),
                    greeting: Some("hi".into()),
                    tools: vec![],
                    model: Some(ModelSelection {
                        provider: Some("xiaomi_mimo".into()),
                        model: Some("mimo-v2.5".into()),
                        ..Default::default()
                    }),
                    extensions: vec!["developer".into()],
                    skills: vec![],
                    knowledge_base: None,
                    max_turns: None,
                    ..Default::default()
                })
            } else {
                None
            },
            width: None,
            height: None,
            built_at: None,
            sdk_hash: None,
            session_id: None,
            app_uid: None,
            surface: crate::agent_drafter::manifest::SurfaceDecl::default(),
            theme: crate::agent_drafter::manifest::ThemeConfig::default(),
        }
    }

    #[test]
    fn starter_escapes_and_substitutes() {
        let html = starter("A <b> & C", "desc");
        assert!(html.contains("A &lt;b&gt; &amp; C"));
        assert!(!html.contains("{{TITLE}}"));
        assert!(!html.contains("{{DESCRIPTION}}"));
    }

    #[test]
    fn assemble_app_injects_theme_base_config_and_bundle() {
        let m = manifest(ArtifactKind::Agentic);
        let out = assemble_app(
            &m,
            "<html><head></head><body>hi</body></html>",
            Some("/apps/demo/"),
            None,
            None,
        );
        assert!(out.contains("biorouter-theme"));
        assert!(out.contains("<base href=\"/apps/demo/\">"));
        // Config ships as a non-executable JSON data island (CSP-friendly), never
        // an inline executable `window.BIOROUTER_APP_CONFIG` script.
        assert!(out.contains("<script type=\"application/json\" id=\"biorouter-app-config\">"));
        assert!(!out.contains("window.BIOROUTER_APP_CONFIG"));
        assert!(out.contains("\"appId\":\"demo\""));
        assert!(out.contains("dist/app.js"));
        // theme precedes the bundle script
        assert!(out.find("biorouter-theme").unwrap() < out.find("dist/app.js").unwrap());
    }

    #[test]
    fn app_config_script_embeds_ws_token_when_given() {
        let m = manifest(ArtifactKind::Agentic);
        // No token → no wsToken key at all (exports, previews).
        let none = app_config_script(&m, None, &[], None);
        assert!(!none.contains("wsToken"), "absent token must not appear");
        // A token → surfaced verbatim in the config JSON the SDK reads.
        let with = app_config_script(&m, None, &[], Some("deadbeefcafef00d0123456789abcdef"));
        assert!(with.contains("\"wsToken\":\"deadbeefcafef00d0123456789abcdef\""));
        // And it flows through the full assemble path.
        let out = assemble_app(
            &m,
            "<html><head></head><body></body></html>",
            None,
            None,
            Some("deadbeefcafef00d0123456789abcdef"),
        );
        assert!(out.contains("\"wsToken\":\"deadbeefcafef00d0123456789abcdef\""));
    }

    #[test]
    fn assemble_app_handles_missing_head_and_body() {
        let m = manifest(ArtifactKind::Agentic);
        let out = assemble_app(&m, "<div>bare</div>", None, None, None);
        assert!(out.contains("biorouter-theme"));
        assert!(out.contains("dist/app.js"));
    }

    #[test]
    fn config_neutralizes_script_breakout() {
        let mut m = manifest(ArtifactKind::Agentic);
        m.agent.as_mut().unwrap().greeting = Some("</script><script>alert(1)</script>".into());
        let out = assemble_app(
            &m,
            "<html><head></head><body></body></html>",
            None,
            None,
            None,
        );
        assert!(!out.contains("</script><script>alert(1)"));
        assert!(out.contains("\\u003c"));
    }

    #[test]
    fn card_has_launch_banner_and_theme() {
        let m = manifest(ArtifactKind::Static);
        let out = assemble_card(&m, "<html><head></head><body><h1>Hi</h1></body></html>");
        assert!(out.contains("biorouter-theme"));
        assert!(out.contains("Applications panel"));
        assert!(out.contains("<h1>Hi</h1>"));
    }

    const ISLAND: &str = "<script type=\"application/json\" id=\"biorouter-app-config\">";

    /// ⚠ An export used to ship an already assembled `index.html`, every
    /// launcher installed it as the entry, and the daemon assembled it again:
    /// two config islands (the SDK read the first, which had no socket token),
    /// two bundle tags, two theme blocks, a doubled `data-br-pack`. Assembling
    /// is idempotent now, so those stores heal on their next serve.
    #[test]
    fn assemble_app_is_idempotent() {
        use crate::agent_drafter::manifest::ThemeConfig;
        let mut m = manifest(ArtifactKind::Agentic);
        m.theme = ThemeConfig {
            pack: "midnight".into(),
            accent: Some("#8b5cf6".into()),
            ..Default::default()
        };
        let raw =
            "<html lang=\"en\"><head><title>t</title></head><body><main>x</main></body></html>";
        let fresh = assemble_app(&m, raw, Some("/apps/demo/"), None, Some("tok"));
        // What an export shipped (no token, an explicit endpoint), nested once
        // more by a re-export, then served.
        let exported = assemble_app_with_endpoints(
            &m,
            raw,
            None,
            None,
            &["ws://127.0.0.1:3000/apps/demo/agent".to_string()],
            None,
        );
        let nested = assemble_app(&m, &exported, Some("/apps/demo/"), None, None);
        for damaged in [&exported, &nested, &fresh] {
            let served = assemble_app(&m, damaged, Some("/apps/demo/"), None, Some("tok"));
            assert_eq!(served, fresh, "re-assembling heals to the fresh page");
            assert_eq!(served.matches(ISLAND).count(), 1);
            let island = served.split_once(ISLAND).unwrap().1;
            assert!(island.contains("\"wsToken\":\"tok\""), "{island}");
            assert!(!island.contains("ws://127.0.0.1:3000"), "{island}");
            assert_eq!(served.matches("src=\"dist/app.js\"").count(), 1);
            assert_eq!(served.matches("<style id=\"biorouter-theme\">").count(), 1);
            assert_eq!(
                served
                    .matches("<style id=\"biorouter-theme-overrides\">")
                    .count(),
                1
            );
            assert_eq!(served.matches("<base href=").count(), 1);
            assert!(served.starts_with("<html data-br-pack=\"midnight\" lang=\"en\">"));
        }
        // The card heals the same way.
        assert_eq!(assemble_card(&m, &nested), assemble_card(&m, raw));
    }

    /// What the author wrote is never stripped: only the assembler's own markup.
    #[test]
    fn strip_injected_leaves_authored_markup_alone() {
        let authored = "<html data-theme=\"x\"><head><base href=\"https://example.org/\">\
                        <style id=\"mine\">p{}</style></head><body>\
                        <script type=\"module\" src=\"./dist/app.js\"></script>\
                        <script type=\"application/json\" id=\"data\">{}</script></body></html>";
        assert_eq!(strip_injected(authored), authored);
    }

    /// ⚠ The insertion offset used to come from a lowercased COPY of the page:
    /// U+2126 OHM SIGN lowercases 3 → 2 bytes and U+0130 'İ' 2 → 3, so a page
    /// with "10 kΩ" spliced the config island into the middle of a tag, and an
    /// offset inside a character panicked the request.
    #[test]
    fn injection_offsets_survive_characters_whose_lowercase_changes_length() {
        let m = manifest(ArtifactKind::Agentic);
        for odd in [
            "10 k\u{2126}",
            "\u{0130}stanbul",
            "\u{212A}elvin \u{212B}ngstr\u{f6}m",
        ] {
            let body = format!("<p>{odd}</p><p>{odd}</p>");
            let raw = format!("<html><HEAD></HEAD><body>{body}</BODY></html>");
            let out = assemble_app(&m, &raw, None, None, Some("t"));
            let (before_island, _) = out.split_once(ISLAND).expect("island");
            assert!(before_island.ends_with(&body), "{odd}: {out}");
            assert!(out.ends_with("</script>\n</BODY></html>"), "{odd}: {out}");
            assert!(
                out.contains("<HEAD><style id=\"biorouter-theme\">"),
                "{odd}"
            );
            // A second pass still finds every needle.
            assert_eq!(assemble_app(&m, &out, None, None, Some("t")), out);
        }
        assert_eq!(find_ascii_ci("k\u{2126}</BODY>", "</body>"), Some(4));
        assert_eq!(find_ascii_ci("\u{0130}", "</body>"), None);
    }

    /// The bundle tag is added only when there is a bundle and the author does
    /// not already load it: a static page with no bundle asked for a file that
    /// 404'd, and one that loads `./dist/app.js` itself ran the bundle twice.
    #[test]
    fn the_bundle_tag_is_added_only_when_needed() {
        let page = "<html><head></head><body>hi</body></html>";
        let st = manifest(ArtifactKind::Static);
        assert!(!assemble_app_page(&st, page, None, None, false, "n").contains("dist/app.js"));
        assert!(assemble_app_page(&st, page, None, None, true, "n").contains(BUNDLE_TAG));
        // An agentic app through the old entry point keeps its tag.
        let ag = manifest(ArtifactKind::Agentic);
        assert!(assemble_app(&ag, page, None, None, None).contains(BUNDLE_TAG));
        assert!(!assemble_app_page(&ag, page, None, None, false, "n").contains("dist/app.js"));
        for own in [
            "<script type=\"module\" src=\"./dist/app.js\"></script>",
            "<script src=\"dist/app.js?v=3\" defer></script>",
            "<script src='/apps/demo/dist/app.js'></script>",
        ] {
            let authored = format!("<html><head>{own}</head><body></body></html>");
            let out = assemble_app_page(&ag, &authored, None, None, true, "n");
            assert_eq!(out.matches("dist/app.js").count(), 1, "{own}: {out}");
        }
    }

    /// The scanner finds the inline code a browser would run, and only that:
    /// data blocks, templates, comments and raw-text elements are not code, and
    /// attribute values are decoded the way the browser hashes them.
    #[test]
    fn authored_inline_code_finds_exactly_what_a_browser_would_run() {
        let html = "<!doctype html><html><head>\n\
            <script>one()</script>\n\
            <script type=\"module\">two()</script>\n\
            <script type=\"application/json\" id=\"d\">{\"not\":\"code\"}</script>\n\
            <script type=\"text/template\"><b onclick=\"no()\">tpl</b></script>\n\
            <script src=\"dist/app.js?v=2\"></script>\n\
            <SCRIPT TYPE=\"TEXT/JAVASCRIPT\">three()</SCRIPT >\n\
            <!-- <script>commented()</script> <b onclick=\"no()\"> -->\n\
            <textarea><script>not a script</script></textarea>\n\
            </head><body>\n\
            <button onclick=\"go(&quot;x&quot;)\" data-br-bind-attr=\"onclick:x\">Go</button>\n\
            <a href=\" JavaScript:alert(1)\">x</a>\n\
            <form action=\"javascript:void 0\"></form>\n\
            <div onmouseover='hover()' ONCLICK=go2()></div>\n\
            </body></html>";
        let code = authored_inline_code(html);
        assert_eq!(code.scripts, ["one()", "two()", "three()"]);
        assert_eq!(code.handlers, ["go(\"x\")", "hover()", "go2()"]);
        assert_eq!(code.script_srcs, ["dist/app.js?v=2"]);
        assert_eq!(code.javascript_urls, 2);
        assert!(code.loads_bundle());
        assert_eq!(authored_inline_code("<p>plain</p>"), InlineCode::default());
    }

    /// One `'sha256-…'` per distinct script and handler, over the text exactly
    /// as the browser hashes it (CRLF folded), and `'unsafe-hashes'` only when a
    /// handler needs it. Never `'unsafe-inline'`.
    #[test]
    fn inline_script_sources_pin_each_authored_block_by_hash() {
        use base64::Engine as _;
        use sha2::Digest as _;
        let hash = |text: &str| {
            format!(
                "'sha256-{}'",
                base64::engine::general_purpose::STANDARD
                    .encode(sha2::Sha256::digest(text.as_bytes()))
            )
        };
        assert!(inline_script_sources("<html><body><p>x</p></body></html>").is_empty());

        let scripts_only =
            inline_script_sources("<script>a()\r\nb()</script><script>a()\nb()</script>");
        assert_eq!(scripts_only, [hash("a()\nb()")]);

        // Handlers are never hashed, and `'unsafe-hashes'` is never sent: with
        // it, injected markup could run any handler the author wrote.
        let with_handlers = inline_script_sources(
            "<button onclick=\"go()\">1</button><button onclick=\"go()\">2</button><script>s()</script>",
        );
        assert_eq!(with_handlers, [hash("s()")]);
        assert!(!with_handlers
            .iter()
            .any(|s| s.contains("unsafe-inline") || s.contains("unsafe-hashes")));
    }

    /// The served page carries no `on…=` handler outside a `<template>`: each
    /// becomes a marker naming this response's nonce, and one script, pinned by
    /// hash like any other, holds the handlers and binds them before the config
    /// island and the bundle.
    #[test]
    fn a_served_page_moves_its_handlers_into_one_bound_script() {
        let m = manifest(ArtifactKind::Static);
        let page = "<html><head></head><body>\
            <button id=\"a\" onclick=\"go(this.dataset.q)\" ONMOUSEOVER='hover()' data-q=\"x\">A</button>\
            <button id=\"b\" onclick=\"go(this.dataset.q)\" onclick=\"dropped()\">B</button>\
            <a href=\"#\"onclick=\"return false\">c</a>\
            <template><button onclick=\"inTemplate()\">t</button></template>\
            <p onclick=\"after()\">p</p>\
            </body></html>";
        let out = assemble_app_page(&m, page, Some("/apps/demo/"), Some("tok"), false, "c0ffee");

        // Every authored handler outside the template is now a marker...
        assert!(out.contains("id=\"a\" data-br-on-click=\"c0ffee.0\" data-br-on-mouseover=\"c0ffee.1\" data-q=\"x\""), "{out}");
        assert!(
            out.contains("id=\"b\" data-br-on-click=\"c0ffee.0\" >B"),
            "{out}"
        );
        assert!(
            out.contains("href=\"#\"data-br-on-click=\"c0ffee.2\""),
            "{out}"
        );
        assert!(out.contains("<p data-br-on-click=\"c0ffee.3\">"), "{out}");
        // ...the repeated attribute a browser drops is gone, and the template's
        // handler is left as written (and stays blocked).
        assert!(!out.contains("dropped()"), "{out}");
        assert!(
            out.contains("<template><button onclick=\"inTemplate()\">"),
            "{out}"
        );
        let outside_template = out.replace("onclick=\"inTemplate()\"", "");
        assert_eq!(
            authored_inline_code(&outside_template).handlers,
            Vec::<String>::new()
        );

        // One binder, holding each distinct handler once, before the island.
        assert_eq!(out.matches("id=\"biorouter-inline-handlers\"").count(), 1);
        let binder_at = out.find("id=\"biorouter-inline-handlers\"").unwrap();
        assert!(binder_at < out.find(CONFIG_ISLAND_TAG).unwrap(), "{out}");
        assert_eq!(out.matches("go(this.dataset.q)").count(), 1, "{out}");
        assert!(out.contains("var nonce = \"c0ffee\";"), "{out}");
        // It is an inline script like any other, so the policy pins it.
        let code = authored_inline_code(&out);
        assert_eq!(code.scripts.len(), 1);
        assert!(code.scripts[0].contains("hover()") && code.scripts[0].contains("after()"));
        assert_eq!(inline_script_sources(&out).len(), 1);

        // A page with no handlers gets no binder.
        let plain = assemble_app_page(
            &m,
            "<html><body><p>x</p></body></html>",
            None,
            Some("t"),
            false,
            "n",
        );
        assert!(!plain.contains("biorouter-inline-handlers"));
        assert!(!plain.contains("data-br-on-"));
    }

    /// A handler that could break the script, or another handler's place in
    /// it, is left as written (blocked, as a browser leaves a handler it
    /// cannot compile) while the others are still bound to the right index.
    #[test]
    fn a_handler_the_binder_cannot_hold_is_left_alone() {
        for bad in [
            "go(",
            "x = '</script><script>evil()'",
            "x = '<!--'",
            "} }, function () { {",
            "} } }) + (function () { { {",
            "x = <b>jsx</b>",
        ] {
            assert!(!handler_is_bindable(bad), "{bad}");
        }
        for good in [
            "go()",
            "return false",
            "if (a) { b(); } else c() // trailing comment",
            "var n = this.value; out.textContent = n * 2;",
            "br.chat(this.dataset.q)",
        ] {
            assert!(handler_is_bindable(good), "{good}");
        }

        let html = "<body><a onclick=\"go(\">1</a><b onclick=\"ok()\">2</b></body>";
        let (out, script) = bind_inline_handlers(html, "n");
        assert!(out.contains("<a onclick=\"go(\">"), "{out}");
        assert!(out.contains("<b data-br-on-click=\"n.0\">"), "{out}");
        assert!(script.unwrap().contains("ok()"));
    }

    /// The binder is real JavaScript: it parses, and under a minimal DOM it
    /// binds exactly the elements whose marker carries its nonce, once, with
    /// `this` the element and `return false` cancelling the event.
    #[test]
    fn the_handler_binder_binds_only_its_own_markers_once() {
        let html =
            "<body><button id=\"a\" onclick=\"hits.push(this.id); return false\">a</button></body>";
        let (out, script) = bind_inline_handlers(html, "abc123");
        let script = script.unwrap();
        let body = script
            .trim_start_matches("<script id=\"biorouter-inline-handlers\">")
            .trim_end()
            .trim_end_matches("</script>");
        assert!(out.contains("data-br-on-click=\"abc123.0\""));
        let Some(node) = which_node() else {
            eprintln!("skipped: no node");
            return;
        };
        // Element `a` carries this page's marker, `injected` a guessed one.
        let harness = format!(
            r#"
var hits = [];
function El(id, attrs) {{
  this.id = id; this.localName = "button"; this.listeners = {{}};
  this.attributes = attrs.map(function (a) {{ return {{ name: a[0], value: a[1] }}; }});
}}
El.prototype.onclick = null;
El.prototype.removeAttribute = function (n) {{
  this.attributes = this.attributes.filter(function (a) {{ return a.name !== n; }});
}};
El.prototype.addEventListener = function (t, f) {{ this.listeners[t] = f; }};
var a = new El("a", [["data-br-on-click", "abc123.0"]]);
var injected = new El("injected", [["data-br-on-click", "0.0"], ["data-br-on-click-x", "abc123"]]);
var elements = [a, injected];
var window = globalThis;
var document = {{ body: {{}}, getElementsByTagName: function () {{ return elements; }} }};
globalThis.HTMLFormElement = function () {{}};
function run() {{
{body}
}}
run();
var event = {{ prevented: false, preventDefault: function () {{ this.prevented = true; }} }};
a.listeners.click(event);
if (hits.join() !== "a" || !event.prevented) throw new Error("not bound: " + hits.join());
if (a.attributes.length !== 0) throw new Error("marker left behind");
if (injected.listeners.click) throw new Error("a guessed marker was bound");
// A second run (an injected copy of the script) binds nothing.
var late = new El("late", [["data-br-on-click", "abc123.0"]]);
elements = [late];
run();
if (late.listeners.click) throw new Error("bound twice");
"#
        );
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("binder.cjs");
        std::fs::write(&file, harness).unwrap();
        let out = std::process::Command::new(node)
            .arg(&file)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn which_node() -> Option<std::path::PathBuf> {
        crate::agent_drafter::bundle::resolve_tool("node").map(|tool| tool.path)
    }

    // ── Theme packs (Apps SDK v2, Pillar 6) ─────────────────────────────────

    #[test]
    fn base_theme_leaves_v1_output_untouched() {
        // The default `biorouter` pack with no overrides must stamp no attribute
        // and inject no overrides layer, so a v1 app renders byte-for-byte as it
        // used to.
        let m = manifest(ArtifactKind::Static);
        assert!(m.theme.is_default());
        let out = assemble_app(
            &m,
            "<html lang=\"en\"><head></head><body>hi</body></html>",
            None,
            None,
            None,
        );
        // `data-br-pack` appears in theme.css pack selectors, so check the
        // stamped ATTRIBUTE specifically (right after `<html`).
        assert!(
            !out.contains("<html data-br-pack"),
            "base pack stamps no attribute"
        );
        assert!(!out.contains("biorouter-theme-overrides"));
    }

    #[test]
    fn named_pack_lands_on_html_element() {
        use crate::agent_drafter::manifest::ThemeConfig;
        let mut m = manifest(ArtifactKind::Static);
        m.theme = ThemeConfig {
            pack: "clinical".into(),
            ..Default::default()
        };
        let out = assemble_app(
            &m,
            "<html lang=\"en\"><head></head><body>hi</body></html>",
            None,
            None,
            None,
        );
        assert!(
            out.contains("<html data-br-pack=\"clinical\" lang=\"en\">"),
            "pack must stamp <html>: {out}"
        );
        // No custom overrides declared → no overrides style layer.
        assert!(!out.contains("biorouter-theme-overrides"));
    }

    #[test]
    fn theme_overrides_are_injected_and_sanitized() {
        use crate::agent_drafter::manifest::ThemeConfig;
        use std::collections::HashMap;
        let mut tokens = HashMap::new();
        tokens.insert("--br-radius".to_string(), "2px".to_string()); // safe
        tokens.insert("color".to_string(), "red".to_string()); // bad key: dropped
        tokens.insert("--br-bg".to_string(), "red; } body{}".to_string()); // breakout: dropped
        let mut m = manifest(ArtifactKind::Agentic);
        m.theme = ThemeConfig {
            pack: "midnight".into(),
            accent: Some("#8b5cf6".into()),
            tokens,
        };
        let out = assemble_app(
            &m,
            "<html><head></head><body></body></html>",
            None,
            None,
            None,
        );
        assert!(out.contains("biorouter-theme-overrides"));
        assert!(
            out.contains("--br-accent: #8b5cf6;"),
            "accent override: {out}"
        );
        assert!(
            out.contains("--br-radius: 2px;"),
            "safe token override kept"
        );
        assert!(!out.contains("color: red"), "unsafe key dropped");
        assert!(!out.contains("red; }"), "breakout value dropped");
        // Overrides key off the pack attribute, which is stamped for midnight.
        assert!(out.contains("<html data-br-pack=\"midnight\">"));
    }

    #[test]
    fn overrides_on_base_pack_still_stamp_the_attribute() {
        use crate::agent_drafter::manifest::ThemeConfig;
        let mut m = manifest(ArtifactKind::Static);
        m.theme = ThemeConfig {
            pack: "biorouter".into(),
            accent: Some("#123456".into()),
            ..Default::default()
        };
        let out = assemble_app(
            &m,
            "<html><head></head><body></body></html>",
            None,
            None,
            None,
        );
        // The base pack needs the attribute too so `:root[data-br-pack]` matches.
        assert!(out.contains("<html data-br-pack=\"biorouter\">"));
        assert!(out.contains("--br-accent: #123456;"));
    }

    #[test]
    fn unknown_pack_falls_back_to_base_and_stamps_nothing() {
        use crate::agent_drafter::manifest::ThemeConfig;
        let mut m = manifest(ArtifactKind::Static);
        m.theme = ThemeConfig {
            pack: "neon-hacker".into(),
            ..Default::default()
        };
        let out = assemble_app(
            &m,
            "<html><head></head><body></body></html>",
            None,
            None,
            None,
        );
        // resolved_pack() → biorouter, no overrides → no attribute stamped.
        assert!(!out.contains("<html data-br-pack"));
    }

    /// What every export carries besides the app's other files: the generated
    /// files, the stripped manifest, and the raw entry and source as authored.
    fn export(m: &Manifest) -> Vec<(String, String)> {
        let mut files = launcher_files(m, None);
        files.push(("manifest.json".to_string(), exported_manifest_json(m)));
        files.push((
            "index.html".to_string(),
            "<html><head></head><body></body></html>".to_string(),
        ));
        files.push(("src/main.ts".to_string(), "import './sdk';".to_string()));
        files
    }

    fn file<'a>(files: &'a [(String, String)], path: &str) -> &'a str {
        &files.iter().find(|(p, _)| p == path).expect(path).1
    }

    #[test]
    fn standalone_export_is_typescript_project() {
        let m = manifest(ArtifactKind::Agentic);
        let files = export(&m);
        let paths: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        for want in [
            "index.html",
            "package.json",
            "serve.mjs",
            "README.md",
            "src/main.ts",
            "run.sh",
            "run.command",
            "biorouter-launch.sh",
            "run.ps1",
            "run.bat",
        ] {
            assert!(paths.contains(&want), "export is missing {want}: {paths:?}");
        }
        assert!(file(&files, "package.json").contains("esbuild"));
    }

    /// The export moves an authored file out of the way of every generated one,
    /// and it can only do that for the names it knows it writes.
    #[test]
    fn every_generated_file_is_a_reserved_name() {
        let m = manifest(ArtifactKind::Agentic);
        for (name, _) in launcher_files(&m, None) {
            assert!(
                GENERATED_EXPORT_FILES.contains(&name.as_str()),
                "{name} is written by every export but is not reserved"
            );
        }
        for name in EXECUTABLE_EXPORT_FILES {
            assert!(GENERATED_EXPORT_FILES.contains(name), "{name}");
        }
        assert!(GENERATED_EXPORT_FILES.contains(&"manifest.json"));
        assert!(GENERATED_EXPORT_FILES.contains(&"export.json"));
    }

    /// The bug that made every export unusable: without a manifest the daemon
    /// 404s the app (`serve_index` / `agent_ws` both `load_manifest`), and
    /// `run.sh`'s install guard never flips. And the author's chat id must not
    /// travel: on another machine it names nothing, or an unrelated chat.
    #[test]
    fn standalone_export_ships_a_manifest_without_the_authors_chat() {
        let mut m = manifest(ArtifactKind::Agentic);
        m.session_id = Some("20261009_3".into());
        let files = export(&m);
        let parsed: Manifest = serde_json::from_str(file(&files, "manifest.json"))
            .expect("exported manifest.json must round-trip");
        assert_eq!(parsed.id, "demo");
        assert_eq!(parsed.entry, "index.html");
        assert!(parsed.agent.is_some());
        assert_eq!(parsed.session_id, None);
        assert!(!file(&files, "manifest.json").contains("20261009_3"));
    }

    /// `serve.mjs` used to be a bare static server, so `npm start` produced a UI
    /// with no backend. It must proxy `/apps/**` (including the WS upgrade), and
    /// it must not become a way into the export folder.
    #[test]
    fn serve_mjs_proxies_the_agent_socket_instead_of_only_serving_files() {
        let m = manifest(ArtifactKind::Agentic);
        let files = export(&m);
        let serve = file(&files, "serve.mjs");
        assert!(
            serve.contains("server.on(\"upgrade\""),
            "must proxy the WebSocket"
        );
        assert!(
            serve.contains("startsWith(\"/apps/\")"),
            "must proxy /apps/**"
        );
        assert!(serve.contains("ensureDaemon"), "must start/reuse a daemon");
        assert!(
            serve.contains("const APP_ID = \"demo\";"),
            "app id must be embedded as JSON"
        );
        assert!(!serve.contains("__APP_ID"), "no placeholder left behind");
        // The per-app WS token rides the query string, so the proxy must forward
        // the FULL req.url (path + query) upstream, not just the path.
        assert!(
            serve.contains("path: req.url"),
            "proxy must preserve query strings (the ws token rides the query)"
        );
        // It fronts the daemon's /apps routes, so it must never leave loopback.
        assert!(
            serve.contains(r#"server.listen(port, "127.0.0.1""#),
            "the dev server must bind loopback only"
        );
        // W2-HRD-1: the proxy forwards the browser's own headers and never adds
        // the daemon secret, or this port would open the app to every local
        // account. The secret is used only to mint the one-time launch link.
        let (_, proxy) = serve.split_once("function proxyHttp").unwrap();
        let (proxy, _) = proxy.split_once("\n}\n").unwrap();
        assert!(
            !proxy.contains("X-Secret-Key"),
            "the proxy must not add the secret"
        );
        assert!(serve.contains("`/apps/${APP_ID}/launch`"));
        assert!(serve.contains("BIOROUTER_SERVER__SECRET_KEY: secret"));
        assert!(serve.contains("works once"));

        // Only its own loopback Host, on pages and sockets alike (DNS rebinding).
        assert!(serve.contains("if (!ownHost(req.headers.host || \"\")) { res.writeHead(421)"));
        assert!(serve.contains("if (!ownHost(req.headers.host || \"\")) { socket.destroy()"));
        // Of the folder, only the app's dist/ and assets/, contained with
        // path.relative (a "/" prefix test 403s every file on Windows), and a
        // malformed escape is a 400, not a crash.
        assert!(serve.contains(r#"for (const sub of ["dist", "assets"])"#));
        assert!(serve.contains("const rel = relative(base, file);"));
        assert!(
            serve.contains("try { name = decodeURIComponent(rest); } catch { res.writeHead(400)")
        );
        assert!(!serve.contains("startsWith(ROOT + \"/\")"));
        assert!(!serve.contains("readFile(join(ROOT, \"payload"));
        // The page is the daemon's, so `/` goes there rather than to a page with
        // no socket token. Not to the one-time link: any local account could
        // fetch `/` and redeem it first.
        assert!(serve.contains("res.writeHead(302, { Location: APP_PREFIX })"));
        assert!(!serve.contains("Location: launch"));
        // A daemon that never answers is stopped, not orphaned.
        assert!(serve.contains("i < 120"));
        assert!(serve.contains("child.kill()"));
        // A bundled daemon only when it runs here.
        assert!(serve.contains("if (runs(bundled)) list.push(bundled);"));
    }

    /// The OS-agnostic launcher set: `run.ps1` (PowerShell) + `run.bat` (thin
    /// wrapper) ship alongside the shell launchers, so the exported folder runs
    /// on Windows too.
    #[test]
    fn windows_launchers_are_generated_and_wired() {
        let m = manifest(ArtifactKind::Agentic);
        let files = export(&m);
        let ps1 = file(&files, "run.ps1");
        let bat = file(&files, "run.bat");

        // The batch wrapper just invokes run.ps1 under a bypassed policy.
        assert!(
            bat.to_lowercase().contains("powershell") && bat.contains("-File \"%~dp0run.ps1\""),
            "run.bat must shell out to run.ps1: {bat}"
        );
        assert!(bat.contains("-ExecutionPolicy Bypass"));

        // run.ps1 installs the payload, prefers a bundled daemon that runs, and
        // opens the daemon origin (so the WS token rides the query string —
        // nothing is hard-coded).
        assert!(
            ps1.contains("Install-Payload"),
            "run.ps1 must install payload"
        );
        assert!(
            ps1.contains("payload\\bin\\biorouterd.exe"),
            "prefers bundled daemon"
        );
        assert!(ps1.contains("& $bundled --version"));
        assert!(ps1.contains("biorouter_app"), "finds the Squirrel install");
        assert!(ps1.contains("/apps/$AppId/"), "opens the daemon origin");
        // W2-HRD-1: through the one-time launch link, minted with the secret of
        // a daemon the launcher started or was told about.
        assert!(ps1.contains("\"$Base/apps/$AppId/launch\""));
        assert!(ps1.contains("$env:BIOROUTER_SERVER__SECRET_KEY = $secret"));
        // The link opens the app for whoever uses it first, so it never reaches
        // a command line: the browser is handed a page that forwards to it.
        assert!(ps1.contains("$Link = \"$Base$LaunchPath\""));
        assert!(ps1.contains("Invoke-Item -LiteralPath $Page"));
        for line in ps1.lines() {
            let opens = [
                "Start-Process",
                "Invoke-Item",
                "Invoke-Expression",
                "cmd",
                "explorer",
            ]
            .iter()
            .any(|opener| line.contains(opener));
            let names_the_link = ["$Link", "$LaunchPath", "?t="]
                .iter()
                .any(|link| line.contains(link));
            assert!(
                !(opens && names_the_link),
                "an opener is handed the link: {line}"
            );
        }
        // ...and the secret leaves the launcher's environment before the browser starts.
        let (_, after_spawn) = ps1
            .split_once("$env:BIOROUTER_SERVER__SECRET_KEY = $secret")
            .unwrap();
        let (spawn, _) = after_spawn
            .split_once("Invoke-Item -LiteralPath $Page")
            .unwrap();
        assert!(spawn.contains("Remove-Item Env:BIOROUTER_SERVER__SECRET_KEY"));
        // A daemon that never answers is stopped rather than left holding a port.
        assert!(spawn.contains("-PassThru"));
        assert!(spawn.contains("Stop-Process -Id $proc.Id"));
        assert!(spawn.contains("$i -lt 120"));
        // It never installs over another app, and never deletes a skill.
        assert!(ps1.contains("Test-SameApp $ours $theirs"));
        assert!(ps1.contains("BIOROUTER_EXPORT_REPLACE"));
        assert!(
            ps1.contains("Move-Item -LiteralPath $dst -Destination (Join-Path $backup $d.Name)")
        );
        // export.json sits at the top of the folder.
        assert!(ps1.contains("(see export.json)"));
        assert!(!ps1.contains("payload/export.json"));
        // The app id is embedded, not a leftover placeholder.
        assert!(ps1.contains("$AppId = \"demo\""));
        assert!(!ps1.contains("__APP_ID__"));
        // Consent + skip hook.
        assert!(ps1.contains("BIOROUTER_EXPORT_YES"));
    }

    /// The shell launcher library exposes the first-run payload installer, and
    /// `run.sh` invokes it. Launcher-mode exports (no `payload/` dir) short-out
    /// harmlessly.
    #[test]
    fn launcher_lib_installs_payload_with_consent() {
        let m = manifest(ArtifactKind::Agentic);
        let files = export(&m);
        let lib = file(&files, "biorouter-launch.sh");
        let run = file(&files, "run.sh");

        assert!(lib.contains("install_payload()"), "defines install_payload");
        assert!(
            lib.contains(r#"[ -d "$DIR/payload" ] || return 0"#),
            "no-op without a payload dir"
        );
        assert!(
            lib.contains(".apps-payload-installed-$APP_ID"),
            "guarded by a marker"
        );
        assert!(lib.contains("BIOROUTER_EXPORT_YES"), "consent skip hook");
        assert!(
            lib.contains("payload/bin/biorouterd"),
            "prefers the bundled daemon"
        );
        assert!(
            run.contains("install_payload"),
            "run.sh calls install_payload"
        );
        // export.json is written at the top of the folder, so that is where the
        // consent prompt and the README point.
        assert!(lib.contains("(see export.json)"));
        assert!(!lib.contains("payload/export.json"));
        assert!(!file(&files, "README.md").contains("payload/export.json"));
        assert!(!lib.contains("__APP_ID__"));
    }

    #[test]
    fn launcher_finds_the_daemon_and_fails_loudly_when_it_cannot() {
        let m = manifest(ArtifactKind::Agentic);
        let files = export(&m);
        let lib = file(&files, "biorouter-launch.sh");
        assert!(lib.contains("BIOROUTERD_BIN"));
        assert!(lib.contains("Biorouter.app/Contents/Resources/bin/biorouterd"));
        assert!(
            lib.contains("BIOROUTER_PORT="),
            "must set the daemon port env var biorouterd actually reads"
        );
        assert!(lib.contains("biorouterd not found. Install Biorouter"));
        assert!(lib.contains(r#"BIOROUTERD="$(find_biorouterd)" || die"#));
        // And the runner opens the DAEMON's own origin, not a static server's,
        // through the app's one-time launch link (W2-HRD-1).
        let run = file(&files, "run.sh");
        assert!(run.contains("launch_url\nopen_url \"$LAUNCH_URL\""));
        assert!(lib.contains(r#"LAUNCH_URL="http://127.0.0.1:$PORT$path""#));
        assert!(lib.contains(r#""http://127.0.0.1:$1/apps/$APP_ID/launch""#));
        // It starts its daemon with a secret it keeps, owner-only.
        assert!(lib.contains(
            r#"BIOROUTER_SERVER__SECRET_KEY="$SECRET" BIOROUTER_PORT="$p" "$BIOROUTERD" agent"#
        ));
        assert!(lib.contains("umask 077"));
        // The secret never rides a command line other accounts can read: every
        // curl that carries it reads the header from stdin.
        assert_eq!(lib.matches("-H @-").count(), 2);
        assert!(!lib.contains("-H \"X-Secret-Key"));
        // A daemon that never answers is waited on for 60 s and then stopped,
        // never left running with a secret nobody knows.
        assert!(lib.contains("for _ in $(seq 1 120); do"));
        assert!(lib.contains(r#"kill "$pid" 2>/dev/null || true"#));
        // Its log is private, never a predictable name in a shared /tmp.
        assert!(lib.contains(r#"LOGS="$LAUNCHER_HOME/logs""#));
        assert!(!lib.contains("TMPDIR"));
    }

    /// Write `name` into `dir`, executable.
    #[cfg(unix)]
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// W2-HRD-1: the launcher never puts the one-time link on a command line,
    /// where every account on the machine can read it and redeem it before the
    /// browser does. Runs the real `open_url` with stub openers that record
    /// their arguments, and checks they were handed a page only this account
    /// can read, which forwards to the link, and never the link itself. The
    /// page goes to the default WEB BROWSER when one is known, not to the .html
    /// handler, which on a developer's machine is often an editor.
    #[cfg(unix)]
    #[test]
    fn the_launcher_hands_its_opener_a_private_page_and_never_the_link() {
        use std::os::unix::fs::PermissionsExt;
        use std::process::Command;

        let dir = tempfile::tempdir().unwrap();
        // Every Unix this ships to has bash; a missing one must fail here, not
        // pass without checking anything.
        let bash = find_usable_bash(dir.path()).expect("a usable bash");
        let m = manifest(ArtifactKind::Agentic);
        let files = export(&m);
        std::fs::write(
            dir.path().join("biorouter-launch.sh"),
            file(&files, "biorouter-launch.sh"),
        )
        .unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let log = dir.path().join("opened.log");
        for opener in ["open", "xdg-open", "gtk-launch"] {
            script(
                &bin,
                opener,
                &format!("#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\n", log.display()),
            );
        }
        // No default browser known: the page goes to the system opener.
        let browser = dir.path().join("browser.txt");
        for resolver in ["osascript", "xdg-settings"] {
            script(
                &bin,
                resolver,
                &format!("#!/bin/sh\ncat '{}' 2>/dev/null\n", browser.display()),
            );
        }
        let config = dir.path().join("config");
        let token = "cd".repeat(32);
        let run = |url: &str| {
            Command::new(&bash)
                .arg("-c")
                .arg(format!(
                    "set -euo pipefail\n. ./biorouter-launch.sh\nPORT=4321\nopen_url '{url}'"
                ))
                .current_dir(dir.path())
                .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
                .env("XDG_CONFIG_HOME", &config)
                .env("HOME", dir.path())
                .env_remove("BIOROUTER_PATH_ROOT")
                .output()
                .unwrap()
        };

        let launch = format!("http://127.0.0.1:4321/apps/{}/?t={token}", m.id);
        let out = run(&launch);
        assert!(
            out.status.success(),
            "open_url failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        // The terminal is the user's alone, so the link is shown there for a
        // browser that cannot read the page.
        assert!(String::from_utf8_lossy(&out.stdout).contains(&format!(
            "open this address instead (it works once): {launch}"
        )));
        let opened = std::fs::read_to_string(&log).expect("an opener ran");
        let handed: Vec<&str> = opened.lines().collect();
        assert_eq!(handed.len(), 1, "one argument, the page: {opened}");
        assert!(
            !opened.contains("?t=") && !opened.contains(&token),
            "{opened}"
        );
        let page = std::path::Path::new(handed[0]);
        assert_eq!(
            page.parent(),
            Some(config.join("biorouter/app-launcher/open").as_path())
        );
        let html = std::fs::read_to_string(page).unwrap();
        assert!(
            html.contains(&format!(r#"content="0;url={launch}""#)),
            "{html}"
        );
        assert!(html.contains(&format!(r#"href="{launch}""#)), "{html}");
        let mode =
            |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(page), 0o600, "the page is this account's alone");
        assert_eq!(mode(page.parent().unwrap()), 0o700, "and so is its folder");

        // A known default browser gets the page, by name, and still never the link.
        std::fs::remove_file(&log).unwrap();
        let app = dir.path().join("Some Browser.app");
        std::fs::create_dir(&app).unwrap();
        let named = if cfg!(target_os = "macos") {
            app.display().to_string()
        } else {
            "some-browser.desktop".to_string()
        };
        std::fs::write(&browser, &named).unwrap();
        assert!(run(&launch).status.success());
        let opened = std::fs::read_to_string(&log).expect("an opener ran");
        let handed: Vec<&str> = opened.lines().collect();
        if cfg!(target_os = "macos") {
            assert_eq!(handed[..2], ["-a", named.as_str()], "{opened}");
        } else {
            assert_eq!(handed[0], "some-browser", "{opened}");
        }
        assert!(handed.last().unwrap().ends_with(".html"), "{opened}");
        assert!(!opened.contains(&token), "{opened}");
        std::fs::remove_file(&browser).unwrap();

        // Anything but this app's launch link on this daemon is refused, and no
        // opener runs.
        std::fs::remove_file(&log).unwrap();
        for other in [
            format!("http://127.0.0.1:4321/apps/other/?t={token}"),
            format!("http://127.0.0.1:9999/apps/{}/?t={token}", m.id),
            format!("http://127.0.0.1:4321/apps/{}/?t={}", m.id, "cd".repeat(31)),
            format!("http://127.0.0.1:4321/apps/{}/?t={token}\"><script>", m.id),
            format!(
                "http://127.0.0.1:4321/apps/{}/?t={}",
                m.id,
                token.to_uppercase()
            ),
            "https://example.test/".to_string(),
        ] {
            assert!(!run(&other).status.success(), "{other}");
            assert!(!log.exists(), "{other} reached an opener");
        }
    }

    /// Run `body` after sourcing the launcher library in `dir`, with only
    /// `/usr/bin:/bin` (plus `extra_path`) on PATH and a fake home.
    #[cfg(unix)]
    fn run_launcher(
        dir: &Path,
        home: &Path,
        env: &[(&str, &str)],
        extra_path: Option<&Path>,
        body: &str,
    ) -> std::process::Output {
        let bash = find_usable_bash(dir).expect("a usable bash");
        let path = match extra_path {
            Some(extra) => format!("{}:/usr/bin:/bin", extra.display()),
            None => "/usr/bin:/bin".to_string(),
        };
        let mut command = std::process::Command::new(bash);
        command
            .arg("-c")
            .arg(format!(
                "set -euo pipefail\n. ./biorouter-launch.sh\n{body}"
            ))
            .current_dir(dir)
            .env("PATH", path)
            .env("HOME", home)
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("BIOROUTER_PATH_ROOT")
            .env_remove("BIOROUTER_EXPORT_REPLACE")
            .env_remove("BIOROUTERD_BIN")
            .stdin(std::process::Stdio::null());
        for (key, value) in env {
            command.env(key, value);
        }
        command.output().unwrap()
    }

    /// The launchers install where the daemon looks. They used to guess
    /// `~/.config/biorouter` on every platform and ignore `BIOROUTER_PATH_ROOT`,
    /// while a Windows daemon reads `%APPDATA%\Block\biorouter\config`: every
    /// Windows launch failed with "does not serve". This runs the bash and Node
    /// expressions for each case and holds them to the Rust resolver; the
    /// PowerShell spelling is held to the same app-strategy names.
    #[cfg(unix)]
    #[test]
    fn the_launchers_find_the_store_where_the_daemon_does() {
        let dir = tempfile::tempdir().unwrap();
        let m = manifest(ArtifactKind::Agentic);
        let files = export(&m);
        std::fs::write(
            dir.path().join("biorouter-launch.sh"),
            file(&files, "biorouter-launch.sh"),
        )
        .unwrap();
        let home = dir.path().join("home");
        let fallback = home.join(".config/biorouter");
        let bash_config = |env: &[(&str, &str)]| {
            let out = run_launcher(dir.path(), &home, env, None, "printf '%s' \"$CFG\"");
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).to_string()
        };
        // The override, exactly as the daemon applies it.
        assert_eq!(
            bash_config(&[("BIOROUTER_PATH_ROOT", "/tmp/sandbox")]),
            crate::paths::resolve_config_dir(Some("/tmp/sandbox"), &fallback)
                .display()
                .to_string()
        );
        // A blank override is no override.
        assert_eq!(
            bash_config(&[("BIOROUTER_PATH_ROOT", "   ")]),
            crate::paths::resolve_config_dir(Some("   "), &fallback)
                .display()
                .to_string()
        );
        // XDG_CONFIG_HOME counts only when absolute, as etcetera reads it.
        assert_eq!(
            bash_config(&[("XDG_CONFIG_HOME", "/xdg")]),
            "/xdg/biorouter"
        );
        assert_eq!(
            bash_config(&[("XDG_CONFIG_HOME", "relative/xdg")]),
            fallback.display().to_string()
        );
        assert_eq!(bash_config(&[]), fallback.display().to_string());

        // And under this process's own environment, the real resolver.
        let ambient = std::process::Command::new(find_usable_bash(dir.path()).unwrap())
            .arg("-c")
            .arg("set -euo pipefail\n. ./biorouter-launch.sh\nprintf '%s' \"$CFG\"")
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&ambient.stdout),
            crate::paths::config_dir().display().to_string()
        );

        // run.ps1: the override, then %APPDATA%\<author>\<app>\config.
        let strategy = &*crate::APP_STRATEGY;
        let ps1 = file(&files, "run.ps1");
        assert!(ps1.contains(r#"$Cfg = Join-Path $env:BIOROUTER_PATH_ROOT "config""#));
        assert!(ps1.contains(&format!(
            r#"$Cfg = Join-Path $env:APPDATA "{}\{}\config""#,
            strategy.author, strategy.app_name
        )));
        assert!(!ps1.contains(".config\\biorouter"));
        assert!(!ps1.contains("XDG_CONFIG_HOME"));

        // serve.mjs: its configDir, run by Node for every platform it ships to.
        let Some(node) = find_command("node", dir.path()) else {
            return;
        };
        let serve = file(&files, "serve.mjs");
        let (_, rest) = serve.split_once("// biorouter:config-dir:start\n").unwrap();
        let (function, _) = rest.split_once("// biorouter:config-dir:end").unwrap();
        let probe = format!(
            r#"import {{ win32, posix }} from "node:path";
import {{ homedir }} from "node:os";
{function}
const cases = [
  [{{ BIOROUTER_PATH_ROOT: "/tmp/sandbox" }}, "linux", "/home/u"],
  [{{ BIOROUTER_PATH_ROOT: "   " }}, "darwin", "/Users/u"],
  [{{ XDG_CONFIG_HOME: "/xdg" }}, "linux", "/home/u"],
  [{{ XDG_CONFIG_HOME: "relative/xdg" }}, "linux", "/home/u"],
  [{{}}, "darwin", "/Users/u"],
  [{{ APPDATA: "C:\\Users\\u\\AppData\\Roaming", XDG_CONFIG_HOME: "/xdg" }}, "win32", "C:\\Users\\u"],
  [{{}}, "win32", "C:\\Users\\u"],
  [{{ BIOROUTER_PATH_ROOT: "D:\\sandbox" }}, "win32", "C:\\Users\\u"],
];
console.log(JSON.stringify([...cases.map(([env, platform, home]) => configDir(env, platform, home)), configDir()]));
"#
        );
        let probe_path = dir.path().join("probe.mjs");
        std::fs::write(&probe_path, probe).unwrap();
        let out = std::process::Command::new(node)
            .arg(&probe_path)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let got: Vec<String> = serde_json::from_slice(&out.stdout).unwrap();
        let windows = format!(
            "C:\\Users\\u\\AppData\\Roaming\\{}\\{}\\config",
            strategy.author, strategy.app_name
        );
        assert_eq!(
            got,
            vec![
                "/tmp/sandbox/config".to_string(),
                "/Users/u/.config/biorouter".to_string(),
                "/xdg/biorouter".to_string(),
                "/home/u/.config/biorouter".to_string(),
                "/Users/u/.config/biorouter".to_string(),
                windows.clone(),
                windows,
                "D:\\sandbox\\config".to_string(),
                crate::paths::config_dir().display().to_string(),
            ]
        );
    }

    #[cfg(unix)]
    fn pretty(value: serde_json::Value) -> String {
        serde_json::to_string_pretty(&value).unwrap()
    }

    /// An export folder for app `demo` in `dir`, as the export writes it.
    #[cfg(unix)]
    fn export_folder(dir: &Path, uid: &str, created: u64, updated: u64) {
        let m = manifest(ArtifactKind::Agentic);
        for (rel, content) in launcher_files(&m, None) {
            std::fs::write(dir.join(rel), content).unwrap();
        }
        std::fs::write(
            dir.join("manifest.json"),
            pretty(serde_json::json!({
                "id": "demo", "title": "Demo", "kind": "agentic", "entry": "index.html",
                "created_at": created, "updated_at": updated, "app_uid": uid,
            })),
        )
        .unwrap();
        std::fs::write(dir.join("index.html"), "<html>export</html>").unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.ts"), "// export").unwrap();
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("assets/logo.png"), [0x89, b'P', b'N', b'G', 0xFF]).unwrap();
    }

    /// The launcher ran `rm -rf` over whatever app had the same id, on every
    /// run: an unrelated app with a colliding name, or the author's own live app
    /// reverted to an old snapshot. Now a different app is never replaced unless
    /// someone says so, and then it is moved aside; a newer installed copy of
    /// this app is opened as it is; and the edit, build, re-run loop still syncs.
    #[cfg(unix)]
    #[test]
    fn install_app_never_replaces_another_app_and_keeps_what_it_moves() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("demo");
        std::fs::create_dir_all(&folder).unwrap();
        export_folder(&folder, "uid-1", 100, 200);
        let home = root.path().join("home");
        let store = home.join(".config/biorouter/agent_drafter/demo");
        let install = |env: &[(&str, &str)]| run_launcher(&folder, &home, env, None, "install_app");

        // A fresh machine: installed, byte for byte.
        let out = install(&[]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            std::fs::read(store.join("assets/logo.png")).unwrap(),
            [0x89, b'P', b'N', b'G', 0xFF]
        );
        assert!(store.join("manifest.json").is_file());

        // The same app, unchanged since the export: an edit in the folder syncs,
        // and the store keeps its own manifest (the author's chat link included).
        let mut theirs: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(store.join("manifest.json")).unwrap())
                .unwrap();
        theirs["session_id"] = serde_json::json!("20261009_3");
        std::fs::write(store.join("manifest.json"), pretty(theirs.clone())).unwrap();
        std::fs::write(folder.join("src/main.ts"), "// edited").unwrap();
        assert!(install(&[]).status.success());
        assert_eq!(
            std::fs::read_to_string(store.join("src/main.ts")).unwrap(),
            "// edited"
        );
        assert!(std::fs::read_to_string(store.join("manifest.json"))
            .unwrap()
            .contains("20261009_3"));

        // The same app, changed after the export: opened as it is.
        theirs["updated_at"] = serde_json::json!(300);
        std::fs::write(store.join("manifest.json"), pretty(theirs.clone())).unwrap();
        std::fs::write(store.join("index.html"), "<html>newer</html>").unwrap();
        let out = install(&[]);
        assert!(out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("changed after this folder was exported")
        );
        assert_eq!(
            std::fs::read_to_string(store.join("index.html")).unwrap(),
            "<html>newer</html>"
        );

        // A different app under the same id: refused without a yes...
        theirs["app_uid"] = serde_json::json!("uid-other");
        std::fs::write(store.join("manifest.json"), pretty(theirs)).unwrap();
        std::fs::create_dir_all(store.join("workspace")).unwrap();
        std::fs::write(store.join("workspace/data.csv"), "theirs").unwrap();
        let out = install(&[]);
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("BIOROUTER_EXPORT_REPLACE=1"));
        assert_eq!(
            std::fs::read_to_string(store.join("workspace/data.csv")).unwrap(),
            "theirs"
        );
        // ...and with one, moved to backups whole, workspace and all.
        let out = install(&[("BIOROUTER_EXPORT_REPLACE", "1")]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !store.join("workspace").exists(),
            "the new app inherits nothing"
        );
        assert_eq!(
            std::fs::read_to_string(store.join("index.html")).unwrap(),
            "<html>export</html>"
        );
        let backups = home.join(".config/biorouter/app-launcher/backups");
        let saved: Vec<_> = std::fs::read_dir(&backups).unwrap().flatten().collect();
        assert_eq!(saved.len(), 1);
        assert_eq!(
            std::fs::read_to_string(saved[0].path().join("demo/workspace/data.csv")).unwrap(),
            "theirs"
        );
    }

    /// An app folder holding every case the digest has a rule for: an entry
    /// in a subfolder, a path with a space, a binary file, a `.DS_Store`, and a
    /// `dist/` that counts only without `src/main.ts`.
    #[cfg(unix)]
    fn digest_fixture(dir: &Path, with_main: bool) {
        let write = |rel: &str, bytes: &[u8]| {
            let path = dir.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        };
        write("pages/index.html", b"<html>entry</html>");
        write("src/sdk.ts", b"// sdk");
        write("src/deep/a b.ts", b"export const x = 1;\n");
        write("assets/logo.png", &[0x89, b'P', b'N', b'G', 0x00, 0xFF]);
        write("assets/.DS_Store", b"finder");
        write("dist/app.js", b"// bundle");
        write("workspace/data.csv", b"runtime data");
        write("manifest.json", b"{}");
        if with_main {
            write("src/main.ts", b"import './sdk';");
        }
    }

    /// The export writes `runtime_digest` into the launchers, and they compare
    /// it with their own reading of the store, so every spelling must agree to
    /// the byte, or an untouched install reads as changed and the edit, build,
    /// re-run loop stops. Bash and Node are run here; PowerShell's spelling is
    /// held to the same rules by text.
    #[cfg(unix)]
    #[test]
    fn the_launchers_digest_the_store_exactly_as_the_export_does() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("folder");
        std::fs::create_dir_all(&folder).unwrap();
        let m = manifest(ArtifactKind::Agentic);
        let files = launcher_files(&m, Some("ab12"));
        std::fs::write(
            folder.join("biorouter-launch.sh"),
            file(&files, "biorouter-launch.sh"),
        )
        .unwrap();
        assert!(file(&files, "biorouter-launch.sh").contains("EXPORT_DIGEST=\"ab12\""));
        assert!(file(&files, "run.ps1").contains("$ExportDigest = \"ab12\""));
        assert!(file(&files, "serve.mjs").contains("const EXPORT_DIGEST = \"ab12\";"));
        // Only hex reaches a launcher, whatever it is handed.
        let hostile = launcher_files(&m, Some("ab\"; rm -rf ~; \"12"));
        assert!(file(&hostile, "biorouter-launch.sh").contains("EXPORT_DIGEST=\"abf12\""));

        let home = root.path().join("home");
        let node = find_command("node", root.path());
        let node_digest = |app: &Path| -> Option<String> {
            let node = node.as_ref()?;
            let serve = file(&files, "serve.mjs");
            let (_, rest) = serve
                .split_once("// biorouter:runtime-digest:start\n")
                .unwrap();
            let (function, _) = rest.split_once("// biorouter:runtime-digest:end").unwrap();
            let probe = format!(
                "import {{ lstatSync, readdirSync, readFileSync }} from \"node:fs\";\n\
                 import {{ createHash }} from \"node:crypto\";\n\
                 import {{ join }} from \"node:path\";\n{function}\n\
                 process.stdout.write(runtimeDigest(process.argv[2], \"pages/index.html\"));\n"
            );
            let probe_path = root.path().join("digest-probe.mjs");
            std::fs::write(&probe_path, probe).unwrap();
            let out = std::process::Command::new(node)
                .arg(&probe_path)
                .arg(app)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            Some(String::from_utf8_lossy(&out.stdout).to_string())
        };
        let bash_digest = |app: &Path| {
            let out = run_launcher(
                &folder,
                &home,
                &[],
                None,
                &format!("ENTRY=pages/index.html\nruntime_digest '{}'", app.display()),
            );
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };

        for with_main in [true, false] {
            let app = root.path().join(format!("app-{with_main}"));
            digest_fixture(&app, with_main);
            let rust = runtime_digest(&app, "pages/index.html").unwrap();
            assert_eq!(rust.len(), 64);
            assert_eq!(
                bash_digest(&app),
                rust,
                "bash, with src/main.ts: {with_main}"
            );
            if let Some(js) = node_digest(&app) {
                assert_eq!(js, rust, "node, with src/main.ts: {with_main}");
            }

            // What does not count: the runtime data, the manifest (the daemon
            // restamps it on every build), Finder's files, and a bundle the
            // daemon derives from src/main.ts.
            std::fs::write(app.join("workspace/data.csv"), "changed").unwrap();
            std::fs::write(app.join("manifest.json"), "{\"built_at\": 1}").unwrap();
            std::fs::write(app.join("assets/.DS_Store"), "changed").unwrap();
            let bundle_changed = {
                std::fs::write(app.join("dist/app.js"), "// rebuilt").unwrap();
                runtime_digest(&app, "pages/index.html").unwrap()
            };
            assert_eq!(
                bundle_changed == rust,
                with_main,
                "dist/ counts only without src/main.ts"
            );
            assert_eq!(bash_digest(&app), bundle_changed);

            // What does: an edit anywhere in src/, assets/ or the entry.
            for rel in ["src/deep/a b.ts", "assets/logo.png", "pages/index.html"] {
                let before = runtime_digest(&app, "pages/index.html").unwrap();
                std::fs::write(app.join(rel), "edited in place").unwrap();
                let after = runtime_digest(&app, "pages/index.html").unwrap();
                assert_ne!(before, after, "{rel}");
                assert_eq!(bash_digest(&app), after, "{rel}");
                if let Some(js) = node_digest(&app) {
                    assert_eq!(js, after, "{rel}");
                }
            }
        }

        // run.ps1 follows the same rules.
        let ps1 = file(&files, "run.ps1");
        for rule in [
            "function Get-RuntimeDigest",
            r#"foreach ($sub in @("src", "assets", "dist"))"#,
            r#"if ($sub -eq "dist" -and $builtFromSource) { continue }"#,
            r#"$_.Name -ne ".DS_Store""#,
            r#"$lines.Add("$hash $rel`n")"#,
            "[Array]::Sort($sorted, [StringComparer]::Ordinal)",
            "Write-InstallRecord $entry",
        ] {
            assert!(ps1.contains(rule), "run.ps1 lacks {rule}");
        }
    }

    /// An export of app `demo` made from `store` (its current contents), as
    /// the export writes it: the store's own files, the launchers carrying the
    /// store's digest, and the manifest without `session_id`.
    #[cfg(unix)]
    fn export_of_store(store: &Path, folder: &Path) {
        std::fs::create_dir_all(folder).unwrap();
        let digest = runtime_digest(store, "index.html");
        assert!(digest.is_some());
        for (rel, content) in launcher_files(&manifest(ArtifactKind::Agentic), digest.as_deref()) {
            std::fs::write(folder.join(rel), content).unwrap();
        }
        let mut exported: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(store.join("manifest.json")).unwrap())
                .unwrap();
        exported.as_object_mut().unwrap().remove("session_id");
        std::fs::write(folder.join("manifest.json"), pretty(exported)).unwrap();
        for rel in ["index.html", "src/main.ts", "src/sdk.ts", "dist/app.js"] {
            std::fs::create_dir_all(folder.join(rel).parent().unwrap()).unwrap();
            std::fs::copy(store.join(rel), folder.join(rel)).unwrap();
        }
    }

    /// Re-syncing trusted an equal `updated_at`, which only the drafter's own
    /// tools bump. So an edit made in the installed copy with the developer
    /// shell or a text editor was reverted by the next run of an old export,
    /// and a newer Biorouter's rebuild (which refreshes src/sdk.ts and stamps
    /// the manifest) was undone into an old runtime under the new stamp, which
    /// the daemon then never rebuilt. Now an install is re-synced only while it
    /// is what this folder was exported from or what a launcher last put there,
    /// and a re-sync leaves the store's own bundle for the daemon to rebuild.
    #[cfg(unix)]
    #[test]
    fn an_install_changed_where_it_lives_is_never_resynced_over() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let cfg = home.join(".config/biorouter");
        let store = cfg.join("agent_drafter/demo");
        let write = |path: &Path, text: &str| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        let read = |path: &Path| std::fs::read_to_string(path).unwrap();
        // The author's app, as the store holds it.
        write(
            &store.join("manifest.json"),
            &pretty(serde_json::json!({
                "id": "demo", "title": "Demo", "kind": "agentic", "entry": "index.html",
                "created_at": 100, "updated_at": 200, "app_uid": "uid-1",
                "built_at": 210, "sdk_hash": "sdk-n", "session_id": "20261009_3",
            })),
        );
        write(&store.join("index.html"), "<html>v1</html>");
        write(&store.join("src/main.ts"), "// v1");
        write(&store.join("src/sdk.ts"), "// sdk n");
        write(&store.join("dist/app.js"), "// bundle of v1 on sdk n");
        let folder = root.path().join("demo");
        export_of_store(&store, &folder);
        let install = |env: &[(&str, &str)]| {
            let out = run_launcher(&folder, &home, env, None, "install_app");
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).to_string()
        };

        // (a) Edited in place after the export, with no `updated_at` bump.
        write(&store.join("src/main.ts"), "// edited with text_editor");
        let said = install(&[]);
        assert!(said.contains("changed where it is installed"), "{said}");
        assert_eq!(
            read(&store.join("src/main.ts")),
            "// edited with text_editor"
        );

        // (b) A newer Biorouter rebuilt it: a new SDK and its stamp.
        write(&store.join("src/main.ts"), "// v1");
        write(&store.join("src/sdk.ts"), "// sdk n+1");
        write(&store.join("dist/app.js"), "// bundle of v1 on sdk n+1");
        let said = install(&[]);
        assert!(said.contains("changed where it is installed"), "{said}");
        assert_eq!(read(&store.join("src/sdk.ts")), "// sdk n+1");
        assert_eq!(
            read(&store.join("dist/app.js")),
            "// bundle of v1 on sdk n+1"
        );

        // Asked to, it replaces the installed copy and keeps a backup of it,
        // but still keeps the store's manifest (the author's chat link) and
        // so its bundle: the daemon rebuilds from the src/ installed here.
        let said = install(&[("BIOROUTER_EXPORT_REPLACE", "1")]);
        assert!(said.contains("Kept a copy"), "{said}");
        assert_eq!(read(&store.join("src/sdk.ts")), "// sdk n");
        assert_eq!(
            read(&store.join("dist/app.js")),
            "// bundle of v1 on sdk n+1"
        );
        assert!(read(&store.join("manifest.json")).contains("20261009_3"));
        let backups: Vec<_> = std::fs::read_dir(cfg.join("app-launcher/backups"))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(read(&backups[0].path().join("src/sdk.ts")), "// sdk n+1");

        // The loop: what a launcher installed is recorded, so the next edit in
        // the folder syncs, and so does one after the daemon rebuilt dist/.
        write(&folder.join("src/main.ts"), "// v2");
        write(&folder.join("dist/app.js"), "// npm run build of v2");
        assert!(!install(&[]).contains("changed where"));
        assert_eq!(read(&store.join("src/main.ts")), "// v2");
        assert_eq!(
            read(&store.join("dist/app.js")),
            "// bundle of v1 on sdk n+1",
            "a kept stamp keeps its bundle; the daemon rebuilds from src/"
        );
        write(&store.join("dist/app.js"), "// the daemon's bundle of v2");
        write(&folder.join("src/main.ts"), "// v3");
        assert!(!install(&[]).contains("changed where"));
        assert_eq!(read(&store.join("src/main.ts")), "// v3");
        assert!(cfg.join("app-launcher/installed/demo").is_file());

        // An app with no src/main.ts has no bundle the daemon derives, so its
        // dist/ is synced like the rest.
        std::fs::remove_file(folder.join("src/main.ts")).unwrap();
        write(&folder.join("dist/app.js"), "// hand-written");
        assert!(!install(&[]).contains("changed where"));
        assert_eq!(read(&store.join("dist/app.js")), "// hand-written");
    }

    /// A full export used to carry a skill Biorouter ships (`office-word`), and
    /// the launcher installed it over the recipient's own seeded copy. Exports
    /// leave them out now; a launcher handed one anyway skips it.
    #[cfg(unix)]
    #[test]
    fn install_payload_never_installs_over_a_skill_biorouter_ships() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("demo");
        std::fs::create_dir_all(folder.join("payload/skills/office-word")).unwrap();
        std::fs::create_dir_all(folder.join("payload/skills/mine")).unwrap();
        export_folder(&folder, "uid-1", 100, 200);
        std::fs::write(
            folder.join("payload/skills/office-word/SKILL.md"),
            "authors",
        )
        .unwrap();
        std::fs::write(folder.join("payload/skills/mine/SKILL.md"), "authors").unwrap();
        let home = root.path().join("home");
        let cfg = home.join(".config/biorouter");
        std::fs::create_dir_all(cfg.join("skills/office-word")).unwrap();
        std::fs::write(cfg.join("skills/office-word/SKILL.md"), "seeded").unwrap();

        let out = run_launcher(
            &folder,
            &home,
            &[("BIOROUTER_EXPORT_YES", "1")],
            None,
            "install_payload",
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let said = String::from_utf8_lossy(&out.stdout);
        assert!(said.contains("skipping skill office-word"), "{said}");
        assert_eq!(
            std::fs::read_to_string(cfg.join("skills/office-word/SKILL.md")).unwrap(),
            "seeded"
        );
        assert_eq!(
            std::fs::read_to_string(cfg.join("skills/mine/SKILL.md")).unwrap(),
            "authors"
        );
        let ps1 = file(
            &launcher_files(&manifest(ArtifactKind::Agentic), None),
            "run.ps1",
        )
        .to_string();
        assert!(ps1.contains(r#"$shipped = @("office-word", "office-powerpoint""#));
    }

    /// The payload lists were space-separated strings expanded unquoted, so a
    /// folder under "My Apps" read as two paths. They are arrays now.
    #[cfg(unix)]
    #[test]
    fn install_payload_works_from_a_folder_with_a_space_and_keeps_your_skill() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("My Apps").join("demo");
        std::fs::create_dir_all(folder.join("payload/skills/s1")).unwrap();
        std::fs::create_dir_all(folder.join("payload/knowledge")).unwrap();
        export_folder(&folder, "uid-1", 100, 200);
        std::fs::write(folder.join("payload/skills/s1/SKILL.md"), "shipped").unwrap();
        std::fs::write(folder.join("payload/knowledge/k.brkb"), "kb").unwrap();
        let home = root.path().join("home");
        let cfg = home.join(".config/biorouter");
        std::fs::create_dir_all(cfg.join("skills/s1")).unwrap();
        std::fs::write(cfg.join("skills/s1/SKILL.md"), "mine").unwrap();

        let out = run_launcher(
            &folder,
            &home,
            &[("BIOROUTER_EXPORT_YES", "1")],
            None,
            "install_payload",
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let said = String::from_utf8_lossy(&out.stdout);
        assert!(said.contains("  - skill: s1"), "{said}");
        assert!(!said.contains("skill: My"), "{said}");
        assert_eq!(
            std::fs::read_to_string(cfg.join("skills/s1/SKILL.md")).unwrap(),
            "shipped"
        );
        assert!(cfg.join("knowledge-imports/k.brkb").is_file());
        assert!(!cfg.join("skills/My").exists());
        let backups: Vec<_> = std::fs::read_dir(cfg.join("app-launcher/backups"))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(
            std::fs::read_to_string(backups[0].path().join("s1/SKILL.md")).unwrap(),
            "mine",
            "the skill you had is kept"
        );
    }

    /// A fat export's daemon is used only when it runs on this machine.
    #[cfg(unix)]
    #[test]
    fn a_bundled_daemon_that_cannot_run_here_is_passed_over() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("demo");
        std::fs::create_dir_all(folder.join("payload/bin")).unwrap();
        export_folder(&folder, "uid-1", 100, 200);
        let installed = root.path().join("installed");
        std::fs::create_dir_all(&installed).unwrap();
        script(&installed, "biorouterd", "#!/bin/sh\nexit 0\n");
        let bundled = script(
            &folder.join("payload/bin"),
            "biorouterd",
            "#!/bin/sh\nexit 1\n",
        );
        let home = root.path().join("home");

        let out = run_launcher(&folder, &home, &[], Some(&installed), "find_biorouterd");
        assert!(out.status.success());
        let found = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        assert_eq!(
            found.canonicalize().unwrap(),
            installed.join("biorouterd").canonicalize().unwrap()
        );

        script(
            &folder.join("payload/bin"),
            "biorouterd",
            "#!/bin/sh\nexit 0\n",
        );
        let out = run_launcher(&folder, &home, &[], Some(&installed), "find_biorouterd");
        // bash reports the folder by its physical path, so compare real paths.
        let found = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        assert_eq!(
            found.canonicalize().unwrap(),
            bundled.canonicalize().unwrap()
        );
    }

    /// The export tests otherwise only match substrings. This actually *parses*
    /// the generated `serve.mjs` (ESM + top-level await) and shell launchers, so
    /// a template edit that produces a syntax error can't ship silently. Checks
    /// each tool only when a usable installation is available.
    #[test]
    fn generated_serve_and_launchers_are_syntactically_valid() {
        use std::io::Write;
        let m = manifest(ArtifactKind::Agentic);
        let files = export(&m);
        let dir = tempfile::tempdir().unwrap();

        let write = |name: &str| {
            let p = dir.path().join(name);
            let mut f = std::fs::File::create(&p).unwrap();
            f.write_all(file(&files, name).as_bytes()).unwrap();
            p
        };
        let serve = write("serve.mjs");
        let run = file(&files, "run.sh");
        let lib = file(&files, "biorouter-launch.sh");

        use std::process::Command;

        #[cfg(not(windows))]
        let node = find_command("node", dir.path());
        #[cfg(windows)]
        let node = Some(PathBuf::from("node")).filter(|node| command_has_version(node, dir.path()));
        if let Some(node) = node {
            // `node --check` infers ESM from the `.mjs` extension, so top-level
            // await parses. (`--input-type` is stdin/eval-only, not for a file.)
            let out = Command::new(&node)
                .arg("--check")
                .arg(&serve)
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "generated serve.mjs is not valid ESM:\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        if let Some(bash) = find_usable_bash(dir.path()) {
            for (name, launcher) in [("run.sh", run), ("biorouter-launch.sh", lib)] {
                // Parse the same on-disk artifact users run. A relative POSIX
                // path works for both Unix Bash and Git Bash, without handing
                // Bash a native Windows temp path or piping its program text.
                std::fs::write(dir.path().join(name), launcher).unwrap();
                let out = Command::new(&bash)
                    .arg("-n")
                    .arg(format!("./{name}"))
                    .current_dir(dir.path())
                    .output()
                    .unwrap();
                assert!(
                    out.status.success(),
                    "generated launcher {} is not valid bash ({}):\nstdout:\n{}\nstderr:\n{}",
                    name,
                    out.status,
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
            }

            // `bash -n` only checks syntax. The launcher runs under `set -u`, and
            // `find_biorouterd` once referenced `$BIOROUTERD_BIN` unguarded — so
            // with that var UNSET (the common case: biorouterd on PATH) the whole
            // launcher aborted with "unbound variable" and the app never opened,
            // while every substring test stayed green. Actually EXECUTE it under
            // `set -u`, var unset, with a stub `biorouterd` on PATH.
            let stub_dir = dir.path().join("stub");
            std::fs::create_dir_all(&stub_dir).unwrap();
            let stub = stub_dir.join("biorouterd");
            std::fs::write(&stub, "#!/bin/sh\nexit 0\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            // The program goes to Bash as a file, like the syntax check above. Passed with
            // `-c`, the whole launcher crosses the Windows command line, and Git Bash's
            // argument parsing re-reads the quotes and glob characters in it: the launch-page
            // code made it report "unexpected EOF while looking for matching `\"'" there,
            // while `bash -n` of the same text on disk passed.
            let script =
                format!("set -euo pipefail\nchmod +x ./biorouterd\n{lib}\nfind_biorouterd\n");
            std::fs::write(stub_dir.join("check-find-biorouterd.sh"), script).unwrap();
            let out = Command::new(&bash)
                .arg("./check-find-biorouterd.sh")
                .current_dir(&stub_dir)
                .env_remove("BIOROUTERD_BIN")
                .env_remove("XDG_CONFIG_HOME")
                .env_remove("BIOROUTER_PATH_ROOT")
                .env("DIR", ".")
                .env("PATH", ".:/usr/bin:/bin")
                .env("HOME", ".")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "find_biorouterd aborts under `set -u` with BIOROUTERD_BIN unset:\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&out.stdout).trim(),
                "./biorouterd",
                "find_biorouterd should locate the on-PATH biorouterd"
            );
        }
    }

    /// `serve.mjs` run for real, against a stand-in daemon: it answers only its
    /// own loopback `Host`, serves nothing of the folder but the app's `dist/`
    /// and `assets/` (never `export.json` or `payload/`), keeps a path that
    /// climbs out of `dist/` in, answers a malformed escape `400` instead of
    /// crashing, and sends `/` to the daemon's page rather than to a page with
    /// no socket token. The string checks above say the code is there; this
    /// says it works.
    #[cfg(unix)]
    #[test]
    fn serve_mjs_serves_only_the_app_and_only_to_its_own_host() {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::os::unix::fs::PermissionsExt;
        use std::process::{Command, Stdio};

        let dir = tempfile::tempdir().unwrap();
        let Some(node) = find_command("node", dir.path()) else {
            return;
        };
        // Two free ports from the range these tests may use.
        let free_port = |skip: u16| -> u16 {
            let start = 47_000 + (std::process::id() % 900) as u16;
            (0..1000u16)
                .map(|i| 47_000 + (start - 47_000 + i) % 1000)
                .find(|&port| {
                    port != skip && std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
                })
                .expect("a free port in 47000-47999")
        };
        let daemon_port = free_port(0);
        let serve_port = free_port(daemon_port);

        let folder = dir.path().join("My Apps").join("demo");
        let root = dir.path().join("root");
        std::fs::create_dir_all(folder.join("dist")).unwrap();
        std::fs::create_dir_all(folder.join("payload")).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        let m = manifest(ArtifactKind::Agentic);
        for (name, content) in launcher_files(&m, None) {
            std::fs::write(folder.join(name), content).unwrap();
        }
        std::fs::write(folder.join("manifest.json"), exported_manifest_json(&m)).unwrap();
        std::fs::write(folder.join("index.html"), "<html><body>raw</body></html>").unwrap();
        std::fs::write(folder.join("dist/app.js"), "console.log('bundle');").unwrap();
        std::fs::write(folder.join("export.json"), "{\"secret\":true}").unwrap();
        std::fs::write(folder.join("payload/notes.txt"), "payload").unwrap();

        // A stand-in daemon: it knows the secret it was started with, mints the
        // app's launch path, and records its pid so the test can stop it.
        let pid_file = dir.path().join("daemon.pid");
        let daemon = dir.path().join("fake-biorouterd");
        std::fs::write(
            &daemon,
            format!(
                r#"#!{node}
const http = require("node:http");
require("node:fs").writeFileSync({pid:?}, String(process.pid));
setTimeout(() => process.exit(0), 120000).unref();
const secret = process.env.BIOROUTER_SERVER__SECRET_KEY;
http.createServer((req, res) => {{
  if (req.headers["x-secret-key"] !== secret) {{ res.writeHead(401); return res.end(); }}
  if (req.url === "/apps") return res.end("[]");
  if (req.method === "POST" && req.url === "/apps/demo/launch") {{
    res.setHeader("content-type", "application/json");
    return res.end(JSON.stringify({{ path: "/apps/demo/?t=" + "ab".repeat(32) }}));
  }}
  res.writeHead(404); res.end();
}}).listen(Number(process.env.BIOROUTER_PORT), "127.0.0.1");
"#,
                node = node.display(),
                pid = pid_file.display().to_string(),
            ),
        )
        .unwrap();
        std::fs::set_permissions(&daemon, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut serve = Command::new(&node)
            .arg(folder.join("serve.mjs"))
            .current_dir(&folder)
            .env("BIOROUTER_PATH_ROOT", &root)
            .env("BIOROUTERD_BIN", &daemon)
            .env("BIOROUTERD_PORT", daemon_port.to_string())
            .env("PORT", serve_port.to_string())
            .env_remove("BIOROUTER_SERVER__SECRET_KEY")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stop = |serve: &mut std::process::Child| {
            let _ = serve.kill();
            let _ = serve.wait();
            if let Ok(pid) = std::fs::read_to_string(&pid_file) {
                let _ = Command::new("kill").arg(pid.trim()).status();
            }
        };
        // The port it really bound: it moves on to the next one when its own is
        // taken by the time it binds, and asking the first would then ask
        // whatever took it.
        let mut lines = BufReader::new(serve.stdout.take().unwrap()).lines();
        let mut bound: Option<u16> = None;
        for line in lines.by_ref().map_while(Result::ok) {
            if let Some(rest) = line.strip_prefix("App running at http://127.0.0.1:") {
                bound = rest
                    .split_whitespace()
                    .next()
                    .and_then(|port| port.parse().ok());
                break;
            }
        }
        let Some(serve_port) = bound else {
            let mut err = String::new();
            let _ = serve.stderr.take().unwrap().read_to_string(&mut err);
            stop(&mut serve);
            panic!("serve.mjs did not come up:\n{err}");
        };

        let get = |path: &str, host: &str| -> (u16, String) {
            let mut stream = std::net::TcpStream::connect(("127.0.0.1", serve_port)).unwrap();
            write!(
                stream,
                "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let mut answer = String::new();
            let _ = stream.read_to_string(&mut answer);
            let status = answer
                .split_whitespace()
                .nth(1)
                .and_then(|code| code.parse().ok())
                .unwrap_or(0);
            (status, answer)
        };
        let own = format!("127.0.0.1:{serve_port}");
        let results = [
            get("/", &own),
            get("/apps/demo/dist/app.js", &own),
            get("/apps/demo/dist/%E0%A4%A", &own),
            get("/apps/demo/dist/..%2f..%2fexport.json", &own),
            get("/export.json", &own),
            get("/payload/notes.txt", &own),
            get(
                "/apps/demo/dist/app.js",
                &format!("rebound.example:{serve_port}"),
            ),
            get("/apps/demo/dist/app.js", &format!("localhost:{serve_port}")),
        ];
        stop(&mut serve);

        let [root_page, bundle, malformed, climbing, export_json, payload, rebound, localhost] =
            results;
        assert_eq!(root_page.0, 302, "{}", root_page.1);
        assert!(
            root_page.1.contains("Location: /apps/demo/\r\n"),
            "{}",
            root_page.1
        );
        assert!(
            !root_page.1.contains("?t="),
            "the one-time link is never handed to `/`"
        );
        assert_eq!(bundle.0, 200, "{}", bundle.1);
        assert!(bundle.1.contains("console.log('bundle');"), "{}", bundle.1);
        assert_eq!(malformed.0, 400, "{}", malformed.1);
        assert_eq!(climbing.0, 403, "{}", climbing.1);
        assert_eq!(export_json.0, 404, "{}", export_json.1);
        assert_eq!(payload.0, 404, "{}", payload.1);
        assert_eq!(rebound.0, 421, "{}", rebound.1);
        assert_eq!(localhost.0, 200, "{}", localhost.1);
        // It installed the raw page into the store the daemon reads.
        assert_eq!(
            std::fs::read_to_string(root.join("config/agent_drafter/demo/index.html")).unwrap(),
            "<html><body>raw</body></html>"
        );
    }
}
