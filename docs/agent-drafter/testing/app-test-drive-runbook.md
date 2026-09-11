# Agent Drafter 100-app test-drive runbook

> **What this is.** The operational runbook for driving BioRouter's Agent Drafter across the 100 app specs in the companion corpus: bring a daemon up, make the agent author each app, drive the result in a browser against a functional and an aesthetic rubric, and log every defect, gap and friction point.
> **Status:** Current, with one rotted section named here. The runbook was written 2026-07-12 against a `feat/apps-sdk-v2` git worktree at `/Users/wanjun/Desktop/biorouter-sdk-v2-wt`, which no longer exists and is not a registered worktree. The Apps SDK v2 primitives it depends on now live in the main tree (`crates/biorouter-mcp/src/agent_drafter/`), so the worktree requirement in "Where the code lives" (§0.1) is void — run everything else from an ordinary checkout. The campaign this runbook drove is recorded in [the 100-app test-drive archive](../../history/agent-drafter-testdrive-100/README.md).
> **Audience:** coding agents running an Agent Drafter test campaign, and the maintainers supervising them.

Agent Drafter is BioRouter's app-authoring MCP extension: it builds **BioRouter apps**, each a served TypeScript front-end wired to its own per-app BioRouter agent over a WebSocket. This runbook tests it end to end — not by hand-writing apps, but by making the agent author them from a spec and then checking whether what came out is the app the spec asked for.

**Identifier scheme used throughout.** Sections are numbered (`§0`–`§10`) and individual rubric checks carry the section number as an ID (`5.2` = "It is not a chatbot", `6.1` = "Theme pack applied"). Those check IDs are cited by the per-app result files the executed run produced, so they are kept verbatim. `spec-NNN` refers to a numbered brief in the corpus — spec 1 is `spec-001`; app ids follow `spec-NNN-<slug>`, for example `spec-001-variant-tribunal`.

**Prerequisites.** You need shell access and a computer-use / browser-automation tool (Playwright MCP or equivalent). Your job is to take each of the **100 app specs** in [the 100 agentic app test specs](hundred-app-test-specs.md) and, for each one:

1. **Make Agent Drafter author the app** — do not hand-write the app yourself; the whole point is to test Agent Drafter, so the *BioRouter agent* must build it via its `create_app` / `build_app` tools while you converse with it.
2. **Iterate conversationally** with that agent — ask it questions, answer its questions, grow its capabilities, and keep refining until the app matches the spec (functionally *and* aesthetically).
3. **Verify with computer-use** — drive the running app in a browser, confirm it is functionally correct and aesthetically aligned with the spec, exercising the agent-driven loop end to end.
4. **Keep a comprehensive findings log** — record every bug, inconsistency, missing capability, and inefficient back-and-forth, so the team can improve Agent Drafter later. This log is a primary deliverable, not an afterthought.

Read this whole document before you start. It encodes hard-won operational detail (ports, keys, gotchas) from a real run.

---

## 0. Where the code lives

The Apps SDK v2 primitives — declared `surface`, `ui_patch`, `app_call`, `signals`, `br.kb`, multi-agent profiles, theme packs, archetype starters, strict CSP, ws-token auth — now live in the main tree under `crates/biorouter-mcp/src/agent_drafter/`. An ordinary checkout is sufficient; substitute your checkout path for `$REPO` everywhere below.

Two reference documents are your background material — read them:

- [Apps SDK reference](../../apps-sdk/sdk-reference.md) — the human-facing SDK reference (manifest surface, every `br.*` API, all `ui_*` tools, the widget catalog, the frame protocol, capability matrix, export).
- [Agent Drafter apps platform design](../apps-platform-design.md) — the design plus the "SDK v2" section.

### 0.1 As originally written — no longer applicable

> **Warning.** The following requirement held on 2026-07-12 and does not hold now. It is preserved because the run archived under `docs/history/agent-drafter-testdrive-100/` was executed under it. Do not act on it.

All of the Apps SDK v2 work lived on a **git worktree**, not on `main`:

```text
Worktree:  /Users/wanjun/Desktop/biorouter-sdk-v2-wt
Branch:    feat/apps-sdk-v2
```

The original instruction was to operate from this worktree, because `main` did not have the v2 SDK primitives and building the specs against `main` would fail or silently produce v1 chatbot-shaped apps. The confirmation step was:

```bash
cd /Users/wanjun/Desktop/biorouter-sdk-v2-wt
git rev-parse --abbrev-ref HEAD    # must print: feat/apps-sdk-v2
git log --oneline -1               # should be an "SDK v2 …" commit
ls docs/agentic-app-test-ideas-100.md docs/apps-sdk-reference.md   # both must exist
```

Both of those documents have since moved: the spec corpus is now `docs/agent-drafter/testing/hundred-app-test-specs.md` and the SDK reference is now `docs/apps-sdk/sdk-reference.md`.

---

## 1. Bring the environment up

> **Note.** The scratch paths below (`/tmp/br-testdrive-target`, `/tmp/br-testdrive.env`, `/tmp/br-daemon.log`, `/tmp/br-testdrive/`), the port `8899`, and the provider `versa_azure` are the values the recorded run used. They are conventions, not requirements — substitute your own, but keep them consistent across the whole batch.

### 1.1 Toolchain

Point `REPO` at your checkout once, then in every shell:

```bash
export REPO=/path/to/your/biorouter/checkout
cd "$REPO"
source bin/activate-hermit          # Node 24 + cargo toolchain; run first, every shell
```

Use an **isolated cargo target dir** so you don't fight other builds:
`export CARGO_TARGET_DIR=/tmp/br-testdrive-target` (any path). Do this in every cargo command.

### 1.2 Build the daemon and CLI (they carry the SDK v2 changes)

```bash
CARGO_TARGET_DIR=/tmp/br-testdrive-target cargo build -p biorouter-server --bin biorouterd
CARGO_TARGET_DIR=/tmp/br-testdrive-target cargo build -p biorouter-cli    --bin biorouter
```

A prior build of `biorouterd` on disk may be **stale** (predating later phases). If you pulled or switched branches, **rebuild** — don't trust an old binary. The binaries land at
`/tmp/br-testdrive-target/debug/{biorouterd,biorouter}`.

### 1.3 esbuild (the app bundler)

The daemon bundles each app's TypeScript with esbuild. Point it at the checkout's copy:

```bash
export BIOROUTER_ESBUILD_BIN="$REPO/ui/desktop/node_modules/.bin/esbuild"
ls "$BIOROUTER_ESBUILD_BIN"    # must exist; if not: (cd ui/desktop && npm ci)
```

### 1.4 The provider (so the app's agent can actually run)

Authoring **and** the per-app runtime agent both call an LLM. The provider the recorded run used is
**`versa_azure`** (UCSF gpt-5.5); its key lives in the macOS keychain, not in plaintext. A freshly
built debug binary may not have keychain access (grants are per-signed-binary). Two options:

**Option A — re-sign the binary so keychain grants apply** (cleanest if you can).
`just copy-binary debug` re-signs dev binaries with the Developer ID. Then the daemon reads the key
from the keychain and macOS shows at most one "Always Allow" prompt.

**Option B — extract the key into env** (what a headless run should do). The BioRouter secrets are a
JSON blob under keychain service `biorouter`. Extract just the one key you need into an env file that
you `source` into the daemon and never print:

> **Warning.** This snippet moves a live provider API key out of the OS keychain into a file on disk. Only run it on a machine you control, keep the `0600` mode it sets, delete the file when the batch ends (see the cleanup line in §9), and never echo the key or paste raw daemon logs into any shared channel — the daemon logs its full spawn env, including secrets, to stdout.

```bash
python3 - <<'PY'
import subprocess, json, os
blob = subprocess.run(['security','find-generic-password','-s','biorouter','-w'],
                      capture_output=True, text=True).stdout.strip()
d = json.loads(blob)
key = d.get('VERSA_AZURE_API_KEY') or d.get('AZURE_OPENAI_API_KEY')
envf = '/tmp/br-testdrive.env'
open(envf,'w').write('export VERSA_AZURE_API_KEY=%r\n' % key); os.chmod(envf, 0o600)
print('wrote', envf, '(len', len(key), ')')
PY
```

Only the key needs supplying: `versa_azure`'s endpoint, deployment and API version are compiled
in, and `VERSA_AZURE_ENDPOINT` / `_DEPLOYMENT_NAME` / `_API_VERSION` in
`~/.config/biorouter/config.yaml` override them. It does not read the public Azure provider's
`AZURE_OPENAI_*` keys, so those change nothing here.

### 1.5 Start the daemon

```bash
source /tmp/br-testdrive.env                      # provider key (Option B)
export BIOROUTER_SERVER__SECRET_KEY=test          # auth for mutating routes (POST /reply etc.)
export BIOROUTER_PORT=8899                        # a fixed, non-3000 port so you know the URL
export BIOROUTER_ESBUILD_BIN="$REPO/ui/desktop/node_modules/.bin/esbuild"
/tmp/br-testdrive-target/debug/biorouterd agent > /tmp/br-daemon.log 2>&1 &
# wait for readiness:
until curl -sf -o /dev/null http://localhost:8899/status; do sleep 1; done
echo "daemon up on :8899"
```

Keep **one** daemon up for the whole batch. Apps are served at
`http://localhost:8899/apps/<app-id>/`. `GET`s under `/apps/*` are auth-exempt; the mutating routes
(`POST /reply`, `POST /agent/start`, `POST /apps/{id}/build`, `DELETE /apps/{id}`) require the secret
`test` — send it per the server's auth scheme (default header in debug is the secret key; confirm the
exact header from `crates/biorouter-server/src/auth.rs` or the generated client
`ui/desktop/src/api/`).

---

## 2. Mental model — what "an Agent Drafter app" is

Knowing this is how you know what "correct" means. An app is a **served web front-end plus a per-app BioRouter agent**, wired by the Apps SDK v2. When you open `/apps/<id>/`, the page connects a WebSocket to that app's agent. The agent doesn't just chat — it **drives the page**:

- **Declared surface** (in `manifest.json`): `surface.actions` (typed verbs the agent invokes on the
  app, e.g. `focus_node(id)`), `surface.signals` (app→agent events fired by user gestures, e.g.
  `node_selected(id)`), `surface.components` (author-registered custom widgets), `surface.state_schema`
  (the shared reactive doc).
- **Control plane tools** the agent calls: `ui_describe` (read the app's declared surface + live
  state), `ui_patch` / `ui_render` / `ui_panel` (paint/patch catalog widgets into `@region:x` targets
  or dock panels), `ui_chart` / `ui_graph` / `ui_figure` (charts, force-graphs, embedded scientific
  figures), `ui_highlight`, `ui_theme`, `ui_layout`, `ui_notify`, `ui_state` / `ui_patch_state`,
  `ui_ask` (blocking question), `ui_suggest` (chips), `app_call` (invoke a declared action),
  `ui_subscribe` (subscribe to signals), `consult` (delegate to a worker profile), `emit_result`
  (structured output). The catalog widgets: `network`, `plot`, `table`, `kpi`, `log`, `figure`,
  `canvas`, `markdown`, `image`, plus custom components.
- **Multi-agent**: `manifest.agent.orchestration.agents` declares worker profiles; the SDK exposes
  `br.agent("critic")`; the main agent can `consult` a profile.
- **Client SDK** (`src/main.ts` uses it): `br.actions.register`, `br.state`, `br.signals`,
  `br.components.register`, `data-br-bind` reactive bindings, `br.kb`, theme packs.

**"Correct" for a spec means:** the manifest declares the surface the spec calls for; the front-end
renders the specified layout with real regions; the user's direct-manipulation gestures fire signals
and update bound state; and when the user issues the spec's natural-language request, the agent (and
its worker profiles) reason in multiple steps, call `ui_describe` then a sequence of `app_call` /
`ui_patch` frames, mutate shared state, and the specified regions update while the presence chip
narrates. It is **not a chatbot** — the chat box is at most a small secondary input.

Store layout for each authored app (inspect it directly):

```text
~/.config/biorouter/agent_drafter/<app-id>/
  manifest.json      # id, title, kind, entry, agent{system_prompt,capabilities,extensions,skills,
                     #   knowledge_base, orchestration.agents, orchestration.routes}, surface{...}, theme{pack}
  index.html         # the authored layout (regions, data-br-bind, data-br-region)
  src/main.ts        # authored client code (br.actions.register, br.state, br.components.register…)
  src/sdk.ts         # vendored SDK runtime (do not edit)
  dist/app.js        # esbuild bundle the browser loads
```

---

## 3. The authoring loop — make the agent build it, and iterate until it's right

**Golden rule: you do not write the app. You make the BioRouter agent write it, and you keep the
conversation going until it matches the spec.** This is what tests Agent Drafter.

### 3.1 Pick an authoring channel

`create_app` and friends are **MCP tools inside an agent turn** — there is no REST "create app"
endpoint. So authoring happens through a **BioRouter chat session that has the `agent_drafter`
extension enabled**. Three channels, pick one (recommended first):

**Channel A — programmatic session over REST (best for scripting 100 apps).**

- `POST /agent/start` with `StartAgentRequest { extension_overrides, working_dir, … }` — use
  `extension_overrides` to **enable `agent_drafter`** (also enable `autovisualiser`, `knowledge`,
  `developer` when a spec needs figures / KB / shell). `agent_drafter` is a builtin MCP server
  (`builtin!(agent_drafter, AgentDrafterServer)`); it must be turned on for the session or the tools
  won't exist.
- `POST /reply` with `ChatRequest { session_id, user_message, conversation_so_far }` → the response is
  an **SSE stream** (`text/event-stream`) of the agent's message deltas, thoughts, and **tool calls**
  (`create_app`, `configure_app`, `build_app`, `launch_app`, `lint_app`, …). Parse the stream to see
  what the agent did and what it returned.
- Iterate by calling `POST /reply` again with a follow-up `user_message` on the **same `session_id`**
  (session state persists server-side; you can also pass `conversation_so_far`).
- Exact field names and enums live in `ui/desktop/openapi.json` (paths `/agent/start`, `/reply`) and the
  generated client `ui/desktop/src/api/` — read them, do not guess. Send the `test` secret on
  these POSTs.

**Channel B — the desktop GUI (visual, uses the `debug-app` skill).**
Launch the Electron dev GUI (per the `debug-app` skill runbook: standalone vite on :5173 plus a
Playwright-owned Electron instance, `unset ELECTRON_RUN_AS_NODE` first, single-instance lock caveat),
open a chat with `agent_drafter` enabled, and type build prompts. Heavier; good for spot-checking a
few, not for all 100.

**Channel C — the CLI TUI.**
`biorouter session` in a tmux PTY (the `debug-app` skill's `cli-driver.sh start|send|snap`). Works,
but scraping a full-screen ratatui TUI for structured build results is brittle.

Whatever the channel, verification and store inspection are the same (§4–§6).

### 3.2 The iteration protocol (per app)

For spec **N**, run this loop. Budget roughly 4–8 authoring rounds; stop early on acceptance, stop late with
a recorded finding (§7).

**Round 0 — kick-off.** Send the *entire spec block* as the first `user_message`, framed as a build
order, e.g.:

> "Build a BioRouter app to this exact specification. Use the archetype that best fits (canvas /
> explorer / dashboard / workbench / wizard). Declare every action, signal, and component the spec
> lists in the manifest surface. Wire the layout regions exactly as described. Set up the multi-agent
> profiles named in the spec. Then build it and tell me the app id.
> \n\n<PASTE THE FULL `### N.` SPEC BLOCK>"

The agent should call `create_app` (with `archetype`), `configure_app` (system prompt, capabilities,
extensions, skills, KB, model routes, `orchestration.agents`), write `index.html` + `src/main.ts`, and
`build_app`. Capture the app id it returns.

**Round 1 — read what it built, before opening a browser.** Inspect the store:

```bash
ID=<app-id>
python3 -m json.tool ~/.config/biorouter/agent_drafter/$ID/manifest.json | sed -n '1,80p'
sed -n '1,120p' ~/.config/biorouter/agent_drafter/$ID/index.html
sed -n '1,160p' ~/.config/biorouter/agent_drafter/$ID/src/main.ts
```

Check against the spec, on paper:

- Does `surface.actions` contain every declared verb (with params)? `surface.signals`?
  `surface.components`? `surface.state_schema`?
- Does `agent.capabilities.ui.enabled` = true, plus `allow_signals` / `allow_html` / `allow_autorun`
  as the spec needs? Are the spec's extensions/skills/KB/model-routes set?
- Does `orchestration.agents` declare the spec's worker profiles?
- Does `index.html` have the named regions (`data-br-region="stage"`, the left rail, right inspector,
  bottom transport bar) and `data-br-bind` bindings the spec calls for?
- Does `theme.pack` match the spec's theme?

**Round 1b — lint.** Ask the agent to `lint_app` (or run the appcheck harness, §4.3). Feed every lint
error back verbatim as the next `user_message`: *"lint reported: <errors>. Fix them."*

**Round 2+ — grow and correct via targeted prompts.** For every gap you found (store review + lint +
the browser checks in §5–§6), send a **specific** follow-up. Examples of the *kind* of prompt that
"grows the agent" and drives iteration:

- "The spec requires a Right 340px inspector region; index.html has no `data-br-region="inspector"`.
  Add it and have the agent patch node dossiers into it."
- "You declared `move_avatar` but the spec also needs `set_color` and `speak` — add those actions and
  register handlers in main.ts."
- "The spec has three agent profiles (Prosecutor, Defense, Chief Justice). You only created one system
  prompt. Declare all three under `orchestration.agents` and make the main agent `consult` them."
- "When I click a node nothing reaches the agent — declare the `node_selected` signal, `ui_subscribe`
  to it, and react by opening the dossier."
- "The presence chip never narrates. Have each agent step emit a short `ui_notify` / narration so the
  user can see what it's doing."
- Answer any `ui_ask` the authoring agent poses (respond on the session), and if it's unsure, **ask it
  clarifying questions back** ("which region should the force-graph mount in?") to converge.

After each fix round, the agent should `update_app` + `build_app`. Re-serve/reload and re-verify.

**Acceptance.** Stop when the app passes the functional and aesthetic rubrics (§5–§6). Record the
round count and every friction point (§7).

**Cleanliness.** Give each app a stable id (e.g. `spec-001-variant-tribunal`). If you need a clean
re-author, `DELETE /apps/<id>` or `rm -rf ~/.config/biorouter/agent_drafter/<id>` and start over — but
**a clean re-author that succeeds where iteration failed is itself a finding** (it means iteration
didn't converge; log it).

### 3.3 Building and rebuilding directly (when you need to force it)

`build_app` is the agent's tool, but you can also rebuild out-of-band:

```bash
curl -s -X POST http://localhost:8899/apps/$ID/build -H 'x-secret-key: test'   # confirm header name in auth.rs
# or bundle by hand to catch TS errors fast:
(cd ~/.config/biorouter/agent_drafter/$ID && "$BIOROUTER_ESBUILD_BIN" --bundle src/main.ts \
   --outfile=dist/app.js --format=iife --target=es2018 --loader:.ts=ts)
```

A hand bundle that errors tells you the authored `main.ts` is broken → feed the esbuild error back to
the agent.

---

## 4. Serve, open, and drive the app with computer-use

### 4.1 Open it

The app is already served (the daemon serves everything in the store). Or use the CLI:
`biorouter apps list` / `biorouter apps open <id>`. The URL is
`http://localhost:8899/apps/<id>/`.

### 4.2 Browser automation (Playwright MCP or equivalent)

Navigate with your computer-use tool. Core moves you'll use: `browser_navigate`,
`browser_snapshot` (accessibility tree — better than a screenshot for finding elements/refs),
`browser_take_screenshot`, `browser_click`, `browser_type` (with `submit:true` to press Enter),
`browser_evaluate` (run JS in the page — your most powerful verification tool),
`browser_console_messages`, `browser_wait_for`, `browser_resize`.

> **Warning.** Stale browser profile lock: if navigation fails with *"Browser is already in use … mcp-chrome-…"*, a stale Chrome holds the profile. Clear it:

```bash
pkill -9 -f "mcp-chrome" 2>/dev/null; sleep 1
rm -f "$HOME/Library/Caches/ms-playwright-mcp/"*/SingletonLock 2>/dev/null
```

> **Note.** A `favicon.ico` 401 console error is benign (the app doesn't serve a favicon). Ignore it; any *other* console error is a real defect.

### 4.3 Optional: the repo's own harnesses (fast, headless sanity)

- `node scripts/agent-drafter/ui-control-harness.mjs` — mounts the real SDK in jsdom and asserts the
  frame protocol (state/bindings/patch/actions/signals/agent facade). Green here means the SDK runtime
  is intact; failures point at the SDK, not the app.
- `ui/desktop/scripts/appcheck/check-all.mjs` and `benchmark.mjs` — enumerate the store, load each app
  against the live daemon, and score it (bound-state paths, declared actions, catalog components,
  archetype). `benchmark.mjs --json` gives you a machine-readable per-app scorecard you can fold into
  your results (see `ui/desktop/scripts/appcheck/BASELINE.md`).

---

## 5. Verify functional correctness (computer-use rubric)

For each app, run these checks with the browser tool and record pass/fail. This is the core of "is it
actually the app the spec asked for, and does it work."

**5.1 Load and wiring.** Confirm the served page is healthy and v2-wired:

```bash
curl -s -D - -o /dev/null http://localhost:8899/apps/$ID/ | grep -iE "content-security-policy|HTTP/"
curl -s http://localhost:8899/apps/$ID/ | grep -oE "biorouter-app-config|wsToken|data-br-pack=\"[a-z-]+\""
curl -s -o /dev/null -w "bundle HTTP %{http_code} %{size_download}b\n" http://localhost:8899/apps/$ID/dist/app.js
```

In the browser after load, the snapshot should show **"Session ready"** and a `ui` capability badge —
that proves the per-app agent WebSocket connected and advertised its capabilities. Console must be
clean (favicon 401 aside).

**5.2 It is not a chatbot.** The primary surface is the specified interface (canvas/graph/map/board),
and any chat box is small and secondary. Fail the app if the "app" is just a chat transcript.

**5.3 Layout matches the spec.** Verify each named region exists at roughly the specified
position/size. Use the snapshot for presence, and `browser_evaluate` for geometry:

```js
() => ['[data-br-region="stage"]','[data-br-region="inspector"]','#pad','.br-presence']
  .map(sel => { const el=document.querySelector(sel); const r=el&&el.getBoundingClientRect();
    return {sel, present:!!el, box: r && {x:Math.round(r.x),y:Math.round(r.y),w:Math.round(r.width),h:Math.round(r.height)}}; })
```

Check the left-rail width, center surface, right-inspector width, bottom transport bar, and the
floating presence chip against the spec's pixel intents (allow reasonable tolerance).

**5.4 The declared surface is real.** In the page, read what the app advertises:

```js
() => ({ agents: window.BioRouter.agents ? window.BioRouter.agents() : [],
         state: window.BioRouter.state.get(),
         boundNodes: [...document.querySelectorAll('[data-br-bind]')].map(n=>n.getAttribute('data-br-bind')) })
```

Cross-check against `manifest.json`'s `surface` (you already inspected it in §3.2) and the spec's
declared actions/signals/components/state.

**5.5 Client-side reactivity (user → shared state → bindings).** Exercise the spec's direct-
manipulation controls (click the manual pad, drag a slider, select a node) and confirm the shared
state doc and `data-br-bind` DOM update — no agent involved:

```js
() => ({ scene: window.BioRouter.state.get('/scene'),
         coordShown: document.querySelector('[data-br-bind="/scene/x"]')?.textContent })
```

**5.6 The agent-driven loop (the headline test).** Type the spec's worked-example instruction into the
app's composer and submit. Wait, then verify **all** of:

- The transcript shows the agent called **`ui_describe`** (to learn the surface) then a **sequence of
  `app_call` / `ui_patch`** frames — read the tool trace in the DOM (tool-call rows) or via the SSE if
  you drove authoring over REST. Multi-step reasoning must be visible, not a single reply.
- **Shared state mutated** (re-read `window.BioRouter.state.get()` — the values changed as instructed).
- **The specified region updated** (screenshot before/after; the canvas moved / the inspector filled /
  the graph re-laid-out / the map recolored — whatever the spec says).
- **Presence narrated** the steps (the ambient chip showed "Agent · doing X").
- Do a second, *different* instruction to prove it's genuinely interactive and repeatable.

**5.7 Multi-agent actually ran.** Confirm ≥2 profiles were involved: `ready.profiles` lists the worker
names, and worker stream frames are agent-tagged / attributed in the transcript (e.g. "Skeptic
refuting …"). If the spec names Prosecutor+Defense+Judge but only one agent ever speaks, it fails the
multi-agent criterion — log it.

**5.8 Signals round-trip.** Perform a user gesture the spec says should notify the agent (select a
node, draw a region), and confirm the agent reacted (a follow-up patch/narration). If nothing reaches
the agent, the signal wasn't declared/subscribed — log it.

Record a per-check verdict. An app is **functionally PASS** only if 5.2, 5.5, 5.6 and 5.7 all hold
and the layout (5.3) substantially matches.

---

## 6. Verify aesthetic alignment (computer-use rubric)

Screenshots plus computed styles, judged against the spec's **Theme & aesthetic** and **Layout** fields.

**6.1 Theme pack applied.** `document.documentElement.getAttribute('data-br-pack')` must equal the
spec's pack. Spot-check the palette and type against the pack's intent:

```js
() => { const cs = getComputedStyle(document.documentElement);
  return { pack: document.documentElement.getAttribute('data-br-pack'),
           mode: document.documentElement.getAttribute('data-br-theme'),
           accent: cs.getPropertyValue('--br-accent').trim(),
           font: getComputedStyle(document.body).fontFamily,
           surface: cs.getPropertyValue('--br-surface').trim() }; }
```

> **Note.** Known SDK behavior to account for, not an app bug: the pack attribute is set on `<html>`, but the *grounds* render in **light** mode unless the app opts into `theme:"auto"` in `createApp`. So a spec that asks for a dark pack (e.g. `midnight`, `terminal`) may render on light grounds with the pack's accent/typography applied. Judge the accent/typography/density against the pack; if the spec truly needs full dark, that's a legitimate finding to log (and you can prompt the agent to set `theme:"auto"`).

**6.2 Density, chrome, motion.** Compare the screenshot to the spec's motif ("dense stacked tracks,
near-zero chrome, amber only on live state"; "generous whitespace, serif headers"). Confirm accent is
used only where the spec says (e.g. only on live/primary state, not decoratively). Motion is hard to
capture in a still — check that CSS transitions exist on the relevant elements
(`getComputedStyle(el).transition`), and note if entrance/idle animation matches "calm, short,
informative" vs "bouncy" (the design language forbids overshoot).

**6.3 Region placement and specified controls.** The buttons named in the spec must be where the spec
puts them (e.g. "Bottom 64px transport bar: *Re-adjudicate* button"; "floating top-right presence
chip"). Confirm via snapshot + geometry.

**6.4 Capture evidence.** Save a screenshot per app to a results folder
(`/tmp/br-testdrive/shots/spec-NNN.png`) for the record and for the aesthetic verdict.

Record an aesthetic verdict: **ALIGNED / PARTIAL / OFF**, with the specific mismatches.

---

## 7. The findings log — a primary deliverable

**Everything that goes wrong, is awkward, or takes too many rounds must be captured so Agent Drafter
can be improved.** Keep two artifacts under `/tmp/br-testdrive/` (or a repo path you choose), and keep
them current as you go — do not reconstruct from memory at the end.

> **Note.** Where the executed run put these: the 2026-07 campaign committed its deliverables into the repo, and they now live under [`docs/history/agent-drafter-testdrive-100/`](../../history/agent-drafter-testdrive-100/README.md): per-app results in `app-results/spec-NNN-<slug>.md`, the rollup as [`audit-findings-register.md`](../../history/agent-drafter-testdrive-100/audit-findings-register.md), and the machine-readable ledger in `data/ledger.json`. For a new run, prefer those kebab-case names over the `FINDINGS.md` spelling used in the templates below.

### 7.1 Per-app result file — `results/spec-NNN.md`

One per spec, written as you test it:

```markdown
# Spec NNN — <App Name>
- **App id:** spec-NNN-...
- **Authoring rounds:** <count>   **Reached acceptance:** yes / partial / no
- **Channel:** REST / GUI / CLI
- **Archetype chosen by the agent:** canvas / explorer / dashboard / workbench / wizard / chat

## Functional verdict: PASS / PARTIAL / FAIL
| Check | Result | Notes |
|---|---|---|
| Not a chatbot (5.2) | ✅/❌ | |
| Layout matches (5.3) | ✅/⚠️/❌ | which regions missing/misplaced |
| Declared surface (5.4) | ✅/❌ | missing actions/signals/components |
| Client reactivity (5.5) | ✅/❌ | |
| Agent-driven loop (5.6) | ✅/❌ | did it ui_describe→app_call? region updated? |
| Multi-agent ran (5.7) | ✅/❌ | which profiles actually spoke |
| Signals round-trip (5.8) | ✅/❌ | |

## Aesthetic verdict: ALIGNED / PARTIAL / OFF
- theme pack applied? mode? accent/typography match? specific mismatches.

## Screenshots: shots/spec-NNN-*.png

## Friction encountered (see FINDINGS.md for the rollup)
- <bullet per issue, tagged>
```

### 7.2 The rollup — `FINDINGS.md`

The cumulative, de-duplicated log the team will act on. Every entry is one issue, tagged by **type**
and **severity**, with enough to reproduce. Suggested taxonomy:

| Tag | What it means |
|---|---|
| `AUTHORING-INEFFICIENCY` | The agent needed too many rounds, kept undoing its own work, misread the spec, or produced a chatbot on the first pass. *Capture the prompt→response that was wasteful and what finally worked.* This is exactly the "inefficient back-and-forth" the team wants surfaced. |
| `SPEC-GAP` | The agent silently dropped a spec requirement (a region, an action, a profile). |
| `SDK-LIMITATION` | The SDK genuinely can't express what the spec needs (a missing widget kind, no way to do a real-time frame loop, a layout the grammar can't produce, a theme that won't go dark, a multi-agent pattern that isn't supported). These ambitious specs are *designed* to surface these — distinguish them clearly from app bugs. |
| `FUNCTIONAL-BUG` | The built app misbehaves (a `ui_patch` targets a nonexistent region, a signal never fires, `app_call` errors, state doesn't bind, the agent stalls). |
| `AESTHETIC-DRIFT` | The look diverges from the spec/design language (pack not applied, decorative accent, wrong density, bouncy motion, cold neutrals). |
| `SECURITY/ROBUSTNESS` | CSP violation, ws-token/auth issue, sanitizer bypass, a crash, an uncaught console error, a hang. |
| `ERGONOMICS` | Anything clunky in the loop itself (unclear tool errors, no lint signal, opaque build failures, the agent not narrating, `ui_ask` not surfacing) that made *driving* Agent Drafter harder than it should be. |

Entry template:

```markdown
### [TYPE][SEV: high/med/low] Short title
- **Where:** spec NNN (<App>), round R.
- **Symptom:** what happened (paste the failing tool error / a screenshot ref / the transcript excerpt).
- **Repro:** the exact prompt(s) and app state that trigger it.
- **Root cause (best guess):** app authoring vs SDK vs daemon vs model.
- **Impact:** blocks the app / degrades it / cosmetic / just slow.
- **Suggested fix or SDK improvement:** e.g. "add a `timeline` catalog widget", "make `create_app`
  seed the declared surface from the prompt's action list", "lint should flag a region referenced by
  ui_patch but absent from index.html", "default `theme:auto` when a dark pack is chosen".
```

**Also keep a top-of-file dashboard** in the rollup: counts by type/severity, the median authoring
rounds to acceptance, the top 10 most-common failure modes, and a "what would most improve Agent
Drafter" shortlist. That rollup is the point of the whole exercise.

---

## 8. Batch execution and discipline

- **Loop over the 100 specs** with a stable id per spec. Keep the daemon and one browser session up
  across the batch.
- **Iteration budget:** cap authoring rounds (roughly 6–8). If an app still fails, stop, record the app as
  `partial`/`fail` with the blocking finding, and move on — don't burn unbounded effort on one spec.
  A stuck app is *data* about Agent Drafter, not a personal failure.
- **Do not lower the bar.** These specs are intentionally ambitious. If Agent Drafter can't build one,
  that is an `SDK-LIMITATION` finding, not a reason to simplify the spec.
- **Never hand-author the app to "make it pass."** If you edit `main.ts`/`manifest.json` yourself to
  fix something, you've stopped testing Agent Drafter. The only permitted human edits are the ids you
  assign and reading/deleting store files. All app content must come from the agent.
- **Keep secrets out of logs.** Don't print the provider key; don't paste `/tmp/br-daemon.log` (it
  contains the spawn env) anywhere shared.
- **Prefer structured reads over screenshots for facts** (manifest, `state.get()`, geometry evals) and
  use screenshots for the *aesthetic* judgment and the record.

---

## 9. Quick reference — the exact loop, condensed

```bash
# once
cd "$REPO" && source bin/activate-hermit
export CARGO_TARGET_DIR=/tmp/br-testdrive-target
cargo build -p biorouter-server --bin biorouterd && cargo build -p biorouter-cli --bin biorouter
python3  # extract VERSA_AZURE_API_KEY → /tmp/br-testdrive.env  (see §1.4)
source /tmp/br-testdrive.env
export BIOROUTER_SERVER__SECRET_KEY=test BIOROUTER_PORT=8899 \
       BIOROUTER_ESBUILD_BIN=$PWD/ui/desktop/node_modules/.bin/esbuild
/tmp/br-testdrive-target/debug/biorouterd agent > /tmp/br-daemon.log 2>&1 &
until curl -sf -o /dev/null http://localhost:8899/status; do sleep 1; done

# per spec N (id = spec-NNN-slug)
#  1. POST /agent/start (enable agent_drafter+autovisualiser+knowledge+developer) → session_id
#  2. POST /reply  user_message = "<build order> + <full ### N. spec block>"   (parse SSE for tool calls)
#  3. read store: manifest.json / index.html / src/main.ts    → note gaps vs spec
#  4. lint_app (or appcheck) → feed errors back via POST /reply
#  5. browser: navigate http://localhost:8899/apps/<id>/ ; snapshot ; screenshot
#  6. run §5 functional checks + §6 aesthetic checks (browser_evaluate + curl header checks)
#  7. for each gap: POST /reply with a SPECIFIC fix prompt → agent update_app+build_app → re-verify
#  8. write results/spec-NNN.md ; append every friction point to the findings rollup   ← do not skip
#  9. accept (rubrics pass) or record partial/fail with the blocking finding ; next spec

# cleanup
pkill -f "biorouterd agent"; pkill -9 -f mcp-chrome; rm -f /tmp/br-testdrive.env
```

---

## 10. Success criteria for the whole run

You are done when, for all 100 specs, you have: an authored app (or a recorded reason it couldn't be
built), a per-app result file with functional + aesthetic verdicts and screenshots, and a
**findings rollup** that tells the BioRouter team, concretely and with reproductions, **where
Agent Drafter is inefficient, where it silently drops requirements, where the SDK is genuinely
limited, and what to improve first.** That findings document — not a pile of passing apps — is the real
output.

## Related documentation

- [100 agentic app test specs](hundred-app-test-specs.md) — the corpus of 100 briefs this runbook consumes, one per iteration of the loop.
- [100-app test-drive archive](../../history/agent-drafter-testdrive-100/README.md) — the evidence, per-app results and blockers from the run executed under this runbook.
- [Audit findings register](../../history/agent-drafter-testdrive-100/audit-findings-register.md) — the findings rollup this runbook's §7 asks you to produce, as it was actually written.
- [Apps SDK reference](../../apps-sdk/sdk-reference.md) — every `br.*` and `ui_*` signature you check an authored app against.
- [Agent Drafter apps platform design](../apps-platform-design.md) — why apps are shaped the way they are, and what the SDK v2 contract promises.
