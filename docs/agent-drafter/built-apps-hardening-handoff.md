# Built apps hardening: handoff

> **What this is.** The handoff for the October 2026 pass over Agent Drafter built apps: the
> Applications view's Launch, chat and Export buttons, and the build, serve and export pipeline
> behind them. It says what is done, how it was verified, and exactly what is left.
> **Status:** In progress. Implementation is complete in five areas; three review fix passes and
> the live GUI verification are not finished.
> **Audience:** The next agent or maintainer picking this branch up.

## Where the work is

- Worktree: `/Users/wgu/Desktop/BioRouter/.claude/worktrees/biorouter-built-apps-testing-cc226e`
- Branch: `claude/biorouter-built-apps-testing-cc226e`, based on `main` at `30e1f0622`.
- All work is in one commit on top of that base (see "Commits" below). About 60 files,
  +20k/-1.9k lines including tests and docs.
- The tree compiles (`cargo check -p biorouter-mcp -p biorouter-server -p biorouter-cli --tests`)
  and typechecks (`cd ui/desktop && npx tsc --noEmit -p tsconfig.json`), measured at handoff.
- Durable notes (outside the repo, not committed):
  `/Users/wgu/biorouter-runs/built-apps-qa/notes/`
  - `audit.md`: the full read-only audit, about 70 findings with file:line evidence, each high
    finding adversarially verified (none refuted). Start here for any "why".
  - `fix-input-security-lifecycle.json`, `fix-input-export.json`, `fix-input-build-serve.json`:
    the open review findings below, with scenario and proposed fix for each.
  - `resume-workflow-reports.json`, `implement-workflow-reports.json`: every agent's full report
    (changes, tests run with counts, decisions, unfinished items).
- Agent transcripts (JSONL, large) for the interrupted fix passes, under
  `/Users/wgu/.claude/projects/-Users-wgu-Desktop-BioRouter--claude-worktrees-biorouter-built-apps-testing-cc226e/64ec303b-fe39-4aa9-86ef-23dc8223d6a1/subagents/workflows/wf_2d8f17a3-25c/`:
  security `agent-a3a0dd547bfbd325b.jsonl`, build/serve `agent-ac72da2eac86b3a10.jsonl`,
  export `agent-a84d6dd8a18f25d4a.jsonl`. They may have partly applied fixes before being cut
  off; read their tails before redoing anything.
- Live sandbox used for reproduction: `/Users/wgu/biorouter-runs/built-apps-qa` (copied seed
  config with a working Versa Azure key, two apps built by GPT-5.5, a `lab-reference` knowledge
  base and a `lab-units` skill for full-export tests). The dev GUI there is stopped. Relaunch with
  `bash ~/biorouter-runs/launch-dev-gui.sh start <worktree> built-apps-qa 9487` after
  `chmod 700 ~/biorouter-runs/built-apps-qa/electron` (the app now refuses a user-data-dir that is
  not 0700, which the launcher does not set). It runs the worktree's `target/debug/biorouterd`,
  which must be built with `--features biorouter/privacy-test-auth`.

## The three reported problems and their root causes

1. **"Launch in browser" showed "Access to 127.0.0.1 was denied".** Reproduced in the installed
   1.92.1 and in a sandboxed dev GUI. On macOS and Linux the desktop reaches a shared daemon
   through a local proxy in the Electron main process (`ui/desktop/src/daemonRuntime.ts`). The
   launch link was joined to the proxy's address, and the proxy answers anything without the
   desktop secret with an empty 403, which Chrome renders as that page. The daemon's own listener
   accepted the same token. The in-chat app preview had the same root cause.
2. **"Open the chat where this app was built" failed with "Session not found"** when the chat had
   been deleted, and the dead tab was persisted and came back after reloads.
3. **Export.** "Full" and "Bundle daemon" were silently ignored (the HTTP route dropped the query
   parameters), `run.command`/`run.sh` were written without the exec bit, every binary file was
   dropped, and the exported `index.html` was already assembled, so an exported agentic app could
   never reach its agent (two config islands; the SDK read the one without a socket token).

## What is done

Each area was implemented, reviewed by two adversarial reviewers (correctness, security) and,
where marked, fixed. Test counts are from the agents' final runs.

### Launch (done, reviewed, fixed)

- `POST /apps/{id}/launch` answers `{path, url}`; `url` is the daemon's own loopback origin
  (`BIOROUTER_APP_BASE_URL`, `0.0.0.0` normalised). The desktop mints through the proxy and sends
  the browser to `url`. The proxy stays secret-only; a regression test proves it still refuses app
  requests without the secret. No passthrough was added (it would have injected the daemon secret).
- The launch page opens in the default web browser (https handler), not the `.html` association.
  Snap browsers on Linux get a "copy a one-time link" dialog instead.
- Distinct error messages (app gone, refused connection, daemon restarted, unreachable).
- In-chat managed preview learns the daemon's app origin from `/headless/health`; links on a stale
  port are rebound by path; previews are aborted and rebound on daemon loss or replacement.
- "Open in browser" / Expand on a local app link routes to the same launch path and toasts failures.
- `biorouter apps open|serve` use the profile's shared daemon first; `/status` must answer `ok`;
  Windows detached daemon gets its own hidden console.
- Live: verified against a real shared daemon behind the real proxy (19 checks), plus CLI runs.

### Chat link (done, reviewed, fixed)

- `ui/desktop/src/components/applications/buildChat.ts` resolves each app's build chat with the
  privacy-gated `GET /sessions/{id}?metadata_only=true` (sent with the user-action proof). States:
  available, deleted (404/400), replaced (chat newer than the app, or not active when the app was
  created), unreachable (403), unknown, none. A subagent-built app opens the parent chat.
- The button is always rendered. When the chat is gone it is `aria-disabled` (still focusable,
  with a tooltip) and the row shows a visible note such as "Build chat deleted". Clicking
  re-checks first and never opens a dead tab. Deletions in any window flip the row immediately.
- Exports no longer carry `session_id`, so an installed export cannot point at an unrelated chat.
- Applications view lists apps whose manifest cannot be read, with Delete only.

### Export (done, reviewed; fix pass interrupted)

- One Rust implementation, `crates/biorouter-mcp/src/agent_drafter/export.rs`, used by the
  `export_app` tool, the desktop and `biorouter serve`.
- `POST /apps/{id}/export` answers a zip (unix modes kept) with an `X-Biorouter-Export-Report`
  header; knowledge bases go through the `HttpCaller` tier gate (private bases need the
  user-action proof; refused ones are skipped with a note). The legacy `GET` is kept for version
  skew, launcher-only and fixed.
- Desktop: `apps:export` IPC (`ui/desktop/src/utils/appExport.ts`) shows the folder picker first,
  extracts the zip safely, applies 0755, asks before replacing and moves the old folder to the
  Trash. The browser surface downloads the zip instead of prompting for a server path.
- Contents: raw entry HTML, binaries byte-exact, `workspace/`, `.vault`, `.git`, `.env*` and key
  files excluded anywhere, reserved launcher names protected, include semantics explicit
  (`[]` means none), skill names validated with no symlink following, built-ins filtered.
- Launchers: correct store path per OS (Windows was wrong), never clobber a different app, bash
  arrays, 60 s readiness with kill on timeout, private log dir, bundled daemon used only if it runs,
  hardened `serve.mjs`.
- Live: exercised on real daemons (ports 47311-47313), not through the GUI.

### Build and serve (done, reviewed; fix pass interrupted)

- esbuild now ships with the desktop: `ui/desktop/scripts/fetch-esbuild.js` (pinned 0.27.3,
  sha512-checked, called from `prepare-platform-binaries.js`) stages `src/bin/esbuild`;
  `find_esbuild` checks next to the executable first and pins the npx fallback. The user's machine
  had three apps with broken fallback bundles because the packaged daemon had no esbuild.
- The type-stripper fallback refuses local and multi-line imports, scopes `main.ts`, never
  overwrites an esbuild bundle and is never stamped fresh.
- Staleness includes `built_at` and source mtimes; never rebuilds to an older SDK; builds are
  serialised per app and written atomically; a failed build serves an error page with the log
  instead of a blank page, and is not retried on every load.
- Static apps: authored inline scripts and handlers run under per-response CSP hashes;
  `dist/app.js` is injected only when it exists; assembly is idempotent (heals stores damaged by
  old exports) and the Unicode offset bug is fixed. App pages send `frame-ancestors 'none'` and
  COOP.
- Lifecycle: static to agentic seeding, kind-specific notes, `smoke_app` messaging.
- Live: packaged-like daemon with bundled esbuild and a minimal PATH, real Chrome.

### Security and lifecycle (done, reviewed; fix pass interrupted)

- Fetch Metadata gate: an app page is served only to a navigation, so one app's script cannot
  fetch another app's page and token; `runstate`/`models` need the page token. Serve shell gets
  `frame-ancestors 'none'`, COOP and the same navigation gate.
- Figures inside apps render through a same-origin figure-frame document with its own policy
  (they were blank under the app CSP). Also fixed an SDK bug that pinned figure frames at 360 px.
- `app_uid` in the manifest and the durable session key, so a deleted and re-created app never
  resumes the old conversation; delete forgets tokens, cookies, UI bridges and the vault key;
  `create_app` with an existing explicit id is refused unless `replace: true`.
- Live: two apps in a real Chromium, cross-app fetch and iframe refused, figures draw.

## What is left

### 1. Finish the three interrupted review fix passes

For each finding: check the code (the interrupted fixer may have already fixed it), fix with a
test or record why it is not a bug. Full scenario and proposed fix for each are in the
`fix-input-*.json` files.

Security and lifecycle (`fix-input-security-lifecycle.json`):
- [high] The Fetch Metadata gate and opener policies do nothing on a plain-HTTP LAN
  `biorouter serve` (browsers send no Fetch Metadata to non-secure origins), so an app can still
  read the serve shell's secret there. `crates/biorouter-server/src/auth.rs:636`. Also correct the
  docs that say the shell is never served to a fetch.
- [medium] Same gate fails open on `serve --host 0.0.0.0`. `auth.rs:638`.
- [medium] Figures are still blank in the desktop in-app preview: the preview request filter
  (`ui/desktop/src/utils/managedAppPreviewPolicy.ts` `isManagedAppRequest`) cancels the new
  figure-frame subframe. `templates/sdk.ts:3948`.
- [low] A figure frame that reloads after a DOM move stays blank (SDK answers the ready message
  once). [low] `forget_app_runtime` drops a UiBridge the cached agent still uses. [low] Factory
  reset bypasses the `on_app_deleted` hook (`routes/reset.rs:204`). [low] Page-token checks mint
  and keep tokens for any id (`APP_WS_TOKENS` growth). [low] Case-insensitive filesystem delete
  forgets the wrong id. [low] A deleted app's open socket keeps file access to the folder a
  replacement takes over.

Export (`fix-input-export.json`):
- [medium] Launcher re-sync overwrites newer store contents without a backup and leaves a stale
  manifest stamp (`render.rs:1056`).
- [medium] `export_app replace:true` permanently deletes the earlier export folder with no
  confirmation (`export.rs:567`); move it aside or to the Trash instead.
- [low] Full export carries Biorouter's own shipped skills and the recipient's launcher replaces
  their shipped copy. [low] `apps:export` reads the user-action proof from current settings, not
  the attached daemon (`main.ts:3205`). [low] The `export_app` target check is a deny list, not an
  allow list (`export.rs:1063`).

Build and serve (`fix-input-build-serve.json`):
- [medium] Building to a temp outfile means `dist/app.css` is never written and hidden CSS files
  pile up (`bundle.rs:1556`).
- [medium] No-downgrade protection lasts one build: the banner records the builder's version, not
  the SDK's (`mod.rs:1489`).
- [medium] SDK refresh and stamp only recognise the literal `"./sdk"` import (`./sdk.js`,
  `./sdk.ts` apps never refresh) (`bundle.rs:1474`).
- [medium] `'unsafe-hashes'` lets runtime-injected HTML run any handler the author wrote
  (`render.rs:444`). Consider rewriting authored `on*=` handlers to listeners at serve time, or
  dropping handler support and linting instead.
- [medium] The npx cache is a predictable path in shared `/tmp`, so a planted esbuild could run
  (`bundle.rs:1006`). Use a per-user 0700 cache dir.
- [low] A refused fallback build still rewrites `src/sdk.ts`. [low] `ArtifactKind::parse("")` now
  returns `None`, so `list_apps` with `kind:""` errors.

### 2. Small follow-ups the agents named but did not own

- `ui/desktop/src/components/MCPUIResourceRenderer.tsx:45` still calls
  `window.electron.openExternal` directly; change to `await openInBrowser(...)` so failures toast.
- `ui/desktop/scripts/appcheck/export-check.mjs` and `export-all.mjs` expect the old assembled
  `index.html`; update their checks to "no `biorouter-app-config` island".
- The generic `write-file` IPC in `main.ts` is still unrestricted (used by `BioRouterHintsModal`
  and `WorkflowsView`); restrict it to `allowedFileRoots()` like `delete-file`.
- `scripts/build-cli-linux-packages.sh` ships no esbuild for CLI-only Linux packages.
- Dead templates `templates/agent.js`, `resize.js`, `starter.html` (only
  `scripts/agent-drafter/preview-runtime-test.mjs` reads them).
- Chat link: a forge-proof identity check would stamp `app_uid` on the session row at
  `create_app`.

### 3. Integration checks (not yet run on the combined tree)

```bash
cargo fmt
./scripts/clippy-lint.sh
cargo test -p biorouter-mcp --lib agent_drafter::
cargo test -p biorouter-server --lib -- routes::apps auth:: routes::web_ui
cargo test -p biorouter-cli --lib commands::apps
cargo test -p biorouter-mcp --test ui_example_apps
cargo test -p biorouter --test privacy_guard_wiring
cd ui/desktop && npm run lint:check && npm run format:check && npm run test:run
```

Known: clippy `string_slice` warnings at `routes/apps.rs` (build-log page and an export test);
rustfmt diffs in shared files `mod.rs`, `render.rs`, `routes/apps.rs`. The census test
(`privacy_guard_wiring`) gained rows from the export route; confirm it passes.

### 4. Live verification in the real GUI (not done)

Nothing has been clicked in a running Electron app since the fixes. With the sandboxed GUI
(see above), verify:
- Launch in browser on the static app and the agentic app: the launch page under
  `<user-data-dir>/app-launch/` must point at the daemon port, not the proxy port; the app loads
  and the agentic app's agent answers a prompt (GPT-5.5 via Versa; never local models).
- The in-chat app card and "Open in browser".
- Delete the build chat of one app; its row must gray out with "Build chat deleted" and clicking
  must not open a tab.
- Export: Launcher, Full (with `lab-reference` and `lab-units` configured on the agentic app) and
  Bundle daemon. The native folder picker needs computer-use or a manual click. Then run each
  export's `run.sh` with a fresh `HOME`/`BIOROUTER_PATH_ROOT` and confirm the app opens and its
  agent connects.
- An agentic app with a figure, and a static app with an inline script.

### 5. Ship

- Split the single commit into reviewable commits by area if wanted (launch, chat link, export,
  build/serve, security/lifecycle, docs).
- Update `CLAUDE.md`'s Agent Drafter section: launch goes to the daemon origin, esbuild ships with
  the desktop, exports are zips built by `export.rs`, app pages require a navigation.
- Push, open a PR against `main` (no `Co-Authored-By` trailer: the `no-ai-coauthor` check rejects
  it), then read CI.

## Known gaps kept on purpose

- Per-app origins (`<id>.localhost`) are the real fix for apps sharing one origin; only the
  interim navigation gate is built. Documented as SD-14 and in the deployment docs.
- App routes have no privacy reach gate for private apps; it needs an app classification design.
  Recorded in `docs/security/privacy-tiers.md`.

## Related documentation

- [Apps platform design](apps-platform-design.md)
- [Apps SDK reference](../apps-sdk/sdk-reference.md)
- [Browser access (`biorouter serve`)](../deployment/browser-access.md)
- [Serve decisions](../deployment/serve-decisions.md)
- [Privacy tiers](../security/privacy-tiers.md)
