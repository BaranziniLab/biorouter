/**
 * Sandbox, fixture and launch helpers for `schedule-artifact.spec.ts`.
 *
 * Kept beside the spec rather than in `tests/e2e/helpers/` so the scenario can
 * land without touching the shared helper directory; folding these into the
 * shared helpers later is a move, not a rewrite.
 *
 * Two things here are load-bearing and were each measured rather than assumed:
 *
 *  1. **Every launch is sandboxed.** `BIOROUTER_PATH_ROOT` relocates the whole
 *     config/data/state triple — `crates/biorouter/src/config/paths.rs:18-27`
 *     resolves `<root>/config`, `<root>/data`, `<root>/state`, and the desktop
 *     main process honours the same variable at `src/main.ts:203`. The daemon
 *     inherits it because `startBiorouterd` spreads `process.env`
 *     (`src/biorouterd.ts:369-372`). Nothing here may run against the real
 *     `~/.config/biorouter`, so the root is always a fresh `mkdtemp` copy of a
 *     seed and never a path the user works in.
 *
 *  2. **The packaged app cannot be launched by `electron.launch`.** Playwright
 *     drives Electron with `--inspect=0` and blocks until the child prints
 *     `Debugger listening on ws://`
 *     (`node_modules/playwright-core/lib/server/electron/electron.js`), and the
 *     packaged build disables `EnableNodeCliInspectArguments`
 *     (`forge.config.ts`), so that line never arrives and the launch hangs
 *     until the test times out. The packaged variant therefore spawns the
 *     executable itself and attaches over CDP — `main.ts:696-699` turns
 *     `ENABLE_PLAYWRIGHT` + `PLAYWRIGHT_CDP_PORT` into a
 *     `remote-debugging-port` switch.
 */

import { _electron as electron, chromium, type Browser, type Page } from '@playwright/test';
import { execFileSync, spawn, type ChildProcess } from 'child_process';
import * as fs from 'fs';
import * as http from 'http';
import * as os from 'os';
import * as path from 'path';

/** Repo-relative root of the desktop package (`ui/desktop`). */
export const DESKTOP_ROOT = path.join(__dirname, '../..');

/**
 * The seed sandbox: a `config/` + `data/` pair holding a provider, an API key
 * and the enabled extensions, copied per run. Overridable so a CI runner can
 * generate one instead of carrying it on disk.
 */
export const SEED_ROOT =
  process.env.BIOROUTER_E2E_SEED || path.join(os.homedir(), 'biorouter-runs', 'seed-config');

/**
 * Where evidence (screenshots) is written. Deliberately NOT under Playwright's
 * `outputDir`: `playwright.config.ts` sets `preserveOutput: 'failures-only'`,
 * which deletes a passing test's output directory, and the whole point of this
 * evidence is to exist on a pass.
 */
export const EVIDENCE_DIR =
  process.env.BIOROUTER_E2E_EVIDENCE_DIR || path.join(DESKTOP_ROOT, 'e2e-evidence');

/** A launched app plus the one page under test and a variant-specific teardown. */
export interface LaunchedApp {
  page: Page;
  close: () => Promise<void>;
}

/**
 * Copy the seed into a fresh temp root and neutralise anything in it that
 * points outside the sandbox.
 */
export function createSandbox(prefix: string): string {
  if (!fs.existsSync(SEED_ROOT)) {
    throw new Error(
      `Seed config not found at ${SEED_ROOT}. Set BIOROUTER_E2E_SEED to a directory holding config/ and data/.`
    );
  }
  const root = fs.mkdtempSync(path.join(os.tmpdir(), prefix));
  fs.cpSync(SEED_ROOT, root, { recursive: true });

  // The seed's own schedule entry names a workflow file OUTSIDE the sandbox, so
  // the scheduler would read (and could run) something this test does not own.
  // Start from no schedules at all.
  fs.mkdirSync(path.join(root, 'data'), { recursive: true });
  fs.writeFileSync(path.join(root, 'data', 'schedule.json'), '[]\n');

  // Electron's own profile, kept inside the sandbox so `requestSingleInstanceLock`
  // (`src/main.ts:740`) cannot collide with a Biorouter the user is running.
  fs.mkdirSync(path.join(root, 'electron'), { recursive: true });
  return root;
}

/**
 * A workflow whose whole job is to produce exactly one Auto Visualiser figure.
 *
 * `extensions` is declared rather than inherited: `resolve_extensions_for_new_session`
 * (`crates/biorouter/src/config/extensions.rs:537-539`) returns the workflow's
 * list verbatim when it has one, so naming `autovisualiser` alone is what makes
 * the run hermetic — no shell, no knowledge base, no skills to wander into.
 * `knowledge_bases` is deliberately absent so `ensure_required_extensions`
 * (`crates/biorouter/src/workflow/runtime.rs:17-30`) adds nothing back.
 *
 * The arguments are spelled out literally because `render_figure` is one of only
 * three advertised tools; the 32 figure *kinds* are not callable tool names.
 */
export function writeBarChartWorkflow(root: string): string {
  const dir = path.join(root, 'workflows');
  fs.mkdirSync(dir, { recursive: true });
  const file = path.join(dir, 'e2e-bar-chart.yaml');
  fs.writeFileSync(
    file,
    `version: 1.0.0
title: E2E bar chart figure
description: >-
  Draws exactly one Auto Visualiser bar chart from fixed numbers, so an
  end-to-end test can assert the figure reaches the artifact side panel.
instructions: |-
  Call the \`render_figure\` tool exactly once, with exactly these arguments:

  {
    "kind": "chart",
    "data": {
      "type": "bar",
      "title": "E2E Sample Counts",
      "labels": ["Alpha", "Beta", "Gamma"],
      "datasets": [{"label": "Samples", "data": [3, 5, 2]}]
    }
  }

  Rules, in order of importance:
  - Call no other tool. Do not call \`describe_figure\` first — the arguments
    above are already complete.
  - Call \`render_figure\` exactly once. Do not redraw, restyle or improve it.
  - After the tool returns, reply with the single word "done" and stop.
extensions:
- type: builtin
  name: autovisualiser
  display_name: Auto Visualiser
  description: Data visualization and UI generation tools
  timeout: 300
  bundled: true
  available_tools: []
`
  );
  return file;
}

/** The environment both variants share. */
function sandboxEnv(root: string): Record<string, string> {
  return {
    BIOROUTER_PATH_ROOT: root,
    // The packaged daemon defaults the keyring ON (`src/biorouterd.ts:358-359`
    // only forces it off for unpackaged builds), so this is what makes the
    // packaged variant read the sandbox's `secrets.yaml` instead of prompting
    // for the user's Keychain.
    BIOROUTER_DISABLE_KEYRING: 'true',
    BIOROUTER_ALLOWLIST_BYPASS: 'true',
  };
}

/** Wait for the renderer to have mounted React. */
export async function waitForRenderer(page: Page): Promise<void> {
  await page.waitForLoadState('domcontentloaded');
  await page.waitForFunction(() => {
    const root = document.getElementById('root');
    return !!root && root.children.length > 0;
  });
}

/**
 * Dismiss the "Install missing dependencies" modal if it appeared.
 *
 * Never install anything: the button is the outline `Dismiss`
 * (`src/components/DependencySetupModal.tsx:437`), which reads `Done` once
 * every dependency is already present.
 */
export async function dismissDependencyModal(page: Page): Promise<void> {
  const dismiss = page.getByRole('button', { name: /^(Dismiss|Done)$/ });
  try {
    await dismiss.first().waitFor({ state: 'visible', timeout: 8000 });
    await dismiss.first().click();
  } catch {
    // No modal: the common case on a seeded sandbox.
  }
}

/** Launch the dev bundle in `.vite/build/` via Playwright's Electron driver. */
export async function launchDevBundle(root: string): Promise<LaunchedApp> {
  const main = path.join(DESKTOP_ROOT, '.vite/build/main.js');
  if (!fs.existsSync(main)) {
    throw new Error(
      `Dev bundle missing at ${main}. Run: npm run generate-api && npm run build:e2e`
    );
  }
  const app = await electron.launch({
    args: [main, `--user-data-dir=${path.join(root, 'electron')}`],
    cwd: DESKTOP_ROOT,
    env: {
      ...process.env,
      ELECTRON_IS_DEV: '1',
      NODE_ENV: 'development',
      ...sandboxEnv(root),
      // An agent shell commonly exports this, and it makes Electron exit
      // instantly with no window and no error.
      ELECTRON_RUN_AS_NODE: '',
    },
  });
  const page = await app.firstWindow();
  await waitForRenderer(page);
  return { page, close: async () => void (await app.close().catch(() => {})) };
}

/** One CDP target as reported by `/json/list`. */
interface CdpTarget {
  type?: string;
  title?: string;
  url?: string;
}

function fetchJson(url: string, timeoutMs: number): Promise<CdpTarget[]> {
  return new Promise((resolve, reject) => {
    const req = http.get(url, { timeout: timeoutMs }, (res) => {
      let body = '';
      res.setEncoding('utf8');
      res.on('data', (chunk) => (body += chunk));
      res.on('end', () => {
        try {
          resolve(JSON.parse(body) as CdpTarget[]);
        } catch (err) {
          reject(err);
        }
      });
    });
    req.on('timeout', () => req.destroy(new Error('timeout')));
    req.on('error', reject);
  });
}

/** Every descendant pid of `pid`, deepest last. Used so teardown kills by pid. */
function descendantPids(pid: number): number[] {
  let children: number[] = [];
  try {
    children = execFileSync('pgrep', ['-P', String(pid)], { encoding: 'utf8' })
      .split('\n')
      .map((line) => Number(line.trim()))
      .filter((n) => Number.isInteger(n) && n > 0);
  } catch {
    return [];
  }
  return children.flatMap((child) => [child, ...descendantPids(child)]);
}

/**
 * Spawn the packaged `Biorouter.app` and attach over CDP.
 *
 * `electron.launch({ executablePath })` cannot be used here — see the module
 * header. The executable is spawned directly, `ENABLE_PLAYWRIGHT` turns on the
 * remote-debugging port, and `/json/list` is polled until the renderer target
 * exists (a target has to be *present*, not merely a listening socket, or
 * `connectOverCDP` attaches to a browser with no pages).
 */
export async function launchPackagedApp(
  appPath: string,
  root: string,
  cdpPort: number
): Promise<LaunchedApp> {
  const exe = path.join(appPath, 'Contents/MacOS/Biorouter');
  if (!fs.existsSync(exe)) {
    throw new Error(`Packaged executable not found at ${exe}`);
  }

  const child: ChildProcess = spawn(exe, [`--user-data-dir=${path.join(root, 'electron')}`], {
    stdio: 'pipe',
    env: {
      ...process.env,
      ENABLE_PLAYWRIGHT: 'true',
      PLAYWRIGHT_CDP_PORT: String(cdpPort),
      ...sandboxEnv(root),
      ELECTRON_RUN_AS_NODE: '',
    },
  });
  const spawnedPid = child.pid;
  if (!spawnedPid) throw new Error('packaged app did not report a pid');
  child.stdout?.on('data', (d) => process.stdout.write(`[packaged] ${d}`));
  child.stderr?.on('data', (d) => process.stderr.write(`[packaged] ${d}`));

  const deadline = Date.now() + 120_000;
  let target: CdpTarget | undefined;
  while (Date.now() < deadline) {
    try {
      const targets = await fetchJson(`http://127.0.0.1:${cdpPort}/json/list`, 5000);
      target = targets.find(
        (t) => t.type === 'page' && !!t.url && !t.url.startsWith('devtools://')
      );
      if (target) break;
    } catch {
      // Not listening yet.
    }
    await new Promise((r) => setTimeout(r, 1500));
  }
  if (!target) {
    throw new Error(`No CDP page target on port ${cdpPort} within 120s`);
  }

  const browser: Browser = await chromium.connectOverCDP(`http://127.0.0.1:${cdpPort}`);
  const pages = browser.contexts().flatMap((ctx) => ctx.pages());
  const page = pages.find((p) => p.url() === target?.url) ?? pages[0];
  if (!page) throw new Error('connected over CDP but the browser exposed no page');
  await waitForRenderer(page);

  return {
    page,
    close: async () => {
      // Collect the daemon (and any other child) BEFORE the parent dies, so the
      // teardown only ever kills pids it started — never a `pkill -f` pattern
      // that would also match another worktree's daemon.
      const tree = descendantPids(spawnedPid);
      await browser.close().catch(() => {});
      for (const pid of [...tree, spawnedPid]) {
        try {
          process.kill(pid, 'SIGTERM');
        } catch {
          // Already gone.
        }
      }
      await new Promise((r) => setTimeout(r, 2000));
      for (const pid of [...tree, spawnedPid]) {
        try {
          process.kill(pid, 'SIGKILL');
        } catch {
          // Already gone.
        }
      }
    },
  };
}
