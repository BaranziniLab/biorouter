//! Build pipeline for Agent Drafter apps.
//!
//! Apps are authored in TypeScript (`src/main.ts` importing `src/sdk.ts`) and
//! bundled into a single browser-runnable `dist/app.js` with **esbuild**. esbuild
//! is located via (in order): `$BIOROUTER_ESBUILD_BIN`, the desktop app's bundled
//! `node_modules/.bin/esbuild` (dev tree), `esbuild` on `PATH`, then `npx esbuild`.
//!
//! When no esbuild is available (e.g. a headless CLI install with no Node), a
//! best-effort vendored type-stripper concatenates the sources into a runnable
//! bundle so simple apps still work. The stripper is intentionally conservative;
//! complex TypeScript should be built where esbuild is present.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Mutex, MutexGuard};
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
    if has_controls
        && !wiring.contains("addEventListener")
        && !auto_chat.may_call_agent()
        && !wiring.contains("onclick")
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
        s.push_str("Fix the ERRORs (they break backend access or portability), then rebuild.");
    }
    s
}

/// Outcome of a build.
#[derive(Debug, Clone)]
pub struct BuildReport {
    pub ok: bool,
    /// "esbuild" or "fallback".
    pub used: String,
    /// Combined stdout/stderr (or fallback notes).
    pub log: String,
}

static NPX_ESBUILD_LOCK: Mutex<()> = Mutex::new(());
static NPX_ESBUILD_UNAVAILABLE: AtomicBool = AtomicBool::new(false);
const ESBUILD_TIMEOUT: Duration = Duration::from_secs(60);

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

fn npx_cache_dir() -> PathBuf {
    std::env::temp_dir().join(format!("biorouter-npx-cache-{}", std::process::id()))
}

/// Locate an esbuild executable. Returns `(program, leading_args)` so the caller
/// can support both a direct binary and `npx esbuild`.
fn find_esbuild() -> Option<(String, Vec<String>)> {
    if let Ok(bin) = std::env::var("BIOROUTER_ESBUILD_BIN") {
        if !bin.trim().is_empty() && Path::new(&bin).exists() {
            return Some((bin, vec![]));
        }
    }
    // Dev tree: ui/desktop/node_modules/.bin/esbuild, discovered relative to CWD
    // and a couple of ancestors (tests/CLI may run from a subdir).
    if let Ok(cwd) = std::env::current_dir() {
        let mut dir: Option<&Path> = Some(cwd.as_path());
        for _ in 0..6 {
            let Some(d) = dir else { break };
            let cand = d.join("ui/desktop/node_modules/.bin/esbuild");
            if cand.exists() {
                return Some((cand.to_string_lossy().to_string(), vec![]));
            }
            dir = d.parent();
        }
    }
    if which("esbuild") {
        return Some(("esbuild".to_string(), vec![]));
    }
    if !NPX_ESBUILD_UNAVAILABLE.load(AtomicOrdering::Acquire) && which("npx") {
        return Some(("npx".to_string(), vec!["--yes".into(), "esbuild".into()]));
    }
    None
}

fn which(prog: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path) {
        let cand = dir.join(prog);
        if cand.is_file() {
            return true;
        }
        #[cfg(windows)]
        {
            if dir.join(format!("{prog}.cmd")).is_file()
                || dir.join(format!("{prog}.exe")).is_file()
            {
                return true;
            }
        }
    }
    false
}

/// Build an app project rooted at `project_dir`. Expects `src/main.ts`; writes
/// `dist/app.js`. Returns a report (never errors on a *build* failure — inspect
/// `report.ok`); only returns `Err` for filesystem problems.
/// The App SDK an app *should* be bundling — the template compiled into this
/// binary.
pub fn sdk_template() -> &'static str {
    include_str!("templates/sdk.ts")
}

/// A short fingerprint of the current App SDK. Stored on the manifest after a
/// build so a daemon can tell that an app's bundle predates an SDK upgrade and
/// rebuild it, rather than serving a stale runtime that ignores frames the
/// server now sends (this is how agent-driven UI reaches apps built before it
/// existed).
pub fn sdk_fingerprint() -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    sdk_template().hash(&mut h);
    format!("{:x}", h.finish())
}

/// Overwrite the app's vendored `src/sdk.ts` when it differs from the template.
///
/// The SDK is a *provided runtime*, not authored code: `create_app` writes it and
/// nothing is meant to edit it. Refreshing it here means "rebuild the app" is all
/// it takes to pick up SDK fixes, instead of a manual re-copy into every project.
/// Returns whether it was replaced.
fn refresh_sdk(project_dir: &Path) -> bool {
    let path = project_dir.join("src/sdk.ts");
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    if current == sdk_template() {
        return false;
    }
    std::fs::write(&path, sdk_template()).is_ok()
}

pub fn build_app(project_dir: &Path) -> std::io::Result<BuildReport> {
    let entry = project_dir.join("src/main.ts");
    if !entry.exists() {
        return Ok(BuildReport {
            ok: false,
            used: "none".into(),
            log: "src/main.ts not found, so there is nothing to build".into(),
        });
    }
    let refreshed = refresh_sdk(project_dir);
    let dist = project_dir.join("dist");
    std::fs::create_dir_all(&dist)?;
    let out = dist.join("app.js");

    let note = |mut report: BuildReport| {
        if refreshed {
            report.log = format!(
                "Refreshed src/sdk.ts to the current App SDK.\n{}",
                report.log
            );
        }
        report
    };

    if let Some((program, lead)) = find_esbuild() {
        match run_esbuild(&program, &lead, &entry, &out) {
            // esbuild ran: surface its result either way rather than silently
            // masking a syntax error with the weaker fallback.
            Ok(report) => return Ok(note(report)),
            Err(e) => {
                // esbuild could not be spawned; fall through to the stripper.
                let mut report = fallback_bundle(project_dir, &out)?;
                report.log = format!("esbuild unavailable ({e}); used fallback.\n{}", report.log);
                return Ok(note(report));
            }
        }
    }

    fallback_bundle(project_dir, &out).map(note)
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
fn run_esbuild(
    program: &str,
    lead: &[String],
    entry: &Path,
    out: &Path,
) -> std::io::Result<BuildReport> {
    run_esbuild_with_timeout(program, lead, entry, out, ESBUILD_TIMEOUT)
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
    entry: &Path,
    out: &Path,
    timeout: Duration,
) -> std::io::Result<BuildReport> {
    let npx_guard = lock_npx_esbuild(program);
    let mut cmd = Command::new(program);
    if npx_guard.is_some() {
        cmd.env("npm_config_cache", npx_cache_dir());
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
    })
}

/// Best-effort transpile without esbuild: concatenate `sdk.ts` + `main.ts`,
/// drop module syntax, and strip the common TypeScript type constructs. Wrapped
/// in an IIFE. Good enough for the generated starter apps; not a real compiler.
fn fallback_bundle(project_dir: &Path, out: &Path) -> std::io::Result<BuildReport> {
    let src = project_dir.join("src");
    let sdk = std::fs::read_to_string(src.join("sdk.ts")).unwrap_or_default();
    let main = std::fs::read_to_string(src.join("main.ts"))?;
    let mut body = String::new();
    body.push_str(&strip_module_syntax(&sdk));
    body.push('\n');
    body.push_str(&strip_module_syntax(&main));
    let js = format!(
        "/* Agent Drafter fallback bundle (no esbuild present). */\n(function(){{\n{body}\n}})();\n"
    );
    std::fs::write(out, js)?;
    Ok(BuildReport {
        ok: true,
        used: "fallback".into(),
        log: "Bundled with the vendored type-stripper (esbuild not found). \
              For full TypeScript support install esbuild or build in the desktop app."
            .into(),
    })
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

fn strip_colon_types(line: &str) -> String {
    // Only strip when a `:` is followed by a space + capitalized/primitive type
    // and not inside a string. Very conservative to avoid mangling object/CSS.
    // This handles `(x: Type)`, `let y: Type =`, `): RetType {`.
    let mut out = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut in_str: Option<char> = None;
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
        let report = fallback_bundle(dir.path(), &out).unwrap();
        assert!(report.ok);
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
        /// name this state (see `zombie_aware_state`), so elsewhere the variant
        /// is matched but never constructed.
        #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
        Zombie,
        /// Still a live process.
        Alive,
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

    /// Linux exposes the state letter directly, so a zombie is nameable even
    /// where nothing ever reaps it -- a container whose pid 1 is not a
    /// subreaper never turns the orphan below into `Gone`.
    #[cfg(all(unix, target_os = "linux"))]
    fn zombie_aware_state(pid: i32) -> DescendantState {
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            // Z is "zombie", X is "dead": terminated, awaiting a reaper.
            Ok(stat) => match proc_stat_state(&stat) {
                Some('Z' | 'X') => DescendantState::Zombie,
                _ => DescendantState::Alive,
            },
            // Reaped between the kill() and this read.
            Err(_) => DescendantState::Gone,
        }
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
            if state != DescendantState::Alive || started.elapsed() >= deadline {
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
        // Under load the shim can be killed before it reaches its
        // `echo "$!" > "$pidfile"`, leaving that file absent or truncated. Such
        // an attempt established no descendant at all, so it proves nothing in
        // either direction -- it is a harness miss, not a reaper failure. Retry
        // it instead of reading a pid that was never written.
        const ATTEMPTS: usize = 5;
        let mut descendant = None;
        for _ in 0..ATTEMPTS {
            let _ = std::fs::remove_file(&descendant_pid);
            let started = Instant::now();
            let error = run_esbuild_with_timeout(
                shim.to_str().unwrap(),
                &[descendant_pid.to_string_lossy().into_owned()],
                &entry,
                &out,
                Duration::from_secs(1),
            )
            .expect_err("the hung compiler must time out");

            assert_eq!(error.kind(), io::ErrorKind::TimedOut);
            assert!(started.elapsed() < Duration::from_secs(3));

            descendant = std::fs::read_to_string(&descendant_pid)
                .ok()
                .and_then(|recorded| recorded.trim().parse::<i32>().ok());
            if descendant.is_some() {
                break;
            }
        }
        let pid = descendant.unwrap_or_else(|| {
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
        const DEADLINE: Duration = Duration::from_secs(2);
        let state = await_descendant_death(pid, DEADLINE);
        assert!(
            matches!(state, DescendantState::Gone | DescendantState::Zombie),
            "the compiler's descendant (pid {pid}) was still {state:?} {DEADLINE:?} after \
             terminate_esbuild returned; the SIGKILL to the process group must leave it \
             dead (Zombie, awaiting its new parent's wait) or reaped (Gone)"
        );
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
            let (program, lead) = find_esbuild().expect("BIOROUTER_ESBUILD_BIN shim");
            let work = TempDir::new().unwrap();
            let entry = work.path().join("main.ts");
            std::fs::write(&entry, "const x: number = 1;\n").unwrap();
            let report =
                run_esbuild(&program, &lead, &entry, &work.path().join("app.js")).expect("spawn");
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
}
