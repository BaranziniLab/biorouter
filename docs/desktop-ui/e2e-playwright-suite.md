# The Playwright end-to-end suite

> **What this is.** A map of `ui/desktop/tests/e2e` — the four gates its specs sit behind, how to run each one, the sandbox contract every live spec must honour, and an assessment of what it would take to run any of it in CI.
> **Status:** Current; gate inventory and timings measured 2026-09-09 against v1.90.3.
> **Audience:** developers working on the desktop UI, maintainers deciding what CI should run

These specs drive the real Electron app: a real renderer, a real `biorouterd`, and — for
some of them — a real model turn billed to a real provider. That is what makes them
worth having and also what keeps them out of CI today. **No GitHub workflow runs
`tests/e2e` at all.** Everything below is therefore about what a developer runs by hand,
plus what would have to be true for a runner to do it instead.

Before adding a spec, read the sandbox contract. A live spec that forgets it does not
fail — it silently drives the developer's own Biorouter against their own configuration
and chat history.

## The four gates

Measured with `npx playwright test --list`: 66 tests across 11 files.

| Gate | Environment | Specs | What it needs |
|------|-------------|-------|---------------|
| **Ungated** | none | `brxt.spec.ts`, `user-message-layout.spec.ts` | The dev bundle only. Runs on a bare `npx playwright test`. |
| **LIVE** | `BIOROUTER_E2E_LIVE=1` (+ optional `BIOROUTER_E2E_PATH_ROOT`) | `app.spec.ts`, `context-management.spec.ts`, `enhanced-context-management.spec.ts`, `knowledge-ingest.spec.ts`, `schedule-artifact.spec.ts` | A configured provider with credit, and a seeded sandbox. |
| **EXTERNAL** | `BIOROUTER_E2E_EXTERNAL=1` | `bioroffice-install.spec.ts`, `bioroffice-verify.spec.ts`, `brxt-spoke-install.spec.ts`, `spokeagent-skills.spec.ts` | Extensions actually installed on the machine, `.brxt` bundles that are not in this repo, and a UCSF passcode. |
| **Packaged** | `BIOROUTER_E2E_LIVE=1` + `BIOROUTER_E2E_APP=<path to Biorouter.app>` | `schedule-artifact.spec.ts` only | A built, signed macOS app in addition to everything LIVE needs. |

A spec off its gate reports as `skipped`, not as passing.

⚠ **An unsandboxed launch really does write to the developer's own config, and it was
measured doing so.** `brxt.spec.ts` used to call `electron.launch` with no
`BIOROUTER_PATH_ROOT`. On 2026-09-09 a run of the whole directory therefore wrote an
entry naming that spec's launch directory into the real
`~/.local/share/biorouter/projects.json`. The path is worth following, because the
obvious suspect is the wrong one: the daemon behaved correctly — `Paths::get_dir` honours
the redirect, which is why the real `sessions.db` and `schedule.json` were untouched. The
writer was the CLI. `crates/biorouter-cli/src/cli.rs` records the process's working
directory into `projects.json` on *every* subcommand before dispatch, and the desktop app
runs `biorouter doctor` at startup, inheriting whatever environment it was given. So "this
spec only opens modals" is not a reason to skip the sandbox: the app writes on its own
behalf, at startup, before a test does anything.

Every spec now launches through `launchApp`, which creates the root itself. That is
enforced rather than documented — `src/test/e2eSandboxInvariant.test.ts` fails if a file
in this directory calls `electron.launch` outside the sanctioned launcher, and it runs in
the ordinary vitest job.

⚠ **A bare positional filter does not filter.** `npx playwright test schedule-artifact`
ran all 66 tests, including, at the time, the unsandboxed ones; `npx playwright test
tests/e2e/schedule-artifact.spec.ts` correctly selects 2. Always pass the path.

## Running each gate

From `ui/desktop`, after `source bin/activate-hermit` at the repo root (the dmg and
Electron toolchain want hermit's Node 24, not a newer system Node):

```bash
npm run generate-api && npm run build:e2e     # rebuild .vite/ — no packaging

# Ungated
npx playwright test tests/e2e/user-message-layout.spec.ts

# LIVE — one spec at a time, always by path
env -u ELECTRON_RUN_AS_NODE BIOROUTER_E2E_LIVE=1 \
  npx playwright test tests/e2e/schedule-artifact.spec.ts

# Packaged — adds the packaged variant of the same scenario
env -u ELECTRON_RUN_AS_NODE BIOROUTER_E2E_LIVE=1 \
  BIOROUTER_E2E_APP=ui/desktop/out/Biorouter-darwin-arm64/Biorouter.app \
  npx playwright test tests/e2e/schedule-artifact.spec.ts
```

`env -u ELECTRON_RUN_AS_NODE` is not optional in an agent shell or any environment that
exports it: with it set, Electron exits immediately with no window and no error.

## The sandbox contract

Every live spec must run against a configuration it owns. The mechanism is
`BIOROUTER_PATH_ROOT`, which relocates the whole triple —
`crates/biorouter/src/config/paths.rs:18-27` resolves `<root>/config`, `<root>/data` and
`<root>/state`, and the Electron main process honours the same variable at
`src/main.ts:203`. The daemon inherits it because `startBiorouterd` spreads
`process.env`.

A conforming spec:

1. copies a seed to a fresh `mkdtemp` root (`config/config.yaml`, `config/secrets.yaml`,
   `config/extensions/`, `data/sessions/sessions.db`, `data/schedule.json`);
2. passes `BIOROUTER_PATH_ROOT=<root>` and `BIOROUTER_DISABLE_KEYRING=true`;
3. gives Electron its own `--user-data-dir=<root>/electron`, so
   `requestSingleInstanceLock` (`src/main.ts:740`) cannot collide with a Biorouter the
   developer is running;
4. overwrites `data/schedule.json` with `[]`, because a seed's schedule entries name
   workflow files outside the sandbox.

`tests/e2e/helpers/sandbox.ts` and `tests/e2e/helpers/app.ts` implement all four and are
the single reference; `launchApp` is the only sanctioned way to start the app.
`schedule-artifact.helpers.ts` adds just what the shared pair cannot: the packaged-app
launch, the workflow fixture and the evidence directory.

Two things the contract does **not** buy you, both measured:

- **`BIOROUTER_DISABLE_KEYRING` matters only for the packaged app.** An unpackaged build
  already forces it off (`src/biorouterd.ts:358-359`); the packaged daemon defaults the
  keyring **on**, so without the variable it reads the OS credential store instead of the
  sandbox's `secrets.yaml`.
- **Writing `[]` does not stop the built-in schedule coming back.** The daemon re-seeds
  `daily-meditation` into the sandbox on startup, and it comes back **unpaused** where the
  seed had it paused. Its cron is `0 0 3 * * *`, so it will not fire during a normal run —
  but a run crossing 03:00 would start a real Soul meditation inside the sandbox.

### Startup dialogs make the whole app invisible

The single most expensive trap in this suite, and it is not specific to any one spec. A
Radix dialog marks the background `aria-hidden` while it is open, and Playwright's
**role queries skip aria-hidden subtrees**. So a page that is plainly rendered behind a
modal answers every `getByRole` with `element(s) not found`.

The symptom is maximally misleading: the assertion times out for its full budget, and the
failure snapshot captured immediately afterwards lists the element it was waiting for. It
reads as a slow app or a bad selector; it is neither.

A seeded sandbox reliably raises `FirstRunPrivacyNotice` ("Some of your chats are now
marked private"), because the seed's session database holds chats that ratchet private.
Its control reads **"Got it"**, so a helper looking only for `Dismiss` walks past it.

The rule for any new live spec: dismiss dialogs before the first role query, detect them
with a CSS locator (`[role="dialog"]`) rather than a role query, and loop — they stack.
`dismissStartupModals` in `schedule-artifact.helpers.ts` does this and names any dialog it
does not know how to close.

## The scheduled-artifact scenario

`schedule-artifact.spec.ts` is the one spec covering the Scheduler and the artifact panel
together, and the only one with a packaged variant. It creates a schedule from a workflow
that asks the Auto Visualiser for one bar chart, triggers it with "Run now", opens the
run's read-only transcript, and asserts the figure is a click-to-open card that opens the
artifact side panel — asserting the panel is **absent** before the click, which is the
contract in [Where a generated artifact is displayed](artifact-display-surfaces.md).

Two behaviours of the app shape the spec and are worth knowing independently:

- **"Run now" returns before the job finishes, and nothing polls afterwards.**
  `Scheduler::run_now` `tokio::spawn`s the run and returns the new session id
  immediately, so the run row appears while the job is still working. On the client side
  `fetchSessions` and `fetchSchedule` run once on mount and once after "Run now", and
  there is **no Refresh control on the detail view** (the Scheduler *list* has one). A
  transcript opened at that moment shows a partial conversation and never updates itself.
  What does refresh is leaving and re-entering: the effect keyed on
  `[scheduleId, selectedSession]` refetches when `selectedSession` returns to null, which
  is what Back does. The spec waits with a Back/re-open loop for that reason, not out of
  caution — a `waitFor` on the artifact card inside a stale transcript would never
  resolve.
- **The packaged app cannot be launched by `electron.launch`.** Playwright drives Electron
  with `--inspect=0` and blocks until the child prints `Debugger listening on ws://`; the
  packaged build disables `EnableNodeCliInspectArguments` (`forge.config.ts`), so that line
  never comes and the launch hangs until the test times out. The packaged variant spawns
  the executable itself with `ENABLE_PLAYWRIGHT=true` + `PLAYWRIGHT_CDP_PORT`
  (`src/main.ts:696-699`), polls `/json/list` for a page target, and attaches with
  `chromium.connectOverCDP`.

Teardown kills by pid — the process the helper spawned and its descendants, found with
`pgrep -P`. Never `pkill -f`: the obvious pattern also matches every other worktree's
daemon and the developer's own running app.

## What a run costs

Measured on an M4 Max, 2026-09-09, across five runs of the scheduled-artifact scenario:

| Phase | Wall clock |
|-------|-----------|
| `npm run generate-api && npm run build:e2e` | 16 s warm, ~2 min cold |
| Dev-bundle variant alone (launch → assertions → teardown) | 69–76 s; test body 29.8–40.8 s |
| Both variants in one invocation | 80 s; dev 29.8 s + packaged 22.3 s |

The model turn dominates and is the part that varies: the scenario budgets 300 s for it
inside a 420 s test timeout, and the observed turns finished in well under a minute. A CI
job running this one scenario on both variants would fit inside `preview-panel`'s 20-minute
budget with room to spare — the cost concern is the provider call, not the clock.

### Evidence, and why it is not where you expect

Two configuration facts, both verified by running the suite and then looking:

- **A passing test's trace is deleted.** `playwright.config.ts` sets `preserveOutput:
  'failures-only'`, which removes the output directory of every test that passed —
  including its `trace.zip`, even with `trace: 'on'`. After a green run `test-results/` is
  empty. Setting `preserveOutput: 'always'` is the one-line change that would keep them.
- **`--reporter=list` suppresses the HTML report.** Passing it overrides the config's
  `[['html'], ['list']]` pair, so no `playwright-report/` is regenerated and attachments go
  nowhere. Drop the flag when you want the report.

The scheduled-artifact scenario therefore writes its screenshot to `ui/desktop/e2e-evidence/`
(gitignored, overridable with `BIOROUTER_E2E_EVIDENCE_DIR`) — deliberately outside
`outputDir`, so it survives a pass — and *also* attaches it for the report.

## What CI would need

Nothing in `.github/workflows/` runs `tests/e2e` today. The three existing Playwright
touch points are unrelated: `frontend.yml`'s `shelf` job installs chromium for the landing
BAAM privacy facet, its `preview-panel` job runs `scripts/preview-panel-e2e.mjs`, and
`apps-smoke.yml` installs chromium for a `cargo test`.

The `preview-panel` job (`.github/workflows/frontend.yml:312-331`) is the right template
and the only place in the repo that already downloads Electron for a test:

- **A `macos-latest` runner.** The suite launches the real Electron runtime, and the
  packaged variant is macOS-specific.
- **`env -u ELECTRON_SKIP_BINARY_DOWNLOAD npm ci`.** The workflow-level environment sets
  `ELECTRON_SKIP_BINARY_DOWNLOAD: "1"` (`frontend.yml:48`) so vitest never downloads a
  ~150 MB binary; a Playwright-Electron job has to opt back in, exactly as `preview-panel`
  does.
- **A `VERSA_AZURE_API_KEY` secret**, for any LIVE spec. This is the decision, not a
  detail: it puts a billable provider call on every run of the job, and a provider outage
  becomes a red build on a pull request that did not touch the desktop.
- **A seed configuration.** Today the seed is a directory on one developer's machine
  (`~/biorouter-runs/seed-config`, overridable with `BIOROUTER_E2E_SEED`). CI needs it
  either committed — minus secrets, which would come from the secret above — or generated
  by a script. Committing the seed's 5542-chat session database is not an option; a
  generated minimal one is the realistic path, and it would also remove the first-run
  privacy notice, since that dialog only appears because the seed has private chats.
- **A longer `timeout-minutes` than `preview-panel`'s 20**, if more than one live spec runs.

### What is CI-able and what never is

**CI-able now:** the ungated specs. `brxt.spec.ts` is sandboxed as of this document's
revision, and the invariant test keeps it that way.

**CI-able given the secret and a generated seed:** `schedule-artifact.spec.ts` (dev
variant), and the LIVE specs generally.

**CI-able only on a job that already builds or downloads a package:** the packaged
variant. It needs a real `Biorouter.app`, so it belongs beside a packaging job or a
release-artifact download, not on a pull-request build.

**Never CI-able as written:** the four EXTERNAL specs. They require extensions installed
out of band, `.brxt` bundles that are not in this repository, and a hardcoded UCSF
passcode. These are developer verification scripts, not gates.

### Prettier is in no workflow, and would not cover this anyway

`lint:check` resolves to `typecheck && eslint && check:themes && check:contrast &&
check:tokens` — no Prettier — and no workflow invokes `format:check` either. Two scope
facts matter for anyone adding a spec:

- `format:check` is `prettier --check "src/**/*.{ts,tsx,css,json}"`, and ESLint is scoped
  to `src/**` as well. `tests/` is outside both.
- `tsconfig.json` has `"include": ["src"]`, so `tsc --noEmit` never sees a spec file.

So **no repo command type-checks or lints `tests/e2e`.** `npx playwright test --list`
proves a spec parses (Playwright transpiles with esbuild, which strips types without
checking them); a real type check needs `tsc` pointed at the files directly. Run both, plus
`prettier --check` on the paths themselves, before pushing.

## Related documentation

- [Where a generated artifact is displayed](artifact-display-surfaces.md) — the one-surface rule the scheduled-artifact scenario asserts, and why a transcript with nowhere to put an artifact does not compile.
- [Launching the dev GUI from a shell without a TTY](launching-the-dev-gui.md) — the five ways a working app looks broken when launched from an agent shell; `ELECTRON_RUN_AS_NODE` is the one that also bites Playwright.
- [Debugging the dev GUI with agent-browser](agent-browser-debugging.md) — the other way to drive the running app over CDP, for exploration rather than assertion.
- [Renderer testing traps](renderer-testing-traps.md) — the sibling catalogue for tests that pass while the code they cover is broken, at the vitest/jsdom layer.
- [Environment variables](../configuration/environment-variables.md) — reference for `BIOROUTER_PATH_ROOT`, `PLAYWRIGHT_CDP_PORT` and the rest of the knobs used here.
