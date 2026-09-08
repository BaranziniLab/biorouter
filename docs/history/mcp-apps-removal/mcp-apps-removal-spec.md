# MCP apps — removal map

> **What this is.** The read-only survey the MCP apps removal was carried out from: every file to delete, every line to edit, the tests that had to change, and the proof that nothing Biorouter-specific depended on the feature. Kept for provenance.
> **Status:** Historical record — executed on 2026-09-08. The survey was made against `main` at `6c5bf2c9`; the removal landed on `0b8d9050`. Where the two disagree, the code is the authority. [The folder README](README.md) carries the removal record in conformed prose.
> **Audience:** maintainers tracing why the MCP apps feature is gone, or auditing what the removal touched.

Verified against `main` @ `6c5bf2c9`. Nothing was changed; no builds were run.

---

### 0. The two "apps" endpoints are provably distinct — do not conflate

| | MCP apps (REMOVE) | Built apps / Agent Drafter (KEEP) |
|---|---|---|
| Route | `GET /agent/list_apps` — `routes/agent.rs:2644`, registered `agent.rs:2708` | `GET /apps` — `routes/apps.rs:138`, registered `apps.rs:6415` |
| Returns | `ListAppsResponse { apps: Vec<BioRouterApp> }` (`agent.rs:2622-2626`) | `Json<Vec<Manifest>>` |
| Source of data | `fetch_mcp_apps()` → `ExtensionManager::get_ui_resources()` (`extension_manager.rs:2485-2528`), which keeps only `ui://` URIs from extensions whose `ServerCapabilities` advertise resources; plus an on-disk cache at `~/.config/biorouter/mcp-apps-cache` (`biorouter_apps/mod.rs:43`) | Agent Drafter manifests on disk |
| Sidebar row | `/apps` "MCP apps" (`AppSidebar.tsx:147-153`) | `/applications` "Built apps" (`AppSidebar.tsx:140-146`) |

`BioRouterApp` (`biorouter_apps/mod.rs:27`) is the MCP-apps type only. `Manifest` is Agent Drafter's. They share nothing.

---

### 1. Files to DELETE

| Path | What it is | Proof nothing outside the feature imports it |
|---|---|---|
| `crates/biorouter/src/biorouter_apps/mod.rs` | `BioRouterApp`, `WindowProps`, `McpAppCache`, `fetch_mcp_apps` | `grep -rn biorouter_apps crates` → only `lib.rs:20`, `openapi.rs:819-824`, `routes/agent.rs:15` |
| `crates/biorouter/src/biorouter_apps/resource.rs` | `McpAppResource`, `UiMetadata`, `CspMetadata`, `ResourceMetadata` | same grep; re-exported only via `mod.rs:14` |
| `crates/biorouter-server/src/routes/mcp_app_proxy.rs` (627 ln, 7 unit tests) | The sandbox document + `POST /mcp-app-proxy/token` | referenced only from `routes/mod.rs:156,213`, `auth.rs:210`, and two doc comments (`workspace.rs:24`, `apps.rs:87`) |
| `crates/biorouter-server/src/routes/templates/mcp_app_proxy.html` | the sandbox HTML, `include_str!` at `mcp_app_proxy.rs:76` | sole file in `routes/templates/` — delete the directory |
| `ui/desktop/src/components/McpApps/McpAppRenderer.tsx` | the inline/fullscreen iframe renderer | CodeGraph blast radius: 2 callers — `apps/StandaloneAppView.tsx`, `ToolCallWithResponse.tsx`; both are in scope |
| `ui/desktop/src/components/McpApps/useSandboxBridge.ts` (296 ln) | postMessage bridge to the proxy | blast radius: 1 caller, `McpAppRenderer.tsx:208` |
| `ui/desktop/src/components/McpApps/utils.ts` | `fetchMcpAppProxyUrl`, `DEFAULT_IFRAME_HEIGHT` | blast radius: `useSandboxBridge.ts:13`, `McpAppRenderer.tsx:21` |
| `ui/desktop/src/components/McpApps/types.ts` | JSON-RPC / host-context types (**not named in the brief** — delete it too) | imported only by `McpAppRenderer.tsx:11-19` |
| `ui/desktop/src/components/apps/AppsView.tsx` | the `/apps` page | blast radius: `App.tsx:54,711` |
| `ui/desktop/src/components/apps/StandaloneAppView.tsx` | the full-page window | blast radius: `App.tsx:55,689` |
| `ui/desktop/src/components/apps/AppsView.test.tsx` | — | self-contained |
| `ui/desktop/src/components/apps/StandaloneAppView.test.tsx` | — | self-contained |

Deleting all four files under `components/apps/` removes the directory — **required**, see §4.

---

### 2. Files to EDIT

#### Rust — server

**`crates/biorouter-server/src/routes/mod.rs`**
- delete `pub mod mcp_app_proxy;` (line 156)
- delete `.merge(mcp_app_proxy::routes(secret_key.clone()))` (line 213)
- line 214 `.merge(workspace::routes(state.clone(), secret_key.clone()))` — `secret_key` now has one consumer; the `.clone()` can drop. Keep the `secret_key` parameter on `configure` (workspace still needs it).

**`crates/biorouter-server/src/auth.rs`**
- delete `| "/mcp-app-proxy"` from `is_unauthenticated_path` (line 210). The exempt set becomes `/status`, `/ui/workspace`, plus the `/tool_bridge/{nonce}` prefix rule (lines 203-205) — **`/tool_bridge` is untouched.**
- delete `assert!(is_unauthenticated_path("/mcp-app-proxy"));` (line 679) inside `the_workspace_socket_is_exempt_and_nothing_that_merely_starts_with_it_is`; keep the rest of that test.

**`crates/biorouter-server/src/routes/agent.rs`**
- delete import line 15 `use biorouter::biorouter_apps::{fetch_mcp_apps, BioRouterApp, McpAppCache};`
- delete `ListAppsRequest` (2617-2620), `ListAppsResponse` (2622-2626), the `#[utoipa::path]` block + `async fn list_apps` (2628-2696)
- delete route registration `.route("/agent/list_apps", get(list_apps))` (line 2708)
- `warn!` has 12 other uses in the file — no import churn.
- **KEEP** `/agent/read_resource` (2288-2320, registered 2706). After removal it has no frontend caller (its only one was `McpAppRenderer.tsx:70,172`), but it is a generic MCP capability, not MCP-apps machinery. Deleting it is a separate decision.

**`crates/biorouter-server/src/openapi.rs`**
- delete `super::routes::agent::list_apps,` (line 409)
- delete `super::routes::agent::ListAppsRequest,` / `ListAppsResponse,` (lines 769-770)
- delete the six schema registrations, lines 819-824 (`BioRouterApp`, `WindowProps`, `McpAppResource`, `CspMetadata`, `UiMetadata`, `ResourceMetadata`)

**`crates/biorouter-server/src/routes/workspace.rs`** — doc comment at lines 22-25 names `mcp_app_proxy::routes(secret_key)` as "the one route that needs it". Reword: workspace is now the only consumer. **No code change.**

**`crates/biorouter-server/src/routes/apps.rs`** — line 87 doc comment: *"Mirrors the app-proxy's existing `script-src 'self'` (`mcp_app_proxy.rs`)."* Drop the cross-reference. **This is Agent Drafter's CSP — do not touch the policy itself.** Everything else in `apps.rs` is Agent Drafter.

#### Rust — core

**`crates/biorouter/src/lib.rs`** — delete `pub mod biorouter_apps;` (line 20)

**`crates/biorouter/src/agents/extension_manager.rs`** — `get_ui_resources` (lines 2485-2528) becomes orphaned (`grep` shows only `biorouter_apps/mod.rs:137` + its own tests). Deleting it means touching **privacy Gate-C sibling tests** — go carefully:
- test `ui_resource_sweep_only_contacts_reachable_resource_servers` (6998-7040) — delete whole test
- `SIBLING_PROBES` entry `"get_ui_resources",` (line 7050) and its `run_sibling` arm (line 7084) — delete both together
- `the_other_two_fanouts_still_serve_the_public_extension` (7216-7237) — delete the `get_ui_resources` half (7221-7226), keep the `list_prompts` half; its doc comment (7214-7215) says "the other two fan-outs" → becomes one
- fixture `list_resources` at 6795-6808 publishes a `ui://…/panel` resource *solely* for that probe (comment at 6801-6802) — that entry can go, the `res://x` one stays
- `supports_resources()` survives: still called at `extension_manager_extension.rs:2986`, `extension_manager.rs:1340`, `:2611`.
- **Lower-risk alternative:** leave `get_ui_resources` in place. It is dead but harmless, and the Gate-C tests keep covering the pattern. If left, add `#[allow(dead_code)]` is not needed — it is `pub`.

**`crates/biorouter/src/agents/builtin_skills/about-biorouter/SKILL.md`** — line 216: rewrite the sidebar sentence to drop `**MCP apps** (apps advertised by installed extensions, shown only when some extension provides one)`. Keep `**Built apps**`. Line 232's `biorouter apps …` is **Agent Drafter** (`crates/biorouter-cli/src/commands/apps.rs:55-57` resolves `biorouter_mcp::agent_drafter::default_root()`) — leave it.

No `Cargo.toml` changes: `sha2` and `tokio_util` both have other users in `crates/biorouter`.

#### Generated API client (regenerate — do not hand-edit)

Driven entirely by the server edits above. After `just generate-openapi && cd ui/desktop && npm run generate-api`, these disappear:
- `ui/desktop/openapi.json`: path `/agent/list_apps` (line 468, `operationId: list_apps`) and schemas `BioRouterApp` (6193), `McpAppResource` (9212), `WindowProps`, `CspMetadata`, `UiMetadata`, `ResourceMetadata`
- `ui/desktop/src/api/sdk.gen.ts`: `export const listApps` (161-165)
- `ui/desktop/src/api/types.gen.ts`: `BioRouterApp` (259), `CspMetadata` (736), `ListAppsRequest` (1668), `ListAppsResponse` (1672), `McpAppResource` (1854), `ResourceMetadata` (2960), `UiMetadata` (3878), `WindowProps` (4095), `ListAppsData` (4517), `ListAppsErrors` (4526), `ListAppsError` (4537), `ListAppsResponses` (4539), `ListAppsResponse2` (4546)
- `ui/desktop/src/api/index.ts`: the barrel re-exports of all of the above

**Verified closed island:** those six schemas are each referenced exactly once in `openapi.json`, all inside the `ListAppsResponse → BioRouterApp` chain. Nothing in Agent Drafter, Auto Visualiser or knowledge references them.

#### Frontend

**`ui/desktop/src/App.tsx`**
- delete imports 54 (`AppsView`), 55 (`StandaloneAppView`)
- delete `<Route path="standalone-app" element={<StandaloneAppView />} />` (689)
- delete `<Route path="apps" element={<AppsView />} />` (711). **Keep 712, `applications`.**
- There is no `apps/:id` route — the second surface is the top-level `standalone-app` hash route driven by query params.

**`ui/desktop/src/components/BioRouterSidebar/AppSidebar.tsx`**
- delete `import { listApps } from '../../api';` (line 20)
- rewrite the doc comment (90-103) — it exists only to distinguish the two rows; the last two sentences about `hasApps` go
- delete the nav entry `{ path: '/apps', label: 'MCP apps', icon: ENTITY_ICONS.mcpApp, … }` (147-153)
- delete `const [hasApps, setHasApps] = useState(false);` (180)
- delete the whole `useEffect` that calls `listApps` (187-200) — **this is the `hasApps` data source**
- delete `visibleComponentItems` (318-320); change line 339 and line 436 to read `componentItems` directly
- `useState`/`useEffect` imports stay (used by the disclosure state)

**`ui/desktop/src/components/ToolCallWithResponse.tsx`** — the discriminator:
- **The condition** is lines 501-503:
  ```
  const hasMcpAppResourceURI = Boolean(
    requestWithMeta._meta?.ui?.resourceUri || resultWithMeta?.value?._meta?.ui?.resourceUri
  );
  ```
  Line 537 gates the artifact path on `!hasMcpAppResourceURI`; line 554 gates the MCP-app path on `hasMcpAppResourceURI && sessionId`.
- delete `McpAppRenderer` import (26) and `FlaskConical` from the icon import (22) — `FlaskConical` is used only at 414, the "MCP Apps are experimental" banner
- delete `type UiMeta` (47-51); change `type ResultMeta = UiMeta & { … }` (72-75) to a plain object type keeping the two `biorouter/tool-calls*` keys
- **KEEP** `ToolResultWithMeta` (77-82) — still used at line 1287 for issue-#28 executed-call telemetry
- delete `type ToolRequestWithMeta` (84-93) — MCP-apps-only
- delete `McpAppWrapperProps` (344-349) and `function McpAppWrapper` (351-420)
- delete lines 499-503 (`requestWithMeta`, `resultWithMeta`, `hasMcpAppResourceURI`)
- change line 537 from `{!hasMcpAppResourceURI && toolResponse?.toolResult && …}` to `{toolResponse?.toolResult && …}`
- delete the `<McpAppWrapper …/>` block (554-561)
- `append` (110, 348, 355, 411, 483, 559) becomes **completely dead** — its only consumer was `McpAppRenderer`'s `ui/message` handler. The chain is `BaseChat.tsx:2392,2410` → `ProgressiveMessageList.tsx:35,73,270` → `BioRouterMessage.tsx:36,75,275` → `ToolCallWithResponse.tsx:110,559`. Cut the whole chain or none of it; cutting it partway leaves an unused destructured param and fails `no-unused-vars`.

**What renders after removal for a tool response that carried an MCP-app resource:** the ordinary `ToolCallView` row, and nothing else — unless the response *also* embeds a `ui://` resource in `content`, in which case it now falls through to the `MCPUIResourceRenderer` branch and becomes a click-to-open artifact card. The typical MCP-App shape points at its resource by URI in `_meta` (fetched separately via `/agent/read_resource`) rather than embedding it, so the usual outcome is just the tool row. No crash, no error card.

**`ui/desktop/src/components/BaseChat.tsx`** — comment only, line 1759: `Dispatched by MCPUIResourceRenderer / McpAppRenderer, both of which render` → drop the second name. The `scroll-chat-to-bottom` listener stays (`MCPUIResourceRenderer`'s prompt actions still dispatch it via `BaseChat.tsx:281`).

**`ui/desktop/src/components/icons/entity-icons.ts`**
- delete `'mcpApp'` from `EntityKind` (line 23)
- delete `mcpApp: AppWindowMac,` (39) and rewrite the comment at 36-37
- delete `AppWindowMac` from the import (line 4) — `entity-icons.ts:39` is its only consumer. `app-icons.tsx:13,165` re-export it; removing that is optional.

**`ui/desktop/src/main.ts`**
- delete `import { BioRouterApp } from './api';` (175)
- delete the entire `ipcMain.handle('launch-app', …)` block (6104-6163), which builds `#/standalone-app?resourceUri=…` (6150-6155)

**`ui/desktop/src/preload.ts`**
- delete `import { BioRouterApp } from './api';` (3)
- delete `launchApp` from the interface (364) and the bridge (745)

**`ui/desktop/src/renderer.tsx`** — delete the browser-mode shim `launchApp: async (url) => window.open(...)` (536). (`getBiorouterdHostPort` / `getSecretKey` stay — many other consumers.)

**`ui/desktop/src/hooks/chatStreamStore.tsx`**
- delete `listApps,` from the import (line 9)
- delete the fire-and-forget cache-warm block (1650-1655)

**`ui/desktop/src/components/Layout/PageHeader.tsx`** — doc comments only: line 8 (*"Optional because MCP apps and a…"*) and line 33 (view list). No code change.

**`ui/desktop/src/components/Layout/ReadableContent.tsx`** — doc comment line 45. No code change.

**`ui/desktop/src/styles/main.css`** — line 481 is prose inside the `--measure-page` comment listing "…applications, MCP apps". **No CSS class is exclusive to this feature** — I grepped: `AppsView` uses only shared primitives, and `McpAppRenderer`'s `bg-bgApp` (line 259) has no other consumer but is a Tailwind theme utility, not an authored class. Nothing to delete from the stylesheet.

---

### 3. Tests to delete or adjust

**Delete:** `ui/desktop/src/components/apps/AppsView.test.tsx`, `ui/desktop/src/components/apps/StandaloneAppView.test.tsx`.

**Must edit or the suite breaks at module load:**

- `ui/desktop/src/styles/measures.test.ts` — `'apps/AppsView.tsx'` appears **twice**: line 94 (`CHAT_MEASURE_VIEWS`) and line 122 (`PAGE_HEADER_VIEWS`). Both do `readFileSync` at module scope → **deleting the file without removing these two lines crashes the whole test binary.**
- `ui/desktop/src/components/settings/settingsVocabulary.test.ts` — `ROOTS` entry at lines 76-78: `{ dir: join(SETTINGS_DIR, '../apps'), outOfScope: ['StandaloneAppView.tsx'] }`. It walks the directory with `readdirSync` (line 85) → ENOENT once `components/apps/` is gone. And the `finds sources under every root it claims to govern` assertion (line 202-205) would fail even if `readdirSync` survived. **Delete the entry and its 2-line comment.**

**Should edit:**

- `ui/desktop/src/components/BioRouterSidebar/AppSidebar.test.tsx` — delete the `listApps` mock (lines 11, 17), the `mockResolvedValue` in `beforeEach` (59), and the whole test `hides the MCP Apps row until an extension advertises one` (275-280).
- `ui/desktop/src/components/Layout/PageHeader.test.tsx` — line 22 uses `"MCP apps"` and line 102 `"Built apps"` as arbitrary title strings; neither tests the feature. Change line 22's string (e.g. to `"Extensions"`); leave 102.
- Eight `chatStreamStore` test files each carry one stale `listApps: vi.fn(...)` line inside a `vi.mock('../api', …)` factory: `chatStreamStore.test.ts:16`, `.adversarial.test.tsx:18`, `.attach.test.tsx:30`, `.mirroredToolCalls.test.ts:25`, `.observe.test.tsx:17`, `.observerIdle.test.tsx:32`, `.submitVerdict.test.tsx:27`, `.userAction.test.tsx:52`. Harmless if left; remove for hygiene.

**Rust:**
- `crates/biorouter-server/src/routes/mcp_app_proxy.rs` — 7 unit tests die with the file (`a_csp_keyword_is_not_a_domain`, `a_quote_can_neither_survive_validation_nor_escaping`, `ordinary_hosts_are_accepted`, `the_bootstrap_runs_by_nonce_and_the_template_carries_it`, `the_guest_frame_is_not_granted_the_daemon_origin`, `only_a_live_minted_token_opens_the_sandbox_document`, `tokens_expire_and_the_store_is_bounded`).
- `crates/biorouter-server/src/auth.rs:679` — one assertion.
- `crates/biorouter/src/agents/extension_manager.rs` — the Gate-C sites listed in §2, **only if** you also delete `get_ui_resources`.
- **No integration tests reference the feature.** `grep -rn "list_apps\|mcp_app\|biorouter_apps\|BioRouterApp" crates/biorouter-server/tests crates/biorouter-test crates/biorouter/tests crates/biorouter-cli` → zero hits.

**E2E:** nothing. `ui/desktop/tests/e2e/` (15 files) has no reference — the only `apps` hit is an unrelated comment in `brxt-spoke-install.spec.ts:20`.

---

### 4. Docs

**Living — must be edited:**

| File | Lines | What to do |
|---|---|---|
| `docs/desktop-ui/artifact-display-surfaces.md` | 78-82 | The **"What deliberately stayed"** bullet says MCP Apps "still render inline". Delete the bullet; move the load-bearing fact — *no Rust code emits `_meta.ui.resourceUri`* — into the rule section, since it now explains why removal is safe. Keep the `@mcp-ui/client` bullet (83-85). |
| `docs/desktop-ui/README.md` | 32, 34 | Two index rows quote the above ("MCP Apps are a different feature") and list "MCP apps" among the component views. |
| `docs/desktop-ui/settings-visual-vocabulary.md` | 3 | Context header lists "plus MCP apps" among the swept views. |
| `docs/desktop-ui/window-scaling-regressions.md` | 210, 220 | Two paragraphs about MCP apps having no reading column. |
| `docs/desktop-ui/preview-panel/current-state.md` | 158, 174 | Status `Current`; `:158` says `http:` is load-bearing for the MCP-app proxy iframe (**re-check that claim after removal** — it may free a CSP narrowing); `:174` is a stale finding about the daemon secret in a URL, already fixed. |
| `docs/cli/qa-checklist.md` | 204 | "interactive MCP Apps/MCP-UI" in the GUI-parity gap list. |
| `docs/design/astryx-adoption/astryx-ui-adoption-design.md` | 308, 322 | `Status: Proposed`. §308 argues *"There is no 'Apps' — there is Applications"* — this removal **executes** that recommendation; annotate rather than delete. §322 lists MCP apps among `PageHeader` consumers. |
| `crates/biorouter/src/agents/builtin_skills/about-biorouter/SKILL.md` | 216 | See §2 — this is shipped model-facing text, the highest-priority edit. |

**Living — no change needed:**
- `docs/apps-sdk/v2-design.md:53` — "MCP-UI/MCP Apps" is a row in an *external ecosystem* comparison table, not a reference to this code.
- `docs/apps-sdk/v2-phase-roadmap.md` — **the brief lists this; it has zero matches.** Nothing to do.
- `docs/security/privacy-tiers-execution-plan.md:13011, 13133, 13316` and `docs/agent-loop/designs/br71-execution-plan.md:12496, 13868, 13889, 13923` — both are execution plans of *completed/approved* work quoting the code as it stood. Per `docs/organization.md` §3 they are records; leave as-is. (If you touch privacy-tiers, note `:13011` claims `McpAppRenderer.tsx:154` is "the ONLY" desktop caller of `/agent/call_tool` — after removal that statement becomes vacuously stronger, not weaker.)
- `docs/design/astryx-adoption/*.html` — generated showcase/spec artifacts.

**Leave untouched (history / releases):** `docs/history/agent-loop-review/improvement-proposals.md`, `.../proposal-lenses/performance.md`, `.../subsystem-reviews/server-reply-flow-and-session-lifecycle.md`, `docs/history/chat-groups/design-judgement-and-plan.md`, `docs/history/performance-2026-06/review-findings.md`, `docs/releases/notes/v1.89.0.md`.

**Where the removal record goes** (`docs/organization.md` §3, lines 144-150): create **`docs/history/mcp-apps-removal/`** with a `README.md` whose first paragraph gives the date, says the feature was inherited from the upstream fork at `54153b8b (2026-01-20 initial commit)`, was never requested, and was removed; and says where the current truth lives (`docs/desktop-ui/artifact-display-surfaces.md`). The exact precedent is `docs/history/dashboard-mode/` — README + `dashboard-mode-removal-spec.md` for a feature built and then deleted. Optionally add `mcp-apps-removal-spec.md` (this map). Then add the folder to `docs/history/README.md`'s index (§6: every folder carries a README, and history has an index).

---

### 5. Capability lost

**Third-party MCP servers lose the ability to ship an interactive UI that runs inside BioRouter.** Concretely: a server that (a) advertises `resources` capability and publishes a `ui://` resource, or (b) returns `_meta.ui.resourceUri` on a tool result, gets a sandboxed iframe running its own HTML — with a two-way JSON-RPC bridge (`ui/open-link`, `ui/message` to append into the chat, `tools/call`, `resources/read`, `notifications/message`), a per-extension CSP declared via `_meta.ui.csp`, and a standalone OS window via the sidebar's Launch button. All of that goes. No shipped extension in `landing/registry.json`'s catalog is affected as far as I could see, but I could not verify third-party servers' runtime behaviour — see §8.

**Proof that Auto Visualiser and Agent Drafter do NOT use this path:**

1. **Capability gate.** `get_ui_resources` filters on `ext.supports_resources()` (`extension_manager.rs:2492`), i.e. the server's `ServerCapabilities`. Auto Visualiser: `crates/biorouter-mcp/src/autovisualiser/mod.rs:516` — `ServerCapabilities::builder().enable_tools().build()`, **no** `.enable_resources()`. Agent Drafter: `agent_drafter/mod.rs:3353`, `agent_drafter/control.rs:4238`, `agent_drafter/evidence.rs:149` — all `.enable_tools()` only. Neither can ever appear in `GET /agent/list_apps`.
2. **Different transport.** Auto Visualiser returns its `ui://` figure as an **embedded resource in the tool result** — `crates/biorouter-mcp/src/autovisualiser/common.rs:403-415` (`Content::resource(ResourceContents::BlobResourceContents{ uri, mime_type: "text/html", blob })`). That is consumed by `ToolCallWithResponse.tsx:540-548` (`isEmbeddedResource` + `isUIResource`) → `MCPUIResourceRenderer`, which emits a click-to-open card and calls `onOpenArtifact` (`MCPUIResourceRenderer.tsx:40-49`) into the artifact side panel. It never touches `McpAppRenderer` or `/mcp-app-proxy`.
3. **The discriminator is never set by us.** `grep -rn "resourceUri\|profile=mcp-app" crates/ --include=*.rs`, excluding `biorouter_apps/`, returns **zero** hits. Independently corroborated by `docs/desktop-ui/artifact-display-surfaces.md:81`.

**No built-in server emits an MCP-app resource.** The only built-in that enables resources at all is Computer Controller (`crates/biorouter-mcp/src/computercontroller/mod.rs:1610-1613`), and its `active_resources` are keyed by `Url::from_file_path(...)` (`:1557`, inserted at `:659-666`) — `file://` URIs, which `get_ui_resources`' `ui://` filter (`extension_manager.rs:2515`) drops. It is unaffected.

**Also unaffected:** the whole MCP extension system, workspace control, `/tool_bridge` (its auth exemption is a separate branch at `auth.rs:203-205`), `@mcp-ui/client` and `isUIResource`, `/agent/call_tool`, `/agent/read_resource`.

---

### 6. Order of operations

1. **Server first.** `lib.rs` → `routes/mod.rs` → `auth.rs` → `routes/agent.rs` → `openapi.rs`; delete `biorouter_apps/` and `mcp_app_proxy.rs` + `routes/templates/`. Decide on `get_ui_resources` (delete + fix the three Gate-C test sites, or leave).
   `cargo check -p biorouter -p biorouter-server` (**not `cargo build` in this tree**).
2. **Regenerate the contract.** `just generate-openapi` (runs `generate_schema` then `npm run generate-api`). This rewrites `ui/desktop/openapi.json`, `types.gen.ts`, `sdk.gen.ts`, `index.ts`. Do this *before* the renderer edits so TypeScript points at every stale `listApps`/`BioRouterApp` import.
3. **Renderer.** `App.tsx` → `AppSidebar.tsx` → `ToolCallWithResponse.tsx` → `chatStreamStore.tsx` → `main.ts` / `preload.ts` / `renderer.tsx` → `entity-icons.ts`; delete `components/apps/` and `components/McpApps/`; decide on the `append` chain.
4. **Tests.** `measures.test.ts` and `settingsVocabulary.test.ts` **before** running the suite — both fail at module load otherwise.
5. **Docs**, then the `docs/history/mcp-apps-removal/` record.

**Verification:**
```bash
cargo check -p biorouter -p biorouter-server
cargo test -p biorouter-server --lib routes::
cargo test -p biorouter-server --lib -- auth
cargo test -p biorouter --lib -- extension_manager      # Gate-C siblings, if you deleted get_ui_resources
cargo test -p biorouter --lib privacy::
just generate-openapi && git diff --stat -- ui/desktop/openapi.json ui/desktop/src/api   # then re-run; second run must be a no-op
just check-openapi-schema                                # scripts/check-openapi-schema.sh
cd ui/desktop && npm run test:run && npm run lint:check && npm run format:check
```
`scripts/check-brand-consistency.sh` is **not relevant** — it has no MCP-apps assertions (grepped). Run `just check-everything` at the end regardless.

Also: `BIOROUTER_DISABLE_KEYRING=true` on any macOS `cargo test` that reaches `biorouter-server`, per the known Keychain deadlock.

---

### 7. Final sweep (should return nothing but history/release docs)

```bash
grep -rn "biorouter_apps\|McpApp\|mcp-app\|mcp_app\|listApps\|list_apps\|standalone-app\|mcp-apps-cache" \
  crates/ ui/desktop/src/ docs/ landing/ --include=* | grep -v ui/desktop/src/web/assets
```
(`ui/desktop/src/web/` is the generated browser bundle — gitignored, rebuilt by `just build-web`.)

---

### 8. What I could not determine

- **Whether any real third-party extension actually uses this.** `SPOKEAgent`, `UCSFOMOPAgent`, `CDWAgent`, `PlaywrightAgent`, `CodeGraphAgent`, `BiorOffice` live in other repos; I only read this tree. If one of them declares `ui://` resources, its users lose that UI. Worth one grep of those repos before merging.
- **Whether `docs/desktop-ui/preview-panel/current-state.md:158`'s claim that `http:` is load-bearing for the MCP-app proxy iframe means a CSP allowance can now be tightened.** The claim is about `isAllowedArtifactFrameNavigation`-adjacent policy; I did not trace it to the live code. Flag for the implementer as a possible follow-up narrowing, not part of this removal.
- **Whether `/agent/read_resource` has any non-frontend consumer.** It has none in this repo after the removal, but it is a generic MCP capability an external client could call. I left it in place deliberately.
---

### Coordinator addendum (2026-09-08 22:35 UTC) — ecosystem check done

All six ecosystem extension repositories were checked for MCP-UI usage (`ui://` and
`resourceUri`): BaranziniLab/SPOKEAgent, UCSFOMOPAgent, CDWAgent, PlaywrightAgent (GitHub code
search) and Broccolito/CodeGraphAgent (code search), Broccolito/BiorOffice (shallow clone +
grep, also `enable_resources`/`list_resources`): **zero hits in every repository**. No installable
Biorouter extension loses a UI. Cite this in the PR body's "Capability lost" section.
