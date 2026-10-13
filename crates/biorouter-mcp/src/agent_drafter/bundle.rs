//! Build pipeline for Agent Drafter apps.
//!
//! Apps are authored in TypeScript (`src/main.ts` importing `src/sdk.ts`) and
//! bundled into a single browser-runnable `dist/app.js` with **esbuild**. esbuild
//! is located via (in order, see [`find_esbuild`]): `$BIOROUTER_ESBUILD_BIN`, the
//! copy the desktop app ships beside `biorouterd`, the checkout's
//! `ui/desktop/node_modules/.bin/esbuild` (dev tree), `esbuild` on the process
//! or login-shell `PATH`, then `npx --yes esbuild@<ESBUILD_VERSION>`.
//!
//! When no esbuild is available (e.g. a headless CLI install with no Node), a
//! vendored type-stripper builds the simple shapes it can prove it handles and
//! REFUSES the rest, saying so. ⚠ It is a last resort, not a second compiler: it
//! used to accept everything, which shipped broken bundles that looked built.
//! See [`fallback_bundle`] for what it refuses and why.
//!
//! Every bundle this module writes starts with a one-line banner
//! ([`BundleBanner`]) naming the builder, the App SDK it carries and the
//! Biorouter that built it. That is how a daemon tells a fallback bundle from a
//! real one, and never rebuilds an app down to an older SDK.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::agent_drafter::store::{ArtifactKind, Manifest};

// ---------------------------------------------------------------------------
// Build-time guardrail harness
// ---------------------------------------------------------------------------
//
// Rather than only handing the model UI templates, we wrap app authoring in a
// harness that *validates whatever the model generates* and feeds actionable
// findings back so it can self-correct. The harness enforces three things on
// the LLM's free-form output: (1) it reaches the Biorouter backend through the
// App SDK / agent protocol, (2) it is self-contained (no external/CDN assets
// that break portability), and (3) it stays aesthetically aligned with the
// Biorouter design system (uses `br-*` classes + tokens, not ad-hoc CSS).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintLevel {
    Error,
    Warn,
}

#[derive(Debug, Clone)]
pub struct LintFinding {
    pub level: LintLevel,
    pub msg: String,
}

/// Validate a built app's authored files against the harness guardrails.
/// Returns findings (Errors block readiness; Warns are nudges). Static source
/// analysis — safe to run on every build/preview.
#[allow(clippy::too_many_lines)]
pub fn lint_app(project_dir: &Path) -> Vec<LintFinding> {
    let mut out: Vec<LintFinding> = Vec::new();
    // Free functions rather than closures: two capturing closures over `out`
    // would be a double mutable borrow.
    fn error(out: &mut Vec<LintFinding>, m: &str) {
        out.push(LintFinding {
            level: LintLevel::Error,
            msg: m.to_string(),
        });
    }
    fn warning(out: &mut Vec<LintFinding>, m: &str) {
        out.push(LintFinding {
            level: LintLevel::Warn,
            msg: m.to_string(),
        });
    }
    let index = std::fs::read_to_string(project_dir.join("index.html")).unwrap_or_default();
    let main = std::fs::read_to_string(project_dir.join("src/main.ts")).unwrap_or_default();
    let il = index.to_lowercase();
    // The manifest carries the agent's system prompt, which is where an app says
    // *how* the agent should drive the UI — so the visual/UI checks below have to
    // see it, not just the markup.
    let manifest: Option<Manifest> = std::fs::read_to_string(project_dir.join("manifest.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    out.extend(super::page_check::findings(
        project_dir,
        manifest.as_ref().map_or("index.html", |m| m.entry.as_str()),
        true,
    ));
    let is_agentic = manifest
        .as_ref()
        .is_none_or(|manifest| manifest.kind == ArtifactKind::Agentic);
    let system_prompt = manifest
        .as_ref()
        .and_then(|m| m.agent.as_ref())
        .map(|a| a.system_prompt.to_lowercase())
        .unwrap_or_default();
    let ui_enabled = manifest
        .as_ref()
        .and_then(|m| m.agent.as_ref())
        .map(|a| a.capabilities.ui.enabled)
        .unwrap_or(true);

    // (2) Self-contained — no external/CDN assets.
    if il.contains("src=\"http") || il.contains("src='http") {
        error(&mut out, "index.html loads an external <script src=\"http…\">. Apps must be self-contained: remove it and use the Biorouter App SDK instead.");
    }
    if il.contains("<link") && (il.contains("href=\"http") || il.contains("href='http")) {
        error(&mut out, "index.html links an external stylesheet. Remove it; the Biorouter design system is injected automatically.");
    }
    for cdn in ["cdn.", "unpkg.com", "jsdelivr", "googleapis", "cdnjs"] {
        if il.contains(cdn) {
            error(&mut out, &format!(
                "index.html references a CDN ('{cdn}'). Remove external assets; exported apps must run offline."
            ));
            break;
        }
    }
    // Inline `<script>` blocks and `on*=` handlers the author wrote in
    // index.html are allowed: the daemon pins each script by hash in the served
    // page's policy and moves the handlers into a script it binds itself
    // (`render::inline_script_sources`, `render::bind_inline_handlers`). A
    // `javascript:` URL is neither, so nothing allows it and the browser
    // refuses it, while the chat preview (which runs under a looser policy)
    // still follows it. Warn rather than error: the rest of the page works.
    let inline = crate::agent_drafter::render::authored_inline_code(&index);
    if inline.javascript_urls > 0 {
        warning(&mut out, "index.html uses a `javascript:` URL (href/src/action). The launched app blocks it, even though the chat preview may follow it. Use a <button> wired with addEventListener in src/main.ts, or an inline onclick handler, instead.");
    }

    // (3) Theme-adaptive text: a hardcoded text color (a hex/rgb literal in a
    // `color:` that isn't a `var(--br-*)`) won't adapt when the app is themed,
    // so it goes invisible (dark-on-dark / light-on-light). Warn so the model
    // switches to the design tokens. Heuristic: look for `color:#…`/`color:rgb`
    // that isn't immediately a `var(`.
    let hardcoded_text_color = {
        let hay = index.replace(' ', "");
        let hay_l = hay.to_lowercase();
        let mut hit = false;
        for pat in ["color:#", "color:rgb", "color:hsl"] {
            for (at, _) in hay_l.match_indices(pat) {
                // skip `background-color`/`border-color` — only bare `color:`
                let preceded_by_dash = at > 0 && hay_l.as_bytes()[at - 1] == b'-';
                if !preceded_by_dash {
                    hit = true;
                    break;
                }
            }
            if hit {
                break;
            }
        }
        hit
    };
    if hardcoded_text_color {
        warning(&mut out, "index.html hardcodes a text color (color:#… / color:rgb(…)). It won't adapt to the app theme and can go invisible (dark-on-dark). Use the design tokens: color: var(--br-text) for body text, var(--br-text-muted) for secondary text.");
    }
    // A *surface* token used as a text color (color:var(--br-muted|bg|surface|
    // medium|strong)) is near-background in both themes → invisible. The intended
    // text tokens are --br-text / --br-text-muted.
    let surface_as_text = {
        let hay = index.replace(' ', "").to_lowercase();
        [
            "color:var(--br-muted)",
            "color:var(--br-bg)",
            "color:var(--br-surface)",
            "color:var(--br-medium)",
            "color:var(--br-strong)",
        ]
        .iter()
        .any(|p| hay.contains(p))
    };
    if surface_as_text {
        warning(&mut out, "index.html uses a SURFACE token as a text color (e.g. color: var(--br-muted)). Surface tokens are near-background in both themes, so the text is invisible. Use color: var(--br-text) or var(--br-text-muted).");
    }

    // (1) Backend wiring through the App SDK / agent protocol.
    if is_agentic && !main.contains("./sdk") {
        error(&mut out, "src/main.ts must `import { createApp } from \"./sdk\"`, which is how the app reaches the Biorouter backend.");
    }
    for line in main.lines() {
        let t = line.trim_start();
        if t.starts_with("import ") && !t.contains("\"./") && !t.contains("'./") {
            error(
                &mut out,
                &format!(
                    "src/main.ts has a non-local import; only import from \"./sdk\": {}",
                    t.trim()
                ),
            );
        }
    }
    let WiringAnalysis {
        source: wiring,
        auto_chat,
    } = analyze_wiring(&main);
    let calls_agent = wiring.contains("br.run")
        || wiring.contains(".prompt(")
        || wiring.contains(".ask(")
        || auto_chat.may_call_agent();
    if is_agentic && !calls_agent {
        out.push(LintFinding {
            level: LintLevel::Warn,
            msg: "src/main.ts never calls the agent (br.run / br.prompt / br.ask) and doesn't enable autoChat. If agent work is intended, wire a control to br.run(prompt, \"#out\"). Intentional local-only apps may omit agent calls.".into(),
        });
    }
    let has_progress_surface = wiring.contains("br.run")
        || (auto_chat.enabled && !auto_chat.unknown)
        || wiring.contains("createApp();")
        || wiring.contains("mountTimeline")
        || il.contains("data-br-progress")
        || il.contains("br-run-status")
        || il.contains("data-br-chat");
    if calls_agent && !has_progress_surface {
        out.push(LintFinding {
            level: LintLevel::Error,
            msg: "Long-running agent work must expose visible step progress. Use br.run(...), the default [data-br-chat] panel, mountTimeline(br, \"#progress\"), or an equivalent br-run-status/debug surface.".into(),
        });
    }

    // (1c) Wiring: if the page has interactive controls but main.ts wires no
    // events (and isn't an auto-chat app), the UI is inert.
    //
    // `br-region` is the clickable map cell. `data-br-region` is an agent render
    // target and is NOT a control — match the class without swallowing the
    // attribute, or every app that declares a render target gets a false warning.
    let has_map_region = il.matches("br-region").count() > il.matches("data-br-region").count();
    let has_controls = il.contains("<button")
        || il.contains("<select")
        || il.contains("type=\"range\"")
        || il.contains("br-chip")
        || has_map_region
        || il.contains("br-dragitem")
        || il.contains("br-dropzone")
        || il.contains("br-tab");
    // A static page may wire its controls in index.html itself — an inline
    // `<script>` or an `onclick=` handler — and both run in the launched app.
    let wired_inline = !inline.handlers.is_empty()
        || inline
            .scripts
            .iter()
            .any(|script| script.contains("addEventListener") || script.contains(".onclick"));
    if has_controls
        && !wiring.contains("addEventListener")
        && !auto_chat.may_call_agent()
        && !wiring.contains("onclick")
        && !wired_inline
    {
        out.push(LintFinding {
            level: LintLevel::Warn,
            msg: "index.html has interactive controls but src/main.ts wires no events (addEventListener). The controls won't do anything, so wire them to br.run(...).".into(),
        });
    }

    // (1b) Element-id consistency: every id main.ts looks up must exist in
    // index.html. A mismatch is a common LLM bug — getElementById returns null
    // and the app crashes on the first addEventListener.
    let referenced = referenced_ids(&main);
    for rid in &referenced {
        let present =
            index.contains(&format!("id=\"{rid}\"")) || index.contains(&format!("id='{rid}'"));
        if !present {
            out.push(LintFinding {
                level: LintLevel::Error,
                msg: format!(
                    "src/main.ts references element id '#{rid}' that is not in index.html, so getElementById would return null and crash. Add the element or fix the id."
                ),
            });
        }
    }

    // (3) Aesthetic alignment with the design system.
    if il.contains("<style") {
        warning(&mut out, "index.html contains a <style> block; prefer the design-system classes/CSS variables over custom CSS for a native look.");
    }
    if il.contains("color:#") || il.contains("color: #") || il.contains("background:#") {
        warning(&mut out, "index.html uses raw hex colors; use var(--br-text)/var(--br-accent)/… tokens so the app matches Biorouter's theme.");
    }
    if !il.contains("br-") {
        warning(&mut out, "index.html uses no Biorouter design-system classes (br-*). The UI will look off-theme; compose with br-card/br-btn/br-select/etc.");
    }
    if !il.contains("br-output") && !il.contains("data-br-chat") {
        warning(&mut out, "No result surface found. Add a <div class=\"br-output\" id=\"out\"></div> (target for br.run) or a [data-br-chat] panel.");
    }
    let authored = format!("{}\n{}\n{}", il, main.to_lowercase(), system_prompt);
    let contains_word = |word: &str| {
        regex::Regex::new(&format!(r"\b{}\b", regex::escape(word)))
            .map(|re| re.is_match(&authored))
            .unwrap_or(false)
    };
    let visual_claim = [
        "visual",
        "visualize",
        "visualizes",
        "visualized",
        "visualizing",
        "visualization",
        "visualizations",
        "chart",
        "charts",
        "diagram",
        "diagrams",
        "figure",
        "figures",
    ]
    .iter()
    .any(|word| contains_word(word))
        || authored.contains("graph visual")
        || authored.contains("visual graph")
        || authored.contains("network visual")
        || authored.contains("knowledge map");
    let asks_for_rendered_visual = [
        "```chart",
        "```graph",
        "```diagram",
        "```network",
        "\\`\\`\\`chart",
        "\\`\\`\\`graph",
        "\\`\\`\\`diagram",
        "\\`\\`\\`network",
        "renderchart",
        "rendergraph",
        // Agent-driven UI satisfies the visualization contract directly: the
        // agent draws into the page rather than emitting a markdown fence.
        "ui_chart",
        "ui_graph",
        "ui_panel",
    ]
    .iter()
    .any(|needle| authored.contains(needle));
    if calls_agent && visual_claim && !asks_for_rendered_visual {
        warning(&mut out, "This app appears to promise visual output, but its prompt/UI never asks for a rendered visual. Either tell the agent to call ui_chart/ui_graph, or to emit a ```chart / ```graph block, so the result surface shows an actual visualization rather than tool logs or prose.");
    }

    // (5) Agent-driven UI coherence.
    if !ui_enabled {
        if main.contains("br.ui.") {
            error(&mut out, "src/main.ts uses `br.ui` but this app sets capabilities.ui.enabled = false, so the agent can never send a UI command, so those handlers are dead. Enable the capability or drop the code.");
        }
        if system_prompt.contains("ui_") {
            warning(&mut out, "The system prompt tells the agent to call ui_* tools, but capabilities.ui.enabled = false so it has none. Enable the capability or rewrite the prompt.");
        }
    } else {
        // Duplicate region names make `ui_render(target=\"@region:x\")` ambiguous —
        // the SDK resolves the first match and silently ignores the rest.
        let names = region_names(&index);
        let mut seen = std::collections::HashSet::new();
        for n in &names {
            if !seen.insert(*n) {
                warning(&mut out, &format!(
                    "index.html declares data-br-region=\"{n}\" more than once. The agent's `@region:{n}` target resolves to the first one only; give each region a unique name."
                ));
            }
        }
    }

    // (6) Apps SDK v2 — custom components (fail closed, design §3.3) + reactive
    // state bindings (design §3.2). The manifest's declared `surface` is the
    // contract the agent's component instances are validated against server-side,
    // so any drift between what `main.ts` registers and what the manifest declares
    // is a build error.
    let declared_components = manifest
        .as_ref()
        .map(|m| m.surface.components.clone())
        .unwrap_or_default();
    let declared_names: std::collections::HashSet<&str> = declared_components
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    let has_state_schema = manifest
        .as_ref()
        .is_some_and(|m| m.surface.state_schema.is_some());

    // Every registration across the app's *authored* TS. `sdk.ts` is the provided
    // runtime (it defines `register`, it doesn't call it), so skip it — otherwise
    // its own API surface would false-positive.
    let authored_ts = read_authored_ts(project_dir);
    let mut undeclared: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut has_dynamic_registration = false;
    for reg in registered_components(&authored_ts) {
        match reg {
            Some(name) if declared_names.contains(name.as_str()) => {}
            Some(name) => {
                undeclared.insert(name);
            }
            None => has_dynamic_registration = true,
        }
    }
    // (6a) Registered but not declared → fail closed: the server can't validate the
    // agent's instances of a component the manifest never declares.
    for name in &undeclared {
        error(&mut out, &format!(
            "src/ registers a custom component \"{name}\" (components.register) that the manifest's surface.components never declares. Declare it with a props schema so the agent's instances are validated server-side, or remove the registration. Custom components fail closed."
        ));
    }
    // (6b) Fail closed on a dynamic registration name: without a string literal the
    // props schema can't be statically extracted and declared.
    if has_dynamic_registration {
        error(&mut out, "A components.register(...) call uses a non-literal component name. Component registrations must use a string literal so the schema can be declared and validated (custom components fail closed), e.g. components.register(\"pathway_map\", { … }).");
    }
    // (6c) Declared but never registered in main.ts → the agent can emit instances
    // the app has no renderer for.
    let main_registered: std::collections::HashSet<String> =
        registered_components(&main).into_iter().flatten().collect();
    for decl in &declared_components {
        if !main_registered.contains(&decl.name) {
            error(&mut out, &format!(
                "manifest declares component \"{name}\" in surface.components but src/main.ts never registers it (components.register(\"{name}\", …)). Register it in main.ts or remove the declaration.",
                name = decl.name
            ));
        }
    }

    // (6d) Prop-fed HTML sink: component props are agent-controlled (prompt-
    // injectable). Feeding them straight into innerHTML/insertAdjacentHTML injects
    // markup into the app's own origin.
    let prop_fed_sink = strip_js_comments(&main).split([';', '\n']).any(|stmt| {
        (stmt.contains("innerHTML") || stmt.contains("insertAdjacentHTML"))
            && stmt.contains("props.")
    });
    if prop_fed_sink {
        warning(&mut out, "src/main.ts feeds component `props` into innerHTML/insertAdjacentHTML. Component props are agent-controlled (prompt-injectable), so render them via textContent or sanitize the HTML instead of injecting markup into the app's own origin.");
    }

    // (6e) Reactive-state bindings without a declared schema. The default structural
    // caps still apply, but a state_schema validates the agent's writes precisely.
    if il.contains("data-br-bind") && !has_state_schema {
        warning(&mut out, "index.html uses data-br-bind* bindings but the manifest declares no surface.state_schema. Declare a state_schema so the agent's writes to the shared state document are validated before they reach these bindings.");
    }

    // (6f) `data-br-bind-attr` refuses event-handler (on*) and `style` targets at
    // runtime because bound state is agent-controlled — the binding layer must be a
    // non-executing sink. Make it a build error so the author fixes it before the
    // binding silently no-ops.
    for attr in bind_attr_targets(&index) {
        let a = attr.to_ascii_lowercase();
        if a.starts_with("on") || a == "style" {
            error(&mut out, &format!(
                "index.html binds data-br-bind-attr to the \"{attr}\" attribute, which the runtime refuses: event-handler (on*) and `style` bindings are blocked because bound state is agent-controlled. Bind a safe attribute instead (text via data-br-bind, or href/src/class/title/aria-*)."
            ));
        }
    }

    // (7) Apps SDK v2 — typed actions (Pillar 1) + app→agent signals (Phase 3).
    // The manifest's `surface.actions` / `surface.signals` are the contract the
    // agent's `app_call` and subscriptions are validated against server-side, so
    // any drift between what `main.ts` wires up and what the manifest declares is
    // a build error (typed actions fail closed, mirroring custom components).
    let declared_actions: Vec<String> = manifest
        .as_ref()
        .map(|m| m.surface.actions.iter().map(|a| a.name.clone()).collect())
        .unwrap_or_default();
    let declared_action_names: std::collections::HashSet<&str> =
        declared_actions.iter().map(|s| s.as_str()).collect();

    // Every `actions.register(...)` in src/main.ts (where the author wires the
    // handlers the agent calls). Literal names carry `Some`, dynamic ones `None`.
    let action_regs = literal_call_args(&main, "actions", "register");
    let registered_action_names: std::collections::HashSet<String> =
        action_regs.iter().flatten().cloned().collect();

    // (7a) Declared but never registered → the agent can `app_call` a verb the app
    // has no handler for.
    for name in &declared_actions {
        if !registered_action_names.contains(name) {
            error(&mut out, &format!(
                "manifest declares action \"{name}\" in surface.actions but src/main.ts never registers it (actions.register(\"{name}\", …)). Register it in main.ts or remove the declaration."
            ));
        }
    }
    // (7b) Registered with a literal name that isn't declared → fail closed: the
    // server can't validate an `app_call` for an action the manifest never declares.
    for reg in action_regs.iter().flatten() {
        if !declared_action_names.contains(reg.as_str()) {
            error(&mut out, &format!(
                "src/main.ts registers an action \"{reg}\" (actions.register) that the manifest's surface.actions never declares. Declare it with a params schema so the agent's app_call is validated server-side, or remove the registration. Typed actions fail closed."
            ));
        }
    }
    // (7c) Fail closed on a dynamic registration name: without a string literal the
    // params schema can't be statically declared and validated.
    if action_regs.iter().any(|r| r.is_none()) {
        error(&mut out, "An actions.register(...) call uses a non-literal action name. Action registrations must use a string literal so the action can be declared and validated (typed actions fail closed), e.g. actions.register(\"run_query\", { … }).");
    }

    // (7d) Signals the app emits, across all authored TS (`sdk.ts` excluded — it
    // provides `emit`, it doesn't call it). Every literal must be declared so the
    // agent can subscribe to it.
    let declared_signals: Vec<String> = manifest
        .as_ref()
        .map(|m| m.surface.signals.iter().map(|s| s.name.clone()).collect())
        .unwrap_or_default();
    let declared_signal_names: std::collections::HashSet<&str> =
        declared_signals.iter().map(|s| s.as_str()).collect();
    let signal_emits = literal_call_args(&authored_ts, "signals", "emit");
    let mut undeclared_signals: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
    let mut has_dynamic_emit = false;
    for emit in &signal_emits {
        match emit {
            Some(name) if declared_signal_names.contains(name.as_str()) => {}
            Some(name) => {
                undeclared_signals.insert(name.clone());
            }
            None => has_dynamic_emit = true,
        }
    }
    for name in &undeclared_signals {
        error(&mut out, &format!(
            "src/ emits a signal \"{name}\" (signals.emit) that the manifest's surface.signals never declares. Declare it in surface.signals so the agent can subscribe to it, or remove the emit."
        ));
    }
    // Dynamic emit → Warning only: emits are validated server-side at runtime, so a
    // computed name is survivable (unlike a registration, which fails closed).
    if has_dynamic_emit {
        warning(&mut out, "A signals.emit(...) call uses a non-literal signal name. Emits are validated server-side at runtime, so a dynamic name is survivable, but it can't be checked against surface.signals here, so prefer a string-literal signal name.");
    }

    // (7e) An app that declares typed actions but still hand-assembles an English
    // prompt into `.run(...)` is bypassing the typed path. Heuristic: a `.run(` line
    // whose argument opens with a template literal, or a line splicing string
    // fragments together (`" +` / `+ "`).
    if !declared_actions.is_empty() {
        let concat_into_run = main.lines().any(|line| match line.split_once(".run(") {
            Some((_, after)) => {
                let arg = after.trim_start();
                arg.starts_with('`') || line.contains("\" +") || line.contains("+ \"")
            }
            None => false,
        });
        if concat_into_run {
            warning(&mut out, "src/main.ts assembles an English prompt into .run(...), but this app declares typed actions; prefer br.call(name, args) over assembling English prompts.");
        }
    }

    // ── A drag-only surface is unreachable by anything but a human mouse ──────
    //
    // The SDK shipped no drag runtime while theme.css advertised drop zones, so
    // the model hand-rolled HTML5 `draggable="true"` + `dragstart`/`drop`. HTML5
    // DnD is not driven by synthetic pointer moves: a coordinate drag from an
    // automated or assistive pointer produces NO `dragstart`. Spec-009's core
    // interaction — building a stratum by dragging covariates — could not be
    // performed by anything except a person with a working mouse.
    //
    // `br.dnd.catalog(...)` gives pointer + click + keyboard parity and emits the
    // declared signal itself. A lint error with no working alternative would just
    // make the model hand-roll something worse, so the primitive is what makes this
    // rule fair.
    let uses_html5_dnd = il.contains("draggable=\"true\"")
        || il.contains("draggable='true'")
        || main.contains("dragstart")
        || main.contains("dataTransfer");
    let uses_dnd_primitive =
        main.contains("br.dnd") || main.contains(".dnd.catalog") || il.contains("data-br-dnd");
    let has_fallback = main.contains("keydown") || main.contains("\"click\"");

    if uses_html5_dnd && !uses_dnd_primitive && !has_fallback {
        error(
            &mut out,
            "this app's drag interaction is HTML5 drag-and-drop with no click or keyboard \
             fallback, so it is unreachable by keyboard, touch, and any automated or assistive \
             pointer (synthetic pointer moves do not fire `dragstart`). Use \
             `br.dnd.catalog({ source, target, signal, onDrop })`, which gives pointer, click \
             and keyboard parity and emits the declared signal for you.",
        );
    } else if uses_html5_dnd && !uses_dnd_primitive {
        warning(
            &mut out,
            "this app hand-rolls HTML5 drag-and-drop. It will not respond to a synthetic or \
             assistive pointer. Prefer `br.dnd.catalog(...)`.",
        );
    }

    // ── A binding with no initial value renders blank on first load ───────────
    //
    // Bindings were evaluated against an empty document until the first (paid)
    // agent turn wrote to it, so every KPI showed blank. That is *why* authors kept
    // a private local state object — which then diverged from the doc the agent
    // reads. Declaring `surface.state_initial` is the fix; not declaring it is the
    // bug's root.
    if let Some(m) = manifest.as_ref() {
        let binds_something = il.contains("data-br-bind");
        if binds_something && m.surface.state_initial.is_none() {
            warning(
                &mut out,
                "index.html has `data-br-bind` bindings but the manifest declares no \
                 `surface.state_initial`, so every bound element renders BLANK until the first \
                 agent turn writes to the shared state. Declare the initial document \
                 (`declare_surface` → `state_initial`) rather than keeping a private local \
                 `state` object, because a local copy silently diverges from the document the agent reads.",
            );
        }
    }

    out
}

/// The `data-br-region="…"` names an app declares — the targets the agent may
/// address as `@region:<name>`.
///
/// Only quoted attribute values are recognised. Written with `split`/`split_once`
/// rather than byte slicing: `&rest[1..]` after taking the first *char* panics on
/// `data-br-region=é`, which is exactly the malformed markup a model emits now
/// and then, and it would take the bundler down with it.
fn region_names(index: &str) -> Vec<&str> {
    let mut names = Vec::new();
    for chunk in index.split("data-br-region=").skip(1) {
        let mut chars = chunk.chars();
        let Some(quote) = chars.next() else {
            continue;
        };
        if quote != '"' && quote != '\'' {
            continue; // unquoted attribute value — nothing well-defined to read
        }
        if let Some((name, _)) = chars.as_str().split_once(quote) {
            names.push(name);
        }
    }
    names
}

/// Concatenate the app's *authored* TypeScript — everything under `src/` except
/// the vendored `sdk.ts`, which is a provided runtime (it defines `register`, it
/// does not call it) rather than authored code. Used to find component
/// registrations without false-positiving on the SDK's own API surface.
fn read_authored_ts(project_dir: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(project_dir.join("src")) {
        for entry in rd.flatten() {
            let path = entry.path();
            let is_ts = path.extension().is_some_and(|e| e == "ts");
            let is_sdk = path.file_name().is_some_and(|n| n == "sdk.ts");
            if is_ts && !is_sdk {
                if let Ok(s) = std::fs::read_to_string(&path) {
                    parts.push(s);
                }
            }
        }
    }
    parts.join("\n")
}

/// Custom-component registrations found in authored TS. Each entry is the
/// string-literal name (`Some`) or `None` for a dynamic/non-literal first
/// argument — which fails closed, because a schema can't be statically declared
/// for a name computed at runtime. Comments are ignored.
fn registered_components(src: &str) -> Vec<Option<String>> {
    let src = strip_js_comments(src);
    let needle = "components.register(";
    let mut out = Vec::new();
    for (idx, _) in src.match_indices(needle) {
        // Whole-word `components`: don't match `subcomponents.register(` etc.
        let boundary_ok = idx == 0 || {
            let b = src.as_bytes()[idx - 1];
            !(b.is_ascii_alphanumeric() || b == b'_')
        };
        if !boundary_ok {
            continue;
        }
        // `idx` is a match start, so `idx + needle.len()` is always a valid char
        // boundary; `.get(..)` keeps clippy's `string_slice` lint happy without an
        // indexing panic risk.
        let rest = src.get(idx + needle.len()..).unwrap_or("").trim_start();
        let mut chars = rest.chars();
        match chars.next() {
            Some(q) if q == '"' || q == '\'' => match chars.as_str().split_once(q) {
                Some((name, _)) => out.push(Some(name.to_string())),
                None => out.push(None), // unterminated literal → treat as dynamic
            },
            _ => out.push(None), // non-string-literal first arg → dynamic
        }
    }
    out
}

/// String-literal first arguments of a whole-word `<object>.<method>(` call in
/// `src`. Each entry is `Some(name)` for a string-literal first argument, or
/// `None` for a dynamic/non-literal one (a name computed at runtime, which can't
/// be statically checked). Comments are ignored, and `object` is matched as a
/// whole word so `subactions.register(` never masquerades as `actions.register(`.
/// The shape mirrors [`registered_components`], generalised for the SDK v2
/// `actions.register` / `signals.emit` call sites.
fn literal_call_args(src: &str, object: &str, method: &str) -> Vec<Option<String>> {
    let src = strip_js_comments(src);
    let needle = format!("{object}.{method}(");
    let mut out = Vec::new();
    for (idx, _) in src.match_indices(needle.as_str()) {
        let boundary_ok = idx == 0 || {
            let b = src.as_bytes()[idx - 1];
            !(b.is_ascii_alphanumeric() || b == b'_')
        };
        if !boundary_ok {
            continue;
        }
        // `idx` is a match start, so `idx + needle.len()` is always a valid char
        // boundary; `.get(..)` keeps clippy's `string_slice` lint happy without an
        // indexing panic risk.
        let rest = src.get(idx + needle.len()..).unwrap_or("").trim_start();
        let mut chars = rest.chars();
        match chars.next() {
            Some(q) if q == '"' || q == '\'' => match chars.as_str().split_once(q) {
                Some((name, _)) => out.push(Some(name.to_string())),
                None => out.push(None), // unterminated literal → treat as dynamic
            },
            _ => out.push(None), // non-string-literal first arg → dynamic
        }
    }
    out
}

/// Attribute names targeted by `data-br-bind-attr` bindings in the markup.
/// Tolerant of the two plausible encodings: the value form
/// `data-br-bind-attr="href:/url title:/tip"` (attr is the part before `:`/`=`)
/// and the name-suffix form `data-br-bind-attr-href="/url"`.
fn bind_attr_targets(index: &str) -> Vec<String> {
    let mut attrs = Vec::new();
    for chunk in index.split("data-br-bind-attr").skip(1) {
        let mut chars = chunk.chars();
        if chars.next() == Some('-') {
            // suffix form: data-br-bind-attr-<attr>=…
            let attr: String = chars
                .as_str()
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !attr.is_empty() {
                attrs.push(attr);
            }
            continue;
        }
        // value form: (optional `=` + whitespace) then a quoted "attr:path …" list.
        let rest = chunk.trim_start_matches(|c: char| c == '=' || c.is_whitespace());
        let mut rc = rest.chars();
        if let Some(q) = rc.next() {
            if q == '"' || q == '\'' {
                if let Some((val, _)) = rc.as_str().split_once(q) {
                    for token in val.split_whitespace() {
                        let attr = token.split([':', '=']).next().unwrap_or("").trim();
                        if !attr.is_empty() {
                            attrs.push(attr.to_string());
                        }
                    }
                }
            }
        }
    }
    attrs
}

/// Element ids that `main.ts` looks up: `getElementById("x")` and CSS-id
/// selectors `"#x"` / `'#x'` (covers querySelector and `br.run(_, "#x")`).
fn referenced_ids(main: &str) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut ids: BTreeSet<String> = BTreeSet::new();
    let main = strip_js_comments(main);
    if let Ok(re) = regex::Regex::new(r#"getElementById\(\s*["']([A-Za-z0-9_-]+)["']"#) {
        for c in re.captures_iter(&main) {
            ids.insert(c[1].to_string());
        }
    }
    if let Ok(re) = regex::Regex::new(r#"["']#([A-Za-z0-9_-]+)["']"#) {
        for c in re.captures_iter(&main) {
            ids.insert(c[1].to_string());
        }
    }
    ids.into_iter().collect()
}

fn strip_js_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut in_block = false;
    for line in source.lines() {
        let mut rest = line;
        loop {
            if in_block {
                if let Some(end) = rest.find("*/") {
                    let (_, after) = rest.split_at(end + 2);
                    rest = after;
                    in_block = false;
                } else {
                    break;
                }
            } else if let Some(start) = rest.find("/*") {
                let (before, marker_and_after) = rest.split_at(start);
                out.push_str(before);
                let (_, after) = marker_and_after.split_at(2);
                rest = after;
                in_block = true;
            } else {
                let code = rest.split_once("//").map(|(code, _)| code).unwrap_or(rest);
                out.push_str(code);
                break;
            }
        }
        out.push('\n');
    }
    out
}

struct WiringAnalysis {
    source: String,
    auto_chat: AutoChatUsage,
}

fn analyze_wiring(source: &str) -> WiringAnalysis {
    let mut parser = tree_sitter::Parser::new();
    let tree = parser
        .set_language(&tree_sitter_javascript::LANGUAGE.into())
        .ok()
        .and_then(|()| parser.parse(source, None));
    let Some(tree) = tree else {
        return WiringAnalysis {
            source: source.to_string(),
            auto_chat: AutoChatUsage {
                enabled: false,
                unknown: source.contains("autoChat"),
            },
        };
    };
    let source = mask_wiring_comments(source, tree.root_node());
    let auto_chat = auto_chat_usage(&source, tree.root_node());
    WiringAnalysis { source, auto_chat }
}

fn mask_wiring_comments(source: &str, root: tree_sitter::Node<'_>) -> String {
    let mut masked = source.as_bytes().to_vec();
    let mut nodes = vec![root];
    while let Some(node) = nodes.pop() {
        if node.kind() == "comment" {
            for byte in &mut masked[node.byte_range()] {
                if !matches!(*byte, b'\n' | b'\r') {
                    *byte = b' ';
                }
            }
        } else {
            // TypeScript may produce error nodes; only actual comment nodes are masked.
            let mut cursor = node.walk();
            nodes.extend(node.children(&mut cursor));
        }
    }
    String::from_utf8(masked).unwrap_or_else(|_| source.to_string())
}

#[derive(Default)]
struct AutoChatUsage {
    enabled: bool,
    unknown: bool,
}

impl AutoChatUsage {
    fn may_call_agent(&self) -> bool {
        self.enabled || self.unknown
    }
}

fn auto_chat_usage(source: &str, root: tree_sitter::Node<'_>) -> AutoChatUsage {
    let mut usage = AutoChatUsage::default();
    for (offset, name) in source.match_indices("autoChat") {
        match literal_auto_chat_value(source, root, offset, name.len()) {
            Some(false) => {}
            Some(true) => usage.enabled = true,
            None => usage.unknown = true,
        }
    }
    usage
}

fn literal_auto_chat_value(
    source: &str,
    root: tree_sitter::Node<'_>,
    offset: usize,
    name_len: usize,
) -> Option<bool> {
    let mut key = root.descendant_for_byte_range(offset, offset + name_len)?;
    if key.kind() == "string_fragment" {
        key = key.parent()?;
    }
    let pair = key.parent()?;
    if pair.kind() != "pair"
        || pair.child_by_field_name("key")? != key
        || !matches!(
            source.get(key.byte_range())?,
            "autoChat" | "\"autoChat\"" | "'autoChat'"
        )
    {
        return None;
    }
    let object = pair.parent()?;
    if object.kind() != "object" || object_has_ambiguous_auto_chat(source, object) {
        return None;
    }
    let value = pair.child_by_field_name("value")?;
    match (value.kind(), source.get(value.byte_range())?) {
        ("false", "false") => Some(false),
        ("true", "true") => Some(true),
        _ => None,
    }
}

fn object_has_ambiguous_auto_chat(source: &str, object: tree_sitter::Node<'_>) -> bool {
    let mut auto_chat_keys = 0;
    let mut cursor = object.walk();
    for member in object.named_children(&mut cursor) {
        if member.kind() == "spread_element" {
            return true;
        }
        let Some(key) = member
            .child_by_field_name("key")
            .or_else(|| member.child_by_field_name("name"))
        else {
            continue;
        };
        if key.kind() == "computed_property_name" {
            return true;
        }
        let Some(key_source) = source.get(key.byte_range()) else {
            return true;
        };
        if matches!(key_source, "autoChat" | "\"autoChat\"" | "'autoChat'") {
            auto_chat_keys += 1;
            if auto_chat_keys > 1 {
                return true;
            }
        }
    }
    false
}

/// Format findings for inclusion in a tool result.
pub fn format_lint(findings: &[LintFinding]) -> String {
    if findings.is_empty() {
        return "Harness: ✓ passes all guardrails (SDK-wired, self-contained, on-theme).".into();
    }
    let mut s = String::from("Harness findings:\n");
    for f in findings {
        let tag = match f.level {
            LintLevel::Error => "✗ ERROR",
            LintLevel::Warn => "• warn",
        };
        s.push_str(&format!("  {tag}: {}\n", f.msg));
    }
    if findings.iter().any(|f| f.level == LintLevel::Error) {
        s.push_str("Fix the ERRORs, then rebuild.");
    }
    s
}

/// Outcome of a build.
#[derive(Debug, Clone)]
pub struct BuildReport {
    pub ok: bool,
    /// "esbuild", "fallback", or "none" when nothing was attempted.
    pub used: String,
    /// Combined stdout/stderr (or fallback notes).
    pub log: String,
    /// The `manifest.sdk_hash` this build earns, `None` when it earned none.
    ///
    /// An esbuild bundle stamps the fingerprint of the App SDK it actually
    /// carries; a fallback bundle stamps `fallback:<fingerprint>`, which a daemon
    /// that can run esbuild treats as stale. ⚠ Stamping the plain fingerprint
    /// after a fallback build is what made broken fallback bundles look fresh
    /// forever.
    pub stamp: Option<String>,
}

impl BuildReport {
    pub(super) fn not_built(used: &str, log: impl Into<String>) -> Self {
        Self {
            ok: false,
            used: used.to_string(),
            log: log.into(),
            stamp: None,
        }
    }

    /// Whether this bundle came from the type-stripper, which a caller should
    /// report as a warning even when it succeeded.
    pub fn is_fallback(&self) -> bool {
        self.used == "fallback"
    }
}

/// The esbuild release Biorouter builds apps with.
///
/// ⚠ Pinned in two places that never import each other: here, for the `npx`
/// last resort, and `ESBUILD_VERSION` in `ui/desktop/scripts/fetch-esbuild.js`,
/// which stages the binary the desktop app ships. An unpinned `npx --yes esbuild`
/// downloaded and ran whatever the registry served that day, on a browser GET.
/// `the_npx_pin_matches_the_shipped_binary` holds the two together.
pub const ESBUILD_VERSION: &str = "0.27.3";

static NPX_ESBUILD_LOCK: Mutex<()> = Mutex::new(());
static NPX_ESBUILD_UNAVAILABLE: AtomicBool = AtomicBool::new(false);
const ESBUILD_TIMEOUT: Duration = Duration::from_secs(60);
/// How long the one login-shell `PATH` probe may take. A shell whose rc files
/// hang must not hang every build behind it.
const LOGIN_SHELL_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long `node --check` may take to verify a fallback bundle.
const NODE_CHECK_TIMEOUT: Duration = Duration::from_secs(20);

fn lock_npx_esbuild(program: &str) -> Option<MutexGuard<'static, ()>> {
    let is_npx = Path::new(program)
        .file_stem()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("npx"));
    is_npx.then(|| {
        NPX_ESBUILD_LOCK
            .lock()
            .expect("npx esbuild lock should not be poisoned")
    })
}

/// The npm cache `npx` installs esbuild into: a directory of this process's
/// own, made once under a random name with mode 0700. `None` when it cannot be
/// made, and then `npx` is not used at all.
///
/// ⚠ It was `<temp>/biorouter-npx-cache-<pid>`. On Linux `<temp>` is `/tmp`,
/// which every account can write, and the pid is in `ps`: another user could
/// make that directory first and plant `_npx/<hash>/node_modules/.bin/esbuild`
/// in it (`<hash>` is a fixed function of the pinned spec), and npx ran the
/// planted binary as this user without asking who owns the cache. A name no
/// one can predict, made by a `mkdir` that fails on an existing entry, can only
/// hold what this process put there. One per process, as before, so two
/// daemons never install into one tree at once.
fn npx_cache_dir() -> Option<&'static Path> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let mut builder = tempfile::Builder::new();
        builder.prefix("biorouter-npx-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o700));
        }
        builder.tempdir().ok().map(tempfile::TempDir::keep)
    })
    .as_deref()
}

/// The `ui/desktop/node_modules/.bin/esbuild` of the checkout `start` sits in,
/// if that checkout has one.
///
/// ⚠ **The ascent stops at the checkout root** — the nearest ancestor holding a
/// `.git` entry — and that bound is why this is a named function rather than a
/// loop inside `find_esbuild`. It used to walk a flat six ancestors, and six is
/// exactly far enough to leave a worktree: from
/// `<repo>/.claude/worktrees/<name>/crates/biorouter-mcp` the sixth step is
/// `<repo>` itself, so a worktree with no install of its own silently borrowed
/// the MAIN checkout's bundler. Every esbuild-dependent test then passed locally
/// for a reason CI can never have, which is the shape of "works on my machine"
/// that is hardest to see: the tool was real, it was just not in the tree under
/// test.
///
/// ⚠ A git worktree's `.git` is a FILE, not a directory, so the bound tests for
/// the ENTRY. Asking `is_dir()` would look right and stop at nothing.
fn esbuild_in_checkout(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        let candidate = d.join("ui/desktop/node_modules/.bin/esbuild");
        if candidate.exists() {
            return Some(candidate);
        }
        // Checked after the candidate, so the root's own install still counts —
        // an ordinary checkout keeps `.git` and `ui/` in the same directory.
        if d.join(".git").exists() {
            return None;
        }
        dir = d.parent();
    }
    None
}

/// The esbuild binary a packaged Biorouter ships in `dir` (the directory
/// holding `biorouterd`), if it is there.
///
/// `ui/desktop/scripts/fetch-esbuild.js` stages it into `src/bin/`, which forge
/// ships as `Contents/Resources/bin` (macOS) / `resources/bin` beside the daemon.
/// Before it did, a Finder-launched daemon — launchd `PATH`, cwd `$HOME` —
/// found no esbuild and no `npx` at all, so every app it built went through the
/// type-stripper.
fn esbuild_beside(dir: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "esbuild.exe"
    } else {
        "esbuild"
    };
    let candidate = dir.join(name);
    candidate.is_file().then_some(candidate)
}

/// A resolved bundler invocation: the program, the arguments that go before the
/// build's own, and the `PATH` its child needs when it was found somewhere the
/// daemon's own `PATH` does not reach.
#[derive(Debug, Clone)]
pub(crate) struct EsbuildCommand {
    program: PathBuf,
    lead: Vec<String>,
    child_path: Option<OsString>,
    /// Where it was found, for the build log.
    source: &'static str,
}

pub(crate) fn packaged_esbuild(executable: &Path) -> Option<PathBuf> {
    let dir = executable.parent()?;
    // CLI-only Linux packages keep helpers under libexec so they do not
    // overwrite a separately installed /usr/bin/esbuild.
    #[cfg(target_os = "linux")]
    {
        if let Some(prefix) = dir.parent() {
            let candidate = prefix.join("libexec/biorouter/esbuild");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    esbuild_beside(dir)
}

/// Locate an esbuild executable, in this order:
///
/// 1. `$BIOROUTER_ESBUILD_BIN` — an explicit choice always wins.
/// 2. The package's helper ([`packaged_esbuild`]): beside the desktop daemon,
///    or under `<install prefix>/libexec/biorouter` in CLI-only Linux packages.
/// 3. This checkout's `ui/desktop/node_modules/.bin/esbuild` (dev tree).
/// 4. `esbuild` on the process `PATH`, then on the login shell's.
/// 5. `npx --yes esbuild@<ESBUILD_VERSION>` — pinned, because it downloads and
///    runs a package from the public registry.
pub(crate) fn find_esbuild() -> Option<EsbuildCommand> {
    let direct = |program: PathBuf, source: &'static str| EsbuildCommand {
        program,
        lead: Vec::new(),
        child_path: None,
        source,
    };
    #[cfg(test)]
    if tests_without_esbuild::active() {
        return tests_without_esbuild::stub().map(|stub| direct(stub, "test stub"));
    }
    if let Ok(bin) = std::env::var("BIOROUTER_ESBUILD_BIN") {
        if !bin.trim().is_empty() && Path::new(&bin).exists() {
            return Some(direct(PathBuf::from(bin), "BIOROUTER_ESBUILD_BIN"));
        }
    }
    if let Some(found) = std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(packaged_esbuild)
    {
        return Some(direct(found, "bundled"));
    }
    // Dev tree: the install belonging to THIS checkout, found from the CWD
    // upwards because a test or the CLI may run from a subdirectory.
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(found) = esbuild_in_checkout(&cwd) {
            return Some(direct(found, "checkout"));
        }
    }
    if let Some(tool) = resolve_tool("esbuild") {
        return Some(EsbuildCommand {
            program: tool.path,
            lead: Vec::new(),
            child_path: tool.child_path,
            source: "PATH",
        });
    }
    if !NPX_ESBUILD_UNAVAILABLE.load(AtomicOrdering::Acquire) {
        if let Some(tool) = resolve_tool("npx") {
            return Some(EsbuildCommand {
                program: tool.path,
                lead: npx_esbuild_args(),
                child_path: tool.child_path,
                source: "npx",
            });
        }
    }
    None
}

/// What goes before the build's own arguments when esbuild runs through `npx`:
/// a pinned release, never "latest".
fn npx_esbuild_args() -> Vec<String> {
    vec!["--yes".into(), format!("esbuild@{ESBUILD_VERSION}")]
}

/// A switch that makes [`find_esbuild`] find nothing on the CURRENT thread, so a
/// test can drive the fallback path deterministically on a machine that has
/// esbuild. Thread-local, because tests run in parallel and the others must
/// keep finding theirs.
#[cfg(test)]
pub(crate) mod tests_without_esbuild {
    use std::cell::{Cell, RefCell};
    use std::path::{Path, PathBuf};

    thread_local! {
        static ACTIVE: Cell<bool> = const { Cell::new(false) };
        static STUB: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }

    pub(crate) fn active() -> bool {
        ACTIVE.with(Cell::get)
    }

    /// The stand-in [`super::find_esbuild`] returns instead, if any.
    pub(crate) fn stub() -> Option<PathBuf> {
        STUB.with(|s| s.borrow().clone())
    }

    /// Run `f` with no esbuild visible to this thread.
    pub(crate) fn run<T>(f: impl FnOnce() -> T) -> T {
        swapped(None, f)
    }

    /// Run `f` with `program` as this thread's only esbuild: a test's stub
    /// that builds deterministically, without the machine's real one.
    pub(crate) fn run_with_stub<T>(program: &Path, f: impl FnOnce() -> T) -> T {
        swapped(Some(program.to_path_buf()), f)
    }

    fn swapped<T>(stub: Option<PathBuf>, f: impl FnOnce() -> T) -> T {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                ACTIVE.with(|a| a.set(false));
                STUB.with(|s| s.borrow_mut().take());
            }
        }
        ACTIVE.with(|a| a.set(true));
        STUB.with(|s| *s.borrow_mut() = stub);
        let _reset = Reset;
        f()
    }

    /// A shell script that writes a fixed bundle to `--outfile=` and exits 0,
    /// standing in for esbuild. Unix only: it is `/bin/sh`.
    #[cfg(unix)]
    pub(crate) fn write_stub(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("esbuild-stub");
        std::fs::write(
            &path,
            "#!/bin/sh\nfor a in \"$@\"; do case \"$a\" in --outfile=*) out=\"${a#--outfile=}\";; esac; done\n\
             printf '(()=>{})();\\n' > \"$out\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
}

/// Whether this process can run a real esbuild. A fallback bundle is stale only
/// when it can: rebuilding one with the type-stripper again buys nothing.
pub fn esbuild_available() -> bool {
    find_esbuild().is_some()
}

/// A program found on a `PATH`, and the `PATH` its child should run with.
pub(crate) struct ResolvedTool {
    pub(crate) path: PathBuf,
    /// `Some` when the program was found only on the login shell's `PATH`.
    /// `npx` is a Node script (`#!/usr/bin/env node`), so finding it there is
    /// no use unless its child can find `node` there too.
    pub(crate) child_path: Option<OsString>,
}

/// Find `program` on the process `PATH`, then on the login shell's.
///
/// A daemon the desktop app starts inherits launchd's `PATH`
/// (`/usr/bin:/bin:/usr/sbin:/sbin` plus the app's own `bin/`), which holds
/// neither Homebrew's `node` nor nvm's `npx`.
pub(crate) fn resolve_tool(program: &str) -> Option<ResolvedTool> {
    let process_dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(path) = find_in_dirs(program, &process_dirs) {
        return Some(ResolvedTool {
            path,
            child_path: None,
        });
    }
    let login_dirs = login_shell_path_dirs();
    let path = find_in_dirs(program, login_dirs)?;
    let mut dirs = process_dirs;
    for dir in login_dirs {
        if !dirs.contains(dir) {
            dirs.push(dir.clone());
        }
    }
    Some(ResolvedTool {
        path,
        child_path: std::env::join_paths(dirs).ok(),
    })
}

fn find_in_dirs(program: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        vec![
            format!("{program}.exe"),
            format!("{program}.cmd"),
            program.to_string(),
        ]
    } else {
        vec![program.to_string()]
    };
    dirs.iter()
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.is_file())
}

/// The directories the user's login shell puts on `PATH`, probed once per
/// process and empty when the probe fails or takes too long.
///
/// The developer extension already answers this for the shell tool
/// ([`crate::developer::paths::get_shell_path_dirs`], which also refuses an
/// unexpected `$SHELL` and strips the daemon's secrets from the probe), so this
/// asks it rather than spawning a second, differently-filtered shell. That
/// function is async and every caller here is blocking code — a
/// `spawn_blocking` build, a plain test — so it runs on a private runtime on
/// its own thread, which works even from inside another runtime, where a
/// `block_on` on the calling thread would panic.
pub(crate) fn login_shell_path_dirs() -> &'static [PathBuf] {
    static DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    DIRS.get_or_init(|| {
        std::thread::spawn(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()?;
            runtime.block_on(async {
                tokio::time::timeout(
                    LOGIN_SHELL_PROBE_TIMEOUT,
                    crate::developer::paths::get_shell_path_dirs(),
                )
                .await
                .ok()?
                .ok()
                .cloned()
            })
        })
        .join()
        .ok()
        .flatten()
        .unwrap_or_default()
    })
}

/// The App SDK an app *should* be bundling — the template compiled into this
/// binary.
pub fn sdk_template() -> &'static str {
    include_str!("templates/sdk.ts")
}

fn fingerprint_of(source: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut h);
    format!("{:x}", h.finish())
}

/// A short fingerprint of the current App SDK. Stored on the manifest after a
/// build so a daemon can tell that an app's bundle predates an SDK upgrade and
/// rebuild it, rather than serving a stale runtime that ignores frames the
/// server now sends (this is how agent-driven UI reaches apps built before it
/// existed).
pub fn sdk_fingerprint() -> String {
    fingerprint_of(sdk_template())
}

/// The fingerprint of the SDK an app actually vendors (`src/sdk.ts`), `None`
/// when it vendors none.
pub fn vendored_sdk_fingerprint(project_dir: &Path) -> Option<String> {
    std::fs::read_to_string(project_dir.join("src/sdk.ts"))
        .ok()
        .map(|source| fingerprint_of(&source))
}

/// This binary's version, the one a bundle banner records.
pub fn biorouter_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// `a > b` for two `major.minor.patch` versions; `false` when either does not
/// parse, so an unreadable version never blocks a rebuild.
pub fn version_is_newer(a: &str, b: &str) -> bool {
    fn parse(v: &str) -> Option<(u64, u64, u64)> {
        let core = v.trim().split(['-', '+']).next()?;
        let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
        Some((parts.next()??, parts.next()??, parts.next()??))
    }
    matches!((parse(a), parse(b)), (Some(a), Some(b)) if a > b)
}

/// The one-line banner that opens every `dist/app.js` this module writes.
///
/// It is the bundle's own record of how it was made, so it travels with the
/// file (an export, a copy) and survives an older daemon re-saving the manifest
/// without fields it does not know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleBanner {
    /// "esbuild" or "fallback".
    pub builder: String,
    /// Fingerprint of the App SDK inside, "none" for an app that bundles none.
    pub sdk: String,
    /// [`sources_fingerprint`] of the `src/` it was built from. `None` on a
    /// bundle from before the field existed.
    pub src: Option<String>,
    /// The Biorouter version that built it.
    pub version: String,
    /// The Biorouter version whose App SDK it carries (`sdkv=`), when that is
    /// not the builder's: a build that KEPT the SDK a newer Biorouter vendored.
    /// `None` on a bundle from before the field existed; see
    /// [`Self::sdk_version`].
    pub sdk_version: Option<String>,
}

const BANNER_PREFIX: &str = "/* biorouter-app-bundle ";
/// What the type-stripper wrote before banners existed. Bundles carrying it are
/// still on users' disks — and were stamped fresh — so they are recognised.
const LEGACY_FALLBACK_MARKER: &str = "/* Agent Drafter fallback bundle";

impl BundleBanner {
    fn line(&self) -> String {
        let note = if self.builder == "fallback" {
            " (Agent Drafter fallback bundle: esbuild was not available)"
        } else {
            ""
        };
        let src = self
            .src
            .as_deref()
            .map(|src| format!(" src={src}"))
            .unwrap_or_default();
        let sdkv = self
            .sdk_version
            .as_deref()
            .map(|v| format!(" sdkv={v}"))
            .unwrap_or_default();
        format!(
            "{BANNER_PREFIX}builder={} sdk={}{src} biorouter={}{sdkv}{note} */\n",
            self.builder, self.sdk, self.version
        )
    }

    /// The Biorouter version the carried App SDK came from: `sdkv=` when the
    /// banner has it, else the builder's version.
    ///
    /// ⚠ Not [`Self::version`]. When an older daemon rebuilds an app and keeps
    /// the SDK a newer one vendored, the builder is the older daemon and the
    /// SDK is still the newer one's. Asking the builder's version made that
    /// older daemon see its own name on the next load, call the SDK drift, and
    /// rebuild down to its older runtime after all — one build later.
    pub fn sdk_version(&self) -> &str {
        self.sdk_version.as_deref().unwrap_or(&self.version)
    }

    fn parse(first_line: &str) -> Option<Self> {
        if first_line.starts_with(LEGACY_FALLBACK_MARKER) {
            return Some(Self {
                builder: "fallback".into(),
                sdk: "unknown".into(),
                src: None,
                version: "0.0.0".into(),
                sdk_version: None,
            });
        }
        let rest = first_line.strip_prefix(BANNER_PREFIX)?;
        let field = |key: &str| {
            rest.split_whitespace()
                .find_map(|word| word.strip_prefix(key))
                .map(str::to_string)
        };
        Some(Self {
            builder: field("builder=")?,
            sdk: field("sdk=")?,
            src: field("src="),
            version: field("biorouter=")?,
            sdk_version: field("sdkv="),
        })
    }

    pub fn is_fallback(&self) -> bool {
        self.builder == "fallback"
    }
}

/// A fingerprint of everything under `project_dir/src`: each file's path and
/// bytes (symlinks skipped, depth bounded). A bundle records the one it was
/// built from in its banner, and is stale once its sources no longer match.
///
/// ⚠ By content, not modification time. A source with a FUTURE mtime (a copy
/// from a machine with a skewed clock, an extracted archive, a `touch`) was
/// newer than every bundle ever built from it, so the app was rebuilt on every
/// page load. SHA-256 rather than the std hasher, whose output may change
/// between Rust releases: this value is written to disk and read back by other
/// builds of Biorouter.
pub fn sources_fingerprint(project_dir: &Path) -> String {
    use sha2::{Digest, Sha256};
    /// Past this a file is fingerprinted by size and mtime, so one huge data
    /// file in `src/` cannot make every page load slow.
    const READ_LIMIT: u64 = 8 << 20;
    fn walk(dir: &Path, base: &Path, depth: usize, h: &mut Sha256) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() && depth < 8 {
                walk(&path, base, depth + 1, h);
            } else if kind.is_file() {
                let rel = path.strip_prefix(base).unwrap_or(&path);
                h.update(rel.to_string_lossy().as_bytes());
                h.update([0]);
                let meta = entry.metadata().ok();
                let len = meta.as_ref().map_or(0, |m| m.len());
                h.update(len.to_le_bytes());
                if len <= READ_LIMIT {
                    if let Ok(bytes) = std::fs::read(&path) {
                        h.update(&bytes);
                    }
                } else if let Some(mtime) = meta.and_then(|m| m.modified().ok()) {
                    let since = mtime
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default();
                    h.update(since.as_nanos().to_le_bytes());
                }
                h.update([0]);
            }
        }
    }
    let mut h = Sha256::new();
    walk(&project_dir.join("src"), project_dir, 0, &mut h);
    h.finalize()[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The banner of the bundle at `path`, `None` when it has none (absent, or
/// written by something else — a custom build script).
pub fn read_bundle_banner(path: &Path) -> Option<BundleBanner> {
    let mut head = [0u8; 256];
    let n = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
    let head = String::from_utf8_lossy(&head[..n]);
    BundleBanner::parse(head.lines().next()?)
}

/// Write `bytes` to `path` through a sibling temp file and a rename, so a
/// reader (a browser fetching `dist/app.js`, a concurrent build) sees the old
/// file or the new one and never a truncated one.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = sibling_temp(path);
    let written = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// A unique name beside `path`, hidden, so nothing serves or bundles it.
fn sibling_temp(path: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, AtomicOrdering::Relaxed)
    ))
}

/// Whether `main` imports the App SDK, however the specifier is spelled:
/// `"./sdk"`, `"./sdk.ts"` or `"./sdk.js"` (esbuild resolves all three to
/// `src/sdk.ts`), in an `import … from`, a bare `import` or an `export … from`,
/// on one line or several.
///
/// ⚠ It matched the literal `"./sdk"` only. An app importing `"./sdk.js"` was
/// then never refreshed, its bundle was stamped with the CURRENT SDK it did not
/// carry, and it kept its creation-time runtime through every upgrade; the
/// fallback bundler left the SDK out of such an app altogether.
fn imports_sdk(main: &str) -> bool {
    static SDK_IMPORT: OnceLock<regex::Regex> = OnceLock::new();
    SDK_IMPORT
        .get_or_init(|| {
            regex::Regex::new(r#"(?:\bfrom|\bimport)\s*["']\./sdk(?:\.ts|\.js)?["']"#)
                .expect("static regex")
        })
        .is_match(main)
}

/// What a build did to `src/sdk.ts`, so a build that installs nothing can put
/// it back.
struct SdkRefresh {
    /// The file's bytes before, `None` when there was no file.
    previous: Option<Vec<u8>>,
}

impl SdkRefresh {
    /// Put `src/sdk.ts` back as it was before the refresh.
    fn undo(self, project_dir: &Path) -> io::Result<()> {
        let path = project_dir.join("src/sdk.ts");
        match self.previous {
            Some(bytes) => write_atomic(&path, &bytes),
            None => match std::fs::remove_file(&path) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            },
        }
    }
}

/// Overwrite the app's vendored `src/sdk.ts` when it differs from the template.
///
/// The SDK is a *provided runtime*, not authored code: `create_app` writes it and
/// nothing is meant to edit it. Refreshing it here means "rebuild the app" is all
/// it takes to pick up SDK fixes, instead of a manual re-copy into every project.
/// Written atomically: a concurrent esbuild must never read half a file.
/// Returns what it replaced, `None` when the file was already current.
fn refresh_sdk(project_dir: &Path) -> io::Result<Option<SdkRefresh>> {
    let path = project_dir.join("src/sdk.ts");
    let previous = std::fs::read(&path).ok();
    if previous.as_deref() == Some(sdk_template().as_bytes()) {
        return Ok(None);
    }
    write_atomic(&path, sdk_template().as_bytes())?;
    Ok(Some(SdkRefresh { previous }))
}

/// How a build should treat the app's vendored SDK.
#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    /// Keep `src/sdk.ts` as it is, because the Biorouter version named here,
    /// NEWER than this one, put it there: rebuilding the app to this binary's
    /// SDK would be a downgrade. `None` (the default) replaces it with this
    /// binary's SDK first. The bundle's banner records whose SDK it carries
    /// ([`BundleBanner::sdk_version`]), so the next build, on any daemon, knows.
    pub keep_sdk_from: Option<String>,
}

/// Build an app project rooted at `project_dir`. Expects `src/main.ts`; writes
/// `dist/app.js`, and `dist/app.css` when the code imports a stylesheet.
/// Returns a report (never errors on a *build* failure — inspect `report.ok`);
/// only returns `Err` for filesystem problems.
pub fn build_app(project_dir: &Path) -> std::io::Result<BuildReport> {
    build_app_with(project_dir, BuildOptions::default())
}

/// [`build_app`] with explicit [`BuildOptions`].
///
/// A build that installs no bundle leaves the app as it found it: a refused
/// fallback is decided before `src/sdk.ts` is touched, and a refresh made for a
/// build that then failed is undone. ⚠ A refused fallback used to rewrite
/// `src/sdk.ts` anyway, so the vendored SDK and the bundle still being served
/// disagreed, on a daemon that could not rebuild either.
pub fn build_app_with(project_dir: &Path, opts: BuildOptions) -> std::io::Result<BuildReport> {
    let entry = project_dir.join("src/main.ts");
    let Ok(main) = std::fs::read_to_string(&entry) else {
        return Ok(BuildReport::not_built(
            "none",
            "src/main.ts not found, so there is nothing to build",
        ));
    };
    let out = project_dir.join("dist/app.js");
    let imports = imports_sdk(&main);
    // An app that neither imports the SDK nor vendors one (a static page with
    // its own main.ts) is not given one it never asked for. One that vendors it
    // is kept current even when the import is spelled in a way `imports_sdk`
    // cannot see: a refresh costs nothing, and a stale SDK stamped as the
    // current one is exactly what the stamp exists to prevent.
    let has_sdk = imports || project_dir.join("src/sdk.ts").is_file();
    let esbuild = find_esbuild();
    if esbuild.is_none() {
        if let Some(refused) = fallback_precheck(&out, &main) {
            return Ok(refused);
        }
    }
    let refresh = match (&opts.keep_sdk_from, has_sdk) {
        (None, true) => refresh_sdk(project_dir)?,
        _ => None,
    };
    let sdk = if has_sdk {
        vendored_sdk_fingerprint(project_dir).unwrap_or_else(|| "none".into())
    } else {
        "none".into()
    };
    let made = BundleMaking {
        project_dir,
        main: &main,
        includes_sdk: imports,
        sdk,
        sdk_version: opts
            .keep_sdk_from
            .filter(|kept| kept.as_str() != biorouter_version()),
    };
    let built = made.build(esbuild);
    let Some(refresh) = refresh else {
        return built;
    };
    match built {
        Ok(mut report) if report.ok => {
            report.log = format!(
                "Refreshed src/sdk.ts to the current App SDK.\n{}",
                report.log
            );
            Ok(report)
        }
        other => {
            if let Err(error) = refresh.undo(project_dir) {
                tracing::warn!(
                    "could not restore src/sdk.ts after a build that installed nothing: {error}"
                );
            }
            other
        }
    }
}

/// Everything one build needs to know beyond the bundler to use.
struct BundleMaking<'a> {
    project_dir: &'a Path,
    main: &'a str,
    /// Whether `main` imports the SDK, so the fallback must include it.
    includes_sdk: bool,
    /// Fingerprint of the vendored SDK, "none" for an app with none.
    sdk: String,
    /// [`BundleBanner::sdk_version`], when it is not this binary's.
    sdk_version: Option<String>,
}

impl BundleMaking<'_> {
    fn build(self, esbuild: Option<EsbuildCommand>) -> io::Result<BuildReport> {
        let dist = self.project_dir.join("dist");
        std::fs::create_dir_all(&dist)?;
        let entry = self.project_dir.join("src/main.ts");
        let mut unavailable = None;
        if let Some(cmd) = esbuild {
            // Taken before esbuild reads them: an edit made during the build
            // then shows as a change, and the next check rebuilds.
            let sources = sources_fingerprint(self.project_dir);
            // esbuild writes into a directory of its own, and what it wrote is
            // installed (banner first) by renames, or not at all.
            let staging = Staging::new(&dist)?;
            match run_esbuild(&cmd, &entry, &staging.0.join("app.js")) {
                Ok(mut report) if report.ok => {
                    let js = std::fs::read_to_string(staging.0.join("app.js"))?;
                    if let Some(finding) =
                        super::page_check::javascript_error(&js, "dist/app.js", false)
                    {
                        return Ok(BuildReport::not_built("esbuild", finding.msg));
                    }
                    if let Some(error) = staged_page_error(self.project_dir, &staging.0, true) {
                        return Ok(BuildReport::not_built("esbuild", error));
                    }
                    let banner = BundleBanner {
                        builder: "esbuild".into(),
                        sdk: self.sdk.clone(),
                        src: Some(sources),
                        version: biorouter_version().into(),
                        sdk_version: self.sdk_version.clone(),
                    };
                    install_bundle(&staging.0, &dist, &banner)?;
                    report.stamp = Some(if self.sdk == "none" {
                        sdk_fingerprint()
                    } else {
                        self.sdk
                    });
                    return Ok(report);
                }
                Ok(report) if cmd.source == "npx" && npm_could_not_fetch(&report.log) => {
                    // npx never reached esbuild (offline, a registry outage):
                    // that is "no esbuild here", not a compile error in the app.
                    NPX_ESBUILD_UNAVAILABLE.store(true, AtomicOrdering::Release);
                    unavailable = Some(format!("npx could not fetch esbuild:\n{}", report.log));
                }
                // esbuild ran: surface its result rather than silently masking a
                // syntax error with the weaker fallback.
                Ok(report) => return Ok(report),
                Err(e) => unavailable = Some(format!("esbuild could not run ({e})")),
            }
        }
        let mut report = fallback_bundle(
            self.project_dir,
            &dist.join("app.js"),
            self.main,
            self.includes_sdk,
            self.sdk_version,
        )?;
        if let Some(why) = unavailable {
            report.log = format!("{why}\n{}", report.log);
        }
        Ok(report)
    }
}

fn staged_page_error(project_dir: &Path, staging: &Path, replaces_css: bool) -> Option<String> {
    let manifest: Option<Manifest> = std::fs::read_to_string(project_dir.join("manifest.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    if manifest.is_none() && !project_dir.join("index.html").is_file() {
        return None;
    }
    let entry = manifest.as_ref().map_or("index.html", |m| m.entry.as_str());
    let findings = super::page_check::staged_findings(project_dir, entry, staging, replaces_css);
    (!findings.is_empty()).then(|| {
        format!(
            "Browser-free page check failed:\n{}",
            format_lint(&findings)
        )
    })
}

/// A fresh hidden directory in `dist/` for esbuild to write into, removed when
/// dropped.
///
/// ⚠ esbuild used to write `--outfile=dist/.app.js.<pid>.<n>.tmp` directly. A
/// stylesheet the code imports is written beside the outfile with its
/// extension swapped, so it landed in `dist/.app.js.<pid>.<n>.css`: nothing
/// installed it as `dist/app.css`, an older `dist/app.css` was served forever,
/// and every rebuild left another hidden stylesheet behind.
struct Staging(PathBuf);

impl Staging {
    fn new(dist: &Path) -> io::Result<Self> {
        let path = sibling_temp(&dist.join("build"));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Install what esbuild wrote into `staging`: `app.css` first when the build
/// made one, else any `dist/app.css` an earlier build left is removed (the
/// code no longer imports a stylesheet); then `app.js`, banner first, with
/// one rename.
fn install_bundle(staging: &Path, dist: &Path, banner: &BundleBanner) -> io::Result<()> {
    let mut bytes = banner.line().into_bytes();
    bytes.extend_from_slice(&std::fs::read(staging.join("app.js"))?);
    let css = staging.join("app.css");
    let installed_css = dist.join("app.css");
    if css.is_file() {
        std::fs::rename(&css, &installed_css)?;
    } else if let Err(error) = std::fs::remove_file(&installed_css) {
        if error.kind() != io::ErrorKind::NotFound {
            return Err(error);
        }
    }
    write_atomic(&dist.join("app.js"), &bytes)
}

/// Whether an `npx` run failed in npm itself, fetching the package, rather than
/// in esbuild. npm names its own failures with an `E…` code (`ENOTFOUND`,
/// `EAI_AGAIN`, `E404`, …); a compile error is esbuild's `✘ [ERROR]` and must
/// stay a build failure.
fn npm_could_not_fetch(log: &str) -> bool {
    [
        "npm ERR! code E",
        "npm error code E",
        "ENOTFOUND",
        "EAI_AGAIN",
        "ECONNREFUSED",
        "ETIMEDOUT",
    ]
    .iter()
    .any(|marker| log.contains(marker))
}

/// Bundle `entry` with esbuild.
///
/// **Environment (issue #62).** The child gets the shared curated environment
/// via [`super::prepare_agent_drafter_child`] —
/// `developer::shell::strip_daemon_private_env_std`, the same rule every other
/// spawn on the agent's behalf uses. esbuild only parses the agent's TypeScript, so this is
/// a weaker boundary than the smoke test's; the reason it gets the rule anyway
/// is the `npx --yes esbuild` fallback in [`find_esbuild`], which downloads a
/// package from the public registry at build time and runs it, lifecycle
/// scripts included. Handing a just-fetched third-party package `biorouterd`'s
/// auth key buys nothing — a bundler needs no credential.
fn run_esbuild(cmd: &EsbuildCommand, entry: &Path, out: &Path) -> std::io::Result<BuildReport> {
    let program = cmd.program.to_string_lossy();
    let mut report = run_esbuild_with_timeout(
        &program,
        &cmd.lead,
        cmd.child_path.as_deref(),
        entry,
        out,
        ESBUILD_TIMEOUT,
    )?;
    if cmd.source == "npx" {
        report.log = format!(
            "Built with npx esbuild@{ESBUILD_VERSION} (no bundled esbuild was found).\n{}",
            report.log
        );
    }
    Ok(report)
}

fn terminate_esbuild(child: &mut Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        // The command is its own process-group leader, so this also reaps an
        // `npx`-spawned compiler or package lifecycle child.
        if unsafe { libc::kill(-pid, libc::SIGKILL) } != 0 {
            let _ = child.kill();
        }
    }
    #[cfg(not(unix))]
    let _ = child.kill();
    let _ = child.wait();
}

fn join_output(reader: std::thread::JoinHandle<io::Result<Vec<u8>>>) -> io::Result<Vec<u8>> {
    reader
        .join()
        .map_err(|_| io::Error::other("esbuild output reader panicked"))?
}

fn run_esbuild_with_timeout(
    program: &str,
    lead: &[String],
    child_path: Option<&OsStr>,
    entry: &Path,
    out: &Path,
    timeout: Duration,
) -> std::io::Result<BuildReport> {
    let npx_guard = lock_npx_esbuild(program);
    let mut cmd = Command::new(program);
    if npx_guard.is_some() {
        // Never npm's default cache or a guessable one: see `npx_cache_dir`.
        let cache = npx_cache_dir()
            .ok_or_else(|| io::Error::other("could not make a private npm cache for npx"))?;
        cmd.env("npm_config_cache", cache);
    }
    if let Some(path) = child_path {
        cmd.env("PATH", path);
    }
    cmd.args(lead);
    cmd.arg(entry);
    cmd.arg("--bundle");
    cmd.arg("--format=iife");
    cmd.arg("--target=es2018");
    cmd.arg("--loader:.ts=ts");
    cmd.arg(format!("--outfile={}", out.display()));
    cmd.arg("--log-level=warning");
    // Last, so nothing set above can leave a daemon credential in the child.
    super::prepare_agent_drafter_child(&mut cmd);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .expect("piped esbuild stdout must be available");
    let mut stderr = child
        .stderr
        .take()
        .expect("piped esbuild stderr must be available");
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes)?;
        Ok(bytes)
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes)?;
        Ok(bytes)
    });

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => {
                terminate_esbuild(&mut child);
                let _ = join_output(stdout_reader);
                let _ = join_output(stderr_reader);
                if npx_guard.is_some() {
                    NPX_ESBUILD_UNAVAILABLE.store(true, AtomicOrdering::Release);
                }
                drop(npx_guard);
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "esbuild exceeded its {} second limit",
                        timeout.as_secs_f64()
                    ),
                ));
            }
            Err(error) => {
                terminate_esbuild(&mut child);
                let _ = join_output(stdout_reader);
                let _ = join_output(stderr_reader);
                drop(npx_guard);
                return Err(error);
            }
        }
    };
    let stdout = join_output(stdout_reader)?;
    let stderr = join_output(stderr_reader)?;
    drop(npx_guard);
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );
    Ok(BuildReport {
        ok: status.success() && out.exists(),
        used: "esbuild".into(),
        log,
        stamp: None,
    })
}

/// What every fallback refusal ends with: the way out.
const FALLBACK_NEEDS_ESBUILD: &str = "This app needs esbuild, which was not found on this \
     machine (the desktop app ships one; a CLI-only install can set BIOROUTER_ESBUILD_BIN or put \
     esbuild on PATH). Until then, keep src/main.ts to single-line imports from \"./sdk\" only.";

/// Why the type-stripper must not build `main`, or `None` when it can.
///
/// The stripper deletes `import` lines and concatenates one file, so anything
/// that changes what an import brings in would be lost silently — it used to
/// drop a second module's code without a word, and leave a multi-line import's
/// tail behind as a syntax error. It also cannot tell a class field's type
/// annotation from an object-literal property on a line of its own
/// (`  at: Date.now(),` would become the shorthand `at,`, which PARSES and then
/// reads an undefined variable), so it refuses that shape too.
fn fallback_refusal(main: &str) -> Option<String> {
    let mut reasons: Vec<String> = Vec::new();
    let dynamic_import = regex::Regex::new(r"\bimport\s*\(").expect("static regex");
    if dynamic_import.is_match(main) {
        reasons.push("src/main.ts uses a dynamic import(), which needs a real bundler".into());
    }
    let ambiguous = regex::Regex::new(
        r"^\s*(?:(?:private|public|protected|readonly|static|declare|override)\s+)*[A-Za-z_$][\w$]*\??\s*:\s*(?:[A-Z]|(?:string|number|boolean|void|any|null|undefined|unknown|never|this|object)\b)",
    )
    .expect("static regex");
    // `function scene(): { x: number } {` — an object type after a parameter
    // list. The stripper removes a NAMED type after `):` but not a `{ … }` one,
    // and the shipped canvas starter has exactly this shape.
    let object_return_type = regex::Regex::new(r"\)\s*:\s*\{").expect("static regex");
    let single_line_sdk_import = regex::Regex::new(
        r#"^import\s+(?:type\s+)?(?:\{[^}]*\}\s+from\s+)?["']\./sdk(?:\.ts|\.js)?["']\s*;?\s*(?://.*)?$"#,
    )
    .expect("static regex");
    let mut skipping_type_block = false;
    let mut depth = 0i32;
    for (n, line) in main.lines().enumerate() {
        let t = line.trim();
        // Interfaces and type aliases are deleted whole by `strip_module_syntax`,
        // so their members are not ambiguous.
        if skipping_type_block {
            depth += count(line, '{') - count(line, '}');
            if depth <= 0 && (t.ends_with(';') || t.ends_with('}')) {
                skipping_type_block = false;
            }
            continue;
        }
        if t.starts_with("interface ")
            || t.starts_with("export interface ")
            || t.starts_with("type ")
            || t.starts_with("export type ")
            || t.starts_with("declare ")
        {
            depth = count(line, '{') - count(line, '}');
            skipping_type_block = depth > 0 || !(t.ends_with(';') || t.ends_with('}'));
            continue;
        }
        if t.starts_with("import ") || t == "import" || t.starts_with("import{") {
            if !single_line_sdk_import.is_match(t) {
                reasons.push(format!(
                    "line {}: `{t}` imports something other than \"./sdk\" in one line (another module, an alias, or a multi-line import)",
                    n + 1
                ));
            } else if t.contains(" as ") {
                reasons.push(format!("line {}: `{t}` renames an SDK import", n + 1));
            }
            continue;
        }
        if t.starts_with("export ") && t.contains(" from ") {
            reasons.push(format!("line {}: `{t}` re-exports another module", n + 1));
            continue;
        }
        if object_return_type.is_match(line) {
            reasons.push(format!(
                "line {}: `{t}` declares an object type after a parameter list, which the fallback cannot strip",
                n + 1
            ));
            continue;
        }
        if ambiguous.is_match(line) {
            reasons.push(format!(
                "line {}: `{t}` could be a typed class field or an object-literal property, which the fallback cannot tell apart",
                n + 1
            ));
        }
    }
    if reasons.is_empty() {
        return None;
    }
    reasons.truncate(8);
    Some(format!(
        "The fallback bundler refused to build this app, rather than build it wrong:\n  - {}\n{FALLBACK_NEEDS_ESBUILD}",
        reasons.join("\n  - ")
    ))
}

/// Run `node --check` on `js` when node can be found. `Ok(None)` = it parsed,
/// `Ok(Some(err))` = it did not, `Err(())` = no node to ask.
fn node_check(js: &str, dir: &Path) -> Result<Option<String>, ()> {
    let node = resolve_tool("node").ok_or(())?;
    // `.cjs`: node refuses to check a file whose extension it does not know
    // (`.tmp`), and reads a plain `.js` as a module when a package.json above
    // says `"type": "module"`. The bundle is a classic script either way.
    let file = sibling_temp(&dir.join("check")).with_extension("cjs");
    std::fs::write(&file, js).map_err(|_| ())?;
    let mut cmd = Command::new(&node.path);
    cmd.arg("--check").arg(&file);
    if let Some(path) = node.child_path.as_deref() {
        cmd.env("PATH", path);
    }
    super::prepare_agent_drafter_child(&mut cmd);
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());
    let outcome = (|| {
        let mut child = cmd.spawn().map_err(|_| ())?;
        let mut stderr = child.stderr.take().ok_or(())?;
        let reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stderr.read_to_end(&mut bytes);
            bytes
        });
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() < NODE_CHECK_TIMEOUT => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(());
                }
            }
        };
        let stderr = reader.join().unwrap_or_default();
        Ok(if status.success() {
            None
        } else {
            Some(String::from_utf8_lossy(&stderr).into_owned())
        })
    })();
    let _ = std::fs::remove_file(&file);
    outcome
}

/// The last resort when no esbuild can run: concatenate `sdk.ts` + `main.ts`,
/// drop module syntax, and strip the common TypeScript type constructs. Not a
/// compiler, and it says so — four rules keep it from doing harm:
///
/// 1. It refuses what it cannot build ([`fallback_refusal`]) with `ok: false`.
/// 2. `main.ts` runs in its OWN function scope nested inside the SDK's, so it
///    can use every SDK name but neither collide with one (`const renderChart`
///    was a SyntaxError) nor replace one (an app's `function escapeHtml`
///    silently swapped out the SDK's hardened escaper).
/// 3. It never overwrites a `dist/app.js` esbuild made: a real bundle of the
///    previous sources beats a guess at the current ones. (It used to, on a
///    plain page view after an SDK upgrade.)
/// 4. When `node` is reachable the result must pass `node --check`, and its
///    stamp is `fallback:<sdk>`, never a plain fingerprint, so a daemon with
///    esbuild rebuilds it properly.
fn fallback_bundle(
    project_dir: &Path,
    out: &Path,
    main: &str,
    includes_sdk: bool,
    sdk_version: Option<String>,
) -> std::io::Result<BuildReport> {
    if let Some(refused) = fallback_precheck(out, main) {
        return Ok(refused);
    }
    let src = project_dir.join("src");
    let (sdk_source, sdk) = if includes_sdk {
        let source = std::fs::read_to_string(src.join("sdk.ts")).unwrap_or_default();
        let fingerprint = fingerprint_of(&source);
        (source, fingerprint)
    } else {
        (String::new(), "none".to_string())
    };
    let banner = BundleBanner {
        builder: "fallback".into(),
        sdk: sdk.clone(),
        src: Some(sources_fingerprint(project_dir)),
        version: biorouter_version().into(),
        sdk_version,
    };
    let js = format!(
        "{}(function(){{\n{}\n;(function(){{\n{}\n}})();\n}})();\n",
        banner.line(),
        strip_module_syntax(&sdk_source),
        strip_module_syntax(main)
    );
    if let Some(finding) =
        super::page_check::javascript_error(&js, "dist/app.js fallback output", false)
    {
        return Ok(BuildReport::not_built("fallback", format!("The fallback bundler's output does not parse, so it was not installed:\n{}\n{FALLBACK_NEEDS_ESBUILD}", finding.msg)));
    }
    let staging = Staging::new(out.parent().unwrap_or(project_dir))?;
    write_atomic(&staging.0.join("app.js"), js.as_bytes())?;
    if let Some(error) = staged_page_error(project_dir, &staging.0, false) {
        return Ok(BuildReport::not_built("fallback", error));
    }
    let verified = match node_check(&js, out.parent().unwrap_or(project_dir)) {
        Ok(None) => "It passed `node --check`.",
        Ok(Some(error)) => {
            return Ok(BuildReport::not_built(
                "fallback",
                format!(
                    "The fallback bundler's output does not parse, so it was not installed:\n{}\n{FALLBACK_NEEDS_ESBUILD}",
                    error.trim()
                ),
            ));
        }
        Err(()) => "It could NOT be verified (no node on this machine), so test the app.",
    };
    write_atomic(out, js.as_bytes())?;
    Ok(BuildReport {
        ok: true,
        used: "fallback".into(),
        log: format!(
            "WARNING: built with Biorouter's limited fallback bundler because esbuild was not \
             found. {verified} The bundle is marked as a fallback, so a Biorouter that has \
             esbuild rebuilds it properly."
        ),
        // A bundle with no SDK in it has no SDK to be out of date: stamped with
        // the current one, as an esbuild bundle is, or an app that vendors an
        // SDK it does not import would be "stale" on every load.
        stamp: Some(format!(
            "fallback:{}",
            if sdk == "none" {
                sdk_fingerprint()
            } else {
                sdk
            }
        )),
    })
}

/// Why the fallback would refuse to build `main` into `out`, decided without
/// touching anything: a shape it cannot build ([`fallback_refusal`]), or a
/// `dist/app.js` esbuild made, which it never replaces.
fn fallback_precheck(out: &Path, main: &str) -> Option<BuildReport> {
    if let Some(refusal) = fallback_refusal(main) {
        return Some(BuildReport::not_built("fallback", refusal));
    }
    if out.exists() && !read_bundle_banner(out).is_some_and(|b| b.is_fallback()) {
        return Some(BuildReport::not_built(
            "fallback",
            format!(
                "dist/app.js was left as it is: it was built by esbuild (from earlier sources), \
                 and the fallback bundler does not replace a real bundle with a guess.\n\
                 {FALLBACK_NEEDS_ESBUILD}"
            ),
        ));
    }
    None
}

/// Remove `import`/`export` statements, `interface`/`type` declarations, and the
/// most common inline type annotations. Conservative and line-oriented.
fn strip_module_syntax(ts: &str) -> String {
    let mut out = Vec::new();
    // `None` = not skipping. `Some(true)` = inside a multi-line `type X = …`
    // alias (ends at a `;` once braces are balanced — covers multi-line union
    // types like `type AgentEvent =\n | {…}\n | {…};`). `Some(false)` = inside an
    // interface/declare block (ends when braces balance).
    let mut skipping: Option<bool> = None;
    let mut depth = 0i32;
    for line in ts.lines() {
        let trimmed = line.trim_start();
        if let Some(is_alias) = skipping {
            depth += count(line, '{') - count(line, '}');
            let done = if is_alias {
                depth <= 0 && trimmed.trim_end().ends_with(';')
            } else {
                depth <= 0
            };
            if done {
                skipping = None;
            }
            continue;
        }
        let is_type_alias = trimmed.starts_with("type ") || trimmed.starts_with("export type ");
        let is_iface = trimmed.starts_with("interface ")
            || trimmed.starts_with("export interface ")
            || trimmed.starts_with("declare ");
        if is_type_alias || is_iface {
            depth = count(line, '{') - count(line, '}');
            let complete = if is_type_alias {
                // Single-line alias: braces balanced AND statement-terminated.
                depth <= 0 && trimmed.trim_end().ends_with(';')
            } else {
                // Single-line interface/declare with balanced braces (rare).
                depth <= 0 && line.contains('}')
            };
            if !complete {
                skipping = Some(is_type_alias);
            }
            continue;
        }
        // Drop import lines entirely (single-file bundle).
        if trimmed.starts_with("import ") {
            continue;
        }
        // Strip a leading `export ` keyword but keep the declaration.
        let mut l = line.to_string();
        if let Some(rest) = l.trim_start().strip_prefix("export ") {
            let (indent, _) = l.split_at(l.len() - l.trim_start().len());
            l = format!("{indent}{rest}");
        }
        // Strip TS-only class-member modifiers (e.g. `private`, `readonly`),
        // which are not valid JS, while preserving indentation. `static` is
        // valid JS and is intentionally kept. Loops to handle combinations like
        // `private readonly foo`.
        loop {
            let ls = l.trim_start();
            let indent_len = l.len() - ls.len();
            let modifier = [
                "private ",
                "public ",
                "protected ",
                "readonly ",
                "abstract ",
                "override ",
            ]
            .iter()
            .find(|kw| ls.starts_with(**kw))
            .map(|kw| kw.len());
            match modifier {
                Some(n) => {
                    let (indent, _) = l.split_at(indent_len);
                    let (_, after_modifier) = ls.split_at(n);
                    l = format!("{indent}{after_modifier}");
                }
                None => break,
            }
        }
        out.push(strip_inline_types(&l));
    }
    out.join("\n")
}

fn count(s: &str, c: char) -> i32 {
    s.chars().filter(|&x| x == c).count() as i32
}

/// Strip the common inline annotations: `: Type` after params/vars and generic
/// `<…>` after identifiers. Heuristic; leaves the runtime semantics intact.
fn strip_inline_types(line: &str) -> String {
    // Remove ` as Type` casts.
    let mut s = regex_lite_replace_as(line);
    // Remove parameter / variable annotations `name: Type` → `name`.
    s = strip_colon_types(&s);
    // Remove value-position generic type args: `new Promise<void>(` → `new Promise(`.
    s = strip_value_generics(&s);
    // Remove TS non-null assertions: `this.ws!.send(` → `this.ws.send(`.
    s = strip_non_null_assertions(&s);
    s
}

/// Strip TS non-null assertion operators (`expr!`) while leaving logical-not
/// (`!expr`) and inequality (`!=`/`!==`) intact. A non-null assertion is a
/// postfix `!`: it follows an operand char and precedes a member/call/terminator.
fn strip_non_null_assertions(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut in_str: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = in_str {
            out.push(c);
            if c == q && (i == 0 || chars[i - 1] != '\\') {
                in_str = None;
            }
            i += 1;
            continue;
        }
        if c == '"' || c == '\'' || c == '`' {
            in_str = Some(c);
            out.push(c);
            i += 1;
            continue;
        }
        if c == '!' {
            let prev = if i > 0 { chars[i - 1] } else { ' ' };
            let next = if i + 1 < chars.len() {
                chars[i + 1]
            } else {
                ' '
            };
            let postfix_operand =
                prev.is_alphanumeric() || prev == '_' || prev == ')' || prev == ']';
            let member_or_end =
                matches!(next, '.' | ')' | ';' | ',' | ']' | '(' | '[') || next.is_whitespace();
            if postfix_operand && member_or_end && next != '=' {
                i += 1; // drop the assertion `!`
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Remove value-position generic type arguments that directly precede a call,
/// e.g. `new Promise<void>(...)` or `foo<T>(...)` → `Promise(...)` / `foo(...)`.
/// Conservative: only strips a `<…>` that (1) immediately follows an identifier
/// char, (2) contains only type-ish characters, and (3) is immediately followed
/// by `(` — so comparisons (`a < b`) and bit-shifts (`a << b`) are untouched.
fn strip_value_generics(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut in_str: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = in_str {
            out.push(c);
            if c == q && (i == 0 || chars[i - 1] != '\\') {
                in_str = None;
            }
            i += 1;
            continue;
        }
        if c == '"' || c == '\'' || c == '`' {
            in_str = Some(c);
            out.push(c);
            i += 1;
            continue;
        }
        if c == '<' && i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
            let mut j = i + 1;
            let mut depth = 1i32;
            let mut type_only = true;
            while j < chars.len() && depth > 0 {
                let d = chars[j];
                if d == '<' {
                    depth += 1;
                } else if d == '>' {
                    depth -= 1;
                } else if !(d.is_alphanumeric()
                    || matches!(d, '_' | ',' | ' ' | '[' | ']' | '.' | '|' | '&'))
                {
                    type_only = false;
                    break;
                }
                j += 1;
            }
            if type_only && depth == 0 && j < chars.len() && chars[j] == '(' {
                // Drop the `<…>` entirely; resume at the `(`.
                i = j;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

fn regex_lite_replace_as(line: &str) -> String {
    // Replace " as Something" casts up to a delimiter. Char-based scan so it is
    // safe on multibyte UTF-8 (em-dash, box-drawing, ellipsis in comments) —
    // byte indexing here previously panicked on a non-char-boundary slice.
    let chars: Vec<char> = line.chars().collect();
    let mut result = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        if i + 4 <= chars.len()
            && chars[i] == ' '
            && chars[i + 1] == 'a'
            && chars[i + 2] == 's'
            && chars[i + 3] == ' '
        {
            i += 4;
            while i < chars.len() {
                let c = chars[i];
                if c == ')' || c == ';' || c == ',' || c == '}' || c == ']' {
                    break;
                }
                i += 1;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

/// What an open bracket on the current line is, for [`strip_colon_types`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum Bracket {
    Paren,
    Square,
    /// A `{` in expression position: an object literal, whose `key: Value`
    /// pairs are NOT type annotations.
    Object,
    /// A `{` opening a block or a class body.
    Block,
}

/// Classify a `{` from what precedes it on the line (`emitted`).
///
/// An object literal follows an operator, an opening bracket, a comma or a
/// value keyword (`return {`); a block follows `)`, `=>`, `else`, a class name,
/// `;` or nothing. The dashboard starter's `emit("x", { at: Date.now() })` was
/// stripped to `{ at )` before this existed.
fn classify_brace(emitted: &str) -> Bracket {
    let before = emitted.trim_end();
    if before.ends_with("=>") {
        return Bracket::Block;
    }
    match before.chars().last() {
        Some(
            '=' | '(' | ',' | ':' | '[' | '?' | '!' | '&' | '|' | '+' | '-' | '*' | '%' | '<' | '>'
            | '~' | '^',
        ) => Bracket::Object,
        Some(c) if c.is_alphanumeric() || c == '_' || c == '$' => {
            let word: String = before
                .chars()
                .rev()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            if matches!(
                word.as_str(),
                "return" | "yield" | "typeof" | "case" | "in" | "of" | "await" | "void" | "throw"
            ) {
                Bracket::Object
            } else {
                Bracket::Block
            }
        }
        _ => Bracket::Block,
    }
}

fn strip_colon_types(line: &str) -> String {
    // Only strip when a `:` is followed by a space + capitalized/primitive type
    // and not inside a string. Very conservative to avoid mangling object/CSS.
    // This handles `(x: Type)`, `let y: Type =`, `): RetType {`.
    let mut out = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut in_str: Option<char> = None;
    // The brackets opened on this line and still open. Only one line is ever
    // visible here, so a `{` opened on an earlier line is unknown — which is
    // why `fallback_refusal` refuses the ambiguous line-start `key: Value`.
    let mut open: Vec<Bracket> = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = in_str {
            out.push(c);
            if c == q && (i == 0 || chars[i - 1] != '\\') {
                in_str = None;
            }
            i += 1;
            continue;
        }
        if c == '"' || c == '\'' || c == '`' {
            in_str = Some(c);
            out.push(c);
            i += 1;
            continue;
        }
        match c {
            '(' => open.push(Bracket::Paren),
            '[' => open.push(Bracket::Square),
            '{' => open.push(classify_brace(&out)),
            ')' | ']' | '}' => {
                open.pop();
            }
            _ => {}
        }
        // `key: Value` inside an object literal is data. A colon right after
        // `)` is still a return type (`{ run(): Promise<void> { … } }`), and
        // `?:` is never valid JS, so only an identifier before it counts.
        let after_identifier = i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
        if c == ':' && after_identifier && open.last() == Some(&Bracket::Object) {
            out.push(c);
            i += 1;
            continue;
        }
        if c == ':' {
            // look ahead: optional space, then a type-ish token
            let mut j = i + 1;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            let starts_type = j < chars.len()
                && (chars[j].is_ascii_uppercase()
                    || matches!(
                        peek_word(&chars, j).as_str(),
                        "string"
                            | "number"
                            | "boolean"
                            | "void"
                            | "any"
                            | "null"
                            | "undefined"
                            | "unknown"
                            | "never"
                            | "this"
                            | "object"
                    ));
            // Only strip in a clearly typed position (preceded by an identifier
            // char, `)`, or `?` for optional params), not an object literal
            // `{ key: value }`.
            let prev = if i > 0 { chars[i - 1] } else { ' ' };
            let typed_position =
                prev.is_alphanumeric() || prev == '_' || prev == ')' || prev == '?';
            if starts_type && typed_position {
                // An optional-param/field marker `name?:` — drop the trailing `?`
                // we already emitted (it is TS-only and invalid in plain JS).
                if prev == '?' {
                    out.pop();
                }
                // Consume the whole type expression — unions `|`, intersections
                // `&`, generics `<…>`, tuples `[…]`, function types `(…) =>` —
                // up to a top-level terminator, tracking nesting so commas /
                // parens inside generics or function types don't end it early.
                i = j;
                let (mut ang, mut par, mut brk) = (0i32, 0i32, 0i32);
                while i < chars.len() {
                    let d = chars[i];
                    match d {
                        '<' => ang += 1,
                        '>' if ang > 0 => ang -= 1,
                        '(' => par += 1,
                        ')' if par > 0 => par -= 1,
                        '[' => brk += 1,
                        ']' if brk > 0 => brk -= 1,
                        _ if ang == 0
                            && par == 0
                            && brk == 0
                            && matches!(d, '=' | ',' | ')' | ';' | '{' | '\n') =>
                        {
                            break;
                        }
                        _ => {}
                    }
                    i += 1;
                }
                out.push(' ');
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

fn peek_word(chars: &[char], start: usize) -> String {
    let mut s = String::new();
    let mut i = start;
    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
        s.push(chars[i]);
        i += 1;
    }
    s
}

/// Default project source files written for a fresh agentic app.
pub fn default_sources() -> Vec<(PathBuf, String)> {
    vec![
        (PathBuf::from("src/sdk.ts"), sdk_template().to_string()),
        (
            PathBuf::from("src/main.ts"),
            include_str!("templates/main.ts").to_string(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tempfile::TempDir;

    #[test]
    fn strip_module_syntax_removes_imports_and_exports() {
        let ts = "import { x } from \"./sdk\";\nexport function f() { return 1; }\ninterface I { a: number }\nconst y = 2;";
        let out = strip_module_syntax(ts);
        assert!(!out.contains("import"));
        assert!(!out.contains("interface"));
        assert!(out.contains("function f()"));
        assert!(out.contains("const y = 2"));
    }

    #[test]
    fn strip_module_syntax_drops_multiline_type_alias_union() {
        // Regression: a multi-line `type X =` union (no `{` on the first line)
        // must be fully removed, not leak its `| { … }` members as statements.
        let ts = "export type E =\n  | { type: \"a\"; x: number }\n  | { type: \"b\"; y: string };\nconst k = 1;";
        let out = strip_module_syntax(ts);
        for l in out.lines() {
            assert!(
                !l.trim_start().starts_with("| "),
                "union member leaked into JS: {l:?}"
            );
        }
        assert!(out.contains("const k = 1"));
    }

    /// The App SDK renders MODEL-authored markdown into `innerHTML` (panels, the
    /// `ui_ask` prompt, chat), and the app is not a sandboxed iframe. So the
    /// markdown escaper must neutralize attribute breakout: escape quotes, and
    /// only ever emit `http(s)` link hrefs stripped of quotes/angles/space. A
    /// regression here is an XSS in the app's own origin, so guard the source.
    #[test]
    fn sdk_markdown_escaper_is_hardened_against_attribute_breakout() {
        let sdk = sdk_template();
        // escapeHtml must cover both quote characters.
        assert!(
            sdk.contains(r#".replace(/"/g, "&quot;")"#),
            "escapeHtml must escape double quotes"
        );
        assert!(
            sdk.contains(r#".replace(/'/g, "&#39;")"#),
            "escapeHtml must escape single quotes"
        );
        // The link rule must stop the URL at whitespace and scrub the href.
        assert!(
            sdk.contains(r#"(https?:[^)\s]+)"#),
            "link URLs must not span whitespace (an injected ` onX=` breaks the attribute)"
        );
        assert!(
            sdk.contains(r#".replace(/["'<>`\s]/g, "")"#),
            "the link href must be stripped of quotes/angles/backtick/space"
        );
    }

    #[test]
    fn regex_lite_replace_as_is_multibyte_safe() {
        // Must not panic on multibyte chars (em-dash, box-drawing, ellipsis)
        // and must preserve them while still stripping ` as T` casts.
        let line = "  // ── header — note … done; const x = y as Foo;";
        let out = regex_lite_replace_as(line);
        assert!(out.contains('─') && out.contains('—') && out.contains('…'));
        assert!(!out.contains(" as Foo"));
    }

    #[test]
    fn fallback_strips_real_sdk_template_into_valid_js() {
        // The actual shipped SDK template must survive the no-esbuild fallback:
        // no panic, no leaked TS (imports / interfaces / union members), and the
        // key runtime symbols survive. If `node` is present, assert it parses.
        let sdk = include_str!("templates/sdk.ts");
        let main = include_str!("templates/main.ts");
        let stripped_sdk = strip_module_syntax(sdk);
        let stripped_main = strip_module_syntax(main);

        for (name, body) in [("sdk.ts", &stripped_sdk), ("main.ts", &stripped_main)] {
            for l in body.lines() {
                let t = l.trim_start();
                assert!(!t.starts_with("import "), "{name}: leaked import: {l:?}");
                assert!(!t.starts_with("| "), "{name}: leaked union member: {l:?}");
                assert!(
                    !t.starts_with("interface ") && !t.starts_with("export "),
                    "{name}: leaked TS decl: {l:?}"
                );
            }
        }
        // Core runtime API must remain.
        assert!(stripped_sdk.contains("function createApp"));
        assert!(stripped_sdk.contains("class BioRouterClient"));
        assert!(stripped_sdk.contains("function renderMarkdown"));
        assert!(stripped_sdk.contains("approve"));

        // Wrap as the fallback bundler does and (optionally) node --check it.
        let js = format!("(function(){{\n{stripped_sdk}\n{stripped_main}\n}})();\n");
        if let Ok(node) = which::which("node") {
            let dir = TempDir::new().unwrap();
            let f = dir.path().join("app.js");
            std::fs::write(&f, &js).unwrap();
            let out = std::process::Command::new(node)
                .arg("--check")
                .arg(&f)
                .output()
                .expect("run node --check");
            assert!(
                out.status.success(),
                "node --check rejected the fallback bundle:\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    #[test]
    fn strip_colon_types_keeps_object_literals() {
        let s = strip_colon_types("const o = { key: value, n: 2 };");
        // object literal colons must be preserved
        assert!(s.contains("key: value") || s.contains("key:value"));
    }

    #[test]
    fn strip_colon_types_removes_param_annotations() {
        let s = strip_colon_types("function f(a: string, b: number) {");
        assert!(!s.contains(": string"));
        assert!(!s.contains(": number"));
        assert!(s.contains("function f(a"));
    }

    #[test]
    fn fallback_bundle_produces_runnable_iife() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("sdk.ts"), "export function hi() { return 1; }").unwrap();
        std::fs::write(src.join("main.ts"), "import { hi } from \"./sdk\";\nhi();").unwrap();
        let out = dir.path().join("dist/app.js");
        std::fs::create_dir_all(dir.path().join("dist")).unwrap();
        let main = std::fs::read_to_string(src.join("main.ts")).unwrap();
        let report = fallback_bundle(dir.path(), &out, &main, true, None).unwrap();
        assert!(report.ok, "{}", report.log);
        let js = std::fs::read_to_string(&out).unwrap();
        assert!(js.contains("function hi()"));
        assert!(js.trim_start().starts_with("/*"));
        assert!(js.contains("(function()"));
    }

    #[test]
    fn build_app_without_entry_is_noop() {
        let dir = TempDir::new().unwrap();
        let report = build_app(dir.path()).unwrap();
        assert!(!report.ok);
    }

    /// Every spelling esbuild resolves to `src/sdk.ts` counts as importing it.
    #[test]
    fn an_sdk_import_is_recognised_however_it_is_spelled() {
        for main in [
            "import { createApp } from \"./sdk\";",
            "import { createApp } from './sdk.js';",
            "import { createApp } from \"./sdk.ts\"",
            "import {\n  createApp,\n  type App,\n} from \"./sdk.js\";",
            "import \"./sdk\";",
            "export * from './sdk';",
            "import{createApp}from\"./sdk\"",
        ] {
            assert!(imports_sdk(main), "{main}");
        }
        for main in [
            "import { x } from \"./sdkx\";",
            "import { x } from \"./sdk/extra\";",
            "import { x } from \"../sdk\";",
            "const path = \"./sdk\";",
            "document.title = 'no imports';",
        ] {
            assert!(!imports_sdk(main), "{main}");
        }
    }

    /// ⚠ A build that installs no bundle used to leave `src/sdk.ts` rewritten
    /// anyway, so the vendored SDK and the bundle still being served disagreed.
    /// A refused fallback now decides before touching it, and a refresh made
    /// for a build that then failed is undone.
    #[test]
    fn a_build_that_installs_nothing_leaves_the_vendored_sdk_alone() {
        let older = "export function createApp() { return 1; }\n";
        let main = "import { createApp } from \"./sdk\";\ncreateApp();\n";
        tests_without_esbuild::run(|| {
            // An esbuild bundle, which the fallback never replaces.
            let dir = project(Some(older), main);
            let bundle =
                "/* biorouter-app-bundle builder=esbuild sdk=0 biorouter=1.0.0 */\n(()=>{})();\n";
            std::fs::write(dir.path().join("dist/app.js"), bundle).unwrap();
            let report = build_app(dir.path()).unwrap();
            assert!(!report.ok, "{}", report.log);
            assert_eq!(
                std::fs::read_to_string(dir.path().join("src/sdk.ts")).unwrap(),
                older
            );
            assert_eq!(
                std::fs::read_to_string(dir.path().join("dist/app.js")).unwrap(),
                bundle
            );

            // A shape the fallback refuses, with no bundle at all.
            let dir = project(
                Some(older),
                &format!("import {{ x }} from \"./x\";\n{main}"),
            );
            assert!(!build_app(dir.path()).unwrap().ok);
            assert_eq!(
                std::fs::read_to_string(dir.path().join("src/sdk.ts")).unwrap(),
                older
            );
        });

        // esbuild ran and failed: the refresh it was given is put back.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let stubs = TempDir::new().unwrap();
            let failing = stubs.path().join("esbuild-fails");
            std::fs::write(&failing, "#!/bin/sh\necho 'x [ERROR] nope' >&2\nexit 1\n").unwrap();
            std::fs::set_permissions(&failing, std::fs::Permissions::from_mode(0o755)).unwrap();
            let dir = project(Some(older), main);
            let report =
                tests_without_esbuild::run_with_stub(&failing, || build_app(dir.path()).unwrap());
            assert!(!report.ok && report.log.contains("nope"), "{}", report.log);
            assert!(!report.log.contains("Refreshed"), "{}", report.log);
            assert_eq!(
                std::fs::read_to_string(dir.path().join("src/sdk.ts")).unwrap(),
                older
            );

            // The same build succeeding keeps the refresh.
            let stub = tests_without_esbuild::write_stub(stubs.path());
            let report =
                tests_without_esbuild::run_with_stub(&stub, || build_app(dir.path()).unwrap());
            assert!(
                report.ok && report.log.contains("Refreshed src/sdk.ts"),
                "{}",
                report.log
            );
            assert_eq!(
                std::fs::read_to_string(dir.path().join("src/sdk.ts")).unwrap(),
                sdk_template()
            );
        }
    }

    /// ⚠ Only the literal `"./sdk"` used to count. An app importing
    /// `"./sdk.js"` was never refreshed and was stamped with the current SDK it
    /// did not carry, so it kept its creation-time runtime through every
    /// upgrade; the fallback left the SDK out of a `"./sdk.ts"` app altogether.
    #[test]
    fn an_sdk_import_with_an_extension_is_refreshed_stamped_and_bundled() {
        let older = "export function createApp() { return 1; }\n";
        #[cfg(unix)]
        {
            let stubs = TempDir::new().unwrap();
            let stub = tests_without_esbuild::write_stub(stubs.path());
            let dir = project(
                Some(older),
                "import { createApp } from \"./sdk.js\";\ncreateApp();\n",
            );
            let report =
                tests_without_esbuild::run_with_stub(&stub, || build_app(dir.path()).unwrap());
            assert!(report.ok, "{}", report.log);
            assert_eq!(
                std::fs::read_to_string(dir.path().join("src/sdk.ts")).unwrap(),
                sdk_template(),
                "refreshed"
            );
            assert_eq!(report.stamp.as_deref(), Some(sdk_fingerprint().as_str()));
            let banner = read_bundle_banner(&dir.path().join("dist/app.js")).unwrap();
            assert_eq!(banner.sdk, sdk_fingerprint());
        }

        let dir = project(
            Some(sdk_template()),
            "import { createApp } from \"./sdk.ts\";\ncreateApp({ autoChat: false });\n",
        );
        let report = tests_without_esbuild::run(|| build_app(dir.path()).unwrap());
        assert!(report.ok, "{}", report.log);
        let js = std::fs::read_to_string(dir.path().join("dist/app.js")).unwrap();
        assert!(
            js.contains("function createApp"),
            "the SDK is in the bundle"
        );
        assert_eq!(
            report.stamp,
            Some(format!("fallback:{}", sdk_fingerprint()))
        );
    }

    /// A fallback bundle of code that carries no SDK has no SDK to be out of
    /// date, so it is stamped with the current one, as an esbuild bundle is.
    /// Otherwise an app that vendors an SDK it does not import is "stale" on
    /// every page load.
    #[test]
    fn a_fallback_bundle_without_the_sdk_is_stamped_current() {
        let dir = project(Some(sdk_template()), "document.title = \"plain\";\n");
        let report = tests_without_esbuild::run(|| build_app(dir.path()).unwrap());
        assert!(report.ok, "{}", report.log);
        assert_eq!(
            report.stamp,
            Some(format!("fallback:{}", sdk_fingerprint()))
        );
        let banner = read_bundle_banner(&dir.path().join("dist/app.js")).unwrap();
        assert_eq!(banner.sdk, "none", "the banner says what is really inside");
    }

    /// What esbuild writes is installed by name: `app.css` beside `app.js` when
    /// the code imports a stylesheet, and a stale `app.css` goes when it no
    /// longer does.
    #[test]
    fn install_bundle_installs_the_stylesheet_and_removes_a_stale_one() {
        let dir = TempDir::new().unwrap();
        let dist = dir.path().join("dist");
        std::fs::create_dir_all(&dist).unwrap();
        let banner = BundleBanner {
            builder: "esbuild".into(),
            sdk: "none".into(),
            src: None,
            version: "1.0.0".into(),
            sdk_version: None,
        };
        let staging = Staging::new(&dist).unwrap();
        std::fs::write(staging.0.join("app.js"), "one();\n").unwrap();
        std::fs::write(staging.0.join("app.css"), "main{color:red}\n").unwrap();
        install_bundle(&staging.0, &dist, &banner).unwrap();
        let staged_at = staging.0.clone();
        drop(staging);
        assert!(!staged_at.exists(), "the staging directory is removed");
        assert_eq!(
            std::fs::read_to_string(dist.join("app.css")).unwrap(),
            "main{color:red}\n"
        );
        let js = std::fs::read_to_string(dist.join("app.js")).unwrap();
        assert!(
            js.starts_with(BANNER_PREFIX) && js.ends_with("one();\n"),
            "{js}"
        );

        let incomplete = Staging::new(&dist).unwrap();
        std::fs::write(incomplete.0.join("app.css"), "bad output").unwrap();
        assert!(install_bundle(&incomplete.0, &dist, &banner).is_err());
        assert_eq!(std::fs::read_to_string(dist.join("app.js")).unwrap(), js);
        assert_eq!(
            std::fs::read_to_string(dist.join("app.css")).unwrap(),
            "main{color:red}\n"
        );
        drop(incomplete);

        let staging = Staging::new(&dist).unwrap();
        std::fs::write(staging.0.join("app.js"), "two();\n").unwrap();
        install_bundle(&staging.0, &dist, &banner).unwrap();
        drop(staging);
        assert!(
            !dist.join("app.css").exists(),
            "a stale stylesheet is removed"
        );
        let names: Vec<String> = std::fs::read_dir(&dist)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["app.js".to_string()]);
    }

    #[test]
    #[cfg(unix)]
    fn a_page_check_failure_keeps_the_previous_javascript_and_stylesheet() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/main.ts"),
            "document.body.dataset.ready = 'new';",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("index.html"),
            r#"<link rel="stylesheet" href="dist/app.css"><p>App</p>"#,
        )
        .unwrap();
        let dist = dir.path().join("dist");
        std::fs::create_dir(&dist).unwrap();
        std::fs::write(dist.join("app.js"), "old();\n").unwrap();
        std::fs::write(dist.join("app.css"), "body{color:red}\n").unwrap();
        let make = || BundleMaking {
            project_dir: dir.path(),
            main: "document.body.dataset.ready = 'new';",
            includes_sdk: false,
            sdk: "none".into(),
            sdk_version: None,
        };
        let compiler = || {
            EsbuildCommand {
            program: "/bin/sh".into(),
            lead: vec!["-c".into(), r#"for arg in "$@"; do case "$arg" in --outfile=*) printf 'updated();\n' > "${arg#--outfile=}";; esac; done"#.into(), "fixture-compiler".into()],
            child_path: None,
            source: "fixture",
        }
        };
        let failed = make().build(Some(compiler())).unwrap();
        assert!(
            !failed.ok && failed.log.contains("dist/app.css"),
            "{}",
            failed.log
        );
        assert_eq!(
            std::fs::read_to_string(dist.join("app.js")).unwrap(),
            "old();\n"
        );
        assert_eq!(
            std::fs::read_to_string(dist.join("app.css")).unwrap(),
            "body{color:red}\n"
        );
        std::fs::write(dir.path().join("index.html"), "<p>No stylesheet needed</p>").unwrap();
        assert!(make().build(Some(compiler())).unwrap().ok);
        assert!(!dist.join("app.css").exists());
    }

    #[test]
    fn a_failed_fallback_page_check_does_not_replace_its_previous_bundle() {
        let dir = project(None, "document.body.dataset.ready = 'old';\n");
        let first = fallback_of(&dir);
        assert!(first.ok, "{}", first.log);
        let old = std::fs::read_to_string(dir.path().join("dist/app.js")).unwrap();
        std::fs::write(dir.path().join("dist/app.css"), "body{color:red}\n").unwrap();
        std::fs::write(
            dir.path().join("index.html"),
            r#"<link rel="stylesheet" href="dist/app.css"><script src="assets/missing.js"></script><p>App</p>"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/main.ts"),
            "document.body.dataset.ready = 'new';\n",
        )
        .unwrap();
        let report = fallback_of(&dir);
        assert!(
            !report.ok && report.log.contains("assets/missing.js"),
            "{}",
            report.log
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("dist/app.js")).unwrap(),
            old
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("dist/app.css")).unwrap(),
            "body{color:red}\n"
        );
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        std::fs::write(dir.path().join("assets/missing.js"), "const value = 1;").unwrap();
        let fixed = fallback_of(&dir);
        assert!(fixed.ok, "{}", fixed.log);
        assert_ne!(
            std::fs::read_to_string(dir.path().join("dist/app.js")).unwrap(),
            old
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("dist/app.css")).unwrap(),
            "body{color:red}\n"
        );
    }

    /// ⚠ esbuild used to write straight to a hidden temp outfile, and it puts
    /// an imported stylesheet beside the outfile with the extension swapped, so
    /// `dist/app.css` was never written and hidden `.css` files piled up. Needs
    /// a local esbuild (never npx: a test does not download).
    #[test]
    fn an_imported_stylesheet_reaches_dist_app_css() {
        if find_esbuild().is_none_or(|cmd| cmd.source == "npx") {
            eprintln!("skipped: no local esbuild");
            return;
        }
        let dir = project(None, "import \"./styles.css\";\nconsole.log(1);\n");
        std::fs::write(dir.path().join("src/styles.css"), "main { color: red; }\n").unwrap();
        let report = build_app(dir.path()).unwrap();
        assert!(report.ok, "{}", report.log);
        let css = std::fs::read_to_string(dir.path().join("dist/app.css")).unwrap();
        assert!(css.contains("color: red"), "{css}");
        let hidden = |dir: &TempDir| -> Vec<String> {
            std::fs::read_dir(dir.path().join("dist"))
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with('.'))
                .collect()
        };
        assert_eq!(hidden(&dir), Vec::<String>::new());

        std::fs::write(dir.path().join("src/main.ts"), "console.log(2);\n").unwrap();
        let report = build_app(dir.path()).unwrap();
        assert!(report.ok, "{}", report.log);
        assert!(!dir.path().join("dist/app.css").exists());
        assert_eq!(hidden(&dir), Vec::<String>::new());
    }

    /// ⚠ The npx cache was `<temp>/biorouter-npx-cache-<pid>`: in a shared
    /// `/tmp` another account could make it first and plant the esbuild npx
    /// would run. It is now a fresh directory of this process's own.
    #[test]
    fn the_npx_cache_is_private_to_this_process() {
        let dir = npx_cache_dir().expect("a private cache directory");
        assert_eq!(npx_cache_dir(), Some(dir), "made once per process");
        assert!(dir.is_dir());
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("biorouter-npx-"), "{name}");
        assert_ne!(
            name,
            format!("biorouter-npx-cache-{}", std::process::id()),
            "not a name another account can predict"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let meta = std::fs::metadata(dir).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o700);
            assert_eq!(meta.uid(), unsafe { libc::geteuid() });
        }
    }

    /// An app project in a temp dir: `src/sdk.ts` (when given) and `src/main.ts`.
    fn project(sdk: Option<&str>, main: &str) -> TempDir {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        if let Some(sdk) = sdk {
            std::fs::write(src.join("sdk.ts"), sdk).unwrap();
        }
        std::fs::write(src.join("main.ts"), main).unwrap();
        std::fs::create_dir_all(dir.path().join("dist")).unwrap();
        dir
    }

    fn fallback_of(dir: &TempDir) -> BuildReport {
        let main = std::fs::read_to_string(dir.path().join("src/main.ts")).unwrap();
        let uses_sdk = imports_sdk(&main);
        fallback_bundle(
            dir.path(),
            &dir.path().join("dist/app.js"),
            &main,
            uses_sdk,
            None,
        )
        .unwrap()
    }

    /// `node --check` on `js`, or `None` when this machine has no node.
    fn node_parses(js: &str) -> Option<Result<(), String>> {
        let node = resolve_tool("node")?;
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("bundle.js");
        std::fs::write(&file, js).unwrap();
        let out = std::process::Command::new(&node.path)
            .arg("--check")
            .arg(&file)
            .output()
            .ok()?;
        Some(if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).into_owned())
        })
    }

    /// The type-stripper concatenates one file and deletes `import` lines, so a
    /// second module's code used to vanish without a word, and a multi-line
    /// import left its tail behind as a syntax error. It now refuses each
    /// shape, says which line, and writes nothing.
    #[test]
    fn fallback_refuses_local_imports() {
        let cases = [
            (
                "import { createApp } from \"./sdk\";\nimport { helper } from \"./util\";\ncreateApp();\n",
                "./util",
            ),
            (
                "import {\n  createApp,\n} from \"./sdk\";\ncreateApp();\n",
                "line 1",
            ),
            (
                "import { createApp } from \"./sdk\";\nconst m = await import(\"./lazy\");\n",
                "dynamic import()",
            ),
            (
                "import { createApp as make } from \"./sdk\";\nmake();\n",
                "renames an SDK import",
            ),
            ("export { x } from \"./other\";\n", "line 1"),
            (
                "function scene(): { x: number } {\n  return { x: 1 };\n}\n",
                "object type after a parameter list",
            ),
        ];
        for (main, expect) in cases {
            let dir = project(Some("export function createApp() {}\n"), main);
            let report = fallback_of(&dir);
            assert!(!report.ok, "refused: {main}");
            assert_eq!(report.used, "fallback");
            assert!(report.stamp.is_none(), "a refusal earns no stamp");
            assert!(report.log.contains(expect), "{main}\n---\n{}", report.log);
            assert!(
                report.log.contains("esbuild"),
                "says the way out: {}",
                report.log
            );
            assert!(!dir.path().join("dist/app.js").exists(), "{main}");
        }
    }

    /// `main.ts` runs in a scope of its own inside the SDK's: it may reuse an
    /// SDK name (`const renderChart` beside the SDK's `function renderChart`
    /// was a SyntaxError) and cannot replace one (an app's `escapeHtml`
    /// silently swapped out the SDK's hardened escaper, by hoisting).
    #[test]
    fn fallback_isolates_main_scope() {
        let sdk = "export function escapeHtml(s: string): string { return \"sdk:\" + s; }\n\
                   export function renderChart() { return 1; }\n\
                   export function report() { globalThis.escaped = escapeHtml(\"x\"); }\n";
        let main = "import { report } from \"./sdk\";\n\
                    const renderChart = 2;\n\
                    function escapeHtml(s: string): string { return \"app:\" + s; }\n\
                    report();\n\
                    globalThis.appChart = renderChart + escapeHtml(\"y\").length;\n";
        let dir = project(Some(sdk), main);
        let report = fallback_of(&dir);
        assert!(report.ok, "{}", report.log);
        let js = std::fs::read_to_string(dir.path().join("dist/app.js")).unwrap();
        // Structure: banner, the SDK's function, then main nested in its own.
        let sdk_at = js.find("function renderChart()").unwrap();
        let nested_at = js.find(";(function(){").unwrap();
        let main_at = js.find("const renderChart = 2").unwrap();
        assert!(sdk_at < nested_at && nested_at < main_at, "{js}");
        if let Some(parsed) = node_parses(&js) {
            parsed.expect("the fallback bundle parses");
            let node = resolve_tool("node").unwrap();
            let file = dir.path().join("dist/app.js");
            let out = std::process::Command::new(&node.path)
                .arg("-e")
                .arg(format!(
                    "require({:?}); console.log(globalThis.escaped, globalThis.appChart)",
                    file.to_string_lossy()
                ))
                .output()
                .unwrap();
            assert_eq!(
                String::from_utf8_lossy(&out.stdout).trim(),
                "sdk:x 7",
                "the SDK keeps its own escapeHtml: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    /// A fallback build never replaces a bundle esbuild (or anything else) made:
    /// a real bundle of the previous sources beats a guess at the current ones.
    /// It does replace an earlier FALLBACK bundle, which is no better than it.
    #[test]
    fn a_fallback_never_overwrites_an_esbuild_bundle() {
        let main = "import { createApp } from \"./sdk\";\ncreateApp();\n";
        let sdk = "export function createApp() {}\n";
        for (existing, replaced) in [
            (
                "/* biorouter-app-bundle builder=esbuild sdk=abc biorouter=1.92.1 */\n(()=>{})();\n",
                false,
            ),
            ("(()=>{ /* a custom build.mjs */ })();\n", false),
            (
                "/* biorouter-app-bundle builder=fallback sdk=abc biorouter=1.92.1 */\n(function(){})();\n",
                true,
            ),
            (
                "/* Agent Drafter fallback bundle (no esbuild present). */\n(function(){})();\n",
                true,
            ),
        ] {
            let dir = project(Some(sdk), main);
            let out = dir.path().join("dist/app.js");
            std::fs::write(&out, existing).unwrap();
            let report = fallback_of(&dir);
            let now = std::fs::read_to_string(&out).unwrap();
            if replaced {
                assert!(report.ok, "{}", report.log);
                assert_ne!(now, existing);
            } else {
                assert!(!report.ok, "{existing}");
                assert!(report.log.contains("left as it is"), "{}", report.log);
                assert_eq!(now, existing, "byte-identical");
            }
        }
    }

    /// A fallback bundle is stamped `fallback:<sdk>`, never the bare
    /// fingerprint that made broken fallback bundles look fresh forever, and
    /// its banner says what built it.
    #[test]
    fn a_fallback_build_is_marked_as_one() {
        let sdk = sdk_template();
        let dir = project(Some(sdk), include_str!("templates/main.ts"));
        let report = tests_without_esbuild::run(|| build_app(dir.path()).unwrap());
        assert!(report.ok, "{}", report.log);
        assert!(report.is_fallback());
        assert_eq!(
            report.stamp,
            Some(format!("fallback:{}", sdk_fingerprint()))
        );
        assert!(report.log.contains("WARNING"), "{}", report.log);
        let banner = read_bundle_banner(&dir.path().join("dist/app.js")).unwrap();
        assert!(banner.is_fallback());
        assert_eq!(banner.sdk, sdk_fingerprint());
        assert_eq!(banner.version, biorouter_version());
    }

    /// Every starter Biorouter ships either builds with the fallback into code
    /// that parses, or is refused with a reason. ⚠ The dashboard starter's
    /// `emit("refresh_requested", { at: Date.now() })` was stripped to
    /// `{ at )`, and two of one user's apps shipped exactly that bundle.
    #[test]
    fn every_shipped_starter_builds_with_the_fallback_or_is_refused() {
        let starters = [
            ("main.ts", include_str!("templates/main.ts")),
            ("canvas", include_str!("templates/starters/canvas/main.ts")),
            (
                "dashboard",
                include_str!("templates/starters/dashboard/main.ts"),
            ),
            (
                "explorer",
                include_str!("templates/starters/explorer/main.ts"),
            ),
            ("wizard", include_str!("templates/starters/wizard/main.ts")),
            (
                "workbench",
                include_str!("templates/starters/workbench/main.ts"),
            ),
        ];
        for (name, main) in starters {
            let dir = project(Some(sdk_template()), main);
            let report = fallback_of(&dir);
            if !report.ok {
                // Refused up front, or caught by `node --check` before it was
                // installed: either way nothing broken was written.
                assert!(
                    report.log.contains("refused") || report.log.contains("does not parse"),
                    "{name}: {}",
                    report.log
                );
                assert!(!dir.path().join("dist/app.js").exists(), "{name}");
                continue;
            }
            let js = std::fs::read_to_string(dir.path().join("dist/app.js")).unwrap();
            if let Some(parsed) = node_parses(&js) {
                parsed
                    .unwrap_or_else(|e| panic!("{name}: the fallback bundle does not parse:\n{e}"));
            }
        }
    }

    #[test]
    fn strip_colon_types_keeps_object_literal_values_mid_expression() {
        let line = "    br.signals.emit(\"refresh_requested\", { at: Date.now() });";
        assert_eq!(strip_colon_types(line), line);
        let ret = "  return { label: String(x), n: Number(y) };";
        assert_eq!(strip_colon_types(ret), ret);
        // A return type after a parameter list is still a type.
        let method = strip_colon_types("  run(a: string): Promise<void> {");
        assert!(
            !method.contains("string") && !method.contains("Promise"),
            "{method}"
        );
        assert!(
            method.starts_with("  run(a") && method.ends_with('{'),
            "{method}"
        );
    }

    /// Issue #386, the input that was reported. Without esbuild the stripper
    /// turned `{ omop_rules: OMOP_RULES, required_outputs: Object.keys(patient) }`
    /// into `{ omop_rules , required_outputs , … }`, and dropped the local
    /// modules `./prevent` and `./preventData` without a word. The literal on
    /// one line now keeps its values and builds; the same literal across lines
    /// and the local modules are refused, by line, with nothing written.
    #[test]
    fn the_reported_literal_keeps_its_values_and_local_modules_are_refused() {
        let literal =
            "const req = { omop_rules: OMOP_RULES, required_outputs: Object.keys(patient) };";
        assert_eq!(strip_colon_types(literal), literal);
        let call = "  br.agent.send({ omop_rules: OMOP_RULES, required_outputs: Object.keys(p) });";
        assert_eq!(strip_colon_types(call), call);

        let sdk = "export function createApp(x: unknown) { globalThis.req = x; }\n";
        let main = format!(
            "import {{ createApp }} from \"./sdk\";\nconst OMOP_RULES = [\"r1\"];\n\
             const patient = {{ age: 1 }};\n{literal}\ncreateApp(req);\n"
        );
        let dir = project(Some(sdk), &main);
        let report = fallback_of(&dir);
        assert!(report.ok, "{}", report.log);
        let js = std::fs::read_to_string(dir.path().join("dist/app.js")).unwrap();
        assert!(js.contains(literal), "{js}");
        if let Some(parsed) = node_parses(&js) {
            parsed.expect("the fallback bundle parses");
        }

        for (main, expect) in [
            (
                "import { createApp } from \"./sdk\";\nimport { risk } from \"./prevent\";\n\
                 import { OMOP_RULES } from \"./preventData\";\ncreateApp(risk(OMOP_RULES));\n",
                "./prevent\"",
            ),
            (
                "import { createApp } from \"./sdk\";\nconst req = {\n  omop_rules: OMOP_RULES,\n  \
                 required_outputs: Object.keys(patient),\n};\ncreateApp(req);\n",
                "line 3",
            ),
        ] {
            let dir = project(Some(sdk), main);
            let report = fallback_of(&dir);
            assert!(!report.ok, "refused: {main}");
            assert!(report.log.contains(expect), "{main}\n---\n{}", report.log);
            assert!(!dir.path().join("dist/app.js").exists(), "{main}");
        }
    }

    /// The banner is the bundle's own record of what built it, read back from
    /// the file; a bundle from before banners is recognised as a fallback only
    /// by the marker the stripper used to write.
    #[test]
    fn a_bundle_banner_round_trips_and_legacy_fallbacks_are_recognised() {
        let mut banner = BundleBanner {
            builder: "esbuild".into(),
            sdk: "1a2b".into(),
            src: Some("0123456789abcdef".into()),
            version: "1.92.1".into(),
            sdk_version: Some("1.93.0".into()),
        };
        assert_eq!(
            BundleBanner::parse(banner.line().trim_end()),
            Some(banner.clone())
        );
        assert_eq!(
            banner.sdk_version(),
            "1.93.0",
            "a kept SDK names its own version"
        );
        // A banner from before `sdkv=` existed: the SDK is the builder's.
        banner.sdk_version = None;
        assert_eq!(
            BundleBanner::parse(banner.line().trim_end()),
            Some(banner.clone())
        );
        assert_eq!(banner.sdk_version(), "1.92.1");
        // A banner from before `src=` existed still reads, with no sources.
        banner.src = None;
        assert_eq!(BundleBanner::parse(banner.line().trim_end()), Some(banner));
        // `sdkv=` is not mistaken for `sdk=`, whichever comes first.
        let parsed = BundleBanner::parse(
            "/* biorouter-app-bundle builder=esbuild sdkv=2.0.0 sdk=ff biorouter=1.0.0 */",
        )
        .unwrap();
        assert_eq!((parsed.sdk.as_str(), parsed.sdk_version()), ("ff", "2.0.0"));
        let legacy =
            BundleBanner::parse("/* Agent Drafter fallback bundle (no esbuild present). */")
                .unwrap();
        assert!(legacy.is_fallback());
        assert_eq!(BundleBanner::parse("(()=>{})();"), None);
        assert_eq!(BundleBanner::parse("/* some other comment */"), None);

        assert!(version_is_newer("1.93.0", "1.92.9"));
        assert!(version_is_newer("2.0.0-beta.1", "1.99.99"));
        assert!(!version_is_newer("1.92.1", "1.92.1"));
        assert!(!version_is_newer("1.9.0", "1.10.0"));
        assert!(
            !version_is_newer("garbage", "1.0.0"),
            "unreadable never blocks"
        );
    }

    /// The packaged app ships esbuild beside `biorouterd`, and that copy is
    /// found by name in the executable's directory.
    #[test]
    fn esbuild_is_found_beside_the_executable() {
        let dir = TempDir::new().unwrap();
        assert_eq!(esbuild_beside(dir.path()), None);
        let name = if cfg!(windows) {
            "esbuild.exe"
        } else {
            "esbuild"
        };
        // The other platform's name is not this platform's binary.
        let other = if cfg!(windows) {
            "esbuild"
        } else {
            "esbuild.exe"
        };
        std::fs::write(dir.path().join(other), b"x").unwrap();
        assert_eq!(esbuild_beside(dir.path()), None);
        std::fs::write(dir.path().join(name), b"x").unwrap();
        assert_eq!(esbuild_beside(dir.path()), Some(dir.path().join(name)));
        // A directory by that name is not a binary.
        let as_dir = TempDir::new().unwrap();
        std::fs::create_dir(as_dir.path().join(name)).unwrap();
        assert_eq!(esbuild_beside(as_dir.path()), None);
    }

    #[test]
    fn packaged_esbuild_is_relative_to_the_selected_executable() {
        let root = TempDir::new().unwrap();
        let selected = root.path().join("selected/bin");
        let other = root.path().join("other/bin");
        std::fs::create_dir_all(&selected).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let name = if cfg!(windows) {
            "esbuild.exe"
        } else {
            "esbuild"
        };
        std::fs::write(other.join(name), b"other package").unwrap();
        assert_eq!(packaged_esbuild(&selected.join("biorouterd")), None);
        std::fs::write(selected.join(name), b"selected package").unwrap();
        assert_eq!(
            packaged_esbuild(&selected.join("biorouterd")),
            Some(selected.join(name))
        );
        assert_eq!(
            packaged_esbuild(&other.join("biorouterd")),
            Some(other.join(name))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cli_packages_find_the_private_esbuild_helper_before_a_path_binary() {
        let dir = TempDir::new().unwrap();
        let bin = dir.path().join("usr/bin");
        let helpers = dir.path().join("usr/libexec/biorouter");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&helpers).unwrap();
        let executable = bin.join("biorouterd");
        assert_eq!(packaged_esbuild(&executable), None);
        let helper = helpers.join("esbuild");
        std::fs::write(&helper, b"packaged").unwrap();
        assert_eq!(packaged_esbuild(&executable), Some(helper.clone()));
        let beside = bin.join("esbuild");
        std::fs::write(&beside, b"desktop").unwrap();
        assert_eq!(packaged_esbuild(&executable), Some(helper.clone()));
        std::fs::remove_file(helper).unwrap();
        assert_eq!(packaged_esbuild(&executable), Some(beside));
    }

    /// The `npx` last resort downloads and runs a registry package, so it names
    /// the exact release the desktop app ships (`fetch-esbuild.js`), never
    /// whatever "latest" is that day.
    #[test]
    fn the_npx_pin_matches_the_shipped_binary() {
        assert_eq!(npx_esbuild_args(), ["--yes", "esbuild@0.27.3"]);
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/desktop/scripts/fetch-esbuild.js");
        let js = std::fs::read_to_string(&script).expect("fetch-esbuild.js");
        let pinned = regex::Regex::new(r"const ESBUILD_VERSION = '([^']+)';")
            .unwrap()
            .captures(&js)
            .expect("fetch-esbuild.js pins ESBUILD_VERSION")[1]
            .to_string();
        assert_eq!(pinned, ESBUILD_VERSION);
    }

    /// npm's own fetch failures mean "no esbuild here"; esbuild's compile
    /// errors stay build failures.
    #[test]
    fn an_npm_fetch_failure_is_told_apart_from_a_compile_error() {
        assert!(npm_could_not_fetch(
            "npm error code ENOTFOUND\nnpm error network"
        ));
        assert!(npm_could_not_fetch("npm ERR! code E404"));
        assert!(!npm_could_not_fetch(
            "✘ [ERROR] Could not resolve \"./util\"\n    src/main.ts:2:23:"
        ));
    }

    #[test]
    fn npx_esbuild_fallback_is_serialized() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let workers = (0..4)
            .map(|_| {
                let active = Arc::clone(&active);
                let peak = Arc::clone(&peak);
                std::thread::spawn(move || {
                    let guard = lock_npx_esbuild("npx.cmd");
                    let running = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(running, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    active.fetch_sub(1, Ordering::SeqCst);
                    drop(guard);
                })
            })
            .collect::<Vec<_>>();

        for worker in workers {
            worker.join().unwrap();
        }

        assert_eq!(peak.load(Ordering::SeqCst), 1);
        assert!(lock_npx_esbuild("esbuild").is_none());
    }

    /// What the kernel can still tell us about a pid that is **not** our child.
    ///
    /// `kill(pid, 0)` cannot answer this on its own: it succeeds for a process
    /// that has already died but has not yet been `wait()`ed by its new parent,
    /// so a zombie is indistinguishable from a live process.
    #[cfg(unix)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum DescendantState {
        /// Reaped: the pid no longer resolves.
        Gone,
        /// Dead, but not yet reaped by whoever inherited it. Only Linux can
        /// name this state in a running poll (see `zombie_aware_state`);
        /// `state_from_proc_read`'s own test constructs it on every unix.
        Zombie,
        /// Still a live process.
        Alive,
        /// The pid resolves, but its state could not be *read*: the procfs
        /// entry was there a moment ago (`kill(pid, 0)` succeeded) and the read
        /// failed for a reason other than the entry being absent -- descriptor
        /// exhaustion, `ENOMEM`, `EACCES`, or no `/proc` mounted at all.
        ///
        /// A failure to observe is not a death. This state differs from `Alive`
        /// only in what it prints: neither satisfies `proves_death`, so both
        /// keep the poll running and then fail on its deadline.
        Unobservable,
    }

    #[cfg(unix)]
    impl DescendantState {
        /// The only two states that are evidence the reaper worked.
        ///
        /// The poll below and the assertion at the end of the reaper test both
        /// ask this one question, so what stops the poll and what satisfies the
        /// assertion cannot drift apart.
        fn proves_death(self) -> bool {
            matches!(self, DescendantState::Gone | DescendantState::Zombie)
        }
    }

    /// Pull the state letter out of one `/proc/<pid>/stat` line.
    ///
    /// Deliberately compiled on every unix rather than behind the Linux `cfg`
    /// that uses it, so the only real parsing here is exercised by the macOS
    /// CI run too -- a `cfg`-gated parser is only ever compiled by the one job
    /// that can also fail on it.
    #[cfg(unix)]
    fn proc_stat_state(stat: &str) -> Option<char> {
        // `comm` is parenthesised and may itself contain spaces and
        // parentheses, so the state letter is the first field after the LAST
        // ')' -- never the third whitespace-separated field.
        stat.rsplit_once(')')?
            .1
            .split_whitespace()
            .next()?
            .chars()
            .next()
    }

    /// Map one attempt to read `/proc/<pid>/stat` onto a descendant state.
    ///
    /// Reached only once `kill(pid, 0)` has just succeeded, so the pid resolved
    /// a moment ago. `ENOENT` is therefore the single error that means what it
    /// looks like: procfs has no entry any more, i.e. the pid was reaped between
    /// that `kill()` and this read. Every other error is a failure to OBSERVE
    /// the process, and for an assertion whose whole job is to prove deadness
    /// that must not read as dead -- see `Unobservable`.
    ///
    /// Deliberately compiled on every unix, for the same reason
    /// `proc_stat_state` is -- the one decision here that can be wrong in a way
    /// no platform's absence should hide.
    #[cfg(unix)]
    fn state_from_proc_read(read: io::Result<String>) -> DescendantState {
        match read {
            // Z is "zombie", X is "dead": terminated, awaiting a reaper.
            Ok(stat) => match proc_stat_state(&stat) {
                Some('Z' | 'X') => DescendantState::Zombie,
                _ => DescendantState::Alive,
            },
            // Reaped between the kill() and this read.
            Err(error) if error.raw_os_error() == Some(libc::ENOENT) => DescendantState::Gone,
            // EMFILE/ENFILE, ENOMEM, EACCES, ... : we failed to look.
            Err(_) => DescendantState::Unobservable,
        }
    }

    /// Linux exposes the state letter directly, so a zombie is nameable even
    /// where nothing ever reaps it -- a container whose pid 1 is not a
    /// subreaper never turns the orphan below into `Gone`.
    #[cfg(all(unix, target_os = "linux"))]
    fn zombie_aware_state(pid: i32) -> DescendantState {
        let read = std::fs::read_to_string(format!("/proc/{pid}/stat"));
        // An absent entry means "reaped" only where procfs is mounted to have
        // an entry in. Without this, a sandbox with no `/proc` answers ENOENT
        // for every pid -- indistinguishable, to the arm above, from a reaper
        // that worked -- and reports every live descendant as dead. Checked
        // only on the error path, and against our OWN entry, which exists on
        // any host where the read above can be believed.
        if read.is_err() && std::fs::metadata("/proc/self/stat").is_err() {
            return DescendantState::Unobservable;
        }
        state_from_proc_read(read)
    }

    /// Everywhere else, a pid that still resolves is reported as alive.
    ///
    /// macOS reveals a zombie only through `sysctl(KERN_PROC_PID)`'s
    /// `kinfo_proc`, which the `libc` crate does not expose on Apple targets --
    /// reading `p_stat` would mean hard-coding a struct offset into a test.
    /// It buys nothing here, because launchd reaps an orphan in milliseconds,
    /// so the poll below resolves this to `Gone` long before its deadline.
    #[cfg(all(unix, not(target_os = "linux")))]
    fn zombie_aware_state(_pid: i32) -> DescendantState {
        DescendantState::Alive
    }

    #[cfg(unix)]
    fn descendant_state(pid: i32) -> DescendantState {
        if unsafe { libc::kill(pid, 0) } != 0 {
            return if io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                DescendantState::Gone
            } else {
                // EPERM and friends: the pid resolves to something we may not
                // signal, which still counts as present.
                DescendantState::Alive
            };
        }
        zombie_aware_state(pid)
    }

    /// Poll until `pid` stops being a live process, or `deadline` expires.
    #[cfg(unix)]
    fn await_descendant_death(pid: i32, deadline: Duration) -> DescendantState {
        let started = Instant::now();
        loop {
            let state = descendant_state(pid);
            if state.proves_death() || started.elapsed() >= deadline {
                return state;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[cfg(unix)]
    #[test]
    fn proc_stat_state_reads_the_letter_after_a_comm_holding_spaces_and_parens() {
        // The ordinary shape.
        assert_eq!(
            proc_stat_state("42 (sleep) Z 1 42 42 0 -1 4194560"),
            Some('Z')
        );
        // A comm may contain both spaces and a ')', which is why the split is
        // on the LAST ')' and not on whitespace.
        assert_eq!(proc_stat_state("42 (od) ah) R 1 42 42"), Some('R'));
        assert_eq!(proc_stat_state("42 (x) X 1"), Some('X'));
        // Nothing parseable must not be reported as a state.
        assert_eq!(proc_stat_state("garbage"), None);
        assert_eq!(proc_stat_state("42 (sleep)"), None);
    }

    /// A `/proc` read that failed for any reason but `ENOENT` is a failure to
    /// OBSERVE the descendant, and must never be reported as its death.
    ///
    /// The direction is the whole point: this poll exists to PROVE a descendant
    /// died, so "I could not look" has to keep polling and then fail, never
    /// satisfy the assertion. Mapping every `Err` onto `Gone` -- which is what
    /// this did before the arm was split -- passed the reaper test with a live
    /// `sleep 30` grandchild on any host that had run out of file descriptors,
    /// was short of memory, or had no `/proc` mounted at all.
    #[cfg(unix)]
    #[test]
    fn a_proc_read_that_failed_for_anything_but_enoent_is_not_evidence_of_death() {
        // ENOENT is the one error that means what the old arm assumed of all of
        // them: procfs no longer has an entry, so the pid really was reaped.
        let gone = state_from_proc_read(Err(io::Error::from_raw_os_error(libc::ENOENT)));
        assert_eq!(gone, DescendantState::Gone);
        assert!(gone.proves_death());

        // Every other errno: the pid resolved a moment ago and we failed to
        // read it. Descriptor exhaustion, ENOMEM, EACCES, EIO.
        for errno in [
            libc::EMFILE,
            libc::ENFILE,
            libc::ENOMEM,
            libc::EACCES,
            libc::EIO,
        ] {
            let state = state_from_proc_read(Err(io::Error::from_raw_os_error(errno)));
            assert_eq!(
                state,
                DescendantState::Unobservable,
                "errno {errno} is a failed observation, not a reported state"
            );
            assert!(
                !state.proves_death(),
                "errno {errno} must not satisfy an assertion that the descendant died"
            );
        }

        // An error carrying no errno at all is just as unobservable.
        let synthetic = state_from_proc_read(Err(io::Error::other("procfs unavailable")));
        assert_eq!(synthetic, DescendantState::Unobservable);
        assert!(!synthetic.proves_death());

        // A readable entry still decides on its state letter, and an entry that
        // does not parse is not a death either.
        assert_eq!(
            state_from_proc_read(Ok("42 (sleep) Z 1 42".to_owned())),
            DescendantState::Zombie
        );
        assert_eq!(
            state_from_proc_read(Ok("42 (sleep) S 1 42".to_owned())),
            DescendantState::Alive
        );
        assert!(!state_from_proc_read(Ok("garbage".to_owned())).proves_death());
    }

    #[cfg(unix)]
    #[test]
    fn a_timed_out_esbuild_reaps_its_whole_process_group() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let shim = dir.path().join("esbuild-shim");
        let descendant_pid = dir.path().join("descendant.pid");
        std::fs::write(
            &shim,
            "#!/bin/sh\npidfile=\"$1\"\nsleep 30 &\necho \"$!\" > \"$pidfile\"\nwait\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&shim).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&shim, permissions).unwrap();

        let entry = dir.path().join("main.ts");
        let out = dir.path().join("app.js");
        std::fs::write(&entry, "const ok = true;").unwrap();

        /// The compiler's own limit, so the reaper runs about a second in.
        const TIMEOUT: Duration = Duration::from_secs(1);
        /// How long the whole attempt gets -- the pid being recorded, and the
        /// call returning after its limit expired. This BOUNDS the call rather
        /// than waiting on it, which is what lets the descendant be inspected
        /// while it is still held; see the comment on the spawn below.
        const BUDGET: Duration = Duration::from_secs(3);
        /// How long after that the descendant gets to stop being a live
        /// process.
        const DEADLINE: Duration = Duration::from_secs(2);
        // Under load the shim can be killed before it reaches its
        // `echo "$!" > "$pidfile"`, leaving that file absent or truncated. Such
        // an attempt established no descendant at all, so it proves nothing in
        // either direction -- it is a harness miss, not a reaper failure. Retry
        // it instead of reading a pid that was never written.
        const ATTEMPTS: usize = 5;
        let mut attempt = None;
        for _ in 0..ATTEMPTS {
            let _ = std::fs::remove_file(&descendant_pid);
            // The call runs on its own thread and the descendant is watched
            // from this one, because a descendant that SURVIVES holds the
            // call open: it inherited the shim's piped stdout/stderr, so
            // `join_output`'s `read_to_end` cannot return until it exits --
            // 30 s for `sleep 30`. Waiting for the call to return before
            // looking at the pid would therefore look at a `sleep` that had
            // long since exited on its own and report the reaper as healthy,
            // and bounding only the call's WALL TIME (as this test did) makes
            // that bound, not the descendant, the assertion a regression
            // trips -- 30 s of waiting for `elapsed() < 3s`, which names
            // neither the descendant nor the reaper.
            let (finished, call_returned) = std::sync::mpsc::channel();
            let call = {
                let shim = shim.clone();
                let args = vec![descendant_pid.to_string_lossy().into_owned()];
                let entry = entry.clone();
                let out = out.clone();
                std::thread::spawn(move || {
                    let result = run_esbuild_with_timeout(
                        shim.to_str().unwrap(),
                        &args,
                        None,
                        &entry,
                        &out,
                        TIMEOUT,
                    );
                    // Only the error is inspected; `used` keeps the payload
                    // Debug-printable for an unexpected success.
                    let _ = finished.send(result.map(|report| report.used));
                })
            };

            let spawned = Instant::now();
            // The shim records the pid milliseconds in, so this normally falls
            // through at once.
            let recorded = loop {
                let recorded = std::fs::read_to_string(&descendant_pid)
                    .ok()
                    .and_then(|recorded| recorded.trim().parse::<i32>().ok());
                if recorded.is_some() || spawned.elapsed() >= BUDGET {
                    break recorded;
                }
                std::thread::sleep(Duration::from_millis(5));
            };
            let returned = call_returned.recv_timeout(BUDGET.saturating_sub(spawned.elapsed()));
            match recorded {
                Some(pid) => {
                    attempt = Some((pid, returned, call));
                    break;
                }
                // Nothing to observe. Leave a call that is still blocked to
                // finish on its own rather than joining it for its descendant's
                // full lifetime.
                None => {
                    if returned.is_ok() {
                        let _ = call.join();
                    }
                }
            }
        }
        let (pid, returned, call) = attempt.unwrap_or_else(|| {
            panic!(
                "the shim never recorded a descendant pid in {ATTEMPTS} attempts, so this run \
                 never observed a process group at all -- a harness failure, not a reaper one"
            )
        });

        // The descendant is a GRANDCHILD: the shim backgrounds `sleep` and
        // waits on it, so `terminate_esbuild`'s `child.wait()` reaps the shim
        // and nothing else. The grandchild is re-parented to init/launchd and
        // stays a zombie until that reaps it -- and `kill(pid, 0)` succeeds for
        // a zombie, so checking once, here, races the reaper. Poll for the pid
        // to actually stop being a live process instead.
        //
        // Asserted FIRST, before anything about the call: this is the property
        // the test is named for, so it is the diagnostic a reaper regression
        // must print.
        let state = await_descendant_death(pid, DEADLINE);
        let call_state = if returned.is_ok() {
            "had returned"
        } else {
            "had not returned"
        };
        assert!(
            state.proves_death(),
            "the compiler's descendant (pid {pid}) was still {state:?} {DEADLINE:?} after \
             run_esbuild_with_timeout's {TIMEOUT:?} limit expired (the call itself {call_state} \
             within {BUDGET:?}); the SIGKILL to the process group must leave it dead (Zombie, \
             awaiting its new parent's wait) or reaped (Gone)"
        );
        // Only then the call itself. Its return is what the old wall-clock
        // bound measured, and a surviving descendant delays it, so it can only
        // be judged once the descendant has been.
        let result = returned.unwrap_or_else(|_| {
            panic!(
                "run_esbuild_with_timeout produced no result within {BUDGET:?} of being called \
                 with a {TIMEOUT:?} limit, though its descendant (pid {pid}) is {state:?}: the \
                 timeout path itself did not return"
            )
        });
        let error = result.expect_err("the hung compiler must time out");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        let _ = call.join();
    }

    #[test]
    fn build_app_bundles_with_available_toolchain() {
        // Uses esbuild if present, else the fallback — either way dist/app.js
        // must exist and contain the SDK code.
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("sdk.ts"), include_str!("templates/sdk.ts")).unwrap();
        std::fs::write(src.join("main.ts"), include_str!("templates/main.ts")).unwrap();
        let report = build_app(dir.path()).unwrap();
        assert!(report.ok, "build failed: {}", report.log);
        let js = std::fs::read_to_string(dir.path().join("dist/app.js")).unwrap();
        assert!(js.contains("Biorouter") || js.contains("createApp"));
    }

    #[test]
    fn lint_app_flags_portability_wiring_and_runtime_breakers() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            dir.path().join("index.html"),
            r#"<html><head><script src="https://cdn.example/app.js"></script></head>
               <body><main class="br-container"><button id="run" class="br-btn">Run</button></main></body></html>"#,
        )
        .unwrap();
        std::fs::write(
            src.join("main.ts"),
            r##"import confetti from "canvas-confetti";
import { createApp } from "./sdk";
const br = createApp({ autoChat: false });
br.run("go", "#missing");
"##,
        )
        .unwrap();

        let findings = lint_app(dir.path());
        let formatted = format_lint(&findings);
        assert!(
            findings.iter().any(|f| f.level == LintLevel::Error),
            "{formatted}"
        );
        assert!(formatted.contains("external <script"), "{formatted}");
        assert!(formatted.contains("non-local import"), "{formatted}");
        assert!(formatted.contains("#missing"), "{formatted}");
    }

    #[test]
    fn lint_static_app_does_not_require_agent_sdk() {
        let formatted = lint_with(
            r#"<html><body><main class="br-container">Static</main></body></html>"#,
            "",
            Some(
                r#"{"id":"static-app","title":"Static","description":"","kind":"static","entry":"index.html","created_at":0,"updated_at":0}"#,
            ),
        );

        assert!(!formatted.contains("src/main.ts must"), "{formatted}");
        assert!(!formatted.contains("ERROR"), "{formatted}");
    }

    /// A static page's own inline script and `onclick=` run in the launched app
    /// (hash-pinned), so they count as wiring; a `javascript:` URL does not run
    /// there and is warned about, never refused.
    #[test]
    fn lint_counts_inline_wiring_and_warns_on_javascript_urls() {
        let manifest = r#"{"id":"s","title":"S","description":"","kind":"static","entry":"index.html","created_at":0,"updated_at":0}"#;
        let wired = lint_with(
            r#"<main class="br-container"><button class="br-btn" onclick="go()">Go</button>
               <div id="out" class="br-output"></div>
               <script>function go() { document.getElementById("out").textContent = "1"; }</script></main>"#,
            "",
            Some(manifest),
        );
        assert!(!wired.contains("won't do anything"), "{wired}");
        assert!(!wired.contains("javascript:"), "{wired}");

        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("index.html"),
            r#"<main class="br-container"><a class="br-btn" href="javascript:go()">Go</a></main>"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("manifest.json"), manifest).unwrap();
        let findings = lint_app(dir.path());
        let url = findings
            .iter()
            .find(|f| f.msg.contains("javascript:"))
            .expect("a javascript: URL is reported");
        assert_eq!(url.level, LintLevel::Warn);
    }

    /// H6: a hardcoded text color won't adapt to the theme and can go invisible.
    #[test]
    fn lint_warns_on_hardcoded_text_color_but_not_theme_tokens() {
        let mk = |html: &str| {
            let dir = TempDir::new().unwrap();
            std::fs::create_dir_all(dir.path().join("src")).unwrap();
            std::fs::write(dir.path().join("index.html"), html).unwrap();
            std::fs::write(
                dir.path().join("src/main.ts"),
                "import { createApp } from \"./sdk\";\ncreateApp();\n",
            )
            .unwrap();
            format_lint(&lint_app(dir.path()))
        };
        // Hardcoded text color → warned.
        let bad = mk(r#"<html><body><p style="color:#282217">hi</p></body></html>"#);
        assert!(bad.contains("hardcodes a text color"), "{bad}");
        // rgb() form too.
        let bad2 = mk(r#"<html><body><p style="color: rgb(40,34,23)">hi</p></body></html>"#);
        assert!(bad2.contains("hardcodes a text color"), "{bad2}");
        let unicode = mk(
            r#"<html><body><p>Résumé</p><div style="background-color:#fff;color:#282217">hi</div></body></html>"#,
        );
        assert!(unicode.contains("hardcodes a text color"), "{unicode}");
        // Theme token → NOT warned; and background-color hardcodes are not text.
        let good = mk(r#"<html><body><p style="color:var(--br-text)">hi</p>
               <div style="background-color:#fff">x</div></body></html>"#);
        assert!(!good.contains("hardcodes a text color"), "{good}");
        // A SURFACE token used as text color → its own warning.
        let surf = mk(r#"<html><body><p style="color: var(--br-muted)">hi</p></body></html>"#);
        assert!(surf.contains("SURFACE token as a text color"), "{surf}");
        // …but a surface token as a *background* is fine.
        let okbg =
            mk(r#"<html><body><div style="background: var(--br-muted)">x</div></body></html>"#);
        assert!(!okbg.contains("SURFACE token as a text color"), "{okbg}");
    }

    #[test]
    fn lint_app_requires_visible_progress_for_manual_prompt_loops() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            dir.path().join("index.html"),
            r#"<html><body><main class="br-container">
               <button id="run" class="br-btn">Run</button>
               <div id="out" class="br-output"></div>
               </main></body></html>"#,
        )
        .unwrap();
        std::fs::write(
            src.join("main.ts"),
            r#"import { createApp } from "./sdk";
const br = createApp({ autoChat: false });
document.getElementById("run")!.addEventListener("click", () => br.prompt("go"));
"#,
        )
        .unwrap();

        let formatted = format_lint(&lint_app(dir.path()));
        assert!(formatted.contains("visible step progress"), "{formatted}");
    }

    #[test]
    fn region_names_reads_quoted_values_and_survives_malformed_markup() {
        assert_eq!(
            region_names(r#"<i data-br-region="a"></i><i data-br-region='b'></i>"#),
            vec!["a", "b"]
        );
        // Unquoted and unterminated values are skipped, not guessed at.
        assert!(region_names("<i data-br-region=a>").is_empty());
        assert!(region_names(r#"<i data-br-region="a>"#).is_empty());
        assert!(region_names("data-br-region=").is_empty());
        // A multibyte char right after `=` used to panic the bundler.
        assert!(region_names("<i data-br-region=é>").is_empty());
        assert_eq!(region_names(r#"<i data-br-region="é—ø">"#), vec!["é—ø"]);
    }

    #[test]
    fn lint_app_warns_when_visual_app_lacks_rendered_visual_contract() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            dir.path().join("index.html"),
            r#"<html><body><main class="br-container">
               <button id="run" class="br-btn">Build graph</button>
               <div id="out" class="br-output"></div>
               </main></body></html>"#,
        )
        .unwrap();
        std::fs::write(
            src.join("main.ts"),
            r##"import { createApp } from "./sdk";
const br = createApp({ autoChat: false });
document.getElementById("run")!.addEventListener("click", () => br.run("visualize the graph as prose", "#out"));
"##,
        )
        .unwrap();

        let formatted = format_lint(&lint_app(dir.path()));
        assert!(
            formatted.contains("never asks for a rendered visual"),
            "{formatted}"
        );
    }

    /// Write a minimal app, optionally with a manifest, and lint it.
    fn lint_with(index: &str, main: &str, manifest: Option<&str>) -> String {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("index.html"), index).unwrap();
        std::fs::write(dir.path().join("src/main.ts"), main).unwrap();
        if let Some(m) = manifest {
            std::fs::write(dir.path().join("manifest.json"), m).unwrap();
        }
        format_lint(&lint_app(dir.path()))
    }

    const NO_PROGRESS_INDEX: &str =
        r#"<main class="br-container"><div id="out" class="br-output"></div></main>"#;
    const UNWIRED_CONTROL_INDEX: &str = r#"<main class="br-container">
        <button id="edit" class="br-btn">Edit local row</button>
        <div id="out" class="br-output"></div></main>"#;

    #[test]
    fn lint_autochat_literal_false_does_not_require_progress() {
        for setting in [
            "autoChat: false",
            "autoChat:false",
            "autoChat \n : \t false",
            "\"autoChat\": false",
            "'autoChat' : false",
            "autoChat /* local-only */ : /* disabled */ false",
            "autoChat: false /* keep disabled */, ui: false",
            "autoChat: // local-only\n false",
        ] {
            let main = format!(
                "import {{ createApp }} from \"./sdk\";\nconst br = createApp({{ {setting} }});"
            );
            let out = lint_with(NO_PROGRESS_INDEX, &main, None);
            assert!(!out.contains("visible step progress"), "{setting}: {out}");
            assert!(!out.contains("ERROR"), "{setting}: {out}");
            assert!(out.contains("never calls the agent"), "{setting}: {out}");
            assert!(
                out.contains("Intentional local-only apps may omit agent calls"),
                "{setting}: {out}"
            );
        }
    }

    #[test]
    fn lint_autochat_comments_do_not_enable_agent_work() {
        let main = r#"import { createApp } from "./sdk";
const br = createApp({ autoChat: false });
// br.prompt("not an executed call");
/* Other example: createApp({ autoChat: true }); br.ask("not executed"); */
"#;
        let out = lint_with(NO_PROGRESS_INDEX, main, None);
        assert!(!out.contains("visible step progress"), "{out}");
        assert!(out.contains("never calls the agent"), "{out}");
    }

    #[test]
    fn lint_autochat_dynamic_values_still_require_progress() {
        for setting in [
            "autoChat: enabled",
            "autoChat",
            "\"autoChat\": enabled",
            "'autoChat': false || enabled",
            "autoChat: false /* not a literal value */ || enabled",
            "autoChat: false ? false : enabled",
            "autoChat: enabled ? false : true",
            "autoChat: Boolean(false)",
            "autoChat: true || enabled",
            "autoChat: true ? enabled : false",
            "autoChat: false, ...options",
            "...options, autoChat: false",
            "autoChat: true, ...options",
        ] {
            let main = format!(
                "import {{ createApp }} from \"./sdk\";\nconst br = createApp({{ {setting} }});"
            );
            let out = lint_with(NO_PROGRESS_INDEX, &main, None);
            assert!(out.contains("visible step progress"), "{setting}: {out}");
            assert!(!out.contains("never calls the agent"), "{setting}: {out}");
        }
    }

    #[test]
    fn lint_autochat_false_does_not_hide_another_enabled_setting() {
        for settings in [
            ["autoChat: false", "autoChat: true"],
            ["autoChat: true", "autoChat: false"],
            ["'autoChat': false", "\"autoChat\": enabled"],
        ] {
            let main = format!(
                "import {{ createApp }} from \"./sdk\";\nconst br = createApp({{ {} }});\nconst other = createApp({{ {} }});",
                settings[0], settings[1]
            );
            let out = lint_with(NO_PROGRESS_INDEX, &main, None);
            assert!(
                !out.contains("never calls the agent"),
                "{settings:?}: {out}"
            );
            if settings[1].contains("enabled") {
                assert!(out.contains("visible step progress"), "{settings:?}: {out}");
            }
        }
    }

    #[test]
    fn lint_autochat_duplicate_keys_do_not_prove_progress_for_manual_calls() {
        for setting in [
            "autoChat: true, autoChat: false",
            "autoChat: false, autoChat: true",
            "'autoChat': true, \"autoChat\": false",
            "\"autoChat\": false, 'autoChat': true",
        ] {
            let main = format!(
                "import {{ createApp }} from \"./sdk\";\nconst br = createApp({{ {setting} }});\nbr.prompt(\"analyze\");"
            );
            let out = lint_with(NO_PROGRESS_INDEX, &main, None);
            assert!(out.contains("visible step progress"), "{setting}: {out}");
            assert!(!out.contains("never calls the agent"), "{setting}: {out}");
        }
    }

    #[test]
    fn lint_autochat_computed_keys_do_not_prove_progress_for_manual_calls() {
        for setting in [
            "autoChat: true, [['auto', 'Chat'].join('')]: false",
            "[['auto', 'Chat'].join('')]: false, autoChat: true",
            "autoChat: true, [setting]: false",
            "[setting]: false, autoChat: true",
            "autoChat: true, get [setting]() { return false; }",
            "get [setting]() { return false; }, autoChat: true",
        ] {
            let main = format!(
                "import {{ createApp }} from \"./sdk\";\nconst br = createApp({{ {setting} }});\nbr.ask(\"analyze\");"
            );
            let out = lint_with(NO_PROGRESS_INDEX, &main, None);
            assert!(out.contains("visible step progress"), "{setting}: {out}");
            assert!(!out.contains("never calls the agent"), "{setting}: {out}");
        }
    }

    #[test]
    fn lint_autochat_false_does_not_exempt_manual_agent_calls() {
        for call in [
            "br.prompt(\"analyze\");",
            "br.ask(\"analyze\");",
            "for (const step of steps) { br.prompt(step); }",
        ] {
            let main = format!(
                "import {{ createApp }} from \"./sdk\";\nconst br = createApp({{ autoChat: false }});\n{call}"
            );
            let out = lint_with(NO_PROGRESS_INDEX, &main, None);
            assert!(out.contains("visible step progress"), "{call}: {out}");
            assert!(!out.contains("never calls the agent"), "{call}: {out}");
        }
    }

    #[test]
    fn lint_autochat_false_does_not_hide_unwired_controls() {
        let main =
            "import { createApp } from \"./sdk\";\nconst br = createApp({ autoChat: false });";
        let out = lint_with(UNWIRED_CONTROL_INDEX, main, None);
        assert!(out.contains("interactive controls"), "{out}");
        assert!(!out.contains("visible step progress"), "{out}");
    }

    #[test]
    fn lint_autochat_true_retains_existing_chat_wiring() {
        for setting in [
            "autoChat: true",
            "autoChat:true",
            "autoChat \n : \t true",
            "\"autoChat\": true",
            "'autoChat' : true",
            "autoChat /* chat */ : /* enabled */ true",
            "autoChat: true, ui: true, options: { label: 'local' }",
        ] {
            let main = format!(
                "import {{ createApp }} from \"./sdk\";\nconst br = createApp({{ {setting} }});"
            );
            let out = lint_with(UNWIRED_CONTROL_INDEX, &main, None);
            assert!(!out.contains("interactive controls"), "{setting}: {out}");
            assert!(!out.contains("never calls the agent"), "{setting}: {out}");
            assert!(!out.contains("visible step progress"), "{setting}: {out}");
        }
    }

    #[test]
    fn lint_autochat_comment_mask_preserves_strings_urls_and_template_calls() {
        for call in [
            "const url = \"https://example.invalid\"; br.prompt(url);",
            "const marker = '/* not a comment */'; br.ask(marker);",
            "const marker = '// not a comment'; br.prompt(marker);",
            "const url: string = \"https://example.invalid\"; br.prompt(url);",
            "const result = `prefix ${br.prompt(\"analyze\")} suffix`;",
            "const result = `https://example.invalid/${br.ask(\"analyze\")}`;",
        ] {
            let main = format!(
                "import {{ createApp }} from \"./sdk\";\nconst br = createApp({{ autoChat: false }});\n{call}"
            );
            let out = lint_with(NO_PROGRESS_INDEX, &main, None);
            assert!(out.contains("visible step progress"), "{call}: {out}");
            assert!(!out.contains("never calls the agent"), "{call}: {out}");
        }
    }

    #[test]
    fn lint_autochat_comment_mask_changes_only_comment_bytes() {
        let source = "const url: string = 'https://example.invalid'; /* 中文 */\n\
                      const br = createApp({ autoChat: false }); // br.prompt('not run');\n\
                      const result = `url://${br.ask('run')}`;";
        let masked = analyze_wiring(source).source;
        assert_eq!(masked.len(), source.len());
        assert!(masked.contains("'https://example.invalid'"));
        assert!(masked.contains("`url://${br.ask('run')}`"));
        assert!(masked.contains("const br = createApp({ autoChat: false });"));
        assert!(!masked.contains("中文"));
        assert!(!masked.contains("br.prompt('not run')"));
    }

    #[test]
    fn lint_autochat_quoted_true_examples_do_not_prove_progress() {
        for example in [
            r#"const example = '{ "autoChat": true }';"#,
            r#"const example = "{ 'autoChat': true }";"#,
            r#"const example = `{ "autoChat": true }`;"#,
            r#"const example = '{ autoChat: true }';"#,
        ] {
            let main = format!(
                "import {{ createApp }} from \"./sdk\";\nconst br = createApp({{ autoChat: false }});\n{example}\nbr.prompt(\"analyze\");"
            );
            let out = lint_with(NO_PROGRESS_INDEX, &main, None);
            assert!(out.contains("visible step progress"), "{example}: {out}");
        }
    }

    #[test]
    fn lint_autochat_false_accepts_local_control_handlers() {
        let main = r#"import { createApp } from "./sdk";
const br = createApp({ autoChat: false });
document.getElementById("edit")!.addEventListener("click", () => {
  document.getElementById("out")!.textContent = "Local row updated";
});
"#;
        let out = lint_with(UNWIRED_CONTROL_INDEX, main, None);
        assert!(!out.contains("interactive controls"), "{out}");
        assert!(!out.contains("visible step progress"), "{out}");
        assert!(!out.contains("ERROR"), "{out}");
    }

    fn manifest_json(system_prompt: &str, ui_enabled: bool) -> String {
        format!(
            r#"{{"id":"a","title":"A","description":"","kind":"agentic","entry":"index.html",
                "created_at":0,"updated_at":0,
                "agent":{{"system_prompt":{},"capabilities":{{"ui":{{"enabled":{ui_enabled}}}}}}}}}"#,
            serde_json::to_string(system_prompt).unwrap()
        )
    }

    /// A manifest carrying a raw `surface` JSON object (SDK v2 contract).
    fn manifest_with_surface(surface: &str) -> String {
        format!(
            r#"{{"id":"a","title":"A","description":"","kind":"agentic","entry":"index.html",
                "created_at":0,"updated_at":0,"surface":{surface}}}"#
        )
    }

    const CHATTY_INDEX: &str = r#"<html><body><main class="br-container">
        <div class="br-card" data-br-chat></div></main></body></html>"#;
    const CHATTY_MAIN: &str = "import { createApp } from \"./sdk\";\ncreateApp();\n";

    /// The agent drawing with `ui_chart` satisfies the visualization contract —
    /// it renders straight into the page, no markdown fence needed.
    #[test]
    fn lint_accepts_ui_chart_as_the_visualization_contract() {
        let out = lint_with(
            CHATTY_INDEX,
            CHATTY_MAIN,
            Some(&manifest_json(
                "Visualize the gene counts: call ui_chart with the top 10.",
                true,
            )),
        );
        assert!(!out.contains("never asks for a rendered visual"), "{out}");
    }

    /// A `data-br-region` is an agent render target, not a clickable control.
    /// Matching it as one made every app that declares a target warn.
    #[test]
    fn lint_does_not_treat_a_render_region_as_an_unwired_control() {
        let index = r#"<html><body><main class="br-container" data-br-main>
            <div class="br-card" data-br-chat></div>
            <section data-br-region="results"></section></main></body></html>"#;
        let out = lint_with(index, CHATTY_MAIN, None);
        assert!(!out.contains("wires no events"), "{out}");
    }

    #[test]
    fn lint_flags_duplicate_region_names_as_ambiguous_targets() {
        let index = r#"<html><body><main class="br-container">
            <div class="br-card" data-br-chat></div>
            <section data-br-region="results"></section>
            <section data-br-region="results"></section></main></body></html>"#;
        let out = lint_with(index, CHATTY_MAIN, None);
        assert!(out.contains("more than once"), "{out}");
        assert!(out.contains("@region:results"), "{out}");
    }

    #[test]
    fn lint_flags_ui_code_in_an_app_that_disabled_the_capability() {
        let main = "import { createApp } from \"./sdk\";\nconst br = createApp();\nbr.ui.onState(() => {});\n";
        let out = lint_with(
            CHATTY_INDEX,
            main,
            Some(&manifest_json("Answer questions.", false)),
        );
        assert!(out.contains("capabilities.ui.enabled = false"), "{out}");
        assert!(
            out.to_lowercase().contains("error"),
            "must block the build: {out}"
        );
    }

    #[test]
    fn lint_flags_a_prompt_that_calls_ui_tools_the_app_does_not_grant() {
        let out = lint_with(
            CHATTY_INDEX,
            CHATTY_MAIN,
            Some(&manifest_json(
                "Always call ui_panel to show results.",
                false,
            )),
        );
        assert!(out.contains("it has none"), "{out}");
    }

    // ── SDK v2: custom components + reactive-state bindings ─────────────────

    /// (Rule 1) A component declared in surface.components but never registered in
    /// main.ts is an error; registering it clears it.
    #[test]
    fn lint_flags_declared_component_not_registered_in_main() {
        let surface = r#"{"components":[{"name":"pathway_map"}]}"#;
        let missing = lint_with(
            CHATTY_INDEX,
            CHATTY_MAIN,
            Some(&manifest_with_surface(surface)),
        );
        assert!(missing.contains("never registers it"), "{missing}");
        assert!(missing.contains("pathway_map"), "{missing}");

        let main = "import { createApp } from \"./sdk\";\nconst br = createApp();\nbr.components.register(\"pathway_map\", { mount() {} });\n";
        let ok = lint_with(CHATTY_INDEX, main, Some(&manifest_with_surface(surface)));
        assert!(!ok.contains("never registers it"), "{ok}");
    }

    /// (Rule 2) A registration with a literal name that isn't declared is an error;
    /// a registration with a NON-literal (dynamic) name fails closed regardless.
    #[test]
    fn lint_flags_registered_but_undeclared_and_fails_closed_on_dynamic_name() {
        // Literal name, no surface declaration → error.
        let main = "import { createApp } from \"./sdk\";\nconst br = createApp();\nbr.components.register(\"gene_card\", { mount() {} });\n";
        let undeclared = lint_with(CHATTY_INDEX, main, Some(&manifest_json("Answer.", true)));
        assert!(undeclared.contains("gene_card"), "{undeclared}");
        assert!(undeclared.contains("never declares"), "{undeclared}");

        // Dynamic name → fail closed with the string-literal error.
        let dynamic = "import { createApp } from \"./sdk\";\nconst br = createApp();\nconst n = pick();\nbr.components.register(n, { mount() {} });\n";
        let dyn_out = lint_with(CHATTY_INDEX, dynamic, Some(&manifest_json("Answer.", true)));
        assert!(dyn_out.contains("non-literal component name"), "{dyn_out}");
        assert!(
            dyn_out.to_lowercase().contains("error"),
            "must block: {dyn_out}"
        );

        // Declared + literal → passes both checks.
        let surface = r#"{"components":[{"name":"gene_card"}]}"#;
        let ok = lint_with(CHATTY_INDEX, main, Some(&manifest_with_surface(surface)));
        assert!(!ok.contains("never declares"), "{ok}");
        assert!(!ok.contains("non-literal"), "{ok}");
    }

    /// (Rule 3) Feeding agent-controlled `props` into an HTML sink warns; textContent
    /// does not.
    #[test]
    fn lint_warns_when_component_props_feed_an_html_sink() {
        let surface = r#"{"components":[{"name":"card"}]}"#;
        let bad = "import { createApp } from \"./sdk\";\nconst br = createApp();\nbr.components.register(\"card\", { mount(el, props) { el.innerHTML = props.body; } });\n";
        let out = lint_with(CHATTY_INDEX, bad, Some(&manifest_with_surface(surface)));
        assert!(out.contains("innerHTML/insertAdjacentHTML"), "{out}");

        // insertAdjacentHTML variant also warns.
        let bad2 = "import { createApp } from \"./sdk\";\nconst br = createApp();\nbr.components.register(\"card\", { mount(el, props) { el.insertAdjacentHTML(\"beforeend\", props.html); } });\n";
        let out2 = lint_with(CHATTY_INDEX, bad2, Some(&manifest_with_surface(surface)));
        assert!(out2.contains("innerHTML/insertAdjacentHTML"), "{out2}");

        // textContent is a safe sink → no warning.
        let good = "import { createApp } from \"./sdk\";\nconst br = createApp();\nbr.components.register(\"card\", { mount(el, props) { el.textContent = props.body; } });\n";
        let ok = lint_with(CHATTY_INDEX, good, Some(&manifest_with_surface(surface)));
        assert!(!ok.contains("innerHTML/insertAdjacentHTML"), "{ok}");
    }

    /// (Rule 4) Bindings in the markup without a declared state schema warn; a
    /// state_schema silences it.
    #[test]
    fn lint_warns_on_bindings_without_a_state_schema() {
        let index = r#"<html><body><main class="br-container">
            <div class="br-card" data-br-chat></div>
            <span data-br-bind="/cohort/count"></span></main></body></html>"#;
        let out = lint_with(index, CHATTY_MAIN, Some(&manifest_json("Answer.", true)));
        assert!(out.contains("no surface.state_schema"), "{out}");

        let surface = r#"{"state_schema":{"type":"object"}}"#;
        let ok = lint_with(index, CHATTY_MAIN, Some(&manifest_with_surface(surface)));
        assert!(!ok.contains("no surface.state_schema"), "{ok}");
    }

    /// (Rule 5) Binding `data-br-bind-attr` to an on* handler or `style` is a
    /// build error; a safe attribute (href) is not.
    #[test]
    fn lint_errors_on_bind_attr_to_event_handler_or_style() {
        let on_attr = r#"<html><body><main class="br-container">
            <div class="br-card" data-br-chat></div>
            <button data-br-bind-attr="onclick:/handler">go</button></main></body></html>"#;
        let out = lint_with(on_attr, CHATTY_MAIN, None);
        assert!(out.contains("the runtime refuses"), "{out}");
        assert!(out.to_lowercase().contains("error"), "must block: {out}");

        let style_attr = r#"<html><body><main class="br-container">
            <div class="br-card" data-br-chat></div>
            <div data-br-bind-attr="style:/css"></div></main></body></html>"#;
        let styled = lint_with(style_attr, CHATTY_MAIN, None);
        assert!(styled.contains("the runtime refuses"), "{styled}");

        // href is a safe target → no bind-attr error (a state-schema warning is fine).
        let href_attr = r#"<html><body><main class="br-container">
            <div class="br-card" data-br-chat></div>
            <a data-br-bind-attr="href:/url">link</a></main></body></html>"#;
        let ok = lint_with(href_attr, CHATTY_MAIN, None);
        assert!(!ok.contains("data-br-bind-attr to the"), "{ok}");
    }

    // ── SDK v2 Phase 3: typed actions + app→agent signals ──────────────────

    /// (Rule 7a) An action declared in surface.actions but never registered in
    /// main.ts is an error; registering it clears the finding.
    #[test]
    fn lint_flags_declared_action_not_registered_in_main() {
        let surface = r#"{"actions":[{"name":"run_query"}]}"#;
        let missing = lint_with(
            CHATTY_INDEX,
            CHATTY_MAIN,
            Some(&manifest_with_surface(surface)),
        );
        assert!(missing.contains("never registers it"), "{missing}");
        assert!(missing.contains("run_query"), "{missing}");

        let main = "import { createApp } from \"./sdk\";\nconst br = createApp();\nbr.actions.register(\"run_query\", () => {});\n";
        let ok = lint_with(CHATTY_INDEX, main, Some(&manifest_with_surface(surface)));
        assert!(!ok.contains("never registers it"), "{ok}");
    }

    /// (Rules 7b/7c) A registration with a literal name that isn't declared is an
    /// error; a NON-literal (dynamic) name fails closed regardless.
    #[test]
    fn lint_flags_registered_but_undeclared_action_and_fails_closed_on_dynamic_name() {
        // Literal name, no surface declaration → error.
        let main = "import { createApp } from \"./sdk\";\nconst br = createApp();\nbr.actions.register(\"save\", () => {});\n";
        let undeclared = lint_with(CHATTY_INDEX, main, Some(&manifest_json("Answer.", true)));
        assert!(undeclared.contains("save"), "{undeclared}");
        assert!(undeclared.contains("never declares"), "{undeclared}");

        // Dynamic name → fail closed with the string-literal error.
        let dynamic = "import { createApp } from \"./sdk\";\nconst br = createApp();\nconst n = pick();\nbr.actions.register(n, () => {});\n";
        let dyn_out = lint_with(CHATTY_INDEX, dynamic, Some(&manifest_json("Answer.", true)));
        assert!(dyn_out.contains("non-literal action name"), "{dyn_out}");
        assert!(
            dyn_out.to_lowercase().contains("error"),
            "must block: {dyn_out}"
        );

        // Declared + literal → passes both checks.
        let surface = r#"{"actions":[{"name":"save"}]}"#;
        let ok = lint_with(CHATTY_INDEX, main, Some(&manifest_with_surface(surface)));
        assert!(!ok.contains("never declares"), "{ok}");
        assert!(!ok.contains("non-literal"), "{ok}");
    }

    /// (Rule 7d) A signal emitted with a literal name the manifest never declares
    /// is an error; a dynamic emit name is only a warning (runtime-validated); a
    /// declared literal is clean.
    #[test]
    fn lint_flags_undeclared_signal_emit_and_warns_on_dynamic_name() {
        // Literal, undeclared → error.
        let main = "import { createApp } from \"./sdk\";\nconst br = createApp();\nbr.signals.emit(\"row_selected\", { id: 1 });\n";
        let undeclared = lint_with(CHATTY_INDEX, main, Some(&manifest_json("Answer.", true)));
        assert!(undeclared.contains("row_selected"), "{undeclared}");
        assert!(undeclared.contains("never declares"), "{undeclared}");

        // Dynamic name → warning, not a build-blocking error.
        let dynamic = "import { createApp } from \"./sdk\";\nconst br = createApp();\nconst s = pick();\nbr.signals.emit(s, {});\n";
        let dyn_out = lint_with(CHATTY_INDEX, dynamic, Some(&manifest_json("Answer.", true)));
        assert!(dyn_out.contains("non-literal signal name"), "{dyn_out}");
        assert!(
            !dyn_out.to_lowercase().contains("error"),
            "dynamic emit is survivable: {dyn_out}"
        );

        // Declared literal → clean.
        let surface = r#"{"signals":[{"name":"row_selected"}]}"#;
        let ok = lint_with(CHATTY_INDEX, main, Some(&manifest_with_surface(surface)));
        assert!(!ok.contains("never declares"), "{ok}");
    }

    /// (Rule 7e) Assembling an English prompt into `.run(...)` while the manifest
    /// declares typed actions warns; using `br.call(...)` does not.
    #[test]
    fn lint_warns_on_prompt_concat_when_typed_actions_exist() {
        let surface = r#"{"actions":[{"name":"run_query"}]}"#;
        let concat = "import { createApp } from \"./sdk\";\nconst br = createApp();\nconst q = \"genes\";\nbr.run(`find ${q} in the graph`);\n";
        let out = lint_with(CHATTY_INDEX, concat, Some(&manifest_with_surface(surface)));
        assert!(out.contains("prefer br.call(name, args)"), "{out}");

        // Explicit `" +` / `+ "` concatenation also warns.
        let plus = "import { createApp } from \"./sdk\";\nconst br = createApp();\nconst q = \"x\";\nbr.run(\"find \" + q + \" now\");\n";
        let plus_out = lint_with(CHATTY_INDEX, plus, Some(&manifest_with_surface(surface)));
        assert!(
            plus_out.contains("prefer br.call(name, args)"),
            "{plus_out}"
        );

        // Using the typed path (br.call) → no warning.
        let typed = "import { createApp } from \"./sdk\";\nconst br = createApp();\nbr.actions.register(\"run_query\", () => {});\nbr.call(\"run_query\", { q: \"genes\" });\n";
        let ok = lint_with(CHATTY_INDEX, typed, Some(&manifest_with_surface(surface)));
        assert!(!ok.contains("prefer br.call(name, args)"), "{ok}");
    }

    /// Issue #62, second half — the environment [`run_esbuild`] spawns with.
    ///
    /// esbuild only *parses* the agent's TypeScript, so this child is weaker
    /// than the smoke test's. It still gets the same rule, for a different
    /// reason: when no local esbuild exists the bundler falls back to
    /// `npx --yes esbuild`, which downloads a package from the public registry
    /// at build time and runs it (lifecycle scripts included). A just-fetched
    /// third-party package has no business holding `biorouterd`'s auth key, and
    /// the tool needs no credential of any kind.
    #[cfg(unix)]
    mod esbuild_child_env {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        /// Child half. `#[ignore]` keeps it out of a normal run.
        #[test]
        #[ignore]
        fn leak_probe_prints_esbuild_child_env() {
            let cmd = find_esbuild().expect("BIOROUTER_ESBUILD_BIN shim");
            let work = TempDir::new().unwrap();
            let entry = work.path().join("main.ts");
            std::fs::write(&entry, "const x: number = 1;\n").unwrap();
            let report = run_esbuild(&cmd, &entry, &work.path().join("app.js")).expect("spawn");
            let canary = std::env::var("BR_TEST_CANARY").unwrap_or_default();
            // Report BioRouter's namespace, the variables the parent injected,
            // and any line carrying the canary under *any* key. Everything else
            // is dropped: printing the whole environment of whoever runs the
            // suite would be its own small leak.
            let filtered = report
                .log
                .lines()
                .filter(|line| {
                    let key = line.split('=').next().unwrap_or("");
                    if key == "BR_TEST_CANARY" {
                        return false; // the channel carrying the canary is not the leak
                    }
                    key.starts_with("BIOROUTER_")
                        || matches!(key, "PATH" | "HOME" | "BR_TEST_USER_VAR")
                        || (!canary.is_empty() && line.contains(&canary))
                })
                .collect::<Vec<_>>()
                .join("\n");
            println!("BEGIN_CHILD_ENV");
            println!("{filtered}");
            println!("END_CHILD_ENV");
        }

        /// Run the probe in a fresh copy of this test binary whose environment
        /// is the daemon's, with `BIOROUTER_ESBUILD_BIN` pointing at a shim that
        /// prints its own environment — so the real `run_esbuild` command is
        /// what gets measured.
        fn run_esbuild_leak_probe(canary: &str) -> String {
            let shim_dir = TempDir::new().expect("temp dir");
            let shim = shim_dir.path().join("esbuild");
            // Absolute `printenv`, and `PATH`/`HOME` pinned below — see
            // [`crate::agent_drafter::printenv_bin`].
            std::fs::write(
                &shim,
                format!("#!/bin/sh\nexec {}\n", crate::agent_drafter::printenv_bin()),
            )
            .expect("write esbuild shim");
            std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755))
                .expect("chmod esbuild shim");

            let m = module_path!();
            let without_crate = m.split_once("::").map(|(_, rest)| rest).unwrap_or(m);
            let probe = format!("{without_crate}::leak_probe_prints_esbuild_child_env");
            let exe = std::env::current_exe().expect("test binary path");
            let out = std::process::Command::new(exe)
                .args([
                    "--exact",
                    &probe,
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("BIOROUTER_ESBUILD_BIN", &shim)
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", shim_dir.path())
                .env("BIOROUTER_SERVER__SECRET_KEY", canary)
                .env("BR_TEST_CANARY", canary)
                .env("BIOROUTER_PORT", "54321")
                .env("BR_TEST_USER_VAR", "user-env-ok")
                .output()
                .expect("re-invoking the test binary must work");
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            stdout
                .split_once("BEGIN_CHILD_ENV\n")
                .and_then(|(_, rest)| rest.split_once("END_CHILD_ENV"))
                .map(|(body, _)| body.to_string())
                .unwrap_or_else(|| {
                    panic!(
                        "probe produced no child environment.\nstdout:\n{stdout}\nstderr:\n{}",
                        String::from_utf8_lossy(&out.stderr)
                    )
                })
        }

        #[test]
        fn daemon_secret_never_reaches_the_esbuild_child() {
            const CANARY: &str = "canary-daemon-secret-esbuild-62";
            let child_env = run_esbuild_leak_probe(CANARY);
            assert!(
                !child_env.contains(CANARY),
                "issue #62: the daemon's auth secret reached the bundler child, and with the \
                 `npx --yes esbuild` fallback that hands biorouterd's key to a package \
                 downloaded at build time.\nchild env:\n{child_env}"
            );
            assert!(
                !child_env.contains("BIOROUTER_SERVER__SECRET_KEY"),
                "the key name itself must be gone, not just the value:\n{child_env}"
            );
        }

        /// The other direction: esbuild is resolved through `PATH`, and the npx
        /// fallback needs `HOME` for the npm cache. A truncated child
        /// environment is its own regression — issue #24 was exactly that.
        #[test]
        fn esbuild_child_still_gets_the_environment_it_needs() {
            let child_env = run_esbuild_leak_probe("canary-unused-esbuild-62");
            for expected in [
                "PATH=",
                "HOME=",
                "BIOROUTER_PORT=54321",
                "BR_TEST_USER_VAR=user-env-ok",
            ] {
                assert!(
                    child_env.lines().any(|l| l.starts_with(expected)),
                    "{expected} is missing: stripping the daemon credential must not censor the \
                     environment the bundler needs:\n{child_env}"
                );
            }
        }
    }

    /// The bundler discovery is confined to the checkout it is run from.
    ///
    /// ⚠ This is the "passes locally, for a reason CI never has" defect, in the
    /// shape that is hardest to see: the tool the tests found was real and
    /// working, it was simply not in the tree under test. `find_esbuild` walked a
    /// flat six ancestors, and six is exactly far enough to leave a worktree —
    /// from `<repo>/.claude/worktrees/<name>/crates/biorouter-mcp` the sixth step
    /// is `<repo>` itself. So every esbuild-dependent test in a worktree with no
    /// install of its own quietly borrowed the main checkout's bundler, and no
    /// failure anywhere said so.
    ///
    /// Tested through `esbuild_in_checkout` rather than `find_esbuild`, because
    /// the latter reads `current_dir()` — process-global state that a parallel
    /// test run cannot set without racing every other test in this binary.
    mod esbuild_discovery {
        use super::super::esbuild_in_checkout;
        use tempfile::TempDir;

        /// An outer checkout that HAS an install, and a worktree inside it that
        /// does not — the real layout, with `.claude/worktrees/<name>` two levels
        /// down and a `.git` FILE rather than a directory.
        fn nested_checkouts() -> (TempDir, std::path::PathBuf) {
            let root = TempDir::new().unwrap();
            let outer = root.path().join("BioRouter");
            let bin = outer.join("ui/desktop/node_modules/.bin");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::write(bin.join("esbuild"), "#!/bin/sh\n").unwrap();
            std::fs::write(outer.join(".git"), "gitdir: elsewhere\n").unwrap();

            let worktree = outer.join(".claude/worktrees/a-worktree");
            std::fs::create_dir_all(worktree.join("crates/biorouter-mcp")).unwrap();
            std::fs::write(worktree.join(".git"), "gitdir: elsewhere\n").unwrap();
            (root, worktree)
        }

        #[test]
        fn a_worktree_without_its_own_install_finds_nothing() {
            let (_root, worktree) = nested_checkouts();
            assert_eq!(
                esbuild_in_checkout(&worktree.join("crates/biorouter-mcp")),
                None,
                "a worktree borrowed the outer checkout's bundler"
            );
        }

        #[test]
        fn a_checkouts_own_install_is_found_from_a_subdirectory() {
            let (_root, worktree) = nested_checkouts();
            let bin = worktree.join("ui/desktop/node_modules/.bin");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::write(bin.join("esbuild"), "#!/bin/sh\n").unwrap();
            assert_eq!(
                esbuild_in_checkout(&worktree.join("crates/biorouter-mcp")),
                Some(bin.join("esbuild")),
                "the worktree's own install must still be found from a crate directory"
            );
        }

        #[test]
        fn the_root_of_a_checkout_is_searched_before_the_boundary_stops_it() {
            // An ordinary clone keeps `.git` and `ui/` in the same directory, so
            // a bound that fired before testing the candidate would find nothing
            // anywhere — which would look like "esbuild is missing" on every
            // developer machine.
            let (_root, worktree) = nested_checkouts();
            let outer = worktree
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
                .unwrap();
            assert_eq!(
                esbuild_in_checkout(outer),
                Some(outer.join("ui/desktop/node_modules/.bin/esbuild"))
            );
        }
    }
}
