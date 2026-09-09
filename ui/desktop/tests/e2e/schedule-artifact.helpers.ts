/**
 * Sandbox, fixture and launch helpers for `schedule-artifact.spec.ts`.
 *
 * What is here is only what the SHARED helpers cannot provide: the packaged-app
 * launch, the workflow fixture, and the evidence directory. Sandboxing, the dev
 * bundle launch and the startup-dialog dance all come from `helpers/sandbox.ts`
 * and `helpers/app.ts`, so there is ONE implementation of the sandbox invariant
 * in this directory rather than two that drift.
 *
 * The one thing that is load-bearing and local, measured rather than assumed:
 *
 *  **The packaged app cannot be launched by `electron.launch`.** Playwright
 *  drives Electron with `--inspect=0` and blocks until the child prints
 *  `Debugger listening on ws://`
 *  (`node_modules/playwright-core/lib/server/electron/electron.js`), and the
 *  packaged build disables `EnableNodeCliInspectArguments` (`forge.config.ts`),
 *  so that line never arrives and the launch hangs until the test times out. Measured directly: a 45 s `electron.launch` against the
 *  notarized 1.90.3 build spent its whole budget and produced no window, failing
 *  with `TimeoutError` from `ProgressController.run`. So the packaged variant
 *  spawns the executable itself and attaches over CDP — `main.ts:696-699` turns
 *  `ENABLE_PLAYWRIGHT` + `PLAYWRIGHT_CDP_PORT` into a `remote-debugging-port`
 *  switch.
 */

import { chromium, type Browser, type Page } from '@playwright/test';
import { execFileSync, spawn, type ChildProcess } from 'child_process';
import * as fs from 'fs';
import * as http from 'http';
import * as path from 'path';
import { closeApp, dismissFirstRunModals, launchApp } from './helpers/app';
import type { Sandbox } from './helpers/sandbox';

/** Repo-relative root of the desktop package (`ui/desktop`). */
export const DESKTOP_ROOT = path.join(__dirname, '../..');

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
 * Launch the dev bundle against an already-created sandbox.
 *
 * A thin adapter over the shared `launchApp`, which owns the sandbox invariant,
 * the `--user-data-dir` half of it, the `ELECTRON_RUN_AS_NODE` clearing and the
 * startup-dialog dance. All this adds is the `{ page, close }` shape the two
 * variants share, so the spec can treat a dev bundle and a packaged app alike.
 */
export async function launchDevBundle(sandbox: Sandbox): Promise<LaunchedApp> {
  const launched = await launchApp({ sandbox });
  return {
    page: launched.page,
    close: () => closeApp(launched),
  };
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

/**
 * Terminate a process and everything it started, by pid.
 *
 * Never `pkill -f`: the obvious pattern (`target/debug/biorouterd agent`) also
 * matches every OTHER worktree's daemon and the developer's own running app.
 */
function killTree(pid: number): void {
  const tree = descendantPids(pid);
  for (const target of [...tree, pid]) {
    try {
      process.kill(target, 'SIGKILL');
    } catch {
      // Already gone.
    }
  }
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
