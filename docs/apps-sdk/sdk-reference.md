# BioRouter Apps SDK v2 reference

> **What this is.** The human-facing developer reference for **BioRouter apps**: the manifest schema, the `br.*` client runtime, the agent-facing `ui_*` tools and widget catalog, the WebSocket frame protocol, the security model, the export format, and the test gates.
> **Status:** Current. It documents what actually ships in this build, and flags the pieces that are only partially realised in the [shipped-state summary](#what-is-and-is-not-shipped) below.
> **Audience:** developers authoring or editing a BioRouter app, and developers working on Agent Drafter itself.
> **Version key.** *v1* is the original Agent Drafter shape — a generated front-end that flattens its controls into one English prompt and streams markdown back. *v2* is the current, additive surface described here: typed actions, shared state, a component catalog, platform APIs and worker profiles. Every v1 manifest, frame and API still works unchanged; v2 fields are all optional. *v2.1* labels work deliberately deferred (see the design doc).

A BioRouter app is a TypeScript front-end (`src/main.ts` plus `index.html`) wired to a *real* per-app BioRouter agent over a WebSocket. The app is authored by the Agent Drafter agent through the `create_app` / `update_app` / `build_app` tools, and it is driven live by its own runtime agent, which calls `ui_*` tools to write into the page. Reach for an app rather than ordinary desktop chat when you want a persistent, purpose-built interface — a graph explorer, a dashboard, a workbench — instead of a transcript. Because `export_app` produces a project humans read and edit, this reference describes the runtime from the author's side as well as the agent's.

The *behaviour* below is verified against the code:

- `crates/biorouter-mcp/src/agent_drafter/manifest.rs` — manifest types
- `crates/biorouter-mcp/src/agent_drafter/mod.rs` — the authoring tools
- `crates/biorouter-mcp/src/agent_drafter/control.rs` — the `ui_*` tool server
- `crates/biorouter-mcp/src/agent_drafter/templates/sdk.ts` — the `br.*` runtime
- `crates/biorouter-server/src/routes/apps.rs` — the `/apps/*` HTTP + WebSocket

The *intent* behind each pillar lives in the [Apps SDK v2 design](v2-design.md); the sequence in which the pillars land is in the [phase roadmap](v2-phase-roadmap.md).

## What is and is not shipped

The design describes nine pillars. Most of the surface below ships as documented; these are the exceptions, gathered in one place so you do not have to hunt for inline "Partial" markers.

| Area | Shipped state |
|---|---|
| Worker profiles (`br.agent`, `consult`) | **Partial — actively landing.** Cross-profile turns are serialized, not parallel; `consult` depth is 1; workers get no `ui_*` control unless the profile opts in. Treat the code in `crates/biorouter-mcp/src/agent_drafter/` and `crates/biorouter-server/src/routes/apps.rs` on `main` as authoritative. |
| Per-app skill scoping | **Advisory only.** The named skills are surfaced to the agent, but BioRouter's skill enable/disable is global, so true per-app isolation is a follow-up. |
| Export payload for external extensions | **Not staged.** External extensions are recorded as pinned registry references in `export.json`; installed-bundle (`.brxt`) staging is out of scope in this build. |
| `bundle_daemon: "all"` (universal daemon) | **Out of scope.** Treated as `"current"` with a note. |
| In-SDK capability re-consent screen | **Not shipped.** The launcher prompts for payload-install consent; the richer capability/credential onboarding screen the design envisions is not built, and `export.json`'s `required_credentials` / `runtime_requirements` are empty. |

## Quick start

```text
create_app  →  (edit src/main.ts + index.html)  →  build_app  →  launch_app  →  export_app
```

1. **Create.** `create_app { title, description, archetype?, extensions?, skills?, knowledge_base?, capabilities?, … }`. Omitting `html`/`src/main.ts` seeds a working, lint-clean **archetype** starter (explorer / dashboard / workbench / wizard / canvas / chat) plus a matching declared `manifest.surface`. Default kind is `agentic`; pass `kind: "static"` for a plain page with no agent. An explicit `id` that is already taken is refused unless this chat created that app, or the call passes `replace: true` (see [deleting, re-creating and broken apps](#deleting-re-creating-and-broken-apps)).
2. **Edit.** `update_app { id, path?, content? | old_str+new_str }`. Editing anything under `src/` marks the bundle stale. Author your UI in `src/main.ts` (which `import`s `./sdk`) and your shell in `index.html`.
3. **Configure (optional).** `configure_app { id, system_prompt?, model?, extensions?, skills?, knowledge_base?, max_turns?, capabilities?, guardrails?, reliability?, orchestration?, output_type?, durable_session? }`.
4. **Build.** `build_app { id }` bundles `src/main.ts → dist/app.js` with esbuild, refreshes the vendored `src/sdk.ts`, stamps `manifest.sdk_hash`, and runs the lint harness. Fix any harness **ERROR** before launch or export. See [building and serving](#building-and-serving) for where esbuild comes from and when an app is rebuilt on its own.
5. **Launch.** `launch_app { id }` returns `http://<host>/apps/<id>/`. That address carries no access cookie, so it is a name for the app rather than a way in. The desktop opens it as a live preview in the chat's artifact panel, on the window's own daemon whatever port the link names (a link from an earlier run names a port that is gone), through a launch link it mints itself. Only `http://127.0.0.1:<port>/apps/<id>/`, the spelling the daemon writes, is taken for an app this way; a `localhost` link is left alone, because that is usually a web development server and the person's own project. The preview's **Open in browser** opens the app in the default browser the same way, and it is always the app the preview shows, whatever address the app's page has put in the toolbar. On Linux, a default browser packaged as a snap (stock Ubuntu's Firefox and Chromium) cannot read the page the desktop hands the link over in, so the desktop offers to copy a one-time link to paste into it instead. The CLI prints the URL, and `biorouter apps open <id>` opens the app from a terminal.
6. **Export.** `export_app { id, target_dir, replace?, mode?, include?, bundle_daemon? }` writes a folder that runs the app on another computer; the Applications view's **Export** does the same through the daemon. See the [export guide](#export-guide).

Inspect anytime with `list_apps`, `read_app`, `preview_app`; remove with `delete_app`; resize the preview card with `set_app_size`.

Apps live under `~/.config/biorouter/agent_drafter/<id>/` (`manifest.json`, `index.html`, `src/main.ts`, `src/sdk.ts`, `dist/app.js`) and are served by `biorouterd` at `/apps/<id>/`, with the agent socket at `/apps/<id>/agent`.

## Building and serving

### Where esbuild comes from

The desktop app ships a pinned esbuild (`esbuild`, or `esbuild.exe` on Windows) beside `biorouterd`, staged at package time by `ui/desktop/scripts/fetch-esbuild.js` from the official `@esbuild/<platform>-<arch>` npm package and checked against the registry's sha512 for that release. The daemon looks for esbuild in this order:

1. `$BIOROUTER_ESBUILD_BIN`.
2. Beside the running executable: the copy the desktop app ships.
3. A source checkout's `ui/desktop/node_modules/.bin/esbuild`, found from the working directory upwards (a dev tree).
4. `esbuild` on the daemon's `PATH`, then on the login shell's `PATH`. A daemon the desktop app starts on macOS has launchd's short `PATH`, which holds no Homebrew or nvm tools.
5. `npx --yes esbuild@<pinned version>`, never an unpinned `esbuild`. Its npm cache is a directory the daemon makes for itself, under a random name and readable only by its own account, never a guessable path in a shared temporary folder another account could fill first.

The pin lives in `ESBUILD_VERSION` in both `fetch-esbuild.js` and `agent_drafter/bundle.rs`, and a test holds the two together.

### The fallback bundler

When no esbuild can run (a CLI-only install with no Node, for example), a small type-stripper builds what it can prove it handles and refuses the rest with a clear message instead of producing a broken bundle. It refuses a `src/main.ts` that imports any module other than `./sdk` (also spelled `./sdk.ts` or `./sdk.js`), renames an SDK import, splits an import over several lines, uses a dynamic `import()`, re-exports another module, declares an object type after a parameter list, or has a line-start `key: Value` it cannot tell from a typed class field. `main.ts` runs in a function scope of its own inside the SDK's, so it can reuse an SDK name without a syntax error and cannot replace an SDK function. When `node` is reachable the result must pass `node --check` before it is installed.

A fallback build never replaces a `dist/app.js` that esbuild (or a custom build script) produced, and it is marked as a fallback twice: in the bundle's first line and in `manifest.sdk_hash`, which reads `fallback:<fingerprint>`. A daemon that can run esbuild treats such a bundle as stale and rebuilds it properly. `build_app`, `launch_app` and `POST /apps/<id>/build` (whose answer carries `"fallback": true`) all report a fallback build as a warning.

Every bundle starts with a one-line comment naming its builder, the fingerprint of the App SDK inside it, a fingerprint of the `src/` files it was built from and the Biorouter version that built it:

```text
/* biorouter-app-bundle builder=esbuild sdk=<fingerprint> src=<fingerprint> biorouter=<version> */
```

When the build kept an SDK a newer Biorouter had vendored, the line also carries `sdkv=<version>`, the version that SDK came from.

### What a build writes

esbuild writes into a fresh hidden directory under `dist/`, and its output is then moved into place: `dist/app.css` first, when the code imports a stylesheet (`import "./styles.css"`), then `dist/app.js`. When the code no longer imports one, a `dist/app.css` left by an earlier build is removed. The page links `dist/app.css` itself if it wants it; the daemon does not add the tag.

The vendored `src/sdk.ts` is refreshed before the build when the code imports the SDK, however the import is spelled, or when the app has a `src/sdk.ts` at all. A build that installs no bundle leaves the app as it found it: when no esbuild can run, the fallback's refusals are decided before `src/sdk.ts` is touched, and a refresh made for a build that then failed is undone.

### When an app is rebuilt on its own

Serving `GET /apps/<id>/` and `launch_app` rebuild an app first when its bundle is stale, and so does an export of an agentic app. Any app with a `src/main.ts` is built, static or agentic. A bundle is stale when:

- there is no `dist/app.js`;
- `built_at` is unset, which `update_app` does when it edits `src/`;
- a file under `src/` changed since the bundle was built, which catches edits made with the developer shell or text editor. The comparison is by content, against the `src=` fingerprint in the bundle's first line, so a file whose modification time is in the future does not make the app rebuild on every load; a bundle without that record (an older one, or one a custom build script wrote) is compared by modification time;
- the fallback bundler built it and esbuild can run now;
- it was built against a different App SDK than this daemon ships, unless the SDK it carries came from a newer Biorouter (`sdkv=`, or the builder's version when that is absent). A daemon never rebuilds an app down to its own older SDK, and when it rebuilds such an app for another reason it keeps the newer SDK and records where it came from, so two daemons of different versions serving one store no longer rebuild each app back and forth.

Builds of one app are serialised: a second request waits for the first, then finds the bundle fresh. Bundles and the refreshed `src/sdk.ts` are written to a temporary file and renamed into place, so a browser never loads half a bundle. A build that failed is remembered against its inputs (the files under `src/`, the bundle and the manifest stamps), so an app whose code does not compile is not rebuilt on every page load; any edit is a new attempt.

When an app that cannot run without a bundle (an agentic app, or one with a `src/main.ts`) has none and its build failed, `GET /apps/<id>/` answers `500` with a short page that shows the escaped build log and asks the person to have the agent fix the app and rebuild it. An app that still has an older bundle keeps serving it, and `launch_app` then returns the address with a warning that the browser gets the previous bundle, without the latest changes, and the build log.

### Static apps and inline scripts

A static app is a page written in `index.html`. Inline `<script>` blocks and `on…=` handlers (`onclick="…"`) written in that file run when the app is launched. The daemon hashes each inline script in the page it serves and names exactly those hashes in that response's `script-src`. Handlers are not hashed. The daemon moves them out of the markup instead: each handler attribute becomes `data-br-on-<event>="<nonce>.<n>"`, with a random nonce made for that one response, and one more inline script (also named by hash) holds the handlers and attaches each to the elements whose marker carries the nonce. It runs once, before the app's bundle and before any agent frame can arrive, and removes the markers as it reads them. A handler runs as a browser runs it: `this` is the element, `event` is the event, names resolve through the element, its form and the document, and `return false` cancels the event.

Code inserted at runtime, a `<script>` or a handler an agent or a data binding adds later, is still blocked. `'unsafe-inline'` and `'unsafe-hashes'` are never sent. `'unsafe-hashes'` was sent until 2026-10, and it let markup injected later (an agent's `html` node, say) run any handler the author wrote, on an element the injection chose: with `onclick="br.chat(this.dataset.q)"` on the page, an injected `<img src=x data-q="…" onerror="br.chat(this.dataset.q)">` sent its own prompt with no click.

What does not run: a handler inside a `<template>` (a clone cannot carry the listener, and binding clones later would bind injected markup too; wire those in code), a handler the binder script cannot hold safely (one that does not parse, or contains `</script` or `<!--`), a handler added or changed at runtime, and a `javascript:` URL in `href`, `src` or `action`, which the lint harness warns about. A static app needs `build_app` only when it has a `src/main.ts`.

The served page gets `<script src="dist/app.js">` only when `dist/app.js` exists and the page does not already load it itself. Agentic apps keep their code in `src/main.ts`, because the SDK (`createApp`, `br.*`) exists only inside the bundle.

Assembly is idempotent: before injecting the theme, the config island, the bundle tag, `<base>` and `data-br-pack`, the daemon removes any copy of them an earlier assembly left in the page. A store entry that an older export overwrote with an already assembled page heals on its next load.

`configure_app` on a static app makes it agentic and adds whichever of `src/sdk.ts` and `src/main.ts` it lacks, leaving `index.html` and an existing `src/main.ts` alone. A blank `kind` is refused rather than read as `static`.

### Checking an app in a real browser

`smoke_app` runs the app in a headless browser against a mock daemon. It is optional: it needs Node.js and Playwright's Chromium on this machine and the harness script from a source checkout, which an installed Biorouter does not ship. When any of them is missing it says which, and that says nothing about the app.

## The manifest

The manifest is `manifest.json`. Every v2 field is optional and defaults to v1 behaviour, so an old manifest deserializes unchanged.

### Manifest blocks at a glance

| Block | What it configures |
|---|---|
| `id`, `title`, `kind`, `entry` | App identity. `kind` is `"agentic"` (default) or `"static"` (a plain page with no agent). |
| `agent` | The agent that backs the app: system prompt, greeting, model, extensions, skills, knowledge base, turn bound, session durability, structured output type. |
| `agent.capabilities` | Deny-by-default capability grants (`ui`, `files`, `data`, `compute`, `vault`, `memory`, `tracing`, `events`) — see the [security model](#security-model). |
| `agent.guardrails` | Goal/scope statements, PII policy, injection checks, and the tools that require approval. Consumed by the turn loop. |
| `agent.reliability` | Tool timeouts and timeout behaviour, parallel tool use, tool-choice reset. Consumed by the turn loop. |
| `agent.orchestration` | `sub_agents` (materialized as agents-as-tools), declarative `workflows`, named model `routes` for `br.call({route})`, and named worker `agents` profiles. |
| `surface` | The typed app surface the agent may address: `state_schema`, `actions` it may call, `signals` it may subscribe to, and custom catalog `components`. |
| `theme` | Theme pack, accent, and custom `--br-*` tokens. |

### Annotated example

Only the Agent Drafter–specific blocks are shown.

```jsonc
{
  "id": "cohort-explorer",
  "title": "Cohort Explorer",
  "kind": "agentic",                 // "agentic" (default) | "static"
  "entry": "index.html",

  // ── The agent that backs the app ──────────────────────────────────────────
  "agent": {
    "system_prompt": "You explore a clinical cohort. …",
    "greeting": "Ask me about the cohort.",
    "model": {                       // omit entirely to inherit the user's provider/model
      "provider": "llamacpp",
      "model": "qwen3.5-4b",
      "settings": { "temperature": 0.2, "max_tokens": 4096,
                    "reasoning_effort": "medium", "verbosity": null }
    },
    "extensions": ["knowledge", "autovisualiser"],
    "skills": ["clinical-biostatistics"],   // advisory scoping — see note below
    "knowledge_base": "trial-kb",
    "max_turns": 48,                 // bound/raise the per-message tool loop (default 24)
    "durable_session": true,         // resumable per-(app,client) session; default true
    "output_type": { "type": "object", "required": ["summary"], "properties": { … } },

    // ── Deny-by-default capability grants (Pillar 4/7) ──────────────────────
    "capabilities": {
      "ui": { "enabled": true, "allow_theme": true, "allow_layout": true,
              "allow_ask": true, "allow_signals": true, "allow_html": false,
              "allow_autorun": false,   // signal-triggered turns; user-granted only
              "max_panels": 12, "ask_timeout_s": 300 },
      "files": { "entries": [{ "name": "data", "local_dir": "/abs/dir",
                               "mode": "ro", "out_dir": false }],
                 "max_file_bytes": 262144 },
      "data": { "sources": [
        { "name": "trial", "kind": "knowledge", "ids": ["trial-kb"],
          "read_only": true },                     // ingest needs read_only:false
        { "name": "ehr", "kind": "omop" }          // sensitive → provider-class gated
      ] },
      "compute": { "sandbox": "docker", "timeout_s": 60, "network": "none",
                   "max_mem": "512m", "cpus": 1.0, "image": null },
      "vault": { "encrypted": ["SPOKEAGENT_PASSCODE"] },
      "memory": { "kb": "trial-kb", "mode": "read_write", "shared_kb": null,
                  "distill": true },
      "tracing": { "enabled": false, "redact": true, "processor": null },
      "events": ["tool", "handoff", "compaction"]  // lifecycle stream → br.on()
    },

    // ── Guardrails / reliability (consumed by the turn loop) ────────────────
    "guardrails": { "goal": "cited risk summary", "business_scope": "…",
                    "pii": "block", "checks": [{ "kind": "injection" }],
                    "needs_approval": ["developer__shell"],
                    "approvals_require_persistence": true },
    "reliability": { "tool_timeout_s": 45, "tool_timeout_behavior": "error_as_result",
                     "tool_use_behavior": { "kind": "run_llm_again" },
                     "parallel_tools": true, "error_to_output": false,
                     "reset_tool_choice": true },

    // ── Multi-agent orchestration ───────────────────────────────────────────
    "orchestration": {
      "sub_agents": {                // SHIPPED: materialized as agents-as-tools
        "stats": { "description": "Biostatistician", "system_prompt": "…",
                   "extensions": ["knowledge"], "skills": [], "max_steps": 8,
                   "max_wall_s": 120 }
      },
      "workflows": { … },            // declarative Tool/Agent step lists
      "routes": {                    // named model routes for br.call({route})
        "fast":  { "provider": "llamacpp", "model": "qwen3.5-4b" },
        "deep":  { "model": "claude-opus-4-8" }   // absent field inherits session's
      },
      "agents": {                    // named worker profiles — subset-capped, serialized
        "critic": { "system_prompt": "Refute every claim.",
                    "model": { "provider": "llamacpp", "model": "qwen3.5-4b" } }
      },
      "lazy_tools": false
    }
  },

  // ── The declared app surface (Pillar 1) ───────────────────────────────────
  "surface": {
    "state_schema": { "type": "object", "properties": { "selection": { … } } },
    "actions": [                     // verbs the AGENT calls via app_call
      { "name": "focus_node", "description": "Center + select a node",
        "params": { "type": "object", "properties": { "id": { "type": "string" } },
                    "required": ["id"] } }
    ],
    "signals": [                     // app→agent notifications (ui_subscribe)
      { "name": "node_selected", "payload": { "type": "object" }, "coalesce_ms": 250,
        "autorun": false }           // opt in (true) to let this signal start a turn (needs allow_autorun)
    ],
    "components": [                  // custom catalog kinds the app registers
      { "name": "pathway_map", "props": { "type": "object" } }
    ]
  },

  // ── Theme (Pillar 6) ──────────────────────────────────────────────────────
  "theme": { "pack": "clinical", "accent": "#2563eb",
             "tokens": { "--br-radius": "6px" } }
}
```

### Manifest field notes

These come from `manifest.rs`.

- `capabilities.ui` is **on by default** (all sub-switches default on except `allow_html` and `allow_autorun`, which are off). `max_panels` default 12; `ask_timeout_s` default 300.
- `allow_signals` lets the agent **listen** to declared `surface.signals`; it does **not** let a signal act. `allow_autorun` (default off, user-granted only — the agent can never self-grant) additionally lets a signal *start a turn*, and only when that signal opts in via `surface.signals[].autorun: true` and the server's autorun budgets hold (6/min, 60/session). Without it every signal is queue-only (context for the next turn).
- `DataSource.ids` scopes a `kind:"knowledge"` source to specific KB id(s). Empty `ids` grants **nothing** by itself — the only exception is a back-compat implicit single grant of the agent's configured `knowledge_base`. A KB id not enumerated here is denied even if it exists (see the [security model](#security-model)). `read_only` defaults `true`; setting it `false` grants write (`br.kb.ingest`).
- `ModelRoute` (at `orchestration.routes`) has optional `provider` + `model`; an absent field inherits the session's current value. Routes resolve against the *user's* configured providers only (apps never carry keys) and are subject to the provider-class rule in the [security model](#security-model).
- Skills scoping is advisory — the named skills are surfaced to the agent, but BioRouter's skill enable/disable is global, so true per-app skill isolation is a follow-up.
- `Capabilities.events` (agent→app lifecycle, advertised as `event:<name>`) is distinct from `surface.signals` (app→agent); the two channels never collide.
- `app_uid` is server-managed, like `created_at`, `sdk_hash` and `session_id`: a random id minted when the app is created, and restored from disk by `update_app` whatever a rewritten manifest says. It is part of the key of the app's durable conversations (`app:<id>:<app_uid>:<client id>`), so an app deleted and re-created under the same id, or replaced through `create_app`, starts with none of the old app's conversations or shared state. An app created before the field existed has none and keeps the key `app:<id>:<client id>`, so its conversations still resume.
- `kind` also reads `"agent"` as `"agentic"`, and writes it back as `"agentic"`.

### Theme packs

`THEME_PACKS` are `biorouter` (base look, no overrides), `clinical`, `lab-notebook`, `terminal`, `journal`, and `midnight` — each a `[data-br-pack]` token layer in `templates/theme.css` with a dark variant. An unknown pack name resolves back to `biorouter`.

`theme.accent` and `theme.tokens` are sanitized at render time: keys must be `--br-*` custom properties (≤48 chars), values ≤64 chars with no `;{}<>()"'\/` (so no `url(...)`/`color-mix(...)` breakout). Unsafe entries are silently dropped.

## Client API reference

`import { createApp } from "./sdk"` and call `createApp(overrides?)`. It merges `{ appId: "app", autoChat: true, ui: true }` with `window.BIOROUTER_APP_CONFIG` (injected into the served page) and your `overrides`, constructs the client, assigns it to `window.BioRouter`, and on DOM-ready connects the socket (and auto-mounts a chat panel into `[data-br-chat]` when `autoChat`). The return value is the `br` client.

```ts
const br = createApp({ autoChat: false });  // false → you build your own UI
```

`AppConfig`: `appId`, `endpoint?`, `endpoints?` (ordered fallbacks), `greeting?`, `autoChat?`, `ui?` (default on), `theme?` (`"light"|"dark"|"auto"`, unset→light), `wsToken?` (per-app auth token minted into the page — appended to the WS URL).

> **Note.** `br.state`, `br.actions`, and their siblings are getters returning small
> method objects; `br.ui` is a field. `br.agent(name)` returns a facade scoped to a
> declared worker profile.

### `br.state` — shared reactive state document

One JSON document per app session; both the app and the agent write it. Paths are RFC-6901 JSON Pointers.

```ts
br.state.get(path?)                  // deep clone of the pointer value, or whole doc
br.state.set(path, value)            // optimistic local apply + state_write frame
br.state.remove(path)                // optimistic remove + state_write
br.state.update(fn)                  // fn(draft) → next doc; whole-doc write
const unsub = br.state.subscribe(path, (value) => { … })   // fires on change; returns unsub
```

`set`/`remove`/`update` capture `baseVersion` (the pre-write version), apply locally for instant feedback, then send a `state_write` frame; the server is the ordering authority and rebroadcasts an authoritative patch (or a snapshot on version conflict). Bind HTML declaratively instead of writing to the DOM by hand:

```html
<span data-br-bind="/cohort/count"></span>        <!-- textContent on change -->
<a   data-br-bind-attr="href:/report/url"></a>     <!-- allowlisted attrs only -->
<div data-br-bind-show="/panel/open"></div>        <!-- el.hidden = !value -->
```

### `br.actions` — verbs the agent calls

```ts
br.actions.register("focus_node", (args) => { …; return { ok: true }; });  // sync or Promise
br.actions.list()                    // string[] of registered names
```

The agent invokes a declared action with the `app_call` tool; your handler's return value resolves that tool call. Every registered name must appear in `surface.actions` (lint enforces it). Results serialized over **64 KB** are truncated to `{ truncated: true, text }`.

### `br.signals` — app-to-agent notifications

```ts
br.signals.emit("node_selected", { id, type });   // declared name; trailing-edge coalesced
br.signals.declared()                // SignalDecl[] the ready surface advertised
```

Emits are coalesced per name (window = the signal's `coalesce_ms`, default 250 ms; `≤0` = immediate). The agent only receives a signal it has `ui_subscribe`d to. Selecting a `network` node auto-emits `node_selected {id, instance}` when declared.

### `br.call` — typed agent turns with structured results

```ts
br.call(name, args?, opts?)          // positional form
br.call({ name, args, text?, outputSchema?, debounceMs?, supersede? })  // object form
// resolves: { value } | { text } | { superseded: true }
```

```ts
const r = await br.call("rank_genes", { cohort, top: 10 });   // typed turn
if (r.value) render(r.value);
```

Resolves `{ value }` when the agent finishes by producing an `output` frame (driven by `outputSchema` via the synthetic `emit_result` tool), `{ text }` when the turn ends without one (prose fallback), or `{ superseded: true }` when a newer superseding call on the same key replaces it (a `cancel` is sent for the stale turn). Use `debounceMs`/`supersede` for slider-driven UIs so you never queue one model call per pixel.

### `br.components` — custom catalog kinds

```ts
br.components.register("pathway_map", {
  props: PathwayMapSchema,                      // must match the manifest declaration
  mount(el, props, ctx) { /* author renderer; props are UNTRUSTED */ },
  update(el, props, prev) { /* optional; else re-mount */ },
});
```

The agent then composes a `{ t: "component", name: "pathway_map", props }` node like any built-in. `ctx` is `{ id, state, run }`.

> **Warning.** Render props via `textContent`, never `innerHTML` — props are
> agent-controlled. A throwing or unregistered component degrades to a neutral
> placeholder plus one `ui_error`.

### `br.kb` — knowledge bases

```ts
br.kb.search(query, { limit?, timeoutMs? })      // default timeout 30 s
br.kb.page(path, { timeoutMs? })
br.kb.graph({ timeoutMs? })                      // → feeds graphToNetwork
br.kb.history(limit?, { timeoutMs? })
br.kb.ingest(items, { onProgress?, timeoutMs? }) // write; default timeout 600 s
br.kb.graphToNetwork(graph)                       // pure client-side {nodes,edges} → NetworkSpec
```

Each op sends a `kb` frame and resolves on the matching `kb_result` (rejects on its `error`); `ingest` streams `kb_progress` to `onProgress`. If no KB grant was advertised in `ready`, reads and writes reject immediately with a clear error (no round-trip). Grants and the write rule are enforced server-side — see the [security model](#security-model).

```ts
const spec = br.kb.graphToNetwork(await br.kb.graph());   // a KB explorer in a few lines
br.ui.apply({ cmd: "render", target: "@region:graph", body: [{ t: "network", id: "g", spec }] });
```

### `br.model` — provider routing

```ts
br.model.list()                      // Promise<providers[]> from GET /apps/<id>/models?token=<page token>
br.model.select(provider, model)     // live-switch the session model
br.model.status(timeoutMs?)          // → { provider, model, ready, detail } (default 10 s)
```

`status()` is how you show a "llamacpp is downloading 42%" affordance. Per-turn model routing goes through `br.call({ route: "deep" })` against a manifest `orchestration.routes` entry, subject to the provider-class rule.

### `br.context`, `br.widgets`, top-level turns and events

```ts
br.context.tokens()    // → { used, limit, ratio }
br.context.history()   // → [{ role, text }]

br.widgets.render(id, tree, target)              // render a WidgetNode tree yourself
br.widgets.action(widgetId, action, payload?)    // (alias: submit) → widget_action frame
br.widgets.get(id)                               // last server-sent WidgetNode for id

await br.run(prompt, "#out", opts?)  // stream markdown + a run-status timeline into a target → full reply string
await br.prompt(text, opts?)         // fire a turn; resolves on `done` (opts: images, debounceMs, supersede)
const text = await br.ask(text)      // collect the whole reply as a string
br.cancel()                          // cancel the in-flight turn

br.on("message", (ev) => { … })      // low-level event stream (see the AgentEvent frames)
br.off(kind, fn)
br.has("data")                       // was a capability advertised in `ready`?
br.sendRaw(frame)                    // escape hatch: send an arbitrary frame
br.approve(requestId, action?)       // HITL: allow a paused tool  (default "allow_once")
br.reject(requestId, reason?)        // HITL: deny a paused tool
```

HITL is human-in-the-loop tool approval: the agent pauses on a tool listed in `guardrails.needs_approval` and the page answers.

Fields: `br.config`, `br.sessionId`, `br.resumed`, `br.activeEndpoint`, `br.ui`.

### `br.ui` — the agent-driven UI runtime, from the app side

```ts
br.ui.onState((state) => { … })      // observe the shared state doc
br.ui.onCommand((cmd) => { … })      // observe every applied ui command
br.ui.regions()                      // [data-br-region] names on the page
br.ui.network(id)?.select("n1")      // NetworkController: select / positions / adopt / destroy
br.ui.network(id)?.positions()       // { nodeId: {x,y} }
br.ui.presence("Scoring variants…")  // show the ambient activity chip
br.ui.presenceText()                 // current chip text
br.ui.apply(cmd)                     // apply a ui command locally (rarely needed)
br.ui.resolveTarget("@region:x")     // → HTMLElement | null
```

### `br.agent` — worker profiles

An app may declare named **worker profiles** in `orchestration.agents` — each a full alternate `AgentConfig` (its own model, prompt, extensions, KB) validated server-side to be a capability subset of the app, and subject to the provider-class rule. The daemon advertises the survivors in `ready.profiles` (cap `MAX_PROFILES = 8`). `br.agent(name)` returns a facade whose turns run on that worker; `br.agents()` lists the declared names.

```ts
const critic = br.agent("critic");          // rejects if "critic" isn't declared
const r = await critic.call("review", { claim });   // turn runs on the critic profile
critic.on("message", (ev) => { … });         // events filtered to this profile
// also: critic.prompt(text), critic.ask(text), critic.run(text, target)
br.agents();                                  // → declared profile names (from ready.profiles)
```

Each facade method stamps `agent: name` on its outgoing `prompt`/`call` frame; the server runs that worker's own session and turn loop. The main agent can also delegate to a profile mid-turn with the `consult` tool.

> **Partial — serialized, not parallel.** Cross-profile turns are serialized: only
> one worker (or the main agent) runs at a time on the app socket. Parallel turns
> across profiles are a stretch goal, not in this build. `consult` depth is 1 (a
> consulted profile cannot itself consult), and workers get no `ui_*` control
> unless the profile opts in. This is an actively-landing feature: treat the code
> on `main` (`validate_profiles` / `WorkerHandle` in `apps.rs`, `consult` in
> `control.rs`, `AgentFacade` in `sdk.ts`) as authoritative for its exact current
> shape.

## Agent-driven UI tools

Every agentic app is granted the `appcontrol` in-process MCP server, whose tools push command frames down the app's own WebSocket. There are **18** core agent-facing tools, plus a conditional `consult` armed only when the app declares worker profiles. Each mutation returns the assigned node ids so the agent can target them later with `ui_patch`.

| Tool | What it does |
|---|---|
| `ui_describe` | Report the live surface: author regions, element ids, mounted panels, instance registry (id→kind), shared-state version+keys, the declared surface, current subscriptions, and which sub-capabilities are allowed. Call it **first**. |
| `ui_panel` | Mount / replace / remove a panel (widget tree). `place`: a dock slot (`dock`/`left`/`right`/`bottom`/`main`/`modal`) or a target (`@region:x` / `@panel:x` / CSS). Re-using an `id` replaces in place. Oldest panel is evicted past `max_panels`. |
| `ui_render` | Render widget nodes into an existing target; `mode` `replace` (default) or `append`. |
| `ui_chart` | Bar/line/pie chart; single-series `{type,title,data:[{label,value}]}` or multi-series `{type,series:[{name,data}]}`. Omit `target` → dock panel. |
| `ui_graph` | Node/edge graph `{title,nodes:[{id}],edges:[{source,target,label}]}`. |
| `ui_highlight` | `outline` (default) / `pulse` / `focus` / `clear`, optional `note`, `scroll` (default true). |
| `ui_theme` | Switch `pack` / `accent` / `mode` (`light`/`dark`/`auto`) / `density` (`comfortable`/`compact`). Gated by `allow_theme`; accent is sanitized. |
| `ui_layout` | A `preset` (`single`/`sidebar-right`/`sidebar-left`/`split`/`dashboard`) or an `areas` grid (≤4×4) + `sizes`. Gated by `allow_layout`. |
| `ui_notify` | Transient toast: `message`, `level` (`info`/`success`/`warn`/`error`), `timeout_ms` (default 4000; 0 = sticky). |
| `ui_state` | Merge/read the shared state doc by top-level key (`set` / `remove`). No-ops (and says so) when nothing changed. |
| `ui_patch_state` | Apply an RFC-6902 JSON Patch to the state doc (≤64 ops). Prefer this for nested edits. |
| `ui_patch` | Incrementally edit the UI by node id (≤32 ops): `add` / `replace` / `set_props` / `remove`. Preserves scroll/focus/input. |
| `ui_html` | Render **server-sanitized** rich HTML (≤64 KB). Gated by `allow_html` (default off). Scripts/styles/forms/iframes/`on*`/unsafe-URL are stripped fail-closed in `control.rs`. |
| `ui_figure` | Render a publication-grade Auto Visualiser figure — `tool` (e.g. `render_volcano`, `render_kaplan_meier`, `render_dashboard`) + that tool's `args` — into a sandboxed frame (see [figures in an app](#figures-in-an-app)). |
| `ui_ask` | Render a form and **block the tool call** until the user submits; the result *is* their answers. Gated by `allow_ask`; `fields` ≤24; times out per `ask_timeout_s`. |
| `ui_suggest` | Offer up to 5 **non-blocking** suggestion chips (next steps the user can tap or ignore) — `chips:[{label ≤80, prompt? ≤500}]`, optional `target`. Core `ui` (no capability). Unlike `ui_ask` it never blocks the turn. |
| `app_call` | Invoke a declared `surface.actions` verb; `args` validated against its schema. Blocks up to 60 s for the app's registered handler. |
| `emit_result` | Deliver a structured result for a `br.call({outputSchema})` the app is awaiting; validated against the output schema; sends an `output` frame. |
| `ui_subscribe` | Replace the set of subscribed `surface.signals`. Gated by `allow_signals`. |
| `consult` | Ask a declared worker profile to independently answer a self-contained sub-question and return its answer. **Main agent only, depth 1**; armed only when the app declares ≥1 valid profile (else a friendly no-op). Blocks up to `CONSULT_TIMEOUT_S` (120 s). |

### Widget catalog

A node's kind is its `t` value. Generic nodes any tree may emit (`WIDGET_KINDS`, validated server-side):

`card`, `row`, `col`, `text`, `badge`, `table`, `chart`, `graph`, `stat`, `divider`, `input`, `select`, `checkbox`, `button`, `form`, `progress`, `markdown`, `image`, `kpi`, `log`, `plot`, `network`, `component`.

Privileged nodes (`PRIVILEGED_WIDGET_KINDS`) — only the dedicated tool may build them, after sanitizing or rendering: `html` (via `ui_html`), `figure` (via `ui_figure`). A generic tree carrying `html`/`figure` is rejected as a fixable error.

Selected node shapes: `stat {label,value,unit?,delta?}`, `kpi {label,value,delta?,unit?}`, `log {lines:[{level?,text}],max?}`, `plot {spec:{type: scatter|area|box|heatmap|bar|line|pie, …}}`, `network {spec:{nodes:[{id,label?,type?,size?}],edges:[{source,target,kind?,label?}],encoding?,physics?}}`, `button {label,action,submit?}` (submit collects form fields).

An **unknown** kind renders a neutral `[unsupported: <kind>]` placeholder (warned once per kind), never an error card.

### Render targets

`@region:<name>` (an author `data-br-region`), `@panel:<id>`, `@chat` (`[data-br-chat]`), `@main` (`[data-br-main]`/`main`/`.br-container`/`body`), or a CSS selector like `#out`. Panels also accept the dock slots `dock`/`left`/`right`/`bottom`/`main`/`modal`.

### Patch operations

```jsonc
{ "op": "add",       "id": "kpi-1", "target": "@region:results", "node": {…}, "parent"?: "id", "index"?: 0 }
{ "op": "replace",   "id": "kpi-1", "node": {…} }
{ "op": "set_props", "id": "kpi-1", "props": {…} }   // shallow-merge
{ "op": "remove",    "id": "kpi-1" }
```

Ids are ≤64 chars; the whole batch validates against a clone and commits only if every op is valid (a rejected batch leaves the page untouched).

## Protocol appendix

The app talks to `biorouterd` over one WebSocket at `GET /apps/<id>/agent`. The protocol is versioned: the server opens with a `ready` frame (`protocol: 2`) that advertises capability tokens; the client feature-detects via `br.has(token)`. Every server-issued `ui` command frame carries `type:"ui"` plus `v` (the `CATALOG_VERSION`, currently `1`).

**Forward and backward tolerance:** an unknown `ui` `cmd` is ignored; an unknown widget kind renders a placeholder; an old bundle simply drops v2 frames it does not understand until rebuilt.

### Server-to-client frames

| `type` | Fields | Meaning |
|---|---|---|
| `ready` | `protocol` (2), `capabilities` [tokens], `sessionId`, `resumed`, `messageCount`, `catalogVersion`, `stateVersion`, `profiles` [worker-profile names], `surface:{ signals:[{name,coalesceMs}], actions:[name] }` | Sent on connect. `capabilities` = `manifest.agent.capabilities.advertised()` (retained by daemon settings for `vault`/`tracing`); `profiles` = the validated `orchestration.agents` names. |
| `message` | `delta` | Assistant text stream. |
| `thought` | `delta` | Thinking stream. |
| `tool` | `name`, `id`, `status` (`pending`/`completed`/`failed`) | Tool activity. |
| `output` | `callId?`, `value`, `schema?` | Structured result for a `br.call({outputSchema})` (from `emit_result`). |
| `done` | — | Turn finished. |
| `error` | `message` | Turn/session error. |
| `context` | `used`, `limit`, `ratio` | Reply to a `tokens` request. |
| `history` | `messages:[{role,text}]` | Reply to a `history` request. |
| `model` | `ok`, `provider`, `model`, `route?`, `error?` | Reply to `modelselect` / route switch. |
| `model_status` | `provider`, `model`, `ready`, `detail` | Reply to `model_status`. |
| `guardrail` | `stage`, `name`, `blocked`, `reason` | e.g. PII input check. |
| `approval` | `requestId`, `tool`, `args`, `prompt` | HITL pause; answer with `approve`/`reject`. |
| `kb_result` | `reqId`, `result` \| `error` | Reply to a `kb` op (capped ~1 MB). |
| `kb_progress` | `reqId`, `stage`, `detail?`, `pct?` | `ingest` progress. |
| `ui` | `v`, `cmd`, … | A UI command: `panel`/`render`/`patch`/`highlight`/`theme`/`layout`/`notify`/`state`/`suggest`/`ask`/`ask_close`; plus `app_call` (dispatched to an action handler). |

The SDK also surfaces v2 lifecycle events on `br.on(...)` when advertised: `usage`, `tool_call`, `handoff`, `compaction`, `trace` (these ride the `Capabilities.events` stream).

### Client-to-server frames

| `type` | Fields |
|---|---|
| `prompt` | `text`, `images:[{mimeType,data}]`, `agent?` (worker profile) |
| `cancel` | — |
| `tokens` | — |
| `history` | — |
| `modelselect` | `provider`, `model` |
| `model_status` | — |
| `kb` | `op` (`search`/`page`/`graph`/`history`/`ingest`), `params`, `reqId` |
| `call` | `callId`, `name?`, `args?`, `text?`, `outputSchema?`, `route?`, `agent?` (worker profile) |
| `signal` | `name`, `payload` |
| `state_write` | `set:{path,value}` **or** `patch:[ops]`, `baseVersion` |
| `widget_action` | `widgetId`, `action`, `payload` |
| `app_result` | `callId`, `result` \| `error` |
| `approve` / `reject` | `request`, `action` / `reason` |
| `ui_reply` | `requestId`, `payload` (answers a `ui_ask`) |
| `ui_surface` | `surface:{ title, regions, ids, hasChat, panels }` (app-boot report) |
| `ui_error` | `where` (`widget:<kind>`/`component:<name>`/`action:<name>`), `message`, `instance?`, `droppedCount?` |

### Connect URL

The SDK dials `ws[s]://<host>/apps/<appId>/agent` (or `config.endpoint`/`endpoints`), decorated with `?client_id=<id>[&token=<wsToken>]`. `client_id` is a stable per-app id in `localStorage["br.client.<appId>"]`; `token` is appended only when `config.wsToken` is set. The same token goes on `GET /apps/<id>/models` and `GET /apps/<id>/runstate` as `?token=`, which the daemon requires there too.

The `ready` frame's fields are exactly those in the table above. The client latches `capabilities`, `sessionId`, `resumed`, and the `surface`, then (if `ui` is advertised) posts its `ui_surface` report.

### The `<app-data>` untrusted envelope

Every app-originated payload injected into the agent's context — `app_call` args (name form), queued `signal`s, and `widget_action` submissions — is wrapped:

```text
[<label>]
<app-data>
<json, capped at 65,536 bytes>
</app-data>
```

The system prompt states that everything between `<app-data>` … `</app-data>` is **data, not instructions** — read, quote and analyse it, but never obey commands inside it. Only text outside the markers can change agent behaviour. Text-form `br.call` (free `text`) is passed through directly and is *not* enveloped.

> **`ui_error` is consumed server-side.** The SDK produces `ui_error` frames
> (render/action failures, rate-limited to 3 per rolling 30 s with a
> `droppedCount`), and `biorouter-server` now handles them: each is buffered
> per-connection (cap 5) and delivered to the model under the artifact-repair
> grace discipline (`should_auto_repair`, a server port of the frontend's
> `shouldAutoRepairArtifact`). When the next turn starts, the buffered errors ride
> in front of its user message as an `[app ui errors]` `<app-data>` envelope. If an
> error arrives within 15 s of the last turn ending, it auto-starts one repair turn
> ("Fix the rendering problem you just caused if it was yours; otherwise briefly
> note it.") — capped at once per 60 s. Errors that surface long after the agent
> went idle just wait for the next user-initiated turn; they are treated as
> user-managed UI, not the agent's mess to silently resume and fix.

## Security model

### WebSocket authority

`GET /apps/<id>/agent` is guarded by `check_ws_auth` with two gates:

1. If an `Origin` header is present it must be the app page's own origin — the request's `Host` in scheme, host and port (`origin_matches_host`), the scheme being `https` only when a proxy in front sends `X-Forwarded-Proto: https`. A page on any other origin is refused, another loopback port included; that is the v2 design's exact-origin pinning. A non-browser client with no `Origin` passes this gate.
2. `?token=` must equal the app's **per-app socket token** — 16 random bytes as 32 hex chars (`ws_token_for`), minted lazily into the served page, kept in memory **per daemon run** (never on disk).

Before either gate, the daemon's own authentication must pass (W2-HRD-1). A browser tab cannot send the secret-key header, so an app's page, bundle, assets, model catalog, run state and socket accept instead that app's **access cookie**: `POST /apps/<id>/launch`, which needs the secret, answers a one-time link (`/apps/<id>/?t=<token>`, good once and for five minutes), and opening it sets an `HttpOnly`, `SameSite=Strict` cookie scoped to `/apps/<id>` and lands on the page.

The launch answer names the link twice. `path` is the page alone, for a caller whose browser already reaches the daemon at the caller's own address: a `biorouter serve` tab, and every client older than `url`. `url` is the same page on the daemon's own listener (`http://127.0.0.1:<port>/apps/<id>/?t=<token>`), with a daemon listening on every interface named by its loopback address (`0.0.0.0` becomes `127.0.0.1`, `[::]` becomes `[::1]`). It is left out when the daemon has published no listener. The desktop needs `url`: it reaches a shared daemon through a local proxy that admits only requests carrying the desktop's secret, and a link on the proxy's address answered the browser with an empty 403 ("Access to 127.0.0.1 was denied"). The proxy is deliberately not opened to app traffic, because it adds the daemon's secret to everything it forwards. The desktop and `biorouter apps open` send the browser to `url`, and only to a loopback `http` address on an explicit port; the desktop keeps an external backend's link on the address it was configured with, because that may be a tunnel whose far end is another machine's loopback. Without the cookie or the secret, every `/apps/*` request gets the same 401 whether or not the app exists. An `Origin` header is never treated as proof. On a `biorouter serve` daemon the served interface's own cookie opens its apps too. Mutating verbs (`POST /build`, `POST /vault`, `DELETE`), `GET /apps` and both export routes (`GET` and `POST /apps/<id>/export`) still require the secret.

A page from a previous daemon run reconnects with a stale cookie and token and is refused: open the app again from Biorouter for a new link.

### One origin for every app

Every app is served from the daemon's one origin, and on `biorouter serve` so is the shell that holds the daemon's secret. The access cookie's `Path=/apps/<id>` decides which requests the browser attaches it to; it is not a boundary between pages of one origin, so app A's script that requests `/apps/B/...` gets B's cookie attached. Until this was closed, A could `fetch('/apps/B/')`, read B's socket token out of the page and drive B's agent, or frame B and read it through the DOM. Four rules now hold:

- On a browser credential (the access cookie, a launch link, the serve interface's cookie), an app's page is served only to a document navigation: `Sec-Fetch-Mode: navigate` with `Sec-Fetch-Dest: document`. A `fetch`, an `<iframe>`, an `<object>` or a `<script src>` of the page gets the refusal page with a `403`. Requests without Fetch Metadata remain accepted on loopback for non-browser clients. Explicit secret-header callers are unaffected. The bundle, assets, socket and figure frame are not pages and load as before at a supported address.
- An app's page cannot be framed, and a page that opens it with `window.open`, another app's included, gets a closed handle ([the app page's own policy](#the-app-pages-own-policy)).
- `GET /apps/<id>/runstate` and `GET /apps/<id>/models` want the page's token as `?token=` as well as the cookie, because only the app's own page holds it. A page built before the SDK sent it is rebuilt on its next load, since the SDK fingerprint changed; until then `br.model.list()` answers `[]`.
- The serve shell is refused to anything but a navigation, cannot be framed, and severs a popup's handle with `Cross-Origin-Opener-Policy: same-origin` ([SD-14](../deployment/serve-decisions.md)).

⚠ The first, second and fourth rules hold only where the browser treats the address as secure: a loopback address, `localhost` or a name under `.localhost`, or `https` through a proxy that sets `X-Forwarded-Proto: https` and preserves Fetch Metadata. Anywhere else a browser sends no Fetch Metadata and ignores opener policies, so every request reads as a navigation. Consequently `biorouter serve --host 0.0.0.0` opened at its plain-HTTP LAN address refuses both the secret-bearing interface (including its token exchange) and every app surface on browser credentials. This also protects the interface from an app tab left open during an update. A forged `X-Forwarded-Proto` without browser-controlled Fetch Metadata is refused. The `403` page gives TLS or SSH guidance. Explicit secret-header API callers continue to work.

Measured in Chrome: from one app's page, a `fetch` of another app's page, of the shell or of another app's `models` route is refused; a frame of either gets the refusal page; and a popup of either opens, but the opener's handle is closed. What is still open: a browser that does not know `noopener-allow-popups` treats it as `unsafe-none`, and there a popup of another app's page still hands its opener a readable handle; a browser that sends no Fetch Metadata at all (Safari before 16.4) is served another app's page on a `fetch`. Origin storage is shared too, so one app can read another's `localStorage` (the SDK keeps only the client id there, which opens nothing without the page token). Serving each app on an origin of its own (`http://<id>.localhost:<port>`, routed by `Host`) closes both and is the planned follow-up. Until it lands, open apps you did not build yourself in a browser profile of their own.

### The app page's own policy

An app's page is served with `Content-Security-Policy` `default-src 'none'; script-src 'self'; …; frame-ancestors 'none'`, plus the hashes of its own inline scripts and of the script that binds its handlers ([static apps and inline scripts](#static-apps-and-inline-scripts)). `frame-ancestors` was `'self'`, which let one app's page frame another's; nothing frames an app page on purpose, because the desktop preview loads it in a view of its own and a browser opens it in a tab. The app's `dist/` and `assets/` files keep `frame-ancestors 'self'`, so an app may frame its own static documents; they hold no socket token.

The page also carries `Cross-Origin-Opener-Policy: noopener-allow-popups`. A browser that knows the value puts the page in a browsing-context group of its own whenever anything opens it, even a page of the same origin, so another app's `window.open('/apps/<id>/')` gets no handle to read it through. It is deliberately not `same-origin`, which the serve shell uses: two documents with the same value on one origin share a group, so an app could script a shell it opened. A browser that does not know the value treats it as `unsafe-none`. App pages and the build-failure page are sent with `Cache-Control: no-store`, because they carry this daemon run's socket token.

### Figures in an app

An Auto Visualiser figure (`ui_figure`, or a `ui://` figure a tool returns while the app's agent has `autovisualiser`) is drawn in a frame that loads `GET /apps/<id>/figure-frame`. That route serves a small shell under its own policy, the same one the desktop's artifact panel uses for figures plus `sandbox allow-scripts` and `frame-ancestors 'self'`: inline scripts run, nothing reaches the network, and the figure has an opaque origin, so it can read none of the app's cookies, storage or pages. The shell tells its parent it is ready, receives the figure's HTML by `postMessage`, and replaces itself with it. The frame then grows to the height the figure reports (`ui-size-change`, between 120 and 1200 pixels); it read a field no figure sends until 2026-10, so every figure stayed clipped at 360 pixels. The figure is sent each time the shell says it is ready, not once: a frame moved in the DOM reloads (`ui_layout` moving a panel does), and the reloaded shell stayed empty when only the first ready was answered.

It used to be a `srcdoc` frame. A `srcdoc` document inherits its parent's policy, and an app page allows no inline script, so every figure drew nothing, its own error card included. A figure a tool returns in CDN mode (the desktop's default) has its library tags replaced with the vendored copies before it is sent, because the figure policy allows no remote host.

The figure frame is admitted by the app's access cookie like the bundle. ⚠ The desktop's in-app preview filters its requests by path, and must allow `figure-frame` beside `models` and `runstate` for figures to draw there.

### Deleting, re-creating and broken apps

- `DELETE /apps/<id>`, the `delete_app` tool, `create_app` replacing an app, and a factory reset of applications all remove the folder through the store, and the daemon then forgets what it held for the app: its socket token, its access cookie and unredeemed launch links, and its vault key (`brsdk_vault_key_<id>`, removed only when present). Every agent socket the app still has open is closed, as if its page had gone away, so a running turn is cancelled: its agent's file, data and compute tools are bound to the folder's path, which the next app built under the id takes over. A page of the deleted app left open loses its connection, its token no longer opens a socket, and its cookie no longer opens anything (on `biorouter serve` the interface's own cookie still opens whatever app is built under that id next, as a fresh page with a new token). Its durable conversations stay in the session store; a new app under the same id never resumes them, because its `app_uid` differs. The id is matched exactly: on a case-insensitive disk, `Dashboard` does not delete `dashboard`.
- Socket retirement covers setup and awaited work as well as the idle and reply loops. Turn cancellation follows the app's lifetime, and connection cleanup detaches the existing UI bridge rather than replacing it, so reinstalling an export with the same `app_uid` can reuse its cached agent. Page tokens are bound to that manifest uid too: a late response from a deleted app cannot authenticate its replacement. Checking an unknown id allocates no cached token or socket lifetime.
- `create_app` with an explicit `id` that is taken replaces the app there only when the same chat created it. An app another chat created is refused with `INVALID_PARAMS` unless the call passes `replace: true`, and replacing deletes that app, its workspace data included.
- An app whose `manifest.json` cannot be loaded is listed instead of hidden. `GET /apps` returns it as a row with `id`, `title` (the id again), `kind: "static"` and a `broken` sentence naming the problem; `list_apps` lists it unfiltered. Such an app can be deleted and nothing else. A folder with no manifest counts as broken only after 30 seconds, because `create_app` writes the manifest just after the folder.

### Sanitization

`ui_html` sanitizes fail-closed server-side (scripts, styles, forms, iframes, `on*` handlers, and non-https/mailto/relative URLs stripped) before the frame leaves the daemon.

State bindings are a non-executing sink: `data-br-bind` writes `textContent` only; `data-br-bind-attr` uses a strict allowlist (`href/src/title/alt/value/placeholder/disabled/hidden/class` + `aria-*` + `data-*`), refuses all `on*`/`style`, and validates URL schemes (no `javascript:`/`data:` for href/src). Custom-component props are agent-controlled and must be rendered via `textContent`.

### Scoped KB grants

`br.kb` resolves a target against `capabilities.data.sources[kind:"knowledge"]`:

- No knowledge source → error ("add a capabilities.data.sources entry …").
- A target listed in some source's `ids` → granted.
- Empty `ids` on **every** knowledge source → grants nothing, except the back-compat implicit single grant of the agent's configured `knowledge_base`.
- A target not among the enumerated `ids` → denied even if the KB exists.
- `ingest` additionally requires `read_only == false` on the granting source (a cross-session integrity decision); reads (`search`/`page`/`graph`/`history`) do not.

### Provider classes

`provider_class(name)` returns `Local` (`llamacpp`/`ollama`/`lmstudio`/… or substring `local`), `Institutional` (`azure`/`bedrock`/`databricks`/`vertex`/`sagemaker`/… or substring `institution`), else `External`.

An app "holds a sensitive data source" when it has an `omop`/`cdw` source **or** a writable (`read_only:false`) `knowledge` source. A sensitive app may **not** route to an `External` provider: a `br.call` route resolving to one is rejected, and such routes are warned about at session start (they stay in the manifest but re-reject at call time).

### Payload caps

| Cap | Value | Applies to |
|---|---|---|
| `APP_PAYLOAD_MAX` | 65,536 B | `app_call` result, `emit_result` value, inbound signal payload, `call` args, `<app-data>` json |
| `STATE_MAX_BYTES` | 262,144 B | whole shared-state document |
| `STATE_MAX_PATCH_OPS` | 64 | one `ui_patch_state` / `state_write` patch |
| `STATE_MAX_DEPTH` / `STATE_MAX_KEYS` | 8 / 2,000 | state doc structure (when no schema) |
| `ui_patch` ops | 32 | one `ui_patch` call |
| instance id length | 64 chars | patch/instance ids |
| `ui_ask` fields | 24 | one form |
| `ui_html` input | 64 KB | before sanitization |
| `APP_CALL_TIMEOUT_S` | 60 s | `app_call` parked wait |
| `max_panels` | 12 (default) | simultaneously mounted agent panels |
| kb_result | ~1 MB | one `kb_result` payload |
| queued signals / frames | 10 / 32 | per-connection buffers (oldest dropped) |
| buffered `ui_error`s | 5 | per-connection buffer (oldest dropped) |
| default `max_turns` | 24 | per-message tool loop |
| `ui_error` | 3 / 30 s | client-side rate limit (`droppedCount`) |
| autorun budget | 6 / min, 60 / session | signal-triggered autonomous turns (server-side) |

## Export guide

An export is a folder that runs the app on another computer. There is one implementation (`crates/biorouter-mcp/src/agent_drafter/export.rs`) and three ways in:

- **The `export_app` tool**: `export_app { id, target_dir, replace?, mode?, include?, bundle_daemon? }` writes the folder at `target_dir`.
- **Applications ▸ Export** in the desktop app: the dialog asks for the mode and the payload, then a folder picker opens before anything else happens (Cancel sends nothing). The window's daemon builds the export and answers a zip, which the desktop unpacks into `<picked folder>/<app id>`.
- **A `biorouter serve` browser**: the same dialog, without the bundled-daemon option, downloads `<app id>.zip`. Nothing is written on the server.

Harness ERRORs block an export. The tool never overrides them; the desktop and the browser show the findings and offer **Export anyway**, which only a person can choose.

### Export modes

- **`launcher`** (the default) carries the app and its launchers. It runs against whatever knowledge bases, skills, extensions and providers the recipient already has.
- **`full`** also carries the app's server-side payload under `payload/` and writes `export.json`. Selection is per item, with `include: {"knowledge_bases": […], "skills": […], "extensions": […]}`. A key that is left out means everything the app references; an empty list means none. The desktop dialog always sends all three lists.
  - Knowledge bases the app references: the agent's `knowledge_base`, the ids of every data source of kind `knowledge`, and `capabilities.memory.kb` and `shared_kb`. The dialog lists all of them.
  - Skills: `agent.skills`, minus the skills that ship with Biorouter (`office-word`, `about-biorouter`, the knowledge skills and the others every install seeds). Extensions: `agent.extensions`, minus the built-in ones. Neither is carried or recorded, even when named explicitly: the recipient already has its own copy, and a shipped skill installed over the recipient's under the seeded name could no longer be removed. A shipped skill that was asked for is named in the report's notes.
  - A skill or extension name must be a plain name (`^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`). A path such as `../../.ssh` is refused before it is joined to anything.
  - A skill is looked up in the skills folder (one bundle level down too) or, over HTTP, in the daemon's skill catalog. Its folder must really sit inside the root it was found in. It is copied without following any link, to a bounded depth and size.
  - A knowledge base is carried only when the caller could open it. The tool asks the calling chat's capability; the HTTP route asks the same decision `GET /knowledge/bases/{id}/export` gets (`HttpCaller::reach_knowledge_base`), so a private base needs the desktop's proof of a person, a private stated capability, or a serve operator on a private provider. A base out of reach, a missing base and a missing skill are each left out with a note; the rest of the export still happens.

`bundle_daemon` is `"none"` (the default) or `"current"`, which puts this machine's `biorouterd` under `payload/bin/` (a "fat" export). The tool reads `"all"` as `"current"` with a note; the HTTP route refuses anything else. The launchers use a bundled daemon only when `biorouterd --version` runs on the recipient's computer, so a fat export opened on another platform falls back to an installed daemon.

### What a folder holds

```text
index.html                    # the app's entry page as it was written (manifest.entry)
src/, assets/, other files    # every authored file, byte for byte (images, fonts, wasm)
dist/app.js, dist/app.css     # what the build writes, for any kind of app that has it
manifest.json                 # the app's manifest without session_id
package.json, serve.mjs, README.md
biorouter-launch.sh, run.command, run.sh, run.ps1, run.bat
app-files/                    # an authored file whose name a generated file uses, if any
payload/knowledge/<id>.brkb   # full mode: each knowledge base carried
payload/skills/<name>/        # full mode: each skill carried
payload/bin/biorouterd[.exe]  # bundle_daemon: "current"
export.json                   # full mode, or a bundled daemon
```

- **The entry page is the raw one.** The daemon that serves the installed copy adds the theme, the config island with that run's socket token, and the bundle tag. An export used to ship a page assembled at export time, with a config island that had no token; the SDK read that island first, so an exported agentic app could never reach its agent.
- **Never exported:** the app's `workspace/` folder (its runtime data), any `.vault`, `.git` or `node_modules` folder at any depth, files named like credentials (`.env*`, `*.pem`, `*.key`, `*.p12`, `*.pfx`, `id_rsa*` and the other SSH key names, `.netrc`, `.npmrc`, `.pypirc`, `credentials`), and links. Each omission is listed in the export's report. The store refuses an app whose folder contains a link, so such an app is not exported at all until the link is removed.
- **No chat id.** `session_id` names a chat on the author's machine. On another machine it names nothing, or an unrelated chat.
- **The launchers are executable** (`run.command`, `run.sh`, `biorouter-launch.sh`, and a bundled daemon), in the folder the tool writes, in each zip entry's unix mode, and after the desktop unpacks the zip.

The tool's `target_dir` must be an absolute path to a folder that does not exist or is empty, and it may not be a link. Because a model picks it, it must also be a folder of the user's own: below Desktop, Documents or Downloads (each resolved through links), and never inside Biorouter's configuration folder, the app store, the knowledge store, a hidden folder, or a folder code is imported from by name (`node_modules`, `site-packages`, `dist-packages`). Without that rule, a chat with Agent Drafter and no shell could write files into a Python package folder or `~/Library`. To export anywhere else, the person uses the Applications view, where they pick the folder themselves.

`replace: true` accepts a non-empty folder only when it is an earlier export of the same app (its `manifest.json` names the app and it has `biorouter-launch.sh`) and it holds nothing the export does not write. The README invites editing an export where it is, so an earlier export that has gained a `.git`, `node_modules`, a `package-lock.json` or any other file of the user's own is refused; files a file manager leaves (`.DS_Store`, `Thumbs.db`, `desktop.ini`) do not count. The folder is built beside the target and renamed into place, so a failed export leaves nothing half-written and a replaced export keeps no stale file. The earlier export is never deleted: it is kept beside the new one as `<folder>.replaced-<date>-<time>`, and the tool's answer names it. The desktop asks before replacing a folder that already has files in it, and moves the old folder to the Trash.

### The HTTP routes

`POST /apps/{id}/export` takes a JSON body, every key optional:

```jsonc
{ "mode": "launcher" | "full",
  "include": { "knowledge_bases": [], "skills": [], "extensions": [] },
  "bundle_daemon": "none" | "current",
  "allow_lint_errors": false }
```

It builds the export in a temporary folder of the daemon's own and answers `200 application/zip`, with every entry under one `<id>/` folder, `Content-Disposition: attachment; filename="<id>.zip"`, and the report in `X-Biorouter-Export-Report` (base64url JSON: `mode`, `files`, `bytes`, `kbs`, `skills`, `extensions`, `daemon`, `notes`, `lint_warnings`, `skipped`). A refusal carries a sentence: `400` for a request it cannot read, `404` for no such app, `409` for an app that cannot be exported as it stands, `422` for harness ERRORs (`{"error", "findings"}`), and `500` for a failure reading or writing. Both export routes require the daemon secret, whatever the method.

`GET /apps/{id}/export` is kept for desktops older than `POST`: a launcher export as a JSON map of text files, `{ id, files, skipped_binary, skipped }`, with the raw entry and the manifest without `session_id`. A binary file cannot travel in that map, so it is named in `skipped_binary`. A current desktop that meets a daemon older than `POST` (it answers `405`) falls back to this route, writes the files with the launchers executable, and says that a full export needs an updated background service.

### export.json

`export.json` sits at the top of the folder:

```jsonc
{ "version": 1, "app": "<id>", "mode": "full|launcher",
  "knowledge_bases": [{ "id", "file", "bytes" }],
  "skills": [{ "name", "path" }],
  "extensions": [{ "name", "source": "registry", "note" }],
  "bundled_daemon": { "platform", "arch", "file", "bytes" } | null,
  "required_credentials": [],        // currently empty: not enumerable without the registry
  "runtime_requirements": [] }
```

External extensions are recorded as registry references, not staged as `.brxt` bundles; installed-bundle staging is out of scope in this build.

### Launchers per OS

- **macOS**: double-click `run.command`.
- **Linux and WSL**: `bash run.sh`.
- **Windows**: double-click `run.bat`, which runs `run.ps1`.
- `biorouter-launch.sh` holds the shared steps that `run.sh` and `run.command` source.

A launcher installs the app into the store the daemon reads, starts or reuses a daemon, installs a full export's payload with consent, asks the daemon for the app's one-time launch link, and opens that link in the default web browser. It also prints the link.

- **Where the store is.** The launchers resolve the configuration folder exactly as the daemon does (`paths::config_dir`): `$BIOROUTER_PATH_ROOT/config` when that is set; on macOS and Linux `$XDG_CONFIG_HOME/biorouter` when that is an absolute path, else `~/.config/biorouter`; on Windows `%APPDATA%\UCSF\biorouter\config`, or the `config` folder inside the older Biorouter folder in `%APPDATA%` while that is the only one (the daemon moves it on its own start, and a launcher that installed into a new current folder first would hide the older one's settings behind it; this is rule 1 of `app_folder`, and `run.ps1` and `serve.mjs` apply it the same way). `run.ps1` used to install under `~\.config\biorouter`, which no Windows daemon reads, so every Windows launch failed with "does not serve". A test runs the bash and Node expressions against the Rust resolver.
- **Never over another app.** The launcher installs only the runtime files: the manifest and authored runtime files, including root assets and custom folders. It compares the store's copy with the folder by `app_uid` (or by creation time for an app made before `app_uid` existed):
  - a different app with this id is replaced only on an interactive yes or `BIOROUTER_EXPORT_REPLACE=1`, and is moved to `app-launcher/backups/` in the Biorouter folder, never deleted;
  - this app changed since the export (the author kept working) is opened as it is, with a line saying how to replace it;
  - an older copy is replaced, with a backup kept;
  - a copy with the same `updated_at` is re-synced, which is the edit, build, re-run loop, but only while it is still what this folder was exported from or what this same export folder last installed there. Anything else was changed where it is installed, and is opened as it is, with the same line about `BIOROUTER_EXPORT_REPLACE=1`.
- **How a launcher tells an unchanged copy.** `updated_at` alone could not: only the drafter's own tools bump it, so an edit made in the installed copy with the developer shell or a text editor, or a newer Biorouter refreshing `src/sdk.ts` on a rebuild, left it equal, and the old folder's launcher reverted the edit or brought back the old SDK. So the export records one SHA-256 over the authored runtime files a launcher replaces (`src/sdk.ts` and `dist/` are omitted for an app with `src/main.ts`, because the daemon derives them; `.DS_Store` files and links left out; one `<hash> <path>` line per file, sorted by byte, then hashed), and every launcher computes the same digest over the store before it re-syncs. Each install writes both a hash of the export folder's real path and the installed runtime digest to `app-launcher/installed/<id>`, so an older folder cannot trust another folder's install record. The loop keeps working after the daemon refreshes its SDK; the refreshed SDK is retained. `bash run.sh` needs `sha256sum` or `shasum` for this; without either, a copy with the same `updated_at` is opened as it is. A test holds the bash and Node digests to the export's.
- **A re-sync keeps the store's bundle.** It keeps the store's manifest, which on the author's machine still names the chat the app was built in, and that manifest's build stamp describes the store's own `dist/`. So for an app with `src/main.ts` the folder's `dist/` is not copied, the store's refreshed SDK is retained, and the daemon rebuilds from the synced `src/` when that changed. Authored files or whole subtrees removed from the export are removed from an unchanged installation too. Copying it under the kept stamp could serve an older SDK that the stamp called current, and the daemon would never rebuild it.
- **Payload paths are bash arrays**, so a folder unzipped under a path with a space installs the right skills. A skill already installed under the same name is moved to backups first. A skill that ships with Biorouter is never installed from a payload, even one that carries it.
- **The daemon.** A launcher uses a daemon whose secret it knows: one it started before (port and secret kept, readable by you alone, in `app-launcher/daemon`) or one named by `BIOROUTERD_PORT` and `BIOROUTER_SERVER__SECRET_KEY`. Otherwise it starts one headlessly (via `BIOROUTER_PORT`), waits up to 60 seconds for it, and stops it before trying another port, so no daemon is left running whose secret nobody knows. The daemon's log goes to `app-launcher/logs/`, a folder only you can read.
- **The browser.** The link is written into a page only you can read, and that page is opened in the default web browser (on macOS the `https:` handler through `open -a`, on Linux the `xdg-settings` browser through `gtk-launch`, else `xdg-open`). Handing it to `open` or `xdg-open` alone opened whatever edits `.html` files, often an editor.

`serve.mjs` (`npm start`) is the editing loop. It installs the app under the same rules, starts or reuses a daemon, and proxies `/apps/**` to it, including the agent WebSocket. On page refresh it rechecks the installed runtime digest and safely syncs the folder's edits, retaining independent changes in the installed copy. All app files, including bundles and images, are served through the daemon's authorization and security policy, so an upgraded or rebuilt daemon bundle is the one the browser receives. Only a `Host` of `127.0.0.1:<port>` or `localhost:<port>` is answered, which closes DNS rebinding. `/` redirects to the app's page on the daemon: only the daemon can put the run's socket token into the page. `payload/`, `export.json` and the scripts are never served.

### First-run consent

A full export's launcher lists its knowledge bases and skills and asks `y/N` before installation. `BIOROUTER_EXPORT_YES=1` provides consent for CI and headless runs. All three launchers import knowledge archives through the running daemon's authenticated multipart API, which preserves their privacy and owner markings. Each successful archive has a receipt, so retrying a partial failure does not duplicate successful imports. Matching app declarations follow the allocated knowledge ids while independent recipient edits remain intact. Selected custom skills are installed with backups after knowledge imports succeed; shipped skills are excluded. A content fingerprint marks complete installation, and failed imports leave it unfinished with an explicit retry message. Launcher mode carries no payload.

The in-SDK capability re-consent screen the design describes is not built. `export.json` is the machine-readable record, and its `required_credentials` and `runtime_requirements` are empty because they cannot be enumerated without the BAAM registry.

## Test gates

| Command | Gates |
|---|---|
| `cargo test -p biorouter-mcp --lib agent_drafter::` | store, tools, render, bundler, `control.rs` `ui_*` tools, manifest/theme/surface types |
| `cargo test -p biorouter-mcp --test ui_example_apps` | the example UI apps emit `ui` frames deterministically |
| `cargo test -p biorouter-server --lib routes::apps` | WS frames, mid-turn dispatch, bridge rebind, parked `ui_ask`, KB grants, provider-class routing |
| `node scripts/agent-drafter/ui-control-harness.mjs` | **SDK v2 self-test** — bundles the real `sdk.ts`, mounts it in jsdom against a mock daemon, and asserts state/bindings, `ui_patch`, signals, `app_call`, `br.call`, `br.kb`, `br.model`, theme/layout, presence, and `wsToken`. Needs esbuild + jsdom (`BIOROUTER_ESBUILD_BIN` / `BIOROUTER_JSDOM_DIR`, or a checkout with `ui/desktop/node_modules`). |
| `node scripts/agent-drafter/ui-control-harness.mjs --app <dir> [--port 8899]` | serve a built app for a real browser to drive via `/__emit` + `/__frames` |
| `ui/desktop/scripts/appcheck/check-ui-app.mjs` | drives a real agent and asserts `ui` frames arrive |

Example apps live under `scripts/agent-drafter-apps/examples/ui/` (`install-examples.sh`).

## Related documentation

- [Apps SDK v2 design](v2-design.md) — the nine-pillar rationale behind every surface documented here, including the pieces still design-only.
- [Apps SDK v2 phase roadmap](v2-phase-roadmap.md) — the order in which these features land, and what each phase must prove before it ships.
- [BioRouter Apps platform design](../agent-drafter/apps-platform-design.md) — the subsystem overview of Agent Drafter, and where this reference sits within it.
- [Auto Visualiser capability](../extensions/built-in/auto-visualiser.md) — the `render_*` tools `ui_figure` embeds into an app panel.
- [Agent Drafter 100-app test-drive runbook](../agent-drafter/testing/app-test-drive-runbook.md) — how to exercise an authored app end-to-end in a browser.
