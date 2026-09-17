# BioRouter Apps platform design

> **What this is.** The design overview of the Agent Drafter subsystem — what a
> BioRouter app is, how the Apps SDK v2 surface is organised, and how the protocol,
> capabilities, themes, export modes and multi-agent orchestration fit together.
> It also preserves, in a clearly marked second half, the v1 redesign notes and the
> app-building campaign logs that produced the current design.
> **Status:** Current. The first half matches the shipped code (`control.rs`,
> `manifest.rs`, `apps.rs`, six theme packs, six archetype starters); the second half
> is a deliberately retained historical record, labelled as such.
> **Audience:** developers working on Agent Drafter / BioRouter Apps.
> **Shorthand.** *MiMo* is the Xiaomi MiMo provider (see
> [Xiaomi MiMo provider notes](../providers/xiaomi-mimo.md)); *round2* / *round3* name
> batch-authoring runs driven by `scripts/agent-drafter-apps/round2.sh` and
> `round3.sh`; `@region:<name>` is an agent render target naming an author-declared
> `<section data-br-region="…">`; *Pillar N* refers to the nine SDK v2 pillars
> enumerated below.

Agent Drafter was reworked from "Claude-style artifacts" into a builder for
**BioRouter apps**: TypeScript front-ends wired to a *real* BioRouter agent
backend. When a user sends a message in a built app, BioRouter runs the full
agent loop — the app's own model, extensions, skills, and knowledge base (KB) — and
streams the answer (text / markdown / tool activity) back into the app. Apps are
*launched in the browser* (the desktop GUI auto-opens the default browser; the CLI
prints a URL), not embedded in a chat iframe.

Apps SDK v2 (design: [Apps SDK v2 design](../apps-sdk/v2-design.md)) turns the app
from "a chat box the agent answers in" into a **typed, two-way surface the agent
drives**. The full developer reference — every `br.*` signature, the manifest schema,
the widget catalog, the frame tables, and the export format — lives in the
[Apps SDK reference](../apps-sdk/sdk-reference.md). This document is the map; that
file is the territory.

## The nine pillars

v2 is nine independently-shippable pillars. Pillars 1–5 rebuild the interaction
core: **(1) the App Contract** — the manifest grows a declared `surface`
(`state_schema`, `actions` the agent calls via `app_call`, `signals` the agent
subscribes to, custom `components`), enforced server-side and lint-checked against
the code that registers it; **(2) shared reactive state** — one JSON document per
session that both sides write, as a snapshot-plus-RFC-6902-patch stream, with
declarative `data-br-bind` / `-bind-attr` / `-bind-show` bindings that re-render
only changed nodes; **(3) Catalog v2** — a flat, id-keyed, morphing widget set
(`ui_patch` edits nodes by id, preserving focus/scroll) plus a science pack
(`network`, `plot`, `figure`, `kpi`, `log`, `markdown`, `image`) and
author-registered custom components; **(4) platform encapsulation** — knowledge
bases and provider routing behind `br.kb` / `br.model`, resolved server-side so
keys never touch the page; **(5) the interaction loop** — coalesced app→agent
signals, an ambient presence chip that narrates every agent UI change, and
`ui_ask` for blocking mid-turn questions (the DynaVis rule: after a natural-language
request tunes a knob, synthesize a persistent bound control for it).

Pillars 6–9 cover surface and lifecycle: **(6) aesthetics** — six curated theme
packs, a bounded `ui_layout` grid grammar, and six archetype starters so a fresh
app is a working example of its shape, not a chatbot; **(7) security extensions** —
a per-app WebSocket token + origin pin, the fail-closed `ui_html` sanitizer,
textContent-only bindings, KB grants scoped to enumerated ids, a provider-class
rule (sensitive data can't route to an external commercial provider), and a table
of payload caps; **(8) multi-agent** — both the sub-agents-as-tools path
(`orchestration.sub_agents`) and named worker *profiles* (`orchestration.agents`
→ `br.agent(name)` + the agent-side `consult` tool, advertised in `ready.profiles`)
ship, with the caveat that cross-profile turns are **serialized** (parallel across
profiles is a stretch goal); **(9) lifecycle** — the Applications-panel round-trip
and a standalone export that can carry the app's server-side payload.

## Protocol v2 overview

The app talks to `biorouterd` over one WebSocket (`GET /apps/<id>/agent`). The
server opens with a **`ready` frame** (`protocol: 2`) advertising capability
tokens (`manifest.agent.capabilities.advertised()`), the catalog + state versions,
and the declared `surface` (signals with coalesce windows, action names). The
client feature-detects with `br.has(token)`. Frames are **versioned**: every
server-issued `ui` command frame carries `type:"ui"` + `v` (`CATALOG_VERSION`,
currently 1). **Fallback is forgiving** — an unknown `ui` `cmd` is ignored, an
unknown widget kind renders a neutral `[unsupported: …]` placeholder, and a stale
bundle drops v2 frames it doesn't understand until rebuilt.

Three things ride these frames beyond the v1 agent stream:

- **The state document** (Pillar 2): the server is the ordering authority. The
  app's `br.state.set/remove/update` send `state_write { set|patch, baseVersion }`;
  the agent's `ui_state` / `ui_patch_state` mutate the same doc; both directions
  get rebroadcast as `state` snapshot/patch frames. A version conflict resnapshots
  the client.
- **The catalog** (Pillar 3): `ui_panel` / `ui_render` mount id-keyed instances;
  `ui_patch { ops: add|replace|set_props|remove }` edits them in place.
- **The app contract** (Pillar 1): `app_call` → the app's registered action
  handler → `app_result`; `br.call({outputSchema})` → the agent's `emit_result`
  → an `output { value }` frame; app→agent `signal`s (subscribed via
  `ui_subscribe`). Every app-originated payload is wrapped in an untrusted
  `<app-data>` … `</app-data>` envelope the system prompt marks as data, not
  instructions.

> **`ui_error` — consumed server-side.** The SDK emits `ui_error` frames
> (render/action failures, rate-limited with a `droppedCount`); the daemon now
> buffers them (cap 5) and delivers them to the model under the artifact-repair
> grace discipline — riding the next turn as an `[app ui errors]` `<app-data>`
> envelope, or auto-starting one repair turn within 15 s of the last turn ending
> (capped at once per 60 s). `ui_suggest` is now a real MCP tool (non-blocking
> suggestion chips, ≤5), alongside the blocking `ui_ask`.

> **Note.** The v1 "WebSocket protocol" section in the historical record below
> predates protocol v2 (no `ready.protocol`/`surface`, no state/call/signal/kb
> frames). Use the
> [reference frame tables](../apps-sdk/sdk-reference.md#protocol-appendix)
> instead.

## Capability matrix

Deny-by-default, except `ui` (its blast radius is the app's own page). Grants live
under `manifest.agent.capabilities`.

| Capability | Default | Effect |
|---|---|---|
| `ui.enabled` | **on** | The `ui_*` tool set. `false` → a text-only app. |
| `ui.allow_theme` | on | `ui_theme` may restyle (pack / accent / mode / density). |
| `ui.allow_layout` | on | `ui_layout` may switch the region layout. |
| `ui.allow_ask` | on | `ui_ask` may block a turn on a user form. |
| `ui.allow_signals` | on | `ui_subscribe` may listen to declared app signals (listen only). |
| `ui.allow_html` | **off** | `ui_html` may inject server-sanitized rich HTML (XSS surface — opt-in). |
| `ui.allow_autorun` | **off** | An app signal may autonomously *start a turn* (spends provider quota — **user-granted only**, never agent-self-granted). Needs the signal to opt in (`surface.signals[].autorun:true`) + server budgets (6/min, 60/session). |
| `ui.max_panels` / `ui.ask_timeout_s` | 12 / 300 s | Panel cap (oldest evicted) / `ui_ask` timeout. |
| `files` | none | Mounted host dirs (`entries[]`, `ro`/`rw`, `out_dir`), `max_file_bytes`. |
| `data.sources[]` | none | `knowledge` / `spoke` / `omop` / `cdw` / `sql`. `ids` scope KB access; `read_only:false` grants KB writes. |
| `compute` | none | `sandbox` (`none`/`local`/`docker`), `timeout_s`, `network`, `max_mem`, `cpus`, `image`. |
| `vault` | none | Secret names referenceable via `{{vault:NAME}}`. |
| `memory` | off | Scratch KB + optional session-end distillation (`mode` `off`/`read`/`read_write`). |
| `tracing` | off | Span export (`redact` on by default; `processor` langfuse/phoenix/otlp). |
| `events[]` | none | Agent→app lifecycle stream to `br.on()` (`tool`/`handoff`/`compaction`/…). |

> **Autorun (shipped, capability-gated).** `ui.allow_autorun` (design §3.5/§3.7)
> lets a declared signal that opts in (`surface.signals[].autorun:true`)
> autonomously start a turn — **default off, user-granted only** (the agent can
> never self-grant), and bounded by per-minute/per-session budgets. Without the
> grant, or without the per-signal opt-in, signals stay queue-only (context for
> the next turn, never a turn trigger).

## Archetypes and starters

`create_app { archetype }` (or an inferred one from the title/description) seeds a
working, lint-clean `index.html` + `src/main.ts` **plus** the matching declared
`manifest.surface`, so a new app is an example of its shape rather than a chat box.
Six archetypes (`Archetype` in `mod.rs`, starters in `templates/starters/`):

| Archetype | Shape |
|---|---|
| `explorer` | A network/graph the agent renders + inspector + search (actions: `focus_node`; signals: `node_selected`, `search_submitted`). |
| `dashboard` | A KPI grid bound to `/metrics/*` + a refresh action (`set_metric`; `refresh_requested`). |
| `workbench` | A data table + row-select signal + a bound detail panel (`open_row`; `row_selected`, `filter_changed`). |
| `wizard` | A staged form that writes state, then submits (`go_to_step`; `step_changed`, `submitted`). |
| `canvas` | An author-registered draw surface + agent-called actions — the avatar/scene shape (`move_avatar`, `reset_scene`; `avatar_moved`; a `scene` component). |
| `chat` | The pre-v2 chat card; one option among six, never the default. |

## Theme packs

`manifest.theme` selects one of six packs (`THEME_PACKS`): `biorouter` (base look,
no overrides), `clinical`, `lab-notebook`, `terminal`, `journal`, `midnight` —
each a `[data-br-pack]` token layer with a dark variant. An unknown pack resolves
back to `biorouter`. `theme.accent` + `theme.tokens` (only `--br-*` custom
properties) are sanitized at render time; `ui_theme` can switch packs at runtime
when `allow_theme` holds. (The design listed a `glass` pack; the shipped set is the
six above.)

## Export modes

`export_app { id, target_dir, mode?, include?, bundle_daemon? }`:

- **`launcher`** (default) — app + launch scripts only; runs against whatever KBs
  / skills / extensions already exist on the target.
- **`full`** — also stages the server-side payload under `payload/`
  (`knowledge/<id>.brkb`, `skills/<name>/`) and writes `export.json`. Per-item
  selection via `include`, else derived from the agent config.

`bundle_daemon: "current"` stages this platform's `biorouterd` under
`payload/bin/` (a "fat" export); `"all"` is out of scope and treated as
`"current"`. External extensions are recorded as **registry references** in
`export.json`, not staged as `.brxt` bundles (out of scope). Every export ships
directly-runnable launchers for all three OSes (`run.command` / `run.sh` /
`run.bat`+`run.ps1`, shared `biorouter-launch.sh`, `serve.mjs` loopback proxy) and
a prebuilt `dist/app.js` — no build step. See the
[export guide](../apps-sdk/sdk-reference.md#export-guide).

## Multi-agent orchestration

Two mechanisms ship. **Sub-agents-as-tools** (`orchestration.sub_agents`): each
declared sub-agent is materialized as an engine recipe
(`materialize_subagent_recipe` in `apps.rs`) and exposed to the primary agent as
an agent-as-tool. **Named worker profiles** (`orchestration.agents`, validated by
`validate_profiles` in `apps.rs`, cap `MAX_PROFILES = 8`): each is a full alternate
`AgentConfig` with its own session/provider/subset-checked capabilities, advertised
in `ready.profiles`. The app addresses one via `br.agent(name)` (frames carry
`agent: name`); the main agent delegates mid-turn via the `consult` tool (main-only,
depth 1). **Serialized, not parallel** — one worker (or the main agent) runs at a
time on the app socket; parallel-across-profiles turns are a stretch goal.

**`consult` runs on the BR-71 workspace spine (2026-07).** A consulted worker's turn
holds the server turn lock, registers its agent in `AgentManager`, and publishes to the
session event bus — so it is observable via `GET /sessions/{id}/events`, steerable via
`POST /interrupt`, and cancellable via `workspace_close scope:"turn"`, exactly like a
glass-box subagent. `consult`'s own contract (name, params, depth-1, per-profile
timeout, blocking answer, error envelopes) is unchanged. See
[BR-71 §8.2](../agent-loop/designs/agent-workspace-control.md).

The bracket closes on **every** exit, including the per-profile deadline. That
deadline drops the worker's future rather than unwinding it, so the closing publish
runs from a destructor (`TerminalOnDrop` in `routes/apps.rs`) and emits
`TurnError { code: "worker_timeout" }` — the same envelope
`workspace::turn::classify_abort` produces for `TurnAbortCode::WorkerTimeout`.
Without it an observer watched the most common consult failure begin and never end.

> **Currency caveat.** This section was written while the worker-profiles work was
> landing, and named a `feat/apps-sdk-v2` branch that no longer exists in this
> repository. Treat the code — `consult` in `control.rs`, `AgentFacade` in
> `sdk.ts` — as authoritative, alongside
> [`br.agent` — worker profiles in the Apps SDK reference](../apps-sdk/sdk-reference.md#bragent--worker-profiles).

## The `biorouter apps` CLI

Apps are browser-rendered, but the CLI gives list/open/serve parity
(`crates/biorouter-cli/src/commands/apps.rs`):

```bash
biorouter apps list [--json]     # table of installed apps (id, title, archetype, updated)
biorouter apps open <id>         # ensure a daemon is up; open http://127.0.0.1:<port>/apps/<id>/
biorouter apps serve <id>        # ensure a daemon is up; print the URL; stay foreground if it started one
```

Daemon management is minimal: it health-checks `BIOROUTER_PORT` (default 3000) via
the auth-exempt `GET /status`, reuses a running daemon, else best-effort spawns
`biorouterd agent`. In-terminal rendering of an app is out of scope.

## Testing story

| Command | Gates |
|---|---|
| `cargo test -p biorouter-mcp --lib agent_drafter::` | store, tools, render, bundler, the `ui_*` tools, manifest/theme/surface types |
| `cargo test -p biorouter-mcp --test ui_example_apps` | example UI apps emit `ui` frames deterministically |
| `cargo test -p biorouter-server --lib routes::apps` | WebSocket frames, mid-turn dispatch, bridge rebind, parked `ui_ask`, KB grants, provider-class routing |
| `node scripts/agent-drafter/ui-control-harness.mjs` | SDK v2 self-test — real `sdk.ts` in jsdom vs a mock daemon: state/bindings, `ui_patch`, signals, `app_call`, `br.call`, `br.kb`, `br.model`, theme/layout, presence, `wsToken` (needs esbuild + jsdom) |
| `node scripts/agent-drafter/ui-control-harness.mjs --app <dir>` | serve a built app for a real browser (`/__emit` + `/__frames`) |
| `ui/desktop/scripts/appcheck/check-ui-app.mjs` | real agent; asserts `ui` frames arrive |

---

## Historical record: the v1 redesign and the app-building campaigns

> **Status of everything below.** Historical. These are the original v1 redesign
> notes plus the undated logs of the campaigns that authored the first ~90 apps.
> They are kept because they record decisions, regressions and their fixes that are
> easy to reintroduce. **Where v1 text conflicts with the sections above, v2 wins**
> — the v1 protocol frames and the 11-tool `ui_*` table in particular have been
> superseded, and each such subsection carries a note naming its v2 replacement.
> The campaign logs carried no dates in the original and none have been added.

### What the redesign changed

| Before | After |
|---|---|
| Static/agentic HTML artifacts | TypeScript app projects (esbuild bundle) |
| `agent.js` "bridge" routed prompts into the chat box (no real reply) | Per-app WebSocket runs the real agent loop and streams the reply |
| Export = Tauri/Rust project | Export = standalone TypeScript project (esbuild + tiny static server) against a BioRouter daemon |
| Shown inline in a sandboxed chat iframe | Served by `biorouterd` at `/apps/<id>/`, opened in the browser |
| No per-artifact model/extension/skill/KB | Manifest carries model (default MiMo), extensions, skills, knowledge base, persona |

### Subsystem architecture

- **Store + manifest** — `crates/biorouter-mcp/src/agent_drafter/store.rs`.
  Each app is a project dir `~/.config/biorouter/agent_drafter/<id>/`:
  `manifest.json`, `index.html`, `src/main.ts`, `src/sdk.ts`, `dist/app.js`.
  Manifest `agent` block: `system_prompt`, `greeting`, `model {provider, model}`,
  `extensions[]`, `skills[]`, `knowledge_base`.
- **App SDK** — `templates/sdk.ts` (authored in TypeScript, bundled into each app).
  Opens the per-app WebSocket, streams events, renders markdown (headings, lists,
  code, links, **GitHub-flavoured Markdown tables**), handles multimodal image
  input, and can auto-mount a chat panel into `[data-br-chat]`.
- **Bundler** — `bundle.rs`. Locates esbuild (`$BIOROUTER_ESBUILD_BIN` → desktop
  `node_modules/.bin/esbuild` → PATH → `npx esbuild`); falls back to a vendored
  type-stripper when esbuild is absent. `src/main.ts` → `dist/app.js` (IIFE).
- **MCP tools** — `mod.rs`: `create_app`, `configure_app`, `update_app`,
  `build_app`, `launch_app`, `list_apps`, `read_app`, `preview_app`, `export_app`,
  `delete_app`, `set_app_size`.
- **Server routes** — `crates/biorouter-server/src/routes/apps.rs`:
  - `GET /apps` — list manifests (JSON)
  - `GET /apps/{id}/` — assembled index.html (theme + base href + bundle)
  - `GET /apps/{id}/dist|assets/{*path}` — bundle + assets
  - `GET /apps/{id}/agent` — **per-app agent WebSocket** (creates a session,
    applies model/extensions/skills/KB/persona, runs `agent.reply`, streams
    `message`/`thought`/`tool`/`done`/`error` frames)
  - `POST /apps/{id}/build`, `DELETE /apps/{id}`
  - Browser-facing GETs under `/apps` are exempt from the secret-key middleware
    (a browser tab can't send the header); the daemon binds localhost only.
- **Frontend** — `ui/desktop/src/components/applications/ApplicationsView.tsx` +
  an "Applications" sidebar entry under Knowledge. Lists built apps; Launch opens
  the app URL in the default browser via the existing `openExternal` IPC.

### WebSocket protocol, v1 (browser ⇄ backend)

> **Superseded by protocol v2** (see "Protocol v2 overview" above and
> the [reference frame tables](../apps-sdk/sdk-reference.md#protocol-appendix)).
> The frames below are the v1 subset; v2 adds the `ready.protocol`/`surface` fields
> and the `state_write` / `call` / `signal` / `kb` / `app_result` / `model_status`
> frames.

Client → server: `{"type":"prompt","text":"…","images":[{"mimeType","data"}]}`,
`{"type":"cancel"}`, `{"type":"tokens"}`, `{"type":"history"}`,
`{"type":"modelselect",…}`, `{"type":"approve"|"reject",…}`,
`{"type":"widget_action",…}`, and — for agent-driven UI —
`{"type":"ui_surface","surface":{…}}` and
`{"type":"ui_reply","requestId":"…","payload":{…}}`.

Server → client: `{"type":"ready","capabilities":[…]}`, `{"type":"message","delta"}`,
`{"type":"thought","delta"}`, `{"type":"tool","name","status"}`,
`{"type":"context"|"history"|"model"|"guardrail"|"approval"}`,
`{"type":"ui","cmd":"panel"|"render"|"chart"…}`, `{"type":"done"}`,
`{"type":"error","message"}`.

### Agent-driven UI, v1 (the `ui_*` tools)

> **Extended in v2.** This v1 table lists the original 11 tools; v2 adds
> `ui_patch_state`, `ui_patch`, `ui_html`, `ui_figure`, `app_call`, `emit_result`,
> and `ui_subscribe` (18 total). Full table + widget catalog in the
> [reference](../apps-sdk/sdk-reference.md#agent-driven-ui-tools).

An app's agent **drives the app**, it doesn't just answer inside it. A per-session
in-process MCP server (`agent_drafter/control.rs`, injected by `configure_agent`
exactly like `datasql`/`files`/`compute`) gives the agent tools whose effect is a
command pushed down that app's own WebSocket:

| Tool | Effect |
|---|---|
| `ui_describe` | Report the page's regions, element ids, mounted panels, state |
| `ui_panel` | Mount / replace / remove a panel or dashboard (widget tree) |
| `ui_render` | Render into `@region:<name>`, `@panel:<id>`, `@chat`, `@main`, or a CSS selector |
| `ui_chart` / `ui_graph` | Draw a bar/line/pie chart, or a node/edge graph |
| `ui_highlight` | Outline / pulse / focus part of the app, with a note |
| `ui_theme` / `ui_layout` | Restyle (accent, light/dark, density); switch to sidebar/split/dashboard |
| `ui_notify` | Transient toast |
| `ui_state` | A shared key/value bag mirrored into `br.ui.state` (`br.ui.onState`) |
| `ui_ask` | Render a form and **block the tool call** until the user submits — the tool result *is* their answers, so the agent branches on them inside one turn |

Design notes:

- **On by default.** `capabilities.ui` (`manifest.rs`) defaults to enabled, unlike
  the deny-by-default `files`/`data`/`compute`/`vault` grants: its blast radius is
  the app's own page. Set `{"ui":{"enabled":false}}` for a deliberately text-only
  app. Sub-switches `allow_theme` / `allow_layout` / `allow_ask` also default on.
- **Authors expose targets** with `<section data-br-region="results">`; the agent
  finds them via `ui_describe` and writes to `@region:results`. Panels need no
  region — the SDK always provides a dock (`.br-dock`), which shifts the page
  rather than covering it.
- **The bridge is rebindable.** `AppState::get_agent` caches one agent per session
  and `add_inprocess_server` is idempotent by name, so a reconnecting browser
  reuses the *same* `AppControlServer`. `UiBridge::attach()` re-points it at the
  new socket (and replays `ui_state`); `detach()` unblocks any parked `ui_ask`.
  Without this, every reload would leave the `ui_*` tools writing into a dead
  channel.
- **The socket is split.** `handle_agent_socket` `select!`s over three sources:
  agent events, agent-issued UI commands, and inbound client frames. That is what
  lets a `ui_ask` — parked *inside* `agent.reply` — be answered mid-turn, and it
  makes `cancel` work mid-turn for the first time. Frames that would start new
  work (`prompt`, `widget_action`) are queued (bounded) rather than dropped.
- **Model-robustness.** `spec`/`body` are `serde_json::Value`, so schemars would
  emit a permissive `true` schema; we attach concrete inlined schemas
  (`#[schemars(with = …, inline)]` — no `$ref`/`$defs`, which several providers
  mishandle) *and* accept a stringified object (`unstringify`), because models
  observably JSON-encode nested objects into strings.

Examples live in `scripts/agent-drafter-apps/examples/ui/` (install with
`scripts/agent-drafter-apps/install-examples.sh`). Gates:
`cargo test -p biorouter-mcp --test ui_example_apps` (deterministic) and
`ui/desktop/scripts/appcheck/check-ui-app.mjs` (drives a real agent and asserts
`ui` command frames arrive).

### Verification evidence

Unit + integration (deterministic, no daemon, no LLM):

```bash
cargo test -p biorouter-mcp --lib agent_drafter::       # 91 pass (store, tools, render,
                                                        #   bundler, control.rs ui_* tools)
cargo test -p biorouter-mcp --test ui_example_apps      #  5 pass (the 10 example apps)
cargo test -p biorouter-mcp --test agent_drafter_registered
cargo test -p biorouter-server --lib routes::apps       # 26 pass (frames, mid-turn dispatch,
                                                        #   bridge rebind, parked ui_ask)
```

The two that matter most, because they pin the design rather than the code:

- `a_parked_ui_ask_is_unparked_by_a_midturn_ui_reply` — drives the real `ui_ask`
  tool against the real mid-turn dispatcher. Without the split socket it hangs.
- `rebinding_the_bridge_keeps_a_reused_server_working` — a reload reuses the
  cached agent's `AppControlServer`; if `attach()` didn't re-point it, every
  `ui_*` call after the first reload would fail forever.

Browser, against the real `sdk.ts` bundle (`scripts/agent-drafter/ui-control-harness.mjs`
stands in for the daemon and speaks the wire protocol):

- every `ui` command applied — panels, dock, stat/progress/table/chart/graph nodes,
  `@region:` render, highlight + focus dimming + callout, theme (accent/mode/density),
  layout presets and `sidebar_width`, sticky and auto-dismiss toasts, state bag;
- `ui_ask` round-trip: form renders all five field kinds, submit sends
  `ui_reply` with typed values, **Escape sends `{"cancelled":true}`** so a parked
  tool can never hang;
- a widget `button` with `submit` posts `widget_action` back into the agent loop;
- a render at a missing target raises a visible warning toast instead of vanishing.

Live, with a real LLM (local Ollama `qwen3.6`, no API key):

- all 10 tools reach the model as `appcontrol__ui_*`;
- given one prompt, the agent called `ui_chart` + `ui_panel` and the page changed;
- all **10/10 example apps** emit `ui` command frames under
  `check-ui-app.mjs` (one needs >180 s on a small local model).

Export, on a **fresh `$HOME`** with no app installed and no daemon on :3000:

- `bash run.sh` installs the app, starts `biorouterd` on the requested port,
  verifies `GET /apps/<id>/` → 200, and opens it; the browser connects to
  `ws://127.0.0.1:<that port>/…` derived from the page origin;
- `node serve.mjs` serves the folder on loopback, starts/reuses a daemon, and
  proxies `/apps/**` including the WebSocket — `br.model.list()` returns 25
  providers through the proxy (it silently returned `[]` before, wrong origin).

### Iteration log: bugs found via testing, then fixed

1. **`biorouterd` requires the `agent` subcommand** — operational note; the bare
   binary prints usage.
2. **"Provider not set"** — a fresh per-connection session's agent has no provider
   until `configure_agent` sets one. If the app's provider can't be created (e.g.
   its API key isn't reachable by the running process), the agent had *no*
   provider and `reply` failed cryptically. **Fix:** `configure_agent` now falls
   back to BioRouter's global provider/model when the app-specific provider can't
   be created (`apps.rs`).
3. **Markdown tables not rendered** — the SDK's renderer handled headings/lists/
   code/links but not GitHub-flavoured Markdown tables (agents emit them often).
   **Fix:** added a table parser to `sdk.ts` `renderMarkdown` + table CSS in
   `theme.css`. Verified the rebuilt bundle emits `<table>`.

### Known limitations and next steps

- **Per-app skill scoping** is advisory (the selected skills are named in the
  system prompt and the `skills` extension is enabled); BioRouter's skill
  enable/disable is global, so true per-app skill isolation is a follow-up.
- **API keys**: a freshly-built `biorouterd` doesn't inherit the GUI binary's
  macOS Keychain grant; pass provider keys via env or run the signed binary.
- **Apps bundle their own `src/sdk.ts`** — SDK improvements reach existing apps
  only after a rebuild (re-copy `sdk.ts` + `build_app`). New apps get the latest.
- The chat-side preview is a static **card** (apps run in the browser, by design).
- Older artifact-format apps from the previous design remain in the store but
  won't serve a working bundle (no `src/main.ts`); recreate them with `create_app`.

### Campaign 1: 16 example apps built by driving MiMo

Each app below was authored end-to-end by the **MiMo model itself** calling the
Agent Drafter tools (`create_app` → `build_app` → `launch_app`) via
`biorouter run --with-builtin agent_drafter -t "…"` (see
`scripts/agent-drafter-apps/round.sh`). All are served by `biorouterd` at
`/apps/<id>/` and pass the checklist below.

| App | Extensions | What it does |
|-----|-----------|--------------|
| spoke-network-explorer | (chart-block) | Natural language → SPOKE graph relationships + **AI-generated inline charts** |
| web-research-assistant | webdocuments | Query → web search → sourced markdown answer |
| pathway-explainer | — | Pathway: overview / steps / genes / regulation |
| gene-function-explorer | — | Gene → function, pathways, expression, disease |
| variant-interpreter | — | Variant → functional impact + ACMG-style evidence |
| clinical-trial-navigator | — | Condition → trial phases, endpoints, criteria |
| drug-interaction-analyzer | — | Drugs → interactions, mechanism, severity |
| lab-protocol-generator | — | Experiment → numbered reproducible protocol |
| literature-summarizer-pro | — | Text → TL;DR / findings / methods / limitations |
| biostatistics-advisor | — | Study design → recommended test + assumptions table |
| differential-diagnosis-helper | — | Symptoms → structured differential diagnosis (with caveat) |
| sequence-analysis-toolkit | — | DNA/RNA/protein → GC, ORFs, translation, motifs |
| cell-type-annotator | — | Marker genes → likely cell type + confidence |
| enzyme-kinetics-tutor | — | Michaelis–Menten / Km / Vmax, step-by-step |
| omics-pipeline-advisor | — | Assay description → tools + workflow + QC |
| medical-term-explainer | — | Term → plain-language + technical definition |

#### Per-app checklist (all green)

For every app: `manifest` valid (agentic, model = MiMo, non-empty system prompt) ·
`GET /apps/<id>/` 200 with theme injected · `GET /apps/<id>/dist/app.js` 200
(esbuild bundle > 500 B) · per-app agent WebSocket streams a real, non-empty,
persona-shaped reply · no error frame. Harness:
`scripts/agent-drafter-apps/round.sh verify` + `ui/desktop/scripts/appcheck/check-app.mjs`.

Round 1: **15/15 pass**. After the chart iteration + SDK propagation + biorouterd
rebuild: regression re-verify **15/15 pass**.

#### Campaign 1 iteration log (drive → find issue → fix → recompile → re-make)

1. **xargs arg-length** in the batch authoring runner (long personas) → switched
   to a batched background loop (`round.sh`).
2. **AI-generated visualizations** (the SPOKE requirement): the SDK rendered no
   charts. **Fix:** added a `renderChart` (dependency-free SVG bar/line) to
   `sdk.ts`, wired a ```chart fenced block into `renderMarkdown`, added chart CSS
   to `theme.css`. Recompiled biorouterd, re-copied `sdk.ts` into all 24 stored
   apps, rebuilt bundles. Verified: SPOKE renders an SVG bar chart in the browser.
3. **`autovisualiser` hijacks visualization**: with that extension the agent calls
   `show_chart` (a `ui://` resource the app WebSocket only surfaces as tool
   activity) and the tool turn timed out, instead of emitting an inline chart
   block. **Fix:** the SPOKE app drops `autovisualiser` and uses a chart-block
   system prompt, so the chart renders inline and the turn finishes promptly.
   (Lesson: apps wanting app-native inline charts should not also load
   `autovisualiser`.)

### Campaign 2: scale-up to 22 MiMo-authored apps, export pipeline, workflow loops

A second push added 6 more app *types* (now 22 apps total, all MiMo-authored via
the tools) and hardened the whole build→export→run pipeline.

New apps: gene-expression-barplot, survival-analysis-explainer,
epidemiology-trend-explorer, pharmacokinetics-visualizer (line charts),
variant-consequence-distribution (pie), clinical-calculator.

Full-fleet results (harness: `scripts/agent-drafter-apps/round.sh`,
`ui/desktop/scripts/appcheck/{check-all,export-all}.mjs`):

- **CHECKLIST 21/21 ok** — serve + esbuild bundle + theme + real streamed reply.
- **EXPORT 21/21 ok** — every app exports a complete, directly-runnable folder
  (index.html + src + sdk + package.json + serve.mjs + run.command + run.sh +
  prebuilt dist/app.js), correct biorouterd endpoint, and the exported `src`
  re-bundles cleanly.
- **Exported app runs standalone** in a browser (served on :8787, talking to a
  biorouterd) — verified live (BRCA1 reply).
- **Agentic multi-turn memory** verified (turn 2 recalled "CFTR").
- **Charts**: bar (SPOKE), line, and pie (variant consequences) render as native
  SVG with the BioRouter palette; markdown tables render bordered.

#### Provider-agnostic apps

Apps no longer hardcode a model. By default an app pins **no** model and inherits
whatever provider/model the user configured in BioRouter (Anthropic, OpenAI,
Azure, Bedrock, Ollama, Xiaomi MiMo, local llama.cpp, …). A specific
provider+model is stored only when explicitly chosen (`configure_app`). The
WebSocket handler applies the app's model or falls back to the global provider.

#### Export made directly runnable and portable

`export_app <id> <target_dir>` (e.g. "export this app to my Desktop") writes a
self-contained folder. Double-click `run.command` (macOS) or `bash run.sh`: it
installs the app into the local store, reuses or starts a `biorouterd`, verifies
the daemon actually serves the app, and opens it. No Node, no `npm install`, no
build step — `dist/app.js` ships prebuilt. `GET /apps/{id}/export` returns the
same scaffold as JSON (for the GUI / tooling).

**This was broken until v1.87.3**, in four independent ways. Recorded here because
each one is easy to reintroduce:

1. **`manifest.json` was never exported** (`collect_files` skipped it and nothing
   re-added it). The manifest *is* the app's registration — `serve_index` and
   `agent_ws` both `load_manifest` and 404 without it. So `run.sh`'s self-install
   copied files into the store that the daemon could not see, and its
   `[ ! -f "$STORE/manifest.json" ]` guard could never become false: it re-copied
   and re-failed on every run. `scaffold_standalone` now emits a canonical
   manifest serialized from the parsed `Manifest`.
2. **The agent endpoint was hard-coded to `ws://127.0.0.1:3000`.** The desktop app
   starts its daemon on an **ephemeral port**, so an exported app failed with
   "Could not reach the BioRouter backend" even while BioRouter was running. The
   export now leaves `endpoint` unset so the SDK derives it from the page's own
   origin (`sameOriginEndpoint`), with `:3000` kept only as a fallback for someone
   opening `index.html` off disk. Both launch paths serve the page from an origin
   that also answers `/apps/<id>/agent`.
3. **`serve.mjs` was a bare static file server.** `npm start` produced a UI with no
   backend at all; the README told the user to start `biorouterd` in another
   terminal, which nobody does. It now locates/starts a daemon and transparently
   proxies `/apps/**` — including the WebSocket upgrade — so page and agent share
   an origin. (`br.model.list()` also stopped 404ing, since it derives its HTTP
   base from the connected endpoint rather than `window.location`.)
4. **The launcher failed silently.** It backgrounded a bare `biorouterd` inside
   `( … & )`, which returns 0 even when the binary is missing, so `set -e` never
   caught it; the health loop then timed out for 40s and it opened a dead URL.
   `biorouter-launch.sh` now searches `BIOROUTERD_BIN` → PATH → `~/.local/bin` →
   `/usr/local/bin` → `/opt/homebrew/bin` → the app bundle, sets `BIOROUTER_PORT`
   (**not** `BIOROUTER_SERVER__PORT` — `Settings` is flat; only the secret key uses
   the `__` form), falls forward through ports, and `die`s with an actionable
   message. `export_app` chmods the launchers +x, and `.vault/` is excluded from
   exports so sealed secrets never leave the author's machine.

#### Workflow-style agentic loops and guardrails

Every user message runs BioRouter's **full agent loop** — multi-step tool calls
+ reasoning, not a single LLM reply — so apps encode real pipelines via
system-prompt steps + extensions (modeled on the knowledge sub-agent loop's
bounded design). `agent.max_turns` bounds/raises that loop (a guardrail against
runaway/cost; the knob workflow apps raise), defaulting to a safe server cap (24).

#### Security and consistency review: findings and fixes

- **serve.mjs path traversal**: the exported static server now resolves under
  ROOT and rejects escapes (verified: `/../../etc/passwd` → blocked).
- **Export route blocking the runtime**: `GET /apps/{id}/export` (and the build
  it may trigger) now runs via `spawn_blocking`, keeping the daemon responsive.
- **`</script>` breakout** in injected config: neutralized (`<` → `<`).
- **Path traversal in the store**: `safe_relative` rejects absolute/`..` paths.
- **Auth posture**: browser-facing GET `/apps/*` is intentionally
  secret-exempt (localhost-only serving, like the MCP UI proxy); management
  verbs (POST/DELETE) require the secret.
- **Provider keys**: never embedded in apps/exports; supplied by the user's
  biorouterd. A fresh unsigned dev `biorouterd` can't read the macOS keychain
  (per-binary grant) → pass the provider key via env in dev.

### Campaign 3: diverse interactive UIs and a build harness (70 more apps)

The apps initially all looked alike because they used the default chat panel. Two
changes fixed that, plus a build-time validation harness:

1. **Rich control design system** (`theme.css`) — `br-select`, `br-slider`,
   `br-switch`, `br-check`, `br-chips`/`br-chip`, `br-tabs`/`br-tab`, `br-grid`,
   `br-dropzone`, `br-draglist`/`br-dragitem`, `br-mapgrid`/`br-region` — plus an
   SDK `br.run(prompt, target)` helper that streams markdown/charts into any
   element (wire a button/slider/select/region/drop to it in one line).
2. **Build harness** (`bundle::lint_app`, surfaced in `build_app` + a `lint_app`
   tool) — validates whatever the model generates and feeds findings back so it
   self-corrects: ① backend wired via the App SDK, ② self-contained (no
   CDN/external assets), ③ on-theme (br-* + a result surface), ④ element-id
   consistency (no `getElementById` null crash), ⑤ controls actually wired to
   events. This is a guardrail around free-form LLM generation, not just
   templates.

**Five agent_drafter iterations** (driven by testing): controls+harness+`br.run`
→ explicit idempotent `create_app` id → element-id lint → `br.run` serialization
(rapid control changes never overlap/orphan) → control-wiring lint.

#### 50 detailed-spec apps (round2) — varied UIs, 10 at a time

Built by MiMo through the tools, in 5 batches, each verified (serve + esbuild
bundle + **custom UI in markup** + real streamed reply) with retry:
**batches 1–5 = 50/50.** Patterns: sliders (dosing/PCR/kinetics/decay),
dropdowns (organism/assay/tissue/biomarker), button grids (amino-acids/codons/
elements/imaging), toggles+chips+checkboxes (DEG filters/QC/pathways/symptoms),
region maps (prevalence/outbreak/biobank/trial-sites), drag-drop (workflow/
gene-set/protocol reorder, abstract/CSV drop), tabs (omics/patient/gene/compound),
and form wizards (study-design/grant-aims/cohort).

#### 20 vague-prompt apps (round3) — the benchmark

Built from one-line ideas with **no UI/SDK/layout guidance** to measure how the
agent handles under-specified requests, relying on its instructions + harness.

| Prompt style | working | with real controls (raw markup) | avg distinct control-types/app |
|---|---|---|---|
| Detailed (round2, 50) | 50/50 | 47/50 | **3.04** |
| Vague (round3, 20)    | 20/20 | 18/20 | **2.10** |

Finding: the harness makes even vague requests yield working, on-theme custom
apps (~90% use real controls), but **detailed tickets produce richer UIs** (more
distinct control types per app). Vague apps trend toward simpler input+button or
single-control layouts. (Verifier note: the served HTML embeds the theme CSS,
which *defines* every `br-*` class — the honest "real controls" metric is
measured against the raw app markup in the store, not the served page.)

#### Autonomous testing setup

Provider key cached to `/tmp/br-mimo.key` (600) + `start-biorouterd.sh` /
`author.sh` read it, so authoring + tests run with **no macOS Keychain prompts**.
Harness: `scripts/agent-drafter-apps/{round2,round3}.sh`,
`ui/desktop/scripts/appcheck/{batch-verify,check-all,export-all,benchmark}.mjs`.

### Where the v1 work was done

The v1 redesign was implemented in an isolated git worktree (branch
`feat/agent-drafter-apps`) because a parallel workstream
(`perf/streaming-and-latency`) was sharing the main working tree and
snapshotted/reset files mid-edit. The worktree kept the two efforts from
clobbering each other. Neither that worktree nor the `feat/agent-drafter-apps`
branch exists in this repository any more.

## Related documentation

- [Apps SDK reference](../apps-sdk/sdk-reference.md) — the territory to this
  document's map: every `br.*` signature, the manifest schema, frame tables and
  export format.
- [Apps SDK v2 design](../apps-sdk/v2-design.md) — the design spec the nine
  pillars above summarise.
- [Apps SDK v2 phase roadmap](../apps-sdk/v2-phase-roadmap.md) — how the pillars
  were sequenced into shippable phases.
- [App test-drive runbook](testing/app-test-drive-runbook.md) — how to drive a
  model through the Agent Drafter tools and verify the app it builds.
- [100-app test-drive audit](../history/agent-drafter-testdrive-100/README.md) —
  the largest authored-app campaign, with per-app verdicts and the defects it found.
