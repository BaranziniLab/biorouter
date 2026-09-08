# BioRouter Apps SDK v2 phase roadmap

> **What this is.** The six-phase implementation plan for Apps SDK v2 — shared state document, catalog v2 with `ui_patch`, typed RPC and signals, `br.kb` and multi-agent profiles, theme packs, and hardening plus standalone export v2. Each phase is independently shippable and mergeable.
> **Status:** Current. Partly executed: written 2026-07-12 (revised after adversarial review), it records the *intended* sequence, not the achieved state. [The SDK reference](sdk-reference.md) is the authority on what actually ships — see the [phase status map](#which-phases-have-landed) below.
> **Audience:** developers implementing the Apps SDK v2 phases.
> **Identifier key.** *Phase 1–6* (with *Phase 4b* as a distinct multi-agent slice) are the units of work referenced from the design and from commit messages. Section references like "design Pillar 9" point into [the Apps SDK v2 design](v2-design.md), which numbers its nine pillars; those numbers are stable and are cited from the code and the reference too.

Every phase ends green — `cargo test -p biorouter-mcp --lib agent_drafter::`, `cargo test -p biorouter-server --lib routes::apps`, `node scripts/agent-drafter/ui-control-harness.mjs`, and a clean `tsc` — and keeps all v1 apps working.

> **Export parity is a per-phase gate** (design [Pillar 9](v2-design.md#pillar-9-app-lifecycle-and-standalone-export)). Each phase's acceptance includes the export smoke: the demo app exported via `GET /apps/{id}/export` and launched by `run.sh` against a headless daemon must exercise that phase's new features — state restore, socket token, `ui_patch`, signals, profiles, theme packs — identically to the panel-served app.

## Terms used throughout

| Term | What it means |
|---|---|
| `ui_patch` | The agent-facing tool that edits mounted UI incrementally by node id (`add`/`replace`/`set_props`/`remove`) instead of re-rendering a subtree. |
| `emit_result` | The synthetic tool injected when a `br.call` supplies an output schema: the model finishes the turn by calling it, and the validated call becomes the app's structured `output` frame. |
| `sdk_hash` | The manifest's fingerprint of the vendored `src/sdk.ts` a bundle was built from. Drift triggers a rebuild, which is how a new runtime reaches existing apps. |
| `check-all.mjs` | The app-corpus regression script under `ui/desktop/scripts/appcheck/`. Its pinned pass count is the v1 baseline every phase must hold. |
| `MAX_QUEUED_FRAMES` | The existing per-connection cap on frames buffered between and during turns. New queued traffic (signals) rides the same discipline rather than adding a second buffer. |
| DynaVis rule | The design's instruction that after an NL-driven parameter change the agent should emit a persistent bound control for that parameter, so the user refines by direct manipulation instead of re-prompting. |
| Idiomorph id-set matching | Keyed DOM reconciliation that matches elements by their id sets and patches in place, so focus, scroll, input state and canvas contexts survive an update. |
| `.brapp` / `.brxt` / `.brkb` | Bundle formats: a prospective one-click app bundle (v2.1), the installed-extension bundle, and the knowledge-base export bundle respectively. |

> **Note on line-number anchors.** Several steps below pin a `file.rs:line`
> location (`apps.rs:206-216`, `sdk.ts:74`, `apps.rs:1099-1111`,
> `apps.rs:708-737`). These were accurate on 2026-07-12 and drift as the phases
> land — treat them as "look here", not as coordinates. One anchor this note
> used to carry has gone entirely: the app-proxy it cited as the CSP precedent
> belonged to the inherited MCP apps feature, removed in September 2026 (see the
> record under `docs/history/`). The CSP decision it supported had already
> shipped in `routes/apps.rs` and is unaffected.

> **Note on estimates.** The week figures are effort estimates relative to the
> 2026-07-12 plan date. No calendar dates were committed, so elapsed progress
> cannot be read off this document; use the status map below instead.

## Which phases have landed

The reference documents shipped behaviour and flags what is only partial. Reading the two documents together:

| Phase | Theme | What the reference records |
|---|---|---|
| 1 | Protocol core | Landed — the shared state document, `state_write`, `ui_patch_state`, `catalog_version` on every `ui` frame, and the per-app socket token are all documented as shipping. |
| 2 | Catalog v2 | Landed — `ui_patch`, the widget catalog including `network`/`plot`/`figure`/`html`, custom `component` nodes, and the unknown-kind placeholder are documented as shipping. |
| 3 | Typed RPC and signals | Landed — `app_call`, `emit_result`, `br.call` with `debounceMs`/`supersede`, `br.signals`, and `ui_subscribe` are documented as shipping. |
| 4 | Platform APIs | Mostly landed — `br.kb`, `br.model`, and `orchestration.routes` with the provider-class rule ship. **Skills scoping is still advisory**, not enforced. |
| 4b | Multi-agent profiles | **Partial, actively landing** — `br.agent`, `ready.profiles` and `consult` exist, but cross-profile turns are serialized rather than parallel and `consult` depth is 1. |
| 5 | Aesthetics | Landed — six theme packs and six `create_app` archetypes ship. |
| 6 | Hardening, autorun, export v2 | Partly landed — `allow_autorun` with budgets, the server-side `ui_error` repair loop, and `export_app`'s `mode`/`include`/`bundle_daemon` ship. Still open: `.brxt` payload staging, `bundle_daemon: "all"`, and the in-SDK capability re-consent and credential screen. |

## Files that every phase touches

- `crates/biorouter-mcp/src/agent_drafter/control.rs` — `ui_*` tools, `UiBridge`, frame emission, validation
- `crates/biorouter-mcp/src/agent_drafter/templates/sdk.ts` — client runtime
- `crates/biorouter-mcp/src/agent_drafter/manifest.rs` — capabilities plus the new surface declarations
- `crates/biorouter-mcp/src/agent_drafter/bundle.rs` — build and `lint_app`
- `crates/biorouter-mcp/src/agent_drafter/mod.rs` — MCP tools and authoring instructions
- `crates/biorouter-server/src/routes/apps.rs` — socket loop, session, extension injection
- `scripts/agent-drafter/ui-control-harness.mjs`, `ui/desktop/scripts/appcheck/*` — harnesses and evals

## Phase 1 — protocol core: shared state, versioning, socket auth, typed surface

Estimated ~1–2 weeks. The foundation every other pillar builds on. Additive; no behaviour change for v1 apps.

0. **Reproducible regression gate (prerequisite).** Vendor and pin the app corpus plus the `round*` authoring scripts into this repo (today `appcheck/check-all.mjs` references an external worktree path), and record the exact command and pass count that constitutes the v1 baseline for `check-all` and `benchmark`.

1. **WS authority hardening.** Moved up from "hardening" because the socket gains real authority in later phases.
   - Exact-origin pinning on the `/apps/{id}/agent` upgrade (scheme + host + port of the app's served origin), replacing the any-localhost `is_local_origin` acceptance (`apps.rs:206-216`).
   - Per-app socket token: minted server-side into the served page's config (same-origin readable only), required as a query parameter or header on upgrade. The export path (`serve.mjs`) proxies it.
   - Tests: route tests — foreign-localhost origin rejected; missing or wrong token rejected; the existing same-origin flow unaffected.

2. **Frame versioning and fallbacks.**
   - Add `catalog_version` to the `ready` frame and every `ui` frame (`control.rs::emit`).
   - SDK: unknown `cmd` → ignore (verify existing); unknown widget kind → labelled placeholder div instead of an "unsupported widget" error (`renderWidget` default arm).
   - Test: an old-SDK fixture receives v2 frames without breakage (harness).

3. **Shared state document.**
   - `BridgeInner.state` becomes `{doc: serde_json::Value, version: u64}`. `ui_state` (compat) merges into the doc; the new tool `ui_patch_state { patch }` applies RFC-6902, bumps the version, and rebroadcasts `{cmd:"state", mode:"patch", patch, version}`.
   - New workspace dependency: the `json-patch` crate — vet its license and maintenance. The client-side binding index needs only JSON Pointer resolution, not the full crate.
   - New client→server frame `state_write { patch | set:{path,value}, base_version }`; the server applies it (last-writer-wins with a version check → on conflict send a fresh snapshot), rebroadcasts, and exposes the doc to the agent via `ui_describe` and tool results.
   - `attach()` replay sends `{cmd:"state", mode:"snapshot", doc, version}` (extend the existing replay).
   - Caps: 256 KB doc, 64 ops per patch, patch-rate limit; schema validation when `surface.state_schema` is present; **default structural caps when absent** (max depth 8, max 2,000 keys, string-length caps).
   - Tests: patch/version/conflict/rebroadcast unit tests in `control.rs`; snapshot-on-reconnect in the harness.

4. **State persistence.**
   - Persist doc and version per durable session (`app:<id>:<client_id>`) via the `ExtensionData` `store_into`/`load` pattern, as `RunState` does — no new sessions column and no migration. If a column ever becomes necessary, that is a human-reviewed schema change per `HOWTOAI.md`.
   - Enforce the 256 KB cap before serialization; restore seeds the `UiBridge` doc *before* `attach()` replays.
   - Test: route test — reconnect restores the doc.

5. **Declarative bindings (client-only, safe sinks).**
   - SDK: index `[data-br-bind]`/`data-br-bind-attr`/`data-br-bind-show` on mount and after any render; on a state snapshot or patch, resolve JSON Pointers and update only affected nodes. Add `br.state.get/set/update/subscribe(path, fn)`.
   - **Safety contract:** `data-br-bind` writes `textContent` only; `data-br-bind-attr` enforces an attribute allowlist (no `on*`) plus URL-scheme validation for `href`/`src`.
   - Tests: harness — a bound span updates on an agent patch; an author write round-trips; `javascript:` URL and `onclick` bind attempts are refused.

6. **Manifest `surface` block plus `ui_describe` v2.**
   - `manifest.rs`: add `Surface { state_schema, actions, signals, components }` (all optional, serde-defaulted).
   - Name them `signals`, not `events` — `Capabilities.events` already exists with opposite-direction semantics (the agent-lifecycle stream to the app via `br.on()`, advertised as `event:<name>`). Document the distinction where both are defined.
   - `ui_describe` merges declarations, runtime registrations, the browser-reported surface, and the state version into one typed report.
   - **Constructor change:** `AppControlServer::new` gains the manifest surface (today it receives only `UiBridge` + `UiCapability`); thread it through `configure_agent` in `apps.rs`.

7. **Docs:** add the protocol section to `docs/agent-drafter/apps-platform-design.md`.

**Acceptance:** a demo app with two bound spans and a button whose author code writes state; the agent patches state and both views update without any `innerHTML` replacement; reload restores everything; a foreign localhost page cannot open the socket.

## Phase 2 — catalog v2: flat IDs, `ui_patch`, morphing, science pack

Estimated ~2–3 weeks.

1. **Flat ID-keyed instances.** Normalize `ui_render`/`ui_panel` trees into an instance map (`id → {kind, props, parent, order}`) held per-connection in the SDK; assign stable auto-IDs where the agent omits them.

2. **`ui_patch` tool** — ops `add | replace | remove | set_props` by id; the server validates each node as `validate_widget` does today; the SDK applies them against the instance map.

3. **Morphing renderer.** Keyed reconciliation on re-render (match by id and kind, patch attributes and children, preserve focus, scroll, inputs and canvas). Modelled on Idiomorph's id-set matching; implemented in-SDK, dependency-free, ~250 lines.
   - Tests: focus survives a `set_props` on a sibling; an input value survives a table refresh (harness).

4. **Science pack, in order of leverage.**
   - a. `network` — port and generalize the BioOKF canvas engine (Barnes-Hut layout, culling, hover/select/drag/zoom, label placement) with a typed spec plus an `encoding` map; selection → declarable signal (wired in Phase 3). Constants become props with the Studio values as defaults.
   - b. `table` v2 — virtualized scroll, sort, filter, row-select.
   - c. `plot` — scatter/area/box/heatmap added to the existing themed-SVG charts; `bind` prop.
   - d. `markdown`, `image`, `kpi`, `log`.
   - e. `figure` — **prerequisite first:** `autovisualiser::common` is a private module (`autovisualiser/mod.rs:8`) and `render_fragment`/`ASSET_SINK` are not re-exported. Add a `pub(crate) fn render_named_figure(tool: &str, args: Value)` API on the autovisualiser side, dispatching to the named `render_*` tool inside `render_fragment`'s asset capture. Then the `ui_figure { tool, args, target }` tool emits the fragment into a sandboxed `srcdoc` iframe node. Asset splicing and dedup are real work — single-figure-per-node (each fragment carries its own assets) is acceptable for this phase. Same crate, so no circular-dependency risk.
   - f. `html` — **server-side, fail-closed sanitization in `control.rs`** with a pinned config (no script/style/form, no `on*`, no `javascript:`/`data:` URLs, SVG/MathML mXSS guards) plus a known-bypass regression corpus; `ui.allow_html` gate (default off); lint rule.

5. **Custom components.** `app.components.register(name, {props, mount, update})`; build-time extraction (`bundle.rs`) into the manifest. **Fail closed:** an unparseable or dynamic registration is a build error, never accept-any. Server-side prop validation against the declared schema (mismatch test); `ui_patch` can emit them like built-ins. Document and lint: props are agent-controlled and untrusted (flag `innerHTML` and URL sinks fed from props).

6. **Fence lowering:** ` ```chart `/` ```graph ` in markdown lower to catalog instances internally, so there is one renderer.

7. **Lint v2 (part 1):** custom component registered but undeclared (Error), declared but unregistered (Error), `html` used without the capability (Error), prop-fed sink (Warn).

**Acceptance:** the agent builds a network-plus-inspector UI via `ui_patch`, updates node props incrementally, and selection state survives every update; a custom `pathway_map` component round-trips validation and a schema mismatch is rejected; an autovis Kaplan-Meier renders inside an app panel; the sanitizer corpus is green.

## Phase 3 — typed RPC and signals: `app_call`, `br.call`, subscriptions, run-supersede

Estimated ~2 weeks.

1. **`app_call`.** A tool generated per declared action (schema in the tool description); frame `{type:"app_call", callId, action, args}`; the SDK dispatches to the registered handler; the result frame `app_result { callId, result | error }` resolves the parked tool call, reusing the `pending` oneshot machinery, timeout and `cancel_all` discipline from `ui_ask`. `app_result` values are size-capped (64 KB, truncation marker) and delivered in the untrusted-data envelope.

2. **`br.call(name, args, {route?, output_schema?})` via the `emit_result` tool convention.** There is **no** structured-output or response-format channel in the `Provider` trait (`providers/base.rs` — `complete*` take only system, messages and tools), so v2 does not build one. When `output_schema` is supplied, the server injects a synthetic `emit_result` tool whose input schema *is* the `output_schema`, instructs the model to finish by calling it, validates the call (the tool path already validates schemas), and emits the new `output {schema, value}` frame — the type exists at `sdk.ts:74` but has never had a server producer; this step adds it. Prose fallback when the model never calls the tool. Provider-level structured output remains a possible later swap behind the same app-facing contract.

3. **`widget_action` becomes typed without losing turn-start semantics.** Today the `WidgetAction` arm (`apps.rs:1099-1111`) synthesizes the *user `Message` that starts the turn* — it is the only ingress for author-declared buttons, not mere formatting. Keep the turn trigger; change the payload to a minimal user-text envelope carrying the structured JSON block. Verify the store's button-driven apps against this before removing the pure-prose form (compat flag for one release).

4. **Signals.** `app.signals.emit(name, payload)` → a `signal` frame (validated against the declaration, coalesced per `coalesce_ms` client-side, rate-capped server-side); the `ui_subscribe {signals}` tool; queued through the existing between-turn and mid-turn frame paths under the `MAX_QUEUED_FRAMES` discipline; delivered to the model as size-capped structured JSON inside the untrusted-data envelope. **Queue-only in this phase:** signals are context for the next turn and never start one. (`autorun` is deferred to Phase 6 behind its own capability and budgets.)

5. **Debounce and supersede runs.** `br.run`/`br.call` options `{debounce_ms, supersede}`; supersede sends `cancel` for the in-flight superseded turn (the cancel path already works mid-turn).

6. **Lint v2 (part 2):** action declared but never registered; signal emitted but never declared; `buildPrompt`-style string concatenation detected while actions exist (Warn: prefer `br.call`).

**Acceptance:** the avatar demo — a declared `move_avatar` action, the agent driving it from NL, state animating the canvas, a `collision` signal reaching the agent, a slider-driven `br.call` superseding stale turns, and `output_schema` results arriving as data via `emit_result`.

## Phase 4 — platform APIs: `br.kb`, model routes, tool-resource bridge

Estimated ~2 weeks. The read-only `br.kb` slice has no Phase-3 dependency and can start right after Phase 2, so the KB-explorer flagship lands early.

1. **`br.kb`** — server-side handlers on the app socket, gated by `data.sources[kind:knowledge]` **scoped to enumerated KB ids** (default: none — never "all bases"): `search`, `page`, `graph`, `history`. `ingest` requires the separately-consented `write:true` (the cross-session KB-poisoning risk is documented in the capability prompt) and streams progress frames, reusing the knowledge SSE machinery. A client namespace with typed results.

2. **Model routes.** Manifest `agent.routes: {name: {provider?, model?}}`; `br.call({route})` and `ui_*`-initiated turns can select one. Validation: routes must resolve against the user's configured providers at session start (same fallback chain as today). **Provider-class constraint:** apps holding sensitive data sources (`omop`, `cdw`, confidential KBs) are restricted to local or institutional provider classes unless the user explicitly consents per app. `br.model.status()` surfaces llamacpp state via the existing `/llamacpp/status` plumbing.

3. **Tool `ui://` resources → `figure` components.** When an extension tool result carries a `ui://` embedded resource, the socket loop emits it as a catalog `figure` instance targeted at the app's declared results region (author-overridable), instead of dropping it. This depends on Phase 2's figure node, not on autovis internals — the resource is already-rendered HTML.

4. **Skills scoping enforcement** for app sessions: the per-app skill list actually filters rather than being advisory.

5. **Docs and capability matrix** in `docs/agent-drafter/apps-platform-design.md`.

### Phase 4b — multi-agent profiles

Design [Pillar 8](v2-design.md#pillar-8-multi-agent-apps). Estimated ~1–2 weeks, after Phase 3's `br.call` lands.

6. **Named profiles.** Wire the dormant `orchestration.agents` manifest map (the struct exists in `manifest.rs`; `apps.rs` never reads it): the `agent` block becomes the `main` profile. Validate at session start that each profile's capabilities, extensions and KB are a **subset** of the app's grants, and that routes obey the provider-class constraint per profile.

7. **Session-per-profile plus frame multiplexing.** Sessions keyed `app:<id>:<client_id>:<profile>`; frames gain an optional `agent` field (omitted = `main`, so every v1 frame is unchanged). `handle_agent_socket` grows a task-per-active-profile with an mpsc merge into the single socket writer — this is the real work: today the loop drives exactly one agent's turn, whereas profile turns must run concurrently (per-app cap, default 3) while same-profile turns stay serialized. Cancel targets a profile.

8. **SDK facade.** `br.agent(name)` → a scoped `call`/`run`/`prompt`/`on`; `main` remains the default for all existing APIs. Presence chips and timeline items attribute the acting profile ("Critic · …"); `handoff` frames (already rendered, `sdk.ts:1156`) get the profile name.

9. **`consult` tool.** `main` — or any profile granted it — can invoke a named profile mid-turn as a tool, parked and resolved like `app_call`, with the result size-capped and enveloped. Keep the shipped sub-agents-as-tools path (`materialize_subagent_recipe`, `apps.rs:708-737`) untouched as the low-cost delegation default; document when to use which.

10. **UI authority.** Only `main` gets `appcontrol` unless a profile declares `ui:true`; worker-profile panels are namespaced so two profiles cannot fight over one panel id.

11. **Tests.** Route tests for subset validation and parallel-profile turns; harness — two profiles answer one fan-out concurrently and render into separate panels; unit — consult parking, timeout and cancel; per-app concurrency cap enforced.

**Acceptance:** the KB-explorer demo app (design [Example A](v2-design.md#example-a-knowledge-graph-explorer)) works end-to-end against a real knowledge base with `network` plus inspector plus agent narration, and cannot read a KB outside its grant. The adversarial-review demo (design [Example D](v2-design.md#example-d-adversarial-evidence-review)) runs both author-orchestrated (`br.agent(...)` fan-out) and agent-orchestrated (`consult`) with visible per-agent attribution.

## Phase 5 — aesthetics: theme packs, layout grammar, archetype starters

Estimated ~1–2 weeks.

1. **Theme packs.** A token-set structure in the manifest; six curated presets in `theme.css` (CSS custom-property layers, including dark variants); `render.rs` injects the selected pack; `ui_theme` pack-switching behind `allow_theme`. The contrast lint generalizes to the active pack.

2. **Layout grammar.** `ui_layout { areas, sizes }` → a validated grid template (bounded vocabulary, no raw CSS). **The 5 existing presets remain as aliases** so v1 `ui_layout{preset}` calls keep working.

3. **Archetype starters.** Six starter template sets (`explorer`, `dashboard`, `workbench`, `wizard`, `canvas`, `chat`) under `templates/starters/`; `create_app` gains `archetype` (default: inferred from the description, `chat` only when asked). Each starter ships a bound state path, one action, one signal, and one catalog component — working and lint-clean.

4. **Authoring instructions rewrite** (`mod.rs`): archetype-first structure, one curated exemplar per archetype, the DynaVis rule, presence and narration guidance, and a `frontend-design` skill pointer.

5. **Applications tab audit** (`ui/desktop/src/components/applications/ApplicationsView.tsx`): archetype badge, theme-pack swatch, and a launch affordance appropriate for non-chat apps — or a documented decision that no change is needed.

**Acceptance:** ten vague-prompt apps authored by a mid-tier model, of which ≥8 select a non-chat archetype and lint clean.

## Phase 6 — hardening, autorun, export v2, error loop, evals, docs

Estimated ~2–3 weeks plus ongoing work.

1. **Strict CSP on served apps** in `routes/apps.rs`: `script-src 'self'` (no `unsafe-inline` — the injected `BIOROUTER_APP_CONFIG` becomes a non-executable `<script type="application/json">` the SDK parses, a `render.rs` change), `connect-src 'self'`, `img-src 'self' data:`, `form-action 'none'`, `base-uri 'self'`, `frame-ancestors 'self'`. Verify export and `serve.mjs` parity.

2. **`autorun` (last, smallest).** The `ui.allow_autorun` capability — default off, user-granted only, never agent-self-granted; per-minute, per-session and daily turn budgets; a presence-layer indicator with a one-click stop. Signals otherwise stay queue-only.

3. **`ui_error` feedback loop.** Catalog render and handler errors post structured `ui_error` frames; the agent sees them only under the live-turn grace discipline (port the `shouldAutoRepairArtifact` semantics); harness test.

4. **Mass SDK reroll plus regression.** Batch rebuild of the whole store on the new SDK, alongside the existing lazy `serve_index` drift rebuild. Acceptance: all stored apps rebuilt, and `check-all` holds the pinned v1 pass count.

5. **Benchmark v2.** Extend `appcheck/benchmark.mjs` to score bound-state paths, declared actions, catalog components and archetype distribution on raw store markup (same honesty rule as v1); wire it into the vendored `round2`/`round3` scripts; record against the pinned v1 baseline.

6. **Docs.**
   - Rewrite the v2 sections of `docs/agent-drafter/apps-platform-design.md`.
   - Write the **human-facing SDK reference**, distinct from the author-agent prompts: `br.*` API, `manifest.surface` JSON Schema, frame and protocol reference, capability matrix, custom-component guide, and three worked examples as annotated source.
   - Update the CLAUDE.md pointer.
   - Add example apps under `scripts/agent-drafter-apps/examples/` (KB explorer, avatar, cohort dashboard).

7. **CLI parity:** `biorouter apps list|open|serve` (launch daemon, print or open the URL). In-terminal rendering stays out of scope.

8. **Standalone export v2.** Design [Pillar 9](v2-design.md#pillar-9-app-lifecycle-and-standalone-export).

   - **Export dialog and modes.** The panel's Export button becomes a dialog; `export_app` gains `mode: "launcher" | "full"`, `include: {knowledge_bases?, skills?, extensions?}`, and `bundle_daemon: "none" | "current" | "all"`. Launcher mode is today's scaffold, kept as a first-class choice (app plus scripts, no payload). Full mode shows per-item toggles pre-checked from the app's agent config, with per-item size estimates and raw KB sources separately toggleable.
   - **Payload staging** into `payload/`: KBs via the existing `.brkb` knowledge export (the `/knowledge/*` routes); skills as marketplace-format zips; external extensions as locally-installed `.brxt` bundles or as pinned, checksummed BAAM registry references. Built-ins need nothing — they travel with the daemon. Everything is enumerated in `export.json`: items, versions, checksums, required credential keys taken from the extensions' declared env keys, and runtime prerequisites.
   - **First-run installer and consent.** `biorouter-launch.sh` and `serve.mjs` detect an uninstalled payload and drive one flow: a single consent screen (design Pillar 7 capability re-consent merged with the payload install list) → import `.brkb` KBs under their original ids and install skills and extensions → a **credential setup dialog** in the SDK for unmet `export.json` credential keys, plus provider setup when none is configured, stored via a daemon route into the OS credential store so secrets never exist in the folder → app opens. Re-launches skip satisfied steps; a same-machine self-export skips consent for grants and payload already present.
   - **OS-agnostic launchers.** Add `run.ps1`/`run.bat` (Windows) beside `run.command`/`run.sh`. Thin mode auto-installs the platform-matching daemon from the pinned GitHub release per OS; fat mode stages `current` or `all` platform binaries. The export README documents macOS quarantine (right-click-Open).
   - **Socket-token and CSP parity in `serve.mjs`**, together with Phase 1's WS auth: the proxy mints or forwards the per-app token and serves the same headers.
   - **Graceful degradation.** Missing KB grants (the user unchecked the toggle) feature-detect via `has()` (harness fixture); unmet runtime prerequisites, such as Node for a `.brxt`, produce a clear remedy message rather than a crash.
   - **Re-import path.** An edited export re-installs through lint and build; `sdk_hash` drift rebuilds. Foreign-machine capability re-consent per design Pillar 7.
   - **Tests and smoke.** A full export with a KB, a skill, and a credential-requiring extension installs into a clean `XDG_CONFIG_HOME`, and the app answers a KB-grounded question after the credential dialog. A launcher-mode export still runs against a pre-configured store. A Linux fat export boots in a clean container, mirroring the `cli-linux` release smoke. A Windows launcher smoke runs in the release Docker path, and macOS has a manual checklist.

9. **Full-matrix verification:** unit plus route plus harness plus `check-ui-app.mjs` live, plus the export running standalone (thin *and* fat), plus an Electron Applications tab smoke (`just agent-browser-ui` or the debug-app skill).

## Sequencing and risk notes

- **Phases 1→2→3 are the dependency spine** (state → catalog → RPC and signals). The read-only `br.kb` slice of Phase 4 can run concurrently after Phase 2, so the flagship KB explorer — the closest analog to the BioOKF exemplar — demos as early as possible. Phase 5 can start once Phase 2 lands; Phase 6 is continuous with a closing milestone.
- **Compat gates every phase:** the existing `agent_drafter::` unit tests, `agent_drafter_registered`, and the vendored `check-all.mjs` corpus must stay at the pinned v1 pass count. Any v1 app that breaks is a phase-blocking bug.
- **Deferred to v2.1**, explicitly, per design [scope decisions](v2-design.md#scope-decisions): `.brapp` distribution and BAAM listing (gated on import re-consent), workflows and scheduled refresh, provider-level structured output, collaborative multi-user apps, and voice input. **Multi-agent profiles are NOT deferred** — they land as Phase 4b; only same-profile parallel turns and cross-process (ACP) orchestration stay out.
- **Biggest technical risks:**
  - Morphing renderer correctness — mitigate with the harness's focus and scroll tests before porting widgets onto it.
  - The `emit_result` convention's reliability across weak local models — mitigate with the lint and instruction pairing plus the prose fallback, and measure it in benchmark v2.
  - Autovis `figure` wiring — the private-module refactor is a prerequisite task, not incidental.
  - WS token and origin changes must not break the Electron Applications tab or exported apps — route tests plus the export smoke cover both.
- **Estimate:** ~12–16 weeks of focused work end-to-end (Phase 4b adds ~1–2; full-payload export adds ~1 to Phase 6), but Phase 1 plus Phase 2 (≈4 weeks) already dissolve the chatbot ceiling, and with the early `br.kb` read slice the KB-explorer class of apps lands by ~week 6. Multi-agent apps (Phase 4b) land by ~week 8–9. Launcher-mode export works throughout — it is the v1 scaffold plus the per-phase parity gate — while full-payload export lands with Phase 6.

## Related documentation

- [Apps SDK v2 design](v2-design.md) — the nine pillars this roadmap sequences, and the rationale behind each acceptance gate.
- [Apps SDK v2 reference](sdk-reference.md) — what has actually shipped, which is how you tell a phase's plan from its outcome.
- [BioRouter Apps platform design](../agent-drafter/apps-platform-design.md) — the subsystem doc several phases here schedule updates to.
- [Agent Drafter SDK strategy RFC (2026-06)](../history/apps-sdk-rfc-2026-06/strategy-and-openai-comparison.md) — the earlier phased SDK proposal this roadmap supersedes; useful for the pre-v2 framing.
- [Agent Drafter 100-app test-drive runbook](../agent-drafter/testing/app-test-drive-runbook.md) — how the authored-app corpus that gates every phase is actually driven.
