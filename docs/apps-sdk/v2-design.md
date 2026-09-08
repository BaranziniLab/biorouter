# BioRouter Apps SDK v2 design

> **What this is.** The design of record for Apps SDK v2 — nine pillars that evolve Agent Drafter from "apps that are mostly chatbots with a run button" into a real application SDK, with a component catalog, shared reactive state, platform APIs behind `br.*`, capability-gated security, and multi-agent worker profiles.
> **Status:** Current. Authored 2026-07-12 after an adversarial review (feasibility against the code, security, completeness) with the findings incorporated; no later revision is recorded in this file. It remains the live design for work still in flight — some pillars have shipped, some are landing, some are still design-only.
> **Audience:** developers working on Agent Drafter and the Apps SDK.
> **Which parts are real.** This document describes *intent*. [The SDK reference](sdk-reference.md) documents what actually ships and flags every Partial / Design-only item; read it whenever you need to know whether something exists today. See the [pillar map](#the-nine-pillars-at-a-glance) below.

**Scope.** Evolve Agent Drafter so its apps expose the *whole* BioRouter platform — any provider, knowledge bases, MCP extensions, skills — through diverse, agent-driven, aesthetically distinct, secure user interfaces, including BioOKF-Studio-class experiences (live network viewers, avatar and scene control, workbenches) that today cannot be expressed at all.

## Terms and shorthand used here

This design draws on an industry and research survey, and uses its vocabulary throughout. The short version:

| Term | What it means here |
|---|---|
| **Family A / B / C / D** | The four architectural families for getting a UI out of an LLM, defined in [the survey table](#four-architectural-families-for-llm-driven-ui). A = model writes code; B = model fills slots in pre-authored components; C = model emits a declarative UI DSL from a catalog; D = the app owns the UI and the agent drives it over events and shared state. |
| **BioOKF Studio** | The exemplar application described in [the vision](#what-v2-is-trying-to-make-possible): a network viewer whose agent drives the GUI through a small semantic API (`selectNode`, `search`, `narrate`, `getState`) rather than through prose. |
| **DynaVis** | The research system behind the pattern "natural language bootstraps, a synthesized persistent control refines" — the most user-validated generative-UI finding in the survey. The **DynaVis rule** is this design's instruction to emit a persistent bound control after an NL-driven parameter change. |
| **Graphologue / Sensecape** | Research systems supporting "externalize agent output as navigable structure (graph, space, tree), not transcript". |
| **Marimo / Ink & Switch "tools, not apps"** | The source of "small, independently-owned, reactive units with visible dependencies keep users oriented when the AI mutates the UI". |
| **Voyager** | The source of "a skill library beats per-turn improvisation": named, reusable capabilities compound. |
| **Townie** | The source of the engineering lessons cited here: shape APIs the way LLMs expect, a few dozen curated examples beat maximalist prompts, error-feedback loops need real infrastructure, and evals come before features. |
| **Airbnb's hardest-won lesson** | Version-stamp every frame and render unknown components as a neutral labelled placeholder, so a newer server never breaks an older client. |
| **Idiomorph / morphing** | Keyed reconciliation that updates the existing DOM in place instead of replacing `innerHTML`, so focus, scroll, input state and canvas contexts survive an agent update. Modelled on Phoenix LiveView / Idiomorph. |

### Version tokens

| Token | What it governs |
|---|---|
| **v1** | The shipped Agent Drafter shape this design evolves: controls flattened into one English prompt, markdown streamed back into one `<div>`. Every v1 frame, tool and API is preserved — v2 is additive. |
| **v2** | This design: the typed surface, catalog, shared state, platform APIs and profiles below. |
| **v2.1** | Work deliberately deferred out of v2 — see [scope decisions](#scope-decisions). |
| `catalog_version` | Stamped on the `ready` frame and every `ui` frame. Lets an older client ignore newer component kinds instead of erroring ([Pillar 3](#pillar-3-catalog-v2)). |
| `sdk_hash` | The manifest's fingerprint of the vendored `src/sdk.ts` a bundle was built from. Drift triggers a rebuild, which is how a new runtime rolls out to existing apps. |

## What v2 is trying to make possible

BioRouter's strength is integration: 43+ providers (institutional, local, commercial), personal knowledge bases with git history and graph derivation, pluggable MCP extensions, and ~85 skills — all behind one agent loop. Agent Drafter already wires a *real* per-app agent to a generated front-end. But the front-ends it produces are almost all the same shape: controls → `buildPrompt()` → one English string → markdown streamed into one `<div>`.

The exemplar of what we want instead is **BioOKF Studio**: a network-viewer application where the agent *drives* the GUI through a small semantic API (`selectNode`, `search`, `narrate`, `getState`), the user sees an ambient "AI agent is doing X" presence, selection stays synchronized across canvas, panels and citations, and the picture encodes domain meaning (negated edges are red and struck-through; uncurated entities look tentative). Nothing about that app is a chatbot — and nothing about it is expressible in today's Agent Drafter SDK.

**SDK v2's contract:** an app is a typed, three-party contract between the *author-agent* (writes the app once), the *runtime-agent* (drives it live, turn by turn), and the *user* (owns attention and data) — with the BioRouter platform (models, knowledge, extensions, skills) available as first-class, capability-gated APIs on both sides.

## What the research established

The load-bearing conclusions are below. The full reports were produced during the design session and are not part of this repository, so this section is the citable summary; where a conclusion drives a decision, the decision states its own reasoning too.

### Four architectural families for LLM-driven UI

| Family | Examples | Who makes the UI | Strength | Weakness |
|---|---|---|---|---|
| **A. Model writes code, host sandboxes it** | Claude Artifacts, Websim, tldraw make-real | Model, per prompt | Unlimited novelty | Fragile, weak bidirectionality, weak models fail |
| **B. Pre-authored components; model fills slots** | OpenAI Apps SDK, MCP-UI/MCP Apps, Vercel AI SDK | Human author; model emits tool calls | Most robust for weak models; standardized postMessage/intent bridge | Model can't invent UI |
| **C. Model emits declarative UI DSL from a catalog** | Google A2UI, Flutter GenUI, Thesys C1 | Model, per turn, constrained to catalog | Safe by construction; novel compositions | Bounded by catalog |
| **D. App owns UI; agent streamed in via events + shared state** | AG-UI/CopilotKit | App author; agent drives | Strongest bidirectionality (JSON-Patch shared state, HITL) | App must be pre-built for it |

**The "author once, drive live" requirement is a hybrid: A at author time, C+D at runtime, over B's transport discipline.** Agent Drafter v1 already has this skeleton (authored TS bundle + `ui_*` frames + blocking `ui_ask` + rebindable bridge) — the survey validates the shape and tells us exactly which pieces are missing: a real component **catalog** with **flat ID-keyed nodes**, **shared reactive state** (snapshot + RFC-6902 patches + data binding), **typed bidirectional RPC**, **version stamping with unknown-component fallback** (Airbnb's hardest-won lesson), and **morphing instead of innerHTML-replace** (Phoenix LiveView/Idiomorph).

### What BioOKF Studio teaches

1. **A tiny semantic verb surface beats a generic bridge.** The whole agent contract is ~6 domain verbs plus `getState()`/`getGraph()`. The agent speaks the *app's* language, never the DOM's.
2. **Structured state reads beat screenshots.** Tool descriptions actively steer the model to `getState()`; screenshots are demoted to "visual inspection only."
3. **Ambient narration, and observe-don't-hijack.** Agent actions flash a banner; user actions don't. The agent's KB focus shows as a pulsing marker without stealing the user's view.
4. **Selection synchronized across representations** (canvas ↔ detail panel ↔ citations ↔ raw source) is what makes it feel like an application.
5. **Domain-honest visual encoding** (epistemic status rendered, not just topology).
6. Its weaknesses are equally instructive: an `execute_js` eval bridge, a hand-maintained state contract, a world-readable fixed socket, polling, one-way communication, and a fully bespoke renderer that generalizes to nothing. **v2 must deliver these UX patterns through declared, typed, capability-gated primitives instead.**

### Why today's generated apps stay chatbot-shaped

From the deep-read of `agent_drafter/` and ~110 real generated apps:

1. **The zero-effort path is a chat box** — default `main.ts` is `createApp()`; default `index.html` ships a `data-br-chat` card.
2. **Single-prompt, string-only ingress** — authored UI reaches the agent only as one English string (`br.run(buildPrompt(), "#out")`); every app hand-flattens sliders, tabs and chips into prose.
3. **Markdown-first egress** — the agent's answer is markdown streamed into one div; "rich output" is fenced ` ```chart `/` ```graph ` blocks re-parsed into capped SVG (3 chart types; 18-node graphs).
4. **Closed 16-widget enum**, form-shaped (card/table/stat/input/button…), no custom components, no free (sanitized) HTML.
5. **No shared reactive state** — `ui_state` is a flat, agent-owned bag; author code can't write it; nothing binds to it.
6. **Replace-only rendering** — `ui_render`/`ui_panel` do `innerHTML = ""`; no patching, no morphing, focus/scroll/input state destroyed.
7. **No canvas, animation, scene or frame loop** — literally no `canvas`, `requestAnimationFrame`, or WebGL path in the SDK.
8. **No generic event subscription** — the agent cannot observe a node click, slider drag, or selection; only `widget_action` (which arrives as *synthetic prose*: `"[widget:id] The user submitted…"`, and is itself the turn trigger) and author-initiated prompts.
9. **The `output{schema,value}` frame is declared in `sdk.ts` but has no server producer** — structured results can't reach author code; everything is re-parsed from markdown.
10. **Serialized `br.run` chain** — a dragged slider queues N full agent turns; no debounce or supersede.
11. **Theme rigidity** — forced light, one accent, one token set; 5 fixed layout presets.

### Interaction-design principles from the research frontier

- **NL bootstraps; a synthesized persistent control refines** (DynaVis — the single most user-validated GenUI pattern).
- **Externalize agent output as navigable structure** (graph/space/tree), not transcript (Graphologue/Sensecape).
- **Small, independently-owned, reactive units** with visible dependencies keep users oriented when the AI mutates the UI (Marimo; Ink & Switch "tools, not apps").
- **Semantic action channel over pixels** when you own the app (computer-use lesson; MCP-UI intents).
- **Skill library over per-turn improvisation** (Voyager): named, reusable capabilities compound.
- **Townie's engineering lessons:** shape APIs the way LLMs expect; ~dozens of curated examples beat maximalist prompts; error-feedback loops need real infrastructure; **evals before features**.

## Architecture: nine pillars

### The whole system in one diagram

```text
                        ┌────────────────────────────────────────────┐
                        │  BioRouter platform                        │
                        │  providers · knowledge · extensions ·      │
                        │  skills · autovisualiser · workflows       │
                        └───────────────┬────────────────────────────┘
                                        │ per-app agent (session, capabilities)
              ┌─────────────────────────┴──────────────────────────┐
              │ biorouterd  /apps/<id>/agent  (split WS)           │
              │  · agent events (message/thought/tool/output)      │
              │  · ui frames (catalog ops, state patches, ask)     │
              │  · app frames (signals, actions, state writes,     │
              │                ui_reply, typed calls)              │
              └─────────────────────────┬──────────────────────────┘
                                        │ versioned, typed frames
   ┌────────────────────────────────────┴─────────────────────────────────┐
   │ App (authored once by the author-agent, served at /apps/<id>/)      │
   │  index.html + main.ts  ←  SDK v2 runtime (sdk.ts)                    │
   │  · declared: regions, actions, signals, state schema, components    │
   │  · shared state doc (snapshot + JSON-Patch, data-br-bind)           │
   │  · catalog renderer (built-ins + science pack + custom components,  │
   │    morphing, ID-keyed)                                              │
   │  · presence layer (narration chip, highlights, ask modals)          │
   └──────────────────────────────────────────────────────────────────────┘
```

### The nine pillars at a glance

Each pillar is independently shippable; they are ordered by leverage. The last column says where to check what has actually landed — the reference is the authority on shipped behaviour, and this design is the authority on intent.

| Pillar | What it adds | Where shipped state is recorded |
|---|---|---|
| [1 — the app contract](#pillar-1-the-app-contract) | Declared `surface`: state schema, actions, signals, components; `app_call`; typed `br.call` | Reference: the manifest and client API sections |
| [2 — shared reactive state](#pillar-2-shared-reactive-state) | One shared JSON doc, RFC-6902 patches, `data-br-bind` | Reference: `br.state`, `ui_patch_state` |
| [3 — catalog v2](#pillar-3-catalog-v2) | Flat ID-keyed instances, `ui_patch`, morphing, science pack, custom components | Reference: the widget catalog and patch operations |
| [4 — platform encapsulation](#pillar-4-platform-encapsulation) | `br.kb`, `br.model`, model routes, tool-resource bridge, skills scoping | Reference: `br.kb`, `br.model` — note that **skills scoping is still advisory** |
| [5 — the interaction loop](#pillar-5-the-interaction-loop) | `ui_subscribe`, signals, presence, `ui_suggest`, autorun budgets | Reference: `br.signals`, `ui_suggest`, the payload caps table |
| [6 — aesthetics](#pillar-6-aesthetics) | Theme packs, layout grammar, archetype starters | Reference: theme packs; quick start (`archetype`) |
| [7 — security model extensions](#pillar-7-security-model-extensions) | Capability gates for every new power, socket token, CSP, trust boundaries | Reference: the security model section |
| [8 — multi-agent apps](#pillar-8-multi-agent-apps) | Named worker profiles, `br.agent`, `consult` | Reference: `br.agent` — **partial; serialized, not parallel** |
| [9 — app lifecycle and export](#pillar-9-app-lifecycle-and-standalone-export) | Export parity, full payload export, credential onboarding, OS launchers | Reference: the export guide — **the in-SDK re-consent screen is not shipped** |

### Pillar 1: the app contract

Typed surface, in both directions.

**The manifest grows a declared surface** (all optional; absent = v1 behaviour). Naming note: the existing `Capabilities.events` field (`manifest.rs:66-69`) is the *agent-lifecycle* stream flowing **to** the app via `br.on()` (`tool`, `handoff`, `compaction`, …, advertised as `event:<name>` tokens). The new app→agent notifications are therefore named **signals** to avoid colliding with it; the two channels stay independent.

```jsonc
// manifest.json (additions)
{
  "surface": {
    "state_schema": { /* JSON Schema for the shared state document */ },
    "actions": [        // app-defined verbs the AGENT may call
      { "name": "move_avatar", "description": "Move the avatar",
        "params": { "type":"object", "properties": { "direction": {"enum":["up","down","left","right"]}, "steps": {"type":"integer","minimum":1,"maximum":20} } } },
      { "name": "focus_node", "description": "Center and select a graph node", "params": { /* … */ } }
    ],
    "signals": [        // app notifications the agent may SUBSCRIBE to
      { "name": "node_selected", "payload": { /* JSON Schema */ }, "coalesce_ms": 250 }
    ],
    "components": [     // custom catalog components (see Pillar 3)
      { "name": "pathway_map", "props": { /* JSON Schema */ } }
    ]
  }
}
```

Author code registers implementations; the SDK enforces that registrations match declarations at build and lint time:

```ts
// main.ts
const app = createApp({ autoChat: false });
app.actions.register("move_avatar", async ({ direction, steps }) => {
  world.move(direction, steps);            // author's own logic — any JS they wrote
  return { position: world.position };     // typed result returned INTO the tool call
});
app.signals.emit("node_selected", { id, type });   // → agent, if subscribed
```

**Agent side.** `AppControlServer` exposes two new tools generated from the manifest, which means `AppControlServer` must now be constructed with the manifest surface (today it receives only `UiBridge` + `UiCapability`):

- `app_call { action, args }` — validated against the declared schema, forwarded as an `{type:"app_call", callId, action, args}` frame; the author's handler result resolves the tool call (same oneshot parking as `ui_ask`, same timeout discipline). *This is the avatar-control primitive:* "move the avatar up three squares" becomes `app_call{action:"move_avatar", args:{direction:"up", steps:3}}` — no prompt-parsing, no DOM.
- `ui_describe` v2 returns the **full typed surface**: regions, panels, actions (with schemas), signals, components, state schema, state version — merging manifest declarations and runtime registrations with the browser-reported surface it returns today, replacing BioOKF's hand-maintained `getState()` with a generated one.

**App side.** `br.call(name, args, {output_schema?})` lets *author code* invoke agent turns with structured arguments and receive structured results.

> **Why the tool path, not a response schema.** The provider abstraction has no
> response-schema channel today. Structured results are therefore produced via the
> tool path, which already validates schemas everywhere: when `output_schema` is
> supplied, the server injects a synthetic `emit_result` tool whose input schema
> *is* the `output_schema`, instructs the model to finish by calling it, validates
> the call, and emits it to the app as an `output {schema, value}` frame (the frame
> type is already declared at `sdk.ts:74`; Phase 3 adds the missing server
> producer). Prose fallback when the model never calls the tool. Provider-level
> structured output (a `response_format` channel in the `Provider` trait across 43+
> providers) is explicitly **not** required for v2; it can replace the
> synthetic-tool mechanism later without changing the app-facing contract.

`widget_action` frames stop being flattened into synthetic prose — but they remain the **turn trigger** they are today (`routes/apps.rs:1099-1111` feeds them in as the user `Message` that starts the turn). v2 keeps that turn-start semantics and swaps the payload encoding: a minimal user-text envelope carrying the structured JSON block, so existing button-driven apps keep working while the model receives typed data.

`br.run(text, target)` remains — it is the right tool for "explain or summarize into this panel" — but gains `{debounce_ms, supersede:true}` options so slider-driven apps cancel stale turns instead of queueing one model call per pixel (replacing the strict `runChain`).

### Pillar 2: shared reactive state

Snapshot, patch and bindings. Replace the flat, agent-owned `ui_state` bag with a **single shared JSON state document** per app session:

- **Both sides write.** Agent: `ui_state` (kept, now merge-into-doc) and new `ui_patch_state { patch: [RFC-6902 ops] }` (via the `json-patch` crate — a new, small workspace dependency). Author code: `br.state.set(path, value)` / `br.state.update(fn)`, which sends a `state_write` frame (new client→server frame). Writes carry a **version counter**; the server is the ordering authority and rebroadcasts accepted patches to both consumers.
- **Snapshot on (re)connect.** Extends the existing `UiBridge.attach()` replay: the bridge holds the doc and version; a reconnecting page receives one `state.snapshot`, then deltas.
- **Declarative binding, safe by construction.** Authored HTML: `<span data-br-bind="/cohort/count">` plus `data-br-bind-attr` and `data-br-bind-show`. The runtime keeps a pointer→nodes index and re-renders *only bound nodes* on patch — fine-grained reactivity in ~200 lines, no framework. **Rendering contract:** `data-br-bind` writes via `textContent` only (never `innerHTML`); `data-br-bind-attr` uses a strict attribute allowlist that excludes all `on*` handlers and validates URL schemes (`https:`/relative only) for `href`/`src`. State values are agent-writable and therefore prompt-injectable; the binding layer must be a non-executing sink.
- **Persistence.** The doc is persisted with the durable session (keyed `app:<id>:<client_id>`) using the **`ExtensionData` `store_into`/`load` pattern** (as `RunState` does — there is no general per-session document column, and we don't add one; a schema migration would require human review per `HOWTOAI.md`). Restore seeds the `UiBridge` doc *before* `attach()` replays the snapshot; the 256 KB cap is enforced before serialization.
- **Caps without schema.** `surface.state_schema` is validated server-side when present; when absent, default structural caps still apply (max depth 8, max 2,000 keys, string-value length caps) so an unschema'd app cannot become an unbounded injection or DoS path. Lint requires a `state_schema` when bindings are used.

This one pillar dissolves friction items 5, 6 (partially), and 11's worst effects: `omics-dashboard`'s tabs, chips and sliders become state paths the agent can also read and patch, instead of ad-hoc DOM classes invisible to it.

### Pillar 3: catalog v2

Flat, ID-keyed, morphing, extensible.

**Representation change:** agent-driven UI becomes a **flat list of ID-keyed component instances** (A2UI's core lesson — LLMs patch flat lists far more reliably than they regenerate nested trees):

- `ui_render` / `ui_panel` keep working (compat), but internally normalize to catalog instances with stable IDs.
- New `ui_patch { ops: [{op:"add"|"replace"|"remove"|"set_props", id, parent?, node?/props?}] }` — incremental edits to individual components.
- The renderer **morphs** (Idiomorph-style keyed reconciliation) instead of `innerHTML=""` — focus, scroll, input state, and canvas contexts survive agent updates.
- **Version stamping and fallback:** every frame carries `catalog_version`; unknown `cmd`s are ignored (already true) and unknown component kinds render a neutral labelled placeholder instead of "unsupported widget" errors (Airbnb forward-compatibility).

**The science pack** — new built-in kinds that make biomedical apps first-class:

| Kind | What it is |
|---|---|
| `network` | The BioOKF force-graph engine, generalized: typed spec `{nodes:[{id,label,type,size?}], edges:[{source,target,kind,style?}], encoding:{type_colors?, families?, negated_kinds?}, physics?}`; zoom/pan/drag/hover/select built in; selection emits a declarable signal. Canvas-based, Barnes-Hut, viewport culling — proven at KB scale in Studio. |
| `plot` | Real interactive charts beyond bar/line/pie (scatter, area, box, heatmap axes), themed, with a `bind` prop for live data. |
| `figure` | An Auto Visualiser fragment embedded in a sandboxed iframe. **Prerequisite:** `autovisualiser::common` is a *private* module today; this needs a small public API on the autovisualiser side (e.g. `pub(crate) fn render_named_figure(tool, args) -> …` wrapping `render_fragment` + the `ASSET_SINK` task-local), plus the asset-splicing story — real wiring work, same crate so no circular dependency. Once wired, all 34 `render_*` tools (volcano, Manhattan, Kaplan-Meier, Sankey, chord, maps, Mermaid…) become available *inside apps*. |
| `table` (v2) | Virtualized (10k+ rows), sortable, filterable, selectable rows → signal. |
| `canvas` | An author-registered draw surface: the author supplies a render function; the agent supplies *data* via props or state. Gives frame loops, simulations, and avatars a sanctioned home without letting the agent write code at runtime. |
| `markdown`, `image`, `kpi`, `log` | Quality-of-life kinds apps keep hand-rolling. |
| `html` | Sanitized rich HTML, **capability-gated** (`ui.allow_html`, default off). Sanitization is **server-side in `control.rs`, fail-closed** (the frame never leaves the daemon unsanitized), with a pinned config: no `<script>`/`<style>`/`<form>`, no `on*` attributes, no `javascript:`/`data:` URLs, SVG/MathML mXSS guards — and a known-bypass regression corpus as an acceptance criterion. With this node enabled, the sanitizer is a primary XSS barrier and must be treated accordingly (see the CSP note in [Pillar 7](#pillar-7-security-model-extensions)). |

**Custom components — the big unlock.** Authors register their own catalog entries:

```ts
app.components.register("pathway_map", {
  props: PathwayMapSchema,        // must match the manifest declaration
  mount(el, props, ctx) { /* author-written renderer */ },
  update(el, props, prev) { /* optional; else re-mount */ },
});
```

Declared schemas are extracted at build time into the manifest, so `control.rs` **validates agent-emitted instances server-side** exactly like built-ins. Extraction **fails closed**: a registration whose schema can't be statically extracted (dynamic or spread props) is a build error, never an accept-any schema. Authors must treat `props` as **untrusted, agent-controlled input** — lint flags `innerHTML` and URL sinks fed from props. The agent then composes `pathway_map` like any other kind. Catalog = built-ins ∪ app-specific components: Family C's safety with Family A's authorial freedom, which is precisely the hybrid the survey recommends.

### Pillar 4: platform encapsulation

The whole of BioRouter behind `br.*`. This is the north star: apps as first-class consumers of everything BioRouter integrates. All of it capability-gated (deny-by-default except core `ui`), all resolved server-side so secrets and keys never enter the page.

- **`br.kb` — knowledge bases.** `search(query)` (BM25), `page(path)`, `graph()` (nodes and edges — feeds straight into the `network` component: *a BioOKF-Studio-class KB explorer becomes a ~50-line app*), `ingest(items) → streamed progress`, `history()`. **Scoped grants:** the `data.sources[kind:"knowledge"]` capability enumerates the specific KB id(s) the app may touch — never "all bases" (default: none). `ingest` requires `write:true`, which is a *separately and prominently consented* grant: a poisoned ingest persists in a git-backed KB that other sessions and agents read, so write access is a cross-session integrity decision, not a checkbox.
- **`br.model` — provider routing.** Extends the existing `list()`/`select()` with model **status** (is llamacpp downloading? context size?), and manifest-level `agent.routes` — named model profiles (`"fast"`, `"deep"`, `"local_only"`) that `br.call`/`br.run` can select per invocation. Routes must resolve to providers the *user* has configured; apps never carry keys. **Provider-class constraint:** an app holding a sensitive data source (`omop`, `cdw`, or a confidential KB) is restricted to an allow-listed provider class (local or institutional) and cannot route that data to an external commercial provider without an explicit, per-app user consent — provider class is a capability, not a post-hoc UI label. Which model answered is additionally surfaced in the UI.
- **Extensions and MCP.** Already injected per-app; v2 adds structured tool progress: `tool` frames gain `args_summary`, and tool results carrying `ui://` embedded resources are emitted as catalog `figure` instances (targeted at the app's declared results region, author-overridable) instead of being dropped.
- **Skills.** The per-app `skills` list becomes *enforced* scoping (today advisory), and the authoring instructions teach the author-agent to pick skills the way it picks extensions.
- **Workflows and schedules — deferred to v2.1** (see [scope decisions](#scope-decisions)): `br.workflow.run(name, args)` plus manifest-declared cron refresh ride the existing scheduler once the core is proven.
- **Vault** stays as-is (`{{vault:NAME}}`), already correct.

### Pillar 5: the interaction loop

Signals, presence and mixed initiative.

- **`ui_subscribe { signals: ["node_selected", …] }`** — the agent opts into declared app signals. Delivery is coalesced or debounced per declaration (`coalesce_ms`), rate-capped server-side, queued through the existing between-turns/mid-turn frame machinery (bounded by the same `MAX_QUEUED_FRAMES` discipline — the per-connection cap on buffered frames), and presented to the model as structured JSON.
- **Untrusted-data envelope.** Every app→agent payload (signal payloads, `app_result` values, `br.call` outputs, `widget_action` data) is per-field size-capped and delivered inside an explicit envelope the system prompt marks as *data, not instructions*. Apps render untrusted content (KB pages, pasted documents, web results), so these payloads are indirect-prompt-injection carriers by construction; the mitigation is capability minimization (scoped KB grants, deny-by-default writes) plus the envelope — never input trust. Default delivery is **queue-only**: signals are context for the next turn, they do not start turns.
- **`autorun` — off by default, a real capability.** A declared signal may additionally be allowed to *start* a turn only when (a) the app declares it, (b) the **user** grants the `ui.allow_autorun` capability (the agent cannot self-grant), and (c) budgets hold: per-minute cap, per-session turn budget, and a daily cap — autonomous turns spend the user's provider quota, which on commercial and institutional providers is real money. Autorun activity renders in the presence layer with a one-click stop.
- **Presence layer (BioOKF's banner, generalized).** The SDK renders an ambient agent-activity chip for every applied `ui_*` frame ("AI · updating cohort table ⋯"), distinguishes agent-driven from user-driven changes, and `ui_highlight` gains a `narrate` note. Observe-don't-hijack: agent updates *mark* rather than steal focus (no auto-scroll unless `scroll:true`).
- **Mixed initiative:** `ui_ask` stays the blocking primitive; new non-blocking `ui_suggest { chips: […] }` renders dismissible suggestion chips (Horvitz: easy to invoke, easy to ignore).
- **The DynaVis rule, in the authoring and runtime instructions:** after fulfilling an NL request that changed a parameter, the agent should emit a *persistent bound control* for it (`ui_patch` adding a slider bound to `/plot/km`), so users refine by direct manipulation instead of re-prompting.

### Pillar 6: aesthetics

Themes, layout grammar and archetype starters.

- **Theme packs.** Manifest `theme` becomes a token set (palette including dark, font stack and scale, radius, density, surface treatment) with ~6 curated presets (`clinical`, `lab-notebook`, `terminal`, `glass`, `journal`, `midnight`) plus custom tokens. Lint keeps enforcing contrast and token usage (the existing rules generalize from "the one palette" to "the active pack"). `ui_theme` can switch packs if allowed. This ends "every app is the BioRouter light theme."
- **Layout grammar.** `ui_layout { areas, sizes }` → validated grid template (bounded vocabulary, no raw CSS from the agent). **The 5 existing presets are retained as aliases** over the grammar so no v1 app that calls `ui_layout{preset}` breaks. Docks and `@region:` targets keep working.
- **Archetype starters.** The single chat-card template is why the median app is a chatbot. Replace it with a starter gallery the author-agent (or `create_app{archetype}`) selects: `explorer` (network/canvas + inspector + search), `dashboard` (bound KPI grid + panels), `workbench` (data table + actions + detail), `wizard` (staged form), `canvas` (scene + controls — the avatar archetype), `chat` (today's default, now one option among six). Each starter ships wired state paths, one registered action, and one subscribed signal — teaching by example (Townie: curated examples beat prompt maximalism).
- The **frontend-design skill** gets referenced from the authoring instructions for visual originality within the token system.
- **Applications tab:** the desktop `ApplicationsView` gets a light audit to surface the new diversity — archetype badge, theme-pack swatch, and a launch affordance that opens non-chat apps into their real UI.

### Pillar 7: security model extensions

Every new power maps onto the existing capability lattice (deny-by-default except core `ui`):

| New surface | Gate / cap |
|---|---|
| `html` component (server-side sanitized, fail-closed) | `ui.allow_html` (default **off**); pinned sanitizer config + bypass regression corpus |
| Custom components / `canvas` | `ui.allow_components` (default **on** — author code already runs on the page; registration adds no new *execution* authority; props are validated server-side and documented as untrusted) |
| App signals → agent | `ui.allow_signals` + per-signal declaration; coalescing + server-side rate caps |
| `autorun` (signal-triggered turns) | `ui.allow_autorun` (default **off**, user-granted only) + per-minute/per-session/daily budgets |
| `app_call` actions | declared-schema validation, per-call timeout, `max_concurrent:1` |
| `app_result` / `br.call` output | per-payload size cap (64 KB, truncation marker) — these enter model context |
| `br.kb.*` | `data.sources[kind:knowledge]` **scoped to enumerated KB ids** (default none); `ingest` requires separately-consented `write:true` |
| Model routes | user-configured providers only; **provider-class capability** for sensitive data sources ([Pillar 4](#pillar-4-platform-encapsulation)) |
| State doc | 256 KB cap, 64 ops/patch, patch-rate limit, schema validation when declared, default structural caps otherwise; bindings render via `textContent` + attribute allowlist only |

> **WebSocket authority is a Phase-1 requirement, not late hardening.** In v1 the
> app socket only drove the app's own DOM; in v2 it carries `app_call`, `br.kb`,
> model routes, and state writes — so the current gate (`is_local_origin`, which
> accepts *any* `http://localhost:*` page, secret-exempt by design at
> `apps.rs:206-216`) is no longer sufficient: any local web content could open
> `/apps/<id>/agent` and drive a capability-bearing agent (cross-site WebSocket
> hijacking). v2 requires (a) exact-origin pinning (scheme + host + port of the
> app's own served origin) and (b) a per-app socket token minted into the served
> page (readable same-origin only) and required on upgrade.

**CSP (corrected).** `'unsafe-inline'` in `script-src` would make CSP inert against exactly the injection classes v2 introduces (`html` node output, binding sinks) — and apps load their code externally (`dist/app.js`), so served apps get the strict policy: `script-src 'self'`; the injected `BIOROUTER_APP_CONFIG` inline script becomes a non-executable `<script type="application/json">` block the SDK parses; plus `connect-src 'self'` (blocks exfiltration), `img-src 'self' data:`, `form-action 'none'`, `base-uri 'self'`, `frame-ancestors 'self'`. Lint already forbids external scripts, so app authors are unaffected.

**Trust boundaries, stated plainly:**

1. App→agent payloads are untrusted (indirect prompt injection) — enveloped, capped, and never a substitute for capability minimization.
2. Agent→app content is untrusted too (a prompt-injected agent) — hence `textContent` bindings, server-side sanitization, catalog validation.
3. **An imported or shared app is an untrusted author: its manifest is a capability *request*, not a grant.** The recipient re-consents on first run (deny-by-default), especially for KB access, `write:true`, model routes, and autorun; the same consent screen enumerates the server-side payload the export wants to install (KBs, skills, extensions — [Pillar 9](#pillar-9-app-lifecycle-and-standalone-export)) before anything touches the recipient's store. Exported apps get the same CSP and serve invariants via `serve.mjs` parity.
4. Apps are **single-user, per-client session-scoped** (state keyed by `client_id`); collaborative multi-viewer apps are out of scope for v2.

Unchanged and reaffirmed: vault plaintext never in frames; path-jailed stores; `.vault/` excluded from export; mutating HTTP requires the secret; structured validation of every agent-emitted frame stays server-side in `control.rs` (weak local models get correction messages, not blank panels).

### Pillar 8: multi-agent apps

Named profiles, delegation, and collaborative or adversarial patterns.

One agent per app is the v1 shape, but multi-agent is already half-present: `orchestration.sub_agents` is **wired today** — declared sub-agents are materialized as engine recipes (`apps.rs:708-737`, `materialize_subagent_recipe`) and exposed to the primary agent as **agents-as-tools** via the core subagent tool (`crates/biorouter/src/agents/subagent_tool.rs`, with its own concurrency cap), and the SDK already renders `handoff{from,to}` frames in the timeline. What is missing is everything *outside* that façade: the app can't address a specific agent, can't run two agents in parallel, can't give a panel its own agent. v2 adds **named agent profiles**:

- **Manifest:** the dormant `orchestration.agents: HashMap<String, AgentConfig>` map (already in `manifest.rs`) becomes the vehicle. The existing `agent` block is the `main` profile; each additional profile carries its own system prompt, model or route, extensions, skills, and KB — and a capability set that must be a **subset** of the app's grants.
- **Sessions and transport:** each profile gets its own session (keyed `app:<id>:<client_id>:<profile>`) and turn loop; frames are multiplexed over the *same* WebSocket with an optional `agent` field (omitted = `main`), so reconnect and replay semantics are unchanged.
- **App side:** `br.agent("critic")` returns a scoped facade (`call`/`run`/`prompt`/`on`). Turns on *different* profiles run in parallel (bounded: default max 3 concurrent per app); turns on the same profile stay serialized. This is what lets a dashboard refresh three panels through three worker profiles concurrently, or a "Debate" button fan the same question out to two differently-prompted profiles and render both answers side by side — **author-orchestrated collaboration**, with no new protocol concepts.
- **Agent side:** alongside the shipped sub-agents-as-tools path, `main` gets a `consult { agent, prompt }` tool to invoke a named profile mid-turn — **agent-orchestrated collaboration**. The canonical adversarial pattern (generator produces, skeptic refutes, only survivors render) becomes: `main` drafts → `consult{agent:"critic"}` → revise → `ui_patch`.
- **UI authority and presence:** only `main` holds `ui_*`/appcontrol by default; a worker profile gets UI control only if its profile says `ui:true`, and its panels and presence chips are attributed ("Critic · reviewing evidence ⋯") so the user always knows *which* agent is acting. Signal and autorun budgets are per-app, not per-profile (no budget multiplication).
- **Patterns unlocked:** adversarial review (generator + critic), panel-of-judges scoring, and pipeline stages (extract → analyze → visualize) each owned by a profile tuned to its task — including **different models per role** (a local model triaging, an institutional model touching PHI, a frontier model writing the synthesis), which is exactly BioRouter's provider-integration strength applied inside one app.
- **Boundaries:** profiles live in-process in `biorouterd` (the `biorouter-acp` protocol remains the layer for *cross-process* agent orchestration, and `br.workflow.run` in v2.1 the layer for declarative DAGs). Two simultaneous turns on the *same* profile remain out of scope.

> **Shipped state.** The reference records this pillar as partial and actively
> landing: cross-profile turns are **serialized, not parallel**, and `consult` depth
> is 1. The parallel-turn design above is intent, not current behaviour.

### Pillar 9: app lifecycle and standalone export

The Applications-panel round-trip plus standalone export. The full lifecycle is a product guarantee, not an implementation detail: **create → appears in the Applications panel → reopen and change it anytime → export as standalone software that carries its full server-side payload and runs without opening BioRouter, on any OS.**

**What already ships (v1) and is retained:** the Applications panel lists every app with launch and delete plus a working one-click **Export** (`ApplicationsView.tsx:114-142` → `GET /apps/{id}/export`, which rebuilds a stale bundle first via `export_scaffold`). The exported folder is directly runnable: `run.command` (macOS) / `run.sh` source `biorouter-launch.sh`, which locates or installs `biorouterd`, self-installs the app into the recipient's store, starts the daemon **headlessly**, verifies it, and opens the default browser — the BioRouter GUI never opens. A `serve.mjs` loopback proxy (static files plus `/apps/**` including the WS upgrade) covers the no-shell path. `.vault/` is excluded; the SDK derives its endpoint from the page origin.

**"Standalone" defined honestly:** the app's intelligence *is* the BioRouter platform — providers, KB, extensions and skills live in `biorouterd`. So standalone means *no BioRouter application (GUI) and no visible BioRouter anything*: a double-clickable folder whose scripts run the daemon as an invisible backend. A generated app with no daemon would have no agent; that trade is inherent and stated.

**v2 additions:**

1. **Export parity is a phase-gate invariant.** Every pillar's features must work identically in the exported form — the strict CSP, the per-app socket token (minted by `serve.mjs` or the launch path), durable state restore (the recipient's session store), multi-agent profiles, `figure` fragments, theme packs. The rule: *if it works in the Applications panel, it works exported.* Each plan phase's acceptance includes the export smoke, not just Phase 6.

2. **Full server-side payload travels with the app — or just a launcher: the user chooses.** An exported app is only as good as the platform pieces its agent depends on, so the export can carry them. The panel's Export becomes a small dialog (and `export_app` gains `mode` + `include` params) offering two modes:
   - **Launcher export** (today's thin form, kept as a first-class choice): app plus launch scripts only — the smallest folder; the app runs against whatever knowledge bases, skills, extensions and providers already exist on the target machine. Right for self-use, same-machine moves, and lab machines that share a configured BioRouter install.
   - **Full export**: the payload bundling below, with **per-item toggles, pre-checked from what the app's agent config actually references** — the user decides item by item what travels:
     - **Knowledge bases** — each granted KB is staged as a `.brkb` bundle (the existing knowledge export format) inside `payload/knowledge/`; raw sources optionally excluded to control size (the dialog shows a size estimate per item). On the recipient's machine the first-run installer imports it into their store under the same KB id, satisfying the app's scoped grant.
     - **Skills** — the app's skill list is staged under `payload/skills/` (the same zip format the marketplace uses) and installed into the recipient's skills dir on first run.
     - **Extensions** — *builtin* extensions (developer, autovisualiser, knowledge, …) travel with the daemon and need nothing. *External* extensions are staged as `.brxt` bundles under `payload/extensions/` when installed locally, or recorded as **pinned, checksummed registry references** (BAAM) the installer fetches on first run — the dialog says which. Runtime prerequisites an extension declares (e.g. Node for PlaywrightAgent) are recorded and checked at first run with a clear remedy message.
     - Everything is enumerated in a **payload manifest** (`export.json`: items, versions, checksums, required credentials, runtime requirements) so the installer is deterministic and the recipient can audit exactly what a shared app wants to install.

3. **Credentials: never bundled, always onboarded.** The export carries no secrets of any kind (vault excluded; provider keys and extension credentials live in the OS credential store). Instead, `export.json` lists the **credential requirements** — the env keys the app's extensions declare (e.g. `SPOKEAGENT_PASSCODE`) plus "a configured provider" — and on first launch the SDK shows a **setup dialog**: which credentials are missing, a field for each, stored via the daemon into the recipient's OS credential store (existing keyring path), then the agent starts. Re-launches skip whatever is already satisfied. A machine with no provider configured gets the same dialog with a guided provider-setup step rather than a dead app. KB grants referencing a base the user chose *not* to bundle degrade gracefully via `has()`.

4. **OS-agnostic by format.** The app itself is a **web application** — HTML/TS in any modern browser — so the UI is OS-agnostic by construction; what differs per OS is only the backend daemon. The export ships launchers for all three platforms (`run.command` for macOS, `run.sh` for Linux, `run.ps1`/`run.bat` for Windows); in **thin** mode each launcher auto-installs the platform-matching `biorouterd` from the pinned GitHub release; in **fat** mode the dialog chooses `current platform` (default, ~108 MB) or `universal` (all platforms bundled, larger, runs anywhere). One exported folder therefore runs on macOS, Linux and Windows. The macOS quarantine caveat is stated in the README (right-click-Open on first run; per-app notarized packaging is v2.1). A *hosted* variant — one daemon serving the app as a plain URL to colleagues' browsers — is the fully-zero-install endgame and lands in v2.1 with the remote-auth work it requires (today's auth model is loopback-only).

5. **First-run experience is one flow.** Launch → daemon up → **single consent screen** combining the [Pillar 7](#pillar-7-security-model-extensions) capability re-consent with the payload install list ("this app will install KB *ms-cohort*, skill *ggplot-visualization*, extension *SPOKEAgent*, and needs 1 credential") → payload installs → credential dialog (item 3 above) → app opens. A same-machine self-export skips consent for grants and payload already present.

6. **Editability round-trip.** The exported folder remains a readable TS project (the [human-facing SDK reference](sdk-reference.md) exists precisely for this); re-importing an edited export re-runs lint and build, and `sdk_hash` drift triggers the rebuild path as usual.

Out of scope, consistent with [scope decisions](#scope-decisions): per-app desktop packaging (a `.dmg`/Tauri wrapper per app), the hosted/URL-sharing variant, and BAAM marketplace listing — all v2.1, gated on the import-re-consent model.

## Worked examples

What becomes possible once the pillars land.

### Example A: knowledge-graph explorer

BioOKF-Studio-class, in roughly 50 lines of authored code.

- The `explorer` starter plus `br.kb.graph()` feeds a `network` component.
- The agent subscribes to the `node_selected` signal; on selection it `br.kb.page()`s the node and `ui_patch`es the inspector panel with a `markdown` component plus a `figure` (Kaplan-Meier from Auto Visualiser) when relevant.
- The user asks "focus on demyelination" → the agent calls `app_call{focus_node}`.
- The presence chip narrates each step.

Every piece is a declared, typed, gated primitive — no eval bridge, no bespoke renderer, no polling.

### Example B: avatar and scene control

"Move the avatar up."

- The `canvas` starter: the author registers a `canvas` component with a `world` model in shared state (`/avatar/position`), plus the actions `move_avatar` and `speak`.
- The user types "walk to the door and greet"; the agent plans and emits `app_call{move_avatar,…}` then `app_call{speak,…}`.
- State patches animate the canvas; `collision` signals flow back if subscribed.

The agent never writes runtime code — it drives declared verbs, exactly like BioOKF's `selectNode` but generated from the manifest. (Voice input is not an SDK primitive in v2; text NL covers the ask.)

### Example C: cohort dashboard on institutional data

- The `dashboard` starter plus `data.sources[omop]` plus the `institutional` model route, provider-class-constrained per [Pillar 4](#pillar-4-platform-encapsulation).
- A KPI grid bound to `/cohort/*` state paths; the agent refreshes via SQL tools and `ui_patch_state`.
- A dragged age-slider fires a debounced, superseding `br.call("refresh_cohort", {age_range})` with `output_schema`, so results land as data in bound components — zero markdown re-parsing.

### Example D: adversarial evidence review

Multi-agent.

- Two profiles: `reviewer` (institutional model, `br.kb` read on the lab's KB) and `skeptic` (system prompt: "refute every claim; demand provenance").
- The user drops in a manuscript claim; author code fans out `br.agent("reviewer").call(...)` and, on its result, `br.agent("skeptic").call(...)`.
- Surviving evidence renders into a `table` with per-row provenance; refuted items go into a struck-through `log` panel — the BioOKF "epistemic status is visible" principle, produced by an adversarial pair.
- Alternatively the whole loop runs agent-side: `main` drafts and `consult{agent:"skeptic"}`s before ever touching the UI.

## Compatibility and migration

- **Old apps keep working untouched.** Every v1 frame, tool and API is preserved; v2 is additive. The `ready` frame advertises `catalog_version` plus capability tokens; `has()` feature-detects.
- **`refresh_sdk` + `sdk_hash`** (already built) roll the new runtime to existing apps: lazily on `serve_index` drift detection, plus an explicit `biorouter`-side batch rebuild step for the whole store. Acceptance: all stored apps rebuild on the new SDK and `check-all` holds its v1 pass count. The app corpus plus the `round*` authoring scripts get vendored and pinned into this repo first (today `check-all.mjs` references an external worktree path), so the regression gate is reproducible.
- **Unknown-frame tolerance** means a stale bundle simply ignores v2 frames until rebuilt.
- **Lint v2** adds rules for the new surface (declared-vs-registered action mismatch, bind paths not in the state schema, a signal without `coalesce_ms`, a custom component without a schema, prop-fed `innerHTML` sinks) so the author-agent self-corrects — the mechanism that made even vague prompts yield working apps in v1.
- The **authoring instructions in `mod.rs` are rewritten around archetypes** with one curated exemplar each, and the "VARY THE INTERFACE" plea becomes structural (starters) rather than rhetorical.
- **A human-facing SDK reference ships alongside** the author-agent prompts: the `br.*` API surface, the `manifest.surface` JSON Schema, the frame and protocol reference, the capability matrix, the custom-component guide, and the three worked examples as annotated source — `export_app` produces a project humans edit, so the SDK needs human docs. That reference now exists: [sdk-reference.md](sdk-reference.md).

## Testing and evals

Evals before features (Townie).

- **Unit:** state-doc patch/version/rebroadcast in `control.rs`; catalog validation including custom schemas (mismatch rejected); the morph renderer (focus and scroll survival) in an SDK harness; `app_call` parking/timeout/cancel alongside the existing `ui_ask` tests; the sanitizer bypass corpus.
- **Integration:** extend `ui-control-harness.mjs` (mock daemon, real SDK) with state binding, `ui_patch`, signals, and `app_call`; extend `check-ui-app.mjs` to assert v2 frames arrive against a real agent.
- **The UI-variety benchmark becomes the eval.** `round2`/`round3` plus `benchmark.mjs` already measure real-controls-per-app on raw store markup. v2 targets: vague prompts ≥ 80% non-chat archetypes; detailed prompts ≥ 2 bound state paths, ≥ 1 declared action, ≥ 1 non-markdown component per app — measured the same honest way, against a pinned v1 baseline.
- **Error-feedback loop:** runtime errors in catalog rendering post a structured `ui_error` frame the agent sees (bounded by the same live-turn grace discipline as artifact auto-repair), closing the self-correction loop in-app.

## Scope decisions

- **Distribution:** standalone export is **first-class in v2** ([Pillar 9](#pillar-9-app-lifecycle-and-standalone-export) — panel export dialog with full server-side payload bundling (KBs, skills, extensions), credential onboarding, runnable without opening BioRouter on macOS/Linux/Windows, optional bundled daemon). What stays v2.1: the `.brapp` bundle plus one-click install mirroring `.brxt`, and BAAM listing — **gated on the import-re-consent model** in [Pillar 7](#pillar-7-security-model-extensions) (a shared manifest is a request, not a grant).
- **CLI:** apps are browser-rendered by design; the CLI gets `biorouter apps list|open|serve` parity (launch daemon, print or open the URL — matching how `launch_app` already behaves in CLI contexts). In-terminal (ratatui) rendering of catalog UIs is out of scope.
- **A Tauri or desktop shell for apps** — apps stay browser-served by `biorouterd` (and inside the Electron Applications tab). Studio's vibrancy, PTY and Finder affordances are not portable SDK primitives.
- **CRDTs** — a single ordering authority (the server) plus JSON Patch suffices; apps are single-user per-client ([Pillar 7](#pillar-7-security-model-extensions)). Revisit only if collaborative editing becomes a requirement.
- **Multi-agent apps are IN scope** ([Pillar 8](#pillar-8-multi-agent-apps)): named agent profiles with parallel turns *across* profiles, `consult` for agent-orchestrated collaboration, and the already-shipped sub-agents-as-tools path. Still out: two simultaneous turns on the *same* profile, and cross-process orchestration (that's `biorouter-acp`'s layer).
- **Voice input** — not an SDK primitive in v2.
- **Workflows and scheduled refresh** — v2.1 ([Pillar 4](#pillar-4-platform-encapsulation)).
- **Provider-level structured output** (a `response_format` channel in the `Provider` trait) — not required; the `emit_result` tool mechanism covers v2 ([Pillar 1](#pillar-1-the-app-contract)) and can be swapped later.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Weak local models can't drive the bigger surface | Everything remains small typed frames (Family B/C discipline); server-side validation returns *fixable* errors; archetype starters carry the structure so the runtime-agent mostly fills slots |
| Scope: seven pillars is a platform | Pillars are independently shippable; the plan phases them; Pillars 1–3 alone dissolve the chatbot ceiling; scope decisions trim v2.1 items explicitly |
| Custom components reintroduce Family-A fragility at runtime | They execute *author* code written once at build time; the agent only supplies schema-validated props (extraction fails closed); authors are linted against prop-fed sinks |
| Indirect prompt injection via app payloads | Untrusted-data envelope + per-field caps + scoped capabilities (Pillars 5 and 7); acknowledged as residual risk, mitigated by minimization not trust |
| Signal floods burn tokens | Declaration-level `coalesce_ms`, server-side rate caps, queue-only default, autorun off by default with budgets |
| Multi-agent profiles multiply cost and complexity | Per-app concurrent-turn cap (default 3), per-app (not per-profile) signal/autorun budgets, profile capabilities ⊆ app capabilities, presence attribution per agent; sub-agents-as-tools already ships and stays the low-cost default |
| Full-payload exports get large or stale | Per-item size estimates + toggles in the export dialog (raw KB sources optional); pinned versions + checksums in `export.json`; registry-reference mode fetches instead of bundling; universal daemon bundling is opt-in |
| Two renderers drift (markdown fences vs catalog) | Fenced ` ```chart `/` ```graph ` become sugar that lowers to catalog instances internally; one renderer |

## Related documentation

- [Apps SDK v2 reference](sdk-reference.md) — what actually ships today, and the authority whenever this design and the code disagree.
- [Apps SDK v2 phase roadmap](v2-phase-roadmap.md) — the six-phase execution plan derived from these pillars, with per-phase acceptance gates.
- [BioRouter Apps platform design](../agent-drafter/apps-platform-design.md) — the Agent Drafter subsystem overview this design extends.
- [Agent Drafter SDK strategy RFC (2026-06)](../history/apps-sdk-rfc-2026-06/strategy-and-openai-comparison.md) — the earlier RFC comparing OpenAI's Agents SDK, which framed the layered-SDK direction.
- [Agent Drafter SDK implementation design (2026-06)](../history/apps-sdk-rfc-2026-06/implementation-design.md) — the code-level companion to that RFC, with the first protocol-v2 sketch.
