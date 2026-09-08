# MCP apps — removal record

> **What this is.** The record of an inherited desktop feature — **MCP apps**, a sandboxed iframe in which a third-party MCP server ran its own interactive HTML inside BioRouter — and of its removal. Nothing here describes a feature that still exists; the folder is an archive.
> **Status:** Historical record — MCP apps was removed on 2026-09-08. The `/apps` route and its sidebar row, `GET /agent/list_apps`, the `/mcp-app-proxy` sandbox document, the `crates/biorouter/src/biorouter_apps/` module, the `components/McpApps/` renderer and the `launch-app` IPC handler are all deleted. Current truth about where a generated artifact is displayed lives in [Where a generated artifact is displayed](../../desktop-ui/artifact-display-surfaces.md).
> **Audience:** maintainers tracing why the feature is gone, and anyone tempted to reintroduce it.

## Files in this folder

| File | What it is |
|---|---|
| [MCP apps removal map](mcp-apps-removal-spec.md) | The read-only survey the removal was carried out from — every deletion with its blast radius, every edit with a file and line, the tests that had to change, and the proof that Auto Visualiser and Agent Drafter do not use this path. Kept for provenance; this index carries the same record in conformed prose. |

The map is a dated record of the tree at `6c5bf2c9`. **Do not rewrite it** to match current reality; where it and the code disagree, the code is the authority.

## What MCP apps was

An MCP server could ship a user interface, and BioRouter would run it. Two entry points fed one renderer:

- **A page.** `GET /agent/list_apps` swept every loaded extension whose `ServerCapabilities` advertised `resources`, kept the resources whose URI began `ui://`, and listed them at `/apps` under a sidebar row labelled **MCP apps**. Results were cached on disk at `~/.config/biorouter/mcp-apps-cache`, so the page had something to show before any session existed. A Launch button opened one in its own Electron window at `#/standalone-app`.
- **A tool result.** When a tool response carried `_meta.ui.resourceUri`, the transcript drew that app inline instead of the ordinary artifact card.

Either way the app ran in a sandboxed iframe served by `/mcp-app-proxy` — a route deliberately exempt from the daemon's secret-key middleware, gated instead by a short-lived minted token — with a two-way JSON-RPC bridge back to the host: `ui/open-link`, `ui/message` (append text into the chat), `tools/call`, `resources/read`, `notifications/message`, and a per-extension CSP the extension declared through `_meta.ui.csp`.

## Why it was removed

**It was inherited, not chosen.** The whole feature arrived with the initial import from the upstream fork — commit `54153b8b`, 2026-01-20, "initial commit" — and was never requested, designed or scheduled by this project. It is not part of BioRouter's product story: the operator's decision on 2026-09-08 was that it does not belong to Biorouter and should be removed completely rather than maintained.

Two things made that cheap:

1. **No Biorouter code could ever reach it.** The tool-result path keys on `_meta.ui.resourceUri`, and no Rust code in this repository has ever emitted that key. The page path filters on `supports_resources()`, and neither Auto Visualiser (`autovisualiser/mod.rs`) nor Agent Drafter (`agent_drafter/mod.rs`, `control.rs`, `evidence.rs`) enables the `resources` capability at all — every one of them builds `ServerCapabilities::builder().enable_tools()`. The one built-in server that does enable resources, Computer Controller, keys its resources by `file://` URI, which the `ui://` filter drops.
2. **No installable extension used it.** All six ecosystem extension repositories — `SPOKEAgent`, `UCSFOMOPAgent`, `CDWAgent`, `PlaywrightAgent`, `CodeGraphAgent` and `BiorOffice` — were searched for `ui://`, `resourceUri`, `enable_resources` and `list_resources` before the removal. Zero hits in every repository.

It also carried a second, parallel "apps" concept. The sidebar shipped two adjacent rows a user could not tell apart: **Built apps** (`/applications`, Agent Drafter's `GET /apps`) and **MCP apps** (`/apps`, `GET /agent/list_apps`). The astryx UI adoption design had already argued that "there is no Apps — there is Applications" and proposed folding the second into the first; the removal settles that by deletion instead.

## Exactly what was removed

| Thing | Detail |
|---|---|
| Core module | `crates/biorouter/src/biorouter_apps/` — `BioRouterApp`, `WindowProps`, `McpAppCache`, `fetch_mcp_apps`, and `resource.rs`'s `McpAppResource` / `UiMetadata` / `CspMetadata` / `ResourceMetadata` |
| Route | `GET /agent/list_apps` with `ListAppsRequest` / `ListAppsResponse`, and its nine utoipa registrations in `openapi.rs` |
| Proxy | `crates/biorouter-server/src/routes/mcp_app_proxy.rs` (627 lines, 7 unit tests) and its `routes/templates/mcp_app_proxy.html`, which was the only file in that directory |
| Auth exemption | `"/mcp-app-proxy"` in `is_unauthenticated_path`. The exempt set is now `/status`, `/ui/workspace` and the `/tool_bridge/{nonce}` prefix rule |
| Extension manager | `ExtensionManager::get_ui_resources`, whose only non-test caller was `fetch_mcp_apps` |
| Renderer | `ui/desktop/src/components/McpApps/` (`McpAppRenderer`, `useSandboxBridge`, `utils`, `types`) and `ui/desktop/src/components/apps/` (`AppsView`, `StandaloneAppView` and both test files) |
| Routes and rail | The `/apps` and `/standalone-app` routes in `App.tsx`; the "MCP apps" sidebar row, the `listApps` probe that decided whether to show it, and the resulting `visibleComponentItems` filter |
| Electron | The `launch-app` IPC handler in `main.ts`, its `preload.ts` bridge and type, and the browser-mode `window.open` shim in `renderer.tsx` |
| Cache warm | The fire-and-forget `listApps` call in `hooks/chatStreamStore.tsx` |
| Icon | `ENTITY_ICONS.mcpApp` and the `'mcpApp'` member of `EntityKind` |
| Disk | Nothing is written to `~/.config/biorouter/mcp-apps-cache` any more. **It is not deleted on upgrade** — an install that ran an older build keeps the directory until the user removes it. It holds no credentials, only cached app metadata |

## What deliberately stayed, and why

- **Agent Drafter's Built apps** — `/applications`, `routes/apps.rs`, `GET /apps` and the `agent_drafter__list_apps` MCP tool. A different feature with a different type (`Manifest`, not `BioRouterApp`) and a different data source. The two shared nothing but a word.
- **`POST /agent/read_resource`.** After the removal it has no caller in this repository; its only one was the deleted renderer. It stays because it is a generic MCP capability rather than MCP-apps machinery, and an external client could legitimately call it. Deleting it is a separate decision.
- **`ExtensionManager::supports_resources()`** — still called from three places, and unrelated to the `ui://` sweep that was removed.
- **`@mcp-ui/client`, `isUIResource` and `MCPUIResourceRenderer`.** These are the *artifact* path, not the MCP-apps path: an Auto Visualiser figure or an Agent Drafter card arrives as an embedded `ui://` resource in the tool result's `content`, and becomes a click-to-open card that opens in the artifact side panel. Removing the MCP-apps discriminator made that path unconditional, which is the one behavioural improvement in this change.
- **`/tool_bridge`.** Its auth exemption is a separate branch in `is_unauthenticated_path` and was not touched.
- **The whole MCP extension system**, workspace control, and `POST /agent/call_tool`.
- **The backend ref-count in `main.ts`.** `retainBackend` / `releaseBackend` existed so a launched app window could share the launcher's daemon. With `launch-app` gone every window retains its own backend and the count is 1 in practice, but the mechanism is kept: a release path that assumed sole ownership would be wrong the day a shared-backend window returns.

## What was lost

Third-party MCP servers can no longer ship an interactive UI that runs inside BioRouter. A server that advertised `resources` with a `ui://` resource, or returned `_meta.ui.resourceUri` on a tool result, got a sandboxed iframe running its own HTML with a JSON-RPC bridge, its own CSP, and a standalone window. That capability is gone, and reinstating it would mean rebuilding the proxy, the bridge and the per-extension CSP rather than reverting a flag.

Nothing shipped in this project used it, and no ecosystem extension did either — but a third party's server, written against the upstream fork's contract, would lose its UI and fall back to the ordinary tool-call row. A tool response that also embeds a `ui://` resource in its `content` now takes the artifact path instead, which is a better outcome than it had before.

## Open follow-up

`docs/desktop-ui/preview-panel/current-state.md` recorded `http:` in `frame-src` as load-bearing **for this proxy iframe**, which ran at the daemon's origin. With the proxy gone, whether that allowance can now be dropped is an open question — the claim was never traced to the live policy, and the removal does not answer it.

## Related documentation

- [Where a generated artifact is displayed](../../desktop-ui/artifact-display-surfaces.md) — the current rule, and the record of the earlier inline-renderer removal this one completes.
- [The astryx UI adoption design](../../design/astryx-adoption/astryx-ui-adoption-design.md) — proposed folding the two apps rows into one; annotated to record that the question was settled by removal instead.
- [The preview panel — current state](../../desktop-ui/preview-panel/current-state.md) — carries the open CSP question above.
- [Agent Drafter apps platform design](../../agent-drafter/apps-platform-design.md) — the feature that keeps the word "apps" and was never part of this one.
